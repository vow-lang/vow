use std::path::Path;

use crate::esbmc::{VerificationResult, run_esbmc_with_max_k_step};

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
}

pub const DEFAULT_AUTO_TIMEOUT_SECS: u32 = 30;

impl SolverConfig {
    pub fn default_config() -> Self {
        Self {
            solver: Solver::Auto,
            encoding: Encoding::Auto,
            timeout_secs: None,
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
    }
}

// ---------------------------------------------------------------------------
// Fallback orchestration (Phase D)
// ---------------------------------------------------------------------------

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
    let timeout = config.timeout_secs.unwrap_or(DEFAULT_AUTO_TIMEOUT_SECS);
    let bv_config = SolverConfig {
        solver: config.solver,
        encoding: Encoding::Bv,
        timeout_secs: Some(timeout),
    }
    .resolve();

    let result = run_esbmc_with_max_k_step(esbmc, c_src, max_k_step, func_name, &bv_config);

    match result {
        VerificationResult::Timeout => {
            // IR encoding only works with Z3. If the resolved BV solver was
            // Boolector or Z3, we can switch to Z3+IR. Bitwuzla cannot do IR.
            let can_ir = !matches!(bv_config.solver, Solver::Bitwuzla);
            if !can_ir {
                return (VerificationResult::Timeout, bv_config);
            }

            let ir_config = SolverConfig {
                solver: Solver::Z3,
                encoding: Encoding::Ir,
                timeout_secs: Some(timeout),
            };
            let ir_result =
                run_esbmc_with_max_k_step(esbmc, c_src, max_k_step, func_name, &ir_config);
            match ir_result {
                VerificationResult::Proven => (VerificationResult::ProvenIr, ir_config),
                other => (other, ir_config),
            }
        }
        other => (other, bv_config),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use vow_ir::{BasicBlock, BlockId, FuncId, Inst, InstId, Ty};
    use vow_syntax::span::Span;

    fn inst(id: u32, opcode: Opcode, ty: Ty, args: Vec<u32>, data: InstData) -> Inst {
        Inst {
            id: InstId(id),
            opcode,
            ty,
            args: args.into_iter().map(InstId).collect(),
            data,
            origin: Span { start: 0, len: 0 },
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
        }
    }

    // -- SolverConfig tests --

    #[test]
    fn test_validate_ir_boolector_rejects() {
        let c = SolverConfig {
            solver: Solver::Boolector,
            encoding: Encoding::Ir,
            timeout_secs: None,
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_validate_ir_bitwuzla_rejects() {
        let c = SolverConfig {
            solver: Solver::Bitwuzla,
            encoding: Encoding::Ir,
            timeout_secs: None,
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_validate_ir_z3_accepts() {
        let c = SolverConfig {
            solver: Solver::Z3,
            encoding: Encoding::Ir,
            timeout_secs: None,
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn test_validate_ir_auto_accepts() {
        let c = SolverConfig {
            solver: Solver::Auto,
            encoding: Encoding::Ir,
            timeout_secs: None,
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn test_resolve_ir_auto_becomes_z3() {
        let c = SolverConfig {
            solver: Solver::Auto,
            encoding: Encoding::Ir,
            timeout_secs: None,
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
    fn test_esbmc_args_default() {
        let c = SolverConfig::default_config();
        assert!(c.esbmc_args().is_empty());
    }

    #[test]
    fn test_esbmc_args_z3_ir() {
        let c = SolverConfig {
            solver: Solver::Z3,
            encoding: Encoding::Ir,
            timeout_secs: None,
        };
        let args = c.esbmc_args();
        assert_eq!(args, vec!["--z3", "--ir"]);
    }

    #[test]
    fn test_esbmc_args_bitwuzla_bv() {
        let c = SolverConfig {
            solver: Solver::Bitwuzla,
            encoding: Encoding::Bv,
            timeout_secs: None,
        };
        let args = c.esbmc_args();
        assert_eq!(args, vec!["--bitwuzla"]);
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
                    },
                    inst(2, Opcode::ConstI32, Ty::I32, vec![], InstData::ConstI32(42)),
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
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
}
