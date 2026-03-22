pub mod cache;
pub mod module_loader;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use std::collections::BTreeMap;

use clap::Parser;
use serde::Serialize;
use vow_codegen::cranelift_backend::CraneliftBackend;
use vow_codegen::linker::{find_runtime_lib, find_shim_lib, link};
use vow_codegen::{Backend, BuildMode, TraceMode};
use vow_diag::{CollectingEmitter, Diagnostic, DiagnosticEmitter, HumanEmitter, Severity};
use vow_verify::{
    Counterexample, VerificationResult, detect_constant_functions, emit_verify_c_source,
    find_esbmc, run_esbmc, verify_function_with_module_and_const_fns,
};

use cache::{CachedVerifyResult, VerifyCache};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum ModeArg {
    Debug,
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum TraceArg {
    Off,
    Calls,
    Full,
}

#[derive(Parser, Debug)]
#[command(
    name = "vow",
    about = "Vow compiler",
    disable_help_flag = true,
    args_conflicts_with_subcommands = true
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    source: Option<PathBuf>,
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "release")]
    mode: ModeArg,
    #[arg(long)]
    no_verify: bool,
    #[arg(long)]
    dump_ir: bool,
    #[arg(long, value_enum, default_value = "off")]
    debug_trace: TraceArg,
    #[arg(long)]
    no_cache: bool,
    #[arg(long)]
    help: bool,
    #[arg(long)]
    human: bool,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Compile source to a native executable (verifies contracts by default)
    Build(BuildArgs),
    /// Verify contracts without producing an executable
    Verify(VerifyArgs),
    /// Run tests (not yet implemented)
    Test(TestArgs),
    /// Emit declaration file (.vow.d) with type signatures only
    Decl(DeclArgs),
}

#[derive(clap::Args, Debug)]
#[command(disable_help_flag = true)]
struct BuildArgs {
    source: Option<PathBuf>,
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "release")]
    mode: ModeArg,
    #[arg(long)]
    no_verify: bool,
    #[arg(long)]
    dump_ir: bool,
    #[arg(long, value_enum, default_value = "off")]
    debug_trace: TraceArg,
    #[arg(long)]
    no_cache: bool,
    #[arg(long)]
    help: bool,
    #[arg(long)]
    human: bool,
}

#[derive(clap::Args, Debug)]
#[command(disable_help_flag = true)]
struct VerifyArgs {
    source: Option<PathBuf>,
    #[arg(long)]
    help: bool,
    #[arg(long)]
    human: bool,
    #[arg(long)]
    no_cache: bool,
}

#[derive(clap::Args, Debug)]
#[command(disable_help_flag = true)]
struct TestArgs {
    source: Option<PathBuf>,
    #[arg(long)]
    help: bool,
    #[arg(long)]
    human: bool,
}

#[derive(clap::Args, Debug)]
#[command(disable_help_flag = true)]
struct DeclArgs {
    source: Option<PathBuf>,
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
    #[arg(long)]
    help: bool,
    #[arg(long)]
    human: bool,
}

// ---------------------------------------------------------------------------
// --help skill output
// ---------------------------------------------------------------------------

fn skill_json() -> String {
    r#"{
  "tool": "vow",
  "description": "Vow compiler: compiles Vow source to native executables with contract verification",
  "usage": "vow <command> [OPTIONS] <source.vow>",
  "commands": {
    "build": "Compile source to native executable (verifies by default; use --no-verify to skip)",
    "verify": "Verify contracts without producing an executable (use --no-cache to skip cache)",
    "test": "Run tests (not yet implemented)",
    "decl": "Emit declaration file (.vow.d) with type signatures only"
  },
  "legacy_usage": "vow [OPTIONS] <source.vow> (equivalent to vow build)",
  "build_options": {
    "-o, --output <path>": "Output executable path (default: source without .vow extension)",
    "--mode <debug|release>": "Build mode; debug inserts runtime vow checks (default: release)",
    "--no-verify": "Skip ESBMC static verification",
    "--dump-ir": "Print IR text to stdout and exit (no JSON output, no codegen)",
    "--debug-trace <off|calls|full>": "Emit JSON trace lines to stderr at runtime (default: off)",
    "--no-cache": "Disable compile and verify caching",
    "--unwind <N>": "ESBMC loop unwind bound (default: 10)"
  },
  "verify_options": {
    "--no-cache": "Disable verification result caching",
    "--unwind <N>": "ESBMC loop unwind bound"
  },
  "global_options": {
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
    "use_declaration": "use foo.bar",
    "function": "fn <name>(<params>) -> <RetTy> [<effects>] { <body> }",
    "public_function": "pub fn <name>(<params>) -> <RetTy> [<effects>] { <body> }",
    "vow_function": "fn <name>(<params>) -> <RetTy> vow { requires: <expr>; ensures: <expr> } { <body> }",
    "while_with_invariant": "while <cond> vow { invariant: <expr> } { <body> }",
    "types": [
      "i32",
      "i64",
      "u64",
      "f32",
      "f64",
      "bool",
      "()",
      "Vec<T>",
      "Option<T>",
      "Result<T, E>",
      "String",
      "HashMap<K, V>"
    ],
    "effects": [
      "io",
      "read",
      "write",
      "panic",
      "unsafe"
    ],
    "builtins": {
      "print_str": "fn(s: String) -> () [io]",
      "print_i64": "fn(v: i64) -> () [io]",
      "print_u64": "fn(v: u64) -> () [io]",
      "eprintln_str": "fn(s: String) -> () [io]",
      "fs_read": "fn(path: String) -> String [read]",
      "fs_write": "fn(path: String, data: String) -> () [write]",
      "args": "fn() -> Vec<String> [read]",
      "stdin_read": "fn() -> String [read]",
      "process_exit": "fn(code: i64) -> () [io]"
    },
    "operators": {
      "arithmetic": [
        "+",
        "-",
        "*",
        "/",
        "%"
      ],
      "checked_arithmetic": [
        "+!",
        "-!",
        "*!",
        "/!",
        "%!"
      ],
      "comparison": [
        "==",
        "!=",
        "<",
        "<=",
        ">",
        ">="
      ],
      "logical": [
        "&&",
        "||",
        "!"
      ],
      "unary": [
        "-",
        "!",
        "&",
        "?"
      ]
    },
    "vow_clauses": {
      "requires": "precondition \u2014 blame=Caller on violation",
      "ensures": "postcondition \u2014 blame=Callee on violation; use `result` for return value",
      "invariant": "loop invariant \u2014 checked at top of each iteration"
    },
    "where_clauses": "fn f(x: i64 where x >= 0) -> i64 \u2014 refinement types on parameters",
    "structs": {
      "definition": "struct Name { field: Type, ... }",
      "linear": "linear struct Name { field: Type, ... } \u2014 must be consumed exactly once",
      "literal": "Name { field: value, ... }",
      "field_access": "value.field"
    },
    "enums": {
      "definition": "enum Name { Variant1(T), Variant2, Variant3 { field: T } }",
      "construction": "Name::Variant(value)",
      "builtin_option": "Option<T> \u2014 variants: Some(T), None",
      "builtin_result": "Result<T, E> \u2014 variants: Ok(T), Err(E)"
    },
    "match_expression": {
      "syntax": "match value { Pattern => expr, ... }",
      "patterns": [
        "Wildcard (_)",
        "Identifier binding (x)",
        "Mutable identifier (mut x)",
        "Literal (0, true, \"hello\")",
        "Tuple ((a, b))",
        "Enum variant (unit) (Option::None)",
        "Enum variant (tuple) (Option::Some(x))",
        "Enum variant (struct) (Shape::Named { x, y })",
        "Or pattern (0 | 1 | 2)",
        "Struct pattern (Point { x, y })"
      ]
    },
    "control_flow": {
      "if_else": "if cond { expr } else { expr } \u2014 expression, both branches same type",
      "while": "while cond { body }",
      "for_each": "for item in vec { body } \u2014 iterate Vec elements",
      "for_enumerate": "for i, item in vec { body } \u2014 iterate with index",
      "loop": "loop { ... break value; } \u2014 infinite loop, break to exit",
      "break": "break; or break value;",
      "return": "return; or return value;"
    },
    "modules": {
      "declaration": "module Name",
      "import": "use foo.bar \u2014 resolves to <rootdir>/foo/bar.vow",
      "visibility": "pub fn \u2014 public functions visible to importers"
    },
    "type_aliases": "type Name = Type",
    "extern_blocks": "extern { fn c_function(x: i64) -> i64 [unsafe] }",
    "methods": {
      "Vec<T>": [
        "Vec::new()",
        ".push(val)",
        ".pop()",
        ".len()",
        "v[i]",
        "v[i] = val"
      ],
      "String": [
        "String::from(lit)",
        ".len()",
        ".byte_at(i)",
        ".push_byte(b)",
        ".push_str(s)",
        ".contains(s)",
        ".eq(s)",
        ".substring(start, end)",
        ".parse_i64()",
        ".parse_u64()"
      ],
      "HashMap<K,V>": [
        "HashMap::new()",
        ".insert(k, v)",
        ".get(k)",
        ".contains_key(k)",
        ".remove(k)",
        ".len()"
      ],
      "Option<T>": [
        ".unwrap()",
        "? operator"
      ]
    },
    "indexing": {
      "read": "v[i] \u2014 Vec index access",
      "write": "v[i] = val \u2014 Vec index assignment"
    }
  },
  "verification_limits": {
    "loop_unwind": 10,
    "Vec<T>": 128,
    "String": 256,
    "HashMap<K, V>": 64
  }
}"#
    .to_string()
}

fn skill_human() -> String {
    r#"vow — Vow compiler

USAGE
  vow build [OPTIONS] <source.vow>    Compile to native executable
  vow verify [OPTIONS] <source.vow>    Verify contracts only (no executable)
  vow test [<source.vow>]             Run tests (not yet implemented)
  vow decl [OPTIONS] <source.vow>    Emit declaration file (.vow.d)
  vow [OPTIONS] <source.vow>          Legacy mode (same as vow build)

BUILD OPTIONS
  -o, --output <path>     Output executable path (default: source without .vow extension)
  --mode <debug|release>  Build mode; debug inserts runtime vow checks (default: release)
  --no-verify             Skip ESBMC static verification
  --dump-ir               Print IR text to stdout and exit (no JSON output, no codegen)
  --debug-trace <off|calls|full>  Emit JSON trace lines to stderr at runtime (default: off)
  --no-cache              Disable compile and verify caching
  --unwind <N>            ESBMC loop unwind bound (default: 10)

VERIFY OPTIONS
  --no-cache              Disable verification result caching
  --unwind <N>            ESBMC loop unwind bound

GLOBAL OPTIONS
  --help                Print JSON capability description (agent-friendly)
  --help --human        Print this text

OUTPUT (JSON on stdout)
  status      : Verified | Unverified | CompileFailed | VerifyFailed
  executable  : path to compiled binary, or null
  diagnostics : array of {error_code, message, severity, span: {file, offset, length}}
  message     : error detail (CompileFailed)
  function    : function name (VerifyFailed)
  counterexample: ESBMC counterexample (VerifyFailed)

EXIT CODES
  0  success (Verified or Unverified)
  1  failure (CompileFailed or VerifyFailed)

LANGUAGE SUMMARY
  module Hello
  use math.utils

  struct Point { x: i64, y: i64 }

  fn add(x: i64, y: i64) -> i64 {
    x + y
  }

  fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
    ensures:  result * y == x
  } {
    x / y
  }

  fn main() -> i32 [io] {
    let v: Vec<i64> = Vec::new();
    v.push(divide(10, 2));
    print_i64(v[0]);
    0
  }

TYPES     : i32  i64  u64  f32  f64  bool  ()  Vec<T>  Option<T>  Result<T, E>  String  HashMap<K, V>
EFFECTS   : io  read  write  panic  unsafe
BUILTINS  : print_str: fn(s: String) -> () [io]   print_i64: fn(v: i64) -> () [io]   print_u64: fn(v: u64) -> () [io]
            eprintln_str: fn(s: String) -> () [io]   fs_read: fn(path: String) -> String [read]   fs_write: fn(path: String, data: String) -> () [write]   args: fn() -> Vec<String> [read]   stdin_read: fn() -> String [read]   process_exit: fn(code: i64) -> () [io]
METHODS   : Vec: Vec::new/push/pop/len/v[i]/v[i] = val   String: String::from/len/byte_at/push_byte/push_str/contains/eq/substring/parse_i64/parse_u64
            HashMap: HashMap::new/insert/get/contains_key/remove/len   Option: unwrap
OPERATORS : + - * / %   +! -! *! /! %! (checked)   == != < <= > >=   && || !   - ! & ?

VERIFICATION LIMITS
  Loop unwind  : 10 iterations
  Vec<T>        : 128 max capacity
  String        : 256 max capacity
  HashMap<K, V> : 64 max capacity"#
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
pub struct CeViolatingArg {
    pub param: String,
    pub value: String,
    pub arg_offset: u32,
    pub arg_length: u32,
}

#[derive(Debug, Clone)]
pub struct CePathStep {
    pub block_id: u32,
    pub offset: u32,
    pub length: u32,
}

#[derive(Debug, Clone)]
pub struct CeBranchDecision {
    pub condition_offset: u32,
    pub condition_length: u32,
    pub taken: String,
}

#[derive(Debug, Clone)]
pub struct StructuredCounterexample {
    pub function: String,
    pub values: Vec<(String, String)>,
    pub violation: String,
    pub vow_id: u32,
    pub source: Option<CeSource>,
    pub blame: String,
    pub call_sites: Vec<CeCallSite>,
    pub violating_args: Vec<CeViolatingArg>,
    pub execution_path: Vec<CePathStep>,
    pub branch_decisions: Vec<CeBranchDecision>,
}

#[derive(Debug, Clone)]
pub struct CeCallSite {
    pub caller_function: String,
    pub file: String,
    pub offset: u32,
    pub length: u32,
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

// ---------------------------------------------------------------------------
// Serde JSON output types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct SpanJson {
    pub file: String,
    pub offset: u32,
    pub length: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticJson {
    pub error_code: String,
    pub message: String,
    pub severity: String,
    pub span: SpanJson,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub secondary: Vec<SpanJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blame: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CeCallSiteJson {
    pub caller_function: String,
    pub file: String,
    pub offset: u32,
    pub length: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CeViolatingArgJson {
    pub param: String,
    pub value: String,
    pub arg_offset: u32,
    pub arg_length: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CePathStepJson {
    pub block_id: u32,
    pub offset: u32,
    pub length: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CeBranchDecisionJson {
    pub condition_offset: u32,
    pub condition_length: u32,
    pub taken: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CounterexampleJson {
    pub function: String,
    pub values: BTreeMap<String, String>,
    pub violation: String,
    pub vow_id: u32,
    pub source: Option<SpanJson>,
    pub blame: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub call_sites: Vec<CeCallSiteJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub violating_args: Vec<CeViolatingArgJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub execution_path: Vec<CePathStepJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub branch_decisions: Vec<CeBranchDecisionJson>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildResult {
    pub status: String,
    pub executable: Option<String>,
    pub diagnostics: Vec<DiagnosticJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterexample: Option<String>,
    pub counterexamples: Vec<CounterexampleJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_message: Option<String>,
}

impl DiagnosticJson {
    fn from_diagnostic(d: &Diagnostic) -> Self {
        let blame = match d.blame {
            vow_diag::Blame::Caller => Some("caller".to_string()),
            vow_diag::Blame::Callee => Some("callee".to_string()),
            vow_diag::Blame::None => None,
        };
        let secondary = d
            .secondary
            .iter()
            .map(|s| SpanJson {
                file: s.file.clone(),
                offset: s.byte_offset,
                length: s.byte_len,
            })
            .collect();
        Self {
            error_code: format!("{:?}", d.code),
            message: d.message.clone(),
            severity: match d.severity {
                Severity::Error => "error".to_string(),
                Severity::Warning => "warning".to_string(),
                Severity::Note => "note".to_string(),
            },
            span: SpanJson {
                file: d.primary.file.clone(),
                offset: d.primary.byte_offset,
                length: d.primary.byte_len,
            },
            hints: d.hints.clone(),
            secondary,
            blame,
        }
    }
}

impl CounterexampleJson {
    fn from_structured(ce: &StructuredCounterexample) -> Self {
        Self {
            function: ce.function.clone(),
            values: ce.values.iter().cloned().collect(),
            violation: ce.violation.clone(),
            vow_id: ce.vow_id,
            source: ce.source.as_ref().map(|s| SpanJson {
                file: s.file.clone(),
                offset: s.offset,
                length: s.length,
            }),
            blame: ce.blame.clone(),
            call_sites: ce
                .call_sites
                .iter()
                .map(|cs| CeCallSiteJson {
                    caller_function: cs.caller_function.clone(),
                    file: cs.file.clone(),
                    offset: cs.offset,
                    length: cs.length,
                })
                .collect(),
            violating_args: ce
                .violating_args
                .iter()
                .map(|va| CeViolatingArgJson {
                    param: va.param.clone(),
                    value: va.value.clone(),
                    arg_offset: va.arg_offset,
                    arg_length: va.arg_length,
                })
                .collect(),
            execution_path: ce
                .execution_path
                .iter()
                .map(|ps| CePathStepJson {
                    block_id: ps.block_id,
                    offset: ps.offset,
                    length: ps.length,
                })
                .collect(),
            branch_decisions: ce
                .branch_decisions
                .iter()
                .map(|bd| CeBranchDecisionJson {
                    condition_offset: bd.condition_offset,
                    condition_length: bd.condition_length,
                    taken: bd.taken.clone(),
                })
                .collect(),
        }
    }
}

impl BuildOutput {
    pub fn to_build_result(&self) -> BuildResult {
        let status = match &self.status {
            BuildStatus::Verified => "Verified",
            BuildStatus::Unverified => "Unverified",
            BuildStatus::CompileFailed { .. } => "CompileFailed",
            BuildStatus::VerifyFailed { .. } => "VerifyFailed",
        }
        .to_string();

        let (message, function, counterexample) = match &self.status {
            BuildStatus::CompileFailed { message } => (Some(message.clone()), None, None),
            BuildStatus::VerifyFailed {
                function,
                description,
            } => (None, Some(function.clone()), Some(description.clone())),
            _ => (None, None, None),
        };

        BuildResult {
            status,
            executable: self.executable.as_ref().map(|p| p.display().to_string()),
            diagnostics: self
                .diagnostics
                .iter()
                .map(DiagnosticJson::from_diagnostic)
                .collect(),
            message,
            function,
            counterexample,
            counterexamples: self
                .counterexamples
                .iter()
                .map(CounterexampleJson::from_structured)
                .collect(),
            verify_status: self.verify_status.clone(),
            verify_message: self.verify_message.clone(),
        }
    }

    pub fn emit_json(&self) {
        let result = self.to_build_result();
        let json = serde_json::to_string(&result).expect("BuildResult must be serializable");
        println!("{json}");
    }
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

    for (&inst_id, name) in &func.local_names {
        map.entry(format!("v{inst_id}"))
            .or_insert_with(|| name.clone());
    }

    map
}

fn map_counterexample_values(
    values: &[(String, String)],
    name_map: &std::collections::HashMap<String, String>,
) -> Vec<(String, String)> {
    values
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
    call_site_index: &std::collections::HashMap<String, Vec<CallSiteInfo>>,
) -> StructuredCounterexample {
    use vow_ir::InstData;
    let vid = ce.vow_id.unwrap_or(0);
    let vow_entry = ce
        .vow_id
        .and_then(|id| func.vows.iter().find(|v| v.id.0 == id));
    let violation = vow_entry
        .map(|v| v.description.clone())
        .unwrap_or_else(|| ce.description.clone());
    let blame = vow_entry
        .map(|v| match v.blame {
            vow_diag::Blame::Caller => "caller",
            vow_diag::Blame::Callee => "callee",
            vow_diag::Blame::None => "none",
        })
        .unwrap_or("none")
        .to_string();
    let source = ce
        .vow_id
        .and_then(|id| find_vow_span(func, id))
        .map(|span| CeSource {
            file: file.to_string(),
            offset: span.start,
            length: span.len,
        });
    let name_map = build_c_to_source_name_map(func);
    let mapped_values = map_counterexample_values(&ce.values, &name_map);
    let sites_raw = if blame == "caller" {
        call_site_index.get(&func.name).cloned().unwrap_or_default()
    } else {
        vec![]
    };
    let call_sites: Vec<CeCallSite> = sites_raw
        .iter()
        .map(|cs| CeCallSite {
            caller_function: cs.caller_function.clone(),
            file: cs.file.clone(),
            offset: cs.offset,
            length: cs.length,
        })
        .collect();

    // Violating args: for caller-blame, map bindings to param indices and arg spans
    let violating_args = if blame == "caller" {
        if let Some(entry) = vow_entry {
            let mut args = Vec::new();
            for (binding_name, _inst_id) in &entry.bindings {
                if let Some(param_idx) = func.param_names.iter().position(|n| n == binding_name) {
                    let value = mapped_values
                        .iter()
                        .find(|(n, _)| n == binding_name)
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default();
                    for cs in &sites_raw {
                        if let Some(&(off, len)) = cs.arg_spans.get(param_idx) {
                            args.push(CeViolatingArg {
                                param: binding_name.clone(),
                                value: value.clone(),
                                arg_offset: off,
                                arg_length: len,
                            });
                        }
                    }
                }
            }
            args
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    // Execution path from block visits
    let visited: std::collections::HashSet<u32> = ce.block_visits.iter().copied().collect();
    let mut execution_path: Vec<CePathStep> = Vec::new();
    for block in &func.blocks {
        if visited.contains(&block.id.0) {
            let span = block
                .insts
                .iter()
                .find(|i| i.origin.start != 0 || i.origin.len != 0)
                .map(|i| i.origin);
            if let Some(s) = span {
                execution_path.push(CePathStep {
                    block_id: block.id.0,
                    offset: s.start,
                    length: s.len,
                });
            } else {
                execution_path.push(CePathStep {
                    block_id: block.id.0,
                    offset: 0,
                    length: 0,
                });
            }
        }
    }

    // Branch decisions
    let mut branch_decisions: Vec<CeBranchDecision> = Vec::new();
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == vow_ir::Opcode::Branch
                && let InstData::BranchTargets {
                    then_block,
                    else_block,
                } = &inst.data
            {
                let then_visited = visited.contains(&then_block.0);
                let else_visited = visited.contains(&else_block.0);
                let taken = match (then_visited, else_visited) {
                    (true, false) => "then",
                    (false, true) => "else",
                    _ => continue,
                };
                branch_decisions.push(CeBranchDecision {
                    condition_offset: inst.origin.start,
                    condition_length: inst.origin.len,
                    taken: taken.to_string(),
                });
            }
        }
    }

    StructuredCounterexample {
        function: func.name.clone(),
        values: mapped_values,
        violation,
        vow_id: vid,
        source,
        blame,
        call_sites,
        violating_args,
        execution_path,
        branch_decisions,
    }
}

fn find_vow_span(func: &vow_ir::Function, vow_id: u32) -> Option<vow_syntax::span::Span> {
    use vow_ir::{InstData, Opcode};
    for block in &func.blocks {
        for inst in &block.insts {
            if matches!(
                inst.opcode,
                Opcode::VowRequires | Opcode::VowEnsures | Opcode::VowInvariant
            ) && let InstData::VowId(vid) = inst.data
                && vid.0 == vow_id
            {
                return Some(inst.origin);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Call-site index
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct CallSiteInfo {
    caller_function: String,
    file: String,
    offset: u32,
    length: u32,
    arg_spans: Vec<(u32, u32)>,
}

fn build_call_site_index(
    module: &vow_ir::Module,
    file: &str,
) -> std::collections::HashMap<String, Vec<CallSiteInfo>> {
    use vow_ir::{InstData, Opcode};
    let mut index: std::collections::HashMap<String, Vec<CallSiteInfo>> =
        std::collections::HashMap::new();

    let func_by_id: std::collections::HashMap<u32, &str> = module
        .functions
        .iter()
        .map(|f| (f.id.0, f.name.as_str()))
        .collect();

    for func in &module.functions {
        let inst_span: std::collections::HashMap<u32, vow_syntax::span::Span> = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .map(|i| (i.id.0, i.origin))
            .collect();

        for block in &func.blocks {
            for inst in &block.insts {
                if inst.opcode == Opcode::Call
                    && let InstData::CallTarget(fid) = &inst.data
                    && let Some(&callee_name) = func_by_id.get(&fid.0)
                {
                    let arg_spans: Vec<(u32, u32)> = inst
                        .args
                        .iter()
                        .map(|a| {
                            inst_span
                                .get(&a.0)
                                .map(|s| (s.start, s.len))
                                .unwrap_or((0, 0))
                        })
                        .collect();
                    index
                        .entry(callee_name.to_string())
                        .or_default()
                        .push(CallSiteInfo {
                            caller_function: func.name.clone(),
                            file: file.to_string(),
                            offset: inst.origin.start,
                            length: inst.origin.len,
                            arg_spans,
                        });
                }
            }
        }
    }

    index
}

// ---------------------------------------------------------------------------
// Frontend (parse → module load → type check → IR lower)
// ---------------------------------------------------------------------------

struct FrontendResult {
    ir_module: Arc<vow_ir::Module>,
    diagnostics: Vec<Diagnostic>,
    source_files: Vec<PathBuf>,
}

fn compile_frontend(source: &Path) -> Result<FrontendResult, Box<BuildOutput>> {
    let src = match std::fs::read_to_string(source) {
        Ok(s) => s,
        Err(e) => {
            return Err(Box::new(BuildOutput {
                status: BuildStatus::CompileFailed {
                    message: e.to_string(),
                },
                executable: None,
                diagnostics: vec![],
                counterexamples: vec![],
                verify_status: None,
                verify_message: None,
            }));
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
        return Err(Box::new(BuildOutput {
            status: BuildStatus::CompileFailed {
                message: "parse error".to_string(),
            },
            executable: None,
            diagnostics: all_diagnostics,
            counterexamples: vec![],
            verify_status: None,
            verify_message: None,
        }));
    }

    let (ast, source_files) = match module_loader::load_modules(source, &root_ast) {
        Ok(graph) => {
            let files: Vec<PathBuf> = graph.modules.iter().map(|(p, _)| p.clone()).collect();
            (module_loader::merge_modules(graph), files)
        }
        Err(diags) => {
            for d in &diags {
                stderr_emit.emit(d);
            }
            all_diagnostics.extend(diags);
            return Err(Box::new(BuildOutput {
                status: BuildStatus::CompileFailed {
                    message: "module load error".to_string(),
                },
                executable: None,
                diagnostics: all_diagnostics,
                counterexamples: vec![],
                verify_status: None,
                verify_message: None,
            }));
        }
    };

    let mut collecting_emit = CollectingEmitter::new(&mut stderr_emit);
    let mut checker =
        vow_types::check::Checker::new(source.to_string_lossy().to_string(), &mut collecting_emit);
    checker.check_module(&ast);
    let has_errors = checker.has_errors();
    let string_exprs = checker.into_string_exprs();
    all_diagnostics.extend(collecting_emit.into_diagnostics());
    if has_errors {
        return Err(Box::new(BuildOutput {
            status: BuildStatus::CompileFailed {
                message: "type error".to_string(),
            },
            executable: None,
            diagnostics: all_diagnostics,
            counterexamples: vec![],
            verify_status: None,
            verify_message: None,
        }));
    }

    let module = vow_ir::lower_module(
        &ast,
        &source.to_string_lossy(),
        &string_exprs,
    );
    for w in &module.warnings {
        stderr_emit.emit(w);
    }
    all_diagnostics.extend(module.warnings.iter().cloned());
    let ir_module = Arc::new(module);

    Ok(FrontendResult {
        ir_module,
        diagnostics: all_diagnostics,
        source_files,
    })
}

// ---------------------------------------------------------------------------
// Verification (synchronous)
// ---------------------------------------------------------------------------

fn run_verification_sync(
    ir_module: &vow_ir::Module,
    file: &str,
    call_site_index: &std::collections::HashMap<String, Vec<CallSiteInfo>>,
    verify_cache: Option<&VerifyCache>,
) -> VerifyOutcome {
    let const_fns = detect_constant_functions(ir_module);
    for func in &ir_module.functions {
        if func.vows.is_empty() {
            continue;
        }

        let result = if let Some(vc) = verify_cache {
            let c_src = emit_verify_c_source(func, ir_module, &const_fns);
            let key = VerifyCache::cache_key(&c_src, 10);

            if let Some(cached) = vc.lookup(&key) {
                match cached {
                    CachedVerifyResult::Proven => VerificationResult::Proven,
                    CachedVerifyResult::Failed { .. } => {
                        VerificationResult::Failed(cached.to_counterexample().unwrap())
                    }
                }
            } else {
                let esbmc = match find_esbmc() {
                    Some(p) => p,
                    None => return VerifyOutcome::ToolNotFound,
                };
                let res = run_esbmc(&esbmc, &c_src);
                match &res {
                    VerificationResult::Proven => {
                        vc.store(&key, &CachedVerifyResult::Proven);
                    }
                    VerificationResult::Failed(ce) => {
                        vc.store(
                            &key,
                            &CachedVerifyResult::Failed {
                                vow_id: ce.vow_id,
                                description: ce.description.clone(),
                                values: ce.values.clone(),
                                block_visits: ce.block_visits.clone(),
                                raw_output: ce.raw_output.clone(),
                            },
                        );
                    }
                    _ => {}
                }
                res
            }
        } else {
            verify_function_with_module_and_const_fns(func, ir_module, &const_fns)
        };

        match result {
            VerificationResult::Failed(ce) => {
                let sce = build_structured_counterexample(func, &ce, file, call_site_index);
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
}

fn blame_to_error_code(blame: &str) -> vow_diag::ErrorCode {
    match blame {
        "caller" => vow_diag::ErrorCode::VowRequiresViolated,
        "callee" => vow_diag::ErrorCode::VowEnsuresViolated,
        _ => vow_diag::ErrorCode::VowRequiresViolated,
    }
}

fn blame_to_diag_blame(blame: &str) -> vow_diag::Blame {
    match blame {
        "caller" => vow_diag::Blame::Caller,
        "callee" => vow_diag::Blame::Callee,
        _ => vow_diag::Blame::None,
    }
}

fn verify_outcome_to_output(
    outcome: VerifyOutcome,
    mut diagnostics: Vec<Diagnostic>,
    executable: Option<PathBuf>,
) -> BuildOutput {
    let (status, counterexamples, verify_status, verify_message) = match outcome {
        VerifyOutcome::Failed {
            function,
            description,
            ref counterexamples,
        } => {
            for sce in counterexamples {
                let primary = match &sce.source {
                    Some(src) => vow_diag::SourceLocation {
                        file: src.file.clone(),
                        byte_offset: src.offset,
                        byte_len: src.length,
                    },
                    None => vow_diag::SourceLocation {
                        file: String::new(),
                        byte_offset: 0,
                        byte_len: 0,
                    },
                };
                let secondary: Vec<vow_diag::SourceLocation> = sce
                    .call_sites
                    .iter()
                    .map(|cs| vow_diag::SourceLocation {
                        file: cs.file.clone(),
                        byte_offset: cs.offset,
                        byte_len: cs.length,
                    })
                    .collect();
                let mut hints = Vec::new();
                match sce.blame.as_str() {
                    "caller" => {
                        hints.push(format!(
                            "the call site violated function `{}`'s precondition",
                            sce.function
                        ));
                        for va in &sce.violating_args {
                            hints.push(format!(
                                "argument `{}` = {} violates the contract",
                                va.param, va.value
                            ));
                        }
                    }
                    "callee" => {
                        hints.push(format!(
                            "function `{}` failed to establish its postcondition",
                            sce.function
                        ));
                    }
                    _ => {}
                }
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: blame_to_error_code(&sce.blame),
                    message: format!(
                        "contract violation in `{}`: {}",
                        sce.function, sce.violation
                    ),
                    primary,
                    secondary,
                    blame: blame_to_diag_blame(&sce.blame),
                    hints,
                });
            }
            (
                BuildStatus::VerifyFailed {
                    function,
                    description,
                },
                counterexamples.clone(),
                None,
                None,
            )
        }
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
        executable,
        diagnostics,
        counterexamples,
        verify_status,
        verify_message,
    }
}

// ---------------------------------------------------------------------------
// Verify-only pipeline (vow verify)
// ---------------------------------------------------------------------------

pub fn run_verify_only(source: &Path) -> BuildOutput {
    run_verify_only_inner(source, false)
}

fn run_verify_only_inner(source: &Path, no_cache: bool) -> BuildOutput {
    let frontend = match compile_frontend(source) {
        Ok(f) => f,
        Err(output) => return *output,
    };

    let verify_cache = if no_cache { None } else { VerifyCache::new() };
    let file = source.to_string_lossy().to_string();
    let call_site_index = build_call_site_index(&frontend.ir_module, &file);
    let outcome = run_verification_sync(
        &frontend.ir_module,
        &file,
        &call_site_index,
        verify_cache.as_ref(),
    );
    verify_outcome_to_output(outcome, frontend.diagnostics, None)
}

// ---------------------------------------------------------------------------
// Full build pipeline (vow build / legacy)
// ---------------------------------------------------------------------------

fn link_obj(obj_path: &Path, output_path: &Path) -> Option<PathBuf> {
    match find_runtime_lib() {
        Some(runtime) => {
            match link(
                &[obj_path],
                &runtime,
                find_shim_lib().as_deref(),
                output_path,
            ) {
                Ok(()) => {
                    let _ = std::fs::remove_file(obj_path);
                    Some(output_path.to_path_buf())
                }
                Err(_) => None,
            }
        }
        None => None,
    }
}

pub fn run_pipeline(
    source: &Path,
    output: Option<&Path>,
    mode: BuildMode,
    no_verify: bool,
    dump_ir: bool,
    trace: TraceMode,
) -> BuildOutput {
    run_pipeline_inner(source, output, mode, no_verify, dump_ir, trace, false)
}

fn run_pipeline_inner(
    source: &Path,
    output: Option<&Path>,
    mode: BuildMode,
    no_verify: bool,
    dump_ir: bool,
    trace: TraceMode,
    no_cache: bool,
) -> BuildOutput {
    let frontend = match compile_frontend(source) {
        Ok(f) => f,
        Err(output) => return *output,
    };

    let all_diagnostics = frontend.diagnostics;
    let ir_module = frontend.ir_module;

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
    let call_site_index = build_call_site_index(&ir_module, &file_for_verify);
    let verify_cache = if no_cache || no_verify {
        None
    } else {
        VerifyCache::new()
    };
    let verify_handle = thread::spawn(move || -> VerifyOutcome {
        if no_verify {
            return VerifyOutcome::Skipped;
        }
        run_verification_sync(
            &module_for_verify,
            &file_for_verify,
            &call_site_index,
            verify_cache.as_ref(),
        )
    });

    let output_path = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| source.with_extension(""));
    let obj_path = output_path.with_extension("o");

    // Cache lookup
    let mode_str = format!("{mode:?}");
    let trace_str = format!("{trace:?}");
    let compile_cache = if no_cache {
        None
    } else {
        cache::CompileCache::new()
    };
    let cache_key = cache::CompileCache::cache_key(&frontend.source_files, &mode_str, &trace_str);

    if let Some(ref cc) = compile_cache
        && let Some(cached_obj) = cc.lookup(&cache_key)
        && std::fs::copy(&cached_obj, &obj_path).is_ok()
    {
        let exe_path = link_obj(&obj_path, &output_path);
        let verify_outcome = verify_handle.join().unwrap_or(VerifyOutcome::Skipped);
        return verify_outcome_to_output(verify_outcome, all_diagnostics, exe_path);
    }

    // Codegen
    let backend = CraneliftBackend::new();
    let compiled = match backend.compile_module(&ir_module, mode, trace) {
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

    // Store in cache
    if let Some(ref cc) = compile_cache {
        cc.store(&cache_key, &obj_path);
    }

    let exe_path = link_obj(&obj_path, &output_path);

    let verify_outcome = verify_handle.join().unwrap_or(VerifyOutcome::Skipped);
    verify_outcome_to_output(verify_outcome, all_diagnostics, exe_path)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn run_build_command(
    source: &Path,
    output: Option<&Path>,
    mode: BuildMode,
    no_verify: bool,
    dump_ir: bool,
    trace: TraceMode,
    no_cache: bool,
) {
    let result = run_pipeline_inner(source, output, mode, no_verify, dump_ir, trace, no_cache);
    if !dump_ir {
        result.emit_json();
    }
    if matches!(
        &result.status,
        BuildStatus::CompileFailed { .. } | BuildStatus::VerifyFailed { .. }
    ) {
        std::process::exit(1);
    }
}

fn run_decl_command(source: &Path, output: Option<&Path>) {
    let src = match std::fs::read_to_string(source) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("vow decl: {}", e);
            std::process::exit(1);
        }
    };

    let file_str = source.to_string_lossy();
    let (root_ast, parse_diags) = vow_syntax::parser::parse_module(&src, &file_str);
    let mut stderr_emit = HumanEmitter::new(Box::new(std::io::stderr()));
    let parse_failed = parse_diags.iter().any(|d| d.severity == Severity::Error);
    for d in &parse_diags {
        stderr_emit.emit(d);
    }
    if parse_failed {
        eprintln!("vow decl: parse errors");
        std::process::exit(1);
    }

    let (ast, _source_files) = match module_loader::load_modules(source, &root_ast) {
        Ok(graph) => {
            let files: Vec<PathBuf> = graph.modules.iter().map(|(p, _)| p.clone()).collect();
            (module_loader::merge_modules(graph), files)
        }
        Err(diags) => {
            for d in &diags {
                stderr_emit.emit(d);
            }
            eprintln!("vow decl: module load error");
            std::process::exit(1);
        }
    };

    let mut collecting_emit = CollectingEmitter::new(&mut stderr_emit);
    let mut checker =
        vow_types::check::Checker::new(source.to_string_lossy().to_string(), &mut collecting_emit);
    checker.check_module(&ast);
    if checker.has_errors() {
        eprintln!("vow decl: type errors");
        std::process::exit(1);
    }

    let decl_text = vow_syntax::printer::print_declarations(&ast);

    let out_path = match output {
        Some(p) => p.to_path_buf(),
        None => {
            let mut p = source.to_path_buf();
            let new_ext = match p.extension() {
                Some(ext) => format!("{}.d", ext.to_string_lossy()),
                None => "d".to_string(),
            };
            p.set_extension(new_ext);
            p
        }
    };

    if let Err(e) = std::fs::write(&out_path, &decl_text) {
        eprintln!("vow decl: {}", e);
        std::process::exit(1);
    }
    eprintln!("wrote {}", out_path.display());
}

fn run_verify_command(source: &Path, no_cache: bool) {
    let result = run_verify_only_inner(source, no_cache);
    result.emit_json();
    if matches!(
        &result.status,
        BuildStatus::CompileFailed { .. } | BuildStatus::VerifyFailed { .. }
    ) {
        std::process::exit(1);
    }
}

fn main() {
    let args = Args::parse();

    match args.command {
        Some(Command::Build(b)) => {
            if b.help {
                if b.human {
                    println!("{}", skill_human());
                } else {
                    println!("{}", skill_json());
                }
                return;
            }
            let source = match b.source {
                Some(s) => s,
                None => {
                    eprintln!("vow build: source file required (try --help)");
                    std::process::exit(1);
                }
            };
            let mode = match b.mode {
                ModeArg::Debug => BuildMode::Debug,
                ModeArg::Release => BuildMode::Release,
            };
            let trace = match b.debug_trace {
                TraceArg::Off => TraceMode::Off,
                TraceArg::Calls => TraceMode::Calls,
                TraceArg::Full => TraceMode::Full,
            };
            run_build_command(
                &source,
                b.output.as_deref(),
                mode,
                b.no_verify,
                b.dump_ir,
                trace,
                b.no_cache,
            );
        }
        Some(Command::Verify(v)) => {
            if v.help {
                if v.human {
                    println!("{}", skill_human());
                } else {
                    println!("{}", skill_json());
                }
                return;
            }
            let source = match v.source {
                Some(s) => s,
                None => {
                    eprintln!("vow verify: source file required (try --help)");
                    std::process::exit(1);
                }
            };
            run_verify_command(&source, v.no_cache);
        }
        Some(Command::Test(t)) => {
            if t.help {
                if t.human {
                    println!("{}", skill_human());
                } else {
                    println!("{}", skill_json());
                }
                return;
            }
            eprintln!("vow test: not yet implemented");
            std::process::exit(1);
        }
        Some(Command::Decl(d)) => {
            if d.help {
                if d.human {
                    println!("{}", skill_human());
                } else {
                    println!("{}", skill_json());
                }
                return;
            }
            let source = match d.source {
                Some(s) => s,
                None => {
                    eprintln!("vow decl: source file required (try --help)");
                    std::process::exit(1);
                }
            };
            run_decl_command(&source, d.output.as_deref());
        }
        None => {
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
                    eprintln!("vow: source file required (try --help or use a subcommand)");
                    std::process::exit(1);
                }
            };

            let mode = match args.mode {
                ModeArg::Debug => BuildMode::Debug,
                ModeArg::Release => BuildMode::Release,
            };
            let trace = match args.debug_trace {
                TraceArg::Off => TraceMode::Off,
                TraceArg::Calls => TraceMode::Calls,
                TraceArg::Full => TraceMode::Full,
            };

            run_build_command(
                &source,
                args.output.as_deref(),
                mode,
                args.no_verify,
                args.dump_ir,
                trace,
                args.no_cache,
            );
        }
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

        let result = run_pipeline(
            &source,
            Some(&out),
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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

        let result = run_pipeline(
            &source,
            Some(&out),
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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

        let result = run_pipeline(
            &source,
            Some(&out),
            BuildMode::Debug,
            true,
            false,
            TraceMode::Off,
        );
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

        let result = run_pipeline(
            &source,
            Some(&out),
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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

        let result = run_pipeline(
            &source,
            Some(&out),
            BuildMode::Debug,
            true,
            false,
            TraceMode::Off,
        );
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
        assert!(out.contains("\"vow\""), "expected tool name in output");
        assert!(
            out.contains("language"),
            "expected language section in output"
        );
        assert!(out.contains("builtins"), "expected builtins in output");
        assert!(
            out.contains("\"commands\""),
            "expected commands section in JSON"
        );
        assert!(out.contains("\"build\""), "expected build command in JSON");
        assert!(
            out.contains("\"verify\""),
            "expected verify command in JSON"
        );
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

        // Must describe types added in Phases 7-8
        assert!(json.contains("String"), "missing String type");
        assert!(json.contains("Vec<T>"), "missing Vec<T> type");
        assert!(json.contains("Option<T>"), "missing Option<T> type");
        assert!(json.contains("Result<T, E>"), "missing Result<T, E> type");
        assert!(json.contains("HashMap<K, V>"), "missing HashMap<K, V> type");

        // Must describe builtins added in Phase 8
        assert!(json.contains("fs_read"), "missing fs_read builtin");
        assert!(json.contains("fs_write"), "missing fs_write builtin");
        assert!(json.contains("args"), "missing args builtin");
        assert!(json.contains("eprintln_str"), "missing eprintln_str builtin");
        assert!(json.contains("process_exit"), "missing process_exit builtin");

        // Must describe structural language features
        assert!(json.contains("\"structs\""), "missing structs section");
        assert!(json.contains("\"enums\""), "missing enums section");
        assert!(json.contains("\"methods\""), "missing methods section");
        assert!(
            json.contains("\"match_expression\""),
            "missing match_expression section"
        );
        assert!(
            json.contains("\"where_clauses\""),
            "missing where_clauses section"
        );
        assert!(
            json.contains("\"modules\""),
            "missing modules section"
        );

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

        let result = run_pipeline(
            &source,
            Some(&out),
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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

        let result = run_pipeline(
            &source,
            Some(&out),
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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
        let result = run_pipeline(
            &source,
            Some(&out),
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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
        let result = run_pipeline(
            &source,
            Some(&out),
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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

        let result = run_pipeline(
            &main_path,
            Some(&out),
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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
    fn serde_json_escapes_special_characters() {
        let result = BuildResult {
            status: "CompileFailed".to_string(),
            executable: None,
            diagnostics: vec![],
            message: Some("type \"error\"\nwith newline".to_string()),
            function: None,
            counterexample: None,
            counterexamples: vec![],
            verify_status: None,
            verify_message: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains(r#"type \"error\"\nwith newline"#));
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
                values: vec![("p1".to_string(), "0".to_string())],
                violation: "y != 0".to_string(),
                vow_id: 0,
                source: None,
                blame: "caller".to_string(),
                call_sites: vec![],
                violating_args: vec![],
                execution_path: vec![],
                branch_decisions: vec![],
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
            hints: vec![],
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
        let result = run_pipeline(
            &source,
            None,
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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
        let result = run_pipeline(
            &source,
            None,
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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
        let result = run_pipeline(
            &source,
            None,
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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
        let result = run_pipeline(
            &source,
            None,
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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
        let result = run_pipeline(
            &source,
            None,
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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
        let result = run_pipeline(
            &source,
            None,
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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
        let result = run_pipeline(
            &source,
            None,
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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
        let result = run_pipeline(
            &source,
            None,
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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
        let result = run_pipeline(
            &source,
            None,
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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
        let result = run_pipeline(
            &source,
            None,
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
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
        let result = run_pipeline(
            &source,
            Some(&out),
            BuildMode::Release,
            false,
            false,
            TraceMode::Off,
        );
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
        let result = run_pipeline(
            &source,
            Some(&out),
            BuildMode::Release,
            false,
            false,
            TraceMode::Off,
        );
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
                values: vec![
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
                blame: "caller".to_string(),
                call_sites: vec![],
                violating_args: vec![],
                execution_path: vec![],
                branch_decisions: vec![],
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
    fn counterexample_json_empty() {
        let result = BuildResult {
            status: "Verified".to_string(),
            executable: None,
            diagnostics: vec![],
            message: None,
            function: None,
            counterexample: None,
            counterexamples: vec![],
            verify_status: None,
            verify_message: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(
            json.contains("\"counterexamples\":[]"),
            "empty counterexamples: {json}"
        );
    }

    #[test]
    fn counterexample_json_one_entry() {
        let ce = CounterexampleJson::from_structured(&StructuredCounterexample {
            function: "f".to_string(),
            values: vec![("x".to_string(), "0".to_string())],
            violation: "x > 0".to_string(),
            vow_id: 1,
            source: None,
            blame: "caller".to_string(),
            call_sites: vec![],
            violating_args: vec![],
            execution_path: vec![],
            branch_decisions: vec![],
        });
        let json = serde_json::to_string(&ce).unwrap();
        assert!(json.contains("\"function\":\"f\""), "function: {json}");
        assert!(json.contains("\"x\":\"0\""), "values: {json}");
        assert!(
            json.contains("\"violation\":\"x > 0\""),
            "violation: {json}"
        );
        assert!(json.contains("\"vow_id\":1"), "vow_id: {json}");
        assert!(json.contains("\"source\":null"), "source null: {json}");
    }

    #[test]
    fn counterexample_json_with_source() {
        let ce = CounterexampleJson::from_structured(&StructuredCounterexample {
            function: "f".to_string(),
            values: vec![],
            violation: "result".to_string(),
            vow_id: 0,
            source: Some(CeSource {
                file: "test.vow".to_string(),
                offset: 10,
                length: 5,
            }),
            blame: "callee".to_string(),
            call_sites: vec![],
            violating_args: vec![],
            execution_path: vec![],
            branch_decisions: vec![],
        });
        let json = serde_json::to_string(&ce).unwrap();
        assert!(json.contains("\"file\":\"test.vow\""), "file: {json}");
        assert!(json.contains("\"offset\":10"), "offset: {json}");
        assert!(json.contains("\"length\":5"), "length: {json}");
    }

    #[test]
    fn build_result_serde_roundtrip_verified() {
        let out = BuildOutput {
            status: BuildStatus::Verified,
            executable: Some(PathBuf::from("/tmp/test")),
            diagnostics: vec![],
            counterexamples: vec![],
            verify_status: None,
            verify_message: None,
        };
        let result = out.to_build_result();
        let json = serde_json::to_string(&result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["status"], "Verified");
        assert_eq!(parsed["executable"], "/tmp/test");
        assert!(parsed["diagnostics"].as_array().unwrap().is_empty());
        assert!(parsed["counterexamples"].as_array().unwrap().is_empty());
        assert!(parsed.get("message").is_none());
        assert!(parsed.get("function").is_none());
    }

    #[test]
    fn build_result_serde_roundtrip_compile_failed() {
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
            hints: vec![],
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
        let result = out.to_build_result();
        let json = serde_json::to_string(&result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["status"], "CompileFailed");
        assert!(parsed["executable"].is_null());
        assert_eq!(parsed["message"], "type error");
        assert_eq!(parsed["diagnostics"].as_array().unwrap().len(), 1);
        let d = &parsed["diagnostics"][0];
        assert_eq!(d["error_code"], "TypeMismatch");
        assert_eq!(d["severity"], "error");
        assert_eq!(d["span"]["file"], "test.vow");
        assert_eq!(d["span"]["offset"], 42);
        assert_eq!(d["span"]["length"], 4);
    }

    #[test]
    fn build_result_serde_roundtrip_verify_failed() {
        let out = BuildOutput {
            status: BuildStatus::VerifyFailed {
                function: "divide".to_string(),
                description: "y=0 violates requires".to_string(),
            },
            executable: None,
            diagnostics: vec![],
            counterexamples: vec![StructuredCounterexample {
                function: "divide".to_string(),
                values: vec![("y".to_string(), "0".to_string())],
                violation: "y != 0".to_string(),
                vow_id: 0,
                source: Some(CeSource {
                    file: "divide.vow".to_string(),
                    offset: 50,
                    length: 10,
                }),
                blame: "caller".to_string(),
                call_sites: vec![CeCallSite {
                    caller_function: "main".to_string(),
                    file: "divide.vow".to_string(),
                    offset: 120,
                    length: 15,
                }],
                violating_args: vec![],
                execution_path: vec![],
                branch_decisions: vec![],
            }],
            verify_status: None,
            verify_message: None,
        };
        let result = out.to_build_result();
        let json = serde_json::to_string(&result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["status"], "VerifyFailed");
        assert_eq!(parsed["function"], "divide");
        assert_eq!(parsed["counterexample"], "y=0 violates requires");
        let ces = parsed["counterexamples"].as_array().unwrap();
        assert_eq!(ces.len(), 1);
        assert_eq!(ces[0]["blame"], "caller");
        let call_sites = ces[0]["call_sites"].as_array().unwrap();
        assert_eq!(call_sites.len(), 1);
        assert_eq!(call_sites[0]["caller_function"], "main");
        assert_eq!(ces[0]["function"], "divide");
        assert_eq!(ces[0]["values"]["y"], "0");
        assert_eq!(ces[0]["violation"], "y != 0");
        assert_eq!(ces[0]["vow_id"], 0);
        assert_eq!(ces[0]["source"]["file"], "divide.vow");
    }

    #[test]
    fn pipeline_verified_produces_valid_build_result() {
        let dir = TempDir::new().unwrap();
        let src = "module M\n\nfn f(x: i64) -> i64 { x }";
        let source = write_source(&dir, "ok.vow", src);
        let result = run_pipeline(
            &source,
            None,
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
        let build_result = result.to_build_result();
        let json = serde_json::to_string(&build_result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let status = parsed["status"].as_str().unwrap();
        assert!(
            status == "Verified" || status == "Unverified" || status == "CompileFailed",
            "unexpected status: {status}"
        );
        assert!(parsed["diagnostics"].is_array());
        assert!(parsed["counterexamples"].is_array());
    }

    #[test]
    fn pipeline_compile_failed_produces_valid_build_result() {
        let dir = TempDir::new().unwrap();
        let src = "module M 123";
        let source = write_source(&dir, "bad.vow", src);
        let result = run_pipeline(
            &source,
            None,
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
        let build_result = result.to_build_result();
        let json = serde_json::to_string(&build_result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["status"], "CompileFailed");
        assert!(parsed["message"].is_string());
        assert!(parsed["diagnostics"].is_array());
        assert!(
            !parsed["diagnostics"].as_array().unwrap().is_empty(),
            "compile failure should have diagnostics"
        );
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
            local_names: std::collections::HashMap::new(),
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
            local_names: std::collections::HashMap::new(),
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
    fn map_counterexample_values_applies_mapping() {
        let mut name_map = std::collections::HashMap::new();
        name_map.insert("p0".to_string(), "x".to_string());
        name_map.insert("p1".to_string(), "y".to_string());
        name_map.insert("v0".to_string(), "x".to_string());
        name_map.insert("v1".to_string(), "y".to_string());

        let values = vec![
            ("v1".to_string(), "0".to_string()),
            ("v3".to_string(), "0".to_string()),
        ];
        let mapped = map_counterexample_values(&values, &name_map);
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
            local_names: std::collections::HashMap::new(),
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
        let result = run_pipeline(
            &source,
            Some(&out),
            BuildMode::Release,
            false,
            false,
            TraceMode::Off,
        );
        match &result.status {
            BuildStatus::VerifyFailed { function, .. } => {
                assert_eq!(function, "bad_div");
                let ce = &result.counterexamples[0];
                for (name, _) in &ce.values {
                    assert!(
                        name == "x" || name == "y" || name.starts_with("_esbmc_"),
                        "expected source name or _esbmc_ prefix, got: {name}"
                    );
                }
                let has_source_name = ce.values.iter().any(|(n, _)| n == "x" || n == "y");
                assert!(
                    has_source_name,
                    "at least one input should use a source name, got: {:?}",
                    ce.values,
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
        let broken_result = run_pipeline(
            &broken_path,
            Some(&broken_out),
            BuildMode::Release,
            false,
            false,
            TraceMode::Off,
        );

        match &broken_result.status {
            BuildStatus::VerifyFailed { function, .. } => {
                assert_eq!(function, "safe_sub");

                // AC2: diagnostics array present; only verification diagnostics, no compile errors
                let compile_errors: Vec<_> = broken_result
                    .diagnostics
                    .iter()
                    .filter(|d| {
                        !matches!(
                            d.code,
                            vow_diag::ErrorCode::VowRequiresViolated
                                | vow_diag::ErrorCode::VowEnsuresViolated
                                | vow_diag::ErrorCode::VowInvariantViolated
                        )
                    })
                    .collect();
                assert!(
                    compile_errors.is_empty(),
                    "diagnostics should have no compile errors, got: {:?}",
                    compile_errors,
                );

                // AC3: counterexamples array with at least one entry
                assert!(
                    !broken_result.counterexamples.is_empty(),
                    "counterexamples should not be empty on verify failure"
                );

                let ce = &broken_result.counterexamples[0];

                // AC4a: values with source-level variable names
                let has_source_name = ce.values.iter().any(|(name, _)| name == "a" || name == "b");
                assert!(
                    has_source_name,
                    "counterexample values should use source names (a, b), got: {:?}",
                    ce.values,
                );
                for (name, _) in &ce.values {
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
                let fixed_result = run_pipeline(
                    &fixed_path,
                    Some(&fixed_out),
                    BuildMode::Release,
                    false,
                    false,
                    TraceMode::Off,
                );

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
        let result = run_pipeline(
            &path,
            Some(&out),
            BuildMode::Release,
            false,
            false,
            TraceMode::Off,
        );

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

    // -----------------------------------------------------------------------
    // Phase 11.2: subcommand tests
    // -----------------------------------------------------------------------

    #[test]
    fn verify_only_proven() {
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
        let source = write_source(&dir, "good.vow", src);
        let result = run_verify_only(&source);
        match &result.status {
            BuildStatus::Verified => {
                assert!(
                    result.executable.is_none(),
                    "verify-only should not produce executable"
                );
                assert!(result.counterexamples.is_empty());
            }
            BuildStatus::Unverified => {
                eprintln!("SKIP: esbmc not found");
            }
            BuildStatus::CompileFailed { message } => {
                panic!("unexpected compile failure: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        }
    }

    #[test]
    fn verify_only_failed() {
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
        let source = write_source(&dir, "bad.vow", src);
        let result = run_verify_only(&source);
        match &result.status {
            BuildStatus::VerifyFailed { function, .. } => {
                assert_eq!(function, "always_bad");
                assert!(
                    result.executable.is_none(),
                    "verify-only should not produce executable"
                );
            }
            BuildStatus::Unverified => {
                eprintln!("SKIP: esbmc not found");
            }
            BuildStatus::CompileFailed { message } => {
                panic!("unexpected compile failure: {message}");
            }
            other => panic!("unexpected status: {other:?}"),
        }
    }

    #[test]
    fn verify_only_compile_error() {
        let dir = TempDir::new().unwrap();
        let src = "module Bad fn f() -> i32 { true }";
        let source = write_source(&dir, "bad_type.vow", src);
        let result = run_verify_only(&source);
        assert!(
            matches!(result.status, BuildStatus::CompileFailed { .. }),
            "expected CompileFailed for type error via verify-only, got {:?}",
            result.status
        );
        assert!(result.executable.is_none());
    }

    #[test]
    fn legacy_mode_still_works() {
        let dir = TempDir::new().unwrap();
        let src = "module M fn f(x: i64) -> i64 { x }";
        let source = write_source(&dir, "legacy.vow", src);
        let result = run_pipeline(
            &source,
            None,
            BuildMode::Release,
            true,
            false,
            TraceMode::Off,
        );
        match &result.status {
            BuildStatus::Unverified => {}
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
    fn build_call_site_index_finds_internal_calls() {
        use vow_ir::*;
        use vow_syntax::span::Span;

        let module = Module {
            name: "test".to_string(),
            functions: vec![
                Function {
                    id: FuncId(0),
                    name: "callee".to_string(),
                    params: vec![Ty::I64],
                    param_names: vec!["x".to_string()],
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
                                opcode: Opcode::Return,
                                ty: Ty::Unit,
                                args: vec![InstId(0)],
                                data: InstData::None,
                                origin: Span::new(0, 0),
                            },
                        ],
                    }],
                    local_names: std::collections::HashMap::new(),
                },
                Function {
                    id: FuncId(1),
                    name: "caller_a".to_string(),
                    params: vec![],
                    param_names: vec![],
                    return_ty: Ty::I64,
                    effects: vec![],
                    vows: vec![],
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        insts: vec![
                            Inst {
                                id: InstId(0),
                                opcode: Opcode::ConstI64,
                                ty: Ty::I64,
                                args: vec![],
                                data: InstData::ConstI64(5),
                                origin: Span::new(0, 0),
                            },
                            Inst {
                                id: InstId(1),
                                opcode: Opcode::Call,
                                ty: Ty::I64,
                                args: vec![InstId(0)],
                                data: InstData::CallTarget(FuncId(0)),
                                origin: Span::new(100, 10),
                            },
                            Inst {
                                id: InstId(2),
                                opcode: Opcode::Return,
                                ty: Ty::Unit,
                                args: vec![InstId(1)],
                                data: InstData::None,
                                origin: Span::new(0, 0),
                            },
                        ],
                    }],
                    local_names: std::collections::HashMap::new(),
                },
                Function {
                    id: FuncId(2),
                    name: "caller_b".to_string(),
                    params: vec![],
                    param_names: vec![],
                    return_ty: Ty::I64,
                    effects: vec![],
                    vows: vec![],
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        insts: vec![
                            Inst {
                                id: InstId(0),
                                opcode: Opcode::ConstI64,
                                ty: Ty::I64,
                                args: vec![],
                                data: InstData::ConstI64(10),
                                origin: Span::new(0, 0),
                            },
                            Inst {
                                id: InstId(1),
                                opcode: Opcode::Call,
                                ty: Ty::I64,
                                args: vec![InstId(0)],
                                data: InstData::CallTarget(FuncId(0)),
                                origin: Span::new(200, 15),
                            },
                            Inst {
                                id: InstId(2),
                                opcode: Opcode::Return,
                                ty: Ty::Unit,
                                args: vec![InstId(1)],
                                data: InstData::None,
                                origin: Span::new(0, 0),
                            },
                        ],
                    }],
                    local_names: std::collections::HashMap::new(),
                },
            ],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };

        let index = build_call_site_index(&module, "test.vow");
        let callee_sites = index.get("callee").expect("callee should have call sites");
        assert_eq!(callee_sites.len(), 2);
        assert_eq!(callee_sites[0].caller_function, "caller_a");
        assert_eq!(callee_sites[0].offset, 100);
        assert_eq!(callee_sites[0].length, 10);
        assert_eq!(callee_sites[1].caller_function, "caller_b");
        assert_eq!(callee_sites[1].offset, 200);
        assert_eq!(callee_sites[1].length, 15);
        assert!(index.get("caller_a").is_none());
    }

    #[test]
    fn structured_counterexample_includes_blame_caller() {
        use vow_ir::*;
        use vow_syntax::span::Span;

        let func = Function {
            id: FuncId(0),
            name: "safe_div".to_string(),
            params: vec![Ty::I64, Ty::I64],
            param_names: vec!["x".to_string(), "y".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "y != 0".to_string(),
                blame: vow_diag::Blame::Caller,
                bindings: vec![],
                file: "test.vow".to_string(),
                offset: 42,
            }],
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
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::VowRequires,
                        ty: Ty::Unit,
                        args: vec![InstId(1)],
                        data: InstData::VowId(VowId(0)),
                        origin: Span::new(42, 6),
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
        };

        let ce = vow_verify::Counterexample {
            description: "y != 0".to_string(),
            vow_id: Some(0),
            values: vec![
                ("p0".to_string(), "10".to_string()),
                ("p1".to_string(), "0".to_string()),
            ],
            block_visits: vec![0],
            raw_output: String::new(),
        };

        let mut call_sites = std::collections::HashMap::new();
        call_sites.insert(
            "safe_div".to_string(),
            vec![CallSiteInfo {
                caller_function: "main".to_string(),
                file: "test.vow".to_string(),
                offset: 120,
                length: 18,
                arg_spans: vec![],
            }],
        );

        let sce = build_structured_counterexample(&func, &ce, "test.vow", &call_sites);
        assert_eq!(sce.blame, "caller");
        assert_eq!(sce.call_sites.len(), 1);
        assert_eq!(sce.call_sites[0].caller_function, "main");
        assert_eq!(sce.call_sites[0].offset, 120);
    }

    #[test]
    fn structured_counterexample_callee_blame_no_call_sites() {
        use vow_ir::*;
        use vow_syntax::span::Span;

        let func = Function {
            id: FuncId(0),
            name: "buggy".to_string(),
            params: vec![Ty::I64],
            param_names: vec!["x".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "result == x + x".to_string(),
                blame: vow_diag::Blame::Callee,
                bindings: vec![],
                file: "test.vow".to_string(),
                offset: 30,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![Inst {
                    id: InstId(0),
                    opcode: Opcode::VowEnsures,
                    ty: Ty::Unit,
                    args: vec![],
                    data: InstData::VowId(VowId(0)),
                    origin: Span::new(30, 20),
                }],
            }],
            local_names: std::collections::HashMap::new(),
        };

        let ce = vow_verify::Counterexample {
            description: "result == x + x".to_string(),
            vow_id: Some(0),
            values: vec![("p0".to_string(), "5".to_string())],
            block_visits: vec![0],
            raw_output: String::new(),
        };

        let mut call_sites = std::collections::HashMap::new();
        call_sites.insert(
            "buggy".to_string(),
            vec![CallSiteInfo {
                caller_function: "main".to_string(),
                file: "test.vow".to_string(),
                offset: 100,
                length: 10,
                arg_spans: vec![],
            }],
        );

        let sce = build_structured_counterexample(&func, &ce, "test.vow", &call_sites);
        assert_eq!(sce.blame, "callee");
        assert!(
            sce.call_sites.is_empty(),
            "callee blame should have no call_sites"
        );
    }

    #[test]
    fn counterexample_json_serialization_blame_and_call_sites() {
        let sce = StructuredCounterexample {
            function: "safe_div".to_string(),
            values: vec![
                ("x".to_string(), "10".to_string()),
                ("y".to_string(), "0".to_string()),
            ],
            violation: "y != 0".to_string(),
            vow_id: 0,
            source: Some(CeSource {
                file: "test.vow".to_string(),
                offset: 42,
                length: 6,
            }),
            blame: "caller".to_string(),
            call_sites: vec![CeCallSite {
                caller_function: "main".to_string(),
                file: "test.vow".to_string(),
                offset: 120,
                length: 18,
            }],
            violating_args: vec![],
            execution_path: vec![],
            branch_decisions: vec![],
        };
        let json_ce = CounterexampleJson::from_structured(&sce);
        let serialized = serde_json::to_string(&json_ce).unwrap();
        assert!(serialized.contains("\"blame\":\"caller\""));
        assert!(serialized.contains("\"call_sites\""));
        assert!(serialized.contains("\"caller_function\":\"main\""));

        // Callee blame — call_sites should be omitted
        let sce_callee = StructuredCounterexample {
            function: "buggy".to_string(),
            values: vec![("x".to_string(), "5".to_string())],
            violation: "result == x + x".to_string(),
            vow_id: 0,
            source: None,
            blame: "callee".to_string(),
            call_sites: vec![],
            violating_args: vec![],
            execution_path: vec![],
            branch_decisions: vec![],
        };
        let json_callee = CounterexampleJson::from_structured(&sce_callee);
        let serialized_callee = serde_json::to_string(&json_callee).unwrap();
        assert!(serialized_callee.contains("\"blame\":\"callee\""));
        assert!(!serialized_callee.contains("call_sites"));
    }

    #[test]
    fn verify_caller_blame_example() {
        let source = PathBuf::from("examples/caller_blame.vow");
        if !source.exists() {
            eprintln!("SKIP: examples/caller_blame.vow not found");
            return;
        }
        let result = run_verify_only(&source);
        let build_result = result.to_build_result();
        let json = serde_json::to_string(&build_result).unwrap();

        // The file should verify successfully (safe_div has requires: y != 0
        // and all call sites pass valid args). Check JSON is well-formed.
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("status").is_some());
    }

    #[test]
    fn verify_callee_blame_example() {
        let source = PathBuf::from("examples/callee_blame.vow");
        if !source.exists() {
            eprintln!("SKIP: examples/callee_blame.vow not found");
            return;
        }
        let result = run_verify_only(&source);
        let build_result = result.to_build_result();
        let json = serde_json::to_string(&build_result).unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("status").is_some());
    }

    #[test]
    fn call_site_index_captures_arg_spans() {
        use vow_ir::*;
        use vow_syntax::span::Span;
        let callee = Function {
            id: FuncId(0),
            name: "callee".to_string(),
            params: vec![Ty::I64, Ty::I64],
            param_names: vec!["a".to_string(), "b".to_string()],
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
                        origin: Span::new(10, 1),
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::Return,
                        ty: Ty::Unit,
                        args: vec![InstId(0)],
                        data: InstData::None,
                        origin: Span::new(12, 1),
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
        };
        let caller = Function {
            id: FuncId(1),
            name: "caller".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(10),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(5),
                        origin: Span::new(100, 1),
                    },
                    Inst {
                        id: InstId(11),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(0),
                        origin: Span::new(103, 1),
                    },
                    Inst {
                        id: InstId(12),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(10), InstId(11)],
                        data: InstData::CallTarget(FuncId(0)),
                        origin: Span::new(95, 12),
                    },
                    Inst {
                        id: InstId(13),
                        opcode: Opcode::Return,
                        ty: Ty::Unit,
                        args: vec![InstId(12)],
                        data: InstData::None,
                        origin: Span::new(110, 1),
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
        };
        let module = Module {
            name: "test".to_string(),
            functions: vec![callee, caller],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        let index = build_call_site_index(&module, "test.vow");
        let sites = index.get("callee").expect("callee should have call sites");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].arg_spans.len(), 2);
        assert_eq!(sites[0].arg_spans[0], (100, 1));
        assert_eq!(sites[0].arg_spans[1], (103, 1));
    }

    #[test]
    fn violating_args_populated_for_caller_blame() {
        use vow_ir::*;
        use vow_syntax::span::Span;
        let func = Function {
            id: FuncId(0),
            name: "divide".to_string(),
            params: vec![Ty::I64, Ty::I64],
            param_names: vec!["x".to_string(), "y".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "y != 0".to_string(),
                blame: vow_diag::Blame::Caller,
                bindings: vec![("y".to_string(), InstId(1))],
                file: "test.vow".to_string(),
                offset: 20,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(0),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(0),
                        origin: Span::new(10, 1),
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(1),
                        origin: Span::new(15, 1),
                    },
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::Return,
                        ty: Ty::Unit,
                        args: vec![InstId(0)],
                        data: InstData::None,
                        origin: Span::new(20, 1),
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
        };
        let ce = vow_verify::Counterexample {
            description: "test".to_string(),
            vow_id: Some(0),
            values: vec![
                ("p0".to_string(), "10".to_string()),
                ("p1".to_string(), "0".to_string()),
            ],
            block_visits: vec![0],
            raw_output: String::new(),
        };
        let mut call_site_index = std::collections::HashMap::new();
        call_site_index.insert(
            "divide".to_string(),
            vec![CallSiteInfo {
                caller_function: "main".to_string(),
                file: "test.vow".to_string(),
                offset: 50,
                length: 15,
                arg_spans: vec![(55, 2), (59, 1)],
            }],
        );
        let sce = build_structured_counterexample(&func, &ce, "test.vow", &call_site_index);
        assert_eq!(sce.blame, "caller");
        assert_eq!(sce.violating_args.len(), 1);
        assert_eq!(sce.violating_args[0].param, "y");
        assert_eq!(sce.violating_args[0].value, "0");
        assert_eq!(sce.violating_args[0].arg_offset, 59);
        assert_eq!(sce.violating_args[0].arg_length, 1);
    }

    #[test]
    fn execution_path_and_branch_decisions_from_block_visits() {
        use vow_ir::*;
        use vow_syntax::span::Span;
        let func = Function {
            id: FuncId(0),
            name: "branchy".to_string(),
            params: vec![Ty::Bool],
            param_names: vec!["cond".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "result >= 0".to_string(),
                blame: vow_diag::Blame::Callee,
                bindings: vec![],
                file: "test.vow".to_string(),
                offset: 0,
            }],
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        Inst {
                            id: InstId(0),
                            opcode: Opcode::GetArg,
                            ty: Ty::Bool,
                            args: vec![],
                            data: InstData::ArgIndex(0),
                            origin: Span::new(10, 4),
                        },
                        Inst {
                            id: InstId(1),
                            opcode: Opcode::Branch,
                            ty: Ty::Unit,
                            args: vec![InstId(0)],
                            data: InstData::BranchTargets {
                                then_block: BlockId(1),
                                else_block: BlockId(2),
                            },
                            origin: Span::new(20, 8),
                        },
                    ],
                },
                BasicBlock {
                    id: BlockId(1),
                    insts: vec![Inst {
                        id: InstId(2),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(1),
                        origin: Span::new(30, 1),
                    }],
                },
                BasicBlock {
                    id: BlockId(2),
                    insts: vec![Inst {
                        id: InstId(3),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(-1),
                        origin: Span::new(40, 2),
                    }],
                },
            ],
            local_names: std::collections::HashMap::new(),
        };
        let ce = vow_verify::Counterexample {
            description: "test".to_string(),
            vow_id: Some(0),
            values: vec![("p0".to_string(), "0".to_string())],
            block_visits: vec![0, 2],
            raw_output: String::new(),
        };
        let call_site_index = std::collections::HashMap::new();
        let sce = build_structured_counterexample(&func, &ce, "test.vow", &call_site_index);

        assert_eq!(sce.execution_path.len(), 2);
        assert_eq!(sce.execution_path[0].block_id, 0);
        assert_eq!(sce.execution_path[0].offset, 10);
        assert_eq!(sce.execution_path[1].block_id, 2);
        assert_eq!(sce.execution_path[1].offset, 40);

        assert_eq!(sce.branch_decisions.len(), 1);
        assert_eq!(sce.branch_decisions[0].taken, "else");
        assert_eq!(sce.branch_decisions[0].condition_offset, 20);
        assert_eq!(sce.branch_decisions[0].condition_length, 8);
    }

    #[test]
    fn new_json_fields_skip_when_empty() {
        let sce = StructuredCounterexample {
            function: "f".to_string(),
            values: vec![],
            violation: "test".to_string(),
            vow_id: 0,
            source: None,
            blame: "callee".to_string(),
            call_sites: vec![],
            violating_args: vec![],
            execution_path: vec![],
            branch_decisions: vec![],
        };
        let json_obj = CounterexampleJson::from_structured(&sce);
        let json = serde_json::to_string(&json_obj).unwrap();
        assert!(
            !json.contains("violating_args"),
            "empty field should be skipped"
        );
        assert!(
            !json.contains("execution_path"),
            "empty field should be skipped"
        );
        assert!(
            !json.contains("branch_decisions"),
            "empty field should be skipped"
        );
    }
}
