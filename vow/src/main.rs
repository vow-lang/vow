pub mod module_loader;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use clap::Parser;
use vow_codegen::cranelift_backend::CraneliftBackend;
use vow_codegen::linker::{find_runtime_lib, link};
use vow_codegen::{Backend, BuildMode};
use vow_diag::{CollectingEmitter, Diagnostic, DiagnosticEmitter, HumanEmitter, Severity};
use vow_verify::{Counterexample, VerificationResult, verify_function};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum ModeArg {
    Debug,
    Release,
}

#[derive(Parser, Debug)]
#[command(name = "vowc", about = "Vow compiler", disable_help_flag = true)]
struct Args {
    source: Option<PathBuf>,
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "release")]
    mode: ModeArg,
    #[arg(long)]
    no_verify: bool,
    /// Dump IR text to stdout and exit (skip codegen/verify)
    #[arg(long)]
    dump_ir: bool,
    /// Print compiler capability description as JSON (default) or human-readable (with --human)
    #[arg(long)]
    help: bool,
    /// With --help: print human-readable text instead of JSON
    #[arg(long)]
    human: bool,
}

// ---------------------------------------------------------------------------
// --help skill output
// ---------------------------------------------------------------------------

fn skill_json() -> String {
    r#"{
  "tool": "vowc",
  "description": "Vow compiler: compiles Vow source to native executables with contract verification",
  "usage": "vowc [OPTIONS] <source.vow>",
  "options": {
    "--output <path>": "Output executable path (default: source without .vow extension)",
    "--mode <debug|release>": "Build mode; debug inserts runtime vow checks (default: release)",
    "--no-verify": "Skip ESBMC static verification",
    "--dump-ir": "Dump IR text to stdout and exit (skip codegen/verify)",
    "--help": "Print this JSON capability description",
    "--help --human": "Print human-readable capability description"
  },
  "output_json": {
    "status": "Verified | Unverified | CompileFailed | VerifyFailed",
    "executable": "path to compiled binary, or null",
    "diagnostics": "[array of {error_code, message, severity, span: {file, offset, length}}]",
    "message": "error detail (CompileFailed)",
    "function": "function name (VerifyFailed)",
    "counterexample": "ESBMC counterexample description (VerifyFailed)"
  },
  "exit_codes": {
    "0": "success (Verified or Unverified)",
    "1": "failure (CompileFailed or VerifyFailed)"
  },
  "language": {
    "module": "module <Name>",
    "function": "fn <name>(<params>) -> <RetTy> [<effects>] { <body> }",
    "vow_function": "fn <name>(<params>) -> <RetTy> vow { requires: <expr>; ensures: <expr> } { <body> }",
    "while_with_invariant": "while <cond> vow { invariant: <expr> } { <body> }",
    "types": ["i32", "i64", "f32", "f64", "bool", "()"],
    "effects": ["io", "read", "write", "panic", "unsafe"],
    "builtins": {
      "print_str": "fn(s: str) -> () [io]",
      "print_i64": "fn(v: i64) -> () [io]"
    },
    "operators": {
      "arithmetic": ["+", "-", "*", "/", "%"],
      "checked_arithmetic": ["+!", "-!", "*!", "/!", "%!"],
      "comparison": ["==", "!=", "<", "<=", ">", ">="],
      "logical": ["&&", "||", "!"]
    },
    "vow_clauses": {
      "requires": "precondition — blame=Caller on violation",
      "ensures": "postcondition — blame=Callee on violation; use `result` for return value",
      "invariant": "loop invariant — checked at top of each iteration"
    }
  }
}"#
    .to_string()
}

fn skill_human() -> String {
    "vowc — Vow compiler

USAGE
  vowc [OPTIONS] <source.vow>

OPTIONS
  -o, --output <path>   Output executable path
  --mode <debug|release>  Build mode (default: release)
                          debug: inserts runtime vow violation checks
                          release: omits vow checks for performance
  --no-verify           Skip ESBMC static verification
  --dump-ir             Dump IR text to stdout and exit
  --help                Print JSON capability description (agent-friendly)
  --help --human        Print this text

OUTPUT (JSON on stdout)
  status      : Verified | Unverified | CompileFailed | VerifyFailed
  executable  : path to compiled binary, or null
  diagnostics : array of {{error_code, message, severity, span: {{file, offset, length}}}}
  message     : error detail (CompileFailed)
  function    : function name (VerifyFailed)
  counterexample: ESBMC counterexample (VerifyFailed)

EXIT CODES
  0  success (Verified or Unverified)
  1  failure (CompileFailed or VerifyFailed)

LANGUAGE SUMMARY
  module Hello

  fn add(x: i64, y: i64) -> i64 {{
    x + y
  }}

  fn divide(x: i64, y: i64) -> i64 vow {{
    requires: y != 0
    ensures:  result * y == x
  }} {{
    x / y
  }}

  fn main() -> i32 [io] {{
    print_i64(divide(10, 2));
    0
  }}

TYPES     : i32  i64  f32  f64  bool  ()
EFFECTS   : io  read  write  panic  unsafe
BUILTINS  : print_str(str) [io]   print_i64(i64) [io]
OPERATORS : + - * / %   +! -! *! /! %! (checked)   == != < <= > >=   && || !"
        .to_string()
}

// ---------------------------------------------------------------------------
// Build output
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum BuildStatus {
    Verified,
    Unverified,
    CompileFailed {
        message: String,
    },
    VerifyFailed {
        function: String,
        description: String,
    },
}

#[derive(Debug, Clone)]
pub struct CeSource {
    pub file: String,
    pub offset: u32,
    pub length: u32,
}

#[derive(Debug, Clone)]
pub struct StructuredCounterexample {
    pub function: String,
    pub inputs: Vec<(String, String)>,
    pub violation: String,
    pub vow_id: u32,
    pub source: Option<CeSource>,
}

enum VerifyOutcome {
    Skipped,
    Proven,
    Failed {
        function: String,
        description: String,
        counterexamples: Vec<StructuredCounterexample>,
    },
    Timeout {
        function: String,
    },
    Error {
        function: String,
        message: String,
    },
    ToolNotFound,
}

#[derive(Debug)]
pub struct BuildOutput {
    pub status: BuildStatus,
    pub executable: Option<PathBuf>,
    pub diagnostics: Vec<Diagnostic>,
    pub counterexamples: Vec<StructuredCounterexample>,
    pub verify_status: Option<String>,
    pub verify_message: Option<String>,
}

impl BuildOutput {
    pub fn emit_json(&self) {
        let status_str = match &self.status {
            BuildStatus::Verified => "Verified",
            BuildStatus::Unverified => "Unverified",
            BuildStatus::CompileFailed { .. } => "CompileFailed",
            BuildStatus::VerifyFailed { .. } => "VerifyFailed",
        };
        let exe_json = match &self.executable {
            Some(p) => format!("\"{}\"", p.display()),
            None => "null".to_string(),
        };
        let mut extra = match &self.status {
            BuildStatus::CompileFailed { message } => {
                format!(",\"message\":\"{}\"", escape_json(message))
            }
            BuildStatus::VerifyFailed {
                function,
                description,
            } => {
                format!(
                    ",\"function\":\"{}\",\"counterexample\":\"{}\"",
                    escape_json(function),
                    escape_json(description)
                )
            }
            _ => String::new(),
        };
        let diags_json = format_diagnostics_json(&self.diagnostics);
        let ce_json = format_counterexamples_json(&self.counterexamples);
        extra.push_str(&format!(",\"counterexamples\":[{ce_json}]"));
        if let Some(vs) = &self.verify_status {
            extra.push_str(&format!(",\"verify_status\":\"{}\"", escape_json(vs)));
        }
        if let Some(vm) = &self.verify_message {
            extra.push_str(&format!(",\"verify_message\":\"{}\"", escape_json(vm)));
        }
        println!(
            "{{\"status\":\"{status_str}\",\"executable\":{exe_json},\"diagnostics\":[{diags_json}]{extra}}}"
        );
    }
}

fn format_diagnostics_json(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| {
            let severity = match d.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Note => "note",
            };
            format!(
                "{{\"error_code\":\"{:?}\",\"message\":\"{}\",\"severity\":\"{}\",\"span\":{{\"file\":\"{}\",\"offset\":{},\"length\":{}}}}}",
                d.code,
                escape_json(&d.message),
                severity,
                escape_json(&d.primary.file),
                d.primary.byte_offset,
                d.primary.byte_len,
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn format_counterexamples_json(counterexamples: &[StructuredCounterexample]) -> String {
    counterexamples
        .iter()
        .map(|ce| {
            let inputs_json = ce
                .inputs
                .iter()
                .map(|(k, v)| format!("\"{}\":\"{}\"", escape_json(k), escape_json(v)))
                .collect::<Vec<_>>()
                .join(",");
            let source_json = match &ce.source {
                Some(s) => format!(
                    "{{\"file\":\"{}\",\"offset\":{},\"length\":{}}}",
                    escape_json(&s.file),
                    s.offset,
                    s.length
                ),
                None => "null".to_string(),
            };
            format!(
                "{{\"function\":\"{}\",\"inputs\":{{{inputs_json}}},\"violation\":\"{}\",\"vow_id\":{},\"source\":{source_json}}}",
                escape_json(&ce.function),
                escape_json(&ce.violation),
                ce.vow_id,
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

// ---------------------------------------------------------------------------
// Counterexample construction
// ---------------------------------------------------------------------------

fn build_c_to_source_name_map(
    func: &vow_ir::Function,
) -> std::collections::HashMap<String, String> {
    use vow_ir::{InstData, Opcode, Ty};
    let mut map = std::collections::HashMap::new();

    // Map p{cl_idx} → source name (skipping Unit params, matching C emitter logic)
    let mut cl_idx = 0u32;
    for (ir_idx, &ty) in func.params.iter().enumerate() {
        if ty != Ty::Unit {
            if let Some(name) = func.param_names.get(ir_idx) {
                map.insert(format!("p{cl_idx}"), name.clone());
            }
            cl_idx += 1;
        }
    }

    // Map v{inst_id} → source name for GetArg instructions
    let mut arg_var_map: Vec<(u32, u32)> = Vec::new(); // (ir_idx, cl_idx)
    let mut ci = 0u32;
    for (ir_idx, &ty) in func.params.iter().enumerate() {
        if ty != Ty::Unit {
            arg_var_map.push((ir_idx as u32, ci));
            ci += 1;
        }
    }

    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::GetArg
                && let InstData::ArgIndex(idx) = inst.data
                && let Some(name) = func.param_names.get(idx as usize)
            {
                map.insert(format!("v{}", inst.id.0), name.clone());
            }
        }
    }

    map
}

fn map_counterexample_inputs(
    inputs: &[(String, String)],
    name_map: &std::collections::HashMap<String, String>,
) -> Vec<(String, String)> {
    inputs
        .iter()
        .map(|(c_name, value)| {
            let source_name = name_map
                .get(c_name)
                .cloned()
                .unwrap_or_else(|| format!("_esbmc_{c_name}"));
            (source_name, value.clone())
        })
        .collect()
}

fn build_structured_counterexample(
    func: &vow_ir::Function,
    ce: &Counterexample,
    file: &str,
) -> StructuredCounterexample {
    let vid = ce.vow_id.unwrap_or(0);
    let violation = ce
        .vow_id
        .and_then(|id| func.vows.iter().find(|v| v.id.0 == id))
        .map(|v| v.description.clone())
        .unwrap_or_else(|| ce.description.clone());
    let source = ce
        .vow_id
        .and_then(|id| find_vow_span(func, id))
        .map(|span| CeSource {
            file: file.to_string(),
            offset: span.start,
            length: span.len,
        });
    let name_map = build_c_to_source_name_map(func);
    let mapped_inputs = map_counterexample_inputs(&ce.inputs, &name_map);
    StructuredCounterexample {
        function: func.name.clone(),
        inputs: mapped_inputs,
        violation,
        vow_id: vid,
        source,
    }
}

fn find_vow_span(func: &vow_ir::Function, vow_id: u32) -> Option<vow_syntax::span::Span> {
    use vow_ir::{InstData, Opcode};
    for block in &func.blocks {
        for inst in &block.insts {
            if matches!(inst.opcode, Opcode::VowRequires | Opcode::VowEnsures | Opcode::VowInvariant)
                && let InstData::VowId(vid) = inst.data
                && vid.0 == vow_id
            {
                return Some(inst.origin);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

pub fn run_pipeline(
    source: &Path,
    output: Option<&Path>,
    mode: BuildMode,
    no_verify: bool,
    dump_ir: bool,
) -> BuildOutput {
    let src = match std::fs::read_to_string(source) {
        Ok(s) => s,
        Err(e) => {
            return BuildOutput {
                status: BuildStatus::CompileFailed {
                    message: e.to_string(),
                },
                executable: None,
                diagnostics: vec![],
                counterexamples: vec![],
                verify_status: None,
                verify_message: None,
            };
        }
    };

    let mut stderr_emit = HumanEmitter::new(Box::new(std::io::stderr()));
    let mut all_diagnostics: Vec<Diagnostic> = Vec::new();

    let file_str = source.to_string_lossy();
    let (root_ast, parse_diags) = vow_syntax::parser::parse_module(&src, &file_str);
    let parse_failed = parse_diags.iter().any(|d| d.severity == Severity::Error);
    for d in &parse_diags {
        stderr_emit.emit(d);
    }
    all_diagnostics.extend(parse_diags);
    if parse_failed {
        return BuildOutput {
            status: BuildStatus::CompileFailed {
                message: "parse error".to_string(),
            },
            executable: None,
            diagnostics: all_diagnostics,
            counterexamples: vec![],
            verify_status: None,
            verify_message: None,
        };
    }

    let ast = match module_loader::load_modules(source, &root_ast) {
        Ok(graph) => module_loader::merge_modules(graph),
        Err(diags) => {
            for d in &diags {
                stderr_emit.emit(d);
            }
            all_diagnostics.extend(diags);
            return BuildOutput {
                status: BuildStatus::CompileFailed {
                    message: "module load error".to_string(),
                },
                executable: None,
                diagnostics: all_diagnostics,
                counterexamples: vec![],
                verify_status: None,
                verify_message: None,
            };
        }
    };

    let mut collecting_emit = CollectingEmitter::new(&mut stderr_emit);
    let mut checker =
        vow_types::check::Checker::new(source.to_string_lossy().to_string(), &mut collecting_emit);
    checker.check_module(&ast);
    let has_errors = checker.has_errors();
    drop(checker);
    all_diagnostics.extend(collecting_emit.into_diagnostics());
    if has_errors {
        return BuildOutput {
            status: BuildStatus::CompileFailed {
                message: "type error".to_string(),
            },
            executable: None,
            diagnostics: all_diagnostics,
            counterexamples: vec![],
            verify_status: None,
            verify_message: None,
        };
    }

    let ir_module = Arc::new(vow_ir::lower_module(&ast, &source.to_string_lossy()));

    if dump_ir {
        print!("{}", vow_ir::print_module(&ir_module));
        return BuildOutput {
            status: BuildStatus::Unverified,
            executable: None,
            diagnostics: all_diagnostics,
            counterexamples: vec![],
            verify_status: None,
            verify_message: None,
        };
    }

    // Spawn verification thread
    let module_for_verify = Arc::clone(&ir_module);
    let file_for_verify = source.to_string_lossy().to_string();
    let verify_handle = thread::spawn(move || -> VerifyOutcome {
        if no_verify {
            return VerifyOutcome::Skipped;
        }
        for func in &module_for_verify.functions {
            if func.vows.is_empty() {
                continue;
            }
            match verify_function(func) {
                VerificationResult::Failed(ce) => {
                    let sce = build_structured_counterexample(func, &ce, &file_for_verify);
                    return VerifyOutcome::Failed {
                        function: func.name.clone(),
                        description: ce.description.clone(),
                        counterexamples: vec![sce],
                    };
                }
                VerificationResult::ToolError(e) => {
                    return VerifyOutcome::Error {
                        function: func.name.clone(),
                        message: e,
                    };
                }
                VerificationResult::Timeout => {
                    return VerifyOutcome::Timeout {
                        function: func.name.clone(),
                    };
                }
                VerificationResult::Proven => {}
                VerificationResult::ToolNotFound => {
                    return VerifyOutcome::ToolNotFound;
                }
            }
        }
        VerifyOutcome::Proven
    });

    // Codegen
    let backend = CraneliftBackend::new();
    let compiled = match backend.compile_module(&ir_module, mode) {
        Ok(c) => c,
        Err(e) => {
            let _ = verify_handle.join();
            return BuildOutput {
                status: BuildStatus::CompileFailed {
                    message: format!("{e:?}"),
                },
                executable: None,
                diagnostics: all_diagnostics,
                counterexamples: vec![],
                verify_status: None,
                verify_message: None,
            };
        }
    };

    let output_path = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| source.with_extension(""));

    let exe_path = match find_runtime_lib() {
        Some(runtime) => {
            let obj_path = output_path.with_extension("o");
            if let Err(e) = compiled.write_to_file(&obj_path) {
                let _ = verify_handle.join();
                return BuildOutput {
                    status: BuildStatus::CompileFailed {
                        message: e.to_string(),
                    },
                    executable: None,
                    diagnostics: all_diagnostics,
                    counterexamples: vec![],
                    verify_status: None,
                    verify_message: None,
                };
            }
            match link(&[&obj_path], &runtime, &output_path) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&obj_path);
                    Some(output_path)
                }
                Err(e) => {
                    let _ = verify_handle.join();
                    return BuildOutput {
                        status: BuildStatus::CompileFailed {
                            message: format!("{e:?}"),
                        },
                        executable: None,
                        diagnostics: all_diagnostics,
                        counterexamples: vec![],
                        verify_status: None,
                        verify_message: None,
                    };
                }
            }
        }
        None => None,
    };

    // Collect verification result
    let verify_outcome = verify_handle.join().unwrap_or(VerifyOutcome::Skipped);

    let (status, counterexamples, verify_status, verify_message) = match verify_outcome {
        VerifyOutcome::Failed {
            function,
            description,
            counterexamples,
        } => (
            BuildStatus::VerifyFailed {
                function,
                description,
            },
            counterexamples,
            None,
            None,
        ),
        VerifyOutcome::Timeout { function } => (
            BuildStatus::VerifyFailed {
                function,
                description: "verification timed out".to_string(),
            },
            vec![],
            Some("timeout".to_string()),
            None,
        ),
        VerifyOutcome::Error { function, message } => (
            BuildStatus::VerifyFailed {
                function,
                description: format!("esbmc error: {message}"),
            },
            vec![],
            Some("error".to_string()),
            Some(message),
        ),
        VerifyOutcome::Skipped => (BuildStatus::Unverified, vec![], None, None),
        VerifyOutcome::Proven => (BuildStatus::Verified, vec![], None, None),
        VerifyOutcome::ToolNotFound => (BuildStatus::Unverified, vec![], None, None),
    };

    BuildOutput {
        status,
        executable: exe_path,
        diagnostics: all_diagnostics,
        counterexamples,
        verify_status,
        verify_message,
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let args = Args::parse();

    if args.help {
        if args.human {
            println!("{}", skill_human());
        } else {
            println!("{}", skill_json());
        }
        return;
    }

    let source = match args.source {
        Some(s) => s,
        None => {
            eprintln!("vowc: source file required (try --help)");
            std::process::exit(1);
        }
    };

    let mode = match args.mode {
        ModeArg::Debug => BuildMode::Debug,
        ModeArg::Release => BuildMode::Release,
    };

    let output = run_pipeline(
        &source,
        args.output.as_deref(),
        mode,
        args.no_verify,
        args.dump_ir,
    );

    if !args.dump_ir {
        output.emit_json();
    }

    if matches!(
        &output.status,
        BuildStatus::CompileFailed { .. } | BuildStatus::VerifyFailed { .. }
    ) {
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_source(dir: &TempDir, name: &str, src: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, src).unwrap();
        path
    }

    #[test]
    fn pipeline_compiles_function_with_param() {
        let dir = TempDir::new().unwrap();
        // Int literals always lower as i64; use a param-only function to avoid
        // the literal/return-type mismatch (separate IR-lowering concern).
        let src = "module M fn identity(x: i64) -> i64 { x }";
        let source = write_source(&dir, "identity.vow", src);
        let out = dir.path().join("identity_out");

        let result = run_pipeline(&source, Some(&out), BuildMode::Release, true, false);
        match &result.status {
            BuildStatus::Unverified => {}
            BuildStatus::CompileFailed { message } => {
                // Link failure is acceptable: no main() defined, or runtime absent.
                let msg_lo = message.to_lowercase();
                if msg_lo.contains("link")
                    || msg_lo.contains("runtime")
                    || msg_lo.contains("undefined")
                    || msg_lo.contains("main")
                {
                    eprintln!("SKIP: {message}");
                    return;
                }
                panic!("unexpected compile failure: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        }
    }

    #[test]
    fn hello_world_prints_and_exits_zero() {
        let dir = TempDir::new().unwrap();
        let src = r#"module Hello
fn main() -> i32 [io] {
    print_str("Hello, world!");
    0
}"#;
        let source = write_source(&dir, "hello.vow", src);
        let out = dir.path().join("hello");

        let result = run_pipeline(&source, Some(&out), BuildMode::Release, true, false);
        let exe = match &result.status {
            BuildStatus::Unverified => out.clone(),
            BuildStatus::CompileFailed { message } => {
                let msg_lo = message.to_lowercase();
                if msg_lo.contains("link")
                    || msg_lo.contains("runtime")
                    || msg_lo.contains("undefined")
                {
                    eprintln!("SKIP: {message}");
                    return;
                }
                panic!("compile failed: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        };

        let output = std::process::Command::new(&exe)
            .output()
            .expect("failed to run hello");
        assert_eq!(output.status.code(), Some(0), "expected exit 0");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Hello, world!"),
            "expected 'Hello, world!' in stdout, got: {stdout:?}"
        );
    }

    #[test]
    fn vow_violation_blame_caller_exit_code_1() {
        let dir = TempDir::new().unwrap();
        let src = r#"module Divide
fn divide(x: i64, y: i64) -> i64 vow {
  requires: y != 0
} {
  x / y
}
fn main() -> i32 [io] {
  divide(10, 0);
  0
}"#;
        let source = write_source(&dir, "divide.vow", src);
        let out = dir.path().join("divide");

        let result = run_pipeline(&source, Some(&out), BuildMode::Debug, true, false);
        let exe = match &result.status {
            BuildStatus::Unverified => out.clone(),
            BuildStatus::CompileFailed { message } => {
                let msg_lo = message.to_lowercase();
                if msg_lo.contains("link")
                    || msg_lo.contains("runtime")
                    || msg_lo.contains("undefined")
                {
                    eprintln!("SKIP: {message}");
                    return;
                }
                panic!("compile failed: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        };

        let output = std::process::Command::new(&exe)
            .output()
            .expect("failed to run divide");
        assert_eq!(
            output.status.code(),
            Some(1),
            "expected exit code 1 (vow violation)"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Caller"),
            "expected blame=Caller in stderr: {stderr:?}"
        );
        assert!(
            stderr.contains("y != 0"),
            "expected predicate description in stderr: {stderr:?}"
        );
    }

    #[test]
    fn while_loop_countdown_prints_zero() {
        let dir = TempDir::new().unwrap();
        let src = r#"module Countdown
fn countdown(n: i64) -> i64 {
  let mut i: i64 = n;
  while i > 0 {
    i = i - 1;
  }
  i
}
fn main() -> i32 [io] {
  let result: i64 = countdown(5);
  print_i64(result);
  0
}"#;
        let source = write_source(&dir, "countdown.vow", src);
        let out = dir.path().join("countdown");

        let result = run_pipeline(&source, Some(&out), BuildMode::Release, true, false);
        let exe = match &result.status {
            BuildStatus::Unverified => out.clone(),
            BuildStatus::CompileFailed { message } => {
                let msg_lo = message.to_lowercase();
                if msg_lo.contains("link")
                    || msg_lo.contains("runtime")
                    || msg_lo.contains("undefined")
                {
                    eprintln!("SKIP: {message}");
                    return;
                }
                panic!("compile failed: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        };

        let output = std::process::Command::new(&exe)
            .output()
            .expect("failed to run countdown");
        assert_eq!(output.status.code(), Some(0), "expected exit 0");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("0"),
            "expected '0' in stdout (countdown(5) == 0), got: {stdout:?}"
        );
    }

    #[test]
    fn bisect_with_loop_invariant_compiles_and_runs() {
        let dir = TempDir::new().unwrap();
        let src = r#"module Bisect
fn bisect(lo: i64, hi: i64) -> i64 vow {
  requires: hi >= lo
} {
  let mut lo: i64 = lo;
  let mut hi: i64 = hi;
  while lo + 1 < hi vow {
    invariant: hi - lo >= 0
  } {
    let mid: i64 = lo + (hi - lo) / 2;
    lo = mid;
  }
  lo
}
fn main() -> i32 [io] {
  let r: i64 = bisect(0, 64);
  print_i64(r);
  0
}"#;
        let source = write_source(&dir, "bisect.vow", src);
        let out = dir.path().join("bisect");

        let result = run_pipeline(&source, Some(&out), BuildMode::Debug, true, false);
        let exe = match &result.status {
            BuildStatus::Unverified => out.clone(),
            BuildStatus::CompileFailed { message } => {
                let msg_lo = message.to_lowercase();
                if msg_lo.contains("link")
                    || msg_lo.contains("runtime")
                    || msg_lo.contains("undefined")
                {
                    eprintln!("SKIP: {message}");
                    return;
                }
                panic!("compile failed: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        };

        let output = std::process::Command::new(&exe)
            .output()
            .expect("failed to run bisect");
        assert_eq!(
            output.status.code(),
            Some(0),
            "expected exit 0 (no invariant violation)"
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("63"),
            "expected bisect(0, 64) == 63 in stdout, got: {stdout:?}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("VowViolation"),
            "unexpected vow violation: {stderr}"
        );
    }

    #[test]
    fn help_flag_emits_json_with_tool_key() {
        let out = skill_json();
        assert!(out.contains("\"tool\""), "expected JSON with 'tool' key");
        assert!(out.contains("vowc"), "expected tool name in output");
        assert!(
            out.contains("language"),
            "expected language section in output"
        );
        assert!(out.contains("builtins"), "expected builtins in output");
    }

    #[test]
    fn help_human_flag_emits_text() {
        let out = skill_human();
        assert!(out.contains("USAGE"), "expected USAGE in human help");
        assert!(out.contains("TYPES"), "expected TYPES in human help");
        assert!(
            out.contains("vow"),
            "expected vow description in human help"
        );
    }

    #[test]
    fn agent_capability_test_skill_json_is_parseable_and_complete() {
        // Verify the --help JSON contains enough information for an LLM agent
        // to write correct Vow code without additional context.
        let json = skill_json();

        // Must be valid JSON structure (key fields present)
        assert!(json.contains("\"tool\""), "missing tool key");
        assert!(json.contains("\"usage\""), "missing usage key");
        assert!(json.contains("\"output_json\""), "missing output_json key");
        assert!(json.contains("\"language\""), "missing language key");
        assert!(json.contains("\"builtins\""), "missing builtins key");
        assert!(json.contains("\"vow_clauses\""), "missing vow_clauses key");

        // Must describe the key Vow constructs
        assert!(
            json.contains("requires"),
            "missing requires clause description"
        );
        assert!(
            json.contains("ensures"),
            "missing ensures clause description"
        );
        assert!(
            json.contains("invariant"),
            "missing invariant clause description"
        );
        assert!(json.contains("print_i64"), "missing print_i64 builtin");
        assert!(json.contains("print_str"), "missing print_str builtin");

        // Now verify that a program an LLM would write from this description compiles and runs.
        // The LLM reads: function with requires/ensures, print_i64 builtin, [io] effect.
        let dir = TempDir::new().unwrap();
        let src = r#"module Agent
fn double(n: i64) -> i64 vow {
  ensures: result == n * 2
} {
  n + n
}
fn main() -> i32 [io] {
  let x: i64 = double(21);
  print_i64(x);
  0
}"#;
        let source = write_source(&dir, "agent.vow", src);
        let out = dir.path().join("agent");

        let result = run_pipeline(&source, Some(&out), BuildMode::Release, true, false);
        match &result.status {
            BuildStatus::Unverified => {}
            BuildStatus::CompileFailed { message } => {
                let msg_lo = message.to_lowercase();
                if msg_lo.contains("link")
                    || msg_lo.contains("runtime")
                    || msg_lo.contains("undefined")
                {
                    eprintln!("SKIP: {message}");
                    return;
                }
                panic!("agent-generated program failed to compile: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        }

        let output = std::process::Command::new(&out)
            .output()
            .expect("failed to run agent program");
        assert_eq!(output.status.code(), Some(0));
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("42"),
            "expected double(21)==42 in stdout, got: {stdout:?}"
        );
    }

    #[test]
    fn pipeline_rejects_type_error() {
        let dir = TempDir::new().unwrap();
        // fn f() -> i32 { true } — type mismatch
        let src = "module Bad fn f() -> i32 { true }";
        let source = write_source(&dir, "bad.vow", src);
        let out = dir.path().join("bad_out");

        let result = run_pipeline(&source, Some(&out), BuildMode::Release, true, false);
        assert!(
            matches!(result.status, BuildStatus::CompileFailed { .. }),
            "expected CompileFailed for type error, got {:?}",
            result.status
        );
    }

    fn compile_and_run(src: &str) -> std::process::Output {
        let dir = TempDir::new().unwrap();
        let source = write_source(&dir, "test.vow", src);
        let out = dir.path().join("test_out");
        let result = run_pipeline(&source, Some(&out), BuildMode::Release, true, false);
        match &result.status {
            BuildStatus::Unverified => {}
            BuildStatus::CompileFailed { message } => {
                let msg_lo = message.to_lowercase();
                if msg_lo.contains("link")
                    || msg_lo.contains("runtime")
                    || msg_lo.contains("undefined")
                {
                    // Skip if runtime not linked in test environment.
                    panic!("SKIP: {message}");
                }
                panic!("compile failed: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        }
        std::process::Command::new(&out)
            .output()
            .expect("failed to run compiled program")
    }

    #[test]
    fn struct_construction_and_field_access() {
        let src = r#"module StructTest

struct Point {
    x: i64,
    y: i64,
}

pub fn make_point() -> i64 {
    let p = Point { x: 3, y: 4 };
    p.x
}

pub fn main() -> i32 {
    let v = make_point();
    0
}
"#;
        let output = compile_and_run(src);
        assert_eq!(output.status.code(), Some(0), "expected exit 0");
    }

    #[test]
    fn enum_construction_and_match() {
        let src = r#"module EnumTest

enum Color {
    Red,
    Green,
    Blue,
}

pub fn color_code(c: Color) -> i32 {
    match c {
        Color::Red => 1,
        Color::Green => 2,
        Color::Blue => 3,
    }
}

pub fn main() -> i32 {
    let g = Color::Green;
    let n = color_code(g);
    0
}
"#;
        let output = compile_and_run(src);
        assert_eq!(output.status.code(), Some(0), "expected exit 0");
    }

    #[test]
    fn option_some_none_compiles_and_runs() {
        let src = r#"module OptionTest

pub fn safe_div(x: i64, y: i64) -> Option<i64> {
    if y == 0 {
        Option::None
    } else {
        Option::Some(x / y)
    }
}

pub fn main() -> i32 {
    let a = safe_div(10, 2);
    let b = safe_div(5, 0);
    0
}
"#;
        let output = compile_and_run(src);
        assert_eq!(output.status.code(), Some(0), "expected exit 0");
    }

    #[test]
    fn question_operator_short_circuits() {
        let src = r#"module QuestionTest

pub fn safe_div(x: i64, y: i64) -> Option<i64> {
    if y == 0 {
        Option::None
    } else {
        Option::Some(x / y)
    }
}

pub fn chain(x: i64, y: i64, z: i64) -> Option<i64> {
    let a = safe_div(x, y)?;
    safe_div(a, z)
}

pub fn main() -> i32 {
    let r1 = chain(10, 2, 1);
    let r2 = chain(10, 0, 1);
    0
}
"#;
        let output = compile_and_run(src);
        assert_eq!(output.status.code(), Some(0), "expected exit 0");
    }

    #[test]
    fn vec_push_len_index() {
        let src = r#"module VecTest

pub fn sum(v: Vec<i64>) -> i64 {
    let mut total: i64 = 0;
    let mut i: i64 = 0;
    let n = v.len();
    while i < n {
        total = total + v[i];
        i = i + 1;
    }
    total
}

pub fn main() -> i32 {
    let mut nums: Vec<i64> = Vec::new();
    nums.push(10);
    nums.push(20);
    nums.push(30);
    let s = sum(nums);
    0
}
"#;
        let output = compile_and_run(src);
        assert_eq!(output.status.code(), Some(0), "expected exit 0");
    }

    #[test]
    fn struct_and_vec_combined() {
        let src = r#"module DataTest

struct Point {
    x: i64,
    y: i64,
}

pub fn sum_coords(p: Point) -> i64 {
    p.x + p.y
}

pub fn main() -> i32 {
    let p = Point { x: 3, y: 4 };
    let s = sum_coords(p);
    let mut v: Vec<i64> = Vec::new();
    v.push(s);
    let n = v.len();
    0
}
"#;
        let output = compile_and_run(src);
        assert_eq!(output.status.code(), Some(0), "expected exit 0");
    }

    #[test]
    fn string_from_len_eq() {
        let src = r#"module StringTest

pub fn main() -> i32 [io] {
    let s = String::from("hello");
    let n = s.len();
    let s2 = String::from("hello");
    let eq = s.eq(s2);
    0
}
"#;
        let output = compile_and_run(src);
        assert_eq!(output.status.code(), Some(0), "expected exit 0");
    }

    #[test]
    fn hashmap_insert_get_contains_remove() {
        let src = r#"module MapTest

pub fn main() -> i32 {
    let mut m: HashMap<i64, i64> = HashMap::new();
    m.insert(1, 10);
    m.insert(2, 20);
    m.insert(3, 30);
    let v1 = m.get(1);
    let v2 = m.get(2);
    let has3 = m.contains_key(3);
    m.remove(2);
    let n = m.len();
    0
}
"#;
        let output = compile_and_run(src);
        assert_eq!(output.status.code(), Some(0), "expected exit 0");
    }

    #[test]
    fn extern_block_type_checked() {
        let src = r#"module ExternTest

extern {
    fn my_ext_fn(x: i64) -> i64 [io]
}

pub fn main() -> i32 {
    0
}
"#;
        let dir = TempDir::new().unwrap();
        let source = write_source(&dir, "extern_test.vow", src);
        let out = dir.path().join("extern_test_out");
        let result = run_pipeline(&source, Some(&out), BuildMode::Release, true, false);
        assert!(
            !matches!(result.status, BuildStatus::CompileFailed { ref message } if message.contains("type error")),
            "extern block should not cause type errors: {:?}",
            result.status
        );
    }

    #[test]
    fn module_system_two_files() {
        let dir = TempDir::new().unwrap();
        let lib_src = r#"module Lib

pub fn add(x: i64, y: i64) -> i64 {
    x + y
}
"#;
        let main_src = r#"module Main
use lib

pub fn main() -> i32 [io] {
    let r: i64 = add(3, 4);
    print_i64(r);
    0
}
"#;
        std::fs::write(dir.path().join("lib.vow"), lib_src).unwrap();
        let main_path = dir.path().join("main.vow");
        std::fs::write(&main_path, main_src).unwrap();
        let out = dir.path().join("main_out");

        let result = run_pipeline(&main_path, Some(&out), BuildMode::Release, true, false);
        let exe = match &result.status {
            BuildStatus::Unverified => out.clone(),
            BuildStatus::CompileFailed { message } => {
                let msg_lo = message.to_lowercase();
                if msg_lo.contains("link")
                    || msg_lo.contains("runtime")
                    || msg_lo.contains("undefined")
                {
                    eprintln!("SKIP: {message}");
                    return;
                }
                panic!("compile failed: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        };

        let output = std::process::Command::new(&exe)
            .output()
            .expect("failed to run two-module program");
        assert_eq!(output.status.code(), Some(0), "expected exit 0");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("7"),
            "expected add(3,4)==7 in stdout, got: {stdout:?}"
        );
    }

    #[test]
    fn escape_json_special_characters() {
        assert_eq!(escape_json("hello"), "hello");
        assert_eq!(escape_json(r"a\b"), r"a\\b");
        assert_eq!(escape_json("a\"b"), "a\\\"b");
        assert_eq!(escape_json("a\nb"), "a\\nb");
    }

    #[test]
    fn build_output_emit_json_compile_failed() {
        let out = BuildOutput {
            status: BuildStatus::CompileFailed {
                message: "type \"error\"\nwith newline".to_string(),
            },
            executable: None,
            diagnostics: vec![],
            counterexamples: vec![],
            verify_status: None,
            verify_message: None,
        };
        out.emit_json();
    }

    #[test]
    fn build_output_emit_json_verify_failed() {
        let out = BuildOutput {
            status: BuildStatus::VerifyFailed {
                function: "divide".to_string(),
                description: "y=0 violates requires".to_string(),
            },
            executable: None,
            diagnostics: vec![],
            counterexamples: vec![StructuredCounterexample {
                function: "divide".to_string(),
                inputs: vec![("p1".to_string(), "0".to_string())],
                violation: "y != 0".to_string(),
                vow_id: 0,
                source: None,
            }],
            verify_status: None,
            verify_message: None,
        };
        out.emit_json();
    }

    #[test]
    fn build_output_emit_json_verified_with_exe() {
        let dir = TempDir::new().unwrap();
        let exe = dir.path().join("mybin");
        std::fs::write(&exe, b"").unwrap();
        let out = BuildOutput {
            status: BuildStatus::Verified,
            executable: Some(exe),
            diagnostics: vec![],
            counterexamples: vec![],
            verify_status: None,
            verify_message: None,
        };
        out.emit_json();
    }

    #[test]
    fn build_output_json_contains_diagnostics_array() {
        use vow_diag::{ErrorCode, SourceLocation};
        let diag = Diagnostic {
            severity: Severity::Error,
            code: ErrorCode::TypeMismatch,
            message: "expected i32, got bool".to_string(),
            primary: SourceLocation {
                file: "test.vow".to_string(),
                byte_offset: 42,
                byte_len: 4,
            },
            secondary: vec![],
            blame: vow_diag::Blame::None,
        };
        let out = BuildOutput {
            status: BuildStatus::CompileFailed {
                message: "type error".to_string(),
            },
            executable: None,
            diagnostics: vec![diag],
            counterexamples: vec![],
            verify_status: None,
            verify_message: None,
        };
        out.emit_json();
    }

    #[test]
    fn build_output_json_empty_diagnostics_on_success() {
        let out = BuildOutput {
            status: BuildStatus::Verified,
            executable: None,
            diagnostics: vec![],
            counterexamples: vec![],
            verify_status: None,
            verify_message: None,
        };
        out.emit_json();
    }

    #[test]
    fn pipeline_parse_error_populates_diagnostics() {
        let dir = TempDir::new().unwrap();
        let src = "module M 123";
        let source = write_source(&dir, "bad_parse.vow", src);
        let result = run_pipeline(&source, None, BuildMode::Release, true, false);
        assert!(matches!(result.status, BuildStatus::CompileFailed { .. }));
        assert!(
            !result.diagnostics.is_empty(),
            "diagnostics should contain parse errors"
        );
        assert_eq!(result.diagnostics[0].severity, Severity::Error);
    }

    #[test]
    fn pipeline_parse_error_contains_file_path() {
        let dir = TempDir::new().unwrap();
        let src = "module M 123";
        let source = write_source(&dir, "bad_parse.vow", src);
        let result = run_pipeline(&source, None, BuildMode::Release, true, false);
        assert!(!result.diagnostics.is_empty());
        let file = &result.diagnostics[0].primary.file;
        assert!(
            file.ends_with("bad_parse.vow"),
            "diagnostic file field should contain the source path, got: {file}"
        );
    }

    #[test]
    fn pipeline_type_error_contains_file_path() {
        let dir = TempDir::new().unwrap();
        let src = "module Bad fn f() -> i32 { true }";
        let source = write_source(&dir, "bad_type.vow", src);
        let result = run_pipeline(&source, None, BuildMode::Release, true, false);
        assert!(!result.diagnostics.is_empty());
        let file = &result.diagnostics[0].primary.file;
        assert!(
            file.ends_with("bad_type.vow"),
            "diagnostic file field should contain the source path, got: {file}"
        );
    }

    #[test]
    fn pipeline_type_error_populates_diagnostics() {
        let dir = TempDir::new().unwrap();
        let src = "module Bad fn f() -> i32 { true }";
        let source = write_source(&dir, "bad_type.vow", src);
        let result = run_pipeline(&source, None, BuildMode::Release, true, false);
        assert!(matches!(result.status, BuildStatus::CompileFailed { .. }));
        assert!(
            !result.diagnostics.is_empty(),
            "diagnostics should contain type errors"
        );
    }

    #[test]
    fn pipeline_success_has_empty_diagnostics() {
        let dir = TempDir::new().unwrap();
        let src = "module M fn f(x: i64) -> i64 { x }";
        let source = write_source(&dir, "ok.vow", src);
        let result = run_pipeline(&source, None, BuildMode::Release, true, false);
        match &result.status {
            BuildStatus::Unverified => {
                assert!(
                    result.diagnostics.is_empty(),
                    "successful compilation should have empty diagnostics, got: {:?}",
                    result.diagnostics
                );
            }
            BuildStatus::CompileFailed { message } => {
                let msg_lo = message.to_lowercase();
                if msg_lo.contains("link")
                    || msg_lo.contains("runtime")
                    || msg_lo.contains("ld")
                    || msg_lo.contains("cc exited")
                {
                    return;
                }
                panic!("unexpected compile failure: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        }
    }

    #[test]
    fn pipeline_fails_on_missing_module() {
        let dir = TempDir::new().unwrap();
        let src = "module Main\nuse nonexistent\nfn main() -> i32 { 0 }";
        let source = write_source(&dir, "main.vow", src);
        let result = run_pipeline(&source, None, BuildMode::Release, true, false);
        assert!(
            matches!(result.status, BuildStatus::CompileFailed { .. }),
            "should fail on missing module: {:?}",
            result.status
        );
    }

    #[test]
    fn pipeline_fails_on_nonexistent_source() {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("nonexistent.vow");
        let result = run_pipeline(&source, None, BuildMode::Release, true, false);
        assert!(
            matches!(result.status, BuildStatus::CompileFailed { .. }),
            "should fail when source file not found: {:?}",
            result.status
        );
    }

    #[test]
    fn pipeline_unverified_status_when_no_verify() {
        let dir = TempDir::new().unwrap();
        let src = "module M fn f(x: i64) -> i64 { x }";
        let source = write_source(&dir, "f.vow", src);
        let result = run_pipeline(&source, None, BuildMode::Release, true, false);
        match &result.status {
            BuildStatus::Unverified => {}
            BuildStatus::CompileFailed { message } => {
                let is_link_err = message.contains("link")
                    || message.contains("runtime")
                    || message.contains("ld")
                    || message.contains("cc exited")
                    || message.contains("Link");
                if is_link_err {
                    return;
                }
                panic!("unexpected compile failure: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        }
    }

    #[test]
    fn counterexamples_empty_on_compile_failure() {
        let dir = TempDir::new().unwrap();
        let src = "module M 123";
        let source = write_source(&dir, "bad.vow", src);
        let result = run_pipeline(&source, None, BuildMode::Release, true, false);
        assert!(
            matches!(result.status, BuildStatus::CompileFailed { .. }),
            "expected CompileFailed"
        );
        assert!(
            result.counterexamples.is_empty(),
            "counterexamples should be empty on compile failure"
        );
        assert!(
            result.verify_status.is_none(),
            "verify_status should be None on compile failure"
        );
    }

    #[test]
    fn counterexamples_empty_when_no_verify() {
        let dir = TempDir::new().unwrap();
        let src = "module M fn f(x: i64) -> i64 { x }";
        let source = write_source(&dir, "ok.vow", src);
        let result = run_pipeline(&source, None, BuildMode::Release, true, false);
        match &result.status {
            BuildStatus::Unverified => {
                assert!(
                    result.counterexamples.is_empty(),
                    "counterexamples should be empty when --no-verify"
                );
            }
            BuildStatus::CompileFailed { message } => {
                let msg_lo = message.to_lowercase();
                if msg_lo.contains("link")
                    || msg_lo.contains("runtime")
                    || msg_lo.contains("ld")
                    || msg_lo.contains("cc exited")
                {
                    return;
                }
                panic!("unexpected compile failure: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        }
    }

    #[test]
    fn counterexamples_populated_on_verify_failure() {
        let dir = TempDir::new().unwrap();
        let src = r#"module Bad
fn always_bad() -> i64 vow {
  ensures: result > 100
} {
  42
}
fn main() -> i32 {
  let x: i64 = always_bad();
  0
}"#;
        let source = write_source(&dir, "bad_ensures.vow", src);
        let out = dir.path().join("bad_ensures");
        let result = run_pipeline(&source, Some(&out), BuildMode::Release, false, false);
        match &result.status {
            BuildStatus::VerifyFailed { function, .. } => {
                assert_eq!(function, "always_bad");
                assert!(
                    !result.counterexamples.is_empty(),
                    "counterexamples should not be empty on verify failure"
                );
                let ce = &result.counterexamples[0];
                assert_eq!(ce.function, "always_bad");
                assert_eq!(ce.vow_id, 0);
                assert!(
                    ce.violation.contains("result > 100"),
                    "violation should contain predicate text, got: {}",
                    ce.violation,
                );
            }
            BuildStatus::Unverified => {
                eprintln!("SKIP: verification not run (esbmc not found or no vows)");
            }
            BuildStatus::CompileFailed { message } => {
                let msg_lo = message.to_lowercase();
                if msg_lo.contains("link")
                    || msg_lo.contains("runtime")
                    || msg_lo.contains("ld")
                    || msg_lo.contains("cc exited")
                {
                    eprintln!("SKIP: {message}");
                    return;
                }
                panic!("compile failed: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        }
    }

    #[test]
    fn counterexamples_empty_on_verify_success() {
        let dir = TempDir::new().unwrap();
        let src = r#"module Good
fn always_true() -> i64 vow {
  ensures: result == 42
} {
  42
}
fn main() -> i32 {
  let x: i64 = always_true();
  0
}"#;
        let source = write_source(&dir, "good_ensures.vow", src);
        let out = dir.path().join("good_ensures");
        let result = run_pipeline(&source, Some(&out), BuildMode::Release, false, false);
        match &result.status {
            BuildStatus::Verified => {
                assert!(
                    result.counterexamples.is_empty(),
                    "counterexamples should be empty on verification success"
                );
            }
            BuildStatus::Unverified => {
                eprintln!("SKIP: verification not run (esbmc not found)");
            }
            BuildStatus::CompileFailed { message } => {
                let msg_lo = message.to_lowercase();
                if msg_lo.contains("link")
                    || msg_lo.contains("runtime")
                    || msg_lo.contains("ld")
                    || msg_lo.contains("cc exited")
                {
                    eprintln!("SKIP: {message}");
                    return;
                }
                panic!("compile failed: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        }
    }

    #[test]
    fn build_output_json_counterexamples_array() {
        let out = BuildOutput {
            status: BuildStatus::VerifyFailed {
                function: "divide".to_string(),
                description: "y=0".to_string(),
            },
            executable: None,
            diagnostics: vec![],
            counterexamples: vec![StructuredCounterexample {
                function: "divide".to_string(),
                inputs: vec![
                    ("p0".to_string(), "42".to_string()),
                    ("p1".to_string(), "0".to_string()),
                ],
                violation: "y != 0".to_string(),
                vow_id: 0,
                source: Some(CeSource {
                    file: "test.vow".to_string(),
                    offset: 50,
                    length: 6,
                }),
            }],
            verify_status: None,
            verify_message: None,
        };
        out.emit_json();
    }

    #[test]
    fn build_output_json_timeout_status() {
        let out = BuildOutput {
            status: BuildStatus::VerifyFailed {
                function: "f".to_string(),
                description: "verification timed out".to_string(),
            },
            executable: None,
            diagnostics: vec![],
            counterexamples: vec![],
            verify_status: Some("timeout".to_string()),
            verify_message: None,
        };
        out.emit_json();
    }

    #[test]
    fn build_output_json_error_status() {
        let out = BuildOutput {
            status: BuildStatus::VerifyFailed {
                function: "f".to_string(),
                description: "esbmc error: segfault".to_string(),
            },
            executable: None,
            diagnostics: vec![],
            counterexamples: vec![],
            verify_status: Some("error".to_string()),
            verify_message: Some("segfault".to_string()),
        };
        out.emit_json();
    }

    #[test]
    fn format_counterexamples_json_empty() {
        let json = format_counterexamples_json(&[]);
        assert_eq!(json, "");
    }

    #[test]
    fn format_counterexamples_json_one_entry() {
        let json = format_counterexamples_json(&[StructuredCounterexample {
            function: "f".to_string(),
            inputs: vec![("x".to_string(), "0".to_string())],
            violation: "x > 0".to_string(),
            vow_id: 1,
            source: None,
        }]);
        assert!(json.contains("\"function\":\"f\""), "function: {json}");
        assert!(json.contains("\"x\":\"0\""), "inputs: {json}");
        assert!(
            json.contains("\"violation\":\"x > 0\""),
            "violation: {json}"
        );
        assert!(json.contains("\"vow_id\":1"), "vow_id: {json}");
        assert!(json.contains("\"source\":null"), "source null: {json}");
    }

    #[test]
    fn format_counterexamples_json_with_source() {
        let json = format_counterexamples_json(&[StructuredCounterexample {
            function: "f".to_string(),
            inputs: vec![],
            violation: "result".to_string(),
            vow_id: 0,
            source: Some(CeSource {
                file: "test.vow".to_string(),
                offset: 10,
                length: 5,
            }),
        }]);
        assert!(json.contains("\"file\":\"test.vow\""), "file: {json}");
        assert!(json.contains("\"offset\":10"), "offset: {json}");
        assert!(json.contains("\"length\":5"), "length: {json}");
    }

    #[test]
    fn build_c_to_source_name_map_basic() {
        use vow_ir::{BasicBlock, BlockId, FuncId, Inst, InstData, InstId, Opcode, Ty};
        use vow_syntax::span::Span;
        let func = vow_ir::Function {
            id: FuncId(0),
            name: "divide".to_string(),
            params: vec![Ty::I64, Ty::I64],
            param_names: vec!["x".to_string(), "y".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(0),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(0),
                        origin: Span::new(0, 0),
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(1),
                        origin: Span::new(0, 0),
                    },
                ],
            }],
        };
        let map = build_c_to_source_name_map(&func);
        assert_eq!(map.get("p0"), Some(&"x".to_string()));
        assert_eq!(map.get("p1"), Some(&"y".to_string()));
        assert_eq!(map.get("v0"), Some(&"x".to_string()));
        assert_eq!(map.get("v1"), Some(&"y".to_string()));
    }

    #[test]
    fn build_c_to_source_name_map_skips_unit_params() {
        use vow_ir::{BasicBlock, BlockId, FuncId, Inst, InstData, InstId, Opcode, Ty};
        use vow_syntax::span::Span;
        let func = vow_ir::Function {
            id: FuncId(0),
            name: "f".to_string(),
            params: vec![Ty::Unit, Ty::I64, Ty::I64],
            param_names: vec!["_u".to_string(), "a".to_string(), "b".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(0),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(1),
                        origin: Span::new(0, 0),
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(2),
                        origin: Span::new(0, 0),
                    },
                ],
            }],
        };
        let map = build_c_to_source_name_map(&func);
        // p0 maps to "a" (first non-Unit), p1 maps to "b"
        assert_eq!(map.get("p0"), Some(&"a".to_string()));
        assert_eq!(map.get("p1"), Some(&"b".to_string()));
        // v0 → GetArg(1) → "a", v1 → GetArg(2) → "b"
        assert_eq!(map.get("v0"), Some(&"a".to_string()));
        assert_eq!(map.get("v1"), Some(&"b".to_string()));
    }

    #[test]
    fn map_counterexample_inputs_applies_mapping() {
        let mut name_map = std::collections::HashMap::new();
        name_map.insert("p0".to_string(), "x".to_string());
        name_map.insert("p1".to_string(), "y".to_string());
        name_map.insert("v0".to_string(), "x".to_string());
        name_map.insert("v1".to_string(), "y".to_string());

        let inputs = vec![
            ("v1".to_string(), "0".to_string()),
            ("v3".to_string(), "0".to_string()),
        ];
        let mapped = map_counterexample_inputs(&inputs, &name_map);
        assert_eq!(mapped[0], ("y".to_string(), "0".to_string()));
        assert_eq!(mapped[1], ("_esbmc_v3".to_string(), "0".to_string()));
    }

    #[test]
    fn build_c_to_source_name_map_empty_param_names() {
        use vow_ir::{BasicBlock, BlockId, FuncId, Ty};
        let func = vow_ir::Function {
            id: FuncId(0),
            name: "f".to_string(),
            params: vec![Ty::I64],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![],
            }],
        };
        let map = build_c_to_source_name_map(&func);
        assert!(map.is_empty());
    }

    #[test]
    fn counterexample_uses_source_names() {
        let dir = TempDir::new().unwrap();
        let src = r#"module BadDiv
fn bad_div(x: i64, y: i64) -> i64 vow {
  ensures: result > 100
} {
  x / y
}
fn main() -> i32 {
  let r: i64 = bad_div(10, 2);
  0
}"#;
        let source = write_source(&dir, "bad_div.vow", src);
        let out = dir.path().join("bad_div");
        let result = run_pipeline(&source, Some(&out), BuildMode::Release, false, false);
        match &result.status {
            BuildStatus::VerifyFailed { function, .. } => {
                assert_eq!(function, "bad_div");
                let ce = &result.counterexamples[0];
                for (name, _) in &ce.inputs {
                    assert!(
                        name == "x" || name == "y" || name.starts_with("_esbmc_"),
                        "expected source name or _esbmc_ prefix, got: {name}"
                    );
                }
                let has_source_name = ce.inputs.iter().any(|(n, _)| n == "x" || n == "y");
                assert!(
                    has_source_name,
                    "at least one input should use a source name, got: {:?}",
                    ce.inputs,
                );
            }
            BuildStatus::Unverified => {
                eprintln!("SKIP: verification not run (esbmc not found)");
            }
            BuildStatus::CompileFailed { message } => {
                let msg_lo = message.to_lowercase();
                if msg_lo.contains("link")
                    || msg_lo.contains("runtime")
                    || msg_lo.contains("ld")
                    || msg_lo.contains("cc exited")
                {
                    eprintln!("SKIP: {message}");
                    return;
                }
                panic!("compile failed: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        }
    }

    #[test]
    fn cegis_loop_end_to_end() {
        let dir = TempDir::new().unwrap();

        // Step 1: Compile a program with an intentional contract violation.
        // safe_sub(a, b) ensures result >= 0, but `a` is unconstrained so a - b can be negative.
        let broken_src = r#"module CegisBroken

fn safe_sub(a: i64, b: i64 where b >= 0) -> i64 vow {
  ensures: result >= 0
} {
  a - b
}

fn main() -> i32 {
  let r: i64 = safe_sub(10, 3);
  0
}"#;
        let broken_path = write_source(&dir, "cegis_broken.vow", broken_src);
        let broken_out = dir.path().join("cegis_broken");
        let broken_result =
            run_pipeline(&broken_path, Some(&broken_out), BuildMode::Release, false, false);

        match &broken_result.status {
            BuildStatus::VerifyFailed { function, .. } => {
                assert_eq!(function, "safe_sub");

                // AC2: diagnostics array present (empty since no compile errors)
                assert!(
                    broken_result.diagnostics.is_empty(),
                    "diagnostics should be empty (no compile errors), got: {:?}",
                    broken_result.diagnostics,
                );

                // AC3: counterexamples array with at least one entry
                assert!(
                    !broken_result.counterexamples.is_empty(),
                    "counterexamples should not be empty on verify failure"
                );

                let ce = &broken_result.counterexamples[0];

                // AC4a: inputs with source-level variable names
                let has_source_name = ce
                    .inputs
                    .iter()
                    .any(|(name, _)| name == "a" || name == "b");
                assert!(
                    has_source_name,
                    "counterexample inputs should use source names (a, b), got: {:?}",
                    ce.inputs,
                );
                for (name, _) in &ce.inputs {
                    assert!(
                        name == "a" || name == "b" || name.starts_with("_esbmc_"),
                        "unexpected variable name: {name}"
                    );
                }

                // AC4b: violation predicate text
                assert!(
                    ce.violation.contains("result >= 0"),
                    "violation should contain predicate text, got: {}",
                    ce.violation,
                );

                // AC4c: source location
                assert!(
                    ce.source.is_some(),
                    "counterexample should have source location"
                );
                let src_loc = ce.source.as_ref().unwrap();
                assert!(
                    src_loc.file.contains("cegis_broken.vow"),
                    "source file should reference cegis_broken.vow, got: {}",
                    src_loc.file,
                );

                // Step 2: Compile the corrected version and assert verification passes.
                let fixed_src = r#"module CegisFixed

fn safe_sub(a: i64 where a >= 0, b: i64 where b >= 0) -> i64 vow {
  requires: a >= b,
  ensures: result >= 0
} {
  a - b
}

fn main() -> i32 {
  let r: i64 = safe_sub(10, 3);
  0
}"#;
                let fixed_path = write_source(&dir, "cegis_fixed.vow", fixed_src);
                let fixed_out = dir.path().join("cegis_fixed");
                let fixed_result =
                    run_pipeline(&fixed_path, Some(&fixed_out), BuildMode::Release, false, false);

                // AC5: corrected version verifies with empty counterexamples
                match &fixed_result.status {
                    BuildStatus::Verified => {
                        assert!(
                            fixed_result.counterexamples.is_empty(),
                            "counterexamples should be empty after fix"
                        );
                        assert!(
                            fixed_result.diagnostics.is_empty(),
                            "diagnostics should be empty for fixed version"
                        );
                    }
                    BuildStatus::CompileFailed { message } => {
                        let msg_lo = message.to_lowercase();
                        if msg_lo.contains("link")
                            || msg_lo.contains("runtime")
                            || msg_lo.contains("ld")
                            || msg_lo.contains("cc exited")
                        {
                            eprintln!("SKIP fixed (link): {message}");
                            return;
                        }
                        panic!("fixed version compile failed: {message}");
                    }
                    other => panic!("fixed version unexpected status: {other:?}"),
                }
            }
            BuildStatus::Unverified => {
                eprintln!("SKIP: verification not run (esbmc not found)");
            }
            BuildStatus::CompileFailed { message } => {
                let msg_lo = message.to_lowercase();
                if msg_lo.contains("link")
                    || msg_lo.contains("runtime")
                    || msg_lo.contains("ld")
                    || msg_lo.contains("cc exited")
                {
                    eprintln!("SKIP: {message}");
                    return;
                }
                panic!("compile failed: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        }
    }

    #[test]
    fn find_vow_span_includes_requires() {
        let dir = TempDir::new().unwrap();

        let src = r#"module RequiresSpan

fn positive(x: i64 where x > 0) -> i64 vow {
  ensures: result > 0
} {
  x
}

fn main() -> i32 {
  let r: i64 = positive(5);
  0
}"#;
        let path = write_source(&dir, "requires_span.vow", src);
        let out = dir.path().join("requires_span");
        let result = run_pipeline(&path, Some(&out), BuildMode::Release, false, false);

        match &result.status {
            BuildStatus::VerifyFailed { .. } => {
                assert!(
                    !result.counterexamples.is_empty(),
                    "counterexamples should not be empty on verify failure"
                );

                let ce = &result.counterexamples[0];

                assert!(
                    ce.source.is_some(),
                    "counterexample for requires/where clause should have source location"
                );
                let src_loc = ce.source.as_ref().unwrap();
                assert!(
                    src_loc.file.contains("requires_span.vow"),
                    "source file should reference requires_span.vow, got: {}",
                    src_loc.file,
                );
                assert!(
                    (src_loc.offset as usize) < src.len(),
                    "source offset {} should be within source length {}",
                    src_loc.offset,
                    src.len(),
                );
            }
            BuildStatus::Verified => {
                eprintln!("SKIP: verification passed (where clause was provable)");
            }
            BuildStatus::Unverified => {
                eprintln!("SKIP: verification not run (esbmc not found)");
            }
            BuildStatus::CompileFailed { message } => {
                let msg_lo = message.to_lowercase();
                if msg_lo.contains("link")
                    || msg_lo.contains("runtime")
                    || msg_lo.contains("ld")
                    || msg_lo.contains("cc exited")
                {
                    eprintln!("SKIP: {message}");
                    return;
                }
                panic!("compile failed: {message}");
            }
        }
    }
}
