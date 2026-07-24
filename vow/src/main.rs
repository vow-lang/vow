mod cache;
mod cex_eval;
mod complexity;
mod contract_quality;
mod contracts;
mod counterexample;
mod frontend;
mod module_loader;
mod perfetto;
mod replay;
mod report;
mod skill;
mod test_runner;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;

use clap::Parser;
use vow_codegen::cranelift_backend::CraneliftBackend;
use vow_codegen::linker::{find_runtime_lib, find_shim_lib, link};
use vow_codegen::{Backend, BuildMode, TraceMode};
use vow_diag::{Diagnostic, DiagnosticEmitter, HumanEmitter, Severity};
use vow_verify::{
    ConstantValue, DEFAULT_ESBMC_MEMLIMIT_MB, DEFAULT_MAX_K_STEP, Encoding, Solver, SolverConfig,
    VerificationResult, VerifyLimits, VerifyRequest, detect_constant_functions,
    emit_verify_c_source, find_esbmc, non_modelable_reason, run_with_fallback, verify,
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
    /// Differential-test counterexamples against runtime semantics (issue #335).
    #[arg(long)]
    replay_cex: bool,
    #[arg(long)]
    perfetto: Option<PathBuf>,
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
    /// Report per-function complexity metrics as JSON
    Complexity(ComplexityArgs),
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
    /// Differential-test counterexamples against runtime semantics (issue #335).
    #[arg(long)]
    replay_cex: bool,
    #[arg(long)]
    perfetto: Option<PathBuf>,
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
    #[arg(long)]
    perfetto: Option<PathBuf>,
    /// Differential-test counterexamples against runtime semantics (issue #335).
    #[arg(long)]
    replay_cex: bool,
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
struct ComplexityArgs {
    source: Option<PathBuf>,
    #[arg(long)]
    cog_anchor: Option<i64>,
    #[arg(long)]
    nloc_anchor: Option<i64>,
    #[arg(long)]
    max_score: Option<i64>,
    #[arg(long)]
    max_cognitive: Option<i64>,
    #[arg(long)]
    max_cyclomatic: Option<i64>,
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
    /// `--replay-cex` outcome: `"confirmed"`, `"diverged"`, or `"skipped"`.
    /// `None` unless replay was requested. See `replay` (issue #335).
    pub replay: Option<String>,
    /// Reason string for a `diverged` or `skipped` replay; `None` otherwise.
    pub replay_reason: Option<String>,
    /// Raw ESBMC counterexample assignments (`p0`, `v3`, …), before
    /// name-mapping. Consumed by replay harness generation; never serialized.
    pub replay_raw_values: Vec<(String, String)>,
    /// Full raw ESBMC counterexample text. ESBMC reports aggregate (Vec/struct)
    /// values as composite literals `{ .len=N, .data={...} }` that the scalar
    /// assignment parser truncates, so vec reconstruction re-parses this.
    /// Never serialized.
    pub replay_raw_output: String,
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
    /// Named `NotRun` (not `Skipped`) to avoid colliding with `SkippedNonModelable`,
    /// which has the opposite exit code.
    NotRun,
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
// Counterexample construction
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Frontend (parse → module load → type check → IR lower)
// ---------------------------------------------------------------------------

fn emit_frontend_diagnostics(
    diagnostics: &[Diagnostic],
    emitter: &mut dyn DiagnosticEmitter,
) -> std::io::Result<()> {
    for diagnostic in diagnostics {
        emitter.try_emit(diagnostic)?;
    }
    emitter.try_finish()
}

fn emit_frontend_diagnostics_to_stderr(diagnostics: &[Diagnostic]) -> std::io::Result<()> {
    let mut stderr_emit = HumanEmitter::new(Box::new(std::io::stderr()));
    emit_frontend_diagnostics(diagnostics, &mut stderr_emit)
}

fn is_broken_pipe(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::BrokenPipe
}

fn write_stderr_best_effort(args: std::fmt::Arguments<'_>) {
    use std::io::Write;

    let _ = writeln!(std::io::stderr(), "{args}");
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

fn frontend_diagnostic_emission_error_to_output(
    error: std::io::Error,
    diagnostics: Vec<Diagnostic>,
) -> BuildOutput {
    BuildOutput {
        status: BuildStatus::CompileFailed {
            message: format!("failed to emit frontend diagnostics: {error}"),
        },
        executable: None,
        diagnostics,
        counterexamples: vec![],
        verify_status: None,
        verify_message: None,
    }
}

fn emit_frontend_result(
    frontend: Result<FrontendBundle, FrontendError>,
    emitter: &mut dyn DiagnosticEmitter,
) -> Result<FrontendBundle, Box<BuildOutput>> {
    match frontend {
        Ok(bundle) => match emit_frontend_diagnostics(bundle.diagnostics(), emitter) {
            Ok(()) => Ok(bundle),
            Err(error) if is_broken_pipe(&error) => Ok(bundle),
            Err(error) => {
                let (diagnostics, _, _) = bundle.into_parts();
                Err(Box::new(frontend_diagnostic_emission_error_to_output(
                    error,
                    diagnostics,
                )))
            }
        },
        Err(frontend_error) => {
            match emit_frontend_diagnostics(frontend_error.diagnostics(), emitter) {
                Ok(()) => Err(Box::new(frontend_error_to_output(frontend_error))),
                Err(error) if is_broken_pipe(&error) => {
                    Err(Box::new(frontend_error_to_output(frontend_error)))
                }
                Err(error) => {
                    let diagnostics = frontend_error.into_diagnostics();
                    Err(Box::new(frontend_diagnostic_emission_error_to_output(
                        error,
                        diagnostics,
                    )))
                }
            }
        }
    }
}

fn compile_frontend(
    source: &Path,
    prof: Option<&perfetto::Profiler>,
) -> Result<FrontendBundle, Box<BuildOutput>> {
    compile_frontend_with_root(source, None, prof)
}

/// Same as `compile_frontend`, but resolves `use` declarations against
/// `module_root` rather than the entry file's parent directory. Used by
/// `vowc test` so tests in `compiler/tests/` can `use` sibling compiler modules.
pub(crate) fn compile_frontend_with_root(
    source: &Path,
    module_root: Option<&Path>,
    prof: Option<&perfetto::Profiler>,
) -> Result<FrontendBundle, Box<BuildOutput>> {
    let frontend = prepare_frontend_with_root(source, module_root, FrontendGoal::LoweredIr, prof);
    let mut stderr_emit = HumanEmitter::new(Box::new(std::io::stderr()));
    emit_frontend_result(frontend, &mut stderr_emit)
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
    call_site_index: &std::collections::HashMap<String, Vec<counterexample::CallSiteInfo>>,
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
                        callee_precondition: ce.callee_precondition,
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
        verify(&VerifyRequest {
            const_fns: Some(const_fns),
            config: Some(&func_config),
            ..VerifyRequest::new(func, ir_module, limits)
        })
    };

    match result {
        VerificationResult::Failed(ce) => {
            let sce = counterexample::build_structured_counterexample_with_module(
                func,
                Some(ir_module),
                &ce,
                file,
                call_site_index,
            );
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
/// Record one per-function ESBMC proof as a span on its worker thread track,
/// plus a flow arrow from the verification handoff origin to the proof start.
/// No-op when profiling is off.
fn record_proof_span(
    prof: Option<&perfetto::Profiler>,
    verify_start: u64,
    idx: usize,
    name: &str,
    tid: u64,
    start_us: u64,
) {
    let Some(p) = prof else { return };
    p.flow(
        "verify->esbmc",
        perfetto::PID_COMPILER,
        perfetto::TID_VERIFY_DRIVER,
        verify_start,
        idx as u64,
        perfetto::FlowEdge::Start,
    );
    p.flow(
        "verify->esbmc",
        perfetto::PID_COMPILER,
        tid,
        start_us,
        idx as u64,
        perfetto::FlowEdge::End,
    );
    p.span(
        &format!("esbmc:{name}"),
        perfetto::PID_COMPILER,
        tid,
        start_us,
        p.now_us().saturating_sub(start_us),
        vec![("function".to_string(), name.to_string())],
    );
}

#[allow(clippy::too_many_arguments)]
fn run_verification_sync(
    ir_module: &vow_ir::Module,
    file: &str,
    call_site_index: &std::collections::HashMap<String, Vec<counterexample::CallSiteInfo>>,
    verify_cache: Option<&VerifyCache>,
    limits: &VerifyLimits,
    jobs: usize,
    config: &SolverConfig,
    prof: Option<&perfetto::Profiler>,
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

    // Handoff origin: a flow arrow runs from here to each per-function proof.
    let verify_start = prof.map(|p| p.now_us()).unwrap_or(0);

    let jobs = jobs.max(1).min(vowed.len());
    if jobs == 1 {
        let mut skipped = Vec::new();
        for (idx, func) in vowed.iter().enumerate() {
            let proof_start = prof.map(|p| p.now_us()).unwrap_or(0);
            let result = verify_one_function(
                func,
                ir_module,
                &const_fns,
                file,
                call_site_index,
                verify_cache,
                limits,
                config,
            );
            record_proof_span(
                prof,
                verify_start,
                idx,
                &func.name,
                perfetto::TID_VERIFY_DRIVER,
                proof_start,
            );
            match result {
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
        for w in 0..jobs {
            let next = &next;
            let stop = &stop;
            let halts = &halts;
            let skipped_acc = &skipped_acc;
            let vowed = &vowed;
            let const_fns = &const_fns;
            let worker_tid = perfetto::TID_WORKER_BASE + w as u64;
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
                    let proof_start = prof.map(|p| p.now_us()).unwrap_or(0);
                    let result = verify_one_function(
                        vowed[idx],
                        ir_module,
                        const_fns,
                        file,
                        call_site_index,
                        verify_cache,
                        limits,
                        config,
                    );
                    record_proof_span(
                        prof,
                        verify_start,
                        idx,
                        &vowed[idx].name,
                        worker_tid,
                        proof_start,
                    );
                    match result {
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
        VerifyOutcome::NotRun => (BuildStatus::Unverified, vec![], None, None),
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
    run_verify_only_inner(
        source,
        false,
        &limits,
        1,
        &SolverConfig::default_config(),
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_verify_only_inner(
    source: &Path,
    no_cache: bool,
    limits: &VerifyLimits,
    jobs: usize,
    config: &SolverConfig,
    prof: Option<&perfetto::Profiler>,
) -> BuildOutput {
    let frontend = match compile_frontend(source, prof) {
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
    let call_site_index = counterexample::build_call_site_index(ir_module, &file);
    let (outcome, skipped) = run_verification_sync(
        ir_module,
        &file,
        &call_site_index,
        verify_cache.as_ref(),
        limits,
        jobs,
        config,
        prof,
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
        None,
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
    prof: Option<&perfetto::Profiler>,
) -> BuildOutput {
    let frontend = match compile_frontend(source, prof) {
        Ok(f) => f,
        Err(output) => return *output,
    };

    run_pipeline_from_frontend(
        frontend, source, output, mode, no_verify, dump_ir, trace, no_cache, limits, jobs, config,
        prof,
    )
}

/// Fail-closed result for a panicked verifier worker (`join()` → `Err`). A
/// verifier crash leaves verification in an unknown state, so the build must
/// report `VerifyFailed` (exit 1) and withhold the executable — never the silent
/// `Unverified`/exit-0 of the old `.unwrap_or((Skipped, _))` fallback (#413).
/// The linked binary is removed so no executable masquerades as built.
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
pub(crate) fn run_pipeline_from_frontend(
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
    prof: Option<&perfetto::Profiler>,
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

    // Upfront ESBMC check: abort before codegen if verification is requested but ESBMC is missing.
    // The test-only VOW_TEST_VERIFIER_PANIC hook lives in the verify worker thread below, so on a
    // machine without ESBMC this early return would short-circuit before the worker is ever spawned
    // — making the panic regression test (#413) depend on an installed verifier. When the hook is
    // armed, skip the early return so the worker runs and panics, exercising the real JoinError
    // path. The env var is never set in production, and `var_os` is only reached once ESBMC is
    // already known to be absent, so the happy path is unchanged.
    if !no_verify && find_esbmc().is_none() && std::env::var_os("VOW_TEST_VERIFIER_PANIC").is_none()
    {
        return verify_outcome_to_output(VerifyOutcome::ToolNotFound, all_diagnostics, None);
    }

    // Spawn verification thread
    let module_for_verify = Arc::clone(&ir_module);
    let file_for_verify = source.to_string_lossy().to_string();
    let call_site_index = counterexample::build_call_site_index(&ir_module, &file_for_verify);
    let verify_cache = if no_cache || no_verify {
        None
    } else {
        VerifyCache::new()
    };
    let verify_limits = *limits;
    let verify_config = *config;
    // Owned clone (Arc-backed) moved into the verify thread so it can record
    // proof spans on its own track; the synchronous side keeps `prof`.
    let verify_prof = prof.cloned();
    let verify_handle = thread::spawn(move || -> (VerifyOutcome, Vec<SkippedFunction>) {
        let driver_start = verify_prof.as_ref().map(|p| p.now_us()).unwrap_or(0);
        let result = if no_verify {
            (VerifyOutcome::NotRun, Vec::new())
        } else {
            // Test-only fault injection: simulate a verifier-worker crash so the
            // fail-closed JoinError path (#413) is exercised end-to-end. Guarded
            // by an env var that is never set in production; one lookup per build.
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
                verify_prof.as_ref(),
            )
        };
        if let Some(p) = verify_prof.as_ref() {
            p.span(
                "verification",
                perfetto::PID_COMPILER,
                perfetto::TID_VERIFY_DRIVER,
                driver_start,
                p.now_us().saturating_sub(driver_start),
                vec![],
            );
        }
        result
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
        // Codegen was skipped (object cache hit). Mark it so a loaded trace's
        // empty codegen region is not mistaken for missing instrumentation.
        if let Some(p) = prof {
            let t = p.now_us();
            p.span(
                "codegen:cache-hit",
                perfetto::PID_COMPILER,
                perfetto::TID_MAIN,
                t,
                0,
                vec![],
            );
        }
        let link_start = prof.map(|p| p.now_us()).unwrap_or(0);
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
        if let Some(p) = prof {
            p.span(
                "link",
                perfetto::PID_COMPILER,
                perfetto::TID_MAIN,
                link_start,
                p.now_us().saturating_sub(link_start),
                vec![],
            );
        }
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
    let codegen_start = prof.map(|p| p.now_us()).unwrap_or(0);
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

    if let Some(p) = prof {
        p.span(
            "codegen",
            perfetto::PID_COMPILER,
            perfetto::TID_MAIN,
            codegen_start,
            p.now_us().saturating_sub(codegen_start),
            vec![],
        );
    }

    // Store in cache
    if let Some(ref cc) = compile_cache
        && let Some(ref key) = cache_key
    {
        cc.store(key, &obj_path);
    }

    let link_start = prof.map(|p| p.now_us()).unwrap_or(0);
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
    if let Some(p) = prof {
        p.span(
            "link",
            perfetto::PID_COMPILER,
            perfetto::TID_MAIN,
            link_start,
            p.now_us().saturating_sub(link_start),
            vec![],
        );
    }

    let (verify_outcome, skipped) = match verify_handle.join() {
        Ok(result) => result,
        Err(_) => return verifier_panicked_output(all_diagnostics, exe_path),
    };
    verify_outcome_to_output_with_skipped(verify_outcome, all_diagnostics, &skipped, exe_path)
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

/// Lifecycle for a `--perfetto` profiling run: holds the profiler, starts a
/// background resource sampler, and writes the gzipped trace on `finish()`. The
/// trace is a pure side artifact — it is produced only after the pipeline has
/// fully returned, so it cannot perturb codegen, the build JSON, or any cache.
struct PerfettoSession {
    prof: perfetto::Profiler,
    sampler: Option<perfetto::ResourceSampler>,
    path: PathBuf,
}

impl PerfettoSession {
    fn start(path: &Path) -> Self {
        let prof = perfetto::Profiler::new();
        let sampler = Some(prof.start_sampler(std::time::Duration::from_millis(25)));
        PerfettoSession {
            prof,
            sampler,
            path: path.to_path_buf(),
        }
    }

    fn finish(mut self) {
        if let Some(s) = self.sampler.take() {
            s.stop();
        }
        if let Err(e) = perfetto::write_trace_gz(&self.prof.snapshot(), &self.path) {
            eprintln!(
                "warning: failed to write perfetto trace to {}: {e}",
                self.path.display()
            );
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
    replay_cex: bool,
    perfetto_path: Option<&Path>,
) {
    let session = perfetto_path.map(PerfettoSession::start);
    let mut result = run_pipeline_inner(
        source,
        output,
        mode,
        no_verify,
        dump_ir,
        trace,
        no_cache,
        limits,
        jobs,
        config,
        session.as_ref().map(|s| &s.prof),
    );
    if replay_cex && !no_verify && !dump_ir {
        replay::run_replay_cex(source, &mut result);
    }
    if let Some(session) = session {
        session.finish();
    }
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
    let mut stderr_broken_pipe = false;
    let frontend = match prepare_frontend(source, FrontendGoal::MergedAst) {
        Ok(bundle) => {
            match emit_frontend_diagnostics_to_stderr(bundle.diagnostics()) {
                Ok(()) => {}
                Err(error) if is_broken_pipe(&error) => stderr_broken_pipe = true,
                Err(error) => {
                    write_stderr_best_effort(format_args!(
                        "vow decl: failed to emit frontend diagnostics: {error}"
                    ));
                    std::process::exit(1);
                }
            }
            bundle
        }
        Err(error) => {
            match emit_frontend_diagnostics_to_stderr(error.diagnostics()) {
                Ok(()) => {
                    write_stderr_best_effort(format_args!("vow decl: {}", error.failure_message()))
                }
                Err(emission_error) if is_broken_pipe(&emission_error) => {}
                Err(emission_error) => write_stderr_best_effort(format_args!(
                    "vow decl: {}; failed to emit frontend diagnostics: {emission_error}",
                    error.failure_message()
                )),
            }
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
        if !stderr_broken_pipe {
            write_stderr_best_effort(format_args!("vow decl: {e}"));
        }
        std::process::exit(1);
    }
    if !stderr_broken_pipe {
        write_stderr_best_effort(format_args!("wrote {}", out_path.display()));
    }
}

fn run_verify_command(
    source: &Path,
    no_cache: bool,
    limits: &VerifyLimits,
    jobs: usize,
    config: &SolverConfig,
    replay_cex: bool,
    perfetto_path: Option<&Path>,
) {
    let session = perfetto_path.map(PerfettoSession::start);
    let mut result = run_verify_only_inner(
        source,
        no_cache,
        limits,
        jobs,
        config,
        session.as_ref().map(|s| &s.prof),
    );
    if replay_cex {
        replay::run_replay_cex(source, &mut result);
    }
    if let Some(session) = session {
        session.finish();
    }
    result.emit_json();
    if matches!(
        &result.status,
        BuildStatus::CompileFailed { .. } | BuildStatus::VerifyFailed { .. } | BuildStatus::Skipped
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
                    println!("{}", skill::human());
                } else {
                    println!("{}", skill::json());
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
                skill::maybe_auto_install(&cwd);
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
                b.replay_cex,
                b.perfetto.as_deref(),
            );
        }
        Some(Command::Verify(v)) => {
            if v.help {
                if v.human {
                    println!("{}", skill::human());
                } else {
                    println!("{}", skill::json());
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
            run_verify_command(
                &source,
                v.no_cache,
                &limits,
                jobs,
                &config,
                v.replay_cex,
                v.perfetto.as_deref(),
            );
        }
        Some(Command::Test(t)) => {
            if t.help {
                if t.human {
                    println!("{}", skill::human());
                } else {
                    println!("{}", skill::json());
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
            test_runner::run_test_command(
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
                    println!("{}", skill::human());
                } else {
                    println!("{}", skill::json());
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
                    println!("{}", skill::human());
                } else {
                    println!("{}", skill::json());
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
            contracts::run_contracts_command(&source, c.verify, c.no_cache, &limits, &config);
        }
        Some(Command::Skill(s)) => {
            if s.help {
                if s.human {
                    println!("{}", skill::human());
                } else {
                    println!("{}", skill::json());
                }
                return;
            }
            match s.action {
                Some(SkillAction::Install { local, global }) => {
                    skill::install(local, global);
                }
                Some(SkillAction::Print { bundle: true }) => {
                    println!("{}", skill::bundle_markdown());
                }
                Some(SkillAction::Print { bundle: false }) => {
                    println!("{}", skill::entrypoint_markdown());
                }
                None => {
                    println!("{}", skill::entrypoint_markdown());
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
        Some(Command::Complexity(c)) => {
            if c.help {
                if c.human {
                    println!("{}", skill::human());
                } else {
                    println!("{}", skill::json());
                }
                return;
            }
            let source = match c.source {
                Some(s) => s,
                None => {
                    eprintln!("vow complexity: source file required (try --help)");
                    std::process::exit(1);
                }
            };
            complexity::run_complexity_command(
                &source,
                c.cog_anchor.unwrap_or(15),
                c.nloc_anchor.unwrap_or(60),
                c.max_score.unwrap_or(-1),
                c.max_cognitive.unwrap_or(-1),
                c.max_cyclomatic.unwrap_or(-1),
            );
        }
        None => {
            if args.help {
                if args.human {
                    println!("{}", skill::human());
                } else {
                    println!("{}", skill::json());
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
                skill::maybe_auto_install(&cwd);
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
                args.replay_cex,
                args.perfetto.as_deref(),
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
    use crate::report::{BuildResult, CounterexampleJson};
    use std::io;
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

    struct EmitFailingDiagnosticEmitter {
        emit_attempts: usize,
        finish_attempts: usize,
        error_kind: io::ErrorKind,
    }

    impl DiagnosticEmitter for EmitFailingDiagnosticEmitter {
        fn try_emit(&mut self, _: &Diagnostic) -> io::Result<()> {
            self.emit_attempts += 1;
            Err(io::Error::from(self.error_kind))
        }

        fn try_finish(&mut self) -> io::Result<()> {
            self.finish_attempts += 1;
            Ok(())
        }
    }

    struct FinishFailingDiagnosticEmitter {
        emit_attempts: usize,
        finish_attempts: usize,
        error_kind: io::ErrorKind,
    }

    impl DiagnosticEmitter for FinishFailingDiagnosticEmitter {
        fn try_emit(&mut self, _: &Diagnostic) -> io::Result<()> {
            self.emit_attempts += 1;
            Ok(())
        }

        fn try_finish(&mut self) -> io::Result<()> {
            self.finish_attempts += 1;
            Err(io::Error::from(self.error_kind))
        }
    }

    fn frontend_test_diagnostic(message: &str) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            code: vow_diag::ErrorCode::UnexpectedToken,
            message: message.to_string(),
            primary: vow_diag::SourceLocation {
                file: "test.vow".to_string(),
                byte_offset: 0,
                byte_len: 1,
            },
            secondary: vec![],
            blame: vow_diag::Blame::None,
            hints: vec![],
        }
    }

    #[test]
    fn emit_frontend_diagnostics_returns_emit_failure_without_finishing() {
        let diagnostics = [
            frontend_test_diagnostic("first"),
            frontend_test_diagnostic("second"),
        ];
        let mut emitter = EmitFailingDiagnosticEmitter {
            emit_attempts: 0,
            finish_attempts: 0,
            error_kind: io::ErrorKind::BrokenPipe,
        };

        let error = emit_frontend_diagnostics(&diagnostics, &mut emitter).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(emitter.emit_attempts, 1);
        assert_eq!(emitter.finish_attempts, 0);
    }

    #[test]
    fn emit_frontend_diagnostics_returns_broken_pipe_from_finish() {
        let diagnostics = [frontend_test_diagnostic("parse error")];
        let mut emitter = FinishFailingDiagnosticEmitter {
            emit_attempts: 0,
            finish_attempts: 0,
            error_kind: io::ErrorKind::BrokenPipe,
        };

        let error = emit_frontend_diagnostics(&diagnostics, &mut emitter).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(emitter.emit_attempts, 1);
        assert_eq!(emitter.finish_attempts, 1);
    }

    #[test]
    fn emit_frontend_diagnostics_returns_non_broken_pipe_from_finish() {
        let diagnostics = [frontend_test_diagnostic("type error")];
        let mut emitter = FinishFailingDiagnosticEmitter {
            emit_attempts: 0,
            finish_attempts: 0,
            error_kind: io::ErrorKind::PermissionDenied,
        };

        let error = emit_frontend_diagnostics(&diagnostics, &mut emitter).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(emitter.emit_attempts, 1);
        assert_eq!(emitter.finish_attempts, 1);
    }

    #[test]
    fn emit_frontend_diagnostics_converts_writer_error_to_compile_failure() {
        let dir = TempDir::new().unwrap();
        let source = write_source(&dir, "bad_parse.vow", "module M 123");
        let frontend = prepare_frontend(&source, FrontendGoal::LoweredIr);
        let expected_diagnostics = frontend.as_ref().unwrap_err().diagnostics().to_vec();
        let mut emitter = EmitFailingDiagnosticEmitter {
            emit_attempts: 0,
            finish_attempts: 0,
            error_kind: io::ErrorKind::PermissionDenied,
        };

        let output = emit_frontend_result(frontend, &mut emitter).unwrap_err();

        match &output.status {
            BuildStatus::CompileFailed { message } => {
                assert!(message.contains("failed to emit frontend diagnostics"));
                assert!(message.contains("permission denied"));
            }
            status => panic!("expected CompileFailed, got {status:?}"),
        }
        assert!(output.executable.is_none());
        assert_eq!(output.diagnostics, expected_diagnostics);
        assert_eq!(emitter.emit_attempts, 1);
        assert_eq!(emitter.finish_attempts, 0);
    }

    #[test]
    fn emit_frontend_diagnostics_broken_pipe_preserves_frontend_failure() {
        let dir = TempDir::new().unwrap();
        let source = write_source(&dir, "bad_parse.vow", "module M 123");
        let frontend = prepare_frontend(&source, FrontendGoal::LoweredIr);
        let expected_diagnostics = frontend.as_ref().unwrap_err().diagnostics().to_vec();
        let mut emitter = EmitFailingDiagnosticEmitter {
            emit_attempts: 0,
            finish_attempts: 0,
            error_kind: io::ErrorKind::BrokenPipe,
        };

        let output = emit_frontend_result(frontend, &mut emitter).unwrap_err();

        match &output.status {
            BuildStatus::CompileFailed { message } => assert_eq!(message, "parse error"),
            status => panic!("expected CompileFailed, got {status:?}"),
        }
        assert!(output.executable.is_none());
        assert_eq!(output.diagnostics, expected_diagnostics);
        assert_eq!(emitter.emit_attempts, 1);
        assert_eq!(emitter.finish_attempts, 0);
    }

    #[test]
    fn capacity_flags_are_not_advertised() {
        // Issue #278: the verify-only --vec-max / --string-max / --hashmap-max /
        // --btreemap-max flags were removed. The collection model bound is an
        // internal verifier detail, not a CLI knob, so it must never reappear in
        // any help or skill surface. (--max-k-step is a real, retained flag.)
        let surfaces = [
            skill::json(),
            skill::human(),
            skill::entrypoint_markdown(),
            skill::bundle_markdown(),
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
    fn vow_violation_blame_caller_exit_code_134() {
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
            Some(134),
            "expected reserved runtime-abort exit code 134 (vow violation, #877)"
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
  let hi: i64 = hi;
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
        let out = skill::json();
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
        let out = skill::human();
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
        let parsed: serde_json::Value = serde_json::from_str(&skill::json()).unwrap();

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
        for name in [
            "parse_i8",
            "parse_i16",
            "parse_i32",
            "parse_i64",
            "parse_i128",
            "parse_u8",
            "parse_u16",
            "parse_u32",
            "parse_u64",
            "parse_u128",
        ] {
            assert!(
                lang["builtins"].get(name).is_none(),
                "unexpected unimplemented builtin {name}"
            );
        }
        let string_methods = lang["methods"]["String"].as_array().unwrap();
        assert!(
            string_methods
                .iter()
                .any(|method| method.as_str() == Some(".parse_i64()"))
        );
        assert!(
            string_methods
                .iter()
                .any(|method| method.as_str() == Some(".parse_u64()"))
        );
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
            "rejected with a type error (fail-closed, never silently unverified); use a where clause on the parameter or a requires/ensures contract"
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
    let nums: Vec<i64> = Vec::new();
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
    let v: Vec<i64> = Vec::new();
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
    let m: HashMap<i64, i64> = HashMap::new();
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
                replay: None,
                replay_reason: None,
                replay_raw_values: vec![],
                replay_raw_output: String::new(),
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
                replay: None,
                replay_reason: None,
                replay_raw_values: vec![],
                replay_raw_output: String::new(),
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
    fn counterexample_schema_documents_violating_arg_empty_value_sentinel() {
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../docs/spec/schemas/counterexample.schema.json"
        ))
        .unwrap();
        let value_schema = &schema["properties"]["violating_args"]["items"]["properties"]["value"];

        assert_eq!(value_schema["type"], "string");
        let description = value_schema["description"].as_str().unwrap_or("");
        assert!(
            description.contains("could not be statically recovered"),
            "violating_args[].value description: {description:?}"
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
            replay: None,
            replay_reason: None,
            replay_raw_values: vec![],
            replay_raw_output: String::new(),
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
            replay: None,
            replay_reason: None,
            replay_raw_values: vec![],
            replay_raw_output: String::new(),
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
                replay: None,
                replay_reason: None,
                replay_raw_values: vec![],
                replay_raw_output: String::new(),
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
        let result = run_verify_only_inner(
            &source,
            true,
            &limits,
            4,
            &SolverConfig::default_config(),
            None,
        );
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
        let result = run_verify_only_inner(
            &source,
            true,
            &limits,
            4,
            &SolverConfig::default_config(),
            None,
        );
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
    // #397 regression: a vowed struct builder (`requires: rgn >= 0`, no `ensures`
    // — the exact `ir_inst_set_region` shape) is modeled via the user-struct heap
    // model and now VERIFIES. Previously its `RegionAlloc`/`FieldSet` body made it
    // `SkippedNonModelable`, lifting the overall status to `Skipped` (exit 1) and
    // blocking `scripts/bootstrap.sh` from reaching `Verified`.
    fn vowed_struct_builder_now_verifies() {
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
        let result = run_verify_only_inner(
            &source,
            true,
            &limits,
            1,
            &SolverConfig::default_config(),
            None,
        );
        match &result.status {
            BuildStatus::Verified => {}
            status if esbmc_not_found(status) => {
                eprintln!("SKIP: esbmc not found");
            }
            BuildStatus::CompileFailed { message } => {
                panic!("unexpected compile failure: {message}");
            }
            other => panic!("expected Verified for modelable struct builder, got {other:?}"),
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
            replay: None,
            replay_reason: None,
            replay_raw_values: vec![],
            replay_raw_output: String::new(),
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
            replay: None,
            replay_reason: None,
            replay_raw_values: vec![],
            replay_raw_output: String::new(),
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
    fn resolve_verify_jobs_preserves_explicit_value() {
        assert_eq!(resolve_verify_jobs(Some(1)), 1);
        assert_eq!(resolve_verify_jobs(Some(3)), 3);
    }

    #[test]
    fn verification_limits_are_configurable_in_help() {
        let json = skill::json();
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

        let human = skill::human();
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
            replay: None,
            replay_reason: None,
            replay_raw_values: vec![],
            replay_raw_output: String::new(),
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
