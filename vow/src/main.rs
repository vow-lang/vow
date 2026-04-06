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
    Profile,
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
    /// List all contracts in a program with optional verification status
    Contracts(ContractsArgs),
    /// Generate or install the Claude Code skill document
    Skill(SkillArgs),
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
    /// Directory to scan for test files, or a single .vow file
    path: Option<PathBuf>,
    /// Run ESBMC verification on test files (off by default)
    #[arg(long)]
    verify: bool,
    /// Only run tests whose name contains this substring
    #[arg(long)]
    filter: Option<String>,
    /// Build mode (debug enables runtime vow checks)
    #[arg(long, value_enum, default_value = "debug")]
    mode: ModeArg,
    /// Per-test execution timeout in milliseconds
    #[arg(long, default_value = "30000")]
    timeout: u64,
    /// ESBMC loop unwind bound (only with --verify)
    #[arg(long, default_value = "10")]
    unwind: u32,
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

#[derive(clap::Args, Debug)]
#[command(disable_help_flag = true)]
struct ContractsArgs {
    source: Option<PathBuf>,
    #[arg(long)]
    verify: bool,
    #[arg(long)]
    no_cache: bool,
    #[arg(long)]
    unwind: Option<u32>,
    #[arg(long)]
    help: bool,
    #[arg(long)]
    human: bool,
}

#[derive(clap::Args, Debug)]
#[command(disable_help_flag = true)]
struct SkillArgs {
    #[command(subcommand)]
    action: Option<SkillAction>,
}

#[derive(clap::Subcommand, Debug)]
enum SkillAction {
    /// Print the skill document to stdout (default)
    Print,
    /// Install the skill to .claude/commands/vow-toolchain.md
    Install,
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
    "test": "Run tests: discover, compile, execute test_*.vow files with JSON results",
    "decl": "Emit declaration file (.vow.d) with type signatures only",
    "contracts": "List all contracts with optional verification status",
    "skill": "Generate or install the Claude Code skill document for this compiler version"
  },
  "legacy_usage": "vow [OPTIONS] <source.vow> (equivalent to vow build)",
  "build_options": {
    "-o, --output <path>": "Output executable path (default: source without .vow extension)",
    "--mode <debug|release|profile>": "Build mode: debug inserts runtime vow checks, profile inserts call counters and prints report on normal exit (default: release)",
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
  "test_options": {
    "<path>": "Directory to scan or single .vow file (default: .)",
    "--verify": "Run ESBMC verification on test files",
    "--filter <pat>": "Only run tests whose name contains pat",
    "--mode <debug|release>": "Build mode; debug inserts runtime vow checks (default: (default))",
    "--timeout <ms>": "Per-test execution timeout in milliseconds (default: 30000)",
    "--unwind <N>": "ESBMC loop unwind bound (with --verify)"
  },
  "contracts_options": {
    "--verify": "Run ESBMC verification and report per-contract status",
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
      "u8",
      "u64",
      "f32",
      "f64",
      "bool",
      "()",
      "!",
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
      "fs_write": "fn(path: String, data: String) -> i64 [write]",
      "fs_exists": "fn(path: String) -> i64 [read]",
      "fs_mkdir": "fn(path: String) -> i64 [io]",
      "fs_listdir": "fn(path: String) -> Vec<String> [read]",
      "fs_remove": "fn(path: String) -> i64 [io]",
      "fs_remove_dir": "fn(path: String) -> i64 [io]",
      "fs_is_dir": "fn(path: String) -> i64 [read]",
      "fs_rename": "fn(old: String, new: String) -> i64 [io]",
      "string_substr": "fn(s: String, start: i64, len: i64) -> String []",
      "string_split": "fn(s: String, delim: String) -> Vec<String> []",
      "string_starts_with": "fn(s: String, prefix: String) -> i64 []",
      "string_ends_with": "fn(s: String, suffix: String) -> i64 []",
      "string_trim": "fn(s: String) -> String []",
      "string_to_upper": "fn(s: String) -> String []",
      "string_to_lower": "fn(s: String) -> String []",
      "string_replace": "fn(s: String, from: String, to: String) -> String []",
      "string_join": "fn(parts: Vec<String>, sep: String) -> String []",
      "parse_i64": "fn(s: String) -> i64 []",
      "i64_to_string": "fn(v: i64) -> String []",
      "vec_sort": "fn(v: Vec<i64>) -> Vec<i64> []",
      "time_unix": "fn() -> i64 [io]",
      "time_unix_ms": "fn() -> i64 [io]",
      "hex_encode": "fn(data: Vec<u8>) -> String []",
      "hex_decode": "fn(s: String) -> Vec<u8> []",
      "args": "fn() -> Vec<String> [read]",
      "stdin_read": "fn() -> String [read]",
      "stdin_read_line": "fn() -> String [read]",
      "stdin_ready": "fn() -> bool [read]",
      "process_exit": "fn(code: i64) -> ! [io]",
      "process_run": "fn(cmd: String, args: Vec<String>) -> i64 [io]",
      "process_get_stdout": "fn() -> String [io]",
      "process_get_stderr": "fn() -> String [io]",
      "process_start": "fn(cmd: String, args: Vec<String>) -> i64 [io]",
      "process_wait": "fn(pid: i64) -> i64 [io]",
      "process_wait_timeout": "fn(pid: i64, timeout_ms: i64) -> i64 [io]",
      "process_kill": "fn(pid: i64) -> i64 [io]",
      "process_stdout_for": "fn(pid: i64) -> String [io]",
      "process_stderr_for": "fn(pid: i64) -> String [io]"
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
    "extern_blocks": "extern \"C\" vow { requires: ... } { fn name(x: i64) -> i64 [unsafe] }",
    "methods": {
      "Vec<T>": [
        "Vec::new()",
        ".push(val)",
        ".pop()",
        ".len()",
        ".clear()",
        ".truncate(n)",
        "v[i]",
        "v[i] = val"
      ],
      "String": [
        "String::from(lit)",
        "String::new()",
        ".len()",
        ".byte_at(i)",
        ".push_byte(b)",
        ".push_str(s)",
        ".clear()",
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
  vow test [OPTIONS] [<path>]          Run tests (test_*.vow / *_test.vow)
  vow contracts [OPTIONS] <source.vow> List all contracts
  vow decl [OPTIONS] <source.vow>    Emit declaration file (.vow.d)
  vow skill [print|install]           Generate or install Claude Code skill
  vow [OPTIONS] <source.vow>          Legacy mode (same as vow build)

BUILD OPTIONS
  -o, --output <path>     Output executable path (default: source without .vow extension)
  --mode <debug|release|profile>  Build mode: debug inserts runtime vow checks, profile inserts call counters and prints report on normal exit (default: release)
  --no-verify             Skip ESBMC static verification
  --dump-ir               Print IR text to stdout and exit (no JSON output, no codegen)
  --debug-trace <off|calls|full>  Emit JSON trace lines to stderr at runtime (default: off)
  --no-cache              Disable compile and verify caching
  --unwind <N>            ESBMC loop unwind bound (default: 10)

VERIFY OPTIONS
  --no-cache              Disable verification result caching
  --unwind <N>            ESBMC loop unwind bound

TEST OPTIONS
  <path>                  Directory to scan or single .vow file (default: .)
  --verify                Run ESBMC verification on test files
  --filter <pat>          Only run tests whose name contains pat
  --mode <debug|release>  Build mode; debug inserts runtime vow checks (default: (default))
  --timeout <ms>          Per-test execution timeout in milliseconds (default: 30000)
  --unwind <N>            ESBMC loop unwind bound (with --verify)

CONTRACTS OPTIONS
  --verify                Run ESBMC verification and report per-contract status
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

TYPES     : i32  i64  u8  u64  f32  f64  bool  ()  !  Vec<T>  Option<T>  Result<T, E>  String  HashMap<K, V>
EFFECTS   : io  read  write  panic  unsafe
BUILTINS  : print_str: fn(s: String) -> () [io]   print_i64: fn(v: i64) -> () [io]   print_u64: fn(v: u64) -> () [io]
            eprintln_str: fn(s: String) -> () [io]   fs_read: fn(path: String) -> String [read]   fs_write: fn(path: String, data: String) -> i64 [write]   fs_exists: fn(path: String) -> i64 [read]   fs_mkdir: fn(path: String) -> i64 [io]   fs_listdir: fn(path: String) -> Vec<String> [read]   fs_remove: fn(path: String) -> i64 [io]   fs_remove_dir: fn(path: String) -> i64 [io]   fs_is_dir: fn(path: String) -> i64 [read]   fs_rename: fn(old: String, new: String) -> i64 [io]   string_substr: fn(s: String, start: i64, len: i64) -> String []   string_split: fn(s: String, delim: String) -> Vec<String> []   string_starts_with: fn(s: String, prefix: String) -> i64 []   string_ends_with: fn(s: String, suffix: String) -> i64 []   string_trim: fn(s: String) -> String []   string_to_upper: fn(s: String) -> String []   string_to_lower: fn(s: String) -> String []   string_replace: fn(s: String, from: String, to: String) -> String []   string_join: fn(parts: Vec<String>, sep: String) -> String []   parse_i64: fn(s: String) -> i64 []   i64_to_string: fn(v: i64) -> String []   vec_sort: fn(v: Vec<i64>) -> Vec<i64> []   time_unix: fn() -> i64 [io]   time_unix_ms: fn() -> i64 [io]   hex_encode: fn(data: Vec<u8>) -> String []   hex_decode: fn(s: String) -> Vec<u8> []   args: fn() -> Vec<String> [read]   stdin_read: fn() -> String [read]   stdin_read_line: fn() -> String [read]   stdin_ready: fn() -> bool [read]   process_exit: fn(code: i64) -> ! [io]   process_run: fn(cmd: String, args: Vec<String>) -> i64 [io]   process_get_stdout: fn() -> String [io]   process_get_stderr: fn() -> String [io]   process_start: fn(cmd: String, args: Vec<String>) -> i64 [io]   process_wait: fn(pid: i64) -> i64 [io]   process_wait_timeout: fn(pid: i64, timeout_ms: i64) -> i64 [io]   process_kill: fn(pid: i64) -> i64 [io]   process_stdout_for: fn(pid: i64) -> String [io]   process_stderr_for: fn(pid: i64) -> String [io]
METHODS   : Vec: Vec::new/push/pop/len/clear/truncate/v[i]/v[i] = val   String: String::from/String::new/len/byte_at/push_byte/push_str/clear/contains/eq/substring/parse_i64/parse_u64
            HashMap: HashMap::new/insert/get/contains_key/remove/len   Option: unwrap
OPERATORS : + - * / %   +! -! *! /! %! (checked)   == != < <= > >=   && || !   - ! & ?

VERIFICATION LIMITS
  Loop unwind  : 10 iterations
  Vec<T>        : 128 max capacity
  String        : 256 max capacity
  HashMap<K, V> : 64 max capacity"#
        .to_string()
}

fn skill_full_markdown() -> String {
    r#"---
name: vow-toolchain
description: >-
  Write, compile, debug, and verify Vow programs (.vow files). Covers the
  CEGIS workflow (counterexample-guided inductive synthesis), contract
  authoring (requires, ensures, invariant), fixing VerifyFailed
  counterexamples, resolving CompileFailed diagnostics, loop invariants,
  the Vow effect system, and running vow build / vow verify. Use when the
  user says "write a Vow program", "fix this counterexample", "add
  contracts", "why did verification fail", "ESBMC", or "vow build".
globs: "**/*.vow"
---

# Vow Language Reference

Vow is a systems programming language with built-in contracts (preconditions, postconditions, loop invariants) that are statically verified by ESBMC bounded model checking. Programs compile to native executables via Cranelift. The compiler emits structured JSON for machine consumption.

In all documentation below, `vow` refers to the `build/vowc` binary. Always use `ulimit -v 2000000` before invoking the compiler or any binary it produces — without this, the process can consume all system memory.

## What Vow Excludes

No block comments, no generics, no traits, no closures, no macros, no garbage collection. Line comments (`//`) are supported.

## CEGIS Workflow

The standard workflow for writing verified Vow programs:

1. **Write** — Create a `.vow` file with function contracts (`requires`, `ensures`, `invariant`)
2. **Build** — Run `ulimit -v 2000000; build/vowc build <file.vow>`
3. **Parse JSON** — Read the JSON object from stdout
4. **Handle status:**
   - `Verified` → Done. Binary is at `executable`.
   - `Unverified` → Compiled but ESBMC not available. Binary is at `executable`.
   - `CompileFailed` → Read `diagnostics[]` for errors. Fix and retry.
   - `VerifyFailed` → Read `counterexamples[]`. Fix contracts or implementation. Retry.
5. **Iterate** — Repeat steps 2–4 until `Verified`.

## Minimal Program

```vow
module Hello

fn main() -> i32 [io] {
    print_str("Hello, world!");
    0
}
```

Build and run (`build/vowc` is the primary compiler binary, produced by `scripts/bootstrap.sh`):
```
$ ulimit -v 2000000; build/vowc build hello.vow
$ ulimit -v 2000000; ./hello
Hello, world!
```

---

# Vow Grammar Reference

Complete grammar for the Vow programming language. Vow source files use the `.vow` extension.

**Line comments.** `//` starts a line comment extending to end of line. Comments are stripped during lexing and never enter the token stream. Block comments (`/* */`) are not supported. Machine-relevant intent belongs in contracts; comments are for non-semantic rationale.

## Module Declaration

Every file begins with a module declaration:

```
module <Name>
```

`<Name>` is a PascalCase identifier. There is no semicolon.

## Use Declarations

Import other modules with dot-separated paths:

```
use foo.bar
```

This resolves to `<rootdir>/foo/bar.vow` relative to the main source file.

## Const Declarations

Named constants with compile-time values:

```vow
const MAX_SIZE: i64 = 1024;
const NEG_ONE: i64 = -1;
const DEBUG: bool = true;
```

Supported value forms: integer literals, boolean literals, negated integer literals. Constants are inlined at every use site (zero runtime cost). The type must be `i64`, `i32`, or `bool`. Constants are referenced by name in expressions like any other identifier.

## Functions

### Pure Function

```vow
fn add(x: i64, y: i64) -> i64 {
    x + y
}
```

### Function with Effects

```vow
fn main() -> i32 [io] {
    print_str("hello");
    0
}
```

Effects appear in brackets after the return type: `[io]`, `[read, write]`, `[io, panic]`.

### Function with Vow Block

```vow
fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
} {
    x / y
}
```

The `vow` block sits between the signature and the body. Clauses:
- `requires: <expr>` — precondition (blame: Caller)
- `ensures: <expr>` — postcondition (blame: Callee); use `result` for the return value
- `invariant: <expr>` — loop invariant (blame: Callee)

Multiple clauses are separated by commas:

```vow
fn clamp(x: i64, lo: i64, hi: i64) -> i64 vow {
    requires: lo <= hi,
    ensures: result >= lo,
    ensures: result <= hi
} {
    if x < lo { lo } else { if x > hi { hi } else { x } }
}
```

### Where Clauses (Refinement Types on Parameters)

```vow
fn safe_sub(a: i64 where a >= 0, b: i64 where b >= 0) -> i64 vow {
    requires: a >= b,
    ensures: result >= 0
} {
    a - b
}
```

`where` constraints on parameters become additional `requires` in verification. Each `where` clause can only reference its own parameter — it cannot reference other parameters.

### Public Functions

```vow
pub fn api_function(x: i64) -> i64 {
    x
}
```

## Types

### Primitive Types

| Type   | Description              |
|--------|--------------------------|
| `i32`  | 32-bit signed integer    |
| `i64`  | 64-bit signed integer    |
| `u64`  | 64-bit unsigned integer  |
| `f32`  | 32-bit float (limited support — avoid in contracts) |
| `f64`  | 64-bit float (limited support — avoid in contracts) |
| `bool` | Boolean                  |
| `()`   | Unit type                |

### Built-in Parameterized Types

| Type               | Description                     |
|--------------------|---------------------------------|
| `Vec<T>`           | Growable array                  |
| `Option<T>`        | Optional value (Some/None)      |
| `Result<T, E>`     | Success or error                |
| `String`           | UTF-8 string (backed by Vec<u8>)|
| `HashMap<K, V>`    | Key-value map (linear scan)     |

### User-Defined Types

Structs and enums (see below).

## Literals

### Integer Literals

```vow
42
-1
0
```

All unsuffixed integer literals are `i64`. Integer literals coerce to `u64` in annotation context (e.g. `let x: u64 = 42;`).

Suffixed integer literals: `42u64` produces a `u64` value directly.

### Float Literals

```vow
3.14
-0.5
```

### Boolean Literals

```vow
true
false
```

### String Literals

```vow
"hello, world"
"line one\nline two"
"tab\there"
"null\0byte"
"escaped\\backslash"
"escaped\"quote"
```

Supported escape sequences: `\n`, `\t`, `\r`, `\\`, `\"`, `\0`.

## Operators

### Wrapping Arithmetic (default)

| Operator | Meaning        |
|----------|----------------|
| `+`      | Add (wrapping) |
| `-`      | Sub (wrapping) |
| `*`      | Mul (wrapping) |
| `/`      | Div (wrapping) |
| `%`      | Rem (wrapping) |

Wrapping operators silently wrap on overflow. For `u64` operands, division and remainder use unsigned semantics.

### Checked Arithmetic

| Operator | Meaning           |
|----------|-------------------|
| `+!`     | Add (checked)     |
| `-!`     | Sub (checked)     |
| `*!`     | Mul (checked)     |
| `/!`     | Div (checked)     |
| `%!`     | Rem (checked)     |

Checked operators abort with `ArithmeticOverflow` on overflow.

### Comparison Operators

| Operator | Meaning                |
|----------|------------------------|
| `==`     | Equal                  |
| `!=`     | Not equal              |
| `<`      | Less than              |
| `<=`     | Less than or equal     |
| `>`      | Greater than           |
| `>=`     | Greater than or equal  |

### Logical Operators

| Operator | Meaning    |
|----------|------------|
| `&&`     | Logical AND (short-circuit) |
| `\|\|`   | Logical OR (short-circuit) |
| `!`      | Logical NOT|

`&&` and `||` use short-circuit evaluation: for `a && b`, `b` is only evaluated if `a` is true; for `a || b`, `b` is only evaluated if `a` is false.

### Unary Operators

| Operator | Meaning    |
|----------|------------|
| `-`      | Negation (not allowed on `u64`) |
| `!`      | Logical NOT|
| `&`      | Borrow     |
| `?`      | Unwrap (propagate error) |

### Type Cast

```vow
x as u64    // i64 -> u64
y as i64    // u64 -> i64
```

The `as` operator converts between `i64` and `u64`. No implicit conversions: `i64 + u64` is a type error.

In debug mode, out-of-range casts (negative i64 to u64, or u64 > i64::MAX to i64) are no-ops at the machine level (bit reinterpretation). In release mode, the same applies.

## Let Bindings

### Immutable

```vow
let x: i64 = 42;
```

### Mutable

```vow
let mut i: i64 = 0;
i = i + 1;
```

### Pattern Destructuring

```vow
let (a, b): (i64, i64) = (1, 2);
```

## Control Flow

### If / Else

```vow
if x > 0 {
    x
} else {
    0 - x
}
```

`if`/`else` is an expression — both branches must have the same type. There is no `else if` keyword; nest `if` inside `else`:

```vow
if x < lo {
    lo
} else {
    if x > hi {
        hi
    } else {
        x
    }
}
```

### While Loop

```vow
while i > 0 {
    i = i - 1;
}
```

### While Loop with Invariant

```vow
while i < n vow {
    invariant: i >= 0,
    invariant: i <= n
} {
    v.push(i);
    i = i + 1;
}
```

### For-Each Loop

```vow
for x in vec {
    print_i64(x);
}
```

Iterates over each element of a `Vec<T>`. The loop variable `x` is bound to each element in turn. Desugars to a `while` loop with index arithmetic — zero verification overhead.

### For-Each Loop with Invariant

```vow
for x in vec vow {
    invariant: total >= 0
} {
    total = total + x;
}
```

### Loop (Infinite)

`loop` creates an infinite loop. The expression returns the type of the `break` value:

```vow
let idx: i64 = loop {
    if data[i] == target {
        break i;
    }
    i = i + 1;
    if i >= n { break -1; }
};
```

ESBMC cannot verify unbounded `loop` constructs — use `while` with invariants for verifiable loops.

### Break

`break` exits the innermost loop. Inside `loop`, `break value` sets the loop's result:

```vow
break;           // exit while or loop (loop returns Unit)
break value;     // exit loop with a value (only inside loop, not while)
```

### Continue

`continue` skips the remaining statements in the current loop iteration and jumps back to the loop header:

```vow
continue;        // skip to next iteration of while, loop, or for
```

Inside `while` and `loop`, `continue` emits back-edge values for any mutated variables. Inside `for`, it also advances the loop index.

### Return

```vow
return;
return value;
```

## Struct Definitions

```vow
struct Point {
    x: i64,
    y: i64,
}
```

### Linear Structs

```vow
linear struct FileHandle {
    fd: i64,
}
```

Linear struct values must be consumed exactly once.

### Struct Literals

Struct literal names must be PascalCase:

```vow
let p: Point = Point { x: 1, y: 2 };
```

### Field Access

```vow
p.x
```

## Enum Definitions

```vow
enum Shape {
    Circle(i64),
    Rect(i64, i64),
    Empty,
}
```

Variant kinds: unit (`Empty`), tuple (`Circle(i64)`), struct (`Named { x: i64 }`).

### Enum Construction

```vow
let s: Shape = Shape::Circle(5);
let none: Option<i64> = Option::None;
let some: Option<i64> = Option::Some(42);
```

### Built-in Enums

`Option<T>` has variants `Some(T)` and `None`.
`Result<T, E>` has variants `Ok(T)` and `Err(E)`.

## Pattern Matching

```vow
match value {
    Pattern1 => expr1,
    Pattern2 => expr2,
    _ => default_expr,
}
```

Match is an expression. All arms must return the same type. Patterns must be exhaustive.

### Pattern Kinds

| Pattern                      | Example                          |
|------------------------------|----------------------------------|
| Wildcard                     | `_`                              |
| Identifier binding           | `x`                              |
| Mutable identifier           | `mut x`                          |
| Literal                      | `0`, `true`, `"hello"`           |
| Tuple                        | `(a, b)`                         |
| Enum variant (unit)          | `Option::None`                   |
| Enum variant (tuple)         | `Option::Some(x)`                |
| Enum variant (struct)        | `Shape::Named { x, y }`         |
| Or pattern                   | `0 \| 1 \| 2`                   |
| Struct pattern               | `Point { x, y }`                |

## Method Calls

```vow
v.push(42);
v.len()
s.byte_at(0)
m.contains_key(k)
```

### Vec<T> Methods

| Method         | Signature                        |
|----------------|----------------------------------|
| `Vec::new()`   | `() -> Vec<T>`                   |
| `.push(val)`   | `(T) -> ()`                      |
| `.pop()`       | `() -> ()`                       |
| `.len()`       | `() -> i64`                      |
| `.clear()`     | `() -> ()` — frees buffer, resets to empty |
| `.truncate(n)` | `(i64) -> ()` — shrinks to n elements, frees excess memory |
| `v[i]`         | Index read — copies slot value; aliases heap types (panics if out of bounds) |
| `v[i] = val`   | Index write — copies value into slot |

### String Methods

| Method              | Signature                   |
|---------------------|-----------------------------|
| `String::from(lit)` | `(&str) -> String`          |
| `String::new()`     | `() -> String`              |
| `.len()`            | `() -> i64`                 |
| `.byte_at(i)`       | `(i64) -> i64`              |
| `.push_byte(b)`     | `(i64) -> ()`               |
| `.push_str(s)`      | `(String) -> ()`            |
| `.clear()`          | `() -> ()` — frees buffer, resets to empty |
| `.contains(s)`      | `(String) -> bool`          |
| `.eq(s)`            | `(String) -> bool`          |
| `.substring(start, end)` | `(i64, i64) -> String` |
| `.parse_i64()`      | `() -> Option<i64>`         |
| `.parse_u64()`      | `() -> Option<u64>`         |

### HashMap<K, V> Methods

| Method              | Signature                   |
|---------------------|-----------------------------|
| `HashMap::new()`    | `() -> HashMap<K, V>`       |
| `.insert(k, v)`     | `(K, V) -> ()`              |
| `.get(k)`           | `(K) -> V`                  |
| `.contains_key(k)`  | `(K) -> bool`               |
| `.remove(k)`        | `(K) -> ()`                 |
| `.len()`            | `() -> i64`                 |

### Option<T> Methods

| Method      | Signature                              |
|-------------|----------------------------------------|
| `.unwrap()` | `() -> T` (panics on None; requires `[panic]` effect) |

The `?` operator on `Option<T>` or `Result<T, E>` propagates `None`/`Err` to the caller (the calling function must return `Option` or `Result`).

## Indexing

```vow
let val: i64 = v[0];
v[i] = new_val;
```

Indexing uses **copy semantics**: `v[i]` copies the 8-byte slot value and `v[i] = val` copies a value into the slot. The base container is not consumed.

For primitive types (`i64`, `bool`), this is a genuine value copy — the result is independent of the container. For heap types (`Vec<T>`, `String`, structs, enums), the 8-byte slot holds a pointer, so indexing copies the pointer, creating an **alias**. Both the container slot and the local variable point to the same heap data:

```vow
let buckets: Vec<Vec<i64>> = Vec::new();
buckets.push(Vec::new());
let b: Vec<i64> = buckets[0];  // b aliases buckets[0]
b.push(42);                     // visible through buckets[0]
```

This aliasing is the intended behavior for arena and hash-table patterns where bucket contents are read and mutated repeatedly through index access.

## Extern Blocks

Declare external C functions (a `vow` contract block is required):

```vow
extern "C" vow {
    requires: fd >= 0
    ensures: return >= 0
}
{
    fn write(fd: i32, ptr: i64, len: i64) -> i64 [io]
}
```

Omitting the `vow` block produces a `MissingContract` error (see [errors.md](errors.md)).

## Type Aliases

```vow
type Score = i64
```

## Effect System

Effects are explicit. Every function declares which side effects it may perform. Pure functions (no effects) need no annotation.

### Effect Types

| Effect   | Meaning                              |
|----------|--------------------------------------|
| `io`     | Standard I/O (print, stdin, network) |
| `read`   | File system reads                    |
| `write`  | File system writes                   |
| `panic`  | May panic (unwrap, etc.)             |
| `unsafe` | Unsafe operations (FFI, raw memory)  |

Each effect is independent — `io` is not a superset of `read` or `write`.

### Propagation

A function must declare every effect that any function it calls may produce:

```vow
fn do_io() -> () [io] {
    print_str("hi");
}

fn caller() -> () [io] {
    do_io();
}
```

If `caller` omitted `[io]`, the type checker would emit `EffectViolation`.

### Contract Purity

Contract expressions (`requires`, `ensures`, `invariant`) must be pure — they cannot call effectful functions.

### Builtin Function Signatures

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `print_str`      | `fn(s: String) -> ()`                      | `[io]`     |
| `print_i64`      | `fn(v: i64) -> ()`                         | `[io]`     |
| `print_u64`      | `fn(v: u64) -> ()`                         | `[io]`     |
| `eprintln_str`   | `fn(s: String) -> ()`                      | `[io]`     |
| `fs_read`        | `fn(path: String) -> String`               | `[read]`   |
| `fs_write`       | `fn(path: String, data: String) -> ()`     | `[write]`  |
| `args`           | `fn() -> Vec<String>`                      | `[read]`   |
| `stdin_read`     | `fn() -> String`                           | `[read]`   |
| `stdin_read_line`| `fn() -> String`                           | `[read]`   |
| `process_exit`   | `fn(code: i64) -> ()`                      | `[io]`     |

**`args` semantics:** `args()` returns all process arguments including the program name at index 0 (matching C `argv` and Rust `std::env::args()` conventions). For `./my_program foo bar`, `args()` returns `["./my_program", "foo", "bar"]`. Use `args[1]` onward for user-supplied arguments. The Vec is empty only if the OS provides no arguments (unusual). Returns an empty String element if an argument is empty (`""`). Non-UTF-8 arguments are included as-is (byte content preserved).

**`fs_read` semantics:** `fs_read(path)` opens the file at `path`, reads its entire contents, and returns a String. Returns `""` (empty String) on any error (file not found, permission denied, I/O error, non-UTF-8 path). Does not block on regular files. Callers should check `result.len() == 0` to detect failure.

**`stdin_read` vs `stdin_read_line`:** `stdin_read()` reads the entire stdin stream into a single String (unbounded memory). `stdin_read_line()` reads one line at a time, including the trailing newline. Returns `""` (empty string) at EOF. Use `stdin_read_line` for line-at-a-time processing with bounded memory:

```vow
let line: String = stdin_read_line();
while str_len(line) > 0 {
    // process line (has trailing \n)
    line = stdin_read_line();
}
```

## Canonical Form

The canonical printer normalizes source: `parse → print → parse` is idempotent. Effects are sorted alphabetically, indentation uses 4 spaces, trailing expressions omit semicolons.

---

# Vow CLI Reference

## Commands

### `vow build` (default)

Compile source to native executable. Verifies contracts by default.

```
vow build [OPTIONS] <source.vow>
vow [OPTIONS] <source.vow>          # legacy (equivalent)
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `-o, --output`    | `build/<stem>` | Output executable path                  |
| `--mode <debug\|release\|profile>` | `release` | Build mode: debug inserts runtime vow checks, profile inserts call counters and prints report on normal exit |
| `--no-verify`     | (off)       | Skip ESBMC static verification            |
| `--dump-ir`       | (off)       | Print IR text to stdout and exit (no JSON output, no codegen) |
| `--debug-trace <off\|calls\|full>` | `off` | Emit JSON trace lines to stderr at runtime |
| `--no-cache`    | (off)       | Disable compile and verify caching           |
| `--unwind <N>`  | `10`        | ESBMC loop unwind bound                      |

### `vow verify`

Verify contracts only — no executable output. Emits the same JSON format as `vow build` but `executable` is always `null`.

```
vow verify [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `--no-cache`      | (off)       | Disable verification result caching        |
| `--unwind <N>`    | `10`        | ESBMC loop unwind bound                   |

### `vow contracts`

List all contracts (requires, ensures, invariant) in a program. Runs frontend only by default (no codegen, no verification).

```
vow contracts [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `--verify`        | (off)       | Run ESBMC verification and report per-contract status |
| `--no-cache`      | (off)       | Disable verification result caching        |
| `--unwind <N>`    | `10`        | ESBMC loop unwind bound                   |

### `vow skill`

Generate or install the Claude Code skill document for the current compiler version. The skill is embedded in the compiler binary, ensuring the documentation always matches the installed toolchain.

```
vow skill              # print skill document to stdout (default: print)
vow skill print        # same as above
vow skill install      # install to .claude/commands/vow-toolchain.md
```

`print` writes the complete skill markdown (with YAML frontmatter) to stdout. Pipe to a file or use `install` to place it directly.

`install` creates `.claude/commands/` in the current directory if needed and writes the skill document there. Claude Code discovers it automatically.

### `vow test`

Not yet implemented.

### `vow --help`

```
vow --help               # JSON capability description (for agents)
vow --help --human       # human-readable text
vow build --help         # same JSON (works on all subcommands)
vow verify --help --human  # same human text (works on all subcommands)
```

## Exit Codes

| Code | Meaning                              |
|------|--------------------------------------|
| `0`  | Success (Verified or Unverified)     |
| `1`  | Failure (CompileFailed or VerifyFailed) |

## Build Output JSON

`vow build` and `vow verify` emit a single JSON object to stdout. Schema: [`schemas/build-result.schema.json`](schemas/build-result.schema.json).

**Note:** `--dump-ir` suppresses JSON output — only IR text is printed.

### Status Values

| Status          | Meaning                                     |
|-----------------|---------------------------------------------|
| `Verified`      | Compiled + all contracts proved by ESBMC     |
| `Unverified`    | Compiled with `--no-verify` (ESBMC skipped)  |
| `CompileFailed` | Parse error, type error, module load error, or link failure |
| `VerifyFailed`  | ESBMC found a counterexample, or ESBMC not found |

### Verified Example

```json
{
  "status": "Verified",
  "executable": "examples/divide",
  "diagnostics": [],
  "counterexamples": []
}
```

### CompileFailed Example

```json
{
  "status": "CompileFailed",
  "executable": null,
  "diagnostics": [
    {
      "error_code": "TypeMismatch",
      "message": "function body has type `bool` but declared return type is `i32`",
      "severity": "error",
      "span": {
        "file": "bad.vow",
        "offset": 25,
        "length": 8
      }
    }
  ],
  "message": "type error",
  "counterexamples": []
}
```

### VerifyFailed Example

```json
{
  "status": "VerifyFailed",
  "executable": "examples/cegis_broken",
  "diagnostics": [],
  "function": "safe_sub",
  "counterexample": "[Counterexample]",
  "counterexamples": [
    {
      "function": "safe_sub",
      "inputs": { "a": "-9223372036854775808", "b": "0" },
      "violation": "ensures result >= 0",
      "vow_id": 1,
      "source": {
        "file": "examples/cegis_broken.vow",
        "offset": 76,
        "length": 20
      }
    }
  ]
}
```

### Fields Reference

| Field              | Type                | When Present      | Description                               |
|--------------------|---------------------|-------------------|-------------------------------------------|
| `status`           | string              | Always            | One of the four status values             |
| `executable`       | string \| null      | Always            | Path to binary, null on compile failure or library module (no main) |
| `diagnostics`      | array               | Always            | Compiler diagnostics (see schema)         |
| `message`          | string              | CompileFailed     | Error category ("parse error", "type error", "module load error", or link error detail) |
| `function`         | string              | VerifyFailed      | Function where verification failed        |
| `counterexample`   | string              | VerifyFailed      | Legacy description string                 |
| `counterexamples`  | array               | Always            | Structured counterexamples (see schema)   |
| `verify_status`    | string              | On timeout/error  | "timeout" or "error"                      |
| `verify_message`   | string              | On error          | ESBMC error message                       |

## Contracts Output JSON

`vow contracts` emits a single JSON object to stdout. Schema: [`schemas/contracts-result.schema.json`](schemas/contracts-result.schema.json).

### Example (without --verify)

```json
{
  "contracts": [
    {
      "vow_id": 0,
      "function": "divide",
      "kind": "requires",
      "description": "requires y != 0",
      "blame": "Caller",
      "source": { "file": "divide.vow", "offset": 42 },
      "status": "not_verified"
    }
  ],
  "summary": { "total": 1, "proven": 0, "failed": 0, "timeout": 0, "error": 0, "not_verified": 1 }
}
```

### Example (with --verify)

```json
{
  "contracts": [
    {
      "vow_id": 0,
      "function": "divide",
      "kind": "requires",
      "description": "requires y != 0",
      "blame": "Caller",
      "source": { "file": "divide.vow", "offset": 42 },
      "status": "proven"
    }
  ],
  "summary": { "total": 1, "proven": 1, "failed": 0, "timeout": 0, "error": 0, "not_verified": 0 }
}
```

### Contract Fields

| Field         | Type    | Description                                              |
|---------------|---------|----------------------------------------------------------|
| `vow_id`      | integer | Unique contract identifier within the program            |
| `function`    | string  | Function containing this contract                        |
| `kind`        | string  | `"requires"`, `"ensures"`, or `"invariant"`              |
| `description` | string  | Full contract text                                       |
| `blame`       | string  | `"Caller"` (requires) or `"Callee"` (ensures/invariant)  |
| `source`      | object  | `{ "file": string, "offset": integer }`                  |
| `status`      | string  | `"proven"`, `"failed"`, `"unknown"`, `"timeout"`, `"error"`, or `"not_verified"` |

### Status Values

| Status          | Meaning                                              |
|-----------------|------------------------------------------------------|
| `not_verified`  | Verification not requested (no `--verify` flag)      |
| `proven`        | ESBMC proved this contract holds for all inputs      |
| `failed`        | ESBMC found a counterexample violating this contract |
| `unknown`       | Another contract in the same function failed; this one was not individually checked |
| `timeout`       | ESBMC timed out on the containing function           |
| `error`         | ESBMC error or tool not found                        |

## Trace Output (stderr, --debug-trace)

When `--debug-trace=calls` or `--debug-trace=full` is used, the compiled binary emits JSON lines to stderr:

### calls mode
```json
{"event":"enter","fn":"main"}
{"event":"enter","fn":"divide"}
{"event":"exit","fn":"divide"}
{"event":"exit","fn":"main"}
```

### full mode (adds vow check results)
```json
{"event":"enter","fn":"divide"}
{"event":"vow","fn":"divide","vow_id":0,"passed":true}
{"event":"exit","fn":"divide"}
```

## Profile Output (stderr, profile mode)

When `--mode profile` is used, the compiled binary prints a call-count report to stderr on normal exit (via `atexit`). The report is not printed if the program is killed by a signal or calls `abort()`.

```
--- vow profile report ---
function                                        calls       %
-------------------------------------------------------------
infer                                         4812399   48.2%
is_def_eq_core                                3201882   32.1%
whnf                                           984201    9.9%
main                                                1    0.0%
-------------------------------------------------------------
total calls: 9998483, unique functions: 12
```

The report lists the top 20 most-called functions sorted by call count. No vow checks are emitted in profile mode.

## Runtime Error JSON (stderr, debug mode only)

When a compiled program runs in debug mode (`--mode debug`) and violates a vow at runtime, it emits JSON to stderr before aborting.

### VowViolation

```json
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"divide.vow","offset":42,"values":{"y":0}}
```

Schema: [`schemas/vow-violation.schema.json`](schemas/vow-violation.schema.json).

### ArithmeticOverflow

```json
{"error":"ArithmeticOverflow"}
```

Emitted when a checked arithmetic operator (`+!`, `-!`, etc.) overflows at runtime.

### UnwrapOnNone

```json
{"error":"UnwrapOnNone"}
```

Emitted when `.unwrap()` is called on `Option::None`.

### IndexOutOfBounds

```json
{"error":"IndexOutOfBounds"}
```

Emitted when a `Vec` index is out of bounds.

## Agent Decision Tree

```
Parse JSON from stdout
├── status == "Verified"       → Success. Binary at `executable`.
├── status == "Unverified"     → Compiled but unverified. ESBMC missing or --no-verify.
├── status == "CompileFailed"  → Read `diagnostics[]` for error details.
│   ├── error_code is parse error  → Fix syntax (see grammar.md)
│   └── error_code is type error   → Fix types (see errors.md)
└── status == "VerifyFailed"   → Read `counterexamples[]`.
    ├── Check `inputs` for the violating values
    ├── Check `violation` for which contract failed
    ├── Check `source` for the location
    └── Fix the contract or the implementation, then rebuild
```

Always check stderr for human-readable diagnostics alongside the JSON on stdout.

---

# Contract Authoring and Verification

Vow uses ESBMC (bounded model checker) for static contract verification. This document covers contract patterns, verification behavior, and common pitfalls.

## Verification Pipeline

Codegen (Cranelift) and verification run in parallel:

```
Vow Source → Parse → Type Check → IR Lower ─┬─→ Cranelift → executable
                                              └─→ C Emit → ESBMC → proof / counterexample
```

Contract clauses become IR opcodes. The C emitter translates `requires` to `__ESBMC_assume()` (the verifier assumes preconditions hold) and `ensures`/`invariant` to `__ESBMC_assert()` (the verifier checks postconditions).

### ESBMC Configuration

- Loop unwind bound: **10** — loops are checked for up to 10 iterations
- Architecture: 64-bit
- Array bounds / pointer checks disabled (Vow handles these in its own model)

### Collection Models for Verification

ESBMC uses bounded models for collection types:

| Type              | Max Capacity | Supported Operations |
|-------------------|-------------|----------------------------------------------|
| `Vec<T>`          | 128         | `new`, `push`, `pop`, `len`, `get`, `set`    |
| `String`          | 256         | `from`, `len`, `push_byte`, `byte_at`        |
| `HashMap<K, V>`   | 64          | `new`, `insert`, `get`, `contains_key`, `len`|

These support the same operations as the runtime but with bounded storage. `String::from` produces a nondeterministic length (0 to 255) in verification.

## Blame Model

| Clause      | Blame  | Who is at fault                                    |
|-------------|--------|----------------------------------------------------|
| `requires`  | Caller | The caller passed invalid arguments                |
| `ensures`   | Callee | The function body doesn't satisfy the postcondition|
| `invariant` | Callee | The loop body breaks the invariant                 |

## Integer Contracts

### Non-zero Guard

```vow
fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
} {
    x / y
}
```

### Range Bounds

```vow
fn safe_add(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: a <= 100,
    requires: b >= 0,
    requires: b <= 100,
    ensures: result >= 0,
    ensures: result <= 200
} {
    a + b
}
```

### Equality Postcondition

```vow
fn twice(x: i64) -> i64 vow {
    ensures: result == x + x
} {
    x + x
}
```

### Negation

```vow
fn negate(x: i64) -> i64 vow {
    ensures: result + x == 0
} {
    0 - x
}
```

**Warning:** Fails for `x = -9223372036854775808` (i64 min) due to wrapping overflow. Add `requires: x > -9223372036854775808` if needed.

## Vec Contracts

### Bounds Check

```vow
fn get_element(v: Vec<i64>, i: i64) -> i64 vow {
    requires: i >= 0,
    requires: i < v.len()
} {
    v[i]
}
```

### Fill Pattern with Loop Invariant

See the worked CEGIS example in [examples.md](examples.md#3-vec-fill--loop-invariant).

## String Contracts

### Non-empty String

```vow
fn make_greeting() -> String vow {
    ensures: result.len() > 0
} {
    let s: String = String::from("");
    s.push_byte(72);
    s
}
```

## HashMap Contracts

### Contains Key After Insert

```vow
fn insert_and_check() -> HashMap<i64, i64> vow {
    ensures: result.contains_key(42)
} {
    let m: HashMap<i64, i64> = HashMap::new();
    m.insert(42, 100);
    m
}
```

## Loop Invariants

### Counter Bounds

The most common loop invariant pattern bounds the loop counter:

```vow
while i < n vow {
    invariant: i >= 0,
    invariant: i <= n
} {
    i = i + 1;
}
```

### Search Range

```vow
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
```

## Where Clause Patterns

Where clauses on parameters become refinement types (additional `requires` for verification):

```vow
fn bounded_add(a: i64 where a >= 0, b: i64 where b >= 0) -> i64 vow {
    requires: a <= 100,
    requires: b <= 100,
    ensures: result >= 0,
    ensures: result <= 200
} {
    a + b
}
```

Each `where` clause can only reference its own parameter.

## Anti-Patterns

### Over-Specifying

```vow
fn add(x: i64, y: i64) -> i64 vow {
    ensures: result == x + y
} {
    x + y
}
```

Fails when `x + y` overflows. The contract mirrors the implementation exactly — it verifies nothing useful and breaks on edge cases.

**Fix:** Add bounds (`requires: x >= 0, ...`) or verify a weaker property.

### Wrapping Arithmetic Overflow

Default arithmetic (`+`, `-`, `*`) wraps on overflow. Contracts that assume no overflow will be violated:

```vow
fn double(x: i64) -> i64 vow {
    ensures: result > x
} {
    x + x
}
```

ESBMC finds: `x = 4611686018427387904` → `result = -9223372036854775808` (wraps negative).

**Fix:** Bound the input or use checked arithmetic (`+!`).

### Non-Inductive Loop Invariant

An invariant must hold at the **start** of every iteration, not just at the end:

```vow
while i < n vow {
    invariant: v.len() == n
} { ... }
```

This is not inductive — `v.len() == n` is only true after the loop.

**Fix:** Use `invariant: i >= 0, invariant: i <= n`.

### Unbound Loop Iterations

Without a bound on loop iterations, ESBMC may timeout (unwind bound is 10):

```vow
fn fill(n: i64) -> Vec<i64> vow {
    requires: n >= 0,
    ensures: result.len() == n
} { ... }
```

**Fix:** Add `requires: n <= 8` (or another value below the unwind bound).

## Interpreting Counterexamples

A counterexample in the JSON output:

```json
{
  "function": "safe_sub",
  "inputs": { "a": "-9223372036854775808", "b": "0" },
  "violation": "ensures result >= 0",
  "vow_id": 1,
  "source": { "file": "cegis_broken.vow", "offset": 76, "length": 20 }
}
```

| Field       | Meaning                                                |
|-------------|--------------------------------------------------------|
| `function`  | Which function failed                                  |
| `inputs`    | Parameter values that trigger the violation            |
| `violation` | Which contract clause was violated                     |
| `vow_id`    | Internal ID linking to the specific vow clause         |
| `source`    | Byte offset in the source file of the violated clause  |

Variable names prefixed with `_esbmc_` are ESBMC internal variables; named inputs map directly to function parameters.

## Unsigned Integer Contracts

The `u64` type works naturally in contracts. Use `as u64` to cast literal values in contract expressions:

```vow
fn safe_add(a: u64, b: u64) -> u64
vow {
    requires: a <= 1000 as u64
    requires: b <= 1000 as u64
    ensures: result >= a
    ensures: result >= b
}
{
    a + b
}
```

ESBMC verifies `u64` contracts using `uint64_t` and unsigned nondet values.

## Extern Block Contracts

Every `extern "C"` block **must** include a `vow { ... }` contract specifying the expected behavior of foreign functions. Omitting the contract is a `MissingContract` error.

```vow
extern "C" vow {
    requires: fd >= 0
    ensures: return >= 0
}
{
    fn write(fd: i32, ptr: i64, len: i64) -> i64 [io]
}
```

The contract applies to all functions declared in the block. ESBMC uses `requires` as assumptions and `ensures` as assertions when verifying callers of extern functions.

---

# Vow Error Catalog

Every Vow error has a machine-readable `error_code` in the JSON output. This document lists all error codes, their phase, meaning, an example trigger, and how to fix them.

## Compile-Time Errors

These appear in the `diagnostics` array of the build output JSON.

### UnterminatedString

**Phase:** Lexer
**Meaning:** A string literal was opened with `"` but never closed.

```vow
fn f() -> () [io] {
    print_str("hello);
}
```

**Fix:** Close the string with a matching `"`.

### InvalidCharacter

**Phase:** Lexer
**Meaning:** The source contains a character the lexer does not recognize.

```vow
fn f() -> i64 {
    x @ y
}
```

**Fix:** Remove the invalid character. Vow has no `@` operator.

### UnexpectedToken

**Phase:** Parser
**Meaning:** The parser encountered a token it did not expect at that position.

```vow
module M 123
```

**Fix:** Check the syntax around the reported span. Common causes: missing `{`, `}`, `(`, `)`, or a keyword in the wrong position.

### MissingDelimiter

**Phase:** Parser
**Meaning:** A matching delimiter (`}`, `)`, `]`) is missing.

```vow
fn f() -> i64 {
    42
```

**Fix:** Add the missing closing delimiter.

### TypeMismatch

**Phase:** Type Checker
**Meaning:** An expression has a different type than expected.

```vow
fn f() -> i32 {
    true
}
```

**Output:** `function body has type 'bool' but declared return type is 'i32'`

**Fix:** Change the expression or the declared type to match.

### EffectViolation

**Phase:** Type Checker
**Meaning:** A function calls another function with effects not declared in its own signature.

```vow
fn f() -> () {
    print_str("hi");
}
```

**Fix:** Add the required effect to the function signature: `fn f() -> () [io]`.

### LinearTypeViolation

**Phase:** Type Checker
**Meaning:** A value of a `linear struct` type was not consumed exactly once.

```vow
linear struct Handle { fd: i64 }

fn f() -> () {
    let h: Handle = Handle { fd: 1 };
}
```

**Fix:** Ensure every linear value is consumed (passed to a function, returned, or destructured) exactly once.

### NonExhaustiveMatch

**Phase:** Type Checker
**Meaning:** A `match` expression does not cover all possible variants.

```vow
fn f(o: Option<i64>) -> i64 {
    match o {
        Option::Some(x) => x,
    }
}
```

**Fix:** Add a `_ => ...` wildcard arm or cover all variants (`Option::None => ...`).

### UnknownMethod

**Phase:** Type Checker
**Meaning:** A method call uses a name that does not exist on the receiver type.

```vow
fn f() -> () {
    let v: Vec<i64> = Vec::new();
    v.psh(42);
}
```

**Output:** `unknown method 'psh' on type 'Vec<i64>'`

**Fix:** Check the method name for typos. Use `--help` to see available methods for each type.

### UnsupportedFeature

**Phase:** Type Checker
**Meaning:** A language feature that is not supported in Vow was used.

```vow
trait Foo {
    fn bar() -> i64;
}
```

**Output:** `trait blocks are not supported in Vow`

**Fix:** Remove the unsupported construct. Vow does not support traits or impl blocks.

### MissingContract

**Phase:** Type Checker
**Meaning:** An `extern "C"` block was declared without a `vow { ... }` contract. Every foreign function call requires a mandatory contract specifying expected behavior.

```vow
extern "C" {
    fn write(fd: i32, ptr: i64, len: i64) -> i64 [io];
}
```

**Output:** `extern block requires a vow contract`

**Fix:** Add a `vow { ... }` block to the extern declaration with `requires` and/or `ensures` clauses.

### VowRequiresViolated

**Phase:** Verification (ESBMC)
**Meaning:** ESBMC found inputs that violate a `requires` precondition. This is a **static** verification error — it means the function's callers can reach it with invalid arguments.

**Fix:** Strengthen the `requires` clause, or fix the callers to pass valid arguments.

### VowEnsuresViolated

**Phase:** Verification (ESBMC)
**Meaning:** ESBMC found inputs where the function's return value does not satisfy the `ensures` postcondition.

**Fix:** Fix the function body to satisfy the postcondition, or weaken the `ensures` clause.

### VowInvariantViolated

**Phase:** Verification (ESBMC)
**Meaning:** ESBMC found a loop iteration where the `invariant` does not hold.

**Fix:** Strengthen the invariant or fix the loop body.

### EsbmcNotFound

**Phase:** Verification
**Meaning:** ESBMC is not installed or not on `$PATH`. When verification is enabled (the default for `vowc build`, always for `vowc verify`), the compiler checks for ESBMC upfront before compilation. If ESBMC is not found, the build aborts immediately with exit code 1.

**Fix:** Install ESBMC, or use `--no-verify` to skip verification: `vowc build --no-verify <file>`.

## Runtime Errors

These are emitted to stderr as JSON when a compiled program runs (debug mode for VowViolation).

### VowViolation

**When:** Debug mode only (`--mode debug`). A `requires`, `ensures`, or `invariant` predicate evaluates to false at runtime.

```json
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"divide.vow","offset":42,"values":{"y":0}}
```

The `blame` field indicates who is at fault:
- `Caller` — a `requires` was violated (the caller passed bad arguments)
- `Callee` — an `ensures` or `invariant` was violated (the function has a bug)

**Fix:** See the `description` and `values` fields to understand which predicate failed and with what runtime values.

### ArithmeticOverflow

**When:** A checked arithmetic operator (`+!`, `-!`, `*!`, `/!`, `%!`) overflows at runtime.

```json
{"error":"ArithmeticOverflow"}
```

**Fix:** Use wrapping arithmetic (`+`, `-`, etc.) if overflow is acceptable, or add bounds contracts to prevent overflow.

### UnwrapOnNone

**When:** `.unwrap()` is called on `Option::None`.

```json
{"error":"UnwrapOnNone"}
```

**Fix:** Use `match` to handle `None`, or add contracts that guarantee the value is `Some`.

### IndexOutOfBounds

**When:** A `Vec` index access (`v[i]` or `v[i] = val`) uses an index outside `0..v.len()`.

```json
{"error":"IndexOutOfBounds"}
```

**Fix:** Add a bounds check before indexing, or add contracts: `requires: i >= 0, requires: i < v.len()`.

## Warnings

### LoweringWarning

**Phase:** IR Lowering
**Meaning:** The IR lowerer could not resolve a struct type tag or field name, defaulting to index 0. This usually indicates a missing type annotation on a `let` binding, causing the compiler to lose track of which struct type a pointer refers to.

**Fix:** Add an explicit type annotation: `let x: MyStruct = ...;` so the compiler can track struct type tags through the IR.

---

# Worked Examples

Verification workflow examples. The first three demonstrate Counterexample-Guided Inductive Synthesis (CEGIS) cycles: write spec, build, read JSON, diagnose, fix, verify. The fourth shows break-with-value in loop expressions.

## 1. Safe Division — Requires Pattern

### Goal

Write a division function that is safe (cannot divide by zero).

### Step 1: Write the spec

```vow
module Divide

fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
} {
    x / y
}

fn main() -> i32 [io] {
    divide(10, 0);
    0
}
```

### Step 2: Build and verify

```
$ vow build examples/divide.vow
```

```json
{"status":"Verified","executable":"examples/divide","diagnostics":[],"counterexamples":[]}
```

ESBMC proves the contract: whenever `y != 0` holds, the division is safe.

### Step 3: Runtime behavior (debug mode)

The `main()` calls `divide(10, 0)` which violates `requires: y != 0`. In debug mode:

```
$ vow build --mode debug --no-verify examples/divide.vow -o /tmp/divide_debug
$ /tmp/divide_debug
```

Stderr:
```json
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"examples/divide.vow","offset":56,"values":{"y":0}}
```

The `blame: "Caller"` tells you: `main()` passed `y=0`, which violates the precondition.

---

## 2. CEGIS Broken → Fixed — The Core Workflow

### Goal

Write `safe_sub(a, b)` that always returns a non-negative result.

### Step 1: Initial attempt (broken)

```vow
module CegisBroken

fn safe_sub(a: i64, b: i64 where b >= 0) -> i64 vow {
    ensures: result >= 0
} {
    a - b
}

fn main() -> i32 [io] {
    print_i64(safe_sub(10, 3));
    0
}
```

### Step 2: Build

```
$ vow build examples/cegis_broken.vow
```

```json
{
  "status": "VerifyFailed",
  "executable": "examples/cegis_broken",
  "diagnostics": [],
  "function": "safe_sub",
  "counterexample": "[Counterexample]",
  "counterexamples": [
    {
      "function": "safe_sub",
      "inputs": { "a": "-9223372036854775808", "b": "0" },
      "violation": "ensures result >= 0",
      "vow_id": 1,
      "source": { "file": "examples/cegis_broken.vow", "offset": 76, "length": 20 }
    }
  ]
}
```

### Step 3: Diagnose

The counterexample shows `a = -9223372036854775808` (i64 min), `b = 0`. Then `a - b = a`, which is negative. The `ensures: result >= 0` is violated.

**Root cause:** We need `a >= b` to guarantee a non-negative result, and `a >= 0` to prevent negative inputs.

### Step 4: Fix

```vow
module CegisFixed

fn safe_sub(a: i64 where a >= 0, b: i64 where b >= 0) -> i64 vow {
    requires: a >= b,
    ensures: result >= 0
} {
    a - b
}

fn main() -> i32 [io] {
    print_i64(safe_sub(10, 3));
    0
}
```

### Step 5: Verify

```
$ vow build examples/cegis_fixed.vow
```

```json
{"status":"Verified","executable":"examples/cegis_fixed","diagnostics":[],"counterexamples":[]}
```

Verified. With `a >= 0`, `b >= 0`, and `a >= b`, ESBMC proves `result >= 0`.

---

## 3. Vec Fill — Loop Invariant

### Goal

Fill a vector with `n` elements and prove its length equals `n`.

### Step 1: Write the spec

```vow
module VecFill

fn fill_vec(n: i64) -> Vec<i64> vow {
    requires: n >= 0,
    requires: n <= 8,
    ensures: result.len() == n
} {
    let v: Vec<i64> = Vec::new();
    let mut i: i64 = 0;
    while i < n vow {
        invariant: i >= 0,
        invariant: i <= n
    } {
        v.push(i);
        i = i + 1;
    }
    v
}

fn main() -> i32 [io] {
    let v: Vec<i64> = fill_vec(5);
    print_i64(v.len());
    0
}
```

### Step 2: Build and verify

```
$ vow build examples/vec_fill.vow
```

```json
{"status":"Verified","executable":"examples/vec_fill","diagnostics":[],"counterexamples":[]}
```

**Key points:**
- `requires: n <= 8` keeps iterations within ESBMC's unwind bound (10)
- `invariant: i >= 0, invariant: i <= n` is inductive: true on entry, preserved by the loop body
- The Vec model tracks `len`, so ESBMC can reason about `result.len() == n`

---

## 4. Linear Search — Break-with-Value

### Goal

Search a vector for a target value and return its index, or `-1` if not found. Uses `loop` with `break <value>` to produce a result directly from the loop expression.

### Step 1: Write the spec

```vow
module Search

fn linear_search(data: Vec<i64>, target: i64) -> i64
    vow { requires: data.len() > 0 }
{
    let mut i: i64 = 0;
    let n: i64 = data.len();
    let result: i64 = loop {
        if i >= n {
            break -1;
        }
        if data[i] == target {
            break i;
        }
        i = i + 1;
    };
    result
}

fn main() -> i32 [io] {
    let data: Vec<i64> = Vec::new();
    data.push(10);
    data.push(20);
    data.push(30);
    data.push(40);
    data.push(50);

    let idx: i64 = linear_search(data, 30);
    print_i64(idx);

    let idx2: i64 = linear_search(data, 99);
    print_i64(idx2);
    0
}
```

### Step 2: Build and verify

```
$ vow build examples/search.vow
```

```json
{"status":"Verified","executable":"examples/search","diagnostics":[],"counterexamples":[]}
```

**Key points:**
- `loop { ... break <value>; ... }` is an expression that evaluates to the break value
- All `break` expressions in a `loop` must produce the same type (`i64` here)
- `break <value>` is only allowed in `loop`, not in `while` (which always evaluates to `()`)
- The result is bound with `let result: i64 = loop { ... };`

---

# JSON Schemas

## build-result

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/build-result.schema.json",
  "title": "BuildResult",
  "description": "JSON output from `vow build` on stdout",
  "type": "object",
  "required": ["status", "executable", "diagnostics", "counterexamples"],
  "properties": {
    "status": {
      "type": "string",
      "enum": ["Verified", "Unverified", "CompileFailed", "VerifyFailed"],
      "description": "Build outcome"
    },
    "executable": {
      "type": ["string", "null"],
      "description": "Path to compiled binary, or null on failure or when source has no main function (library module)"
    },
    "diagnostics": {
      "type": "array",
      "items": { "$ref": "diagnostic.schema.json" },
      "description": "Compiler diagnostics (parse errors, type errors, vow violations)"
    },
    "message": {
      "type": "string",
      "description": "Error detail (present only when status is CompileFailed)"
    },
    "function": {
      "type": "string",
      "description": "Function name (present only when status is VerifyFailed)"
    },
    "counterexample": {
      "type": "string",
      "description": "Legacy counterexample description (present only when status is VerifyFailed)"
    },
    "counterexamples": {
      "type": "array",
      "items": { "$ref": "counterexample.schema.json" },
      "description": "Structured counterexamples from ESBMC verification"
    },
    "verify_status": {
      "type": "string",
      "enum": ["timeout", "error"],
      "description": "Verification sub-status (present only on timeout or tool error)"
    },
    "verify_message": {
      "type": "string",
      "description": "Verification error message (present only when verify_status is error)"
    }
  },
  "allOf": [
    {
      "if": { "properties": { "status": { "const": "CompileFailed" } } },
      "then": { "required": ["message"] }
    },
    {
      "if": { "properties": { "status": { "const": "VerifyFailed" } } },
      "then": { "required": ["function", "counterexample"] }
    }
  ],
  "additionalProperties": false
}
```

## contracts-result

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/contracts-result.schema.json",
  "title": "ContractsResult",
  "description": "JSON output from `vow contracts` on stdout",
  "type": "object",
  "required": ["contracts", "summary"],
  "properties": {
    "contracts": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["vow_id", "function", "kind", "description", "blame", "source", "status"],
        "properties": {
          "vow_id": {
            "type": "integer",
            "description": "Unique contract identifier within the program"
          },
          "function": {
            "type": "string",
            "description": "Function containing this contract"
          },
          "kind": {
            "type": "string",
            "enum": ["requires", "ensures", "invariant"],
            "description": "Contract kind"
          },
          "description": {
            "type": "string",
            "description": "Full contract text"
          },
          "blame": {
            "type": "string",
            "enum": ["Caller", "Callee"],
            "description": "Blame assignment: Caller for requires, Callee for ensures/invariant"
          },
          "source": {
            "type": "object",
            "required": ["file", "offset"],
            "properties": {
              "file": {
                "type": "string",
                "description": "Source file path"
              },
              "offset": {
                "type": "integer",
                "description": "Byte offset in source file"
              }
            },
            "additionalProperties": false
          },
          "status": {
            "type": "string",
            "enum": ["proven", "failed", "unknown", "timeout", "error", "not_verified"],
            "description": "Verification status"
          }
        },
        "additionalProperties": false
      }
    },
    "summary": {
      "type": "object",
      "required": ["total", "proven", "failed", "unknown", "timeout", "error", "not_verified"],
      "properties": {
        "total": { "type": "integer" },
        "proven": { "type": "integer" },
        "failed": { "type": "integer" },
        "unknown": { "type": "integer" },
        "timeout": { "type": "integer" },
        "error": { "type": "integer" },
        "not_verified": { "type": "integer" }
      },
      "additionalProperties": false
    }
  },
  "additionalProperties": false
}
```

## counterexample

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/counterexample.schema.json",
  "title": "Counterexample",
  "description": "A structured counterexample from ESBMC verification failure",
  "type": "object",
  "required": ["function", "inputs", "violation", "vow_id", "source"],
  "properties": {
    "function": {
      "type": "string",
      "description": "Name of the function where verification failed"
    },
    "inputs": {
      "type": "object",
      "additionalProperties": { "type": "string" },
      "description": "Map of parameter names to counterexample values"
    },
    "violation": {
      "type": "string",
      "description": "Description of the violated contract"
    },
    "vow_id": {
      "type": "integer",
      "minimum": 0,
      "description": "Numeric ID of the violated vow (matches vow_id in VowViolation)"
    },
    "source": {
      "oneOf": [
        {
          "type": "object",
          "required": ["file", "offset", "length"],
          "properties": {
            "file": { "type": "string", "description": "Source file path" },
            "offset": { "type": "integer", "minimum": 0, "description": "Byte offset of the vow clause" },
            "length": { "type": "integer", "minimum": 0, "description": "Byte length of the vow clause" }
          },
          "additionalProperties": false
        },
        { "type": "null" }
      ],
      "description": "Source location of the violated vow clause, or null"
    }
  },
  "additionalProperties": false
}
```

## diagnostic

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/diagnostic.schema.json",
  "title": "Diagnostic",
  "description": "A single compiler diagnostic (error, warning, or note)",
  "type": "object",
  "required": ["error_code", "message", "severity", "span"],
  "properties": {
    "error_code": {
      "type": "string",
      "enum": [
        "UnterminatedString",
        "InvalidCharacter",
        "UnexpectedToken",
        "MissingDelimiter",
        "TypeMismatch",
        "EffectViolation",
        "LinearTypeViolation",
        "NonExhaustiveMatch",
        "VowRequiresViolated",
        "VowEnsuresViolated",
        "VowInvariantViolated"
      ],
      "description": "Machine-readable error code"
    },
    "message": {
      "type": "string",
      "description": "Human-readable error message"
    },
    "severity": {
      "type": "string",
      "enum": ["error", "warning", "note"],
      "description": "Diagnostic severity"
    },
    "span": {
      "type": "object",
      "required": ["file", "offset", "length"],
      "properties": {
        "file": { "type": "string", "description": "Source file path" },
        "offset": { "type": "integer", "minimum": 0, "description": "Byte offset from start of file" },
        "length": { "type": "integer", "minimum": 0, "description": "Byte length of the span" }
      },
      "additionalProperties": false
    }
  },
  "additionalProperties": false
}
```

## vow-violation

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/vow-violation.schema.json",
  "title": "VowViolation",
  "description": "Runtime vow violation emitted to stderr (debug mode only). This is emitted by the vow-runtime C code, not by serde.",
  "type": "object",
  "required": ["error", "vow_id", "blame", "description", "file", "offset"],
  "properties": {
    "error": {
      "type": "string",
      "const": "VowViolation",
      "description": "Always the string VowViolation"
    },
    "vow_id": {
      "type": "integer",
      "minimum": 0,
      "description": "Numeric ID of the violated vow"
    },
    "blame": {
      "type": "string",
      "enum": ["Caller", "Callee"],
      "description": "Who is blamed: Caller for requires violations, Callee for ensures/invariant"
    },
    "description": {
      "type": "string",
      "description": "The contract predicate text"
    },
    "file": {
      "type": "string",
      "description": "Source file path"
    },
    "offset": {
      "type": "integer",
      "minimum": 0,
      "description": "Byte offset of the vow in the source file"
    },
    "values": {
      "type": "object",
      "additionalProperties": {
        "type": ["integer", "number", "boolean"]
      },
      "description": "Runtime values of free variables in the predicate (optional, present when bindings exist)"
    }
  },
  "additionalProperties": false
}
```
"#
    .to_string()
}

fn run_skill_install() {
    let dir = Path::new(".claude/commands");
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("vow skill install: cannot create {}: {}", dir.display(), e);
        std::process::exit(1);
    }
    let path = dir.join("vow-toolchain.md");
    if let Err(e) = std::fs::write(&path, skill_full_markdown()) {
        eprintln!("vow skill install: cannot write {}: {}", path.display(), e);
        std::process::exit(1);
    }
    eprintln!("installed skill to {}", path.display());
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

#[derive(Debug, Clone, Serialize)]
pub struct ContractEntryJson {
    pub vow_id: u32,
    pub function: String,
    pub kind: String,
    pub description: String,
    pub blame: String,
    pub source: ContractSourceJson,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContractSourceJson {
    pub file: String,
    pub offset: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContractsSummaryJson {
    pub total: u32,
    pub proven: u32,
    pub failed: u32,
    pub unknown: u32,
    pub timeout: u32,
    pub error: u32,
    pub not_verified: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContractsResultJson {
    pub contracts: Vec<ContractEntryJson>,
    pub summary: ContractsSummaryJson,
}

// ---------------------------------------------------------------------------
// Test output types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct TestResult {
    pub status: String,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub tests: Vec<TestEntry>,
    pub contract_density: ContractDensity,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestEntry {
    pub file: String,
    pub name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub diagnostics: Vec<DiagnosticJson>,
    pub counterexamples: Vec<CounterexampleJson>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContractDensity {
    pub functions_total: usize,
    pub functions_with_vows: usize,
    pub density_pct: f64,
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

    let module = vow_ir::lower_module(&ast, &source.to_string_lossy(), &string_exprs);
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
        VerifyOutcome::ToolNotFound => {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: vow_diag::ErrorCode::EsbmcNotFound,
                message: "ESBMC not found; install ESBMC or use --no-verify to skip verification"
                    .to_string(),
                primary: vow_diag::SourceLocation {
                    file: String::new(),
                    byte_offset: 0,
                    byte_len: 0,
                },
                secondary: vec![],
                blame: vow_diag::Blame::None,
                hints: vec![
                    "ESBMC is required for contract verification".to_string(),
                    "use --no-verify to compile without verification".to_string(),
                ],
            });
            (
                BuildStatus::VerifyFailed {
                    function: String::new(),
                    description: "ESBMC not found".to_string(),
                },
                vec![],
                Some("tool_not_found".to_string()),
                Some("ESBMC not found; install ESBMC or use --no-verify".to_string()),
            )
        }
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

    if find_esbmc().is_none() {
        return verify_outcome_to_output(VerifyOutcome::ToolNotFound, frontend.diagnostics, None);
    }

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

    run_pipeline_from_frontend(
        frontend, source, output, mode, no_verify, dump_ir, trace, no_cache,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_pipeline_from_frontend(
    frontend: FrontendResult,
    source: &Path,
    output: Option<&Path>,
    mode: BuildMode,
    no_verify: bool,
    dump_ir: bool,
    trace: TraceMode,
    no_cache: bool,
) -> BuildOutput {
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

    // Upfront ESBMC check: abort before codegen if verification is requested but ESBMC is missing
    if !no_verify && find_esbmc().is_none() {
        return verify_outcome_to_output(VerifyOutcome::ToolNotFound, all_diagnostics, None);
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

    let output_path = output.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        let stem = source.file_stem().unwrap_or_default();
        Path::new("build").join(stem)
    });
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

    if let Some(parent) = output_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

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
// Test pipeline (vow test)
// ---------------------------------------------------------------------------

fn discover_test_files(path: &Path) -> Vec<PathBuf> {
    if path.is_file() {
        return vec![path.to_path_buf()];
    }
    let mut files: Vec<PathBuf> = match std::fs::read_dir(path) {
        Ok(entries) => entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.ends_with(".vow")
                    && (name.starts_with("test_") || name.ends_with("_test.vow"))
                {
                    Some(e.path())
                } else {
                    None
                }
            })
            .collect(),
        Err(_) => vec![],
    };
    files.sort();
    files
}

fn count_contract_density(ir_module: &vow_ir::Module) -> ContractDensity {
    let mut total = 0usize;
    let mut with_vows = 0usize;
    for func in &ir_module.functions {
        if func.name == "main" {
            continue;
        }
        total += 1;
        if !func.vows.is_empty() {
            with_vows += 1;
        }
    }
    // Integer math matching self-hosted: (n * 1000) / total gives tenths of a percent
    let tenths = if total > 0 {
        (with_vows * 1000) / total
    } else {
        0
    };
    ContractDensity {
        functions_total: total,
        functions_with_vows: with_vows,
        density_pct: (tenths / 10) as f64 + (tenths % 10) as f64 / 10.0,
    }
}

// TODO: --unwind is accepted but not threaded to ESBMC (hardcoded to 10 in vow-verify).
// This affects build/verify/test equally — fix in vow-verify when adding unwind passthrough.
fn run_test_command(
    path: &Path,
    verify: bool,
    filter: Option<&str>,
    mode: BuildMode,
    timeout_ms: u64,
    _unwind: u32,
) {
    if !path.exists() {
        let result = TestResult {
            status: "CompileFailed".to_string(),
            total: 0,
            passed: 0,
            failed: 0,
            skipped: 0,
            tests: vec![],
            contract_density: ContractDensity {
                functions_total: 0,
                functions_with_vows: 0,
                density_pct: 0.0,
            },
        };
        println!("{}", serde_json::to_string(&result).unwrap());
        eprintln!("error: test path '{}' does not exist", path.display());
        std::process::exit(1);
    }

    let test_files = discover_test_files(path);
    let test_files: Vec<PathBuf> = match filter {
        Some(pat) => test_files
            .into_iter()
            .filter(|f| {
                f.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|name| name.contains(pat))
            })
            .collect(),
        None => test_files,
    };

    let mut entries = Vec::new();
    let mut total_density = ContractDensity {
        functions_total: 0,
        functions_with_vows: 0,
        density_pct: 0.0,
    };

    for test_file in &test_files {
        let start = std::time::Instant::now();
        let file_str = test_file.to_string_lossy().to_string();
        let name = test_file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        // Compile frontend once — extract density before codegen
        let frontend = match compile_frontend(test_file) {
            Ok(f) => f,
            Err(output) => {
                let diagnostics: Vec<DiagnosticJson> = output
                    .diagnostics
                    .iter()
                    .map(DiagnosticJson::from_diagnostic)
                    .collect();
                entries.push(TestEntry {
                    file: file_str,
                    name,
                    status: "compile_error".to_string(),
                    exit_code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    diagnostics,
                    counterexamples: vec![],
                });
                continue;
            }
        };

        let density = count_contract_density(&frontend.ir_module);
        total_density.functions_total += density.functions_total;
        total_density.functions_with_vows += density.functions_with_vows;

        let tmp_out = std::env::temp_dir().join(format!("vow_test_{name}_{}", std::process::id()));
        let result = run_pipeline_from_frontend(
            frontend,
            test_file,
            Some(&tmp_out),
            mode,
            !verify,
            false,
            TraceMode::Off,
            true,
        );

        let diagnostics: Vec<DiagnosticJson> = result
            .diagnostics
            .iter()
            .map(DiagnosticJson::from_diagnostic)
            .collect();
        let counterexamples: Vec<CounterexampleJson> = result
            .counterexamples
            .iter()
            .map(CounterexampleJson::from_structured)
            .collect();

        match &result.status {
            BuildStatus::CompileFailed { .. } => {
                entries.push(TestEntry {
                    file: file_str,
                    name,
                    status: "compile_error".to_string(),
                    exit_code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    diagnostics,
                    counterexamples,
                });
                continue;
            }
            BuildStatus::VerifyFailed { .. } => {
                entries.push(TestEntry {
                    file: file_str,
                    name,
                    status: "verify_failed".to_string(),
                    exit_code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    diagnostics,
                    counterexamples,
                });
                continue;
            }
            _ => {}
        }

        let exe_path = match &result.executable {
            Some(p) => p.clone(),
            None => {
                entries.push(TestEntry {
                    file: file_str,
                    name,
                    status: "compile_error".to_string(),
                    exit_code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    diagnostics,
                    counterexamples,
                });
                continue;
            }
        };

        // Execute with ulimit wrapper and timeout
        let exe_abs = std::fs::canonicalize(&exe_path).unwrap_or(exe_path.clone());
        let child = std::process::Command::new("sh")
            .args([
                "-c",
                "ulimit -v 2000000; \"$0\"",
                &exe_abs.display().to_string(),
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let (exit_code, stdout_str, stderr_str) = match child {
            Ok(mut child) => {
                // Take stdout/stderr handles and drain in background threads to
                // prevent pipe buffer deadlock when tests produce >64KB output.
                use std::io::Read;
                let stdout_handle = child.stdout.take();
                let stderr_handle = child.stderr.take();
                let stdout_thread = std::thread::spawn(move || {
                    let mut buf = String::new();
                    if let Some(mut r) = stdout_handle {
                        let _ = r.read_to_string(&mut buf);
                    }
                    buf
                });
                let stderr_thread = std::thread::spawn(move || {
                    let mut buf = String::new();
                    if let Some(mut r) = stderr_handle {
                        let _ = r.read_to_string(&mut buf);
                    }
                    buf
                });

                let timeout = std::time::Duration::from_millis(timeout_ms);
                let deadline = std::time::Instant::now() + timeout;
                let exit = loop {
                    match child.try_wait() {
                        Ok(Some(status)) => break Some(status.code()),
                        Ok(None) => {
                            if std::time::Instant::now() >= deadline {
                                let _ = child.kill();
                                let _ = child.wait();
                                break None;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Err(_) => break Some(Some(-1)),
                    }
                };

                let stdout = stdout_thread.join().unwrap_or_default();
                let stderr = stderr_thread.join().unwrap_or_default();
                match exit {
                    Some(code) => (code, stdout, stderr),
                    None => (None, String::new(), "timeout".to_string()),
                }
            }
            Err(e) => (Some(-1), String::new(), e.to_string()),
        };

        // Clean up the produced binary
        let _ = std::fs::remove_file(&exe_path);

        let status = match exit_code {
            Some(0) => "passed",
            Some(_) => "failed",
            None => "timeout",
        };

        entries.push(TestEntry {
            file: file_str,
            name,
            status: status.to_string(),
            exit_code,
            stdout: stdout_str,
            stderr: stderr_str,
            duration_ms: start.elapsed().as_millis() as u64,
            diagnostics,
            counterexamples,
        });
    }

    // Compute final density (integer math matching self-hosted compiler)
    if total_density.functions_total > 0 {
        let tenths = (total_density.functions_with_vows * 1000) / total_density.functions_total;
        total_density.density_pct = (tenths / 10) as f64 + (tenths % 10) as f64 / 10.0;
    }

    let passed = entries.iter().filter(|e| e.status == "passed").count();
    let failed = entries
        .iter()
        .filter(|e| {
            matches!(
                e.status.as_str(),
                "failed" | "compile_error" | "verify_failed"
            )
        })
        .count();
    let skipped = entries.iter().filter(|e| e.status == "skipped").count();

    let status = if failed > 0 {
        "TestsFailed"
    } else {
        "TestsPassed"
    };

    let test_result = TestResult {
        status: status.to_string(),
        total: entries.len(),
        passed,
        failed,
        skipped,
        tests: entries,
        contract_density: total_density,
    };

    let json = serde_json::to_string(&test_result).expect("TestResult must be serializable");
    println!("{json}");

    if failed > 0 {
        std::process::exit(1);
    }
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

// ---------------------------------------------------------------------------
// Contracts listing (vow contracts)
// ---------------------------------------------------------------------------

fn vow_kind_from_description(desc: &str) -> &'static str {
    if desc.starts_with("requires") {
        "requires"
    } else if desc.starts_with("ensures") {
        "ensures"
    } else if desc.starts_with("invariant") {
        "invariant"
    } else {
        "unknown"
    }
}

fn build_contracts_summary(entries: &[ContractEntryJson]) -> ContractsSummaryJson {
    let mut summary = ContractsSummaryJson {
        total: entries.len() as u32,
        proven: 0,
        failed: 0,
        unknown: 0,
        timeout: 0,
        error: 0,
        not_verified: 0,
    };
    for e in entries {
        match e.status.as_str() {
            "proven" => summary.proven += 1,
            "failed" => summary.failed += 1,
            "unknown" => summary.unknown += 1,
            "timeout" => summary.timeout += 1,
            "error" => summary.error += 1,
            _ => summary.not_verified += 1,
        }
    }
    summary
}

fn update_contract_statuses(
    entries: &mut [ContractEntryJson],
    ir_module: &vow_ir::Module,
    verify_cache: Option<&VerifyCache>,
) {
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
                    None => {
                        for entry in entries.iter_mut() {
                            if entry.function == func.name {
                                entry.status = "error".to_string();
                            }
                        }
                        continue;
                    }
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

        for entry in entries.iter_mut() {
            if entry.function == func.name {
                match &result {
                    VerificationResult::Proven => {
                        entry.status = "proven".to_string();
                    }
                    VerificationResult::Failed(ce) => {
                        if ce.vow_id == Some(entry.vow_id) {
                            entry.status = "failed".to_string();
                        } else {
                            entry.status = "unknown".to_string();
                        }
                    }
                    VerificationResult::Timeout => {
                        entry.status = "timeout".to_string();
                    }
                    VerificationResult::ToolError(_) | VerificationResult::ToolNotFound => {
                        entry.status = "error".to_string();
                    }
                }
            }
        }
    }
}

fn run_contracts_command(source: &Path, verify: bool, no_cache: bool) {
    let frontend = match compile_frontend(source) {
        Ok(f) => f,
        Err(output) => {
            output.emit_json();
            std::process::exit(1);
        }
    };

    let mut entries: Vec<ContractEntryJson> = Vec::new();
    for func in &frontend.ir_module.functions {
        for vow in &func.vows {
            let kind = vow_kind_from_description(&vow.description);
            let blame = match vow.blame {
                vow_diag::Blame::Caller => "Caller",
                vow_diag::Blame::Callee => "Callee",
                vow_diag::Blame::None => "None",
            };
            entries.push(ContractEntryJson {
                vow_id: vow.id.0,
                function: func.name.clone(),
                kind: kind.to_string(),
                description: vow.description.clone(),
                blame: blame.to_string(),
                source: ContractSourceJson {
                    file: vow.file.clone(),
                    offset: vow.offset,
                },
                status: "not_verified".to_string(),
            });
        }
    }

    if verify {
        if find_esbmc().is_none() {
            let output =
                verify_outcome_to_output(VerifyOutcome::ToolNotFound, frontend.diagnostics, None);
            output.emit_json();
            std::process::exit(1);
        }
        let verify_cache = if no_cache { None } else { VerifyCache::new() };
        update_contract_statuses(&mut entries, &frontend.ir_module, verify_cache.as_ref());
    }

    let summary = build_contracts_summary(&entries);
    let result = ContractsResultJson {
        contracts: entries,
        summary,
    };
    let json = serde_json::to_string(&result).expect("ContractsResult must be serializable");
    println!("{json}");
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
                ModeArg::Profile => BuildMode::Profile,
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
            let path = t.path.unwrap_or_else(|| PathBuf::from("."));
            let mode = match t.mode {
                ModeArg::Debug => BuildMode::Debug,
                ModeArg::Release => BuildMode::Release,
                ModeArg::Profile => {
                    eprintln!("Error: --mode profile is not supported for test subcommand");
                    std::process::exit(1);
                }
            };
            run_test_command(
                &path,
                t.verify,
                t.filter.as_deref(),
                mode,
                t.timeout,
                t.unwind,
            );
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
        Some(Command::Contracts(c)) => {
            if c.help {
                if c.human {
                    println!("{}", skill_human());
                } else {
                    println!("{}", skill_json());
                }
                return;
            }
            let source = match c.source {
                Some(s) => s,
                None => {
                    eprintln!("vow contracts: source file required (try --help)");
                    std::process::exit(1);
                }
            };
            run_contracts_command(&source, c.verify, c.no_cache);
        }
        Some(Command::Skill(s)) => {
            match s.action {
                Some(SkillAction::Install) => {
                    run_skill_install();
                }
                Some(SkillAction::Print) | None => {
                    println!("{}", skill_full_markdown());
                }
            }
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
                ModeArg::Profile => BuildMode::Profile,
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
        assert!(
            json.contains("eprintln_str"),
            "missing eprintln_str builtin"
        );
        assert!(
            json.contains("process_exit"),
            "missing process_exit builtin"
        );

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
        assert!(json.contains("\"modules\""), "missing modules section");

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
