mod cache;
mod frontend;
mod module_loader;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;

use std::collections::BTreeMap;

use clap::Parser;
use serde::Serialize;
use vow_codegen::cranelift_backend::CraneliftBackend;
use vow_codegen::linker::{find_runtime_lib, find_shim_lib, link};
use vow_codegen::{Backend, BuildMode, TraceMode};
use vow_diag::{Diagnostic, DiagnosticEmitter, HumanEmitter, Severity};
use vow_verify::{
    ConstantValue, Counterexample, DEFAULT_ESBMC_MEMLIMIT_MB, DEFAULT_MAX_K_STEP, Encoding, Solver,
    SolverConfig, UNSUPPORTED_OP_VOW_ID, VerificationResult, VerifyLimits,
    detect_constant_functions, emit_verify_c_source, find_esbmc, non_modelable_reason,
    run_with_fallback, verify_function_with_module_and_const_fns_configured,
};

use cache::{CachedFailure, VerifyCache};
use frontend::{
    FrontendBundle, FrontendError, FrontendGoal, prepare_frontend, prepare_frontend_with_root,
};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum ModeArg {
    Debug,
    Release,
    Profile,
    Sanitize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum TraceArg {
    Off,
    Calls,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum SolverArg {
    Boolector,
    Z3,
    Bitwuzla,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum EncodingArg {
    Bv,
    Ir,
    Auto,
}

fn make_solver_config(
    solver: SolverArg,
    encoding: EncodingArg,
    timeout: Option<u32>,
) -> SolverConfig {
    let s = match solver {
        SolverArg::Boolector => Solver::Boolector,
        SolverArg::Z3 => Solver::Z3,
        SolverArg::Bitwuzla => Solver::Bitwuzla,
        SolverArg::Auto => Solver::Auto,
    };
    let e = match encoding {
        EncodingArg::Bv => Encoding::Bv,
        EncodingArg::Ir => Encoding::Ir,
        EncodingArg::Auto => Encoding::Auto,
    };
    let config = SolverConfig {
        solver: s,
        encoding: e,
        timeout_secs: timeout,
        memlimit_mb: Some(DEFAULT_ESBMC_MEMLIMIT_MB),
    };
    if let Err(msg) = config.validate() {
        eprintln!("error: {msg}");
        std::process::exit(1);
    }
    config
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
    #[arg(long, default_value_t = DEFAULT_MAX_K_STEP)]
    max_k_step: u32,
    #[arg(long, value_enum, default_value = "auto")]
    solver: SolverArg,
    #[arg(long, value_enum, default_value = "auto")]
    encoding: EncodingArg,
    #[arg(long)]
    timeout: Option<u32>,
    #[arg(long)]
    verify_jobs: Option<u32>,
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
    /// Run mutation testing on a Vow source tree (self-hosted only)
    Mutants(MutantsArgs),
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
    #[arg(long, default_value_t = DEFAULT_MAX_K_STEP)]
    max_k_step: u32,
    #[arg(long, value_enum, default_value = "auto")]
    solver: SolverArg,
    #[arg(long, value_enum, default_value = "auto")]
    encoding: EncodingArg,
    #[arg(long)]
    timeout: Option<u32>,
    #[arg(long)]
    verify_jobs: Option<u32>,
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
    #[arg(long, default_value_t = DEFAULT_MAX_K_STEP)]
    max_k_step: u32,
    #[arg(long, value_enum, default_value = "auto")]
    solver: SolverArg,
    #[arg(long, value_enum, default_value = "auto")]
    encoding: EncodingArg,
    #[arg(long)]
    timeout: Option<u32>,
    #[arg(long)]
    verify_jobs: Option<u32>,
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
    /// Resolve `use` declarations against this directory instead of each
    /// test file's parent. Use when running a single test file that lives
    /// in a subdirectory: `vow test compiler/tests/test_x.vow --module-root compiler`.
    #[arg(long)]
    module_root: Option<PathBuf>,
    /// Build mode (debug enables runtime vow checks)
    #[arg(long, value_enum, default_value = "debug")]
    mode: ModeArg,
    /// Per-test execution timeout in milliseconds
    #[arg(long, default_value = "30000")]
    timeout: u64,
    /// ESBMC max k-induction step (only with --verify)
    #[arg(long, default_value_t = DEFAULT_MAX_K_STEP)]
    max_k_step: u32,
    #[arg(long)]
    verify_jobs: Option<u32>,
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
    max_k_step: Option<u32>,
    #[arg(long, value_enum, default_value = "auto")]
    solver: SolverArg,
    #[arg(long, value_enum, default_value = "auto")]
    encoding: EncodingArg,
    #[arg(long)]
    timeout: Option<u32>,
    /// Accepted for CLI parity with build/verify/test; ignored because
    /// `update_contract_statuses` has no pool wiring yet (see #175 follow-ups).
    #[arg(long)]
    verify_jobs: Option<u32>,
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
    #[arg(long)]
    help: bool,
    #[arg(long)]
    human: bool,
}

#[derive(clap::Subcommand, Debug)]
enum SkillAction {
    /// Print the skill document to stdout (default)
    Print {
        /// Print the full self-contained bundle for raw API harnesses
        #[arg(long)]
        bundle: bool,
    },
    /// Install the skill to .claude/skills/vow/
    Install {
        /// Install into the current git project's .claude/ directory
        #[arg(long)]
        local: bool,
        /// Install into $HOME/.claude/ on Linux
        #[arg(long)]
        global: bool,
    },
}

#[derive(clap::Args, Debug)]
#[command(
    disable_help_flag = true,
    trailing_var_arg = true,
    allow_hyphen_values = true
)]
struct MutantsArgs {
    /// All remaining arguments forwarded verbatim
    args: Vec<String>,
    // `help` and `human` are absorbed by clap so flags like `--help` don't
    // surface as parse errors; the handler below ignores them and prints
    // a fixed redirect to the self-hosted compiler regardless.
    #[arg(long)]
    help: bool,
    #[arg(long)]
    human: bool,
}

// ---------------------------------------------------------------------------
// --help skill output
// ---------------------------------------------------------------------------

// GENERATE:SKILL_JSON:START
fn skill_json() -> String {
    r##"{
  "schema_version": "2",
  "kind": "tool_help",
  "tool": "vow",
  "audience": "agent",
  "default_format": "json",
  "description": "Vow compiler: compiles Vow source to native executables with contract verification",
  "usage": "vow <command> [OPTIONS] <source.vow>",
  "legacy_usage": "vow [OPTIONS] <source.vow> (equivalent to vow build)",
  "references": {
    "grammar": "reference/grammar.md",
    "cli": "reference/cli.md",
    "contracts": "reference/contracts.md",
    "errors": "reference/errors.md",
    "examples": "examples/examples.md",
    "schemas": {
      "build_result": "schemas/build-result.schema.json",
      "contracts_result": "schemas/contracts-result.schema.json",
      "diagnostic": "schemas/diagnostic.schema.json",
      "counterexample": "schemas/counterexample.schema.json",
      "mutants_result": "schemas/mutants-result.schema.json",
      "test_result": "schemas/test-result.schema.json",
      "vow_violation": "schemas/vow-violation.schema.json"
    }
  },
  "invocation": {
    "canonical": "vow <command> [OPTIONS] <source.vow>",
    "default_command": "build",
    "legacy_equivalent": "vow [OPTIONS] <source.vow>",
    "source_argument": {
      "name": "source",
      "kind": "path",
      "required": true,
      "suffix": ".vow"
    }
  },
  "commands": {
    "build": "Compile source to native executable (verifies by default; use --no-verify to skip)",
    "verify": "Verify contracts without producing an executable (use --no-cache to skip cache)",
    "test": "Run tests: discover, compile, execute test_*.vow files with JSON results",
    "decl": "Emit declaration file (.vow.d) with type signatures only",
    "contracts": "List all contracts with optional verification status",
    "skill": "Generate or install the Claude Code skill document for this compiler version"
  },
  "command_details": {
    "build": {
      "status": "implemented",
      "usage": "vow build [OPTIONS] <source.vow>",
      "default_when_command_omitted": true,
      "arguments": [
        {
          "name": "source",
          "kind": "path",
          "required": true,
          "suffix": ".vow"
        }
      ],
      "options": [
        {
          "form": "-o, --output <path>",
          "description": "Output executable path (default: source without .vow extension)",
          "short": "-o",
          "long": "--output",
          "value_name": "path",
          "value_kind": "path",
          "default": "source without .vow extension"
        },
        {
          "form": "--mode <debug|release|profile|sanitize>",
          "description": "Build mode: debug inserts runtime vow checks, profile inserts call counters and prints report on normal exit, sanitize adds debug checks + Vec provenance tracking (default: release)",
          "long": "--mode",
          "value_name": "debug|release|profile|sanitize",
          "value_kind": "enum",
          "values": [
            "debug",
            "release",
            "profile",
            "sanitize"
          ],
          "default": "release"
        },
        {
          "form": "--no-verify",
          "description": "Skip ESBMC static verification",
          "long": "--no-verify",
          "value_kind": "flag"
        },
        {
          "form": "--dump-ir",
          "description": "Print IR text to stdout and exit (no JSON output, no codegen)",
          "long": "--dump-ir",
          "value_kind": "flag"
        },
        {
          "form": "--debug-trace <off|calls|full>",
          "description": "Emit JSON trace lines to stderr at runtime (default: off)",
          "long": "--debug-trace",
          "value_name": "trace",
          "value_kind": "enum",
          "values": [
            "off",
            "calls",
            "full"
          ],
          "default": "off"
        },
        {
          "form": "--no-cache",
          "description": "Disable verification result caching, and (for --no-verify builds) the compile-object cache. See \"Compile-object cache behavior\" below",
          "long": "--no-cache",
          "value_kind": "flag"
        },
        {
          "form": "--max-k-step <N>",
          "description": "ESBMC incremental BMC max iterations (default: 50)",
          "long": "--max-k-step",
          "value_name": "N",
          "value_kind": "integer",
          "default": 50
        },
        {
          "form": "--solver <boolector|z3|bitwuzla|auto>",
          "description": "ESBMC SMT solver; auto selects per-function via heuristic (default: auto)",
          "long": "--solver",
          "value_name": "boolector|z3|bitwuzla|auto",
          "value_kind": "enum",
          "values": [
            "boolector",
            "z3",
            "bitwuzla",
            "auto"
          ],
          "default": "auto"
        },
        {
          "form": "--encoding <bv|ir|auto>",
          "description": "ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 (default: auto)",
          "long": "--encoding",
          "value_name": "bv|ir|auto",
          "value_kind": "enum",
          "values": [
            "bv",
            "ir",
            "auto"
          ],
          "default": "auto"
        },
        {
          "form": "--timeout <N>",
          "description": "ESBMC per-function timeout in seconds. Under --encoding auto, a 30s default is applied so the BV-timeout fallback to --encoding ir --solver z3 can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit --timeout overrides both. --timeout 0 is honoured as an immediate watchdog kill (default: 300 (or 30 when --encoding is auto))",
          "long": "--timeout",
          "value_name": "N",
          "value_kind": "integer",
          "default": "300 (or 30 when --encoding is auto)"
        },
        {
          "form": "--verify-jobs <N>",
          "description": "Max concurrent ESBMC verification jobs (default: num_cpus/2)",
          "long": "--verify-jobs",
          "value_name": "N",
          "value_kind": "integer",
          "default": "num_cpus/2"
        }
      ],
      "stdout": {
        "format": "json",
        "schema_ref": "schemas/build-result.schema.json",
        "suppressed_by": [
          "--dump-ir"
        ]
      },
      "stderr": {
        "channels": [
          "diagnostic stream",
          "debug trace"
        ],
        "debug_trace_flag": "--debug-trace <off|calls|full>"
      },
      "notes": [
        "verification is enabled by default",
        "debug mode inserts runtime vow checks"
      ]
    },
    "verify": {
      "status": "implemented",
      "usage": "vow verify [OPTIONS] <source.vow>",
      "arguments": [
        {
          "name": "source",
          "kind": "path",
          "required": true,
          "suffix": ".vow"
        }
      ],
      "options": [
        {
          "form": "--no-cache",
          "description": "Disable verification result caching",
          "long": "--no-cache",
          "value_kind": "flag"
        },
        {
          "form": "--max-k-step <N>",
          "description": "ESBMC incremental BMC max iterations (default: 50)",
          "long": "--max-k-step",
          "value_name": "N",
          "value_kind": "integer",
          "default": 50
        },
        {
          "form": "--solver <boolector|z3|bitwuzla|auto>",
          "description": "ESBMC SMT solver; auto selects per-function via heuristic (default: auto)",
          "long": "--solver",
          "value_name": "boolector|z3|bitwuzla|auto",
          "value_kind": "enum",
          "values": [
            "boolector",
            "z3",
            "bitwuzla",
            "auto"
          ],
          "default": "auto"
        },
        {
          "form": "--encoding <bv|ir|auto>",
          "description": "ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 (default: auto)",
          "long": "--encoding",
          "value_name": "bv|ir|auto",
          "value_kind": "enum",
          "values": [
            "bv",
            "ir",
            "auto"
          ],
          "default": "auto"
        },
        {
          "form": "--timeout <N>",
          "description": "ESBMC per-function timeout in seconds. Under --encoding auto, a 30s default is applied so the BV-timeout fallback to --encoding ir --solver z3 can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit --timeout overrides both. --timeout 0 is honoured as an immediate watchdog kill (default: 300 (or 30 when --encoding is auto))",
          "long": "--timeout",
          "value_name": "N",
          "value_kind": "integer",
          "default": "300 (or 30 when --encoding is auto)"
        },
        {
          "form": "--verify-jobs <N>",
          "description": "Max concurrent ESBMC verification jobs (default: num_cpus/2)",
          "long": "--verify-jobs",
          "value_name": "N",
          "value_kind": "integer",
          "default": "num_cpus/2"
        }
      ],
      "stdout": {
        "format": "json",
        "schema_ref": "schemas/build-result.schema.json",
        "fixed_fields": {
          "executable": null
        }
      },
      "notes": [
        "runs verification only and never emits a binary"
      ]
    },
    "test": {
      "status": "implemented",
      "usage": "vow test [OPTIONS] [<path>]",
      "arguments": [
        {
          "name": "path",
          "kind": "path",
          "required": false,
          "default": ".",
          "description": "Directory to scan or single .vow file"
        }
      ],
      "options": [
        {
          "form": "--verify",
          "description": "Run ESBMC verification on test files",
          "long": "--verify",
          "value_kind": "flag"
        },
        {
          "form": "--filter <pat>",
          "description": "Only run tests whose file stem contains pat (default: (none))",
          "long": "--filter",
          "value_name": "pat",
          "value_kind": "string",
          "default": "(none)"
        },
        {
          "form": "--module-root <path>",
          "description": "Resolve use declarations against <path>. Defaults to the scan path when it's a directory, otherwise the entry file's parent directory. (default: (auto))",
          "long": "--module-root",
          "value_name": "path",
          "value_kind": "string",
          "default": "(auto)"
        },
        {
          "form": "--mode <debug|release>",
          "description": "Build mode; debug inserts runtime vow checks (default: (default))",
          "long": "--mode",
          "value_name": "mode",
          "value_kind": "enum",
          "values": [
            "debug",
            "release"
          ],
          "default": "(default)"
        },
        {
          "form": "--timeout <ms>",
          "description": "Per-test execution timeout in milliseconds (default: 30000)",
          "long": "--timeout",
          "value_name": "ms",
          "value_kind": "string",
          "default": "30000"
        },
        {
          "form": "--max-k-step <N>",
          "description": "ESBMC incremental BMC max iterations (with --verify)",
          "long": "--max-k-step",
          "value_name": "N",
          "value_kind": "integer",
          "default": 50
        },
        {
          "form": "--verify-jobs <N>",
          "description": "Max concurrent ESBMC verification jobs (with --verify)",
          "long": "--verify-jobs",
          "value_name": "N",
          "value_kind": "integer",
          "default": "num_cpus/2"
        }
      ],
      "stdout": {
        "format": "json"
      },
      "notes": [
        "discovers test_*.vow and *_test.vow files",
        "each test must contain main() -> i32 returning 0 on success",
        "default mode is debug (runtime vow checks enabled)"
      ]
    },
    "decl": {
      "status": "implemented",
      "usage": "vow decl [OPTIONS] <source.vow>",
      "arguments": [
        {
          "name": "source",
          "kind": "path",
          "required": true,
          "suffix": ".vow"
        }
      ],
      "options": [
        {
          "form": "-o, --output <path>",
          "description": "Output declaration file path (default: <source>.vow.d)",
          "short": "-o",
          "long": "--output",
          "value_name": "path",
          "value_kind": "path",
          "default": "<source>.vow.d"
        }
      ],
      "stdout": {
        "format": "none"
      },
      "side_effects": [
        {
          "kind": "write_file",
          "default_path": "<source>.vow.d"
        }
      ]
    },
    "contracts": {
      "status": "implemented",
      "usage": "vow contracts [OPTIONS] <source.vow>",
      "arguments": [
        {
          "name": "source",
          "kind": "path",
          "required": true,
          "suffix": ".vow"
        }
      ],
      "options": [
        {
          "form": "--verify",
          "description": "Run ESBMC verification and report per-contract status",
          "long": "--verify",
          "value_kind": "flag"
        },
        {
          "form": "--no-cache",
          "description": "Disable verification result caching",
          "long": "--no-cache",
          "value_kind": "flag"
        },
        {
          "form": "--max-k-step <N>",
          "description": "ESBMC incremental BMC max iterations (default: 50)",
          "long": "--max-k-step",
          "value_name": "N",
          "value_kind": "integer",
          "default": 50
        },
        {
          "form": "--solver <boolector|z3|bitwuzla|auto>",
          "description": "ESBMC SMT solver (with --verify)",
          "long": "--solver",
          "value_name": "boolector|z3|bitwuzla|auto",
          "value_kind": "enum",
          "values": [
            "boolector",
            "z3",
            "bitwuzla",
            "auto"
          ],
          "default": "auto"
        },
        {
          "form": "--encoding <bv|ir|auto>",
          "description": "ESBMC encoding mode (with --verify); ir requires z3 (default: auto)",
          "long": "--encoding",
          "value_name": "bv|ir|auto",
          "value_kind": "enum",
          "values": [
            "bv",
            "ir",
            "auto"
          ],
          "default": "auto"
        },
        {
          "form": "--verify-jobs <N>",
          "description": "Accepted for CLI parity with build/verify/test; currently a no-op (the contracts verifier is serial)",
          "long": "--verify-jobs",
          "value_name": "N",
          "value_kind": "integer",
          "default": "num_cpus/2"
        }
      ],
      "stdout": {
        "format": "json",
        "schema_ref": "schemas/contracts-result.schema.json"
      },
      "notes": [
        "runs frontend only by default",
        "use --verify for per-contract ESBMC status"
      ]
    }
  },
  "build_options": {
    "-o, --output <path>": "Output executable path (default: source without .vow extension)",
    "--mode <debug|release|profile|sanitize>": "Build mode: debug inserts runtime vow checks, profile inserts call counters and prints report on normal exit, sanitize adds debug checks + Vec provenance tracking (default: release)",
    "--no-verify": "Skip ESBMC static verification",
    "--dump-ir": "Print IR text to stdout and exit (no JSON output, no codegen)",
    "--debug-trace <off|calls|full>": "Emit JSON trace lines to stderr at runtime (default: off)",
    "--no-cache": "Disable verification result caching, and (for --no-verify builds) the compile-object cache. See \"Compile-object cache behavior\" below",
    "--max-k-step <N>": "ESBMC incremental BMC max iterations (default: 50)",
    "--solver <boolector|z3|bitwuzla|auto>": "ESBMC SMT solver; auto selects per-function via heuristic (default: auto)",
    "--encoding <bv|ir|auto>": "ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 (default: auto)",
    "--timeout <N>": "ESBMC per-function timeout in seconds. Under --encoding auto, a 30s default is applied so the BV-timeout fallback to --encoding ir --solver z3 can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit --timeout overrides both. --timeout 0 is honoured as an immediate watchdog kill (default: 300 (or 30 when --encoding is auto))",
    "--verify-jobs <N>": "Max concurrent ESBMC verification jobs (default: num_cpus/2)"
  },
  "verify_options": {
    "--no-cache": "Disable verification result caching",
    "--max-k-step <N>": "ESBMC incremental BMC max iterations (default: 50)",
    "--solver <boolector|z3|bitwuzla|auto>": "ESBMC SMT solver; auto selects per-function via heuristic (default: auto)",
    "--encoding <bv|ir|auto>": "ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 (default: auto)",
    "--timeout <N>": "ESBMC per-function timeout in seconds. Under --encoding auto, a 30s default is applied so the BV-timeout fallback to --encoding ir --solver z3 can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit --timeout overrides both. --timeout 0 is honoured as an immediate watchdog kill (default: 300 (or 30 when --encoding is auto))",
    "--verify-jobs <N>": "Max concurrent ESBMC verification jobs (default: num_cpus/2)"
  },
  "test_options": {
    "--verify": "Run ESBMC verification on test files",
    "--filter <pat>": "Only run tests whose file stem contains pat (default: (none))",
    "--module-root <path>": "Resolve use declarations against <path>. Defaults to the scan path when it's a directory, otherwise the entry file's parent directory. (default: (auto))",
    "--mode <debug|release>": "Build mode; debug inserts runtime vow checks (default: (default))",
    "--timeout <ms>": "Per-test execution timeout in milliseconds (default: 30000)",
    "--max-k-step <N>": "ESBMC incremental BMC max iterations (with --verify)",
    "--verify-jobs <N>": "Max concurrent ESBMC verification jobs (with --verify)"
  },
  "decl_options": {
    "-o, --output <path>": "Output declaration file path (default: <source>.vow.d)"
  },
  "contracts_options": {
    "--verify": "Run ESBMC verification and report per-contract status",
    "--no-cache": "Disable verification result caching",
    "--max-k-step <N>": "ESBMC incremental BMC max iterations (default: 50)",
    "--solver <boolector|z3|bitwuzla|auto>": "ESBMC SMT solver (with --verify)",
    "--encoding <bv|ir|auto>": "ESBMC encoding mode (with --verify); ir requires z3 (default: auto)",
    "--verify-jobs <N>": "Accepted for CLI parity with build/verify/test; currently a no-op (the contracts verifier is serial)"
  },
  "global_options": {
    "--help": "Emit versioned JSON tool-help data",
    "--help --human": "Emit legacy human-readable help (compatibility mode)"
  },
  "outputs": {
    "build_result": {
      "schema_ref": "schemas/build-result.schema.json",
      "emitted_by": [
        "build",
        "verify"
      ],
      "status_values": [
        "Verified",
        "Unverified",
        "Skipped",
        "CompileFailed",
        "VerifyFailed"
      ],
      "legacy_fields": [
        "counterexample"
      ]
    },
    "contracts_result": {
      "schema_ref": "schemas/contracts-result.schema.json",
      "emitted_by": [
        "contracts"
      ]
    },
    "diagnostic": {
      "schema_ref": "schemas/diagnostic.schema.json",
      "embedded_in": "build_result.diagnostics"
    },
    "runtime_vow_violation": {
      "schema_ref": "schemas/vow-violation.schema.json",
      "emitted_on": "stderr",
      "requires_mode": "debug"
    },
    "runtime_trace": {
      "emitted_on": "stderr",
      "enabled_by": "--debug-trace <off|calls|full>",
      "format": "jsonl"
    }
  },
  "output_json": {
    "status": "Verified | Unverified | Skipped | CompileFailed | VerifyFailed",
    "executable": "path to compiled binary, or null",
    "diagnostics": "[array of {error_code, message, severity, span: {file, offset, length}}]",
    "message": "error detail (CompileFailed)",
    "function": "function name (VerifyFailed)",
    "counterexample": "ESBMC counterexample description (VerifyFailed)"
  },
  "diagnostics": {
    "schema_ref": "schemas/diagnostic.schema.json",
    "fields": [
      "error_code",
      "message",
      "severity",
      "span.file",
      "span.offset",
      "span.length"
    ]
  },
  "exit_codes": {
    "0": "success (Verified or Unverified)",
    "1": "failure (CompileFailed, VerifyFailed, or Skipped)"
  },
  "language": {
    "module": "module <Name>",
    "use_declaration": "use foo.bar",
    "const_declaration": "const NAME: i64 = 1024",
    "comments": "// line comments only; block comments unsupported",
    "let_binding": "let name: Type = expr; or let mut name: Type = expr;",
    "function": "fn <name>(<params>) -> <RetTy> [<effects>] { <body> }",
    "public_function": "pub fn <name>(<params>) -> <RetTy> [<effects>] { <body> }",
    "vow_function": "fn <name>(<params>) -> <RetTy> vow { requires: <expr>; ensures: <expr> } { <body> }",
    "while_with_invariant": "while <cond> vow { invariant: <expr> } { <body> }",
    "literals": {
      "integer": "42 | -1 | 42u64 (unsuffixed integers default to i64)",
      "float": "3.14 | -0.5",
      "bool": "true | false",
      "string": "\"text\" with escapes \\n \\t \\r \\\\ \\\" \\0"
    },
    "casts": "x as u64 or y as i64",
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
      "HashMap<K, V>",
      "BTreeMap<K, V>"
    ],
    "effects": [
      "io",
      "read",
      "write",
      "panic",
      "unsafe"
    ],
    "builtins": {
      "pin_to_root": "fn(value: String) -> String and fn<T>(value: Vec<T>) -> Vec<T> for flat scalar T []",
      "print_str": "fn(s: String) -> () [io]",
      "print_i64": "fn(v: i64) -> () [io]",
      "print_u64": "fn(v: u64) -> () [io]",
      "eprintln_str": "fn(s: String) -> () [io]",
      "debug_str": "fn(s: String) -> () []",
      "debug_i64": "fn(v: i64) -> () []",
      "debug_u64": "fn(v: u64) -> () []",
      "fs_read": "fn(path: String) -> String [read]",
      "fs_open": "fn(path: String) -> i64 [read]",
      "fs_read_line": "fn(handle: i64) -> String [read]",
      "fs_status": "fn(handle: i64) -> i64 [read]",
      "fs_close": "fn(handle: i64) -> i64 [read]",
      "fs_write": "fn(path: String, data: String) -> i64 [write]",
      "fs_exists": "fn(path: String) -> i64 [read]",
      "fs_mkdir": "fn(path: String) -> i64 [io]",
      "fs_listdir": "fn(path: String) -> Vec<String> [read]",
      "fs_remove": "fn(path: String) -> i64 [io]",
      "fs_remove_dir": "fn(path: String) -> i64 [io]",
      "fs_is_dir": "fn(path: String) -> i64 [read]",
      "fs_is_symlink": "fn(path: String) -> i64 [read]",
      "fs_rename": "fn(old: String, new: String) -> i64 [io]",
      "string_substr": "fn(s: String, start: i64, len: i64) -> String []",
      "string_split": "fn(s: String, delim: String) -> Vec<String> []",
      "string_starts_with": "fn(s: String, prefix: String) -> i64 []",
      "string_ends_with": "fn(s: String, suffix: String) -> i64 []",
      "string_matches_literal_at": "fn(s: String, pos: i64, literal: String literal) -> i64 []",
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
      "num_cpus": "fn() -> i64 [io]",
      "memory_root_arena_bytes": "fn() -> u64 [io]",
      "memory_peak_bytes": "fn() -> u64 [io]",
      "memory_alloc_count_since_start": "fn() -> u64 [io]",
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
      "bitwise": [
        "&",
        "|",
        "^",
        "<<",
        ">>"
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
      "linear": "linear struct Name { field: Type, ... } \u2014 linear obligation must be consumed or returned before region close",
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
        "Vec::from_raw_parts_copy(ptr, len)",
        ".push(val)",
        ".pop()",
        ".len()",
        ".clear()",
        ".truncate(n)",
        "v[i]",
        "v[i] = val"
      ],
      "String": [
        "String::from(s)",
        "String::new()",
        "String::from_raw_parts_copy(ptr, len)",
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
      "BTreeMap<K,V>": [
        "BTreeMap::new()",
        ".insert(k, v)",
        ".get(k)",
        ".contains(k)",
        ".len()"
      ],
      "Option<T>": [
        ".unwrap()",
        "? operator"
      ]
    },
    "error_propagation": "? on Option<T> or Result<T, E> propagates None/Err to the caller",
    "indexing": {
      "read": "v[i] \u2014 Vec index access",
      "write": "v[i] = val \u2014 Vec index assignment"
    },
    "feature_status": {
      "implemented": {
        "function_vow_blocks": "requires / ensures / invariant",
        "where_clauses": "parameter-level refinement sugar",
        "loop_invariants": "simple invariant predicates"
      },
      "partial": {
        "refinement_type_predicates": "parsed but semantically erased; use where clauses or function vows for verification",
        "effect_tracking": "user-defined effect propagation is enforced; some builtin panic/unsafe effects are not yet modeled"
      },
      "target": {
        "module_level_vow_blocks": "specified in docs but not parsed or represented in the AST",
        "quantifiers": "forall / exists are not yet in the lexer or parser"
      },
      "unsupported": [
        "user-defined generics",
        "traits",
        "closures",
        "operator overloading",
        "macros",
        "assert / assume statements"
      ]
    }
  },
  "verification_defaults": {
    "strategy": "k-induction-parallel",
    "max_k_step": 50
  }
}"##
    .to_string()
}
// GENERATE:SKILL_JSON:END

// GENERATE:SKILL_HUMAN:START
fn skill_human() -> String {
    r##"vow — Vow compiler

USAGE
  vow build [OPTIONS] <source.vow>    Compile to native executable
  vow verify [OPTIONS] <source.vow>    Verify contracts only (no executable)
  vow test [OPTIONS] [<path>]          Run tests with JSON results
  vow contracts [OPTIONS] <source.vow> List all contracts
  vow decl [OPTIONS] <source.vow>    Emit declaration file (.vow.d)
  vow skill [print [--bundle]|install [--local|--global]]
                                        Generate or install Claude Code skill
  vow [OPTIONS] <source.vow>          Legacy mode (same as vow build)

BUILD OPTIONS
  -o, --output <path>     Output executable path (default: source without .vow extension)
  --mode <debug|release|profile|sanitize>  Build mode: debug inserts runtime vow checks, profile inserts call counters and prints report on normal exit, sanitize adds debug checks + Vec provenance tracking (default: release)
  --no-verify             Skip ESBMC static verification
  --dump-ir               Print IR text to stdout and exit (no JSON output, no codegen)
  --debug-trace <off|calls|full>  Emit JSON trace lines to stderr at runtime (default: off)
  --no-cache              Disable verification result caching, and (for --no-verify builds) the compile-object cache. See "Compile-object cache behavior" below
  --max-k-step <N>        ESBMC incremental BMC max iterations (default: 50)
  --solver <boolector|z3|bitwuzla|auto>  ESBMC SMT solver; auto selects per-function via heuristic (default: auto)
  --encoding <bv|ir|auto>  ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 (default: auto)
  --timeout <N>           ESBMC per-function timeout in seconds. Under --encoding auto, a 30s default is applied so the BV-timeout fallback to --encoding ir --solver z3 can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit --timeout overrides both. --timeout 0 is honoured as an immediate watchdog kill (default: 300 (or 30 when --encoding is auto))
  --verify-jobs <N>       Max concurrent ESBMC verification jobs (default: num_cpus/2)

VERIFY OPTIONS
  --no-cache              Disable verification result caching
  --max-k-step <N>        ESBMC incremental BMC max iterations (default: 50)
  --solver <boolector|z3|bitwuzla|auto>  ESBMC SMT solver; auto selects per-function via heuristic (default: auto)
  --encoding <bv|ir|auto>  ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 (default: auto)
  --timeout <N>           ESBMC per-function timeout in seconds. Under --encoding auto, a 30s default is applied so the BV-timeout fallback to --encoding ir --solver z3 can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit --timeout overrides both. --timeout 0 is honoured as an immediate watchdog kill (default: 300 (or 30 when --encoding is auto))
  --verify-jobs <N>       Max concurrent ESBMC verification jobs (default: num_cpus/2)

TEST OPTIONS
  --verify                Run ESBMC verification on test files
  --filter <pat>          Only run tests whose file stem contains pat (default: (none))
  --module-root <path>    Resolve use declarations against <path>. Defaults to the scan path when it's a directory, otherwise the entry file's parent directory. (default: (auto))
  --mode <debug|release>  Build mode; debug inserts runtime vow checks (default: (default))
  --timeout <ms>          Per-test execution timeout in milliseconds (default: 30000)
  --max-k-step <N>        ESBMC incremental BMC max iterations (with --verify)
  --verify-jobs <N>       Max concurrent ESBMC verification jobs (with --verify)

CONTRACTS OPTIONS
  --verify                Run ESBMC verification and report per-contract status
  --no-cache              Disable verification result caching
  --max-k-step <N>        ESBMC incremental BMC max iterations (default: 50)
  --solver <boolector|z3|bitwuzla|auto>  ESBMC SMT solver (with --verify)
  --encoding <bv|ir|auto>  ESBMC encoding mode (with --verify); ir requires z3 (default: auto)
  --verify-jobs <N>       Accepted for CLI parity with build/verify/test; currently a no-op (the contracts verifier is serial)

DECL OPTIONS
  -o, --output <path>     Output declaration file path (default: <source>.vow.d)

GLOBAL OPTIONS
  --help                Emit versioned JSON tool-help data
  --help --human        Emit legacy text help

OUTPUT (JSON on stdout)
  status      : Verified | Unverified | Skipped | CompileFailed | VerifyFailed
  executable  : path to compiled binary, or null
  diagnostics : array of {error_code, message, severity, span: {file, offset, length}}
  message     : error detail (CompileFailed)
  function    : function name (VerifyFailed)
  counterexample: ESBMC counterexample (VerifyFailed)

EXIT CODES
  0  success (Verified or Unverified)
  1  failure (CompileFailed, VerifyFailed, or Skipped)

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

TYPES     : i32  i64  u8  u64  f32  f64  bool  ()  !  Vec<T>  Option<T>  Result<T, E>  String  HashMap<K, V>  BTreeMap<K, V>
EFFECTS   : io  read  write  panic  unsafe
BUILTINS  : pin_to_root: fn(value: String) -> String and fn<T>(value: Vec<T>) -> Vec<T> for flat scalar T []   print_str: fn(s: String) -> () [io]   print_i64: fn(v: i64) -> () [io]
            print_u64: fn(v: u64) -> () [io]   eprintln_str: fn(s: String) -> () [io]   debug_str: fn(s: String) -> () []   debug_i64: fn(v: i64) -> () []   debug_u64: fn(v: u64) -> () []   fs_read: fn(path: String) -> String [read]   fs_open: fn(path: String) -> i64 [read]   fs_read_line: fn(handle: i64) -> String [read]   fs_status: fn(handle: i64) -> i64 [read]   fs_close: fn(handle: i64) -> i64 [read]   fs_write: fn(path: String, data: String) -> i64 [write]   fs_exists: fn(path: String) -> i64 [read]   fs_mkdir: fn(path: String) -> i64 [io]   fs_listdir: fn(path: String) -> Vec<String> [read]   fs_remove: fn(path: String) -> i64 [io]   fs_remove_dir: fn(path: String) -> i64 [io]   fs_is_dir: fn(path: String) -> i64 [read]   fs_is_symlink: fn(path: String) -> i64 [read]   fs_rename: fn(old: String, new: String) -> i64 [io]   string_substr: fn(s: String, start: i64, len: i64) -> String []   string_split: fn(s: String, delim: String) -> Vec<String> []   string_starts_with: fn(s: String, prefix: String) -> i64 []   string_ends_with: fn(s: String, suffix: String) -> i64 []   string_matches_literal_at: fn(s: String, pos: i64, literal: String literal) -> i64 []   string_trim: fn(s: String) -> String []   string_to_upper: fn(s: String) -> String []   string_to_lower: fn(s: String) -> String []   string_replace: fn(s: String, from: String, to: String) -> String []   string_join: fn(parts: Vec<String>, sep: String) -> String []   parse_i64: fn(s: String) -> i64 []   i64_to_string: fn(v: i64) -> String []   vec_sort: fn(v: Vec<i64>) -> Vec<i64> []   time_unix: fn() -> i64 [io]   time_unix_ms: fn() -> i64 [io]   num_cpus: fn() -> i64 [io]   memory_root_arena_bytes: fn() -> u64 [io]   memory_peak_bytes: fn() -> u64 [io]   memory_alloc_count_since_start: fn() -> u64 [io]   hex_encode: fn(data: Vec<u8>) -> String []   hex_decode: fn(s: String) -> Vec<u8> []   args: fn() -> Vec<String> [read]   stdin_read: fn() -> String [read]   stdin_read_line: fn() -> String [read]   stdin_ready: fn() -> bool [read]   process_exit: fn(code: i64) -> ! [io]   process_run: fn(cmd: String, args: Vec<String>) -> i64 [io]   process_get_stdout: fn() -> String [io]   process_get_stderr: fn() -> String [io]   process_start: fn(cmd: String, args: Vec<String>) -> i64 [io]   process_wait: fn(pid: i64) -> i64 [io]   process_wait_timeout: fn(pid: i64, timeout_ms: i64) -> i64 [io]   process_kill: fn(pid: i64) -> i64 [io]   process_stdout_for: fn(pid: i64) -> String [io]   process_stderr_for: fn(pid: i64) -> String [io]
METHODS   : Vec: Vec::new/Vec::from_raw_parts_copy/push/pop/len/clear/truncate/v[i]/v[i] = val   String: String::from/String::new/String::from_raw_parts_copy/len/byte_at/push_byte/push_str/clear/contains/eq/substring/parse_i64/parse_u64
            HashMap: HashMap::new/insert/get/contains_key/remove/len   BTreeMap: BTreeMap::new/insert/get/contains/len   Option: unwrap
OPERATORS : + - * / %   +! -! *! /! %! (checked)   == != < <= > >=   && || !   & | ^ << >> (bitwise, integer-only)   unary - ! & ?

VERIFICATION DEFAULTS (--max-k-step)
  Strategy        : k-induction-parallel (incremental BMC + k-induction)
  Incremental BMC : 50 max iterations (--max-k-step)"##
        .to_string()
}
// GENERATE:SKILL_HUMAN:END

// GENERATE:SKILL_FULL:START
fn skill_entrypoint_markdown() -> String {
    r#"---
name: vow
description: >-
  Write, compile, debug, and verify Vow programs (.vow files) with contracts,
  CEGIS, ESBMC counterexamples, diagnostics, and vow build / vow verify.
when_to_use: >-
  Use when the user edits or creates .vow files, says "write a Vow program",
  "fix this counterexample", "add contracts", "why did verification fail",
  "ESBMC", "vow build", or "vow verify".
argument-hint: "[file.vow]"
---

# Vow

Use this skill when writing, compiling, debugging, or verifying Vow programs.
Keep the workflow tight: run the compiler, read the structured JSON, fix the
program or contract, and repeat until the result is `Verified`.

## Installed toolchain (live)

!`(command -v vow >/dev/null 2>&1 && vow --help 2>/dev/null | head -200) || (command -v build/vowc >/dev/null 2>&1 && build/vowc --help 2>/dev/null | head -200)`

## Core workflow

1. Write a `.vow` file with explicit contracts.
2. Run `ulimit -v 2000000; build/vowc build <file.vow>`.
3. Parse stdout JSON and inspect `status`, `diagnostics`, and `counterexamples`.
4. Fix compile errors, verification failures, or weak contracts, then rerun.

## Minimal program

```vow
module Hello

fn main() -> i32 [io] {
    print_str("Hello, world!");
    0
}
```

## Reference files

- Grammar, types, effects, builtins: [reference/grammar.md](reference/grammar.md)
- CLI commands, flags, JSON output: [reference/cli.md](reference/cli.md)
- Contracts and CEGIS guidance: [reference/contracts.md](reference/contracts.md)
- Which contracts to write (taxonomy & strength): [reference/contracts-methodology.md](reference/contracts-methodology.md)
- Diagnostics and fixes: [reference/errors.md](reference/errors.md)
- Worked examples: [examples/examples.md](examples/examples.md)
- JSON schemas: [schemas/](schemas/)
"#
    .to_string()
}

fn skill_bundle_markdown() -> String {
    r##"---
name: vow
description: >-
  Write, compile, debug, and verify Vow programs (.vow files) with contracts,
  CEGIS, ESBMC counterexamples, diagnostics, and vow build / vow verify.
when_to_use: >-
  Use when the user edits or creates .vow files, says "write a Vow program",
  "fix this counterexample", "add contracts", "why did verification fail",
  "ESBMC", "vow build", or "vow verify".
argument-hint: "[file.vow]"
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

## Development Discipline

Vow is written by agents under a finite context window. These principles apply to every task, not just language design.

**Deep modules, not shallow ones.** A deep module packs a lot of functionality behind a simple interface and hides complexity. A shallow module exposes a complex interface for not much functionality and surfaces complexity to every caller. Prefer deep. When you design or extend a module, ask: does the interface hide more complexity than it exposes? If not, collapse the module or move its logic behind a narrower boundary. Many exported symbols, thin wrappers, and "pass-through" functions are signs of a shallow module.

**Surgical changes.** Prefer many small changes over one large change. Small changes are easier to review, debug, bisect with `git bisect`, and revert. A bug fix fixes the bug — it does not also refactor the surrounding code, rename variables, or reformat the file. Unrelated improvements belong in their own commits or PRs. If a task grows, split it; do not bundle.

**Keep files small, functions smaller.** Context is precious. Every file an agent reads consumes budget that could go toward solving the problem. A 2000-line file forces whole-file reads and pushes other context out; a 200-line file does not. A 100-line function is harder to reason about than four 25-line functions with clear names. Split by responsibility as soon as a unit stops fitting a single coherent idea. This applies to both Vow source code and any tooling around it.

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
| `u8`   | 8-bit unsigned integer   |
| `u64`  | 64-bit unsigned integer  |
| `f32`  | 32-bit float (limited support — avoid in contracts) |
| `f64`  | 64-bit float (limited support — avoid in contracts) |
| `bool` | Boolean                  |
| `()`   | Unit type                |
| `!`    | Never type (diverges)    |

### Built-in Parameterized Types

| Type               | Description                     |
|--------------------|---------------------------------|
| `Vec<T>`           | Growable array                  |
| `Option<T>`        | Optional value (Some/None)      |
| `Result<T, E>`     | Success or error                |
| `String`           | UTF-8 string (backed by Vec<u8>)|
| `HashMap<K, V>`    | Key-value map (linear scan)     |
| `BTreeMap<K, V>`   | Sorted key-value map (binary search; ascending iteration). `K` must be `i64`; `V` may be any non-linear type |

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

String literals have type `String` and are backed by a read-only static
descriptor. Passing or returning a literal does not allocate. To obtain a
mutable, arena-owned copy, use `String::from("...")`.

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

### Bitwise Operators

| Operator | Meaning      |
|----------|--------------|
| `&`      | Bitwise AND  |
| `\|`     | Bitwise OR   |
| `^`      | Bitwise XOR  |
| `<<`     | Left shift   |
| `>>`     | Right shift  |

Bitwise operators require integer operands of the same type. Shift expressions return the left operand's type. `>>` is arithmetic for `i64` and logical for `u64`.

Unsuffixed integer literals are `i64` by default but coerce to the other operand's integer type when used with a bitwise or shift operator. The same coercion applies to constant expressions composed entirely of unsuffixed integer literals — including arithmetic (`1 + 1`), bitwise (`1 << 3`), and unary negation (`-5`). For example, given `let x: u64 = ...`, the expressions `x << 3`, `3 & x`, and `x << (1 + 1)` all type-check (the literal-constant side coerces to `u64`). This matches the coercion rule already used by arithmetic operators and comparisons. Use a `u64` suffix (`3u64`) to force the `u64` type explicitly.

### Logical Operators

| Operator | Meaning    |
|----------|------------|
| `&&`     | Logical AND (short-circuit) |
| `\|\|`   | Logical OR (short-circuit) |
| `!`      | Logical NOT|

`&&` and `||` use short-circuit evaluation: for `a && b`, `b` is only evaluated if `a` is true; for `a || b`, `b` is only evaluated if `a` is false.

### Operator Precedence

From loosest to tightest, Vow follows the usual C/Rust precedence for logical and bitwise operators:

`||`, `&&`, comparisons (`== != < <= > >=`), `|`, `^`, `&`, `<< >>`, `+ -`, `* / %`

Unary `-`, `!`, `&`, and `?` bind tighter than every binary operator.

Single `&` is overloaded by position: prefix `&expr` is borrow, while infix `lhs & rhs` is bitwise AND.

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

Linear struct values carry a linear obligation. The obligation must either be consumed before the value's owning region closes or transferred to the caller by returning the value.

### Struct Literals

Struct literal names must be PascalCase:

```vow
let p: Point = Point { x: 1, y: 2 };
```

### Field Access

```vow
p.x
```

### Field Assignment

```vow
p.x = 10;
```

### Passing Semantics

Structs are heap-allocated. A struct value is a pointer to a heap region, so passing a struct to a function passes the pointer — the function operates on the same heap data, not a copy. Field assignments inside the called function are visible to the caller:

```vow
fn shift_right(p: Point, dx: i64) {
    p.x = p.x + dx;
}

fn main() -> i32 [io] {
    let p: Point = Point { x: 0, y: 0 };
    shift_right(p, 5);
    print_i64(p.x);  // 5 — mutation visible to caller
    0
}
```

This enables in-place mutation patterns (e.g., make/unmake in search trees) without cloning. The same aliasing semantics apply when structs are stored in containers — see [Indexing](#indexing). To avoid aliasing, construct a fresh struct literal with the desired field values.

**Note:** For `linear struct` types, passing the value to a function consumes it; the caller cannot access it afterward. Returning a linear value transfers the obligation to the caller, so this is the normal way to hand an updated linear value back out of a function.

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
| `Vec::from_raw_parts_copy(ptr, len)` | `(i64, i64) -> Vec<T>` for flat scalar `T` |
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
| `String::from(s)`   | `(String) -> String` — mutable copy |
| `String::new()`     | `() -> String`              |
| `String::from_raw_parts_copy(ptr, len)` | `(i64, i64) -> String` |
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

### BTreeMap<K, V> Methods

Keys must be `i64` (K violations raise `BTreeMapKeyTypeMustBeI64`). Values may be any
non-linear type — primitives, structs, `Vec<T>`, `Option<T>`, or nested combinations.
A `V` that is or transitively contains a `linear struct` is rejected with
`BTreeMapValueMustBeNonLinear`, because the runtime/verifier shift values bitwise and
would silently duplicate a linear obligation.
Storage is two parallel sorted arrays (binary-search lookup, sorted-insert writes).
Iteration order is ascending by key and is **deterministic across runs and compilers** —
prefer `BTreeMap` over `HashMap` for any map whose iteration affects compiler output.

| Method              | Signature                   |
|---------------------|-----------------------------|
| `BTreeMap::new()`   | `() -> BTreeMap<K, V>`      |
| `.insert(k, v)`     | `(K, V) -> Option<V>` (returns the previous value bound to `k`, if any) |
| `.get(k)`           | `(K) -> Option<V>` (returns the value bound to `k`, or `None`)          |
| `.contains(k)`      | `(K) -> bool`               |
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

#### FFI Wrapper Intrinsics

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `pin_to_root`    | `fn(value: String) -> String` and `fn<T>(value: Vec<T>) -> Vec<T>` for flat scalar `T` | `[]` |

`pin_to_root` is a compiler intrinsic, not a user-defined generic. Each call site is monomorphised from the argument type. It always deep-copies the supported heap value into root storage; it does not inspect descriptor tags and does not claim idempotency. The current supported forms are `String` and `Vec<T>` where `T` is a flat scalar slot type (`i*`, `u*`, `f32`, `f64`, `bool`). Pointer-containing payloads, user structs, enums, and maps require hand-written deep-copy wrappers at the FFI boundary.

`String::from_raw_parts_copy(ptr: i64, len: i64)` copies `len` bytes from a raw C pointer into a fresh `String`. `Vec::from_raw_parts_copy(ptr: i64, len: i64)` copies `len` flat scalar slots into a fresh `Vec<T>`. The surface length type is `i64`; the code generator converts pointer and length values to the platform pointer-sized ABI type at the FFI boundary. Both helpers have a `FreshInCaller` return summary.

For pointer-containing C payloads, a wrapper must be written per type: call the extern, recursively copy every Vow-owned heap subobject into the target region, free every C-owned pointer according to the extern's ownership contract, then return the Vow-placed value. A bytewise copy of a pointer-containing payload is unsound because it preserves stale pointers into C-owned storage.

#### Print / IO

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `print_str`      | `fn(s: String) -> ()`                      | `[io]`     |
| `print_i64`      | `fn(v: i64) -> ()`                         | `[io]`     |
| `print_u64`      | `fn(v: u64) -> ()`                         | `[io]`     |
| `eprintln_str`   | `fn(s: String) -> ()`                      | `[io]`     |

#### Debug

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `debug_str`      | `fn(s: String) -> ()`                      | `[]`       |
| `debug_i64`      | `fn(v: i64) -> ()`                         | `[]`       |
| `debug_u64`      | `fn(v: u64) -> ()`                         | `[]`       |

**Debug print semantics:** Debug prints are effect-free and callable from pure functions. In debug and sanitize modes (`--mode debug`, `--mode sanitize`), they write to stderr. In release and profile modes, the debug call itself is not emitted — no function call occurs. However, argument expressions are still evaluated (a direct literal such as `"label"` is static, while `String::from("label")` still allocates a mutable copy). They are also no-ops during verification. Use them to trace values inside pure kernel code without restructuring the effect hierarchy.

#### Filesystem

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `fs_read`        | `fn(path: String) -> String`               | `[read]`   |
| `fs_open`        | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_read_line`   | `fn(handle: i64) -> String`                | `[read]`   |
| `fs_status`      | `fn(handle: i64) -> i64`                   | `[read]`   |
| `fs_close`       | `fn(handle: i64) -> i64`                   | `[read]`   |
| `fs_write`       | `fn(path: String, data: String) -> i64`    | `[write]`  |
| `fs_exists`      | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_mkdir`       | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_listdir`     | `fn(path: String) -> Vec<String>`          | `[read]`   |
| `fs_remove`      | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_remove_dir`  | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_is_dir`      | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_is_symlink`  | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_rename`      | `fn(old: String, new: String) -> i64`      | `[io]`     |

#### String Operations

| Function              | Signature                                        | Effects |
|-----------------------|--------------------------------------------------|---------|
| `string_substr`       | `fn(s: String, start: i64, len: i64) -> String`  | `[]`    |
| `string_split`        | `fn(s: String, delim: String) -> Vec<String>`    | `[]`    |
| `string_starts_with`  | `fn(s: String, prefix: String) -> i64`           | `[]`    |
| `string_ends_with`    | `fn(s: String, suffix: String) -> i64`           | `[]`    |
| `string_matches_literal_at` | `fn(s: String, pos: i64, literal: String literal) -> i64` | `[]` |
| `string_trim`         | `fn(s: String) -> String`                        | `[]`    |
| `string_to_upper`     | `fn(s: String) -> String`                        | `[]`    |
| `string_to_lower`     | `fn(s: String) -> String`                        | `[]`    |
| `string_replace`      | `fn(s: String, from: String, to: String) -> String` | `[]` |
| `string_join`         | `fn(parts: Vec<String>, sep: String) -> String`  | `[]`    |

#### Conversion

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `parse_i64`      | `fn(s: String) -> i64`                     | `[]`       |
| `i64_to_string`  | `fn(v: i64) -> String`                     | `[]`       |

#### Collections

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `vec_sort`       | `fn(v: Vec<i64>) -> Vec<i64>`              | `[]`       |

#### Time

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `time_unix`      | `fn() -> i64`                              | `[io]`     |
| `time_unix_ms`   | `fn() -> i64`                              | `[io]`     |

#### System

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `num_cpus`       | `fn() -> i64`                              | `[io]`     |
| `memory_root_arena_bytes` | `fn() -> u64`                    | `[io]`     |
| `memory_peak_bytes` | `fn() -> u64`                           | `[io]`     |
| `memory_alloc_count_since_start` | `fn() -> u64`              | `[io]`     |

`num_cpus()` returns the number of available logical CPUs (from `std::thread::available_parallelism`), or `1` if the query fails. Used to size worker pools (e.g. the default `--verify-jobs` value).

`memory_root_arena_bytes()` returns the current bytes retained by root-region arena chunks. `memory_peak_bytes()` returns the peak live bytes retained by all open arena chunks since process start. `memory_alloc_count_since_start()` returns the number of successful Vow arena allocation requests since process start. These queries do not allocate; they are effectful because they observe runtime process state.

#### Encoding

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `hex_encode`     | `fn(data: Vec<u8>) -> String`              | `[]`       |
| `hex_decode`     | `fn(s: String) -> Vec<u8>`                 | `[]`       |

#### Input

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `args`           | `fn() -> Vec<String>`                      | `[read]`   |
| `stdin_read`     | `fn() -> String`                           | `[read]`   |
| `stdin_read_line`| `fn() -> String`                           | `[read]`   |
| `stdin_ready`    | `fn() -> bool`                             | `[read]`   |

#### Process Management

| Function              | Signature                                        | Effects |
|-----------------------|--------------------------------------------------|---------|
| `process_exit`        | `fn(code: i64) -> !`                             | `[io]`  |
| `process_run`         | `fn(cmd: String, args: Vec<String>) -> i64`      | `[io]`  |
| `process_get_stdout`  | `fn() -> String`                                 | `[io]`  |
| `process_get_stderr`  | `fn() -> String`                                 | `[io]`  |
| `process_start`       | `fn(cmd: String, args: Vec<String>) -> i64`      | `[io]`  |
| `process_wait`        | `fn(pid: i64) -> i64`                            | `[io]`  |
| `process_wait_timeout`| `fn(pid: i64, timeout_ms: i64) -> i64`           | `[io]`  |
| `process_kill`        | `fn(pid: i64) -> i64`                             | `[io]`  |
| `process_stdout_for`  | `fn(pid: i64) -> String`                         | `[io]`  |
| `process_stderr_for`  | `fn(pid: i64) -> String`                         | `[io]`  |

**`args` semantics:** `args()` returns all process arguments including the program name at index 0 (matching C `argv` and Rust `std::env::args()` conventions). For `./my_program foo bar`, `args()` returns `["./my_program", "foo", "bar"]`. Use `args[1]` onward for user-supplied arguments. The Vec is empty only if the OS provides no arguments (unusual). Returns an empty String element if an argument is empty (`""`). Non-UTF-8 arguments are included as-is (byte content preserved).

**`fs_read` semantics:** `fs_read(path)` opens the file at `path`, reads its entire contents, and returns a String. Returns `""` (empty String) on any error (file not found, permission denied, I/O error, non-UTF-8 path). Does not block on regular files. Callers should check `result.len() == 0` to detect failure.

**Streaming file input:** `fs_open(path)` opens a file for incremental reading and returns a positive handle, or `-1` on path/open error. `fs_read_line(handle)` reads one line from the current cursor and returns it as a String, including the trailing newline when present. It returns `""` at EOF, for an invalid handle, or after a read error. A blank line is returned as `"\n"`, so newline-delimited callers can distinguish a real blank line from EOF by content. After `fs_read_line(handle)` returns `""`, call `fs_status(handle)` to distinguish EOF from error: `0` means the handle is open with no EOF/error state, `1` means EOF, and `-1` means invalid handle or read error. `fs_status(handle)` reports the result of the most recent `fs_read_line(handle)` call on that open handle; read it immediately after a `""` return because later reads may update it. `fs_close(handle)` releases the handle and returns `0` on success or `-1` for an invalid/already-closed handle. Long-running programs must close handles they no longer need. All streaming handle operations use the `[read]` effect, including `fs_close`, because closing a read handle releases read-stream state and does not mutate filesystem contents. The current runtime stores streaming handles in one process-global table, and `fs_read_line` holds that table lock while it reads the next line. This keeps the API simple for single-stream file processing, but it is not intended for latency-sensitive concurrent reads from multiple slow handles.

**Filesystem return values:** `fs_write`, `fs_mkdir`, `fs_remove`, `fs_remove_dir`, and `fs_rename` return `i64`: 0 on success, non-zero on failure. `fs_open`, `fs_status`, and `fs_close` use the streaming status codes above. `fs_exists`, `fs_is_dir`, and `fs_is_symlink` are predicates: they return 1 for true, 0 for false. Errors (null pointer, invalid UTF-8) also return 0, so callers cannot distinguish "false" from "error". `fs_is_symlink` uses `lstat`-equivalent semantics: a symlink reports 1 even when its target is a regular file or directory.

**`string_starts_with` / `string_ends_with` / `string_matches_literal_at` return values:** Return `i64`: 1 if true, 0 if false.

**`string_matches_literal_at` literal operand:** The third argument must be written as a string literal at the call site. The compiler lowers that literal to static bytes plus an explicit byte length, so no temporary `String` allocation is created and embedded NUL bytes are preserved. Passing a variable or computed `String` as the third argument is a type-check error (`StaticLiteralRequired`). Use `string_starts_with`, `string_ends_with`, or `String` methods when the needle must be dynamic.

**`process_run` vs `process_start`:** `process_run(cmd, args)` runs a subprocess synchronously and returns its exit code. After it returns, `process_get_stdout()` and `process_get_stderr()` retrieve the captured output of the most recent `process_run` call. `process_start(cmd, args)` launches a subprocess asynchronously and returns a process ID. Use `process_wait(pid)` to wait for completion and get the exit code, and `process_stdout_for(pid)` / `process_stderr_for(pid)` to retrieve output.

**`process_wait_timeout`:** `process_wait_timeout(pid, timeout_ms)` polls a process started with `process_start` until it exits or the timeout (in milliseconds) elapses. Returns the exit code on completion, `-1` on error, or `-2` on timeout. After a timeout, the process is still running; use `process_kill(pid)` to terminate it.

**`process_kill`:** `process_kill(pid)` sends a kill signal to a running process and waits for it to exit. Returns 0 on success, -1 on error. No-op (returns 0) if the process has already completed.

**`stdin_read` vs `stdin_read_line`:** `stdin_read()` reads the entire stdin stream into a single String (unbounded memory). `stdin_read_line()` reads one line at a time, including the trailing newline. Returns `""` (empty string) at EOF. The returned String is runtime scratch storage valid until the next `stdin_read_line()` call. Process each line before reading the next one for bounded memory; use `pin_to_root(line)` before the next read when a line must be stored, returned, passed to a function that may store it, mutated, or otherwise retained. The direct scratch line is read-only. The scratch buffer keeps the largest line capacity seen so far, so memory is bounded by maximum line length rather than total input, but one very large line can retain that capacity for the process lifetime.

```vow
let lines: Vec<String> = Vec::new();
let mut line: String = stdin_read_line();
while str_len(line) > 0 {
    // Without pin_to_root, lines.push(line) would store the scratch alias, not a copy.
    lines.push(pin_to_root(line));
    line = stdin_read_line();
}
```

```vow
let mut line: String = stdin_read_line();
while str_len(line) > 0 {
    // process line (has trailing \n)
    line = stdin_read_line();
}
```

**`stdin_ready`:** `stdin_ready()` returns `true` if `stdin_read_line()` would return immediately without blocking, `false` otherwise. Uses a non-blocking poll with zero timeout. Use this in computation loops that must remain responsive to external input:

```vow
while !stdin_ready() && depth < max_depth {
    // continue searching
    depth = depth + 1;
}
if stdin_ready() {
    let cmd: String = stdin_read_line();
    // handle command
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
| `--mode <debug\|release\|profile\|sanitize>` | `release` | Build mode: debug inserts runtime vow checks, profile inserts call counters and prints report on normal exit, sanitize adds debug checks + Vec provenance tracking |
| `--no-verify`     | (off)       | Skip ESBMC static verification            |
| `--dump-ir`       | (off)       | Print IR text to stdout and exit (no JSON output, no codegen) |
| `--debug-trace <off\|calls\|full>` | `off` | Emit JSON trace lines to stderr at runtime |
| `--no-cache`    | (off)       | Disable verification result caching, and (for `--no-verify` builds) the compile-object cache. See "Compile-object cache behavior" below |
| `--max-k-step <N>` | `50`     | ESBMC incremental BMC max iterations          |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver; auto selects per-function via heuristic |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 |
| `--timeout <N>` | `300` (or `30` when `--encoding` is `auto`) | ESBMC per-function timeout in seconds. Under `--encoding auto`, a 30s default is applied so the BV-timeout fallback to `--encoding ir --solver z3` can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit `--timeout` overrides both. `--timeout 0` is honoured as an immediate watchdog kill |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent ESBMC verification jobs |

**Compile-object cache behavior.** The on-disk compile-object cache (`$VOW_CACHE_DIR` or `~/.cache/vow/`, where each entry is a `<key>.o` artifact keyed by a content hash of all dependencies, mode, and trace settings) is automatically disabled whenever ESBMC verification is active. This guarantees the linked binary always comes from the same codegen run whose IR was verified, closing the integrity gap where a stale or attacker-supplied `.o` could be linked against freshly-verified IR. Concretely the cache only activates on `vow build --no-verify` invocations; it is bypassed on the default `vow build` path. `--no-cache` additionally disables the cache for `--no-verify` builds.

### `vow verify`

Verify contracts only — no executable output. Emits the same JSON format as `vow build` but `executable` is always `null`.

```
vow verify [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `--no-cache`      | (off)       | Disable verification result caching        |
| `--max-k-step <N>` | `50`       | ESBMC incremental BMC max iterations       |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver; auto selects per-function via heuristic |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 |
| `--timeout <N>` | `300` (or `30` when `--encoding` is `auto`) | ESBMC per-function timeout in seconds. Under `--encoding auto`, a 30s default is applied so the BV-timeout fallback to `--encoding ir --solver z3` can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit `--timeout` overrides both. `--timeout 0` is honoured as an immediate watchdog kill |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent ESBMC verification jobs |

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
| `--max-k-step <N>` | `50`       | ESBMC incremental BMC max iterations       |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver (with --verify)           |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode (with --verify); ir requires z3 |
| `--verify-jobs <N>` | `num_cpus/2` | Accepted for CLI parity with build/verify/test; currently a no-op (the contracts verifier is serial) |

When `--verify` is requested but ESBMC is not installed, the command still emits the full contracts-result JSON schema with every entry's `status` set to `error` and exits with code 1 (fail-closed). Install ESBMC, or omit `--verify`, to obtain proven/failed/unknown statuses.

### `vow skill`

Generate or install the Claude Code skill document for the current compiler version. The skill is embedded in the compiler binary, ensuring the documentation always matches the installed toolchain.

```
vow skill              # print skill document to stdout (default: print)
vow skill print        # print concise Claude Code SKILL.md entrypoint
vow skill print --bundle  # print self-contained bundle for raw API harnesses
vow skill install      # prompt for local or global install target
vow skill install --local   # install to ./.claude/skills/vow/
vow skill install --global  # install to $HOME/.claude/skills/vow/ on Linux
```

`print` writes the concise installed `SKILL.md` entrypoint (with YAML frontmatter) to stdout. `print --bundle` writes a complete self-contained skill document to stdout for non–Claude Code harnesses that cannot load supporting files.

`install` writes `SKILL.md` plus supporting files under `reference/`, `examples/`, and `schemas/`. Claude Code discovers the skill from the `.claude/skills/` directory and uses the frontmatter description/`when_to_use` metadata to load it for `.vow` file work as well as creation and verification-debugging prompts before a `.vow` file exists.

When no scope flag is provided, `install` prompts on stderr for local (`./.claude`) or global (`$HOME/.claude`) installation. Scripts and agents should pass `--local` or `--global` explicitly. `--local` requires the current directory to contain both `.git` and `.claude/`; otherwise it exits with an error and writes nothing. `--global` installs under `$HOME/.claude/skills/vow/` and fails if `$HOME` is unset or empty.

**Auto-install on build.** The first time `vow build` (or the legacy `vow <source.vow>` form) runs in a directory that already contains a `.claude/` subtree but no `.claude/skills/vow/SKILL.md`, the compiler installs the skill silently. This bootstraps Claude Code projects without requiring an explicit `vow skill install`. Unlike explicit `--local`, auto-install only requires `.claude/`; it does not require the directory to be a git checkout. Auto-install is skipped when `.claude/` does not exist (so it never pollutes non–Claude Code projects) and when the skill file is already present (so user edits are never overwritten). Auto-install never fails the build.

### `vow test`

Discover, compile, run, and report on Vow test files. Tests are normal `.vow` programs with `main() -> i32` — no test-specific syntax.

```
vow test [OPTIONS] [<path>]
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `<path>`          | `.`         | Directory to scan or single `.vow` file    |
| `--verify`        | (off)       | Run ESBMC verification on test files       |
| `--filter <pat>`  | (none)      | Only run tests whose file stem contains pat |
| `--module-root <path>` | (auto)  | Resolve `use` declarations against `<path>`. Defaults to the scan path when it's a directory, otherwise the entry file's parent directory. |
| `--mode debug`    | (default)   | Insert runtime vow checks                 |
| `--mode release`  | `debug`     | Omit all vow checks for performance       |
| `--timeout <ms>`  | `30000`     | Per-test execution timeout in milliseconds |
| `--max-k-step <N>` | `50`       | ESBMC incremental BMC max iterations (with --verify) |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent ESBMC verification jobs (with --verify) |

Test discovery: files matching `test_*.vow` or `*_test.vow` under the given directory **and its subdirectories**, sorted alphabetically. Each test must contain `main() -> i32` returning 0 on success.

**Module resolution for directory scans.** When `<path>` is a directory, every discovered test resolves its `use` declarations against `<path>` rather than the test file's own parent directory. This lets internal-unit tests live in a subdirectory like `compiler/tests/test_region.vow` and still `use region;` to import the module under test (which lives at `compiler/region.vow`). Single-file invocations (`vow test path/to/test_foo.vow`) keep the default behaviour of resolving `use` against the file's parent directory.

**Test Output JSON:**

```json
{
  "status": "TestsPassed",
  "total": 3,
  "passed": 3,
  "failed": 0,
  "skipped": 0,
  "tests": [
    {
      "file": "compiler/test_arith.vow",
      "name": "test_arith",
      "status": "passed",
      "exit_code": 0,
      "stdout": "7",
      "stderr": "",
      "duration_ms": 72,
      "diagnostics": [],
      "counterexamples": []
    }
  ],
  "contract_density": {
    "functions_total": 1,
    "functions_with_vows": 0,
    "density_pct": 0.0
  }
}
```

| Status Field   | Meaning                                           |
|----------------|---------------------------------------------------|
| `TestsPassed`  | All tests passed                                  |
| `TestsFailed`  | One or more tests failed                          |

Per-test status: `passed`, `failed`, `timeout`, `compile_error`, `verify_failed`, `skipped`.

### `vow decl`

Emit declaration file output only.

```
vow decl [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `-o, --output`    | `<source>.vow.d` | Output declaration file path          |

### `vow mutants` (self-hosted only)

Run mutation testing on a Vow source tree. Implemented in the self-hosted compiler only; the Rust bootstrap compiler emits an error pointing the user to `build/vowc`. See `docs/mutants.md` for full details on output schema, mutation kinds, skip-list, and known limitations.

```
vowc mutants version
vowc mutants list  [--root DIR] [--shard X/Y]
vowc mutants run   [--root DIR] [--shard X/Y]
                   [--tier1-cmd 'cmd'] [--tier2-cmd 'cmd']
                   [--tier1-timeout-secs N] [--tier2-timeout-secs N]
                   [--tier2-budget-secs N]
                   [--workdir DIR] [--output-dir DIR] [--force-unlock]
```

| Flag | Default | Notes |
|---|---|---|
| `--root` | `compiler` | Directory whose `*.vow` files are mutated. `test_*.vow` files are excluded. |
| `--shard X/Y` | `0/1` | Round-robin split of the deterministic mutant ID space. Mutant `id` is selected iff `id % Y == X`. |
| `--tier1-cmd` | `scripts/bootstrap.sh --skip-cargo` | Fast oracle. Anything but exit 0 = caught at Tier 1. |
| `--tier2-cmd` | `scripts/full_test.sh` | Full oracle. Only run on Tier-1 survivors. |
| `--tier1-timeout-secs` | `180` | Per-mutant Tier-1 wall-clock cap. |
| `--tier2-timeout-secs` | `3600` | Per-mutant Tier-2 wall-clock cap. |
| `--tier2-budget-secs` | `7200` | Per-shard total Tier-2 budget. Once exhausted, surviving Tier-1 mutants are emitted with `status:"unrun"`. |
| `--workdir` | `/tmp/vow-mutants-<ms>` | Path of the throwaway `git worktree` used for all mutations. |
| `--output-dir` | `mutants.out` | Directory for `mutants.json`, `outcomes.json`, status text files, `diff/`, `logs/`. |
| `--force-unlock` | off | Remove a stale `output_dir/.lock` before starting. |

Output schemas: see `docs/spec/schemas/mutants-result.schema.json`.

### `vow --help`

`vow --help` is agent-first. It emits versioned JSON capability data for the tool, command set,
language surface, result schemas, and implementation status. `--help --human` exists only as a
legacy compatibility mode and is not the canonical interface.

```
vow --help               # versioned JSON tool-help protocol
vow --help --human       # legacy compatibility text
vow build --help         # same JSON (works on all subcommands)
vow verify --help --human  # same legacy text (works on all subcommands)
```

## Exit Codes

| Code | Meaning                                                                            |
|------|------------------------------------------------------------------------------------|
| `0`  | Success (`Verified` or `Unverified`)                                               |
| `1`  | Failure (`CompileFailed`, `VerifyFailed`, or `Skipped`)                            |

`vow build` and `vow verify` both fail closed on `Skipped`: if ESBMC was asked to verify a vowed
function but the verifier could not model the function body, the contract was not statically
proved, so the run exits non-zero. Use `--no-verify` if you genuinely want to skip verification —
that path produces `Unverified` (exit 0).

## Build Output JSON

`vow build` and `vow verify` emit a single JSON object to stdout. Schema: [`schemas/build-result.schema.json`](schemas/build-result.schema.json).

**Note:** `--dump-ir` suppresses JSON output — only IR text is printed.

### Status Values

| Status          | Meaning                                     |
|-----------------|---------------------------------------------|
| `Verified`      | Compiled + every vowed function's contract was statically proved by ESBMC. |
| `Unverified`    | Compiled but ESBMC was not invoked (e.g. `--no-verify`, `--dump-ir`). Exit 0. |
| `Skipped`       | ESBMC was invoked but at least one vowed function could not be modelled (e.g. body uses `RegionAlloc`, `FieldSet`, `Linear*`, `Load`/`Store`, `RemF*`, or has effects). Each such function appears as a `VerificationSkipped` *Warning* in `diagnostics[]`. Their contracts are runtime-checked under `--mode debug` but were not statically proved; the run fails closed with exit 1. |
| `CompileFailed` | Parse error, type error, module load error, or link failure |
| `VerifyFailed`  | ESBMC produced a non-Verified outcome: a counterexample, timeout, `VERIFICATION UNKNOWN` (`verify_status: "unknown"`), tool error, or the tool was not found. Inspect `counterexamples[]` (definitive failures) and `verify_status`/`verify_message` (soft failures) to distinguish. |

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
| `verify_status`    | string              | On backend failure | `"timeout"`, `"unknown"`, `"error"`, or `"tool_not_found"` |
| `verify_message`   | string              | On backend failure | ESBMC/backend error detail                |

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
      "status": "not_verified",
      "quality": "substantive"
    }
  ],
  "summary": { "total": 1, "proven": 0, "failed": 0, "timeout": 0, "error": 0, "not_verified": 1, "skipped": 0, "quality": { "weak": 0, "tautological": 0, "substantive": 1 } }
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
      "status": "proven",
      "quality": "substantive"
    }
  ],
  "summary": { "total": 1, "proven": 1, "failed": 0, "timeout": 0, "error": 0, "not_verified": 0, "skipped": 0, "quality": { "weak": 0, "tautological": 0, "substantive": 1 } }
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
| `status`      | string  | `"proven"`, `"proven-ir"`, `"failed"`, `"unknown"`, `"timeout"`, `"error"`, `"not_verified"`, or `"skipped"` |
| `quality`     | string  | Static clause-shape classification (no ESBMC): `"weak"`, `"tautological"`, or `"substantive"` |

### Status Values

| Status          | Meaning                                              |
|-----------------|------------------------------------------------------|
| `not_verified`  | Verification not requested (no `--verify` flag)      |
| `proven`        | ESBMC proved this contract holds for all inputs (bit-vector encoding, overflow modeled) |
| `proven-ir`     | ESBMC proved this contract under integer-arithmetic encoding after BV timed out; overflow is not modeled by IR, but the BV caller preconditions still guard against it |
| `failed`        | ESBMC found a counterexample violating this contract |
| `unknown`       | ESBMC could not conclude for this contract — either `VERIFICATION UNKNOWN` was reported for the containing function (k-induction's forward condition unable to prove or falsify), or another contract in the same function failed and this one was not individually checked |
| `timeout`       | ESBMC timed out on the containing function (BV and — when applicable — IR fallback both timed out) |
| `error`         | ESBMC error or tool not found                        |
| `skipped`       | The containing function's body uses opcodes the verifier cannot model (e.g. `RegionAlloc` from struct construction). Contract is documentary; runtime checks still apply under `--mode debug`. Surfaces as a `VerificationSkipped` Warning in the build JSON's `diagnostics[]` and lifts the overall build/verify status to `Skipped` (fail-closed, exit 1) — use `--no-verify` if you want a non-failing path that does not invoke ESBMC at all. |

### Quality Values

`quality` is a static classification of each clause's *shape*, computed without ESBMC and independent of `status`. It surfaces the "proven but trivial" problem: a `weak` contract can be `proven` while constraining almost nothing. See `docs/spec/contracts-methodology.md` for the full taxonomy.

| Quality        | Meaning                                                                                      |
|----------------|----------------------------------------------------------------------------------------------|
| `weak`         | An `ensures` that only bounds `result` by an integer literal (e.g. `result >= 0`). Satisfied by almost any implementation. |
| `tautological` | A constant clause that references no program value (e.g. `true`, `0 >= 0`). Constrains nothing. |
| `substantive`  | Everything else — equality, relational, inverse/round-trip, dispatch-totality, or function-call shapes. The classifier is conservative: anything not provably weak/tautological is reported `substantive`. |

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

## Runtime Error JSON (stderr, debug/sanitize mode)

When a compiled program runs in debug mode (`--mode debug`) or sanitize mode (`--mode sanitize`) and violates a vow at runtime, it emits JSON to stderr before aborting.

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

### UseAfterFree (sanitize mode only)

```json
{"error":"UseAfterFree","op":"push","vec":"0x55a1b2c3d4e0"}
```

Emitted when a Vec operation is attempted on a Vec that has already been freed.

### DoubleFree (sanitize mode only)

```json
{"error":"DoubleFree","vec":"0x55a1b2c3d4e0"}
```

Emitted when a Vec is freed twice.

### StaleIndex (sanitize mode only)

```json
{"error":"StaleIndex","index":5,"expected_gen":3,"actual_gen":7,"vec":"0x55a1b2c3d4e0"}
```

Emitted when `__vow_sanitize_check_generation` detects that a Vec slot's generation counter does not match the expected value, indicating the slot was overwritten since the index was recorded.

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

- Verification strategy: **k-induction-parallel** (incremental BMC + k-induction proof)
- Incremental BMC with `--max-k-step` (default: **50**) — loops are verified incrementally up to N iterations
- Architecture: 64-bit
- Array bounds / pointer checks disabled (Vow handles these in its own model)

### Collection Models for Verification

ESBMC is a *bounded* model checker, so it models collection types as
fixed-size arrays and reasons about them up to a finite capacity. These
capacities are an internal property of the verifier, not of the language:

| Type              | Model Capacity | Supported Operations |
|-------------------|----------------|----------------------------------------------|
| `Vec<T>`          | 128            | `new`, `push`, `pop`, `len`, `get`, `set`    |
| `String`          | 256            | `from`, `len`, `push_byte`, `push_str`, `byte_at`, `matches_literal_at` |
| `HashMap<K, V>`   | 64             | `new`, `insert`, `get`, `contains_key`, `len`|
| `BTreeMap<K, V>`  | 64             | `new`, `insert`, `get`, `contains_key`, `len`|

**These bounds are not a language feature and are not user-tunable.** A `Vec`
in a Vow program grows dynamically on the heap with no fixed maximum; the
capacity above only describes how far the *bounded* model checker reasons. The
language and its contracts are deliberately decoupled from what any particular
prover can prove: replace ESBMC with a stronger (or unbounded) checker and the
same source, the same contracts, and the same CLI keep working — the only
difference is that proof covers more (or all) of the state space. For this
reason a `requires`/`ensures` clause must never encode a verifier bound (e.g.
`requires: v.len() <= 128`); see "Verification-Driven Bounds (Anti-Pattern)"
below and `docs/design/verifier-model-bounds.md`.

These models support the same operations as the runtime but with bounded
storage. String literals carry their concrete length and bytes in verification,
and `String::from` copies that model from its source value. The effective string
model capacity is automatically at least the longest static string literal, so
literal byte initializers always fit the model array. Operations whose bytes are
not statically known, such as `String::from_cstr`, produce a nondeterministic
length (0 to max-1). `string_matches_literal_at` is modeled against the
literal's concrete bytes and byte length; the third argument must be a string
literal so the verifier never has to infer static text from a dynamic `String`.

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

Use range bounds only when they reflect genuine semantic constraints (e.g., overflow prevention), not to appease the verifier:

```vow
fn safe_add(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: a <= 4611686018427387903,
    requires: b <= 4611686018427387903,
    ensures: result >= a,
    ensures: result >= b
} {
    a + b
}
```

The bounds here prevent `a + b` from overflowing `i64` — a legitimate semantic concern, not a verifier limitation.

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
    requires: a <= 4611686018427387903,
    requires: b <= 4611686018427387903,
    ensures: result >= a,
    ensures: result >= b
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

Without a bound on loop iterations, ESBMC may timeout (default max-k-step is 50):

```vow
fn fill(n: i64) -> Vec<i64> vow {
    requires: n >= 0,
    ensures: result.len() == n
} { ... }
```

ESBMC will only verify this for small `n` values. **Do not** add `requires: n <= 8` to the contract — that would distort the semantic specification. The contract is correct as-is; ESBMC's bounded verification provides partial assurance.

### Verification-Driven Bounds (Anti-Pattern)

**Never** add artificial bounds to contracts solely to help ESBMC verify them:

```vow
// WRONG: bounds exist only to appease the verifier
fn gcd(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: a + b > 0,
    requires: a <= 15,   // <-- verifier artifact, not semantic
    requires: b <= 15,   // <-- verifier artifact, not semantic
    ensures: result > 0
} { ... }
```

```vow
// CORRECT: only genuine semantic constraints
fn gcd(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: a + b > 0,
    ensures: result > 0
} { ... }
```

Contracts express what is mathematically required for correctness. ESBMC verifies within its capabilities (bounded loops, bounded arithmetic, bounded collection models) — if it cannot fully prove a correct contract, that is acceptable. Partial verification is better than a distorted specification. The same rule is why the verifier's collection model capacities (see "Collection Models for Verification") are internal defaults rather than CLI flags or contract clauses: a bound that belongs to the prover must never leak into the language.

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

# Contract Methodology: What to Verify

This document answers a question that `contracts.md` does not: given a function,
**which** properties are worth proving, how do you tell a strong contract from a
hollow one, and how do you express the strong shapes within ESBMC's reach.

`contracts.md` is the *reference* (syntax, blame, the verification pipeline,
anti-patterns). This is the *methodology* (judgement). Read that first.

## The core principle: strength, not volume

A proven contract is worth nothing if it would also hold for an incorrect
implementation. The number of contracts a codebase proves is not a quality
signal — the *discriminating power* of each contract is.

This is measurable. Polikarpova, Furia, Pei, Wei, and Meyer (the originator of
Design by Contract), *"What Good Are Strong Specifications?"* (ICSE 2013), found
that testing implementations against **strong** specifications — comprehensive
functional pre/post/invariants — detected roughly **twice as many bugs** as
testing against standard/weak contracts. Their conclusion: *"the quality of
specifications limits the value of verification."*

A concrete Vow example of the trap. Many tag constants in the self-hosted
compiler carry this contract:

```vow
fn EFF_IO() -> i64 vow { ensures: result > 0 } { 1 }
```

ESBMC proves `1 > 0` in milliseconds. But `ensures: result > 0` also holds for
`{ 2 }`, `{ 99 }`, and every other positive constant — it does not pin the value
this function is *supposed* to return. It is a real postcondition, but a **weak**
one: it constrains the output to a half-line, not to a point. Proving 354 of
these does not make the compiler more correct; it makes the verification report
longer.

The fix is not "delete contracts" — it is "make each contract say something only
the correct implementation satisfies."

## A taxonomy of contract shapes

Each shape below lists its *intent*, *when it applies*, a *real Vow example*, and
*strength notes*. The expressibility/verifiability status of every shape is
collected in the matrix at the end.

### 1. Domain precondition (range / validity bound)

**Intent:** restrict the inputs the function promises to handle correctly. Blame
falls on the caller (`requires`).

**When:** the function is only correct on a subset of its parameter types — a byte
in `0..=255`, a non-zero divisor, an in-bounds index.

```vow
fn write_u8(out: Vec<i64>, v: i64) vow {
    requires: v >= 0,
    requires: v <= 255
} { out.push(v); }
```

**Strength:** a precondition is strong when it is the *true* domain of the
function — no wider (which would admit miscompilation) and no narrower (a
verifier-driven bound like `requires: n <= 8`, forbidden by `contracts.md`).
A bounds-check precondition such as `requires: i >= 0, requires: i < v.len()` is
the standard guard for every indexing operation.

### 2. Output-range postcondition (the weak default — use sparingly)

**Intent:** constrain the result to a range.

**When:** the range *is* the full functional spec — e.g. a function whose only
guarantee is non-negativity. This is rare. Most uses are the weak trap above.

```vow
fn item_kind(v: i64) -> i64 vow {
    requires: v >= 0,
    ensures: result >= 0          // weak: any non-negative value satisfies this
} { v / 4294967296 }
```

**Strength:** weak by default. Reach for shape 3, 4, or 5 instead whenever the
function actually computes a *specific* value. If you find yourself writing
`ensures: result >= 0` on a function that returns a computed quantity, ask what
the result *equals* or *inverts*, and assert that.

### 3. Exact functional postcondition (equality)

**Intent:** pin the result to the value the function is defined to produce.

**When:** the output is a closed-form function of the inputs (arithmetic,
bit-packing, encodings).

```vow
fn region_pack(kind: i64, val: i64) -> i64 vow {
    requires: kind >= 0,
    requires: kind <= 3,
    requires: val >= 0,
    requires: val <= 4294967295,
    ensures: result == val * 4 + kind     // exact: only the right answer passes
} { val * 4 + kind }
```

**Strength:** strong — a wrong shift or offset is caught immediately. Note the
preconditions are not verifier appeasement: they bound `val` and `kind` so the
packed result cannot overflow `i64` (a genuine semantic constraint). Contrast
`region_pack` (exact) with `item_pack`/`item_kind` (shape 2, only `>= 0`): the
same bit-packing pattern, one specified strongly and one weakly.

### 4. Round-trip / inverse

**Intent:** prove that an encode/decode (or pack/unpack, serialize/deserialize)
pair compose to the identity on the valid domain.

**When:** two functions are defined as inverses — `pack`/`unpack`,
`encode`/`decode`, `to_bytes`/`from_bytes`.

The strongest form asserts the inverse directly, calling the partner function in
the postcondition (pure-function calls are allowed in contracts):

```vow
fn region_kind(r: i64) -> i64 vow {
    requires: r >= 0,
    ensures: result == r - (r / 4) * 4,   // exact extractor
    ensures: result <= 3
} { r - (r / 4) * 4 }
```

A `region_pack` then `region_kind`/`region_val` round-trip recovers `(kind, val)`
exactly. **Strength:** very strong — round-trip is the property a serialization
layer must have, and it catches the entire class of "encoder and decoder drifted
apart" bugs that output-range contracts miss completely.

### 5. Dispatch totality (fail-closed decoders)

**Intent:** prove that a decoder/dispatcher maps **every** valid input to a
defined output and **never** silently falls through to a default.

**When:** a function switches over a tag/opcode/discriminator. This is the
single highest-value shape for Vow, because silent-fallback normalization
(mapping an unknown tag to a valid-looking default) is the bug class issue #81
was filed over.

The pattern has two halves — a validity precondition and an explicit error
sentinel for the unreachable tail:

```vow
fn is_valid_binop(op: i64) -> bool { op >= 0 && op <= 22 }

fn binop_opcode(op: i64, operand_ty: i64) -> i64 vow {
    requires: is_valid_binop(op)
} {
    if op == BINOP_ADD() { return ...; }
    // ... one arm per valid op ...
    -1                                    // unreachable under the precondition
}
```

**Strength — and a live hardening gap.** The precondition pins the domain, but
this contract does **not yet prove totality**: nothing asserts the function never
returns the `-1` sentinel. The strong form adds a postcondition that rules out
the fallthrough:

```vow
fn binop_opcode(op: i64, operand_ty: i64) -> i64 vow {
    requires: is_valid_binop(op),
    ensures: result != -1                 // proves every valid op is handled
} { ... }
```

With `ensures: result != -1`, ESBMC must show that on every `op` in `0..=22` some
arm returns before the sentinel — i.e. the dispatch is exhaustive. If an agent
later adds opcode 23 to `is_valid_binop` but forgets the matching arm,
verification fails instead of miscompiling. This is the contract that converts a
silent fallback into a caught error.

> Vow has no surface quantifier (`forall i in 0..n`) today, so "covers all valid
> inputs" is expressed as `requires` (pin the finite domain) + a postcondition
> that excludes the failure value, letting ESBMC enumerate the finite branch
> structure. Bounded quantifiers are tracked as a roadmap item (#467/#470).

### 6. Relational / cross-function (uniqueness, agreement)

**Intent:** state a property that spans more than one function or more than one
argument.

**When:** tags in a family must be distinct; two collections must have equal
length; a result must relate two inputs.

The argument-relational form is directly expressible:

```vow
fn build_pairs(ids: Vec<i64>, names: Vec<i64>) -> Vec<Pair> vow {
    requires: ids.len() == names.len()
} { ... }
```

The *cross-function uniqueness* form — "`tok_kw_fn() != tok_kw_let()` for every
pair in the family" — is expressible only as O(n²) pairwise inequalities, which
does not scale to dozens of tag constants. **The better fix is structural, not
contractual:** encode each family as a base+offset range or a generated table so
uniqueness is a property of the *encoding* rather than something every function
must restate. Treat a wall of zero-argument tag constants as an API smell that
shape-6 contracts cannot economically repair.

### 7. Loop invariant / frame

Covered in `contracts.md` (counter bounds, search-range invariants, the
inductiveness requirement). The methodology point: an invariant is strong when it
is the property the loop *maintains toward its postcondition*, not merely
`i >= 0`. See `contracts.md` §Loop Invariants and the worked CEGIS cycle in
`examples.md`.

## Hollow contracts: three failure modes to detect

A contract can pass verification while proving nothing. There are three distinct
ways this happens; a contract-quality tool should distinguish them.

### Weakness

The clause is satisfiable and true, but so loose that an incorrect
implementation also satisfies it (`ensures: result >= 0` on a computed value).
This is the 354-contract problem. **Detection (mutation-based):** mutate the
implementation — flip a constant, swap an operator — and re-verify. If the
contract still proves against the *mutated* body, it does not constrain that
behavior and is too weak. Vow already has the machinery for this in
`vowc mutants`; a weak contract is one whose function's mutants survive.

### Tautology

The clause is true independent of the program — `ensures: true`,
`ensures: result == result`, `ensures: x >= 0 || x < 0`. **Detection:** the clause
is valid (provable) with the function body removed; a cheap check folds constant
clauses and flags any clause with no dependence on parameters or `result`.

### Vacuity (antecedent failure)

The clause is proved only because its **preconditions are unsatisfiable**, so the
path it guards is dead and the postcondition never has to hold. Because Vow
lowers `requires` to `__ESBMC_assume`, a contradictory or over-strong precondition
makes *any* `ensures` provable — an assume-false / dead-path proof.

This is the classic vacuity of Beer, Ben-David, Eisner, and Rodeh, *"Efficient
Detection of Vacuity in Temporal Model Checking"* (Formal Methods in System
Design, 2001): a subformula is vacuous when replacing it changes nothing about
the result. Their industrial data is the reason to take it seriously — across
years of hardware verification at IBM, ~20% of formulas were trivially valid on
first runs, and trivial validity *always* indicated a real defect in the design,
spec, or environment.

**Detection (the `false` re-check):** re-verify each obligation with its `ensures`
replaced by `ensures: false`. If `assert(false)` still passes, the path is
unreachable under the preconditions — the original proof was vacuous. A non-vacuous
obligation must *fail* this check.

**Interesting witnesses.** Beer et al. also propose the dual of a counterexample:
for a proof that holds, emit a non-trivial *witness* — concrete inputs that
exercise the property for a substantive reason — so the author can confirm the
proof is not hollow. Vow's structured output is well-suited to carrying a witness
alongside each `Verified` result.

## When to write contracts

### Builtins and `extern` blocks

Runtime functions (`Vec.push`, `String.from`, `HashMap.insert`) are implemented in
Rust/C and cannot be verified by ESBMC. Their behavior enters verification through
the `vow` contract on the `extern "C"` block, which becomes an **assumed**
(`__ESBMC_assume`) surface for callers. Because these contracts are *assumed, not
checked*, they are the most dangerous place for an error: a wrong `ensures` on an
extern block silently weakens every proof that depends on it. Extern contracts
must be reviewed as assumptions, audited against the runtime implementation, and
kept minimal. (Omitting the block is a `MissingContract` error — see `errors.md`.)

### Library functions (written in Vow)

Public Vow functions are fully within verification reach. Give each one its true
domain precondition and the strongest postcondition shape that applies (3–6, not
2). Add contracts when the function's contract is *known*, which is usually at
definition time for pure utilities and after the signature stabilizes for APIs.

### Application code (including agent-generated)

Vow's target author is an AI agent, and the failure mode to design against is
*volume over substance* — an agent emitting many `ensures: result >= 0` clauses
because the prompt said "add contracts." Skill guidance should push the opposite:
for each function, identify which shape applies (equality, round-trip, dispatch
totality, relational) and write that one; prefer one discriminating contract to
five weak ones. The Specification Pattern System of Dwyer, Avrunin, and Corbett
(ICSE 1999) — a survey-validated catalog built specifically to turn imprecise
intent into precise specifications — is the model for guiding an author from "this
should be valid" to a postcondition that says what *valid* means.

## Expressibility and verifiability matrix

Whether a shape is usable depends on five independent axes, not just "can the
syntax say it":

- **expressible** — surface Vow has the syntax
- **typechecked** — the checker validates the clause to `bool` (added in #81 Phase 0)
- **lowerable** — the lowerer emits IR for the clause
- **modelable** — the C emitter / ESBMC model supports the operations used
  (pure, non-effectful helpers only; see `is_modelable` in the C emitter)
- **backend** — ESBMC actually discharges it within bounds

| Shape | expressible | typechecked | lowerable | modelable | backend |
|-------|:-----------:|:-----------:|:---------:|:---------:|:-------:|
| 1. Domain precondition | ✓ | ✓ | ✓ | ✓ | ✓ |
| 2. Output-range postcond. | ✓ | ✓ | ✓ | ✓ | ✓ (but weak) |
| 3. Exact equality | ✓ | ✓ | ✓ | ✓ | ✓ within overflow bounds |
| 4. Round-trip / inverse | ✓ | ✓ | ✓ | ✓ if partner is pure & modelable | ✓ for arithmetic |
| 5. Dispatch totality | ✓ | ✓ | ✓ | ✓ (pure dispatch) | ✓ over finite domain |
| 6a. Argument-relational | ✓ | ✓ | ✓ | ✓ | ✓ |
| 6b. Cross-fn uniqueness | ✓ (O(n²)) | ✓ | ✓ | ✓ | ✓ but unscalable → prefer structural encoding |
| 7. Loop invariant | ✓ | ✓ | ✓ | ✓ | partial (bounded / k-induction) |
| Bounded quantifier (`forall i in 0..n`) | ✗ (no surface syntax) | — | — | — | — (roadmap #467/#470) |

Contract expressions must be **pure** — they cannot call effectful functions
(`grammar.md` §Contract Purity). A property that needs an effectful helper is
blocked at the *modelable* axis, not the expressible one; classify such gaps as
model limitations, not contract-language limitations.

## Tooling (planned)

`vow contracts --verify` lists contracts and verifies them per function today. The
quality work tracked in #81 / roadmap WS-3.2 extends it to a **per-obligation**
check that emits a quality signal per clause — flagging tautologies (cheap, no
ESBMC), vacuous obligations (the `false` re-check), and, via the `vowc mutants`
harness, weak obligations whose function's mutants survive. Until that ships, the
three detections above are checks an author can run by hand.

## References

- N. Polikarpova, C. A. Furia, Y. Pei, Y. Wei, B. Meyer. *What Good Are Strong
  Specifications?* ICSE 2013. https://arxiv.org/abs/1208.3337
- I. Beer, S. Ben-David, C. Eisner, Y. Rodeh. *Efficient Detection of Vacuity in
  Temporal Model Checking.* Formal Methods in System Design 18:141–163, 2001.
- M. Dwyer, G. Avrunin, J. Corbett. *Property Specification Patterns for
  Finite-State Verification.* ICSE 1999.
- B. Meyer. *Object-Oriented Software Construction* (Design by Contract).

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

### StaticLiteralRequired

**Phase:** Type Checker
**Meaning:** A compiler intrinsic requires a string literal operand so it can be lowered without allocation.

```vow
fn f(s: String, key: String) -> i64 {
    string_matches_literal_at(s, 0, key)
}
```

**Output:** `string_matches_literal_at requires a string literal as its third argument`

**Fix:** Pass a literal directly, for example `string_matches_literal_at(s, 0, "name")`.

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
**Meaning:** A value of a `linear struct` type is used in a way that is immediately invalid before region inference runs, such as consuming it twice, consuming it inside a loop that may execute more than once, or consuming it after only some control-flow paths already consumed it.

```vow
linear struct Handle { fd: i64 }

fn f(h: Handle) -> Handle {
    let h2: Handle = h;
    let h3: Handle = h;  // h was already consumed
    h2
}
```

**Fix:** Restructure ownership so each path uses a consumed linear value at most once. Obligations that are simply left live at scope exit are reported later as `RegionLinear`.

### RegionLinear

**Phase:** Region Inference
**Meaning:** A `linear struct` value can remain live when its owning region closes. Returning the value transfers the linear obligation to the caller; consuming it before the close satisfies the obligation.

```vow
linear struct Handle { fd: i64 }

fn f() -> i64 {
    let h: Handle = Handle { fd: 1 };
    0
}
```

**Fix:** Consume the value before the region closes, or return it so the caller receives the obligation.

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

### BTreeMapKeyTypeMustBeI64

**Phase:** Type Checker
**Meaning:** A `BTreeMap<K, V>` was instantiated with `K` not equal to `i64`. Phase 1 of the BTreeMap stdlib only supports `i64` keys; the runtime helpers and ESBMC C model are hard-coded to i64.

```vow
fn f() -> () {
    let m: BTreeMap<bool, i64> = BTreeMap::new();
    m.insert(true, 1);
}
```

**Output:** `BTreeMap key type must be i64; found 'bool'`

**Fix:** Use `BTreeMap<i64, V>`. If you need string or struct keys, hash or intern them to `i64` at the call site and keep a side-table for the originals.

### BTreeMapValueMustBeNonLinear

**Phase:** Type Checker
**Meaning:** A `BTreeMap<K, V>` was instantiated with a `V` that is or transitively contains a `linear struct`. Non-linear containers like `BTreeMap`, `Vec`, and `HashMap` cannot hold linear values because their internal shift/copy operations are bitwise and would silently duplicate the linear ownership obligation.

```vow
linear struct Token { id: i64 }
fn f() -> () {
    let m: BTreeMap<i64, Token> = BTreeMap::new();
}
```

**Output:** `BTreeMap value type must be non-linear; found 'Token'`

**Fix:** Either drop the `linear` qualifier on the struct, or keep handles in a `Vec<i64>` indirection and consume the linear values via direct function calls outside the map.

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

### ContractTypeMismatch

**Phase:** Type Checker
**Meaning:** A `requires`, `ensures`, or `invariant` clause expression does not have type `bool`.

```vow
fn add(a: i64, b: i64) -> i64 vow {
    requires: a + b
} {
    a + b
}
```

**Output:** `` `requires` clause has type `i64` but must be `bool` ``

**Fix:** Ensure every contract clause is a boolean expression (comparison, logical operator, or a call to a predicate function returning `bool`).

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

### RegionConflict

**Phase:** Region Inference (arena-per-scope, Phase 3)
**Meaning:** A heap-typed value's required lifetime cannot be satisfied by the regions the surrounding code provides. This fires when an interprocedural store-effect constraint is unsatisfiable against the **inferred** region — that is, the value's `region(I) = LUB(must_outlive(I))` resolves to a concrete block strictly narrower than the target container's region.

> **Coverage note (as of issue #314):** the check is now semantic, consulting
> the inferred region populated by §4.1 step 3's LUB pass rather than the
> raw IR opcode. A fresh allocation routed through a callee's store-effect
> chain into a parameter container has its inferred region widened to
> `Caller(HiddenRegionIdx(N))` by §4.1 step 2's must-outlive marker
> propagation, where `N` is the precise slot index implied by the
> destination (issue #317 slot-aware inference). Such single-slot routings
> satisfy the constraint and are accepted. Allocations whose caller-region
> markers require more than one hidden caller-arena slot resolve to
> `Caller(HiddenRegionIdx::AMBIGUOUS)` and are rejected when the directly
> fresh heap value is stored into a parameter-rooted target; allocations
> whose inferred region is a strictly narrower block also fire
> `RegionConflict`.

```vow
fn store_into(out: Vec<String>, prefix: String) [io] {
    let s: String = String::from(prefix);
    s.push_str(String::from(" world"));
    out.push(s);  // s is allocated in this function's scope but escapes into out's region
}
```

**Fix:** Move the allocation to a wider scope, or copy the value into the target region (e.g., `String::from(s)` into the outer arena). For routings that compile cleanly but you'd like to know about (root-region placement), see `RegionRootEscape` below. See `docs/design/arena_memory.md` §4.4 for the full rejection vs. visibility distinction.

### RegionRootEscape

**Phase:** Region Inference (arena-per-scope, Phase 3)
**Severity:** Note (informational — does not fail the build)
**Meaning:** A heap allocation's inferred region is `Caller`, and the surrounding function publishes a `FreshInCaller` return summary or store effect — so the allocation may flow up the caller chain to `main` and ultimately land in the root region (`__vow_root_arena`, never freed). This is a memory-cost decision the compiler surfaces visibly per `docs/design/arena_memory.md` §4.4: silent root-region placement caused growth-with-no-signal in earlier compiler versions, and the note restores that signal without conflating it with unsoundness (`RegionConflict`).

The note is conservative — it fires for any `Caller`-region allocation in a function that could route to a caller, even if the actual concrete chain in this program doesn't reach `main`. False positives are tolerated because the diagnostic is non-blocking.

```json
{
  "error_code": "RegionRootEscape",
  "severity": "note",
  "message": "allocation may live in the root region: routed via store-effect chain to a caller whose target_region ultimately resolves to root",
  "hints": [
    "if intentional (e.g. program-lifetime data), no action needed; if you want this allocation freed earlier, restructure so the value is returned rather than stored into a parameter container"
  ]
}
```

**Fix:** Often none — if the program is short-lived (a checker, a CLI tool) or the values are genuinely program-lifetime, the note is informational. To free the allocation earlier, restructure so the value is **returned** from the constructing function rather than stored into a parameter container; the canonical `FreshInCaller` return path (`fn make_X() -> X`) does not trigger the note for the returned value or any allocation installed as a field of the returned struct (e.g. `Item { name: String::from("hi") }`). The exemption applies only to the *currently-installed* field initializers — a field overwritten before the return (`x.f = A; x.f = B; return x`) does not suppress the dead allocation `A`, which fires the note as expected (per-block last-write dedup, issue #326).

### VerificationSkipped

**Phase:** Verification (Warning surfaced alongside `BuildStatus::Skipped`)
**Meaning:** The function carries a `vow {}` block but its body uses opcodes the verifier's C model cannot represent — most commonly `RegionAlloc` and `FieldSet` produced by struct construction, also `Load`/`Store`, `RemF*`, and the `Linear*` family. The function is skipped before any C is emitted or ESBMC is invoked. The contract becomes documentary: runtime checks still apply in `--mode debug`, but no static proof is attempted.

```json
{
  "error_code": "VerificationSkipped",
  "severity": "warning",
  "message": "skipped verification of `ir_inst_set_region`: function `ir_inst_set_region` is not modelable in the verifier (contains unsupported opcode `RegionAlloc`)",
  "hints": [
    "the contract is documentary; runtime checks still apply in --mode debug"
  ]
}
```

**Why the build fails closed.** Per `CLAUDE.md`'s "Contract Authoring" guidance, contracts express semantic correctness and must not be weakened to fit the verifier. When the verifier's bounded model checker cannot represent a function's body, the function is skipped with a structured warning instead of tripping the defense-in-depth `__ESBMC_assert(0, "vow:UNSUPPORTED_OP_VOW_ID")` that historically broke the bootstrap on every vowed struct-builder. But a skipped contract is still an unproved contract, so the build lifts its overall status to `Skipped` (exit 1). Use `--no-verify` if you explicitly want a non-failing path that does not invoke ESBMC at all (`Unverified`, exit 0).

**Fix:** Refactor the function so its body uses only modelable opcodes — typically by splitting allocation/initialisation away from the contract-bearing computation. Alternatively, run with `--no-verify` if the contract is intentionally documentary.

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

### RegionLiteralMutation

**When:** A `Vec`, `String`, or `HashMap` mutation is attempted on a literal-backed container — one whose descriptor carries the `VOW_CAP_RODATA` sentinel (backing lives in `.rodata` or was pinned to the root region). Calls that statically trace a mutating target to a literal are rejected during compilation with this code; a runtime fallback emits the JSON shape below if an unchecked mutation reaches a `VOW_CAP_RODATA` descriptor. See `docs/design/arena_memory.md` §6.1, §7.3.

```json
{"error":"RegionLiteralMutation","operation":"String::push_str","origin":"rodata"}
```

A plain-text hint follows on the next line (not a JSON field). The hint text is dispatched on the operation's type prefix:

```
hint: make an explicit mutable copy with String::from(value) before mutating  # for String::* operations
hint: construct a mutable Vec and copy entries before mutating                # for Vec::*    operations
hint: construct a mutable HashMap and copy entries before mutating  # for HashMap::* operations
```

The `operation` field identifies the source-level method that trapped (e.g., `Vec::push`, `Vec::pop`, `HashMap::insert`, `String::clear`). The `origin` field identifies the storage class of the immutable backing; today only `rodata` is emitted.

**Fix:** Obtain an explicit mutable copy before mutation: `String::from(value)`, or construct a fresh mutable container and copy the entries you need before mutating.

### StackOverflow

**When:** The native call stack is exhausted, typically due to unbounded recursion.

```json
{"error":"StackOverflow"}
```

In debug or sanitize mode, the diagnostic includes call depth and the function that was executing when the overflow occurred:

```json
{"error":"StackOverflow","depth":10693,"function":"recurse"}
```

The signal handler is installed in **all** build modes. The `depth` and `function` fields are only available in debug/sanitize mode where call-depth instrumentation is emitted.

**Fix:** Add a base case to recursive functions, or restructure the algorithm to use iteration instead of recursion.

### OutOfMemory

**When:** A runtime arena operation (`__vow_arena_open` or `__vow_arena_alloc`) failed because the underlying `malloc` returned null. Non-recoverable from within Vow (`docs/design/arena_memory.md` §3.3, §16).

```json
{"error":"OutOfMemory","operation":"arena_alloc"}
```

The `operation` field is `arena_open` for the initial chunk allocation or `arena_alloc` for a later fallback chunk allocation.

**Fix:** Reduce working-set size, raise the process memory limit, or run on a machine with more memory. This is not a Vow program error.

## Warnings

### LoweringWarning

**Phase:** IR Lowering
**Meaning:** The IR lowerer could not resolve a struct type tag or field name, defaulting to index 0. This usually indicates a missing type annotation on a `let` binding, causing the compiler to lose track of which struct type a pointer refers to.

**Fix:** Add an explicit type annotation: `let x: MyStruct = ...;` so the compiler can track struct type tags through the IR.

---

# Worked Examples

Verification workflow examples. The first three demonstrate Counterexample-Guided Inductive Synthesis (CEGIS) cycles: write spec, build, read JSON, diagnose, fix, verify. The fourth shows break-with-value in loop expressions. The fifth shows an EOF-safe interactive command loop using `stdin_read_line()`. The sixth shows bounded-memory streaming file input.

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
- `requires: n <= 8` keeps iterations tractable for verification
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

## 5. Command Loop — EOF-Safe `stdin_read_line`

### Goal

Write a line-oriented command interpreter that reads from stdin, dispatches commands, skips empty lines, and exits cleanly on EOF. This is the canonical pattern for CI-safe interactive programs.

### Step 1: Write the program

```vow
module CmdLoop

fn trim_newline(s: String) -> String {
    let n: i64 = s.len();
    if n == 0 { return s; }
    let last: i64 = s.byte_at(n - 1);
    if last == 10 {
        if n >= 2 {
            let prev: i64 = s.byte_at(n - 2);
            if prev == 13 {
                return s.substring(0, n - 2);
            }
        }
        return s.substring(0, n - 1);
    }
    s
}

fn skip_spaces(s: String, start: i64) -> i64 {
    let mut i: i64 = start;
    let n: i64 = s.len();
    while i < n {
        if s.byte_at(i) != 32 { return i; }
        i = i + 1;
    }
    i
}

fn main() -> i32 [read, io] {
    let mut line: String = stdin_read_line();
    while line.len() > 0 {
        let cmd: String = trim_newline(line);

        if cmd.len() > 0 {
            if cmd.eq(String::from("quit")) {
                return 0;
            }

            if cmd.eq(String::from("hello")) {
                print_str(String::from("Hello, world!\n"));
            } else {
                if cmd.len() >= 5 {
                    let prefix: String = cmd.substring(0, 5);
                    if prefix.eq(String::from("echo ")) {
                        let start: i64 = skip_spaces(cmd, 5);
                        let text: String = cmd.substring(start, cmd.len());
                        print_str(text);
                        print_str(String::from("\n"));
                    } else {
                        print_str(String::from("unknown: "));
                        print_str(cmd);
                        print_str(String::from("\n"));
                    }
                } else {
                    print_str(String::from("unknown: "));
                    print_str(cmd);
                    print_str(String::from("\n"));
                }
            }
        }

        line = stdin_read_line();
    }
    0
}
```

### Step 2: Build

```
$ vow build --no-verify examples/cmdloop.vow -o /tmp/cmdloop
```

```json
{"status":"Unverified","executable":"/tmp/cmdloop","diagnostics":[],"counterexamples":[]}
```

No contracts here — this example focuses on the I/O pattern, not verification.

### Step 3: Run with piped input

```
$ printf 'hello\necho Vow is great\n\nbogus\nquit\n' | /tmp/cmdloop
Hello, world!
Vow is great
unknown: bogus
```

The `quit` command causes an early `return 0`. Empty lines are silently skipped.

### Step 4: Run with EOF (no quit)

```
$ printf 'hello\necho test\n' | /tmp/cmdloop
Hello, world!
test
```

When stdin is exhausted, `stdin_read_line()` returns `""` (length 0), the `while` condition fails, and the program exits cleanly with code 0.

### Key points

- **EOF detection:** `stdin_read_line()` returns `""` at EOF. Check `.len() > 0` to exit the loop.
- **Newline stripping:** `stdin_read_line()` includes the trailing `\n` (or `\r\n`). Strip it with `byte_at` + `substring` before comparing commands.
- **Empty line handling:** After trimming, `cmd.len() == 0` means the line was blank — skip it.
- **Effects:** `stdin_read_line()` requires `[read]`; `print_str()` requires `[io]`. The `main` function declares both.
- **CI-safe:** No blocking reads, no prompts — the program processes whatever stdin provides and exits at EOF. Safe to run in pipelines and test harnesses.

## 6. Streaming File Input

`fs_read(path)` materializes the entire file as one `String`. Use `fs_open` plus `fs_read_line` for newline-delimited files that may be large.

```vow
module StreamingFile

fn main() -> i32 [read, io] {
    let argv: Vec<String> = args();
    if argv.len() < 2 {
        eprintln_str(String::from("usage: streaming_file <path>"));
        return 1;
    }

    let h: i64 = fs_open(argv[1]);
    if h < 0 {
        eprintln_str(String::from("could not open input"));
        return 1;
    }

    let mut lines: i64 = 0;
    let mut bytes: i64 = 0;
    let mut line: String = fs_read_line(h);
    while line.len() > 0 {
        lines = lines + 1;
        bytes = bytes + line.len();
        line = fs_read_line(h);
    }

    if fs_status(h) != 1 {
        fs_close(h);
        eprintln_str(String::from("read error"));
        return 1;
    }
    if fs_close(h) != 0 {
        eprintln_str(String::from("close error"));
        return 1;
    }

    print_i64(lines);
    print_str(String::from("\n"));
    print_i64(bytes);
    print_str(String::from("\n"));
    0
}
```

Key points:

- `fs_read_line(handle)` includes the trailing newline when present.
- Blank lines are returned as `"\n"`; EOF returns `""`.
- Check `fs_status(handle)` after `fs_read_line(handle)` returns `""`: `1` means EOF, `-1` means invalid handle or read error.
- Close each successful handle with `fs_close(handle)` and check for a non-zero close result.

## 7. BTreeMap basic usage

`BTreeMap<i64, V>` is the deterministic alternative to `HashMap` — sorted ascending by key, binary-search lookup. Use it when iteration order affects program output (codegen, serialization, or any reproducible build).

```vow
module BTreeMapExample

fn fetch(m: BTreeMap<i64, i64>) -> Option<i64> [io] {
    let r: Option<i64> = m.get(7);
    let v: i64 = r?;
    print_i64(v);
    print_str(String::from("\n"));
    Option::Some(v)
}

fn main() -> i32 [io] {
    let m: BTreeMap<i64, i64> = BTreeMap::new();
    m.insert(7, 42);
    let prev: Option<i64> = m.insert(7, 99);
    // prev is Some(42); the second insert overwrote the first.
    fetch(m);
    print_i64(m.len());
    0
}
```

Note that `.insert` returns `Option<V>` (the previous value, if any), and `.get` returns `Option<V>`. Use `?` to short-circuit on `None`. Phase 1 only supports `i64` keys; using any other key type raises `BTreeMapKeyTypeMustBeI64`.

### Why BTreeMap and not HashMap

`HashMap.insert` returns `()` and its iteration order is unspecified. For maps whose iteration is observable in the output binary, the byte-identical bootstrap requirement (`stage1 == stage2` sha256) demands deterministic order. `BTreeMap` provides it; `HashMap` does not.

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
      "enum": ["Verified", "Unverified", "Skipped", "CompileFailed", "VerifyFailed"],
      "description": "Build outcome. `Skipped` means ESBMC was invoked but at least one vowed function could not be modelled; the build fails closed with exit 1 (distinct from `Unverified`, which means ESBMC was not invoked, e.g. `--no-verify`/`--dump-ir`, exit 0)."
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
      "enum": ["timeout", "unknown", "error", "tool_not_found"],
      "description": "Verification sub-status (present only when the verification backend did not produce a proof or counterexample)"
    },
    "verify_message": {
      "type": "string",
      "description": "Verification backend error detail (present only when verify_status is set)"
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
        "required": ["vow_id", "function", "kind", "description", "blame", "source", "status", "quality"],
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
            "enum": ["proven", "proven-ir", "failed", "unknown", "timeout", "error", "not_verified", "skipped"],
            "description": "Verification status"
          },
          "quality": {
            "type": "string",
            "enum": ["weak", "tautological", "substantive"],
            "description": "Static, no-ESBMC classification of the clause shape: weak (an ensures that only bounds result by a constant), tautological (constant clause that says nothing), or substantive (equality/relational/inverse/call). See contracts-methodology.md"
          }
        },
        "additionalProperties": false
      }
    },
    "summary": {
      "type": "object",
      "required": ["total", "proven", "failed", "unknown", "timeout", "error", "not_verified", "skipped", "quality"],
      "properties": {
        "total": { "type": "integer" },
        "proven": { "type": "integer" },
        "failed": { "type": "integer" },
        "unknown": { "type": "integer" },
        "timeout": { "type": "integer" },
        "error": { "type": "integer" },
        "not_verified": { "type": "integer" },
        "skipped": { "type": "integer" },
        "quality": {
          "type": "object",
          "description": "Static contract-quality tallies independent of verification status",
          "required": ["weak", "tautological", "substantive"],
          "properties": {
            "weak": { "type": "integer" },
            "tautological": { "type": "integer" },
            "substantive": { "type": "integer" }
          },
          "additionalProperties": false
        }
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
        "StaticLiteralRequired",
        "EffectViolation",
        "LinearTypeViolation",
        "NonExhaustiveMatch",
        "VowRequiresViolated",
        "VowEnsuresViolated",
        "VowInvariantViolated",
        "UnknownMethod",
        "UnsupportedFeature",
        "LoweringWarning",
        "MissingContract",
        "ContractTypeMismatch",
        "EsbmcNotFound",
        "IoError",
        "RegionConflict",
        "RegionLinear",
        "RegionRootEscape"
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

## mutants-result

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.org/schemas/mutants-result.schema.json",
  "title": "MutantsOutput",
  "description": "Output of `vow-mutants run` populates a directory (default `mutants.out/`). Each file has its own schema; the union is documented here.",
  "$defs": {
    "Mutant": {
      "description": "Per-mutant catalog record (in mutants.json).",
      "type": "object",
      "required": ["name", "file", "line", "col", "off", "len", "kind", "from", "to", "label", "clause_index"],
      "properties": {
        "name":         { "type": "string", "description": "Stable cargo-mutants-style name: `file:line:col: <label>`." },
        "file":         { "type": "string", "description": "Repo-relative source path. Independent of --workdir." },
        "line":         { "type": "integer", "minimum": 1, "description": "1-based source line of `off`." },
        "col":          { "type": "integer", "minimum": 1, "description": "1-based source column of `off`." },
        "off":          { "type": "integer", "minimum": 0, "description": "Byte offset of the mutation site." },
        "len":          { "type": "integer", "minimum": 0, "description": "Byte length of the mutation span." },
        "kind":         { "enum": ["op-flip", "const-flip", "body-replace", "contract-weaken"] },
        "from":         { "type": "string", "description": "Original source text at the site (empty for body-replace)." },
        "to":           { "type": "string", "description": "Replacement text spliced in for this mutation." },
        "label":        { "type": "string", "description": "Human-readable summary of the mutation." },
        "clause_index": { "type": "integer", "minimum": 0, "description": "Disambiguates sibling contract clauses on the same function. 0 for non-contract sites." }
      },
      "additionalProperties": false
    },
    "Outcome": {
      "description": "Per-mutant verdict (in outcomes.json).",
      "type": "object",
      "required": ["id", "name", "status", "tier", "oracle_ms"],
      "properties": {
        "id":         { "type": "integer", "minimum": 0, "description": "Position of this mutant within the shard's `mutants.json` array." },
        "name":       { "type": "string", "description": "Same name as the corresponding Mutant record." },
        "status":     { "enum": ["caught", "missed", "timeout", "unviable", "unrun"] },
        "tier":       { "enum": [1, 2], "description": "Oracle tier that produced the verdict." },
        "oracle_ms":  { "type": "integer", "minimum": 0, "description": "Per-mutant oracle wall-clock in milliseconds. Non-deterministic — excluded from the determinism guarantee." }
      },
      "additionalProperties": false
    },
    "Summary": {
      "description": "Aggregate counts across all of this shard's mutants. Same shape as the stdout summary line.",
      "type": "object",
      "required": ["total", "caught", "missed", "timeout", "unviable", "unrun", "shard"],
      "properties": {
        "total":     { "type": "integer", "minimum": 0 },
        "caught":    { "type": "integer", "minimum": 0 },
        "missed":    { "type": "integer", "minimum": 0 },
        "timeout":   { "type": "integer", "minimum": 0 },
        "unviable":  { "type": "integer", "minimum": 0 },
        "unrun":     { "type": "integer", "minimum": 0, "description": "Tier-1 survivors not run because the Tier-2 budget was exhausted." },
        "shard":     { "type": "string", "pattern": "^[0-9]+/[1-9][0-9]*$" }
      },
      "additionalProperties": false
    },
    "MutantsJson": {
      "description": "Format of `mutants.out/mutants.json`. Written before testing begins.",
      "type": "object",
      "required": ["version", "tool", "shard", "mutants"],
      "properties": {
        "version":  { "const": 1 },
        "tool":     { "const": "vow-mutants" },
        "shard":    { "type": "string", "pattern": "^[0-9]+/[1-9][0-9]*$" },
        "mutants":  { "type": "array", "items": { "$ref": "#/$defs/Mutant" } }
      },
      "additionalProperties": false
    },
    "OutcomesJson": {
      "description": "Format of `mutants.out/outcomes.json`. Written after all mutants in this shard have been classified.",
      "type": "object",
      "required": ["version", "summary", "outcomes"],
      "properties": {
        "version":  { "const": 1 },
        "summary":  { "$ref": "#/$defs/Summary" },
        "outcomes": { "type": "array", "items": { "$ref": "#/$defs/Outcome" } }
      },
      "additionalProperties": false
    }
  }
}
```

## test-result

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/test-result.schema.json",
  "title": "TestResult",
  "description": "JSON output from `vow test` on stdout",
  "type": "object",
  "required": ["status", "total", "passed", "failed", "skipped", "tests", "contract_density"],
  "properties": {
    "status": {
      "type": "string",
      "enum": ["TestsPassed", "TestsFailed"],
      "description": "Overall test outcome"
    },
    "total": {
      "type": "integer",
      "description": "Total number of tests discovered"
    },
    "passed": {
      "type": "integer",
      "description": "Number of tests that passed"
    },
    "failed": {
      "type": "integer",
      "description": "Number of tests that failed"
    },
    "skipped": {
      "type": "integer",
      "description": "Number of tests that were skipped"
    },
    "tests": {
      "type": "array",
      "items": { "$ref": "#/$defs/TestEntry" },
      "description": "Per-test results"
    },
    "contract_density": {
      "$ref": "#/$defs/ContractDensity",
      "description": "Contract density across tested modules"
    }
  },
  "$defs": {
    "TestEntry": {
      "type": "object",
      "required": ["file", "name", "status", "stdout", "stderr", "duration_ms", "diagnostics", "counterexamples"],
      "properties": {
        "file": { "type": "string", "description": "Path to the test .vow file" },
        "name": { "type": "string", "description": "Test name (file stem)" },
        "status": {
          "type": "string",
          "enum": ["passed", "failed", "timeout", "skipped", "compile_error", "verify_failed"],
          "description": "Per-test outcome"
        },
        "exit_code": {
          "type": ["integer", "null"],
          "description": "Process exit code, null on compile/verify failure or timeout"
        },
        "stdout": { "type": "string", "description": "Captured stdout from test binary" },
        "stderr": { "type": "string", "description": "Captured stderr from test binary" },
        "duration_ms": { "type": "integer", "description": "Wall-clock duration in milliseconds" },
        "diagnostics": {
          "type": "array",
          "items": { "$ref": "diagnostic.schema.json" },
          "description": "Compiler diagnostics (on compile_error)"
        },
        "counterexamples": {
          "type": "array",
          "items": { "$ref": "counterexample.schema.json" },
          "description": "ESBMC counterexamples (on verify_failed)"
        }
      }
    },
    "ContractDensity": {
      "type": "object",
      "required": ["functions_total", "functions_with_vows", "density_pct"],
      "properties": {
        "functions_total": { "type": "integer", "description": "Total non-main functions across tested modules" },
        "functions_with_vows": { "type": "integer", "description": "Functions with at least one vow block" },
        "density_pct": { "type": "number", "description": "Percentage of functions with vow contracts" }
      }
    }
  }
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
"##
    .to_string()
}

fn skill_support_files() -> &'static [(&'static str, &'static str)] {
    &[
        (
            r#"reference/grammar.md"#,
            r#"# Vow Grammar Reference

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
| `u8`   | 8-bit unsigned integer   |
| `u64`  | 64-bit unsigned integer  |
| `f32`  | 32-bit float (limited support — avoid in contracts) |
| `f64`  | 64-bit float (limited support — avoid in contracts) |
| `bool` | Boolean                  |
| `()`   | Unit type                |
| `!`    | Never type (diverges)    |

### Built-in Parameterized Types

| Type               | Description                     |
|--------------------|---------------------------------|
| `Vec<T>`           | Growable array                  |
| `Option<T>`        | Optional value (Some/None)      |
| `Result<T, E>`     | Success or error                |
| `String`           | UTF-8 string (backed by Vec<u8>)|
| `HashMap<K, V>`    | Key-value map (linear scan)     |
| `BTreeMap<K, V>`   | Sorted key-value map (binary search; ascending iteration). `K` must be `i64`; `V` may be any non-linear type |

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

String literals have type `String` and are backed by a read-only static
descriptor. Passing or returning a literal does not allocate. To obtain a
mutable, arena-owned copy, use `String::from("...")`.

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

### Bitwise Operators

| Operator | Meaning      |
|----------|--------------|
| `&`      | Bitwise AND  |
| `\|`     | Bitwise OR   |
| `^`      | Bitwise XOR  |
| `<<`     | Left shift   |
| `>>`     | Right shift  |

Bitwise operators require integer operands of the same type. Shift expressions return the left operand's type. `>>` is arithmetic for `i64` and logical for `u64`.

Unsuffixed integer literals are `i64` by default but coerce to the other operand's integer type when used with a bitwise or shift operator. The same coercion applies to constant expressions composed entirely of unsuffixed integer literals — including arithmetic (`1 + 1`), bitwise (`1 << 3`), and unary negation (`-5`). For example, given `let x: u64 = ...`, the expressions `x << 3`, `3 & x`, and `x << (1 + 1)` all type-check (the literal-constant side coerces to `u64`). This matches the coercion rule already used by arithmetic operators and comparisons. Use a `u64` suffix (`3u64`) to force the `u64` type explicitly.

### Logical Operators

| Operator | Meaning    |
|----------|------------|
| `&&`     | Logical AND (short-circuit) |
| `\|\|`   | Logical OR (short-circuit) |
| `!`      | Logical NOT|

`&&` and `||` use short-circuit evaluation: for `a && b`, `b` is only evaluated if `a` is true; for `a || b`, `b` is only evaluated if `a` is false.

### Operator Precedence

From loosest to tightest, Vow follows the usual C/Rust precedence for logical and bitwise operators:

`||`, `&&`, comparisons (`== != < <= > >=`), `|`, `^`, `&`, `<< >>`, `+ -`, `* / %`

Unary `-`, `!`, `&`, and `?` bind tighter than every binary operator.

Single `&` is overloaded by position: prefix `&expr` is borrow, while infix `lhs & rhs` is bitwise AND.

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

Linear struct values carry a linear obligation. The obligation must either be consumed before the value's owning region closes or transferred to the caller by returning the value.

### Struct Literals

Struct literal names must be PascalCase:

```vow
let p: Point = Point { x: 1, y: 2 };
```

### Field Access

```vow
p.x
```

### Field Assignment

```vow
p.x = 10;
```

### Passing Semantics

Structs are heap-allocated. A struct value is a pointer to a heap region, so passing a struct to a function passes the pointer — the function operates on the same heap data, not a copy. Field assignments inside the called function are visible to the caller:

```vow
fn shift_right(p: Point, dx: i64) {
    p.x = p.x + dx;
}

fn main() -> i32 [io] {
    let p: Point = Point { x: 0, y: 0 };
    shift_right(p, 5);
    print_i64(p.x);  // 5 — mutation visible to caller
    0
}
```

This enables in-place mutation patterns (e.g., make/unmake in search trees) without cloning. The same aliasing semantics apply when structs are stored in containers — see [Indexing](#indexing). To avoid aliasing, construct a fresh struct literal with the desired field values.

**Note:** For `linear struct` types, passing the value to a function consumes it; the caller cannot access it afterward. Returning a linear value transfers the obligation to the caller, so this is the normal way to hand an updated linear value back out of a function.

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
| `Vec::from_raw_parts_copy(ptr, len)` | `(i64, i64) -> Vec<T>` for flat scalar `T` |
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
| `String::from(s)`   | `(String) -> String` — mutable copy |
| `String::new()`     | `() -> String`              |
| `String::from_raw_parts_copy(ptr, len)` | `(i64, i64) -> String` |
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

### BTreeMap<K, V> Methods

Keys must be `i64` (K violations raise `BTreeMapKeyTypeMustBeI64`). Values may be any
non-linear type — primitives, structs, `Vec<T>`, `Option<T>`, or nested combinations.
A `V` that is or transitively contains a `linear struct` is rejected with
`BTreeMapValueMustBeNonLinear`, because the runtime/verifier shift values bitwise and
would silently duplicate a linear obligation.
Storage is two parallel sorted arrays (binary-search lookup, sorted-insert writes).
Iteration order is ascending by key and is **deterministic across runs and compilers** —
prefer `BTreeMap` over `HashMap` for any map whose iteration affects compiler output.

| Method              | Signature                   |
|---------------------|-----------------------------|
| `BTreeMap::new()`   | `() -> BTreeMap<K, V>`      |
| `.insert(k, v)`     | `(K, V) -> Option<V>` (returns the previous value bound to `k`, if any) |
| `.get(k)`           | `(K) -> Option<V>` (returns the value bound to `k`, or `None`)          |
| `.contains(k)`      | `(K) -> bool`               |
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

#### FFI Wrapper Intrinsics

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `pin_to_root`    | `fn(value: String) -> String` and `fn<T>(value: Vec<T>) -> Vec<T>` for flat scalar `T` | `[]` |

`pin_to_root` is a compiler intrinsic, not a user-defined generic. Each call site is monomorphised from the argument type. It always deep-copies the supported heap value into root storage; it does not inspect descriptor tags and does not claim idempotency. The current supported forms are `String` and `Vec<T>` where `T` is a flat scalar slot type (`i*`, `u*`, `f32`, `f64`, `bool`). Pointer-containing payloads, user structs, enums, and maps require hand-written deep-copy wrappers at the FFI boundary.

`String::from_raw_parts_copy(ptr: i64, len: i64)` copies `len` bytes from a raw C pointer into a fresh `String`. `Vec::from_raw_parts_copy(ptr: i64, len: i64)` copies `len` flat scalar slots into a fresh `Vec<T>`. The surface length type is `i64`; the code generator converts pointer and length values to the platform pointer-sized ABI type at the FFI boundary. Both helpers have a `FreshInCaller` return summary.

For pointer-containing C payloads, a wrapper must be written per type: call the extern, recursively copy every Vow-owned heap subobject into the target region, free every C-owned pointer according to the extern's ownership contract, then return the Vow-placed value. A bytewise copy of a pointer-containing payload is unsound because it preserves stale pointers into C-owned storage.

#### Print / IO

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `print_str`      | `fn(s: String) -> ()`                      | `[io]`     |
| `print_i64`      | `fn(v: i64) -> ()`                         | `[io]`     |
| `print_u64`      | `fn(v: u64) -> ()`                         | `[io]`     |
| `eprintln_str`   | `fn(s: String) -> ()`                      | `[io]`     |

#### Debug

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `debug_str`      | `fn(s: String) -> ()`                      | `[]`       |
| `debug_i64`      | `fn(v: i64) -> ()`                         | `[]`       |
| `debug_u64`      | `fn(v: u64) -> ()`                         | `[]`       |

**Debug print semantics:** Debug prints are effect-free and callable from pure functions. In debug and sanitize modes (`--mode debug`, `--mode sanitize`), they write to stderr. In release and profile modes, the debug call itself is not emitted — no function call occurs. However, argument expressions are still evaluated (a direct literal such as `"label"` is static, while `String::from("label")` still allocates a mutable copy). They are also no-ops during verification. Use them to trace values inside pure kernel code without restructuring the effect hierarchy.

#### Filesystem

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `fs_read`        | `fn(path: String) -> String`               | `[read]`   |
| `fs_open`        | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_read_line`   | `fn(handle: i64) -> String`                | `[read]`   |
| `fs_status`      | `fn(handle: i64) -> i64`                   | `[read]`   |
| `fs_close`       | `fn(handle: i64) -> i64`                   | `[read]`   |
| `fs_write`       | `fn(path: String, data: String) -> i64`    | `[write]`  |
| `fs_exists`      | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_mkdir`       | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_listdir`     | `fn(path: String) -> Vec<String>`          | `[read]`   |
| `fs_remove`      | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_remove_dir`  | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_is_dir`      | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_is_symlink`  | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_rename`      | `fn(old: String, new: String) -> i64`      | `[io]`     |

#### String Operations

| Function              | Signature                                        | Effects |
|-----------------------|--------------------------------------------------|---------|
| `string_substr`       | `fn(s: String, start: i64, len: i64) -> String`  | `[]`    |
| `string_split`        | `fn(s: String, delim: String) -> Vec<String>`    | `[]`    |
| `string_starts_with`  | `fn(s: String, prefix: String) -> i64`           | `[]`    |
| `string_ends_with`    | `fn(s: String, suffix: String) -> i64`           | `[]`    |
| `string_matches_literal_at` | `fn(s: String, pos: i64, literal: String literal) -> i64` | `[]` |
| `string_trim`         | `fn(s: String) -> String`                        | `[]`    |
| `string_to_upper`     | `fn(s: String) -> String`                        | `[]`    |
| `string_to_lower`     | `fn(s: String) -> String`                        | `[]`    |
| `string_replace`      | `fn(s: String, from: String, to: String) -> String` | `[]` |
| `string_join`         | `fn(parts: Vec<String>, sep: String) -> String`  | `[]`    |

#### Conversion

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `parse_i64`      | `fn(s: String) -> i64`                     | `[]`       |
| `i64_to_string`  | `fn(v: i64) -> String`                     | `[]`       |

#### Collections

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `vec_sort`       | `fn(v: Vec<i64>) -> Vec<i64>`              | `[]`       |

#### Time

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `time_unix`      | `fn() -> i64`                              | `[io]`     |
| `time_unix_ms`   | `fn() -> i64`                              | `[io]`     |

#### System

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `num_cpus`       | `fn() -> i64`                              | `[io]`     |
| `memory_root_arena_bytes` | `fn() -> u64`                    | `[io]`     |
| `memory_peak_bytes` | `fn() -> u64`                           | `[io]`     |
| `memory_alloc_count_since_start` | `fn() -> u64`              | `[io]`     |

`num_cpus()` returns the number of available logical CPUs (from `std::thread::available_parallelism`), or `1` if the query fails. Used to size worker pools (e.g. the default `--verify-jobs` value).

`memory_root_arena_bytes()` returns the current bytes retained by root-region arena chunks. `memory_peak_bytes()` returns the peak live bytes retained by all open arena chunks since process start. `memory_alloc_count_since_start()` returns the number of successful Vow arena allocation requests since process start. These queries do not allocate; they are effectful because they observe runtime process state.

#### Encoding

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `hex_encode`     | `fn(data: Vec<u8>) -> String`              | `[]`       |
| `hex_decode`     | `fn(s: String) -> Vec<u8>`                 | `[]`       |

#### Input

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `args`           | `fn() -> Vec<String>`                      | `[read]`   |
| `stdin_read`     | `fn() -> String`                           | `[read]`   |
| `stdin_read_line`| `fn() -> String`                           | `[read]`   |
| `stdin_ready`    | `fn() -> bool`                             | `[read]`   |

#### Process Management

| Function              | Signature                                        | Effects |
|-----------------------|--------------------------------------------------|---------|
| `process_exit`        | `fn(code: i64) -> !`                             | `[io]`  |
| `process_run`         | `fn(cmd: String, args: Vec<String>) -> i64`      | `[io]`  |
| `process_get_stdout`  | `fn() -> String`                                 | `[io]`  |
| `process_get_stderr`  | `fn() -> String`                                 | `[io]`  |
| `process_start`       | `fn(cmd: String, args: Vec<String>) -> i64`      | `[io]`  |
| `process_wait`        | `fn(pid: i64) -> i64`                            | `[io]`  |
| `process_wait_timeout`| `fn(pid: i64, timeout_ms: i64) -> i64`           | `[io]`  |
| `process_kill`        | `fn(pid: i64) -> i64`                             | `[io]`  |
| `process_stdout_for`  | `fn(pid: i64) -> String`                         | `[io]`  |
| `process_stderr_for`  | `fn(pid: i64) -> String`                         | `[io]`  |

**`args` semantics:** `args()` returns all process arguments including the program name at index 0 (matching C `argv` and Rust `std::env::args()` conventions). For `./my_program foo bar`, `args()` returns `["./my_program", "foo", "bar"]`. Use `args[1]` onward for user-supplied arguments. The Vec is empty only if the OS provides no arguments (unusual). Returns an empty String element if an argument is empty (`""`). Non-UTF-8 arguments are included as-is (byte content preserved).

**`fs_read` semantics:** `fs_read(path)` opens the file at `path`, reads its entire contents, and returns a String. Returns `""` (empty String) on any error (file not found, permission denied, I/O error, non-UTF-8 path). Does not block on regular files. Callers should check `result.len() == 0` to detect failure.

**Streaming file input:** `fs_open(path)` opens a file for incremental reading and returns a positive handle, or `-1` on path/open error. `fs_read_line(handle)` reads one line from the current cursor and returns it as a String, including the trailing newline when present. It returns `""` at EOF, for an invalid handle, or after a read error. A blank line is returned as `"\n"`, so newline-delimited callers can distinguish a real blank line from EOF by content. After `fs_read_line(handle)` returns `""`, call `fs_status(handle)` to distinguish EOF from error: `0` means the handle is open with no EOF/error state, `1` means EOF, and `-1` means invalid handle or read error. `fs_status(handle)` reports the result of the most recent `fs_read_line(handle)` call on that open handle; read it immediately after a `""` return because later reads may update it. `fs_close(handle)` releases the handle and returns `0` on success or `-1` for an invalid/already-closed handle. Long-running programs must close handles they no longer need. All streaming handle operations use the `[read]` effect, including `fs_close`, because closing a read handle releases read-stream state and does not mutate filesystem contents. The current runtime stores streaming handles in one process-global table, and `fs_read_line` holds that table lock while it reads the next line. This keeps the API simple for single-stream file processing, but it is not intended for latency-sensitive concurrent reads from multiple slow handles.

**Filesystem return values:** `fs_write`, `fs_mkdir`, `fs_remove`, `fs_remove_dir`, and `fs_rename` return `i64`: 0 on success, non-zero on failure. `fs_open`, `fs_status`, and `fs_close` use the streaming status codes above. `fs_exists`, `fs_is_dir`, and `fs_is_symlink` are predicates: they return 1 for true, 0 for false. Errors (null pointer, invalid UTF-8) also return 0, so callers cannot distinguish "false" from "error". `fs_is_symlink` uses `lstat`-equivalent semantics: a symlink reports 1 even when its target is a regular file or directory.

**`string_starts_with` / `string_ends_with` / `string_matches_literal_at` return values:** Return `i64`: 1 if true, 0 if false.

**`string_matches_literal_at` literal operand:** The third argument must be written as a string literal at the call site. The compiler lowers that literal to static bytes plus an explicit byte length, so no temporary `String` allocation is created and embedded NUL bytes are preserved. Passing a variable or computed `String` as the third argument is a type-check error (`StaticLiteralRequired`). Use `string_starts_with`, `string_ends_with`, or `String` methods when the needle must be dynamic.

**`process_run` vs `process_start`:** `process_run(cmd, args)` runs a subprocess synchronously and returns its exit code. After it returns, `process_get_stdout()` and `process_get_stderr()` retrieve the captured output of the most recent `process_run` call. `process_start(cmd, args)` launches a subprocess asynchronously and returns a process ID. Use `process_wait(pid)` to wait for completion and get the exit code, and `process_stdout_for(pid)` / `process_stderr_for(pid)` to retrieve output.

**`process_wait_timeout`:** `process_wait_timeout(pid, timeout_ms)` polls a process started with `process_start` until it exits or the timeout (in milliseconds) elapses. Returns the exit code on completion, `-1` on error, or `-2` on timeout. After a timeout, the process is still running; use `process_kill(pid)` to terminate it.

**`process_kill`:** `process_kill(pid)` sends a kill signal to a running process and waits for it to exit. Returns 0 on success, -1 on error. No-op (returns 0) if the process has already completed.

**`stdin_read` vs `stdin_read_line`:** `stdin_read()` reads the entire stdin stream into a single String (unbounded memory). `stdin_read_line()` reads one line at a time, including the trailing newline. Returns `""` (empty string) at EOF. The returned String is runtime scratch storage valid until the next `stdin_read_line()` call. Process each line before reading the next one for bounded memory; use `pin_to_root(line)` before the next read when a line must be stored, returned, passed to a function that may store it, mutated, or otherwise retained. The direct scratch line is read-only. The scratch buffer keeps the largest line capacity seen so far, so memory is bounded by maximum line length rather than total input, but one very large line can retain that capacity for the process lifetime.

```vow
let lines: Vec<String> = Vec::new();
let mut line: String = stdin_read_line();
while str_len(line) > 0 {
    // Without pin_to_root, lines.push(line) would store the scratch alias, not a copy.
    lines.push(pin_to_root(line));
    line = stdin_read_line();
}
```

```vow
let mut line: String = stdin_read_line();
while str_len(line) > 0 {
    // process line (has trailing \n)
    line = stdin_read_line();
}
```

**`stdin_ready`:** `stdin_ready()` returns `true` if `stdin_read_line()` would return immediately without blocking, `false` otherwise. Uses a non-blocking poll with zero timeout. Use this in computation loops that must remain responsive to external input:

```vow
while !stdin_ready() && depth < max_depth {
    // continue searching
    depth = depth + 1;
}
if stdin_ready() {
    let cmd: String = stdin_read_line();
    // handle command
}
```

## Canonical Form

The canonical printer normalizes source: `parse → print → parse` is idempotent. Effects are sorted alphabetically, indentation uses 4 spaces, trailing expressions omit semicolons.
"#,
        ),
        (
            r#"reference/cli.md"#,
            r#"# Vow CLI Reference

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
| `--mode <debug\|release\|profile\|sanitize>` | `release` | Build mode: debug inserts runtime vow checks, profile inserts call counters and prints report on normal exit, sanitize adds debug checks + Vec provenance tracking |
| `--no-verify`     | (off)       | Skip ESBMC static verification            |
| `--dump-ir`       | (off)       | Print IR text to stdout and exit (no JSON output, no codegen) |
| `--debug-trace <off\|calls\|full>` | `off` | Emit JSON trace lines to stderr at runtime |
| `--no-cache`    | (off)       | Disable verification result caching, and (for `--no-verify` builds) the compile-object cache. See "Compile-object cache behavior" below |
| `--max-k-step <N>` | `50`     | ESBMC incremental BMC max iterations          |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver; auto selects per-function via heuristic |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 |
| `--timeout <N>` | `300` (or `30` when `--encoding` is `auto`) | ESBMC per-function timeout in seconds. Under `--encoding auto`, a 30s default is applied so the BV-timeout fallback to `--encoding ir --solver z3` can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit `--timeout` overrides both. `--timeout 0` is honoured as an immediate watchdog kill |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent ESBMC verification jobs |

**Compile-object cache behavior.** The on-disk compile-object cache (`$VOW_CACHE_DIR` or `~/.cache/vow/`, where each entry is a `<key>.o` artifact keyed by a content hash of all dependencies, mode, and trace settings) is automatically disabled whenever ESBMC verification is active. This guarantees the linked binary always comes from the same codegen run whose IR was verified, closing the integrity gap where a stale or attacker-supplied `.o` could be linked against freshly-verified IR. Concretely the cache only activates on `vow build --no-verify` invocations; it is bypassed on the default `vow build` path. `--no-cache` additionally disables the cache for `--no-verify` builds.

### `vow verify`

Verify contracts only — no executable output. Emits the same JSON format as `vow build` but `executable` is always `null`.

```
vow verify [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `--no-cache`      | (off)       | Disable verification result caching        |
| `--max-k-step <N>` | `50`       | ESBMC incremental BMC max iterations       |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver; auto selects per-function via heuristic |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 |
| `--timeout <N>` | `300` (or `30` when `--encoding` is `auto`) | ESBMC per-function timeout in seconds. Under `--encoding auto`, a 30s default is applied so the BV-timeout fallback to `--encoding ir --solver z3` can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit `--timeout` overrides both. `--timeout 0` is honoured as an immediate watchdog kill |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent ESBMC verification jobs |

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
| `--max-k-step <N>` | `50`       | ESBMC incremental BMC max iterations       |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver (with --verify)           |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode (with --verify); ir requires z3 |
| `--verify-jobs <N>` | `num_cpus/2` | Accepted for CLI parity with build/verify/test; currently a no-op (the contracts verifier is serial) |

When `--verify` is requested but ESBMC is not installed, the command still emits the full contracts-result JSON schema with every entry's `status` set to `error` and exits with code 1 (fail-closed). Install ESBMC, or omit `--verify`, to obtain proven/failed/unknown statuses.

### `vow skill`

Generate or install the Claude Code skill document for the current compiler version. The skill is embedded in the compiler binary, ensuring the documentation always matches the installed toolchain.

```
vow skill              # print skill document to stdout (default: print)
vow skill print        # print concise Claude Code SKILL.md entrypoint
vow skill print --bundle  # print self-contained bundle for raw API harnesses
vow skill install      # prompt for local or global install target
vow skill install --local   # install to ./.claude/skills/vow/
vow skill install --global  # install to $HOME/.claude/skills/vow/ on Linux
```

`print` writes the concise installed `SKILL.md` entrypoint (with YAML frontmatter) to stdout. `print --bundle` writes a complete self-contained skill document to stdout for non–Claude Code harnesses that cannot load supporting files.

`install` writes `SKILL.md` plus supporting files under `reference/`, `examples/`, and `schemas/`. Claude Code discovers the skill from the `.claude/skills/` directory and uses the frontmatter description/`when_to_use` metadata to load it for `.vow` file work as well as creation and verification-debugging prompts before a `.vow` file exists.

When no scope flag is provided, `install` prompts on stderr for local (`./.claude`) or global (`$HOME/.claude`) installation. Scripts and agents should pass `--local` or `--global` explicitly. `--local` requires the current directory to contain both `.git` and `.claude/`; otherwise it exits with an error and writes nothing. `--global` installs under `$HOME/.claude/skills/vow/` and fails if `$HOME` is unset or empty.

**Auto-install on build.** The first time `vow build` (or the legacy `vow <source.vow>` form) runs in a directory that already contains a `.claude/` subtree but no `.claude/skills/vow/SKILL.md`, the compiler installs the skill silently. This bootstraps Claude Code projects without requiring an explicit `vow skill install`. Unlike explicit `--local`, auto-install only requires `.claude/`; it does not require the directory to be a git checkout. Auto-install is skipped when `.claude/` does not exist (so it never pollutes non–Claude Code projects) and when the skill file is already present (so user edits are never overwritten). Auto-install never fails the build.

### `vow test`

Discover, compile, run, and report on Vow test files. Tests are normal `.vow` programs with `main() -> i32` — no test-specific syntax.

```
vow test [OPTIONS] [<path>]
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `<path>`          | `.`         | Directory to scan or single `.vow` file    |
| `--verify`        | (off)       | Run ESBMC verification on test files       |
| `--filter <pat>`  | (none)      | Only run tests whose file stem contains pat |
| `--module-root <path>` | (auto)  | Resolve `use` declarations against `<path>`. Defaults to the scan path when it's a directory, otherwise the entry file's parent directory. |
| `--mode debug`    | (default)   | Insert runtime vow checks                 |
| `--mode release`  | `debug`     | Omit all vow checks for performance       |
| `--timeout <ms>`  | `30000`     | Per-test execution timeout in milliseconds |
| `--max-k-step <N>` | `50`       | ESBMC incremental BMC max iterations (with --verify) |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent ESBMC verification jobs (with --verify) |

Test discovery: files matching `test_*.vow` or `*_test.vow` under the given directory **and its subdirectories**, sorted alphabetically. Each test must contain `main() -> i32` returning 0 on success.

**Module resolution for directory scans.** When `<path>` is a directory, every discovered test resolves its `use` declarations against `<path>` rather than the test file's own parent directory. This lets internal-unit tests live in a subdirectory like `compiler/tests/test_region.vow` and still `use region;` to import the module under test (which lives at `compiler/region.vow`). Single-file invocations (`vow test path/to/test_foo.vow`) keep the default behaviour of resolving `use` against the file's parent directory.

**Test Output JSON:**

```json
{
  "status": "TestsPassed",
  "total": 3,
  "passed": 3,
  "failed": 0,
  "skipped": 0,
  "tests": [
    {
      "file": "compiler/test_arith.vow",
      "name": "test_arith",
      "status": "passed",
      "exit_code": 0,
      "stdout": "7",
      "stderr": "",
      "duration_ms": 72,
      "diagnostics": [],
      "counterexamples": []
    }
  ],
  "contract_density": {
    "functions_total": 1,
    "functions_with_vows": 0,
    "density_pct": 0.0
  }
}
```

| Status Field   | Meaning                                           |
|----------------|---------------------------------------------------|
| `TestsPassed`  | All tests passed                                  |
| `TestsFailed`  | One or more tests failed                          |

Per-test status: `passed`, `failed`, `timeout`, `compile_error`, `verify_failed`, `skipped`.

### `vow decl`

Emit declaration file output only.

```
vow decl [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `-o, --output`    | `<source>.vow.d` | Output declaration file path          |

### `vow mutants` (self-hosted only)

Run mutation testing on a Vow source tree. Implemented in the self-hosted compiler only; the Rust bootstrap compiler emits an error pointing the user to `build/vowc`. See `docs/mutants.md` for full details on output schema, mutation kinds, skip-list, and known limitations.

```
vowc mutants version
vowc mutants list  [--root DIR] [--shard X/Y]
vowc mutants run   [--root DIR] [--shard X/Y]
                   [--tier1-cmd 'cmd'] [--tier2-cmd 'cmd']
                   [--tier1-timeout-secs N] [--tier2-timeout-secs N]
                   [--tier2-budget-secs N]
                   [--workdir DIR] [--output-dir DIR] [--force-unlock]
```

| Flag | Default | Notes |
|---|---|---|
| `--root` | `compiler` | Directory whose `*.vow` files are mutated. `test_*.vow` files are excluded. |
| `--shard X/Y` | `0/1` | Round-robin split of the deterministic mutant ID space. Mutant `id` is selected iff `id % Y == X`. |
| `--tier1-cmd` | `scripts/bootstrap.sh --skip-cargo` | Fast oracle. Anything but exit 0 = caught at Tier 1. |
| `--tier2-cmd` | `scripts/full_test.sh` | Full oracle. Only run on Tier-1 survivors. |
| `--tier1-timeout-secs` | `180` | Per-mutant Tier-1 wall-clock cap. |
| `--tier2-timeout-secs` | `3600` | Per-mutant Tier-2 wall-clock cap. |
| `--tier2-budget-secs` | `7200` | Per-shard total Tier-2 budget. Once exhausted, surviving Tier-1 mutants are emitted with `status:"unrun"`. |
| `--workdir` | `/tmp/vow-mutants-<ms>` | Path of the throwaway `git worktree` used for all mutations. |
| `--output-dir` | `mutants.out` | Directory for `mutants.json`, `outcomes.json`, status text files, `diff/`, `logs/`. |
| `--force-unlock` | off | Remove a stale `output_dir/.lock` before starting. |

Output schemas: see `docs/spec/schemas/mutants-result.schema.json`.

### `vow --help`

`vow --help` is agent-first. It emits versioned JSON capability data for the tool, command set,
language surface, result schemas, and implementation status. `--help --human` exists only as a
legacy compatibility mode and is not the canonical interface.

```
vow --help               # versioned JSON tool-help protocol
vow --help --human       # legacy compatibility text
vow build --help         # same JSON (works on all subcommands)
vow verify --help --human  # same legacy text (works on all subcommands)
```

## Exit Codes

| Code | Meaning                                                                            |
|------|------------------------------------------------------------------------------------|
| `0`  | Success (`Verified` or `Unverified`)                                               |
| `1`  | Failure (`CompileFailed`, `VerifyFailed`, or `Skipped`)                            |

`vow build` and `vow verify` both fail closed on `Skipped`: if ESBMC was asked to verify a vowed
function but the verifier could not model the function body, the contract was not statically
proved, so the run exits non-zero. Use `--no-verify` if you genuinely want to skip verification —
that path produces `Unverified` (exit 0).

## Build Output JSON

`vow build` and `vow verify` emit a single JSON object to stdout. Schema: [`schemas/build-result.schema.json`](schemas/build-result.schema.json).

**Note:** `--dump-ir` suppresses JSON output — only IR text is printed.

### Status Values

| Status          | Meaning                                     |
|-----------------|---------------------------------------------|
| `Verified`      | Compiled + every vowed function's contract was statically proved by ESBMC. |
| `Unverified`    | Compiled but ESBMC was not invoked (e.g. `--no-verify`, `--dump-ir`). Exit 0. |
| `Skipped`       | ESBMC was invoked but at least one vowed function could not be modelled (e.g. body uses `RegionAlloc`, `FieldSet`, `Linear*`, `Load`/`Store`, `RemF*`, or has effects). Each such function appears as a `VerificationSkipped` *Warning* in `diagnostics[]`. Their contracts are runtime-checked under `--mode debug` but were not statically proved; the run fails closed with exit 1. |
| `CompileFailed` | Parse error, type error, module load error, or link failure |
| `VerifyFailed`  | ESBMC produced a non-Verified outcome: a counterexample, timeout, `VERIFICATION UNKNOWN` (`verify_status: "unknown"`), tool error, or the tool was not found. Inspect `counterexamples[]` (definitive failures) and `verify_status`/`verify_message` (soft failures) to distinguish. |

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
| `verify_status`    | string              | On backend failure | `"timeout"`, `"unknown"`, `"error"`, or `"tool_not_found"` |
| `verify_message`   | string              | On backend failure | ESBMC/backend error detail                |

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
      "status": "not_verified",
      "quality": "substantive"
    }
  ],
  "summary": { "total": 1, "proven": 0, "failed": 0, "timeout": 0, "error": 0, "not_verified": 1, "skipped": 0, "quality": { "weak": 0, "tautological": 0, "substantive": 1 } }
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
      "status": "proven",
      "quality": "substantive"
    }
  ],
  "summary": { "total": 1, "proven": 1, "failed": 0, "timeout": 0, "error": 0, "not_verified": 0, "skipped": 0, "quality": { "weak": 0, "tautological": 0, "substantive": 1 } }
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
| `status`      | string  | `"proven"`, `"proven-ir"`, `"failed"`, `"unknown"`, `"timeout"`, `"error"`, `"not_verified"`, or `"skipped"` |
| `quality`     | string  | Static clause-shape classification (no ESBMC): `"weak"`, `"tautological"`, or `"substantive"` |

### Status Values

| Status          | Meaning                                              |
|-----------------|------------------------------------------------------|
| `not_verified`  | Verification not requested (no `--verify` flag)      |
| `proven`        | ESBMC proved this contract holds for all inputs (bit-vector encoding, overflow modeled) |
| `proven-ir`     | ESBMC proved this contract under integer-arithmetic encoding after BV timed out; overflow is not modeled by IR, but the BV caller preconditions still guard against it |
| `failed`        | ESBMC found a counterexample violating this contract |
| `unknown`       | ESBMC could not conclude for this contract — either `VERIFICATION UNKNOWN` was reported for the containing function (k-induction's forward condition unable to prove or falsify), or another contract in the same function failed and this one was not individually checked |
| `timeout`       | ESBMC timed out on the containing function (BV and — when applicable — IR fallback both timed out) |
| `error`         | ESBMC error or tool not found                        |
| `skipped`       | The containing function's body uses opcodes the verifier cannot model (e.g. `RegionAlloc` from struct construction). Contract is documentary; runtime checks still apply under `--mode debug`. Surfaces as a `VerificationSkipped` Warning in the build JSON's `diagnostics[]` and lifts the overall build/verify status to `Skipped` (fail-closed, exit 1) — use `--no-verify` if you want a non-failing path that does not invoke ESBMC at all. |

### Quality Values

`quality` is a static classification of each clause's *shape*, computed without ESBMC and independent of `status`. It surfaces the "proven but trivial" problem: a `weak` contract can be `proven` while constraining almost nothing. See `docs/spec/contracts-methodology.md` for the full taxonomy.

| Quality        | Meaning                                                                                      |
|----------------|----------------------------------------------------------------------------------------------|
| `weak`         | An `ensures` that only bounds `result` by an integer literal (e.g. `result >= 0`). Satisfied by almost any implementation. |
| `tautological` | A constant clause that references no program value (e.g. `true`, `0 >= 0`). Constrains nothing. |
| `substantive`  | Everything else — equality, relational, inverse/round-trip, dispatch-totality, or function-call shapes. The classifier is conservative: anything not provably weak/tautological is reported `substantive`. |

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

## Runtime Error JSON (stderr, debug/sanitize mode)

When a compiled program runs in debug mode (`--mode debug`) or sanitize mode (`--mode sanitize`) and violates a vow at runtime, it emits JSON to stderr before aborting.

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

### UseAfterFree (sanitize mode only)

```json
{"error":"UseAfterFree","op":"push","vec":"0x55a1b2c3d4e0"}
```

Emitted when a Vec operation is attempted on a Vec that has already been freed.

### DoubleFree (sanitize mode only)

```json
{"error":"DoubleFree","vec":"0x55a1b2c3d4e0"}
```

Emitted when a Vec is freed twice.

### StaleIndex (sanitize mode only)

```json
{"error":"StaleIndex","index":5,"expected_gen":3,"actual_gen":7,"vec":"0x55a1b2c3d4e0"}
```

Emitted when `__vow_sanitize_check_generation` detects that a Vec slot's generation counter does not match the expected value, indicating the slot was overwritten since the index was recorded.

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
"#,
        ),
        (
            r#"reference/contracts.md"#,
            r#"# Contract Authoring and Verification

Vow uses ESBMC (bounded model checker) for static contract verification. This document covers contract patterns, verification behavior, and common pitfalls.

## Verification Pipeline

Codegen (Cranelift) and verification run in parallel:

```
Vow Source → Parse → Type Check → IR Lower ─┬─→ Cranelift → executable
                                              └─→ C Emit → ESBMC → proof / counterexample
```

Contract clauses become IR opcodes. The C emitter translates `requires` to `__ESBMC_assume()` (the verifier assumes preconditions hold) and `ensures`/`invariant` to `__ESBMC_assert()` (the verifier checks postconditions).

### ESBMC Configuration

- Verification strategy: **k-induction-parallel** (incremental BMC + k-induction proof)
- Incremental BMC with `--max-k-step` (default: **50**) — loops are verified incrementally up to N iterations
- Architecture: 64-bit
- Array bounds / pointer checks disabled (Vow handles these in its own model)

### Collection Models for Verification

ESBMC is a *bounded* model checker, so it models collection types as
fixed-size arrays and reasons about them up to a finite capacity. These
capacities are an internal property of the verifier, not of the language:

| Type              | Model Capacity | Supported Operations |
|-------------------|----------------|----------------------------------------------|
| `Vec<T>`          | 128            | `new`, `push`, `pop`, `len`, `get`, `set`    |
| `String`          | 256            | `from`, `len`, `push_byte`, `push_str`, `byte_at`, `matches_literal_at` |
| `HashMap<K, V>`   | 64             | `new`, `insert`, `get`, `contains_key`, `len`|
| `BTreeMap<K, V>`  | 64             | `new`, `insert`, `get`, `contains_key`, `len`|

**These bounds are not a language feature and are not user-tunable.** A `Vec`
in a Vow program grows dynamically on the heap with no fixed maximum; the
capacity above only describes how far the *bounded* model checker reasons. The
language and its contracts are deliberately decoupled from what any particular
prover can prove: replace ESBMC with a stronger (or unbounded) checker and the
same source, the same contracts, and the same CLI keep working — the only
difference is that proof covers more (or all) of the state space. For this
reason a `requires`/`ensures` clause must never encode a verifier bound (e.g.
`requires: v.len() <= 128`); see "Verification-Driven Bounds (Anti-Pattern)"
below and `docs/design/verifier-model-bounds.md`.

These models support the same operations as the runtime but with bounded
storage. String literals carry their concrete length and bytes in verification,
and `String::from` copies that model from its source value. The effective string
model capacity is automatically at least the longest static string literal, so
literal byte initializers always fit the model array. Operations whose bytes are
not statically known, such as `String::from_cstr`, produce a nondeterministic
length (0 to max-1). `string_matches_literal_at` is modeled against the
literal's concrete bytes and byte length; the third argument must be a string
literal so the verifier never has to infer static text from a dynamic `String`.

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

Use range bounds only when they reflect genuine semantic constraints (e.g., overflow prevention), not to appease the verifier:

```vow
fn safe_add(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: a <= 4611686018427387903,
    requires: b <= 4611686018427387903,
    ensures: result >= a,
    ensures: result >= b
} {
    a + b
}
```

The bounds here prevent `a + b` from overflowing `i64` — a legitimate semantic concern, not a verifier limitation.

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
    requires: a <= 4611686018427387903,
    requires: b <= 4611686018427387903,
    ensures: result >= a,
    ensures: result >= b
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

Without a bound on loop iterations, ESBMC may timeout (default max-k-step is 50):

```vow
fn fill(n: i64) -> Vec<i64> vow {
    requires: n >= 0,
    ensures: result.len() == n
} { ... }
```

ESBMC will only verify this for small `n` values. **Do not** add `requires: n <= 8` to the contract — that would distort the semantic specification. The contract is correct as-is; ESBMC's bounded verification provides partial assurance.

### Verification-Driven Bounds (Anti-Pattern)

**Never** add artificial bounds to contracts solely to help ESBMC verify them:

```vow
// WRONG: bounds exist only to appease the verifier
fn gcd(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: a + b > 0,
    requires: a <= 15,   // <-- verifier artifact, not semantic
    requires: b <= 15,   // <-- verifier artifact, not semantic
    ensures: result > 0
} { ... }
```

```vow
// CORRECT: only genuine semantic constraints
fn gcd(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: a + b > 0,
    ensures: result > 0
} { ... }
```

Contracts express what is mathematically required for correctness. ESBMC verifies within its capabilities (bounded loops, bounded arithmetic, bounded collection models) — if it cannot fully prove a correct contract, that is acceptable. Partial verification is better than a distorted specification. The same rule is why the verifier's collection model capacities (see "Collection Models for Verification") are internal defaults rather than CLI flags or contract clauses: a bound that belongs to the prover must never leak into the language.

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
"#,
        ),
        (
            r#"reference/contracts-methodology.md"#,
            r#"# Contract Methodology: What to Verify

This document answers a question that `contracts.md` does not: given a function,
**which** properties are worth proving, how do you tell a strong contract from a
hollow one, and how do you express the strong shapes within ESBMC's reach.

`contracts.md` is the *reference* (syntax, blame, the verification pipeline,
anti-patterns). This is the *methodology* (judgement). Read that first.

## The core principle: strength, not volume

A proven contract is worth nothing if it would also hold for an incorrect
implementation. The number of contracts a codebase proves is not a quality
signal — the *discriminating power* of each contract is.

This is measurable. Polikarpova, Furia, Pei, Wei, and Meyer (the originator of
Design by Contract), *"What Good Are Strong Specifications?"* (ICSE 2013), found
that testing implementations against **strong** specifications — comprehensive
functional pre/post/invariants — detected roughly **twice as many bugs** as
testing against standard/weak contracts. Their conclusion: *"the quality of
specifications limits the value of verification."*

A concrete Vow example of the trap. Many tag constants in the self-hosted
compiler carry this contract:

```vow
fn EFF_IO() -> i64 vow { ensures: result > 0 } { 1 }
```

ESBMC proves `1 > 0` in milliseconds. But `ensures: result > 0` also holds for
`{ 2 }`, `{ 99 }`, and every other positive constant — it does not pin the value
this function is *supposed* to return. It is a real postcondition, but a **weak**
one: it constrains the output to a half-line, not to a point. Proving 354 of
these does not make the compiler more correct; it makes the verification report
longer.

The fix is not "delete contracts" — it is "make each contract say something only
the correct implementation satisfies."

## A taxonomy of contract shapes

Each shape below lists its *intent*, *when it applies*, a *real Vow example*, and
*strength notes*. The expressibility/verifiability status of every shape is
collected in the matrix at the end.

### 1. Domain precondition (range / validity bound)

**Intent:** restrict the inputs the function promises to handle correctly. Blame
falls on the caller (`requires`).

**When:** the function is only correct on a subset of its parameter types — a byte
in `0..=255`, a non-zero divisor, an in-bounds index.

```vow
fn write_u8(out: Vec<i64>, v: i64) vow {
    requires: v >= 0,
    requires: v <= 255
} { out.push(v); }
```

**Strength:** a precondition is strong when it is the *true* domain of the
function — no wider (which would admit miscompilation) and no narrower (a
verifier-driven bound like `requires: n <= 8`, forbidden by `contracts.md`).
A bounds-check precondition such as `requires: i >= 0, requires: i < v.len()` is
the standard guard for every indexing operation.

### 2. Output-range postcondition (the weak default — use sparingly)

**Intent:** constrain the result to a range.

**When:** the range *is* the full functional spec — e.g. a function whose only
guarantee is non-negativity. This is rare. Most uses are the weak trap above.

```vow
fn item_kind(v: i64) -> i64 vow {
    requires: v >= 0,
    ensures: result >= 0          // weak: any non-negative value satisfies this
} { v / 4294967296 }
```

**Strength:** weak by default. Reach for shape 3, 4, or 5 instead whenever the
function actually computes a *specific* value. If you find yourself writing
`ensures: result >= 0` on a function that returns a computed quantity, ask what
the result *equals* or *inverts*, and assert that.

### 3. Exact functional postcondition (equality)

**Intent:** pin the result to the value the function is defined to produce.

**When:** the output is a closed-form function of the inputs (arithmetic,
bit-packing, encodings).

```vow
fn region_pack(kind: i64, val: i64) -> i64 vow {
    requires: kind >= 0,
    requires: kind <= 3,
    requires: val >= 0,
    requires: val <= 4294967295,
    ensures: result == val * 4 + kind     // exact: only the right answer passes
} { val * 4 + kind }
```

**Strength:** strong — a wrong shift or offset is caught immediately. Note the
preconditions are not verifier appeasement: they bound `val` and `kind` so the
packed result cannot overflow `i64` (a genuine semantic constraint). Contrast
`region_pack` (exact) with `item_pack`/`item_kind` (shape 2, only `>= 0`): the
same bit-packing pattern, one specified strongly and one weakly.

### 4. Round-trip / inverse

**Intent:** prove that an encode/decode (or pack/unpack, serialize/deserialize)
pair compose to the identity on the valid domain.

**When:** two functions are defined as inverses — `pack`/`unpack`,
`encode`/`decode`, `to_bytes`/`from_bytes`.

The strongest form asserts the inverse directly, calling the partner function in
the postcondition (pure-function calls are allowed in contracts):

```vow
fn region_kind(r: i64) -> i64 vow {
    requires: r >= 0,
    ensures: result == r - (r / 4) * 4,   // exact extractor
    ensures: result <= 3
} { r - (r / 4) * 4 }
```

A `region_pack` then `region_kind`/`region_val` round-trip recovers `(kind, val)`
exactly. **Strength:** very strong — round-trip is the property a serialization
layer must have, and it catches the entire class of "encoder and decoder drifted
apart" bugs that output-range contracts miss completely.

### 5. Dispatch totality (fail-closed decoders)

**Intent:** prove that a decoder/dispatcher maps **every** valid input to a
defined output and **never** silently falls through to a default.

**When:** a function switches over a tag/opcode/discriminator. This is the
single highest-value shape for Vow, because silent-fallback normalization
(mapping an unknown tag to a valid-looking default) is the bug class issue #81
was filed over.

The pattern has two halves — a validity precondition and an explicit error
sentinel for the unreachable tail:

```vow
fn is_valid_binop(op: i64) -> bool { op >= 0 && op <= 22 }

fn binop_opcode(op: i64, operand_ty: i64) -> i64 vow {
    requires: is_valid_binop(op)
} {
    if op == BINOP_ADD() { return ...; }
    // ... one arm per valid op ...
    -1                                    // unreachable under the precondition
}
```

**Strength — and a live hardening gap.** The precondition pins the domain, but
this contract does **not yet prove totality**: nothing asserts the function never
returns the `-1` sentinel. The strong form adds a postcondition that rules out
the fallthrough:

```vow
fn binop_opcode(op: i64, operand_ty: i64) -> i64 vow {
    requires: is_valid_binop(op),
    ensures: result != -1                 // proves every valid op is handled
} { ... }
```

With `ensures: result != -1`, ESBMC must show that on every `op` in `0..=22` some
arm returns before the sentinel — i.e. the dispatch is exhaustive. If an agent
later adds opcode 23 to `is_valid_binop` but forgets the matching arm,
verification fails instead of miscompiling. This is the contract that converts a
silent fallback into a caught error.

> Vow has no surface quantifier (`forall i in 0..n`) today, so "covers all valid
> inputs" is expressed as `requires` (pin the finite domain) + a postcondition
> that excludes the failure value, letting ESBMC enumerate the finite branch
> structure. Bounded quantifiers are tracked as a roadmap item (#467/#470).

### 6. Relational / cross-function (uniqueness, agreement)

**Intent:** state a property that spans more than one function or more than one
argument.

**When:** tags in a family must be distinct; two collections must have equal
length; a result must relate two inputs.

The argument-relational form is directly expressible:

```vow
fn build_pairs(ids: Vec<i64>, names: Vec<i64>) -> Vec<Pair> vow {
    requires: ids.len() == names.len()
} { ... }
```

The *cross-function uniqueness* form — "`tok_kw_fn() != tok_kw_let()` for every
pair in the family" — is expressible only as O(n²) pairwise inequalities, which
does not scale to dozens of tag constants. **The better fix is structural, not
contractual:** encode each family as a base+offset range or a generated table so
uniqueness is a property of the *encoding* rather than something every function
must restate. Treat a wall of zero-argument tag constants as an API smell that
shape-6 contracts cannot economically repair.

### 7. Loop invariant / frame

Covered in `contracts.md` (counter bounds, search-range invariants, the
inductiveness requirement). The methodology point: an invariant is strong when it
is the property the loop *maintains toward its postcondition*, not merely
`i >= 0`. See `contracts.md` §Loop Invariants and the worked CEGIS cycle in
`examples.md`.

## Hollow contracts: three failure modes to detect

A contract can pass verification while proving nothing. There are three distinct
ways this happens; a contract-quality tool should distinguish them.

### Weakness

The clause is satisfiable and true, but so loose that an incorrect
implementation also satisfies it (`ensures: result >= 0` on a computed value).
This is the 354-contract problem. **Detection (mutation-based):** mutate the
implementation — flip a constant, swap an operator — and re-verify. If the
contract still proves against the *mutated* body, it does not constrain that
behavior and is too weak. Vow already has the machinery for this in
`vowc mutants`; a weak contract is one whose function's mutants survive.

### Tautology

The clause is true independent of the program — `ensures: true`,
`ensures: result == result`, `ensures: x >= 0 || x < 0`. **Detection:** the clause
is valid (provable) with the function body removed; a cheap check folds constant
clauses and flags any clause with no dependence on parameters or `result`.

### Vacuity (antecedent failure)

The clause is proved only because its **preconditions are unsatisfiable**, so the
path it guards is dead and the postcondition never has to hold. Because Vow
lowers `requires` to `__ESBMC_assume`, a contradictory or over-strong precondition
makes *any* `ensures` provable — an assume-false / dead-path proof.

This is the classic vacuity of Beer, Ben-David, Eisner, and Rodeh, *"Efficient
Detection of Vacuity in Temporal Model Checking"* (Formal Methods in System
Design, 2001): a subformula is vacuous when replacing it changes nothing about
the result. Their industrial data is the reason to take it seriously — across
years of hardware verification at IBM, ~20% of formulas were trivially valid on
first runs, and trivial validity *always* indicated a real defect in the design,
spec, or environment.

**Detection (the `false` re-check):** re-verify each obligation with its `ensures`
replaced by `ensures: false`. If `assert(false)` still passes, the path is
unreachable under the preconditions — the original proof was vacuous. A non-vacuous
obligation must *fail* this check.

**Interesting witnesses.** Beer et al. also propose the dual of a counterexample:
for a proof that holds, emit a non-trivial *witness* — concrete inputs that
exercise the property for a substantive reason — so the author can confirm the
proof is not hollow. Vow's structured output is well-suited to carrying a witness
alongside each `Verified` result.

## When to write contracts

### Builtins and `extern` blocks

Runtime functions (`Vec.push`, `String.from`, `HashMap.insert`) are implemented in
Rust/C and cannot be verified by ESBMC. Their behavior enters verification through
the `vow` contract on the `extern "C"` block, which becomes an **assumed**
(`__ESBMC_assume`) surface for callers. Because these contracts are *assumed, not
checked*, they are the most dangerous place for an error: a wrong `ensures` on an
extern block silently weakens every proof that depends on it. Extern contracts
must be reviewed as assumptions, audited against the runtime implementation, and
kept minimal. (Omitting the block is a `MissingContract` error — see `errors.md`.)

### Library functions (written in Vow)

Public Vow functions are fully within verification reach. Give each one its true
domain precondition and the strongest postcondition shape that applies (3–6, not
2). Add contracts when the function's contract is *known*, which is usually at
definition time for pure utilities and after the signature stabilizes for APIs.

### Application code (including agent-generated)

Vow's target author is an AI agent, and the failure mode to design against is
*volume over substance* — an agent emitting many `ensures: result >= 0` clauses
because the prompt said "add contracts." Skill guidance should push the opposite:
for each function, identify which shape applies (equality, round-trip, dispatch
totality, relational) and write that one; prefer one discriminating contract to
five weak ones. The Specification Pattern System of Dwyer, Avrunin, and Corbett
(ICSE 1999) — a survey-validated catalog built specifically to turn imprecise
intent into precise specifications — is the model for guiding an author from "this
should be valid" to a postcondition that says what *valid* means.

## Expressibility and verifiability matrix

Whether a shape is usable depends on five independent axes, not just "can the
syntax say it":

- **expressible** — surface Vow has the syntax
- **typechecked** — the checker validates the clause to `bool` (added in #81 Phase 0)
- **lowerable** — the lowerer emits IR for the clause
- **modelable** — the C emitter / ESBMC model supports the operations used
  (pure, non-effectful helpers only; see `is_modelable` in the C emitter)
- **backend** — ESBMC actually discharges it within bounds

| Shape | expressible | typechecked | lowerable | modelable | backend |
|-------|:-----------:|:-----------:|:---------:|:---------:|:-------:|
| 1. Domain precondition | ✓ | ✓ | ✓ | ✓ | ✓ |
| 2. Output-range postcond. | ✓ | ✓ | ✓ | ✓ | ✓ (but weak) |
| 3. Exact equality | ✓ | ✓ | ✓ | ✓ | ✓ within overflow bounds |
| 4. Round-trip / inverse | ✓ | ✓ | ✓ | ✓ if partner is pure & modelable | ✓ for arithmetic |
| 5. Dispatch totality | ✓ | ✓ | ✓ | ✓ (pure dispatch) | ✓ over finite domain |
| 6a. Argument-relational | ✓ | ✓ | ✓ | ✓ | ✓ |
| 6b. Cross-fn uniqueness | ✓ (O(n²)) | ✓ | ✓ | ✓ | ✓ but unscalable → prefer structural encoding |
| 7. Loop invariant | ✓ | ✓ | ✓ | ✓ | partial (bounded / k-induction) |
| Bounded quantifier (`forall i in 0..n`) | ✗ (no surface syntax) | — | — | — | — (roadmap #467/#470) |

Contract expressions must be **pure** — they cannot call effectful functions
(`grammar.md` §Contract Purity). A property that needs an effectful helper is
blocked at the *modelable* axis, not the expressible one; classify such gaps as
model limitations, not contract-language limitations.

## Tooling (planned)

`vow contracts --verify` lists contracts and verifies them per function today. The
quality work tracked in #81 / roadmap WS-3.2 extends it to a **per-obligation**
check that emits a quality signal per clause — flagging tautologies (cheap, no
ESBMC), vacuous obligations (the `false` re-check), and, via the `vowc mutants`
harness, weak obligations whose function's mutants survive. Until that ships, the
three detections above are checks an author can run by hand.

## References

- N. Polikarpova, C. A. Furia, Y. Pei, Y. Wei, B. Meyer. *What Good Are Strong
  Specifications?* ICSE 2013. https://arxiv.org/abs/1208.3337
- I. Beer, S. Ben-David, C. Eisner, Y. Rodeh. *Efficient Detection of Vacuity in
  Temporal Model Checking.* Formal Methods in System Design 18:141–163, 2001.
- M. Dwyer, G. Avrunin, J. Corbett. *Property Specification Patterns for
  Finite-State Verification.* ICSE 1999.
- B. Meyer. *Object-Oriented Software Construction* (Design by Contract).
"#,
        ),
        (
            r#"reference/errors.md"#,
            r#"# Vow Error Catalog

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

### StaticLiteralRequired

**Phase:** Type Checker
**Meaning:** A compiler intrinsic requires a string literal operand so it can be lowered without allocation.

```vow
fn f(s: String, key: String) -> i64 {
    string_matches_literal_at(s, 0, key)
}
```

**Output:** `string_matches_literal_at requires a string literal as its third argument`

**Fix:** Pass a literal directly, for example `string_matches_literal_at(s, 0, "name")`.

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
**Meaning:** A value of a `linear struct` type is used in a way that is immediately invalid before region inference runs, such as consuming it twice, consuming it inside a loop that may execute more than once, or consuming it after only some control-flow paths already consumed it.

```vow
linear struct Handle { fd: i64 }

fn f(h: Handle) -> Handle {
    let h2: Handle = h;
    let h3: Handle = h;  // h was already consumed
    h2
}
```

**Fix:** Restructure ownership so each path uses a consumed linear value at most once. Obligations that are simply left live at scope exit are reported later as `RegionLinear`.

### RegionLinear

**Phase:** Region Inference
**Meaning:** A `linear struct` value can remain live when its owning region closes. Returning the value transfers the linear obligation to the caller; consuming it before the close satisfies the obligation.

```vow
linear struct Handle { fd: i64 }

fn f() -> i64 {
    let h: Handle = Handle { fd: 1 };
    0
}
```

**Fix:** Consume the value before the region closes, or return it so the caller receives the obligation.

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

### BTreeMapKeyTypeMustBeI64

**Phase:** Type Checker
**Meaning:** A `BTreeMap<K, V>` was instantiated with `K` not equal to `i64`. Phase 1 of the BTreeMap stdlib only supports `i64` keys; the runtime helpers and ESBMC C model are hard-coded to i64.

```vow
fn f() -> () {
    let m: BTreeMap<bool, i64> = BTreeMap::new();
    m.insert(true, 1);
}
```

**Output:** `BTreeMap key type must be i64; found 'bool'`

**Fix:** Use `BTreeMap<i64, V>`. If you need string or struct keys, hash or intern them to `i64` at the call site and keep a side-table for the originals.

### BTreeMapValueMustBeNonLinear

**Phase:** Type Checker
**Meaning:** A `BTreeMap<K, V>` was instantiated with a `V` that is or transitively contains a `linear struct`. Non-linear containers like `BTreeMap`, `Vec`, and `HashMap` cannot hold linear values because their internal shift/copy operations are bitwise and would silently duplicate the linear ownership obligation.

```vow
linear struct Token { id: i64 }
fn f() -> () {
    let m: BTreeMap<i64, Token> = BTreeMap::new();
}
```

**Output:** `BTreeMap value type must be non-linear; found 'Token'`

**Fix:** Either drop the `linear` qualifier on the struct, or keep handles in a `Vec<i64>` indirection and consume the linear values via direct function calls outside the map.

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

### ContractTypeMismatch

**Phase:** Type Checker
**Meaning:** A `requires`, `ensures`, or `invariant` clause expression does not have type `bool`.

```vow
fn add(a: i64, b: i64) -> i64 vow {
    requires: a + b
} {
    a + b
}
```

**Output:** `` `requires` clause has type `i64` but must be `bool` ``

**Fix:** Ensure every contract clause is a boolean expression (comparison, logical operator, or a call to a predicate function returning `bool`).

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

### RegionConflict

**Phase:** Region Inference (arena-per-scope, Phase 3)
**Meaning:** A heap-typed value's required lifetime cannot be satisfied by the regions the surrounding code provides. This fires when an interprocedural store-effect constraint is unsatisfiable against the **inferred** region — that is, the value's `region(I) = LUB(must_outlive(I))` resolves to a concrete block strictly narrower than the target container's region.

> **Coverage note (as of issue #314):** the check is now semantic, consulting
> the inferred region populated by §4.1 step 3's LUB pass rather than the
> raw IR opcode. A fresh allocation routed through a callee's store-effect
> chain into a parameter container has its inferred region widened to
> `Caller(HiddenRegionIdx(N))` by §4.1 step 2's must-outlive marker
> propagation, where `N` is the precise slot index implied by the
> destination (issue #317 slot-aware inference). Such single-slot routings
> satisfy the constraint and are accepted. Allocations whose caller-region
> markers require more than one hidden caller-arena slot resolve to
> `Caller(HiddenRegionIdx::AMBIGUOUS)` and are rejected when the directly
> fresh heap value is stored into a parameter-rooted target; allocations
> whose inferred region is a strictly narrower block also fire
> `RegionConflict`.

```vow
fn store_into(out: Vec<String>, prefix: String) [io] {
    let s: String = String::from(prefix);
    s.push_str(String::from(" world"));
    out.push(s);  // s is allocated in this function's scope but escapes into out's region
}
```

**Fix:** Move the allocation to a wider scope, or copy the value into the target region (e.g., `String::from(s)` into the outer arena). For routings that compile cleanly but you'd like to know about (root-region placement), see `RegionRootEscape` below. See `docs/design/arena_memory.md` §4.4 for the full rejection vs. visibility distinction.

### RegionRootEscape

**Phase:** Region Inference (arena-per-scope, Phase 3)
**Severity:** Note (informational — does not fail the build)
**Meaning:** A heap allocation's inferred region is `Caller`, and the surrounding function publishes a `FreshInCaller` return summary or store effect — so the allocation may flow up the caller chain to `main` and ultimately land in the root region (`__vow_root_arena`, never freed). This is a memory-cost decision the compiler surfaces visibly per `docs/design/arena_memory.md` §4.4: silent root-region placement caused growth-with-no-signal in earlier compiler versions, and the note restores that signal without conflating it with unsoundness (`RegionConflict`).

The note is conservative — it fires for any `Caller`-region allocation in a function that could route to a caller, even if the actual concrete chain in this program doesn't reach `main`. False positives are tolerated because the diagnostic is non-blocking.

```json
{
  "error_code": "RegionRootEscape",
  "severity": "note",
  "message": "allocation may live in the root region: routed via store-effect chain to a caller whose target_region ultimately resolves to root",
  "hints": [
    "if intentional (e.g. program-lifetime data), no action needed; if you want this allocation freed earlier, restructure so the value is returned rather than stored into a parameter container"
  ]
}
```

**Fix:** Often none — if the program is short-lived (a checker, a CLI tool) or the values are genuinely program-lifetime, the note is informational. To free the allocation earlier, restructure so the value is **returned** from the constructing function rather than stored into a parameter container; the canonical `FreshInCaller` return path (`fn make_X() -> X`) does not trigger the note for the returned value or any allocation installed as a field of the returned struct (e.g. `Item { name: String::from("hi") }`). The exemption applies only to the *currently-installed* field initializers — a field overwritten before the return (`x.f = A; x.f = B; return x`) does not suppress the dead allocation `A`, which fires the note as expected (per-block last-write dedup, issue #326).

### VerificationSkipped

**Phase:** Verification (Warning surfaced alongside `BuildStatus::Skipped`)
**Meaning:** The function carries a `vow {}` block but its body uses opcodes the verifier's C model cannot represent — most commonly `RegionAlloc` and `FieldSet` produced by struct construction, also `Load`/`Store`, `RemF*`, and the `Linear*` family. The function is skipped before any C is emitted or ESBMC is invoked. The contract becomes documentary: runtime checks still apply in `--mode debug`, but no static proof is attempted.

```json
{
  "error_code": "VerificationSkipped",
  "severity": "warning",
  "message": "skipped verification of `ir_inst_set_region`: function `ir_inst_set_region` is not modelable in the verifier (contains unsupported opcode `RegionAlloc`)",
  "hints": [
    "the contract is documentary; runtime checks still apply in --mode debug"
  ]
}
```

**Why the build fails closed.** Per `CLAUDE.md`'s "Contract Authoring" guidance, contracts express semantic correctness and must not be weakened to fit the verifier. When the verifier's bounded model checker cannot represent a function's body, the function is skipped with a structured warning instead of tripping the defense-in-depth `__ESBMC_assert(0, "vow:UNSUPPORTED_OP_VOW_ID")` that historically broke the bootstrap on every vowed struct-builder. But a skipped contract is still an unproved contract, so the build lifts its overall status to `Skipped` (exit 1). Use `--no-verify` if you explicitly want a non-failing path that does not invoke ESBMC at all (`Unverified`, exit 0).

**Fix:** Refactor the function so its body uses only modelable opcodes — typically by splitting allocation/initialisation away from the contract-bearing computation. Alternatively, run with `--no-verify` if the contract is intentionally documentary.

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

### RegionLiteralMutation

**When:** A `Vec`, `String`, or `HashMap` mutation is attempted on a literal-backed container — one whose descriptor carries the `VOW_CAP_RODATA` sentinel (backing lives in `.rodata` or was pinned to the root region). Calls that statically trace a mutating target to a literal are rejected during compilation with this code; a runtime fallback emits the JSON shape below if an unchecked mutation reaches a `VOW_CAP_RODATA` descriptor. See `docs/design/arena_memory.md` §6.1, §7.3.

```json
{"error":"RegionLiteralMutation","operation":"String::push_str","origin":"rodata"}
```

A plain-text hint follows on the next line (not a JSON field). The hint text is dispatched on the operation's type prefix:

```
hint: make an explicit mutable copy with String::from(value) before mutating  # for String::* operations
hint: construct a mutable Vec and copy entries before mutating                # for Vec::*    operations
hint: construct a mutable HashMap and copy entries before mutating  # for HashMap::* operations
```

The `operation` field identifies the source-level method that trapped (e.g., `Vec::push`, `Vec::pop`, `HashMap::insert`, `String::clear`). The `origin` field identifies the storage class of the immutable backing; today only `rodata` is emitted.

**Fix:** Obtain an explicit mutable copy before mutation: `String::from(value)`, or construct a fresh mutable container and copy the entries you need before mutating.

### StackOverflow

**When:** The native call stack is exhausted, typically due to unbounded recursion.

```json
{"error":"StackOverflow"}
```

In debug or sanitize mode, the diagnostic includes call depth and the function that was executing when the overflow occurred:

```json
{"error":"StackOverflow","depth":10693,"function":"recurse"}
```

The signal handler is installed in **all** build modes. The `depth` and `function` fields are only available in debug/sanitize mode where call-depth instrumentation is emitted.

**Fix:** Add a base case to recursive functions, or restructure the algorithm to use iteration instead of recursion.

### OutOfMemory

**When:** A runtime arena operation (`__vow_arena_open` or `__vow_arena_alloc`) failed because the underlying `malloc` returned null. Non-recoverable from within Vow (`docs/design/arena_memory.md` §3.3, §16).

```json
{"error":"OutOfMemory","operation":"arena_alloc"}
```

The `operation` field is `arena_open` for the initial chunk allocation or `arena_alloc` for a later fallback chunk allocation.

**Fix:** Reduce working-set size, raise the process memory limit, or run on a machine with more memory. This is not a Vow program error.

## Warnings

### LoweringWarning

**Phase:** IR Lowering
**Meaning:** The IR lowerer could not resolve a struct type tag or field name, defaulting to index 0. This usually indicates a missing type annotation on a `let` binding, causing the compiler to lose track of which struct type a pointer refers to.

**Fix:** Add an explicit type annotation: `let x: MyStruct = ...;` so the compiler can track struct type tags through the IR.
"#,
        ),
        (
            r#"examples/examples.md"#,
            r#"# Worked Examples

Verification workflow examples. The first three demonstrate Counterexample-Guided Inductive Synthesis (CEGIS) cycles: write spec, build, read JSON, diagnose, fix, verify. The fourth shows break-with-value in loop expressions. The fifth shows an EOF-safe interactive command loop using `stdin_read_line()`. The sixth shows bounded-memory streaming file input.

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
- `requires: n <= 8` keeps iterations tractable for verification
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

## 5. Command Loop — EOF-Safe `stdin_read_line`

### Goal

Write a line-oriented command interpreter that reads from stdin, dispatches commands, skips empty lines, and exits cleanly on EOF. This is the canonical pattern for CI-safe interactive programs.

### Step 1: Write the program

```vow
module CmdLoop

fn trim_newline(s: String) -> String {
    let n: i64 = s.len();
    if n == 0 { return s; }
    let last: i64 = s.byte_at(n - 1);
    if last == 10 {
        if n >= 2 {
            let prev: i64 = s.byte_at(n - 2);
            if prev == 13 {
                return s.substring(0, n - 2);
            }
        }
        return s.substring(0, n - 1);
    }
    s
}

fn skip_spaces(s: String, start: i64) -> i64 {
    let mut i: i64 = start;
    let n: i64 = s.len();
    while i < n {
        if s.byte_at(i) != 32 { return i; }
        i = i + 1;
    }
    i
}

fn main() -> i32 [read, io] {
    let mut line: String = stdin_read_line();
    while line.len() > 0 {
        let cmd: String = trim_newline(line);

        if cmd.len() > 0 {
            if cmd.eq(String::from("quit")) {
                return 0;
            }

            if cmd.eq(String::from("hello")) {
                print_str(String::from("Hello, world!\n"));
            } else {
                if cmd.len() >= 5 {
                    let prefix: String = cmd.substring(0, 5);
                    if prefix.eq(String::from("echo ")) {
                        let start: i64 = skip_spaces(cmd, 5);
                        let text: String = cmd.substring(start, cmd.len());
                        print_str(text);
                        print_str(String::from("\n"));
                    } else {
                        print_str(String::from("unknown: "));
                        print_str(cmd);
                        print_str(String::from("\n"));
                    }
                } else {
                    print_str(String::from("unknown: "));
                    print_str(cmd);
                    print_str(String::from("\n"));
                }
            }
        }

        line = stdin_read_line();
    }
    0
}
```

### Step 2: Build

```
$ vow build --no-verify examples/cmdloop.vow -o /tmp/cmdloop
```

```json
{"status":"Unverified","executable":"/tmp/cmdloop","diagnostics":[],"counterexamples":[]}
```

No contracts here — this example focuses on the I/O pattern, not verification.

### Step 3: Run with piped input

```
$ printf 'hello\necho Vow is great\n\nbogus\nquit\n' | /tmp/cmdloop
Hello, world!
Vow is great
unknown: bogus
```

The `quit` command causes an early `return 0`. Empty lines are silently skipped.

### Step 4: Run with EOF (no quit)

```
$ printf 'hello\necho test\n' | /tmp/cmdloop
Hello, world!
test
```

When stdin is exhausted, `stdin_read_line()` returns `""` (length 0), the `while` condition fails, and the program exits cleanly with code 0.

### Key points

- **EOF detection:** `stdin_read_line()` returns `""` at EOF. Check `.len() > 0` to exit the loop.
- **Newline stripping:** `stdin_read_line()` includes the trailing `\n` (or `\r\n`). Strip it with `byte_at` + `substring` before comparing commands.
- **Empty line handling:** After trimming, `cmd.len() == 0` means the line was blank — skip it.
- **Effects:** `stdin_read_line()` requires `[read]`; `print_str()` requires `[io]`. The `main` function declares both.
- **CI-safe:** No blocking reads, no prompts — the program processes whatever stdin provides and exits at EOF. Safe to run in pipelines and test harnesses.

## 6. Streaming File Input

`fs_read(path)` materializes the entire file as one `String`. Use `fs_open` plus `fs_read_line` for newline-delimited files that may be large.

```vow
module StreamingFile

fn main() -> i32 [read, io] {
    let argv: Vec<String> = args();
    if argv.len() < 2 {
        eprintln_str(String::from("usage: streaming_file <path>"));
        return 1;
    }

    let h: i64 = fs_open(argv[1]);
    if h < 0 {
        eprintln_str(String::from("could not open input"));
        return 1;
    }

    let mut lines: i64 = 0;
    let mut bytes: i64 = 0;
    let mut line: String = fs_read_line(h);
    while line.len() > 0 {
        lines = lines + 1;
        bytes = bytes + line.len();
        line = fs_read_line(h);
    }

    if fs_status(h) != 1 {
        fs_close(h);
        eprintln_str(String::from("read error"));
        return 1;
    }
    if fs_close(h) != 0 {
        eprintln_str(String::from("close error"));
        return 1;
    }

    print_i64(lines);
    print_str(String::from("\n"));
    print_i64(bytes);
    print_str(String::from("\n"));
    0
}
```

Key points:

- `fs_read_line(handle)` includes the trailing newline when present.
- Blank lines are returned as `"\n"`; EOF returns `""`.
- Check `fs_status(handle)` after `fs_read_line(handle)` returns `""`: `1` means EOF, `-1` means invalid handle or read error.
- Close each successful handle with `fs_close(handle)` and check for a non-zero close result.

## 7. BTreeMap basic usage

`BTreeMap<i64, V>` is the deterministic alternative to `HashMap` — sorted ascending by key, binary-search lookup. Use it when iteration order affects program output (codegen, serialization, or any reproducible build).

```vow
module BTreeMapExample

fn fetch(m: BTreeMap<i64, i64>) -> Option<i64> [io] {
    let r: Option<i64> = m.get(7);
    let v: i64 = r?;
    print_i64(v);
    print_str(String::from("\n"));
    Option::Some(v)
}

fn main() -> i32 [io] {
    let m: BTreeMap<i64, i64> = BTreeMap::new();
    m.insert(7, 42);
    let prev: Option<i64> = m.insert(7, 99);
    // prev is Some(42); the second insert overwrote the first.
    fetch(m);
    print_i64(m.len());
    0
}
```

Note that `.insert` returns `Option<V>` (the previous value, if any), and `.get` returns `Option<V>`. Use `?` to short-circuit on `None`. Phase 1 only supports `i64` keys; using any other key type raises `BTreeMapKeyTypeMustBeI64`.

### Why BTreeMap and not HashMap

`HashMap.insert` returns `()` and its iteration order is unspecified. For maps whose iteration is observable in the output binary, the byte-identical bootstrap requirement (`stage1 == stage2` sha256) demands deterministic order. `BTreeMap` provides it; `HashMap` does not.
"#,
        ),
        (
            r#"schemas/build-result.schema.json"#,
            r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/build-result.schema.json",
  "title": "BuildResult",
  "description": "JSON output from `vow build` on stdout",
  "type": "object",
  "required": ["status", "executable", "diagnostics", "counterexamples"],
  "properties": {
    "status": {
      "type": "string",
      "enum": ["Verified", "Unverified", "Skipped", "CompileFailed", "VerifyFailed"],
      "description": "Build outcome. `Skipped` means ESBMC was invoked but at least one vowed function could not be modelled; the build fails closed with exit 1 (distinct from `Unverified`, which means ESBMC was not invoked, e.g. `--no-verify`/`--dump-ir`, exit 0)."
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
      "enum": ["timeout", "unknown", "error", "tool_not_found"],
      "description": "Verification sub-status (present only when the verification backend did not produce a proof or counterexample)"
    },
    "verify_message": {
      "type": "string",
      "description": "Verification backend error detail (present only when verify_status is set)"
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
"#,
        ),
        (
            r#"schemas/contracts-result.schema.json"#,
            r#"{
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
        "required": ["vow_id", "function", "kind", "description", "blame", "source", "status", "quality"],
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
            "enum": ["proven", "proven-ir", "failed", "unknown", "timeout", "error", "not_verified", "skipped"],
            "description": "Verification status"
          },
          "quality": {
            "type": "string",
            "enum": ["weak", "tautological", "substantive"],
            "description": "Static, no-ESBMC classification of the clause shape: weak (an ensures that only bounds result by a constant), tautological (constant clause that says nothing), or substantive (equality/relational/inverse/call). See contracts-methodology.md"
          }
        },
        "additionalProperties": false
      }
    },
    "summary": {
      "type": "object",
      "required": ["total", "proven", "failed", "unknown", "timeout", "error", "not_verified", "skipped", "quality"],
      "properties": {
        "total": { "type": "integer" },
        "proven": { "type": "integer" },
        "failed": { "type": "integer" },
        "unknown": { "type": "integer" },
        "timeout": { "type": "integer" },
        "error": { "type": "integer" },
        "not_verified": { "type": "integer" },
        "skipped": { "type": "integer" },
        "quality": {
          "type": "object",
          "description": "Static contract-quality tallies independent of verification status",
          "required": ["weak", "tautological", "substantive"],
          "properties": {
            "weak": { "type": "integer" },
            "tautological": { "type": "integer" },
            "substantive": { "type": "integer" }
          },
          "additionalProperties": false
        }
      },
      "additionalProperties": false
    }
  },
  "additionalProperties": false
}
"#,
        ),
        (
            r#"schemas/counterexample.schema.json"#,
            r#"{
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
"#,
        ),
        (
            r#"schemas/diagnostic.schema.json"#,
            r#"{
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
        "StaticLiteralRequired",
        "EffectViolation",
        "LinearTypeViolation",
        "NonExhaustiveMatch",
        "VowRequiresViolated",
        "VowEnsuresViolated",
        "VowInvariantViolated",
        "UnknownMethod",
        "UnsupportedFeature",
        "LoweringWarning",
        "MissingContract",
        "ContractTypeMismatch",
        "EsbmcNotFound",
        "IoError",
        "RegionConflict",
        "RegionLinear",
        "RegionRootEscape"
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
"#,
        ),
        (
            r#"schemas/mutants-result.schema.json"#,
            r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.org/schemas/mutants-result.schema.json",
  "title": "MutantsOutput",
  "description": "Output of `vow-mutants run` populates a directory (default `mutants.out/`). Each file has its own schema; the union is documented here.",
  "$defs": {
    "Mutant": {
      "description": "Per-mutant catalog record (in mutants.json).",
      "type": "object",
      "required": ["name", "file", "line", "col", "off", "len", "kind", "from", "to", "label", "clause_index"],
      "properties": {
        "name":         { "type": "string", "description": "Stable cargo-mutants-style name: `file:line:col: <label>`." },
        "file":         { "type": "string", "description": "Repo-relative source path. Independent of --workdir." },
        "line":         { "type": "integer", "minimum": 1, "description": "1-based source line of `off`." },
        "col":          { "type": "integer", "minimum": 1, "description": "1-based source column of `off`." },
        "off":          { "type": "integer", "minimum": 0, "description": "Byte offset of the mutation site." },
        "len":          { "type": "integer", "minimum": 0, "description": "Byte length of the mutation span." },
        "kind":         { "enum": ["op-flip", "const-flip", "body-replace", "contract-weaken"] },
        "from":         { "type": "string", "description": "Original source text at the site (empty for body-replace)." },
        "to":           { "type": "string", "description": "Replacement text spliced in for this mutation." },
        "label":        { "type": "string", "description": "Human-readable summary of the mutation." },
        "clause_index": { "type": "integer", "minimum": 0, "description": "Disambiguates sibling contract clauses on the same function. 0 for non-contract sites." }
      },
      "additionalProperties": false
    },
    "Outcome": {
      "description": "Per-mutant verdict (in outcomes.json).",
      "type": "object",
      "required": ["id", "name", "status", "tier", "oracle_ms"],
      "properties": {
        "id":         { "type": "integer", "minimum": 0, "description": "Position of this mutant within the shard's `mutants.json` array." },
        "name":       { "type": "string", "description": "Same name as the corresponding Mutant record." },
        "status":     { "enum": ["caught", "missed", "timeout", "unviable", "unrun"] },
        "tier":       { "enum": [1, 2], "description": "Oracle tier that produced the verdict." },
        "oracle_ms":  { "type": "integer", "minimum": 0, "description": "Per-mutant oracle wall-clock in milliseconds. Non-deterministic — excluded from the determinism guarantee." }
      },
      "additionalProperties": false
    },
    "Summary": {
      "description": "Aggregate counts across all of this shard's mutants. Same shape as the stdout summary line.",
      "type": "object",
      "required": ["total", "caught", "missed", "timeout", "unviable", "unrun", "shard"],
      "properties": {
        "total":     { "type": "integer", "minimum": 0 },
        "caught":    { "type": "integer", "minimum": 0 },
        "missed":    { "type": "integer", "minimum": 0 },
        "timeout":   { "type": "integer", "minimum": 0 },
        "unviable":  { "type": "integer", "minimum": 0 },
        "unrun":     { "type": "integer", "minimum": 0, "description": "Tier-1 survivors not run because the Tier-2 budget was exhausted." },
        "shard":     { "type": "string", "pattern": "^[0-9]+/[1-9][0-9]*$" }
      },
      "additionalProperties": false
    },
    "MutantsJson": {
      "description": "Format of `mutants.out/mutants.json`. Written before testing begins.",
      "type": "object",
      "required": ["version", "tool", "shard", "mutants"],
      "properties": {
        "version":  { "const": 1 },
        "tool":     { "const": "vow-mutants" },
        "shard":    { "type": "string", "pattern": "^[0-9]+/[1-9][0-9]*$" },
        "mutants":  { "type": "array", "items": { "$ref": "#/$defs/Mutant" } }
      },
      "additionalProperties": false
    },
    "OutcomesJson": {
      "description": "Format of `mutants.out/outcomes.json`. Written after all mutants in this shard have been classified.",
      "type": "object",
      "required": ["version", "summary", "outcomes"],
      "properties": {
        "version":  { "const": 1 },
        "summary":  { "$ref": "#/$defs/Summary" },
        "outcomes": { "type": "array", "items": { "$ref": "#/$defs/Outcome" } }
      },
      "additionalProperties": false
    }
  }
}
"##,
        ),
        (
            r#"schemas/test-result.schema.json"#,
            r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/test-result.schema.json",
  "title": "TestResult",
  "description": "JSON output from `vow test` on stdout",
  "type": "object",
  "required": ["status", "total", "passed", "failed", "skipped", "tests", "contract_density"],
  "properties": {
    "status": {
      "type": "string",
      "enum": ["TestsPassed", "TestsFailed"],
      "description": "Overall test outcome"
    },
    "total": {
      "type": "integer",
      "description": "Total number of tests discovered"
    },
    "passed": {
      "type": "integer",
      "description": "Number of tests that passed"
    },
    "failed": {
      "type": "integer",
      "description": "Number of tests that failed"
    },
    "skipped": {
      "type": "integer",
      "description": "Number of tests that were skipped"
    },
    "tests": {
      "type": "array",
      "items": { "$ref": "#/$defs/TestEntry" },
      "description": "Per-test results"
    },
    "contract_density": {
      "$ref": "#/$defs/ContractDensity",
      "description": "Contract density across tested modules"
    }
  },
  "$defs": {
    "TestEntry": {
      "type": "object",
      "required": ["file", "name", "status", "stdout", "stderr", "duration_ms", "diagnostics", "counterexamples"],
      "properties": {
        "file": { "type": "string", "description": "Path to the test .vow file" },
        "name": { "type": "string", "description": "Test name (file stem)" },
        "status": {
          "type": "string",
          "enum": ["passed", "failed", "timeout", "skipped", "compile_error", "verify_failed"],
          "description": "Per-test outcome"
        },
        "exit_code": {
          "type": ["integer", "null"],
          "description": "Process exit code, null on compile/verify failure or timeout"
        },
        "stdout": { "type": "string", "description": "Captured stdout from test binary" },
        "stderr": { "type": "string", "description": "Captured stderr from test binary" },
        "duration_ms": { "type": "integer", "description": "Wall-clock duration in milliseconds" },
        "diagnostics": {
          "type": "array",
          "items": { "$ref": "diagnostic.schema.json" },
          "description": "Compiler diagnostics (on compile_error)"
        },
        "counterexamples": {
          "type": "array",
          "items": { "$ref": "counterexample.schema.json" },
          "description": "ESBMC counterexamples (on verify_failed)"
        }
      }
    },
    "ContractDensity": {
      "type": "object",
      "required": ["functions_total", "functions_with_vows", "density_pct"],
      "properties": {
        "functions_total": { "type": "integer", "description": "Total non-main functions across tested modules" },
        "functions_with_vows": { "type": "integer", "description": "Functions with at least one vow block" },
        "density_pct": { "type": "number", "description": "Percentage of functions with vow contracts" }
      }
    }
  }
}
"##,
        ),
        (
            r#"schemas/vow-violation.schema.json"#,
            r#"{
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
"#,
        ),
    ]
}
// GENERATE:SKILL_FULL:END

fn install_skill_tree_to(root: &Path) -> std::io::Result<PathBuf> {
    let dir = root.join(".claude/skills/vow");
    std::fs::create_dir_all(&dir).map_err(|e| {
        std::io::Error::new(e.kind(), format!("cannot create {}: {}", dir.display(), e))
    })?;
    let path = dir.join("SKILL.md");
    std::fs::write(&path, skill_entrypoint_markdown()).map_err(|e| {
        std::io::Error::new(e.kind(), format!("cannot write {}: {}", path.display(), e))
    })?;
    for (relative_path, contents) in skill_support_files() {
        let support_path = dir.join(relative_path);
        if let Some(parent) = support_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                std::io::Error::new(
                    e.kind(),
                    format!("cannot create {}: {}", parent.display(), e),
                )
            })?;
        }
        std::fs::write(&support_path, contents).map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!("cannot write {}: {}", support_path.display(), e),
            )
        })?;
    }
    Ok(path)
}

fn run_skill_install_scoped<R: std::io::BufRead, W: std::io::Write>(
    cwd: &Path,
    home: Option<&Path>,
    local: bool,
    global: bool,
    stdin: &mut R,
    stderr: &mut W,
) -> Result<PathBuf, String> {
    if local && global {
        return Err("vow skill install: --local and --global are mutually exclusive".to_string());
    }

    let mut use_local = local;
    let mut use_global = global;
    if !use_local && !use_global {
        stderr
            .write_all(b"Install Vow skill locally (./.claude) or globally (~/.claude)? [l/g]: ")
            .map_err(|e| format!("vow skill install: cannot write prompt: {e}"))?;
        let mut answer = String::new();
        stdin
            .read_line(&mut answer)
            .map_err(|e| format!("vow skill install: cannot read answer: {e}"))?;
        let normalized = answer.trim().to_ascii_lowercase();
        if normalized == "l" || normalized == "local" {
            use_local = true;
        } else if normalized == "g" || normalized == "global" {
            use_global = true;
        } else {
            return Err(
                "vow skill install: stdin closed or unrecognised answer; pass --local or --global"
                    .to_string(),
            );
        }
    }

    let root = if use_local {
        if !cwd.join(".claude").is_dir() {
            return Err(
                "vow skill install: --local requires a .claude/ directory in the current project"
                    .to_string(),
            );
        }
        if !cwd.join(".git").exists() {
            return Err(
                "vow skill install: --local requires the current project to be a git checkout"
                    .to_string(),
            );
        }
        cwd.to_path_buf()
    } else if use_global {
        home.filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| "vow skill install: --global requires $HOME".to_string())?
            .to_path_buf()
    } else {
        return Err("vow skill install: no install target selected".to_string());
    };

    install_skill_tree_to(&root).map_err(|e| format!("vow skill install: {e}"))
}

fn run_skill_install(local: bool, global: bool) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let stderr = std::io::stderr();
    let mut stderr = stderr.lock();
    match run_skill_install_scoped(
        &cwd,
        home.as_deref(),
        local,
        global,
        &mut stdin,
        &mut stderr,
    ) {
        Ok(path) => eprintln!("installed skill to {}", path.display()),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// First-run auto-install: when `.claude/` already exists in the working
/// directory but the Vow skill is not yet installed, drop a fresh copy in
/// place. Silent on success, silent on failure — must never fail the build.
fn maybe_auto_install_skill(cwd: &Path) {
    let claude_dir = cwd.join(".claude");
    if !claude_dir.is_dir() {
        return;
    }
    let target = claude_dir.join("skills/vow/SKILL.md");
    if target.exists() {
        return;
    }
    let _ = install_skill_tree_to(cwd);
}

// ---------------------------------------------------------------------------
// Build output
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum BuildStatus {
    Verified,
    Unverified,
    /// ESBMC ran but ≥1 vowed function non-modelable; fail closed, exit 1.
    Skipped,
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
    /// ESBMC not invoked (`--no-verify`); maps to `BuildStatus::Unverified` (exit 0).
    Skipped,
    /// ESBMC ran but ≥1 vowed function non-modelable; maps to `BuildStatus::Skipped` (exit 1).
    SkippedNonModelable,
    Proven,
    Failed {
        function: String,
        description: String,
        counterexamples: Vec<StructuredCounterexample>,
    },
    Timeout {
        function: String,
    },
    /// ESBMC finished but returned `VERIFICATION UNKNOWN` — neither proof
    /// nor counterexample. Distinct from Timeout (no wall-clock cutoff) and
    /// from Error (no parser failure / process crash).
    Unknown {
        function: String,
        reason: String,
    },
    Error {
        function: String,
        message: String,
    },
    ToolNotFound,
}

/// A vowed function the verifier skipped; surfaces as a Warning in `BuildOutput.diagnostics`.
#[derive(Debug, Clone)]
struct SkippedFunction {
    function: String,
    reason: String,
}

/// Per-function verdict: continue, skip-with-warning, or halt.
enum PerFuncResult {
    Ok,
    Skipped(SkippedFunction),
    Halt(VerifyOutcome),
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
    #[serde(skip)]
    pub function_id: u32,
    pub kind: String,
    pub description: String,
    pub blame: String,
    pub source: ContractSourceJson,
    pub status: String,
    /// Static quality classification of the clause shape (no ESBMC): one of
    /// `weak` (an `ensures` that only bounds `result` by a constant),
    /// `tautological` (constant clause that says nothing about the program),
    /// or `substantive` (equality / relational / inverse / call). See
    /// docs/spec/contracts-methodology.md.
    pub quality: String,
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
    pub skipped: u32,
    pub quality: ContractsQualityJson,
}

/// Static contract-quality tallies (independent of verification status).
#[derive(Debug, Clone, Serialize)]
pub struct ContractsQualityJson {
    pub weak: u32,
    pub tautological: u32,
    pub substantive: u32,
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
            BuildStatus::Skipped => "Skipped",
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
    // ESBMC tripped a fail-closed assertion that vow-verify's c_emitter inserts
    // for opcodes the verifier model does not handle. The sentinel id is
    // reserved and never matches a user-authored vow, so synthesize a
    // diagnostic that an agent can act on instead of letting the code below
    // fall through to the generic "unmatched id" path.
    let unsupported_op = ce.vow_id == Some(UNSUPPORTED_OP_VOW_ID);
    let vow_entry = ce
        .vow_id
        .and_then(|id| func.vows.iter().find(|v| v.id.0 == id));
    let violation = if unsupported_op {
        "function uses side-effecting operations not supported for verification".to_string()
    } else {
        vow_entry
            .map(|v| v.description.clone())
            .unwrap_or_else(|| ce.description.clone())
    };
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

fn emit_frontend_diagnostics(diagnostics: &[Diagnostic]) {
    let mut stderr_emit = HumanEmitter::new(Box::new(std::io::stderr()));
    for diagnostic in diagnostics {
        stderr_emit.emit(diagnostic);
    }
    stderr_emit.finish();
}

fn frontend_error_to_output(error: FrontendError) -> BuildOutput {
    let message = error.failure_message().to_string();
    let diagnostics = error.into_diagnostics();
    BuildOutput {
        status: BuildStatus::CompileFailed { message },
        executable: None,
        diagnostics,
        counterexamples: vec![],
        verify_status: None,
        verify_message: None,
    }
}

fn compile_frontend(source: &Path) -> Result<FrontendBundle, Box<BuildOutput>> {
    compile_frontend_with_root(source, None)
}

/// Same as `compile_frontend`, but resolves `use` declarations against
/// `module_root` rather than the entry file's parent directory. Used by
/// `vowc test` so tests in `compiler/tests/` can `use` sibling compiler modules.
fn compile_frontend_with_root(
    source: &Path,
    module_root: Option<&Path>,
) -> Result<FrontendBundle, Box<BuildOutput>> {
    match prepare_frontend_with_root(source, module_root, FrontendGoal::LoweredIr) {
        Ok(bundle) => {
            emit_frontend_diagnostics(bundle.diagnostics());
            Ok(bundle)
        }
        Err(error) => {
            emit_frontend_diagnostics(error.diagnostics());
            Err(Box::new(frontend_error_to_output(error)))
        }
    }
}

// ---------------------------------------------------------------------------
// Verification (synchronous)
// ---------------------------------------------------------------------------

/// Thread-safe: `verify_cache` writes are content-addressed.
#[allow(clippy::too_many_arguments)]
fn verify_one_function(
    func: &vow_ir::Function,
    ir_module: &vow_ir::Module,
    const_fns: &std::collections::HashMap<vow_ir::FuncId, ConstantValue>,
    file: &str,
    call_site_index: &std::collections::HashMap<String, Vec<CallSiteInfo>>,
    verify_cache: Option<&VerifyCache>,
    limits: &VerifyLimits,
    config: &SolverConfig,
) -> PerFuncResult {
    // Non-modelable vowed functions must be skipped here; the C emitter would emit __ESBMC_assert(0) traps for them.
    if let Some(reason) = non_modelable_reason(func, ir_module, const_fns) {
        return PerFuncResult::Skipped(SkippedFunction {
            function: func.name.clone(),
            reason,
        });
    }

    // Resolve Auto solver via heuristic (Phase B).
    // Skip heuristic when encoding is Ir — that forces Z3 via resolve().
    let func_config = if config.solver == Solver::Auto && config.encoding != Encoding::Ir {
        let heuristic = vow_verify::classify_function(func);
        SolverConfig {
            solver: heuristic.solver,
            encoding: config.encoding,
            timeout_secs: config.timeout_secs,
            memlimit_mb: config.memlimit_mb,
        }
    } else {
        *config
    };

    let result = if let Some(vc) = verify_cache {
        let c_src = emit_verify_c_source(func, ir_module, const_fns, limits);
        let key = VerifyCache::cache_key(
            &c_src,
            limits.max_k_step,
            func_config.solver_str(),
            func_config.encoding_str(),
            func_config.memlimit_mb,
        );

        // Security: lookup only returns FAILED entries (PROVEN is never trusted
        // from disk). The Phase D IR-fallback probe only consumed cached
        // PROVEN, so it is removed: with PROVEN no longer cached, that probe
        // could only return None.
        if let Some(cached) = vc.lookup(&key) {
            VerificationResult::Failed(cached.to_counterexample())
        } else {
            let esbmc = match find_esbmc() {
                Some(p) => p,
                None => return PerFuncResult::Halt(VerifyOutcome::ToolNotFound),
            };
            let (res, resolved_config) =
                run_with_fallback(&esbmc, &c_src, limits.max_k_step, &func.name, &func_config);
            // Security: never cache PROVEN — a forged on-disk entry must not
            // be able to bypass ESBMC on a later run.
            if let VerificationResult::Failed(ce) = &res {
                let store_key = VerifyCache::cache_key(
                    &c_src,
                    limits.max_k_step,
                    resolved_config.solver_str(),
                    resolved_config.encoding_str(),
                    resolved_config.memlimit_mb,
                );
                vc.store(
                    &store_key,
                    &CachedFailure {
                        vow_id: ce.vow_id,
                        description: ce.description.clone(),
                        values: ce.values.clone(),
                        block_visits: ce.block_visits.clone(),
                        raw_output: ce.raw_output.clone(),
                    },
                );
            }
            res
        }
    } else {
        verify_function_with_module_and_const_fns_configured(
            func,
            ir_module,
            const_fns,
            limits.max_k_step,
            &func_config,
            limits,
        )
    };

    match result {
        VerificationResult::Failed(ce) => {
            let sce = build_structured_counterexample(func, &ce, file, call_site_index);
            PerFuncResult::Halt(VerifyOutcome::Failed {
                function: func.name.clone(),
                description: ce.description.clone(),
                counterexamples: vec![sce],
            })
        }
        VerificationResult::ToolError(e) => PerFuncResult::Halt(VerifyOutcome::Error {
            function: func.name.clone(),
            message: e,
        }),
        VerificationResult::Timeout => PerFuncResult::Halt(VerifyOutcome::Timeout {
            function: func.name.clone(),
        }),
        VerificationResult::Unknown { reason } => PerFuncResult::Halt(VerifyOutcome::Unknown {
            function: func.name.clone(),
            reason,
        }),
        VerificationResult::Proven | VerificationResult::ProvenIr => PerFuncResult::Ok,
        VerificationResult::ToolNotFound => PerFuncResult::Halt(VerifyOutcome::ToolNotFound),
        // The verifier-side gate already short-circuits this path; the
        // emit-and-run code above never returns Skipped today. Treat any
        // future Skipped from those entry points the same way.
        VerificationResult::Skipped { reason } => PerFuncResult::Skipped(SkippedFunction {
            function: func.name.clone(),
            reason,
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_verification_sync(
    ir_module: &vow_ir::Module,
    file: &str,
    call_site_index: &std::collections::HashMap<String, Vec<CallSiteInfo>>,
    verify_cache: Option<&VerifyCache>,
    limits: &VerifyLimits,
    jobs: usize,
    config: &SolverConfig,
) -> (VerifyOutcome, Vec<SkippedFunction>) {
    let const_fns = detect_constant_functions(ir_module);

    let vowed: Vec<&vow_ir::Function> = ir_module
        .functions
        .iter()
        .filter(|f| !f.vows.is_empty())
        .collect();

    if vowed.is_empty() {
        return (VerifyOutcome::Proven, Vec::new());
    }

    let jobs = jobs.max(1).min(vowed.len());
    if jobs == 1 {
        let mut skipped = Vec::new();
        for func in &vowed {
            match verify_one_function(
                func,
                ir_module,
                &const_fns,
                file,
                call_site_index,
                verify_cache,
                limits,
                config,
            ) {
                PerFuncResult::Ok => {}
                PerFuncResult::Skipped(s) => skipped.push(s),
                PerFuncResult::Halt(out) => return (out, skipped),
            }
        }
        if skipped.is_empty() {
            return (VerifyOutcome::Proven, skipped);
        }
        return (VerifyOutcome::SkippedNonModelable, skipped);
    }

    // Stop after first halt-class outcome (Failed/Error/Timeout/ToolNotFound);
    // return lowest-indexed halt for deterministic reporting. Skipped
    // functions never halt — they're aggregated and reported as warnings.
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let halts: StdMutex<Vec<Option<VerifyOutcome>>> =
        StdMutex::new((0..vowed.len()).map(|_| None).collect());
    let skipped_acc: StdMutex<Vec<Option<SkippedFunction>>> =
        StdMutex::new((0..vowed.len()).map(|_| None).collect());

    thread::scope(|scope| {
        for _ in 0..jobs {
            let next = &next;
            let stop = &stop;
            let halts = &halts;
            let skipped_acc = &skipped_acc;
            let vowed = &vowed;
            let const_fns = &const_fns;
            scope.spawn(move || {
                loop {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let idx = next.fetch_add(1, Ordering::AcqRel);
                    if idx >= vowed.len() {
                        break;
                    }
                    // Always finish what we've claimed so `halts[idx]` reflects
                    // its true verdict — otherwise lowest-index halt reporting
                    // becomes timing-dependent. The pre-check already avoids claims
                    // in the common post-halt case.
                    match verify_one_function(
                        vowed[idx],
                        ir_module,
                        const_fns,
                        file,
                        call_site_index,
                        verify_cache,
                        limits,
                        config,
                    ) {
                        PerFuncResult::Ok => {}
                        PerFuncResult::Skipped(s) => {
                            let mut guard =
                                skipped_acc.lock().expect("verify skipped mutex poisoned");
                            guard[idx] = Some(s);
                        }
                        PerFuncResult::Halt(out) => {
                            let mut guard = halts.lock().expect("verify halts mutex poisoned");
                            guard[idx] = Some(out);
                            drop(guard);
                            // Release pairs with sibling threads' stop.load(Acquire) to propagate early-exit.
                            stop.store(true, Ordering::Release);
                        }
                    }
                }
            });
        }
    });

    let halts = halts.into_inner().expect("verify halts mutex poisoned");
    let outcome = halts.into_iter().flatten().next().unwrap_or_else(|| {
        if skipped_acc
            .lock()
            .expect("verify skipped mutex poisoned")
            .iter()
            .any(Option::is_some)
        {
            VerifyOutcome::SkippedNonModelable
        } else {
            VerifyOutcome::Proven
        }
    });
    let skipped: Vec<SkippedFunction> = skipped_acc
        .into_inner()
        .expect("verify skipped mutex poisoned")
        .into_iter()
        .flatten()
        .collect();
    (outcome, skipped)
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
    diagnostics: Vec<Diagnostic>,
    executable: Option<PathBuf>,
) -> BuildOutput {
    verify_outcome_to_output_with_skipped(outcome, diagnostics, &[], executable)
}

fn verify_outcome_to_output_with_skipped(
    outcome: VerifyOutcome,
    mut diagnostics: Vec<Diagnostic>,
    skipped: &[SkippedFunction],
    executable: Option<PathBuf>,
) -> BuildOutput {
    for s in skipped {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            code: vow_diag::ErrorCode::VerificationSkipped,
            message: format!("skipped verification of `{}`: {}", s.function, s.reason),
            primary: vow_diag::SourceLocation {
                file: String::new(),
                byte_offset: 0,
                byte_len: 0,
            },
            secondary: vec![],
            blame: vow_diag::Blame::None,
            hints: vec![
                "the contract is documentary; runtime checks still apply in --mode debug"
                    .to_string(),
            ],
        });
    }
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
        VerifyOutcome::Unknown { function, reason } => (
            BuildStatus::VerifyFailed {
                function,
                description: format!("verification result unknown: {reason}"),
            },
            vec![],
            Some("unknown".to_string()),
            Some(reason),
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
        VerifyOutcome::SkippedNonModelable => (BuildStatus::Skipped, vec![], None, None),
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
    let limits = VerifyLimits::default();
    run_verify_only_inner(source, false, &limits, 1, &SolverConfig::default_config())
}

fn run_verify_only_inner(
    source: &Path,
    no_cache: bool,
    limits: &VerifyLimits,
    jobs: usize,
    config: &SolverConfig,
) -> BuildOutput {
    let frontend = match compile_frontend(source) {
        Ok(f) => f,
        Err(output) => return *output,
    };
    let all_diagnostics = frontend.diagnostics().to_vec();
    let ir_module = frontend
        .ir()
        .expect("LoweredIr goal must produce IR for verify-only");

    if find_esbmc().is_none() {
        return verify_outcome_to_output(VerifyOutcome::ToolNotFound, all_diagnostics, None);
    }

    let verify_cache = if no_cache { None } else { VerifyCache::new() };
    let file = source.to_string_lossy().to_string();
    let call_site_index = build_call_site_index(ir_module, &file);
    let (outcome, skipped) = run_verification_sync(
        ir_module,
        &file,
        &call_site_index,
        verify_cache.as_ref(),
        limits,
        jobs,
        config,
    );
    verify_outcome_to_output_with_skipped(outcome, all_diagnostics, &skipped, None)
}

// ---------------------------------------------------------------------------
// Full build pipeline (vow build / legacy)
// ---------------------------------------------------------------------------

fn link_obj(obj_path: &Path, output_path: &Path) -> Result<PathBuf, String> {
    let runtime = find_runtime_lib().ok_or_else(|| {
        "could not find libvow_runtime.a; build it with `cargo build --release --all` \
         or set VOW_RUNTIME_PATH"
            .to_string()
    })?;
    link(
        &[obj_path],
        &runtime,
        find_shim_lib().as_deref(),
        output_path,
    )
    .map_err(|e| format!("link failed: {e:?}"))?;
    let _ = std::fs::remove_file(obj_path);
    Ok(output_path.to_path_buf())
}

// Compile-object cache is only enabled when `--no-cache` is off and verification is off, so verified builds always link a fresh codegen output.
fn compile_cache_enabled(no_cache: bool, no_verify: bool) -> bool {
    !no_cache && no_verify
}

pub fn run_pipeline(
    source: &Path,
    output: Option<&Path>,
    mode: BuildMode,
    no_verify: bool,
    dump_ir: bool,
    trace: TraceMode,
) -> BuildOutput {
    let limits = VerifyLimits::default();
    run_pipeline_inner(
        source,
        output,
        mode,
        no_verify,
        dump_ir,
        trace,
        false,
        &limits,
        1,
        &SolverConfig::default_config(),
    )
}

#[allow(clippy::too_many_arguments)]
fn run_pipeline_inner(
    source: &Path,
    output: Option<&Path>,
    mode: BuildMode,
    no_verify: bool,
    dump_ir: bool,
    trace: TraceMode,
    no_cache: bool,
    limits: &VerifyLimits,
    jobs: usize,
    config: &SolverConfig,
) -> BuildOutput {
    let frontend = match compile_frontend(source) {
        Ok(f) => f,
        Err(output) => return *output,
    };

    run_pipeline_from_frontend(
        frontend, source, output, mode, no_verify, dump_ir, trace, no_cache, limits, jobs, config,
    )
}

/// Fail-closed result for a panicked verifier worker (`join()` → `Err`). A
/// verifier crash leaves verification in an unknown state, so the build must
/// report `VerifyFailed` (exit 1) and withhold the executable — never the silent
/// `Unverified`/exit-0 of the old `.unwrap_or((Skipped, _))` fallback (#413).
/// The linked object is removed so no binary masquerades as built.
fn verifier_panicked_output(
    diagnostics: Vec<Diagnostic>,
    executable: Option<PathBuf>,
) -> BuildOutput {
    if let Some(path) = &executable {
        let _ = std::fs::remove_file(path);
    }
    BuildOutput {
        status: BuildStatus::VerifyFailed {
            function: String::new(),
            description: "verification thread panicked".to_string(),
        },
        executable: None,
        diagnostics,
        counterexamples: vec![],
        verify_status: Some("panicked".to_string()),
        verify_message: Some("verification thread panicked".to_string()),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_pipeline_from_frontend(
    frontend: FrontendBundle,
    source: &Path,
    output: Option<&Path>,
    mode: BuildMode,
    no_verify: bool,
    dump_ir: bool,
    trace: TraceMode,
    no_cache: bool,
    limits: &VerifyLimits,
    jobs: usize,
    config: &SolverConfig,
) -> BuildOutput {
    // Consume the frontend bundle immediately so the AST `Module` field can
    // be dropped before codegen + verify start. The `deps` manifest survives
    // for the cache-key computation below; `all_diagnostics` flows into the
    // final BuildOutput; `ir_module` is the only large allocation we keep
    // alive past this point (shared via Arc with the verify thread). See #178.
    let (all_diagnostics, ir_opt, deps) = frontend.into_parts();
    let ir_module = ir_opt.expect("LoweredIr goal must produce IR for build pipeline");

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
    let verify_limits = *limits;
    let verify_config = *config;
    let verify_handle = thread::spawn(move || -> (VerifyOutcome, Vec<SkippedFunction>) {
        if no_verify {
            return (VerifyOutcome::Skipped, Vec::new());
        }
        // Test-only fault injection: simulate a verifier-worker crash so the
        // fail-closed JoinError path (#413) is exercised end-to-end. Guarded by
        // an env var that is never set in production; one lookup per build.
        if std::env::var_os("VOW_TEST_VERIFIER_PANIC").is_some() {
            panic!("injected verifier panic (VOW_TEST_VERIFIER_PANIC)");
        }
        run_verification_sync(
            &module_for_verify,
            &file_for_verify,
            &call_site_index,
            verify_cache.as_ref(),
            &verify_limits,
            jobs,
            &verify_config,
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
    // Disable object cache when verification is active: linked binary must come from the same codegen run as the verified IR.
    let compile_cache = if compile_cache_enabled(no_cache, no_verify) {
        cache::CompileCache::new()
    } else {
        None
    };
    // Skip the dependency-content hash when the cache is disabled — no point reading every dep file with no possible hit/store. `and_then` propagates a None from `cache_key` (fail-closed on per-dep canonicalize/open/read errors) so neither lookup nor store fires with an incomplete dep set.
    let cache_key = compile_cache
        .as_ref()
        .and_then(|_| cache::CompileCache::cache_key(&deps, &mode_str, &trace_str));
    if compile_cache.is_some() && cache_key.is_none() {
        eprintln!("warning: compile cache bypassed — one or more dependencies could not be hashed");
    }

    if let Some(ref cc) = compile_cache
        && let Some(ref key) = cache_key
        && let Some(cached_obj) = cc.lookup(key)
        && std::fs::copy(&cached_obj, &obj_path).is_ok()
    {
        let exe_path = match link_obj(&obj_path, &output_path) {
            Ok(p) => Some(p),
            Err(message) => {
                let _ = verify_handle.join();
                return BuildOutput {
                    status: BuildStatus::CompileFailed { message },
                    executable: None,
                    diagnostics: all_diagnostics,
                    counterexamples: vec![],
                    verify_status: None,
                    verify_message: None,
                };
            }
        };
        let (verify_outcome, skipped) = match verify_handle.join() {
            Ok(result) => result,
            Err(_) => return verifier_panicked_output(all_diagnostics, exe_path),
        };
        return verify_outcome_to_output_with_skipped(
            verify_outcome,
            all_diagnostics,
            &skipped,
            exe_path,
        );
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
    if let Some(ref cc) = compile_cache
        && let Some(ref key) = cache_key
    {
        cc.store(key, &obj_path);
    }

    let exe_path = match link_obj(&obj_path, &output_path) {
        Ok(p) => Some(p),
        Err(message) => {
            let _ = verify_handle.join();
            return BuildOutput {
                status: BuildStatus::CompileFailed { message },
                executable: None,
                diagnostics: all_diagnostics,
                counterexamples: vec![],
                verify_status: None,
                verify_message: None,
            };
        }
    };

    let (verify_outcome, skipped) = match verify_handle.join() {
        Ok(result) => result,
        Err(_) => return verifier_panicked_output(all_diagnostics, exe_path),
    };
    verify_outcome_to_output_with_skipped(verify_outcome, all_diagnostics, &skipped, exe_path)
}

// ---------------------------------------------------------------------------
// Test pipeline (vow test)
// ---------------------------------------------------------------------------

fn discover_test_files(path: &Path) -> Vec<PathBuf> {
    if path.is_file() {
        return vec![path.to_path_buf()];
    }
    let mut files: Vec<PathBuf> = Vec::new();
    collect_test_files(path, &mut files);
    files.sort();
    files
}

fn collect_test_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let entry_path = entry.path();
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            collect_test_files(&entry_path, out);
        } else if file_type.is_file() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".vow") && (name.starts_with("test_") || name.ends_with("_test.vow"))
            {
                out.push(entry_path);
            }
        }
    }
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
    let tenths = ((with_vows * 1000).checked_div(total)).unwrap_or(0);
    ContractDensity {
        functions_total: total,
        functions_with_vows: with_vows,
        density_pct: (tenths / 10) as f64 + (tenths % 10) as f64 / 10.0,
    }
}

#[allow(clippy::too_many_arguments)]
fn run_test_command(
    path: &Path,
    verify: bool,
    filter: Option<&str>,
    module_root_override: Option<&Path>,
    mode: BuildMode,
    timeout_ms: u64,
    limits: &VerifyLimits,
    jobs: usize,
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

    let _ = std::fs::create_dir_all("build");

    // Resolve module root precedence:
    //   1. explicit --module-root <path> wins (covers single-file invocation
    //      against a tests/ subdir, e.g. `vow test compiler/tests/test_x.vow
    //      --module-root compiler`),
    //   2. otherwise, when the scan path is a directory, use the scan path,
    //   3. otherwise (single-file scan with no override), fall back to the
    //      entry file's parent dir (None).
    let module_root: Option<&Path> = if let Some(override_path) = module_root_override {
        Some(override_path)
    } else if path.is_dir() {
        Some(path)
    } else {
        None
    };

    for test_file in &test_files {
        let start = std::time::Instant::now();
        let file_str = test_file.to_string_lossy().to_string();
        let name = test_file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        // Compile frontend once — extract density before codegen
        let frontend = match compile_frontend_with_root(test_file, module_root) {
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

        let density = count_contract_density(
            frontend
                .ir()
                .expect("LoweredIr goal must produce IR for test density"),
        );
        total_density.functions_total += density.functions_total;
        total_density.functions_with_vows += density.functions_with_vows;

        let tmp_out = Path::new("build").join(format!("vow_test_{name}_{}", std::process::id()));
        let result = run_pipeline_from_frontend(
            frontend,
            test_file,
            Some(&tmp_out),
            mode,
            !verify,
            false,
            TraceMode::Off,
            true,
            limits,
            jobs,
            &SolverConfig::default_config(),
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
            BuildStatus::VerifyFailed { .. } | BuildStatus::Skipped => {
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
    if let Some(tenths) =
        (total_density.functions_with_vows * 1000).checked_div(total_density.functions_total)
    {
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

/// User value verbatim; rejects 0 with a clear error. None → num_cpus/2 clamped to ≥1.
fn resolve_verify_jobs(opt: Option<u32>) -> usize {
    match opt {
        Some(0) => {
            eprintln!("error: --verify-jobs must be >= 1");
            std::process::exit(1);
        }
        Some(n) => n as usize,
        None => {
            let n = std::thread::available_parallelism()
                .map(|p| p.get())
                .unwrap_or(1);
            (n / 2).max(1)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_build_command(
    source: &Path,
    output: Option<&Path>,
    mode: BuildMode,
    no_verify: bool,
    dump_ir: bool,
    trace: TraceMode,
    no_cache: bool,
    limits: &VerifyLimits,
    jobs: usize,
    config: &SolverConfig,
) {
    let result = run_pipeline_inner(
        source, output, mode, no_verify, dump_ir, trace, no_cache, limits, jobs, config,
    );
    if !dump_ir {
        result.emit_json();
    }
    if matches!(
        &result.status,
        BuildStatus::CompileFailed { .. } | BuildStatus::VerifyFailed { .. } | BuildStatus::Skipped
    ) {
        std::process::exit(1);
    }
}

fn run_decl_command(source: &Path, output: Option<&Path>) {
    let frontend = match prepare_frontend(source, FrontendGoal::MergedAst) {
        Ok(bundle) => {
            emit_frontend_diagnostics(bundle.diagnostics());
            bundle
        }
        Err(error) => {
            emit_frontend_diagnostics(error.diagnostics());
            eprintln!("vow decl: {}", error.failure_message());
            std::process::exit(1);
        }
    };

    let decl_text = vow_syntax::printer::print_declarations(frontend.module());

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

fn run_verify_command(
    source: &Path,
    no_cache: bool,
    limits: &VerifyLimits,
    jobs: usize,
    config: &SolverConfig,
) {
    let result = run_verify_only_inner(source, no_cache, limits, jobs, config);
    result.emit_json();
    if matches!(
        &result.status,
        BuildStatus::CompileFailed { .. } | BuildStatus::VerifyFailed { .. } | BuildStatus::Skipped
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

/// Strip the leading `requires`/`ensures`/`invariant` keyword from a clause
/// description, leaving the predicate text. `clause_description` formats every
/// vow as `"{kind} {printed_expr}"`, so the predicate is everything after the
/// first space.
fn contract_predicate_text(description: &str) -> &str {
    match description.split_once(' ') {
        Some((_, rest)) => rest.trim(),
        None => "",
    }
}

/// Static, no-ESBMC quality classification of a contract clause by shape.
/// See docs/spec/contracts-methodology.md for the methodology this implements.
///
/// - `tautological`: the predicate is the constant `true` or references no
///   program value at all (e.g. `0 >= 0`) — it constrains nothing. A `false`
///   predicate is a contradiction, not a tautology, so it is left `substantive`
///   here; flagging it as vacuous is the deferred `false` re-check.
/// - `weak`: an `ensures` that only bounds `result` by an integer literal on
///   one side (e.g. `result >= 0`, `result > 0`, `result <= 3`). Satisfiable by
///   almost any implementation — the 354-contract trap #81 was filed over.
/// - `substantive`: everything else (equality, relational, inverse, calls).
///
/// The classifier is deliberately conservative: anything it cannot prove weak
/// is reported `substantive`, so it never over-flags a meaningful contract.
fn classify_contract_quality(kind: &str, description: &str) -> &'static str {
    let p = contract_predicate_text(description);
    if p.is_empty() || p == "true" || !p.chars().any(|c| c.is_ascii_alphabetic()) {
        return "tautological";
    }
    if kind == "ensures" && is_weak_result_bound(p) {
        return "weak";
    }
    "substantive"
}

/// True when `pred` is a single ordering comparison between `result` and an
/// integer literal — the weak postcondition shape. Compound predicates,
/// equalities, and calls are excluded (they are potentially substantive).
fn is_weak_result_bound(pred: &str) -> bool {
    if pred.contains("&&")
        || pred.contains("||")
        || pred.contains("==")
        || pred.contains("!=")
        || pred.contains('(')
    {
        return false;
    }
    for op in ["<=", ">="] {
        if let Some((lhs, rhs)) = pred.split_once(op) {
            return is_weak_result_comparison(lhs, rhs);
        }
    }
    for op in ['<', '>'] {
        if let Some((lhs, rhs)) = pred.split_once(op) {
            return is_weak_result_comparison(lhs, rhs);
        }
    }
    false
}

fn is_weak_result_comparison(lhs: &str, rhs: &str) -> bool {
    let lhs = lhs.trim();
    let rhs = rhs.trim();
    // Reject anything with a second comparison operator on either side.
    if has_ordering_op(lhs) || has_ordering_op(rhs) {
        return false;
    }
    (lhs == "result" && is_int_literal(rhs)) || (rhs == "result" && is_int_literal(lhs))
}

fn has_ordering_op(s: &str) -> bool {
    s.contains('<') || s.contains('>')
}

fn is_int_literal(s: &str) -> bool {
    let digits = s.strip_prefix('-').unwrap_or(s);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
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
        skipped: 0,
        quality: ContractsQualityJson {
            weak: 0,
            tautological: 0,
            substantive: 0,
        },
    };
    for e in entries {
        match e.status.as_str() {
            "proven" | "proven-ir" => summary.proven += 1,
            "failed" => summary.failed += 1,
            "unknown" => summary.unknown += 1,
            "timeout" => summary.timeout += 1,
            "error" => summary.error += 1,
            "skipped" => summary.skipped += 1,
            _ => summary.not_verified += 1,
        }
        match e.quality.as_str() {
            "weak" => summary.quality.weak += 1,
            "tautological" => summary.quality.tautological += 1,
            _ => summary.quality.substantive += 1,
        }
    }
    summary
}

fn update_contract_statuses(
    entries: &mut [ContractEntryJson],
    ir_module: &vow_ir::Module,
    verify_cache: Option<&VerifyCache>,
    limits: &VerifyLimits,
    config: &SolverConfig,
) {
    let const_fns = detect_constant_functions(ir_module);
    for func in &ir_module.functions {
        if func.vows.is_empty() {
            continue;
        }

        if non_modelable_reason(func, ir_module, &const_fns).is_some() {
            for entry in entries.iter_mut() {
                if entry.function_id == func.id.0 {
                    entry.status = "skipped".to_string();
                }
            }
            continue;
        }

        let result = if let Some(vc) = verify_cache {
            let c_src = emit_verify_c_source(func, ir_module, &const_fns, limits);
            let key = VerifyCache::cache_key(
                &c_src,
                limits.max_k_step,
                config.solver_str(),
                config.encoding_str(),
                config.memlimit_mb,
            );

            // Security: lookup only returns FAILED (PROVEN is never trusted
            // from disk); the Phase D IR-fallback probe consumed only cached
            // PROVEN and is removed since that path can never hit.
            if let Some(cached) = vc.lookup(&key) {
                VerificationResult::Failed(cached.to_counterexample())
            } else {
                let esbmc = match find_esbmc() {
                    Some(p) => p,
                    None => {
                        for entry in entries.iter_mut() {
                            if entry.function_id == func.id.0 {
                                entry.status = "error".to_string();
                            }
                        }
                        continue;
                    }
                };
                let (res, resolved_config) =
                    run_with_fallback(&esbmc, &c_src, limits.max_k_step, &func.name, config);
                // Security: never cache PROVEN.
                if let VerificationResult::Failed(ce) = &res {
                    let store_key = VerifyCache::cache_key(
                        &c_src,
                        limits.max_k_step,
                        resolved_config.solver_str(),
                        resolved_config.encoding_str(),
                        resolved_config.memlimit_mb,
                    );
                    vc.store(
                        &store_key,
                        &CachedFailure {
                            vow_id: ce.vow_id,
                            description: ce.description.clone(),
                            values: ce.values.clone(),
                            block_visits: ce.block_visits.clone(),
                            raw_output: ce.raw_output.clone(),
                        },
                    );
                }
                res
            }
        } else {
            verify_function_with_module_and_const_fns_configured(
                func,
                ir_module,
                &const_fns,
                limits.max_k_step,
                config,
                limits,
            )
        };

        for entry in entries.iter_mut() {
            if entry.function_id == func.id.0 {
                match &result {
                    VerificationResult::Proven => {
                        entry.status = "proven".to_string();
                    }
                    VerificationResult::ProvenIr => {
                        entry.status = "proven-ir".to_string();
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
                    VerificationResult::Unknown { .. } => {
                        entry.status = "unknown".to_string();
                    }
                    VerificationResult::ToolError(_) | VerificationResult::ToolNotFound => {
                        entry.status = "error".to_string();
                    }
                    VerificationResult::Skipped { .. } => {
                        entry.status = "skipped".to_string();
                    }
                }
            }
        }
    }
}

fn run_contracts_command(
    source: &Path,
    verify: bool,
    no_cache: bool,
    limits: &VerifyLimits,
    config: &SolverConfig,
) {
    let frontend = match compile_frontend(source) {
        Ok(f) => f,
        Err(output) => {
            output.emit_json();
            std::process::exit(1);
        }
    };
    let ir_module = frontend
        .ir()
        .expect("LoweredIr goal must produce IR for contracts");

    let mut entries: Vec<ContractEntryJson> = Vec::new();
    for func in &ir_module.functions {
        for vow in &func.vows {
            let kind = vow_kind_from_description(&vow.description);
            let blame = match vow.blame {
                vow_diag::Blame::Caller => "Caller",
                vow_diag::Blame::Callee => "Callee",
                vow_diag::Blame::None => "None",
            };
            let quality = classify_contract_quality(kind, &vow.description);
            entries.push(ContractEntryJson {
                vow_id: vow.id.0,
                function: func.name.clone(),
                function_id: func.id.0,
                kind: kind.to_string(),
                description: vow.description.clone(),
                blame: blame.to_string(),
                source: ContractSourceJson {
                    file: vow.file.clone(),
                    offset: vow.offset,
                },
                status: "not_verified".to_string(),
                quality: quality.to_string(),
            });
        }
    }

    let mut exit_code = 0;
    if verify {
        if find_esbmc().is_none() {
            for entry in &mut entries {
                entry.status = "error".to_string();
            }
            exit_code = 1;
        } else {
            let verify_cache = if no_cache { None } else { VerifyCache::new() };
            update_contract_statuses(
                &mut entries,
                ir_module,
                verify_cache.as_ref(),
                limits,
                config,
            );
        }
    }

    let summary = build_contracts_summary(&entries);
    let result = ContractsResultJson {
        contracts: entries,
        summary,
    };
    let json = serde_json::to_string(&result).expect("ContractsResult must be serializable");
    println!("{json}");
    if exit_code != 0 {
        std::process::exit(exit_code);
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
                ModeArg::Profile => BuildMode::Profile,
                ModeArg::Sanitize => BuildMode::Sanitize,
            };
            let trace = match b.debug_trace {
                TraceArg::Off => TraceMode::Off,
                TraceArg::Calls => TraceMode::Calls,
                TraceArg::Full => TraceMode::Full,
            };
            let limits = VerifyLimits {
                max_k_step: b.max_k_step,
                ..VerifyLimits::default()
            };
            let jobs = resolve_verify_jobs(b.verify_jobs);
            let bconfig = make_solver_config(b.solver, b.encoding, b.timeout);
            if let Ok(cwd) = std::env::current_dir() {
                maybe_auto_install_skill(&cwd);
            }
            run_build_command(
                &source,
                b.output.as_deref(),
                mode,
                b.no_verify,
                b.dump_ir,
                trace,
                b.no_cache,
                &limits,
                jobs,
                &bconfig,
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
            let limits = VerifyLimits {
                max_k_step: v.max_k_step,
                ..VerifyLimits::default()
            };
            let jobs = resolve_verify_jobs(v.verify_jobs);
            let config = make_solver_config(v.solver, v.encoding, v.timeout);
            run_verify_command(&source, v.no_cache, &limits, jobs, &config);
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
                ModeArg::Sanitize => BuildMode::Sanitize,
            };
            let limits = VerifyLimits {
                max_k_step: t.max_k_step,
                ..VerifyLimits::default()
            };
            let jobs = resolve_verify_jobs(t.verify_jobs);
            run_test_command(
                &path,
                t.verify,
                t.filter.as_deref(),
                t.module_root.as_deref(),
                mode,
                t.timeout,
                &limits,
                jobs,
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
            let limits = VerifyLimits {
                max_k_step: c.max_k_step.unwrap_or(DEFAULT_MAX_K_STEP),
                ..VerifyLimits::default()
            };
            // Accepted for CLI parity; resolved via the same path as
            // build/verify/test, then discarded because update_contract_statuses
            // has no pool wiring today.
            let _ = resolve_verify_jobs(c.verify_jobs);
            let config = make_solver_config(c.solver, c.encoding, c.timeout);
            run_contracts_command(&source, c.verify, c.no_cache, &limits, &config);
        }
        Some(Command::Skill(s)) => {
            if s.help {
                if s.human {
                    println!("{}", skill_human());
                } else {
                    println!("{}", skill_json());
                }
                return;
            }
            match s.action {
                Some(SkillAction::Install { local, global }) => {
                    run_skill_install(local, global);
                }
                Some(SkillAction::Print { bundle: true }) => {
                    println!("{}", skill_bundle_markdown());
                }
                Some(SkillAction::Print { bundle: false }) => {
                    println!("{}", skill_entrypoint_markdown());
                }
                None => {
                    println!("{}", skill_entrypoint_markdown());
                }
            }
        }
        Some(Command::Mutants(_)) => {
            eprintln!(
                "vow: `mutants` is implemented in the self-hosted compiler only.\n\
                 Use `build/vowc mutants <subcommand>` after running `scripts/bootstrap.sh`."
            );
            std::process::exit(2);
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
                ModeArg::Sanitize => BuildMode::Sanitize,
            };
            let trace = match args.debug_trace {
                TraceArg::Off => TraceMode::Off,
                TraceArg::Calls => TraceMode::Calls,
                TraceArg::Full => TraceMode::Full,
            };

            let limits = VerifyLimits {
                max_k_step: args.max_k_step,
                ..VerifyLimits::default()
            };
            let jobs = resolve_verify_jobs(args.verify_jobs);
            let config = make_solver_config(args.solver, args.encoding, args.timeout);
            if let Ok(cwd) = std::env::current_dir() {
                maybe_auto_install_skill(&cwd);
            }
            run_build_command(
                &source,
                args.output.as_deref(),
                mode,
                args.no_verify,
                args.dump_ir,
                trace,
                args.no_cache,
                &limits,
                jobs,
                &config,
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

    fn esbmc_not_found(status: &BuildStatus) -> bool {
        matches!(
            status,
            BuildStatus::VerifyFailed { description, .. }
                if description.contains("ESBMC not found")
        )
    }

    #[test]
    fn capacity_flags_are_not_advertised() {
        // Issue #278: the verify-only --vec-max / --string-max / --hashmap-max /
        // --btreemap-max flags were removed. The collection model bound is an
        // internal verifier detail, not a CLI knob, so it must never reappear in
        // any help or skill surface. (--max-k-step is a real, retained flag.)
        let surfaces = [
            skill_json(),
            skill_human(),
            skill_entrypoint_markdown(),
            skill_bundle_markdown(),
        ];
        for surface in &surfaces {
            for flag in [
                "--vec-max",
                "--string-max",
                "--hashmap-max",
                "--btreemap-max",
            ] {
                assert!(
                    !surface.contains(flag),
                    "removed capacity flag `{flag}` still advertised in a help/skill surface"
                );
            }
        }
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
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["schema_version"], "2");
        assert_eq!(parsed["kind"], "tool_help");
        assert_eq!(parsed["tool"], "vow");
        assert_eq!(parsed["audience"], "agent");
        assert!(parsed["language"].is_object(), "expected language section");
        assert!(parsed["commands"]["build"].is_string());
        assert!(parsed["command_details"]["build"]["options"].is_array());
        assert!(parsed["outputs"]["build_result"].is_object());
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
    fn skill_install_writes_concise_entrypoint_and_support_files() {
        let dir = TempDir::new().unwrap();
        install_skill_tree_to(dir.path()).unwrap();

        let skill_dir = dir.path().join(".claude/skills/vow");
        let contents = std::fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
        assert!(contents.starts_with("---\nname: vow\n"));
        assert!(contents.contains("when_to_use: >-"));
        assert!(!contents.contains("\npaths:"));
        assert!(!contents.contains("\nallowed-tools:"));
        assert!(contents.lines().count() < 500);
        assert!(
            !contents.contains("```json"),
            "SKILL.md entrypoint should link to schemas, not inline them"
        );

        let grammar = std::fs::read_to_string(skill_dir.join("reference/grammar.md")).unwrap();
        assert!(grammar.contains("# Vow Grammar Reference"));
        let schema =
            std::fs::read_to_string(skill_dir.join("schemas/build-result.schema.json")).unwrap();
        assert!(schema.contains("\"title\": \"BuildResult\""));
    }

    #[test]
    fn skill_bundle_markdown_contains_full_skill_document() {
        let out = skill_bundle_markdown();
        assert!(out.starts_with("---\nname: vow\n"));
        assert!(!out.contains("\npaths:"));
        assert!(!out.contains("\nallowed-tools:"));
        assert!(out.contains("# Vow Language Reference"));
        assert!(out.contains("### `vow skill`"));
        assert!(out.contains("schemas/build-result.schema.json"));
    }

    /// `npx skills add vow-lang/vow` clones the repo and scans `skills/*/SKILL.md`.
    /// The checked-in `skills/vow/` mirror must therefore match what the compiler
    /// installs, or the two install paths drift.
    #[test]
    fn checked_in_skills_vow_matches_install_output() {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("CARGO_MANIFEST_DIR has a parent");
        let skill_md = repo_root.join("skills/vow/SKILL.md");
        let checked_in =
            std::fs::read_to_string(&skill_md).expect("skills/vow/SKILL.md must exist");
        assert!(
            checked_in.starts_with("---\nname: vow\n"),
            "checked-in skills/vow/SKILL.md must declare `name: vow` for npx skills"
        );
        assert_eq!(
            checked_in,
            skill_entrypoint_markdown(),
            "skills/vow/SKILL.md drifted from compiler-embedded skill; \
             run `uv run python scripts/generate_help.py`"
        );
        for (rel, expected) in skill_support_files() {
            let path = repo_root.join("skills/vow").join(rel);
            let actual = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("missing skills/vow/{rel}: {e}"));
            assert_eq!(
                actual, *expected,
                "skills/vow/{rel} drifted from compiler-embedded copy"
            );
        }
    }

    #[test]
    fn auto_install_skill_skips_when_no_claude_dir() {
        let dir = TempDir::new().unwrap();
        maybe_auto_install_skill(dir.path());
        let claude_dir = dir.path().join(".claude");
        assert!(
            !claude_dir.exists(),
            "auto-install must not create .claude/ when absent"
        );
    }

    #[test]
    fn auto_install_skill_installs_when_claude_dir_present() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        maybe_auto_install_skill(dir.path());
        let installed = dir.path().join(".claude/skills/vow/SKILL.md");
        assert!(
            installed.exists(),
            "auto-install must populate the skill when .claude/ is present"
        );
        let contents = std::fs::read_to_string(installed).unwrap();
        assert!(!contents.contains("\nallowed-tools:"));
    }

    #[test]
    fn auto_install_skill_leaves_existing_file_untouched() {
        let dir = TempDir::new().unwrap();
        let target_dir = dir.path().join(".claude/skills/vow");
        std::fs::create_dir_all(&target_dir).unwrap();
        let target = target_dir.join("SKILL.md");
        std::fs::write(&target, "user-managed content").unwrap();
        maybe_auto_install_skill(dir.path());
        let contents = std::fs::read_to_string(&target).unwrap();
        assert_eq!(contents, "user-managed content");
    }

    #[test]
    fn skill_install_local_writes_into_current_git_project() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::fs::write(dir.path().join(".git"), "gitdir: ../real-git-dir\n").unwrap();
        let mut stdin = std::io::Cursor::new(Vec::<u8>::new());
        let mut stderr = Vec::new();

        let installed =
            run_skill_install_scoped(dir.path(), None, true, false, &mut stdin, &mut stderr)
                .unwrap();

        assert_eq!(installed, dir.path().join(".claude/skills/vow/SKILL.md"));
        assert!(installed.exists());
        assert!(
            dir.path()
                .join(".claude/skills/vow/reference/cli.md")
                .exists()
        );
    }

    #[test]
    fn skill_install_accepts_explicit_scope_flags() {
        let parsed = Args::try_parse_from(["vow", "skill", "install", "--local"]).unwrap();
        let Some(Command::Skill(skill)) = parsed.command else {
            panic!("expected skill command");
        };
        let Some(SkillAction::Install { local, global }) = skill.action else {
            panic!("expected install action");
        };
        assert!(local);
        assert!(!global);

        let parsed = Args::try_parse_from(["vow", "skill", "install", "--global"]).unwrap();
        let Some(Command::Skill(skill)) = parsed.command else {
            panic!("expected skill command");
        };
        let Some(SkillAction::Install { local, global }) = skill.action else {
            panic!("expected install action");
        };
        assert!(!local);
        assert!(global);
    }

    #[test]
    fn skill_install_global_writes_under_home() {
        let cwd = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let mut stdin = std::io::Cursor::new(Vec::<u8>::new());
        let mut stderr = Vec::new();

        let installed = run_skill_install_scoped(
            cwd.path(),
            Some(home.path()),
            false,
            true,
            &mut stdin,
            &mut stderr,
        )
        .unwrap();

        assert_eq!(installed, home.path().join(".claude/skills/vow/SKILL.md"));
        assert!(installed.exists());
        assert!(
            home.path()
                .join(".claude/skills/vow/schemas/diagnostic.schema.json")
                .exists()
        );
    }

    #[test]
    fn skill_install_local_requires_git_and_claude_project() {
        let dir = TempDir::new().unwrap();
        let mut stdin = std::io::Cursor::new(Vec::<u8>::new());
        let mut stderr = Vec::new();

        let err = run_skill_install_scoped(dir.path(), None, true, false, &mut stdin, &mut stderr)
            .unwrap_err();
        assert!(err.contains(".claude"));

        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        let err = run_skill_install_scoped(dir.path(), None, true, false, &mut stdin, &mut stderr)
            .unwrap_err();
        assert!(err.contains("git"));
    }

    #[test]
    fn skill_install_prompts_when_scope_is_omitted() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::fs::write(dir.path().join(".git"), "gitdir: ../real-git-dir\n").unwrap();
        let mut stdin = std::io::Cursor::new(b"local\n".to_vec());
        let mut stderr = Vec::new();

        let installed =
            run_skill_install_scoped(dir.path(), None, false, false, &mut stdin, &mut stderr)
                .unwrap();
        assert_eq!(installed, dir.path().join(".claude/skills/vow/SKILL.md"));
        assert!(String::from_utf8(stderr).unwrap().contains("[l/g]"));

        let mut stdin = std::io::Cursor::new(Vec::<u8>::new());
        let mut stderr = Vec::new();
        let err = run_skill_install_scoped(dir.path(), None, false, false, &mut stdin, &mut stderr)
            .unwrap_err();
        assert!(err.contains("--local or --global"));
    }

    #[test]
    fn agent_capability_test_skill_json_is_parseable_and_complete() {
        // Verify the --help JSON contains enough information for an LLM agent
        // to write correct Vow code without additional context.
        let parsed: serde_json::Value = serde_json::from_str(&skill_json()).unwrap();

        assert_eq!(parsed["schema_version"], "2");
        assert_eq!(parsed["kind"], "tool_help");
        assert_eq!(parsed["tool"], "vow");
        assert_eq!(parsed["default_format"], "json");
        assert!(parsed["references"]["grammar"].is_string());
        assert!(parsed["references"]["schemas"]["build_result"].is_string());
        assert!(parsed["command_details"]["build"]["options"].is_array());
        assert!(parsed["command_details"]["verify"]["options"].is_array());
        assert!(parsed["command_details"]["decl"]["options"].is_array());
        assert!(parsed["outputs"]["contracts_result"].is_object());

        let lang = &parsed["language"];
        assert!(lang["builtins"]["print_i64"].is_string());
        assert!(lang["builtins"]["print_str"].is_string());
        assert!(lang["types"].to_string().contains("String"));
        assert!(lang["types"].to_string().contains("Vec<T>"));
        assert!(lang["types"].to_string().contains("Option<T>"));
        assert!(lang["types"].to_string().contains("Result<T, E>"));
        assert!(lang["types"].to_string().contains("HashMap<K, V>"));
        assert!(lang["structs"].is_object());
        assert!(lang["enums"].is_object());
        assert!(lang["methods"].is_object());
        assert!(lang["match_expression"].is_object());
        assert!(lang["where_clauses"].is_string());
        assert!(lang["modules"].is_object());
        assert_eq!(
            lang["feature_status"]["target"]["module_level_vow_blocks"],
            "specified in docs but not parsed or represented in the AST"
        );
        assert_eq!(
            lang["feature_status"]["target"]["quantifiers"],
            "forall / exists are not yet in the lexer or parser"
        );
        assert_eq!(
            lang["feature_status"]["partial"]["refinement_type_predicates"],
            "parsed but semantically erased; use where clauses or function vows for verification"
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
    fn build_args_accept_max_k_step_flag() {
        let args = Args::try_parse_from(["vow", "build", "--max-k-step", "5", "main.vow"]).unwrap();
        match args.command {
            Some(Command::Build(build)) => assert_eq!(build.max_k_step, 5),
            other => panic!("expected build command, got {other:?}"),
        }
    }

    #[test]
    fn verify_args_accept_max_k_step_flag() {
        let args =
            Args::try_parse_from(["vow", "verify", "--max-k-step", "7", "main.vow"]).unwrap();
        match args.command {
            Some(Command::Verify(verify)) => assert_eq!(verify.max_k_step, 7),
            other => panic!("expected verify command, got {other:?}"),
        }
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
            status if esbmc_not_found(status) => {
                eprintln!("SKIP: verification not run (esbmc not found)");
            }
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
            status if esbmc_not_found(status) => {
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
    fn region_conflict_diagnostic_matches_external_schema() {
        // Spec §13.1: a RegionConflict diagnostic emitted on the build output
        // MUST serialise to {error_code, message, severity, span:{file,
        // offset, length}}. The error_code MUST be the string "RegionConflict".
        use vow_diag::{ErrorCode, SourceLocation};
        let diag = Diagnostic {
            severity: Severity::Error,
            code: ErrorCode::RegionConflict,
            message: "value `v` is placed in region(b) which closes before \
                      region(a), the container it is stored into; move the \
                      allocation to a wider scope"
                .to_string(),
            primary: SourceLocation {
                file: "f.vow".to_string(),
                byte_offset: 1024,
                byte_len: 3,
            },
            secondary: vec![],
            blame: vow_diag::Blame::None,
            hints: vec![],
        };
        let out = BuildOutput {
            status: BuildStatus::CompileFailed {
                message: "region error".to_string(),
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
        let d = &parsed["diagnostics"][0];
        assert_eq!(d["error_code"], "RegionConflict");
        assert_eq!(d["severity"], "error");
        assert_eq!(d["span"]["file"], "f.vow");
        assert_eq!(d["span"]["offset"], 1024);
        assert_eq!(d["span"]["length"], 3);
        // Exactly the keys spec §13.1 prescribes — no extra fields surface
        // beyond the optional `secondary`/`hints`/`blame` (omitted here).
        let d_obj = d.as_object().unwrap();
        for required in &["error_code", "message", "severity", "span"] {
            assert!(
                d_obj.contains_key(*required),
                "missing required key {required}"
            );
        }
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
        use vow_ir::{
            BasicBlock, BlockId, FuncId, Inst, InstData, InstId, Opcode, RegionId, RegionSummary,
            Ty,
        };
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
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(1),
                        origin: Span::new(0, 0),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let map = build_c_to_source_name_map(&func);
        assert_eq!(map.get("p0"), Some(&"x".to_string()));
        assert_eq!(map.get("p1"), Some(&"y".to_string()));
        assert_eq!(map.get("v0"), Some(&"x".to_string()));
        assert_eq!(map.get("v1"), Some(&"y".to_string()));
    }

    #[test]
    fn build_c_to_source_name_map_skips_unit_params() {
        use vow_ir::{
            BasicBlock, BlockId, FuncId, Inst, InstData, InstId, Opcode, RegionId, RegionSummary,
            Ty,
        };
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
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(2),
                        origin: Span::new(0, 0),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
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
        use vow_ir::{BasicBlock, BlockId, FuncId, RegionSummary, Ty};
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
            summary: RegionSummary::default(),
            source_file: String::new(),
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
            status if esbmc_not_found(status) => {
                eprintln!("SKIP: verification not run (esbmc not found)");
            }
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
            status if esbmc_not_found(status) => {
                eprintln!("SKIP: verification not run (esbmc not found)");
            }
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
            status if esbmc_not_found(status) => {
                eprintln!("SKIP: verification not run (esbmc not found)");
            }
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
            // Skipped: this fixture is a pure-arithmetic function with no struct
            // construction, so hitting this arm means something changed upstream.
            BuildStatus::Skipped => {
                panic!("unexpected Skipped status for requires_span fixture — investigate");
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
            status if esbmc_not_found(status) => {
                eprintln!("SKIP: esbmc not found");
            }
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
            status if esbmc_not_found(status) => {
                eprintln!("SKIP: esbmc not found");
            }
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

    // Exercises `run_verification_sync`'s threaded pool (`jobs > 1`). The public
    // `run_pipeline` / `run_verify_only` hardcode jobs=1, so without this test
    // the parallel code path is only covered via the CLI.
    #[test]
    fn verify_only_inner_runs_threaded_pool() {
        let dir = TempDir::new().unwrap();
        let src = r#"module MultiVow
fn a() -> i64 vow {
  ensures: result == 1
} {
  1
}
fn b() -> i64 vow {
  ensures: result == 2
} {
  2
}
fn c() -> i64 vow {
  ensures: result == 3
} {
  3
}
fn d() -> i64 vow {
  ensures: result == 4
} {
  4
}
fn main() -> i32 {
  0
}"#;
        let source = write_source(&dir, "multi.vow", src);
        let limits = VerifyLimits::default();
        // jobs=4 with 4 vowed functions forces the threaded path.
        let result =
            run_verify_only_inner(&source, true, &limits, 4, &SolverConfig::default_config());
        match &result.status {
            status if esbmc_not_found(status) => {
                eprintln!("SKIP: esbmc not found");
            }
            BuildStatus::Verified => {
                assert!(result.executable.is_none());
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

    // Locks in the lowest-index determinism guarantee on the threaded pool:
    // two functions are both provably wrong, but `fail_a` appears before
    // `fail_b` in source order, so it must be reported.
    #[test]
    fn verify_only_inner_reports_lowest_index_failure() {
        let dir = TempDir::new().unwrap();
        let src = r#"module FailDeterminism
fn ok_1() -> i64 vow {
  ensures: result == 1
} {
  1
}
fn fail_a() -> i64 vow {
  ensures: result == 99
} {
  1
}
fn ok_2() -> i64 vow {
  ensures: result == 2
} {
  2
}
fn fail_b() -> i64 vow {
  ensures: result == 99
} {
  2
}
fn main() -> i32 {
  0
}"#;
        let source = write_source(&dir, "fail_det.vow", src);
        let limits = VerifyLimits::default();
        let result =
            run_verify_only_inner(&source, true, &limits, 4, &SolverConfig::default_config());
        match &result.status {
            status if esbmc_not_found(status) => {
                eprintln!("SKIP: esbmc not found");
            }
            BuildStatus::VerifyFailed { function, .. } => {
                assert_eq!(
                    function, "fail_a",
                    "expected lowest-index failure fail_a, got {function}"
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
    fn vowed_struct_builder_is_skipped_not_failed() {
        let dir = TempDir::new().unwrap();
        let src = r#"module SkipDemo
struct Foo { x: i64, rgn: i64 }
fn make_foo(x: i64, rgn: i64) -> Foo vow {
  requires: rgn >= 0
} {
  Foo { x: x, rgn: rgn }
}
fn main() -> i32 {
  0
}"#;
        let source = write_source(&dir, "skip_demo.vow", src);
        let limits = VerifyLimits::default();
        let result =
            run_verify_only_inner(&source, true, &limits, 1, &SolverConfig::default_config());
        match &result.status {
            BuildStatus::Skipped => {
                assert!(
                    result
                        .diagnostics
                        .iter()
                        .any(|d| matches!(d.severity, vow_diag::Severity::Warning)
                            && d.message.contains("make_foo")),
                    "expected a Warning diagnostic naming `make_foo`, got: {:?}",
                    result.diagnostics
                );
            }
            status if esbmc_not_found(status) => {
                eprintln!("SKIP: esbmc not found");
            }
            BuildStatus::CompileFailed { message } => {
                panic!("unexpected compile failure: {message}");
            }
            other => panic!("expected Skipped for non-modelable vowed fn, got {other:?}"),
        }
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
                                region: RegionId::Root,
                            },
                            Inst {
                                id: InstId(1),
                                opcode: Opcode::Return,
                                ty: Ty::Unit,
                                args: vec![InstId(0)],
                                data: InstData::None,
                                origin: Span::new(0, 0),
                                region: RegionId::Root,
                            },
                        ],
                    }],
                    local_names: std::collections::HashMap::new(),
                    summary: RegionSummary::default(),
                    source_file: String::new(),
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
                                region: RegionId::Root,
                            },
                            Inst {
                                id: InstId(1),
                                opcode: Opcode::Call,
                                ty: Ty::I64,
                                args: vec![InstId(0)],
                                data: InstData::CallTarget(FuncId(0)),
                                origin: Span::new(100, 10),
                                region: RegionId::Root,
                            },
                            Inst {
                                id: InstId(2),
                                opcode: Opcode::Return,
                                ty: Ty::Unit,
                                args: vec![InstId(1)],
                                data: InstData::None,
                                origin: Span::new(0, 0),
                                region: RegionId::Root,
                            },
                        ],
                    }],
                    local_names: std::collections::HashMap::new(),
                    summary: RegionSummary::default(),
                    source_file: String::new(),
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
                                region: RegionId::Root,
                            },
                            Inst {
                                id: InstId(1),
                                opcode: Opcode::Call,
                                ty: Ty::I64,
                                args: vec![InstId(0)],
                                data: InstData::CallTarget(FuncId(0)),
                                origin: Span::new(200, 15),
                                region: RegionId::Root,
                            },
                            Inst {
                                id: InstId(2),
                                opcode: Opcode::Return,
                                ty: Ty::Unit,
                                args: vec![InstId(1)],
                                data: InstData::None,
                                origin: Span::new(0, 0),
                                region: RegionId::Root,
                            },
                        ],
                    }],
                    local_names: std::collections::HashMap::new(),
                    summary: RegionSummary::default(),
                    source_file: String::new(),
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
        assert!(!index.contains_key("caller_a"));
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
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(1),
                        origin: Span::new(0, 0),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::VowRequires,
                        ty: Ty::Unit,
                        args: vec![InstId(1)],
                        data: InstData::VowId(VowId(0)),
                        origin: Span::new(42, 6),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
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
    fn structured_counterexample_unsupported_op_sentinel() {
        use vow_ir::*;
        use vow_syntax::span::Span;

        let func = Function {
            id: FuncId(0),
            name: "uses_unsupported".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "some real vow".to_string(),
                blame: vow_diag::Blame::Callee,
                bindings: vec![],
                file: "test.vow".to_string(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![Inst {
                    id: InstId(0),
                    opcode: Opcode::Return,
                    ty: Ty::Unit,
                    args: vec![],
                    data: InstData::None,
                    origin: Span::new(0, 0),
                    region: RegionId::Root,
                }],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };

        let ce = vow_verify::Counterexample {
            description: "[Counterexample]".to_string(),
            vow_id: Some(UNSUPPORTED_OP_VOW_ID),
            values: vec![],
            block_visits: vec![],
            raw_output: String::new(),
        };

        let call_sites = std::collections::HashMap::new();
        let sce = build_structured_counterexample(&func, &ce, "test.vow", &call_sites);

        assert_eq!(sce.vow_id, UNSUPPORTED_OP_VOW_ID);
        assert!(
            sce.violation.contains("not supported for verification"),
            "expected unsupported-op message, got {:?}",
            sce.violation
        );
        assert_ne!(
            sce.violation, "[Counterexample]",
            "must not fall through to raw ESBMC line"
        );
        assert_eq!(sce.blame, "none");
        assert!(sce.source.is_none());
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
                    region: RegionId::Root,
                }],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
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
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::Return,
                        ty: Ty::Unit,
                        args: vec![InstId(0)],
                        data: InstData::None,
                        origin: Span::new(12, 1),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
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
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(11),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(0),
                        origin: Span::new(103, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(12),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(10), InstId(11)],
                        data: InstData::CallTarget(FuncId(0)),
                        origin: Span::new(95, 12),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(13),
                        opcode: Opcode::Return,
                        ty: Ty::Unit,
                        args: vec![InstId(12)],
                        data: InstData::None,
                        origin: Span::new(110, 1),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
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
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(1),
                        origin: Span::new(15, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::Return,
                        ty: Ty::Unit,
                        args: vec![InstId(0)],
                        data: InstData::None,
                        origin: Span::new(20, 1),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
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
                            region: RegionId::Root,
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
                            region: RegionId::Root,
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
                        region: RegionId::Root,
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
                        region: RegionId::Root,
                    }],
                },
            ],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
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
    fn discover_test_files_accepts_file_and_sorted_test_names() {
        let dir = TempDir::new().unwrap();
        let single = write_source(&dir, "plain.vow", "module Plain fn main() -> i32 { 0 }");
        assert_eq!(discover_test_files(&single), vec![single.clone()]);

        write_source(&dir, "notes.vow", "module Notes");
        let beta = write_source(&dir, "beta_test.vow", "module Beta");
        let alpha = write_source(&dir, "test_alpha.vow", "module Alpha");
        let files = discover_test_files(dir.path());
        assert_eq!(files, vec![beta, alpha]);
    }

    #[test]
    fn discover_test_files_recurses_into_subdirectories() {
        let dir = TempDir::new().unwrap();
        let top = write_source(&dir, "test_top.vow", "module Top");
        let nested_dir = dir.path().join("tests");
        std::fs::create_dir(&nested_dir).unwrap();
        let nested = nested_dir.join("test_nested.vow");
        std::fs::write(&nested, "module Nested").unwrap();
        // Non-test files in the subdir must be skipped, like at top level.
        std::fs::write(nested_dir.join("helper.vow"), "module Helper").unwrap();

        let files = discover_test_files(dir.path());
        // Lexicographic sort on the full path: "test_top.vow" < "tests/test_nested.vow"
        // because '_' (0x5F) sorts before 's' (0x73). Tests rely on stable ordering,
        // so anchor the expected sequence to the observed lexicographic rule.
        assert_eq!(files, vec![top, nested]);
    }

    #[test]
    fn discover_test_files_skips_symlinks() {
        // DirEntry::file_type() does not follow symlinks, so both symlinked
        // files and symlinked dirs are silently skipped. The self-hosted side
        // matches via __vow_fs_is_symlink. Verify the Rust behaviour stays
        // pinned so the two compilers can't drift.
        let dir = TempDir::new().unwrap();
        let real_test = write_source(&dir, "test_real.vow", "module Real");

        // Symlink to a regular .vow file outside the scan tree — must be skipped.
        let external = TempDir::new().unwrap();
        let external_target = external.path().join("test_external.vow");
        std::fs::write(&external_target, "module External").unwrap();
        let symlinked_file = dir.path().join("test_symlink.vow");
        std::os::unix::fs::symlink(&external_target, &symlinked_file).unwrap();

        // Symlink to a directory — its contents must not be recursed into.
        let external_dir = external.path().join("nested");
        std::fs::create_dir(&external_dir).unwrap();
        std::fs::write(
            external_dir.join("test_inside_symlink.vow"),
            "module Inside",
        )
        .unwrap();
        let symlinked_dir = dir.path().join("subdir_symlink");
        std::os::unix::fs::symlink(&external_dir, &symlinked_dir).unwrap();

        let files = discover_test_files(dir.path());
        assert_eq!(files, vec![real_test]);
    }

    #[test]
    fn count_contract_density_ignores_main_and_reports_tenths() {
        use vow_ir::{BasicBlock, BlockId, FuncId, RegionSummary, Ty, VowEntry, VowId};

        let make_func = |id, name: &str, vows| vow_ir::Function {
            id: FuncId(id),
            name: name.to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::Unit,
            effects: vec![],
            vows,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let vowed = VowEntry {
            id: VowId(0),
            description: "ensures: true".to_string(),
            blame: vow_diag::Blame::Callee,
            bindings: vec![],
            file: "test.vow".to_string(),
            offset: 0,
        };
        let module = vow_ir::Module {
            name: "Density".to_string(),
            functions: vec![
                make_func(0, "main", vec![vowed.clone()]),
                make_func(1, "with_vow", vec![vowed]),
                make_func(2, "without_vow", vec![]),
            ],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };

        let density = count_contract_density(&module);
        assert_eq!(density.functions_total, 2);
        assert_eq!(density.functions_with_vows, 1);
        assert_eq!(density.density_pct, 50.0);
    }

    #[test]
    fn resolve_verify_jobs_preserves_explicit_value() {
        assert_eq!(resolve_verify_jobs(Some(1)), 1);
        assert_eq!(resolve_verify_jobs(Some(3)), 3);
    }

    #[test]
    fn build_contracts_summary_counts_each_status_bucket() {
        let source = ContractSourceJson {
            file: "test.vow".to_string(),
            offset: 0,
        };
        let entry = |status: &str| ContractEntryJson {
            vow_id: 0,
            function: "f".to_string(),
            function_id: 0,
            kind: "ensures".to_string(),
            // Description must agree with the hard-coded `quality` below.
            description: "ensures: result == x".to_string(),
            blame: "callee".to_string(),
            source: source.clone(),
            status: status.to_string(),
            quality: "substantive".to_string(),
        };
        let summary = build_contracts_summary(&[
            entry("proven"),
            entry("proven-ir"),
            entry("failed"),
            entry("unknown"),
            entry("timeout"),
            entry("error"),
            entry("skipped"),
            entry("not-run"),
        ]);

        assert_eq!(summary.total, 8);
        assert_eq!(summary.proven, 2);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.unknown, 1);
        assert_eq!(summary.timeout, 1);
        assert_eq!(summary.error, 1);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.not_verified, 1);
        assert_eq!(summary.quality.substantive, 8);
        assert_eq!(summary.quality.weak, 0);
        assert_eq!(summary.quality.tautological, 0);
    }

    #[test]
    fn classify_contract_quality_flags_weak_result_bounds() {
        // The 354-contract trap: an ensures that only bounds result by a constant.
        assert_eq!(
            classify_contract_quality("ensures", "ensures result >= 0"),
            "weak"
        );
        assert_eq!(
            classify_contract_quality("ensures", "ensures result > 0"),
            "weak"
        );
        assert_eq!(
            classify_contract_quality("ensures", "ensures result <= 3"),
            "weak"
        );
        // result vs negative literal is still a constant bound.
        assert_eq!(
            classify_contract_quality("ensures", "ensures result >= -1"),
            "weak"
        );
        // Strict single-char operator path (`<`, not `<=`).
        assert_eq!(
            classify_contract_quality("ensures", "ensures result < 3"),
            "weak"
        );
    }

    #[test]
    fn classify_contract_quality_keeps_substantive_clauses() {
        // Equality, relational, inverse, totality, and call shapes are not weak.
        assert_eq!(
            classify_contract_quality("ensures", "ensures result == val * 4 + kind"),
            "substantive"
        );
        assert_eq!(
            classify_contract_quality("ensures", "ensures result != -1"),
            "substantive"
        );
        assert_eq!(
            classify_contract_quality("ensures", "ensures result >= a"),
            "substantive"
        );
        assert_eq!(
            classify_contract_quality("ensures", "ensures item_kind(result) == kind"),
            "substantive"
        );
        // A one-sided bound is a legitimate precondition, not a weak postcondition.
        assert_eq!(
            classify_contract_quality("requires", "requires v <= 255"),
            "substantive"
        );
        // A `false` predicate is a contradiction, not a tautology; the static
        // classifier leaves it substantive (vacuity detection is a follow-up).
        assert_eq!(
            classify_contract_quality("ensures", "ensures false"),
            "substantive"
        );
    }

    #[test]
    fn classify_contract_quality_flags_tautologies() {
        assert_eq!(
            classify_contract_quality("ensures", "ensures true"),
            "tautological"
        );
        // No reference to any program value — constant comparison.
        assert_eq!(
            classify_contract_quality("ensures", "ensures 0 >= 0"),
            "tautological"
        );
    }

    #[test]
    fn verification_limits_are_configurable_in_help() {
        let json = skill_json();
        assert!(
            json.contains("\"verification_defaults\""),
            "JSON must contain verification_defaults"
        );
        assert!(
            json.contains("\"--max-k-step\""),
            "JSON must contain --max-k-step"
        );
        assert!(
            !json.contains("\"--unwind\""),
            "JSON must not contain --unwind"
        );
        assert!(
            !json.contains("\"verification_limits\""),
            "JSON must not contain verification_limits"
        );

        let human = skill_human();
        assert!(
            human.contains("VERIFICATION DEFAULTS"),
            "Human help must contain VERIFICATION DEFAULTS"
        );
        assert!(
            human.contains("--max-k-step"),
            "Human help must contain --max-k-step"
        );
        assert!(
            human.contains("Incremental BMC"),
            "Human help must contain Incremental BMC"
        );
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

    #[test]
    fn compile_cache_only_enabled_for_unverified_unflagged_builds() {
        // Regression guard for the security gate: the compile-object cache may be enabled only when `--no-cache` is off and `--no-verify` is on. Any other combination must keep the cache disabled.
        assert!(
            !compile_cache_enabled(false, false),
            "default verified build (no_cache=false, no_verify=false): cache must be disabled"
        );
        assert!(
            !compile_cache_enabled(true, false),
            "no_cache=true with verification: cache must be disabled"
        );
        assert!(
            !compile_cache_enabled(true, true),
            "no_cache=true: cache must be disabled regardless of verification"
        );
        assert!(
            compile_cache_enabled(false, true),
            "--no-verify without --no-cache: cache must be enabled"
        );
    }
}
