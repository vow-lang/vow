use std::path::Path;

use crate::esbmc::{VerificationResult, memory_limit_reason, run_esbmc_with_max_k_step};

// ---------------------------------------------------------------------------
// Solver / Encoding / Config types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Solver {
    Boolector,
    Z3,
    Bitwuzla,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Bv,
    Ir,
    Auto,
}

#[derive(Debug, Clone, Copy)]
pub struct SolverConfig {
    pub solver: Solver,
    pub encoding: Encoding,
    pub timeout_secs: Option<u32>,
    /// Per-ESBMC-process memory cap. `Some(n)` is emitted as `--memlimit nm`.
    pub memlimit_mb: Option<u32>,
}

pub const DEFAULT_AUTO_TIMEOUT_SECS: u32 = 30;
/// Default per-ESBMC-process cap. This bounds solver RSS without changing Vow contracts.
pub const DEFAULT_ESBMC_MEMLIMIT_MB: u32 = 4096;

impl SolverConfig {
    pub fn default_config() -> Self {
        Self {
            solver: Solver::Auto,
            encoding: Encoding::Auto,
            timeout_secs: None,
            memlimit_mb: Some(DEFAULT_ESBMC_MEMLIMIT_MB),
        }
    }

    /// Validate the configuration. Returns an error message if invalid.
    pub fn validate(&self) -> Result<(), String> {
        match (self.encoding, self.solver) {
            (Encoding::Ir, Solver::Boolector) => Err(
                "--encoding ir requires --solver z3 (Boolector does not support integer mode)"
                    .into(),
            ),
            (Encoding::Ir, Solver::Bitwuzla) => Err(
                "--encoding ir requires --solver z3 (Bitwuzla does not support integer mode)"
                    .into(),
            ),
            _ => Ok(()),
        }
    }

    /// Resolve Auto fields to concrete values.
    /// After this, solver and encoding are never Auto.
    pub fn resolve(&self) -> Self {
        let solver = match (self.encoding, self.solver) {
            // --encoding ir forces Z3
            (Encoding::Ir, Solver::Auto) => Solver::Z3,
            (_, Solver::Auto) => Solver::Boolector,
            (_, s) => s,
        };
        let encoding = match self.encoding {
            Encoding::Auto => Encoding::Bv,
            e => e,
        };
        Self {
            solver,
            encoding,
            timeout_secs: self.timeout_secs,
            memlimit_mb: self.memlimit_mb,
        }
    }

    /// Convert resolved config to ESBMC CLI flags.
    pub fn esbmc_args(&self) -> Vec<String> {
        let resolved = self.resolve();
        let mut args = Vec::new();
        match resolved.solver {
            Solver::Z3 => args.push("--z3".to_string()),
            Solver::Bitwuzla => args.push("--bitwuzla".to_string()),
            Solver::Boolector | Solver::Auto => {} // boolector is ESBMC default
        }
        match resolved.encoding {
            Encoding::Ir => args.push("--ir".to_string()),
            Encoding::Bv | Encoding::Auto => {} // bv is ESBMC default
        }
        if let Some(mb) = resolved.memlimit_mb {
            args.push("--memlimit".to_string());
            args.push(format!("{mb}m"));
        }
        args
    }

    /// Returns the solver name string for cache keys and diagnostics.
    pub fn solver_str(&self) -> &'static str {
        match self.resolve().solver {
            Solver::Boolector | Solver::Auto => "boolector",
            Solver::Z3 => "z3",
            Solver::Bitwuzla => "bitwuzla",
        }
    }

    /// Returns the encoding name string for cache keys and diagnostics.
    pub fn encoding_str(&self) -> &'static str {
        match self.resolve().encoding {
            Encoding::Bv | Encoding::Auto => "bv",
            Encoding::Ir => "ir",
        }
    }
}

// ---------------------------------------------------------------------------
// Heuristic: BV solver selection (Phase B)
// ---------------------------------------------------------------------------

use vow_ir::{Function, InstData, Opcode};

/// Classify a function's IR to select the best BV solver.
/// Never selects Encoding::Ir — that's reserved for timeout fallback (Phase D).
pub fn classify_function(func: &Function) -> SolverConfig {
    let mut has_bitwise = false;
    let mut has_mul_div_rem = false;
    let mut has_large_const = false;

    for block in &func.blocks {
        for inst in &block.insts {
            match inst.opcode {
                // Bitwise / shift
                Opcode::BitAndI64
                | Opcode::BitOrI64
                | Opcode::XorI64
                | Opcode::XorI32
                | Opcode::ShlI64
                | Opcode::ShrI64
                | Opcode::BitAndU64
                | Opcode::BitOrU64
                | Opcode::XorU64
                | Opcode::ShlU64
                | Opcode::ShrU64 => {
                    has_bitwise = true;
                }
                // Multiplication / division / remainder (I64)
                Opcode::WrappingMulI64
                | Opcode::CheckedMulI64
                | Opcode::WrappingDivI64
                | Opcode::CheckedDivI64
                | Opcode::WrappingRemI64
                | Opcode::CheckedRemI64
                // I32 variants
                | Opcode::WrappingMulI32
                | Opcode::CheckedMulI32
                | Opcode::WrappingDivI32
                | Opcode::CheckedDivI32
                | Opcode::WrappingRemI32
                | Opcode::CheckedRemI32
                // U64 variants
                | Opcode::WrappingMulU64
                | Opcode::CheckedMulU64
                | Opcode::WrappingDivU64
                | Opcode::CheckedDivU64
                | Opcode::WrappingRemU64
                | Opcode::CheckedRemU64 => {
                    has_mul_div_rem = true;
                }
                _ => {}
            }
            match &inst.data {
                InstData::ConstI64(v) if v.unsigned_abs() > (1u64 << 32) => {
                    has_large_const = true;
                }
                InstData::ConstU64(v) if *v > (1u64 << 32) => {
                    has_large_const = true;
                }
                _ => {}
            }
        }
    }

    let solver = if has_bitwise {
        Solver::Bitwuzla
    } else if has_mul_div_rem && has_large_const {
        Solver::Z3
    } else {
        Solver::Boolector
    };

    SolverConfig {
        solver,
        encoding: Encoding::Bv, // never auto-select Ir
        timeout_secs: None,
        memlimit_mb: Some(DEFAULT_ESBMC_MEMLIMIT_MB),
    }
}

// ---------------------------------------------------------------------------
// Fallback orchestration (Phase D)
// ---------------------------------------------------------------------------

/// Soundness guard for the adaptive-retry ladder (see docs/verifier-discipline.md).
///
/// A resource-limited retry runs the *weaker* IR encoding, which does not model
/// integer overflow. If that retry proves the property it MUST be surfaced as
/// `ProvenIr`, never as a bare `Proven`: the label keeps a weaker-encoding proof
/// distinguishable from a full bit-vector proof so downstream tooling can route
/// on it. Returning `Proven` here would launder the weaker result into an
/// unqualified proof — the verifier-side mirror of the contract rule that
/// artificial bounds must not enter a proof obligation. A violation is a
/// verifier bug and fails closed: the panic surfaces as `verify_status:
/// "panicked"`, never `Verified`.
fn enforce_retry_never_launders_proof(retry: &VerificationResult) {
    assert!(
        !matches!(retry, VerificationResult::Proven),
        "verifier-discipline violation: resource-limited IR retry returned bare \
         `Proven`; a proof found under the weaker IR encoding must be labeled \
         `ProvenIr` (see docs/verifier-discipline.md)"
    );
}

/// Run ESBMC with fallback: if BV times out in auto mode, retry with --ir --z3.
/// Returns the result and the config that produced it.
pub fn run_with_fallback(
    esbmc: &Path,
    c_src: &str,
    max_k_step: u32,
    func_name: &str,
    config: &SolverConfig,
) -> (VerificationResult, SolverConfig) {
    // If encoding is explicit (not Auto), run once — no fallback.
    if config.encoding != Encoding::Auto {
        let resolved = config.resolve();
        let result = run_esbmc_with_max_k_step(esbmc, c_src, max_k_step, func_name, &resolved);
        return (result, resolved);
    }

    // Auto mode: run with BV first, fallback to IR on timeout.
    //
    // The default 30s cap is only meaningful when the IR fallback is
    // actually reachable. Bitwuzla can't run the IR encoding, so applying
    // `DEFAULT_AUTO_TIMEOUT_SECS` when the resolved BV solver is Bitwuzla
    // would amount to a silent regression — users who pick Bitwuzla for
    // bitwise-heavy contracts would get cut off at 30s with no retry.
    // Leave those runs uncapped unless the user set --timeout explicitly.
    let bv_solver = match config.solver {
        Solver::Auto => Solver::Boolector,
        s => s,
    };
    let timeout_secs = if config.timeout_secs.is_some() {
        config.timeout_secs
    } else if matches!(bv_solver, Solver::Bitwuzla) {
        None
    } else {
        Some(DEFAULT_AUTO_TIMEOUT_SECS)
    };
    let bv_config = SolverConfig {
        solver: config.solver,
        encoding: Encoding::Bv,
        timeout_secs,
        memlimit_mb: config.memlimit_mb,
    }
    .resolve();

    let result = run_esbmc_with_max_k_step(esbmc, c_src, max_k_step, func_name, &bv_config);

    let resource_limited = matches!(&result, VerificationResult::Timeout)
        || matches!(&result, VerificationResult::Unknown { reason } if reason == &memory_limit_reason());
    if resource_limited {
        // Timeout/memlimit means BV could neither prove nor disprove within
        // the resource budget, so auto mode may retry with Z3+IR. Other
        // UNKNOWN outcomes are explicit ESBMC inconclusive results and must
        // not be hidden by proving the weaker IR abstraction.
        let can_ir = !matches!(bv_config.solver, Solver::Bitwuzla);
        if !can_ir {
            return (result, bv_config);
        }

        // Reuse the same timeout/memlimit policy for the IR retry: user
        // timeout override if set, else the 30s default.
        let ir_timeout = config.timeout_secs.or(Some(DEFAULT_AUTO_TIMEOUT_SECS));
        let ir_config = SolverConfig {
            solver: Solver::Z3,
            encoding: Encoding::Ir,
            timeout_secs: ir_timeout,
            memlimit_mb: config.memlimit_mb,
        };
        // The IR retry must run the SAME unwind bound as the BV attempt.
        // Reducing the unwind on retry ("halve unwind on timeout") would prove a
        // strictly weaker obligation and is forbidden — see docs/verifier-discipline.md.
        //
        // `ir_max_k_step` is deliberately just `max_k_step`, so the assertion below is
        // a tautology *today* — that is intentional. It is a trip wire, not a check of
        // current behaviour: it exists so a future edit that derives a smaller bound
        // (e.g. `max_k_step / 2`) fails loudly here instead of silently proving the
        // weaker obligation.
        let ir_max_k_step = max_k_step;
        assert!(
            ir_max_k_step >= max_k_step,
            "verifier-discipline violation: IR retry unwind {ir_max_k_step} is below \
             the BV unwind {max_k_step}; a retry must never reduce the unwind bound \
             (see docs/verifier-discipline.md)"
        );
        let ir_result =
            run_esbmc_with_max_k_step(esbmc, c_src, ir_max_k_step, func_name, &ir_config);
        let retry = match ir_result {
            VerificationResult::Proven => (VerificationResult::ProvenIr, ir_config),
            // IR returned a counterexample, but IR does not model overflow —
            // a CE found only under IR can be infeasible under BV. Report the
            // original resource-limited result rather than an unsound Failed.
            VerificationResult::Failed(_) => (result, ir_config),
            other => (other, ir_config),
        };
        enforce_retry_never_launders_proof(&retry.0);
        retry
    } else {
        (result, bv_config)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use vow_ir::{BasicBlock, BlockId, FuncId, Inst, InstId, RegionId, RegionSummary, Ty};
    use vow_syntax::span::Span;

    fn inst(id: u32, opcode: Opcode, ty: Ty, args: Vec<u32>, data: InstData) -> Inst {
        Inst {
            id: InstId(id),
            opcode,
            ty,
            args: args.into_iter().map(InstId).collect(),
            data,
            origin: Span { start: 0, len: 0 },
            region: RegionId::Root,
        }
    }

    fn make_func(name: &str, params: Vec<Ty>, return_ty: Ty, insts: Vec<Inst>) -> Function {
        Function {
            id: FuncId(0),
            name: name.to_string(),
            params,
            param_names: vec![],
            return_ty,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts,
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        }
    }

    // -- SolverConfig tests --

    #[test]
    fn test_validate_ir_boolector_rejects() {
        let c = SolverConfig {
            solver: Solver::Boolector,
            encoding: Encoding::Ir,
            timeout_secs: None,
            memlimit_mb: Some(DEFAULT_ESBMC_MEMLIMIT_MB),
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_validate_ir_bitwuzla_rejects() {
        let c = SolverConfig {
            solver: Solver::Bitwuzla,
            encoding: Encoding::Ir,
            timeout_secs: None,
            memlimit_mb: Some(DEFAULT_ESBMC_MEMLIMIT_MB),
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_validate_ir_z3_accepts() {
        let c = SolverConfig {
            solver: Solver::Z3,
            encoding: Encoding::Ir,
            timeout_secs: None,
            memlimit_mb: Some(DEFAULT_ESBMC_MEMLIMIT_MB),
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn test_validate_ir_auto_accepts() {
        let c = SolverConfig {
            solver: Solver::Auto,
            encoding: Encoding::Ir,
            timeout_secs: None,
            memlimit_mb: Some(DEFAULT_ESBMC_MEMLIMIT_MB),
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn test_resolve_ir_auto_becomes_z3() {
        let c = SolverConfig {
            solver: Solver::Auto,
            encoding: Encoding::Ir,
            timeout_secs: None,
            memlimit_mb: Some(DEFAULT_ESBMC_MEMLIMIT_MB),
        };
        let r = c.resolve();
        assert_eq!(r.solver, Solver::Z3);
        assert_eq!(r.encoding, Encoding::Ir);
    }

    #[test]
    fn test_resolve_auto_auto_becomes_boolector_bv() {
        let c = SolverConfig::default_config();
        let r = c.resolve();
        assert_eq!(r.solver, Solver::Boolector);
        assert_eq!(r.encoding, Encoding::Bv);
    }

    #[test]
    fn test_default_config_sets_esbmc_memlimit() {
        let c = SolverConfig::default_config();
        assert_eq!(c.memlimit_mb, Some(DEFAULT_ESBMC_MEMLIMIT_MB));
    }

    #[test]
    fn test_resolve_preserves_esbmc_memlimit() {
        let c = SolverConfig {
            solver: Solver::Auto,
            encoding: Encoding::Auto,
            timeout_secs: None,
            memlimit_mb: Some(1024),
        };
        let r = c.resolve();
        assert_eq!(r.memlimit_mb, Some(1024));
    }

    #[test]
    fn test_esbmc_args_default() {
        let c = SolverConfig::default_config();
        assert_eq!(c.esbmc_args(), vec!["--memlimit", "4096m"]);
    }

    #[test]
    fn test_esbmc_args_z3_ir() {
        let c = SolverConfig {
            solver: Solver::Z3,
            encoding: Encoding::Ir,
            timeout_secs: None,
            memlimit_mb: Some(2048),
        };
        let args = c.esbmc_args();
        assert_eq!(args, vec!["--z3", "--ir", "--memlimit", "2048m"]);
    }

    #[test]
    fn test_esbmc_args_bitwuzla_bv() {
        let c = SolverConfig {
            solver: Solver::Bitwuzla,
            encoding: Encoding::Bv,
            timeout_secs: None,
            memlimit_mb: Some(512),
        };
        let args = c.esbmc_args();
        assert_eq!(args, vec!["--bitwuzla", "--memlimit", "512m"]);
    }

    #[test]
    fn test_esbmc_args_omits_memlimit_when_none() {
        let c = SolverConfig {
            solver: Solver::Boolector,
            encoding: Encoding::Bv,
            timeout_secs: None,
            memlimit_mb: None,
        };
        assert!(c.esbmc_args().is_empty());
    }

    // -- Heuristic tests --

    #[test]
    fn test_classify_simple_add() {
        let func = make_func(
            "add",
            vec![Ty::I64, Ty::I64],
            Ty::I64,
            vec![
                inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                inst(
                    2,
                    Opcode::WrappingAddI64,
                    Ty::I64,
                    vec![0, 1],
                    InstData::None,
                ),
                inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
            ],
        );
        let c = classify_function(&func);
        assert_eq!(c.solver, Solver::Boolector);
        assert_eq!(c.encoding, Encoding::Bv);
    }

    #[test]
    fn test_classify_bitwise_ops() {
        let func = make_func(
            "mask",
            vec![Ty::I64, Ty::I64],
            Ty::I64,
            vec![
                inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                inst(2, Opcode::BitAndI64, Ty::I64, vec![0, 1], InstData::None),
                inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
            ],
        );
        let c = classify_function(&func);
        assert_eq!(c.solver, Solver::Bitwuzla);
        assert_eq!(c.encoding, Encoding::Bv);
    }

    #[test]
    fn test_classify_mul_with_large_const() {
        let func = make_func(
            "scale",
            vec![Ty::I64],
            Ty::I64,
            vec![
                inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                inst(
                    1,
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(9223372036854775807),
                ),
                inst(
                    2,
                    Opcode::WrappingDivI64,
                    Ty::I64,
                    vec![1, 0],
                    InstData::None,
                ),
                inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
            ],
        );
        let c = classify_function(&func);
        assert_eq!(c.solver, Solver::Z3);
        assert_eq!(c.encoding, Encoding::Bv);
    }

    #[test]
    fn test_classify_mul_without_large_const() {
        let func = make_func(
            "mul",
            vec![Ty::I64, Ty::I64],
            Ty::I64,
            vec![
                inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                inst(
                    2,
                    Opcode::WrappingMulI64,
                    Ty::I64,
                    vec![0, 1],
                    InstData::None,
                ),
                inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
            ],
        );
        let c = classify_function(&func);
        assert_eq!(c.solver, Solver::Boolector);
        assert_eq!(c.encoding, Encoding::Bv);
    }

    #[test]
    fn test_classify_mixed_bitwise_and_mul() {
        let func = make_func(
            "mixed",
            vec![Ty::I64, Ty::I64],
            Ty::I64,
            vec![
                inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                inst(
                    2,
                    Opcode::WrappingMulI64,
                    Ty::I64,
                    vec![0, 1],
                    InstData::None,
                ),
                inst(3, Opcode::ShlI64, Ty::I64, vec![2, 1], InstData::None),
                inst(
                    4,
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(i64::MAX),
                ),
                inst(5, Opcode::Return, Ty::Unit, vec![], InstData::None),
            ],
        );
        let c = classify_function(&func);
        assert_eq!(c.solver, Solver::Bitwuzla); // bitwise takes priority
        assert_eq!(c.encoding, Encoding::Bv);
    }

    #[test]
    fn test_classify_never_selects_ir() {
        // Even with mul + large const, encoding must be Bv
        let func = make_func(
            "big",
            vec![Ty::I64],
            Ty::I64,
            vec![
                inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                inst(
                    1,
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(i64::MAX),
                ),
                inst(
                    2,
                    Opcode::WrappingMulI64,
                    Ty::I64,
                    vec![0, 1],
                    InstData::None,
                ),
                inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
            ],
        );
        let c = classify_function(&func);
        assert_eq!(c.encoding, Encoding::Bv); // NEVER Ir from heuristic
    }

    // -- Phase D: run_with_fallback tests --

    use crate::c_emitter::VerifyLimits;
    use crate::esbmc::{emit_verify_c_source, find_esbmc};
    use std::collections::HashMap;
    use vow_diag::Blame;
    use vow_ir::{InstData, Module as IrModule, Opcode, VowEntry, VowId};

    fn trivially_true_fn() -> Function {
        Function {
            id: FuncId(0),
            name: "always_ok".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I32,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "true".to_string(),
                blame: Blame::Callee,
                bindings: vec![],
                file: String::new(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(
                        0,
                        Opcode::ConstBool,
                        Ty::Bool,
                        vec![],
                        InstData::ConstBool(true),
                    ),
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::VowEnsures,
                        ty: Ty::Unit,
                        args: vec![InstId(0)],
                        data: InstData::VowId(VowId(0)),
                        origin: Span { start: 0, len: 0 },
                        region: RegionId::Root,
                    },
                    inst(2, Opcode::ConstI32, Ty::I32, vec![], InstData::ConstI32(42)),
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        }
    }

    fn emit_c_for(func: &Function) -> String {
        let module = IrModule {
            name: "test".to_string(),
            functions: vec![func.clone()],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        let const_fns = HashMap::new();
        emit_verify_c_source(func, &module, &const_fns, &VerifyLimits::default())
    }

    /// Phase D: when encoding is explicit (Bv), run_with_fallback runs once
    /// and returns the resolved config unchanged.
    #[test]
    fn fallback_explicit_bv_one_shot() {
        let esbmc = match find_esbmc() {
            Some(p) => p,
            None => {
                eprintln!("SKIP: esbmc not found");
                return;
            }
        };
        let func = trivially_true_fn();
        let c_src = emit_c_for(&func);
        let cfg = SolverConfig {
            solver: Solver::Boolector,
            encoding: Encoding::Bv,
            timeout_secs: None,
            memlimit_mb: Some(DEFAULT_ESBMC_MEMLIMIT_MB),
        };
        let (result, resolved) = run_with_fallback(&esbmc, &c_src, 5, &func.name, &cfg);
        assert!(matches!(result, VerificationResult::Proven));
        assert_eq!(resolved.solver, Solver::Boolector);
        assert_eq!(resolved.encoding, Encoding::Bv);
    }

    /// Phase D: when encoding is Auto and BV proves the function, no IR
    /// retry is attempted and the returned config reports encoding=Bv.
    #[test]
    fn fallback_auto_bv_proves_trivial_no_retry() {
        let esbmc = match find_esbmc() {
            Some(p) => p,
            None => {
                eprintln!("SKIP: esbmc not found");
                return;
            }
        };
        let func = trivially_true_fn();
        let c_src = emit_c_for(&func);
        let cfg = SolverConfig {
            solver: Solver::Auto,
            encoding: Encoding::Auto,
            timeout_secs: None,
            memlimit_mb: Some(DEFAULT_ESBMC_MEMLIMIT_MB),
        };
        let (result, resolved) = run_with_fallback(&esbmc, &c_src, 5, &func.name, &cfg);
        assert!(matches!(result, VerificationResult::Proven));
        // No fallback kicked in, so encoding must be Bv (IR is only used on timeout).
        assert_eq!(resolved.encoding, Encoding::Bv);
    }

    /// Phase D: the Bitwuzla guard — when BV solver is Bitwuzla and BV
    /// times out in Auto encoding, run_with_fallback must NOT fall back to
    /// IR (IR requires Z3). Force a timeout by using timeout_secs=0.
    #[test]
    fn fallback_bitwuzla_never_retries_ir() {
        let esbmc = match find_esbmc() {
            Some(p) => p,
            None => {
                eprintln!("SKIP: esbmc not found");
                return;
            }
        };
        let func = trivially_true_fn();
        let c_src = emit_c_for(&func);
        let cfg = SolverConfig {
            solver: Solver::Bitwuzla,
            encoding: Encoding::Auto,
            timeout_secs: Some(0), // force BV to time out immediately
            memlimit_mb: Some(DEFAULT_ESBMC_MEMLIMIT_MB),
        };
        let (result, resolved) = run_with_fallback(&esbmc, &c_src, 5, &func.name, &cfg);
        // The BV run might finish between spawn and the first 50ms poll; if
        // so, we accept Proven. What we must never see is ProvenIr — that
        // would mean the guard was bypassed and IR was tried with Bitwuzla.
        match result {
            VerificationResult::Timeout => {
                assert_eq!(resolved.solver, Solver::Bitwuzla);
                assert_eq!(resolved.encoding, Encoding::Bv);
            }
            VerificationResult::Proven => {
                assert_eq!(resolved.solver, Solver::Bitwuzla);
                assert_eq!(resolved.encoding, Encoding::Bv);
            }
            VerificationResult::ProvenIr => {
                panic!("bitwuzla must not fall back to IR encoding");
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }

    /// UNKNOWN is an explicit ESBMC outcome, not a wall-clock timeout.
    /// Auto fallback must not let a weaker IR proof upgrade it to ProvenIr.
    ///
    /// Unix-only: the fake-esbmc is a `#!/bin/sh` script that cannot be
    /// launched directly on Windows even with the chmod skipped.
    #[cfg(unix)]
    #[test]
    fn fallback_auto_bv_unknown_does_not_retry_ir() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let esbmc = dir.path().join("fake-esbmc");
        std::fs::write(
            &esbmc,
            r#"#!/bin/sh
for arg in "$@"; do
    if [ "$arg" = "--ir" ]; then
        echo "VERIFICATION SUCCESSFUL"
        exit 0
    fi
done
echo "Unable to prove or falsify the program, giving up."
echo "VERIFICATION UNKNOWN"
"#,
        )
        .expect("write fake esbmc");
        let mut perms = std::fs::metadata(&esbmc).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&esbmc, perms).expect("chmod fake esbmc");

        let cfg = SolverConfig {
            solver: Solver::Auto,
            encoding: Encoding::Auto,
            timeout_secs: Some(5),
            memlimit_mb: Some(DEFAULT_ESBMC_MEMLIMIT_MB),
        };
        let (result, resolved) =
            run_with_fallback(&esbmc, "int main(void) { return 0; }", 5, "main", &cfg);

        match result {
            VerificationResult::Unknown { reason } => {
                assert!(reason.contains("Unable to prove or falsify"));
            }
            other => panic!("UNKNOWN must not be upgraded by IR fallback: {other:?}"),
        }
        assert_eq!(resolved.solver, Solver::Boolector);
        assert_eq!(resolved.encoding, Encoding::Bv);
    }

    /// A memory-limit hit is a verifier resource exhaustion, like timeout,
    /// not ESBMC's logical `VERIFICATION UNKNOWN`. Auto mode may retry IR.
    #[cfg(unix)]
    #[test]
    fn fallback_auto_bv_memlimit_retries_with_ir() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let esbmc = dir.path().join("fake-esbmc");
        std::fs::write(
            &esbmc,
            r#"#!/bin/sh
for arg in "$@"; do
    if [ "$arg" = "--ir" ]; then
        echo "VERIFICATION SUCCESSFUL"
        exit 0
    fi
done
echo "Out of memory: memory limit exceeded"
exit 6
"#,
        )
        .expect("write fake esbmc");
        let mut perms = std::fs::metadata(&esbmc).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&esbmc, perms).expect("chmod fake esbmc");

        let cfg = SolverConfig {
            solver: Solver::Auto,
            encoding: Encoding::Auto,
            timeout_secs: Some(5),
            memlimit_mb: Some(DEFAULT_ESBMC_MEMLIMIT_MB),
        };
        let (result, resolved) =
            run_with_fallback(&esbmc, "int main(void) { return 0; }", 5, "main", &cfg);

        assert!(matches!(result, VerificationResult::ProvenIr));
        assert_eq!(resolved.solver, Solver::Z3);
        assert_eq!(resolved.encoding, Encoding::Ir);
    }

    /// Phase D: when BV times out under Auto encoding (non-Bitwuzla solver),
    /// the fallback retries with Z3+IR. Force the BV timeout with
    /// timeout_secs=0 so the IR retry is what actually runs; IR proves the
    /// trivially-true ensures and we get ProvenIr.
    #[test]
    fn fallback_auto_bv_timeout_retries_with_ir() {
        let esbmc = match find_esbmc() {
            Some(p) => p,
            None => {
                eprintln!("SKIP: esbmc not found");
                return;
            }
        };
        let func = trivially_true_fn();
        let c_src = emit_c_for(&func);
        let cfg = SolverConfig {
            solver: Solver::Boolector,
            encoding: Encoding::Auto,
            timeout_secs: Some(0),
            memlimit_mb: Some(DEFAULT_ESBMC_MEMLIMIT_MB),
        };
        let (result, resolved) = run_with_fallback(&esbmc, &c_src, 5, &func.name, &cfg);
        // The BV run may or may not race the 50ms poll; accept either
        // Proven (BV finished first) or ProvenIr (fallback kicked in).
        match result {
            VerificationResult::Proven => {
                assert_eq!(resolved.encoding, Encoding::Bv);
            }
            VerificationResult::ProvenIr => {
                assert_eq!(resolved.solver, Solver::Z3);
                assert_eq!(resolved.encoding, Encoding::Ir);
            }
            VerificationResult::Timeout => {
                // Both BV and IR timed out — still valid for the guard test,
                // but the resolved config must reflect the IR attempt since
                // fallback was enabled.
                assert_eq!(resolved.encoding, Encoding::Ir);
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }

    /// Issue #337 discipline: a forced timeout must never surface as a proof.
    /// Both the BV attempt and the IR retry time out (the fake emits output
    /// containing "timeout", which `classify_esbmc_output` maps to `Timeout`),
    /// so the fallback must return `Timeout` — never `Proven`/`ProvenIr`.
    /// Deterministic: no real wall-clock timeout, so no race with the poll.
    #[cfg(unix)]
    #[test]
    fn fallback_forced_timeout_never_verified() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let esbmc = dir.path().join("fake-esbmc");
        std::fs::write(
            &esbmc,
            r#"#!/bin/sh
echo "esbmc: timeout reached"
exit 0
"#,
        )
        .expect("write fake esbmc");
        let mut perms = std::fs::metadata(&esbmc).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&esbmc, perms).expect("chmod fake esbmc");

        let cfg = SolverConfig {
            solver: Solver::Auto,
            encoding: Encoding::Auto,
            timeout_secs: Some(5),
            memlimit_mb: Some(DEFAULT_ESBMC_MEMLIMIT_MB),
        };
        let (result, _resolved) =
            run_with_fallback(&esbmc, "int main(void) { return 0; }", 5, "main", &cfg);

        match result {
            VerificationResult::Timeout => {}
            VerificationResult::Proven | VerificationResult::ProvenIr => {
                panic!("a forced timeout must never produce a proof: {result:?}");
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    /// Issue #337 discipline: when the BV attempt times out and the IR retry
    /// proves the property, the proof MUST be labeled `ProvenIr` (weaker
    /// encoding), never a bare `Proven`. Exercises the relabel and the
    /// `enforce_retry_never_launders_proof` guard's happy path.
    #[cfg(unix)]
    #[test]
    fn fallback_forced_timeout_ir_proof_is_labeled_proven_ir() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let esbmc = dir.path().join("fake-esbmc");
        std::fs::write(
            &esbmc,
            r#"#!/bin/sh
for arg in "$@"; do
    if [ "$arg" = "--ir" ]; then
        echo "VERIFICATION SUCCESSFUL"
        exit 0
    fi
done
echo "esbmc: timeout reached"
exit 0
"#,
        )
        .expect("write fake esbmc");
        let mut perms = std::fs::metadata(&esbmc).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&esbmc, perms).expect("chmod fake esbmc");

        let cfg = SolverConfig {
            solver: Solver::Auto,
            encoding: Encoding::Auto,
            timeout_secs: Some(5),
            memlimit_mb: Some(DEFAULT_ESBMC_MEMLIMIT_MB),
        };
        let (result, resolved) =
            run_with_fallback(&esbmc, "int main(void) { return 0; }", 5, "main", &cfg);

        assert!(
            matches!(result, VerificationResult::ProvenIr),
            "IR retry proof must be labeled ProvenIr, got {result:?}"
        );
        assert_eq!(resolved.solver, Solver::Z3);
        assert_eq!(resolved.encoding, Encoding::Ir);
    }
}
