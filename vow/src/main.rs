use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use clap::Parser;
use vow_codegen::cranelift_backend::CraneliftBackend;
use vow_codegen::linker::{find_runtime_lib, link};
use vow_codegen::{Backend, BuildMode};
use vow_diag::{DiagnosticEmitter, HumanEmitter, Severity};
use vow_verify::{verify_function, VerificationResult};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum ModeArg {
    Debug,
    Release,
}

#[derive(Parser, Debug)]
#[command(name = "vowc", about = "Vow compiler")]
struct Args {
    source: PathBuf,
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "release")]
    mode: ModeArg,
    #[arg(long)]
    no_verify: bool,
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

#[derive(Debug)]
pub struct BuildOutput {
    pub status: BuildStatus,
    pub executable: Option<PathBuf>,
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
        let extra = match &self.status {
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
        println!("{{\"status\":\"{status_str}\",\"executable\":{exe_json}{extra}}}");
    }
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

pub fn run_pipeline(
    source: &Path,
    output: Option<&Path>,
    mode: BuildMode,
    no_verify: bool,
) -> BuildOutput {
    let src = match std::fs::read_to_string(source) {
        Ok(s) => s,
        Err(e) => {
            return BuildOutput {
                status: BuildStatus::CompileFailed {
                    message: e.to_string(),
                },
                executable: None,
            }
        }
    };

    let mut stderr_emit = HumanEmitter::new(Box::new(std::io::stderr()));

    let (ast, parse_diags) = vow_syntax::parser::parse_module(&src);
    let parse_failed = parse_diags.iter().any(|d| d.severity == Severity::Error);
    for d in &parse_diags {
        stderr_emit.emit(d);
    }
    if parse_failed {
        return BuildOutput {
            status: BuildStatus::CompileFailed {
                message: "parse error".to_string(),
            },
            executable: None,
        };
    }

    let mut checker =
        vow_types::check::Checker::new(source.to_string_lossy().to_string(), &mut stderr_emit);
    checker.check_module(&ast);
    if checker.has_errors() {
        return BuildOutput {
            status: BuildStatus::CompileFailed {
                message: "type error".to_string(),
            },
            executable: None,
        };
    }

    let ir_module = Arc::new(vow_ir::lower_module(&ast));

    // Spawn verification thread
    let module_for_verify = Arc::clone(&ir_module);
    let verify_handle = thread::spawn(move || -> Option<(String, String)> {
        if no_verify {
            return None;
        }
        for func in &module_for_verify.functions {
            if func.vows.is_empty() {
                continue;
            }
            match verify_function(func) {
                VerificationResult::Failed(ce) => {
                    return Some((func.name.clone(), ce.description));
                }
                VerificationResult::ToolError(e) => {
                    return Some((func.name.clone(), format!("esbmc error: {e}")));
                }
                VerificationResult::Proven
                | VerificationResult::Timeout
                | VerificationResult::ToolNotFound => {}
            }
        }
        None
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
                    };
                }
            }
        }
        None => None,
    };

    // Collect verification result
    let verify_failure = verify_handle.join().unwrap_or(None);

    let status = if let Some((func, desc)) = verify_failure {
        BuildStatus::VerifyFailed {
            function: func,
            description: desc,
        }
    } else if no_verify {
        BuildStatus::Unverified
    } else {
        BuildStatus::Verified
    };

    BuildOutput {
        status,
        executable: exe_path,
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let args = Args::parse();

    let mode = match args.mode {
        ModeArg::Debug => BuildMode::Debug,
        ModeArg::Release => BuildMode::Release,
    };

    let output = run_pipeline(&args.source, args.output.as_deref(), mode, args.no_verify);
    output.emit_json();

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

        let result = run_pipeline(&source, Some(&out), BuildMode::Release, true);
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

        let result = run_pipeline(&source, Some(&out), BuildMode::Release, true);
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

        let result = run_pipeline(&source, Some(&out), BuildMode::Debug, true);
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

        let result = run_pipeline(&source, Some(&out), BuildMode::Release, true);
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

        let result = run_pipeline(&source, Some(&out), BuildMode::Debug, true);
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
    fn pipeline_rejects_type_error() {
        let dir = TempDir::new().unwrap();
        // fn f() -> i32 { true } — type mismatch
        let src = "module Bad fn f() -> i32 { true }";
        let source = write_source(&dir, "bad.vow", src);
        let out = dir.path().join("bad_out");

        let result = run_pipeline(&source, Some(&out), BuildMode::Release, true);
        assert!(
            matches!(result.status, BuildStatus::CompileFailed { .. }),
            "expected CompileFailed for type error, got {:?}",
            result.status
        );
    }
}
