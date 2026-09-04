//! Command-line surface for `vowc`: the clap argument types plus the pure
//! translation from those surface types into the compiler's domain types
//! (`BuildMode`, `TraceMode`, `SolverConfig`, verify-job count).
//!
//! The translation lives here, next to the argument types it consumes, rather
//! than smeared across the dispatcher in `main`. `solver_config` and
//! `resolve_verify_jobs` are pure — they return `Result` instead of calling
//! `std::process::exit`, so the failure paths are unit-testable. The
//! exit-on-error policy stays at the `main` call sites.

use std::path::PathBuf;

use clap::Parser;
use vow_codegen::{BuildMode, TraceMode};
use vow_verify::{DEFAULT_MAX_K_STEP, Encoding, Solver, SolverConfig, default_memlimit_mb};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ModeArg {
    Debug,
    Release,
    Profile,
    Sanitize,
}

impl ModeArg {
    /// Translate the CLI build mode into the codegen `BuildMode`.
    pub fn to_build_mode(self) -> BuildMode {
        match self {
            ModeArg::Debug => BuildMode::Debug,
            ModeArg::Release => BuildMode::Release,
            ModeArg::Profile => BuildMode::Profile,
            ModeArg::Sanitize => BuildMode::Sanitize,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum TraceArg {
    Off,
    Calls,
    Full,
}

impl TraceArg {
    /// Translate the CLI trace mode into the codegen `TraceMode`.
    pub fn to_trace_mode(self) -> TraceMode {
        match self {
            TraceArg::Off => TraceMode::Off,
            TraceArg::Calls => TraceMode::Calls,
            TraceArg::Full => TraceMode::Full,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SolverArg {
    Boolector,
    Z3,
    Bitwuzla,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum EncodingArg {
    Bv,
    Ir,
    Auto,
}

/// Build and validate a `SolverConfig` from the CLI solver/encoding/timeout
/// flags. Returns the validation error message on an incompatible combination
/// (e.g. `--encoding ir --solver boolector`); the caller is responsible for
/// reporting it and exiting.
pub fn solver_config(
    solver: SolverArg,
    encoding: EncodingArg,
    timeout: Option<u32>,
) -> Result<SolverConfig, String> {
    let solver = match solver {
        SolverArg::Boolector => Solver::Boolector,
        SolverArg::Z3 => Solver::Z3,
        SolverArg::Bitwuzla => Solver::Bitwuzla,
        SolverArg::Auto => Solver::Auto,
    };
    let encoding = match encoding {
        EncodingArg::Bv => Encoding::Bv,
        EncodingArg::Ir => Encoding::Ir,
        EncodingArg::Auto => Encoding::Auto,
    };
    let config = SolverConfig {
        solver,
        encoding,
        timeout_secs: timeout,
        memlimit_mb: default_memlimit_mb(),
    };
    config.validate()?;
    Ok(config)
}

/// Resolve the `--verify-jobs` flag into a worker count. An explicit `0` is
/// rejected; `None` defaults to half the available parallelism, clamped to at
/// least 1. Returns the error message for the caller to report and exit on.
pub fn resolve_verify_jobs(opt: Option<u32>) -> Result<usize, String> {
    match opt {
        Some(0) => Err("--verify-jobs must be >= 1".to_string()),
        Some(n) => Ok(n as usize),
        None => {
            let n = std::thread::available_parallelism()
                .map(|p| p.get())
                .unwrap_or(1);
            Ok((n / 2).max(1))
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "vow",
    about = "Vow compiler",
    disable_help_flag = true,
    args_conflicts_with_subcommands = true
)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,

    pub source: Option<PathBuf>,
    #[arg(short = 'o', long)]
    pub output: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "release")]
    pub mode: ModeArg,
    #[arg(long)]
    pub no_verify: bool,
    #[arg(long)]
    pub dump_ir: bool,
    #[arg(long, value_enum, default_value = "off")]
    pub debug_trace: TraceArg,
    #[arg(long)]
    pub no_cache: bool,
    #[arg(long, default_value_t = DEFAULT_MAX_K_STEP)]
    pub max_k_step: u32,
    #[arg(long, value_enum, default_value = "auto")]
    pub solver: SolverArg,
    #[arg(long, value_enum, default_value = "auto")]
    pub encoding: EncodingArg,
    #[arg(long)]
    pub timeout: Option<u32>,
    #[arg(long)]
    pub verify_jobs: Option<u32>,
    /// Differential-test counterexamples against runtime semantics (issue #335).
    #[arg(long)]
    pub replay_cex: bool,
    #[arg(long)]
    pub perfetto: Option<PathBuf>,
    #[arg(long)]
    pub help: bool,
    #[arg(long)]
    pub human: bool,
}

#[derive(clap::Subcommand, Debug)]
pub enum Command {
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
    /// Report per-function complexity metrics as JSON
    Complexity(ComplexityArgs),
}

#[derive(clap::Args, Debug)]
#[command(disable_help_flag = true)]
pub struct BuildArgs {
    pub source: Option<PathBuf>,
    #[arg(short = 'o', long)]
    pub output: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "release")]
    pub mode: ModeArg,
    #[arg(long)]
    pub no_verify: bool,
    #[arg(long)]
    pub dump_ir: bool,
    #[arg(long, value_enum, default_value = "off")]
    pub debug_trace: TraceArg,
    #[arg(long)]
    pub no_cache: bool,
    #[arg(long, default_value_t = DEFAULT_MAX_K_STEP)]
    pub max_k_step: u32,
    #[arg(long, value_enum, default_value = "auto")]
    pub solver: SolverArg,
    #[arg(long, value_enum, default_value = "auto")]
    pub encoding: EncodingArg,
    #[arg(long)]
    pub timeout: Option<u32>,
    #[arg(long)]
    pub verify_jobs: Option<u32>,
    /// Differential-test counterexamples against runtime semantics (issue #335).
    #[arg(long)]
    pub replay_cex: bool,
    #[arg(long)]
    pub perfetto: Option<PathBuf>,
    #[arg(long)]
    pub help: bool,
    #[arg(long)]
    pub human: bool,
}

#[derive(clap::Args, Debug)]
#[command(disable_help_flag = true)]
pub struct VerifyArgs {
    pub source: Option<PathBuf>,
    #[arg(long)]
    pub help: bool,
    #[arg(long)]
    pub human: bool,
    #[arg(long)]
    pub no_cache: bool,
    #[arg(long, default_value_t = DEFAULT_MAX_K_STEP)]
    pub max_k_step: u32,
    #[arg(long, value_enum, default_value = "auto")]
    pub solver: SolverArg,
    #[arg(long, value_enum, default_value = "auto")]
    pub encoding: EncodingArg,
    #[arg(long)]
    pub timeout: Option<u32>,
    #[arg(long)]
    pub verify_jobs: Option<u32>,
    #[arg(long)]
    pub perfetto: Option<PathBuf>,
    /// Differential-test counterexamples against runtime semantics (issue #335).
    #[arg(long)]
    pub replay_cex: bool,
}

#[derive(clap::Args, Debug)]
#[command(disable_help_flag = true)]
pub struct TestArgs {
    /// Directory to scan for test files, or a single .vow file
    pub path: Option<PathBuf>,
    /// Run ESBMC verification on test files (off by default)
    #[arg(long)]
    pub verify: bool,
    /// Only run tests whose name contains this substring
    #[arg(long)]
    pub filter: Option<String>,
    /// Resolve `use` declarations against this directory instead of each
    /// test file's parent. Use when running a single test file that lives
    /// in a subdirectory: `vow test compiler/tests/test_x.vow --module-root compiler`.
    #[arg(long)]
    pub module_root: Option<PathBuf>,
    /// Build mode (debug enables runtime vow checks)
    #[arg(long, value_enum, default_value = "debug")]
    pub mode: ModeArg,
    /// Per-test execution timeout in milliseconds
    #[arg(long, default_value = "30000")]
    pub timeout: u64,
    /// ESBMC max k-induction step (only with --verify)
    #[arg(long, default_value_t = DEFAULT_MAX_K_STEP)]
    pub max_k_step: u32,
    #[arg(long)]
    pub verify_jobs: Option<u32>,
    #[arg(long)]
    pub help: bool,
    #[arg(long)]
    pub human: bool,
}

#[derive(clap::Args, Debug)]
#[command(disable_help_flag = true)]
pub struct DeclArgs {
    pub source: Option<PathBuf>,
    #[arg(short = 'o', long)]
    pub output: Option<PathBuf>,
    #[arg(long)]
    pub help: bool,
    #[arg(long)]
    pub human: bool,
}

#[derive(clap::Args, Debug)]
#[command(disable_help_flag = true)]
pub struct ComplexityArgs {
    pub source: Option<PathBuf>,
    #[arg(long)]
    pub cog_anchor: Option<i64>,
    #[arg(long)]
    pub nloc_anchor: Option<i64>,
    #[arg(long)]
    pub max_score: Option<i64>,
    #[arg(long)]
    pub max_cognitive: Option<i64>,
    #[arg(long)]
    pub max_cyclomatic: Option<i64>,
    #[arg(long)]
    pub help: bool,
    #[arg(long)]
    pub human: bool,
}

#[derive(clap::Args, Debug)]
#[command(disable_help_flag = true)]
pub struct ContractsArgs {
    pub source: Option<PathBuf>,
    #[arg(long)]
    pub verify: bool,
    #[arg(long)]
    pub no_cache: bool,
    #[arg(long)]
    pub max_k_step: Option<u32>,
    #[arg(long, value_enum, default_value = "auto")]
    pub solver: SolverArg,
    #[arg(long, value_enum, default_value = "auto")]
    pub encoding: EncodingArg,
    #[arg(long)]
    pub timeout: Option<u32>,
    /// Accepted for CLI parity with build/verify/test; ignored because
    /// `update_contract_statuses` has no pool wiring yet (see #175 follow-ups).
    #[arg(long)]
    pub verify_jobs: Option<u32>,
    #[arg(long)]
    pub help: bool,
    #[arg(long)]
    pub human: bool,
}

#[derive(clap::Args, Debug)]
#[command(disable_help_flag = true)]
pub struct SkillArgs {
    #[command(subcommand)]
    pub action: Option<SkillAction>,
    #[arg(long)]
    pub help: bool,
    #[arg(long)]
    pub human: bool,
}

#[derive(clap::Subcommand, Debug)]
pub enum SkillAction {
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
pub struct MutantsArgs {
    /// All remaining arguments forwarded verbatim
    pub args: Vec<String>,
    // `help` and `human` are absorbed by clap so flags like `--help` don't
    // surface as parse errors; the handler below ignores them and prints
    // a fixed redirect to the self-hosted compiler regardless.
    #[arg(long)]
    pub help: bool,
    #[arg(long)]
    pub human: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_verify_jobs_rejects_zero() {
        assert_eq!(
            resolve_verify_jobs(Some(0)),
            Err("--verify-jobs must be >= 1".to_string())
        );
    }

    #[test]
    fn resolve_verify_jobs_uses_explicit_value() {
        assert_eq!(resolve_verify_jobs(Some(5)), Ok(5));
    }

    #[test]
    fn resolve_verify_jobs_default_is_at_least_one() {
        // Independent property: the auto-derived worker count is always usable.
        let jobs = resolve_verify_jobs(None).expect("None must resolve to a default");
        assert!(jobs >= 1, "default job count must be >= 1, got {jobs}");
    }

    #[test]
    fn solver_config_rejects_ir_encoding_with_boolector() {
        // Boolector has no integer mode; ir encoding requires z3. The exact
        // message is the verifier's documented rejection (solver_strategy.rs).
        // SolverConfig has no PartialEq, so assert on the error arm directly.
        let err = solver_config(SolverArg::Boolector, EncodingArg::Ir, None)
            .expect_err("boolector + ir encoding must be rejected");
        assert_eq!(
            err,
            "--encoding ir requires --solver z3 (Boolector does not support integer mode)"
        );
    }

    #[test]
    fn solver_config_accepts_z3_ir_and_carries_timeout() {
        let config = solver_config(SolverArg::Z3, EncodingArg::Ir, Some(10))
            .expect("z3 + ir is a valid combination");
        assert_eq!(config.solver, Solver::Z3);
        assert_eq!(config.encoding, Encoding::Ir);
        assert_eq!(config.timeout_secs, Some(10));
        assert_eq!(config.memlimit_mb, default_memlimit_mb());
    }

    #[test]
    fn solver_config_accepts_auto_defaults() {
        let config =
            solver_config(SolverArg::Auto, EncodingArg::Auto, None).expect("auto/auto is valid");
        assert_eq!(config.solver, Solver::Auto);
        assert_eq!(config.encoding, Encoding::Auto);
        assert_eq!(config.timeout_secs, None);
    }

    #[test]
    fn solver_config_accepts_bitwuzla_and_bv_encoding() {
        let config = solver_config(SolverArg::Bitwuzla, EncodingArg::Bv, None)
            .expect("bitwuzla + bv is a valid combination");
        assert_eq!(config.solver, Solver::Bitwuzla);
        assert_eq!(config.encoding, Encoding::Bv);
    }

    #[test]
    fn to_build_mode_translates_every_variant() {
        assert_eq!(ModeArg::Debug.to_build_mode(), BuildMode::Debug);
        assert_eq!(ModeArg::Release.to_build_mode(), BuildMode::Release);
        assert_eq!(ModeArg::Profile.to_build_mode(), BuildMode::Profile);
        assert_eq!(ModeArg::Sanitize.to_build_mode(), BuildMode::Sanitize);
    }

    #[test]
    fn to_trace_mode_translates_every_variant() {
        assert_eq!(TraceArg::Off.to_trace_mode(), TraceMode::Off);
        assert_eq!(TraceArg::Calls.to_trace_mode(), TraceMode::Calls);
        assert_eq!(TraceArg::Full.to_trace_mode(), TraceMode::Full);
    }
}
