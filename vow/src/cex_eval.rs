// Constant / partial evaluation of `vow_ir` instructions for counterexample
// construction. Two closely related evaluators live here:
//
//   * the *callee* evaluators (`eval_callee_i64_for_call_site` /
//     `eval_callee_bool_for_call_site`) fold a callee's precondition predicate
//     against the concrete argument values recorded at a call site, resolving
//     `GetArg` from the caller's `CallSiteInfo`;
//   * the *counterexample* evaluator (`eval_const_i64_for_counterexample`) folds
//     a call argument's own instruction subtree to a literal when building the
//     call-site index — it has no call site to draw `GetArg` from and does not
//     fold `ConstU64`.
//
// Both i64 evaluators share the exact same integer arithmetic; that folding
// lives once in `fold_binary_i64`, parameterized by the caller's own operand
// evaluator so each keeps its distinct leaf rules.

use std::collections::HashMap;

use crate::counterexample::CallSiteInfo;

/// Fold a binary i64 arithmetic opcode, recursing into operands via
/// `eval_operand`. Returns `None` for any non-arithmetic opcode (so a caller can
/// delegate its catch-all arm here) and for operands that do not fold. The two
/// i64 evaluators below share this — the only difference between them is their
/// leaf rules, which each supplies through `eval_operand`.
fn fold_binary_i64(inst: &vow_ir::Inst, eval_operand: impl Fn(u32) -> Option<i64>) -> Option<i64> {
    use vow_ir::Opcode;
    let op: fn(i64, i64) -> i64 = match inst.opcode {
        Opcode::WrappingAddI32
        | Opcode::CheckedAddI32
        | Opcode::WrappingAddI64
        | Opcode::CheckedAddI64 => i64::wrapping_add,
        Opcode::WrappingSubI32
        | Opcode::CheckedSubI32
        | Opcode::WrappingSubI64
        | Opcode::CheckedSubI64 => i64::wrapping_sub,
        Opcode::WrappingMulI32
        | Opcode::CheckedMulI32
        | Opcode::WrappingMulI64
        | Opcode::CheckedMulI64 => i64::wrapping_mul,
        _ => return None,
    };
    let lhs = eval_operand(inst.args.first()?.0)?;
    let rhs = eval_operand(inst.args.get(1)?.0)?;
    Some(op(lhs, rhs))
}

/// Fold an instruction to an `i64` in the *callee* context, resolving `GetArg`
/// from `call_site`'s recorded argument values.
fn eval_callee_i64_for_call_site(
    inst_id: u32,
    inst_by_id: &HashMap<u32, &vow_ir::Inst>,
    call_site: &CallSiteInfo,
) -> Option<i64> {
    use vow_ir::{InstData, Opcode};
    let inst = *inst_by_id.get(&inst_id)?;
    match inst.opcode {
        Opcode::ConstI32 => {
            if let InstData::ConstI32(v) = inst.data {
                Some(v as i64)
            } else {
                None
            }
        }
        Opcode::ConstI64 => {
            if let InstData::ConstI64(v) = inst.data {
                Some(v)
            } else {
                None
            }
        }
        Opcode::ConstU64 => {
            if let InstData::ConstU64(v) = inst.data {
                i64::try_from(v).ok()
            } else {
                None
            }
        }
        Opcode::GetArg => {
            if let InstData::ArgIndex(idx) = inst.data {
                call_site
                    .arg_values
                    .get(idx as usize)?
                    .as_ref()?
                    .parse::<i64>()
                    .ok()
            } else {
                None
            }
        }
        _ => fold_binary_i64(inst, |id| {
            eval_callee_i64_for_call_site(id, inst_by_id, call_site)
        }),
    }
}

/// Fold an instruction to a `bool` in the *callee* context (comparisons and
/// logical connectives over the i64 evaluator, plus `ConstBool` / `GetArg`).
pub(crate) fn eval_callee_bool_for_call_site(
    inst_id: u32,
    inst_by_id: &HashMap<u32, &vow_ir::Inst>,
    call_site: &CallSiteInfo,
) -> Option<bool> {
    use vow_ir::{InstData, Opcode};
    let inst = *inst_by_id.get(&inst_id)?;
    match inst.opcode {
        Opcode::ConstBool => {
            if let InstData::ConstBool(v) = inst.data {
                Some(v)
            } else {
                None
            }
        }
        Opcode::GetArg => {
            if let InstData::ArgIndex(idx) = inst.data {
                match call_site.arg_values.get(idx as usize)?.as_deref()? {
                    "true" => Some(true),
                    "false" => Some(false),
                    _ => None,
                }
            } else {
                None
            }
        }
        Opcode::EqI32 | Opcode::EqI64 => {
            let lhs = eval_callee_i64_for_call_site(inst.args.first()?.0, inst_by_id, call_site)?;
            let rhs = eval_callee_i64_for_call_site(inst.args.get(1)?.0, inst_by_id, call_site)?;
            Some(lhs == rhs)
        }
        Opcode::NeI32 | Opcode::NeI64 => {
            let lhs = eval_callee_i64_for_call_site(inst.args.first()?.0, inst_by_id, call_site)?;
            let rhs = eval_callee_i64_for_call_site(inst.args.get(1)?.0, inst_by_id, call_site)?;
            Some(lhs != rhs)
        }
        Opcode::LtI32 | Opcode::LtI64 => {
            let lhs = eval_callee_i64_for_call_site(inst.args.first()?.0, inst_by_id, call_site)?;
            let rhs = eval_callee_i64_for_call_site(inst.args.get(1)?.0, inst_by_id, call_site)?;
            Some(lhs < rhs)
        }
        Opcode::LeI32 | Opcode::LeI64 => {
            let lhs = eval_callee_i64_for_call_site(inst.args.first()?.0, inst_by_id, call_site)?;
            let rhs = eval_callee_i64_for_call_site(inst.args.get(1)?.0, inst_by_id, call_site)?;
            Some(lhs <= rhs)
        }
        Opcode::GtI32 | Opcode::GtI64 => {
            let lhs = eval_callee_i64_for_call_site(inst.args.first()?.0, inst_by_id, call_site)?;
            let rhs = eval_callee_i64_for_call_site(inst.args.get(1)?.0, inst_by_id, call_site)?;
            Some(lhs > rhs)
        }
        Opcode::GeI32 | Opcode::GeI64 => {
            let lhs = eval_callee_i64_for_call_site(inst.args.first()?.0, inst_by_id, call_site)?;
            let rhs = eval_callee_i64_for_call_site(inst.args.get(1)?.0, inst_by_id, call_site)?;
            Some(lhs >= rhs)
        }
        Opcode::Not => {
            let value =
                eval_callee_bool_for_call_site(inst.args.first()?.0, inst_by_id, call_site)?;
            Some(!value)
        }
        Opcode::And => {
            let lhs = eval_callee_bool_for_call_site(inst.args.first()?.0, inst_by_id, call_site)?;
            let rhs = eval_callee_bool_for_call_site(inst.args.get(1)?.0, inst_by_id, call_site)?;
            Some(lhs && rhs)
        }
        Opcode::Or => {
            let lhs = eval_callee_bool_for_call_site(inst.args.first()?.0, inst_by_id, call_site)?;
            let rhs = eval_callee_bool_for_call_site(inst.args.get(1)?.0, inst_by_id, call_site)?;
            Some(lhs || rhs)
        }
        _ => None,
    }
}

/// Fold a call argument's instruction subtree to an `i64` literal for the
/// call-site index. No call site: `GetArg` is unresolvable, and `ConstU64` is
/// intentionally not folded here (a nested `ConstU64` therefore leaves the whole
/// expression unfolded).
pub(crate) fn eval_const_i64_for_counterexample(
    inst_id: u32,
    inst_by_id: &HashMap<u32, &vow_ir::Inst>,
) -> Option<i64> {
    use vow_ir::{InstData, Opcode};
    let inst = *inst_by_id.get(&inst_id)?;
    match inst.opcode {
        Opcode::ConstI32 => {
            if let InstData::ConstI32(v) = inst.data {
                Some(v as i64)
            } else {
                None
            }
        }
        Opcode::ConstI64 => {
            if let InstData::ConstI64(v) = inst.data {
                Some(v)
            } else {
                None
            }
        }
        _ => fold_binary_i64(inst, |id| eval_const_i64_for_counterexample(id, inst_by_id)),
    }
}

/// Render a single constant instruction's value as its display string, or `None`
/// if the instruction is not a supported constant.
pub(crate) fn inst_constant_value_for_counterexample(inst: &vow_ir::Inst) -> Option<String> {
    use vow_ir::{InstData, Opcode};
    match inst.opcode {
        Opcode::ConstI32 => {
            if let InstData::ConstI32(v) = inst.data {
                Some(v.to_string())
            } else {
                None
            }
        }
        Opcode::ConstI64 => {
            if let InstData::ConstI64(v) = inst.data {
                Some(v.to_string())
            } else {
                None
            }
        }
        Opcode::ConstU64 => {
            if let InstData::ConstU64(v) = inst.data {
                Some(v.to_string())
            } else {
                None
            }
        }
        Opcode::ConstBool => {
            if let InstData::ConstBool(v) = inst.data {
                Some(v.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vow_ir::{Inst, InstData, InstId, Opcode, RegionId, Ty};
    use vow_syntax::span::Span;

    fn inst(id: u32, opcode: Opcode, args: &[u32], data: InstData) -> Inst {
        Inst {
            id: InstId(id),
            opcode,
            ty: Ty::I64,
            args: args.iter().copied().map(InstId).collect(),
            data,
            origin: Span::new(0, 0),
            region: RegionId::Root,
        }
    }

    fn index(insts: &[Inst]) -> std::collections::HashMap<u32, &Inst> {
        insts.iter().map(|i| (i.id.0, i)).collect()
    }

    fn call_site(arg_values: Vec<Option<String>>) -> CallSiteInfo {
        CallSiteInfo {
            caller_function: "caller".to_string(),
            file: "test.vow".to_string(),
            offset: 0,
            length: 0,
            arg_spans: vec![],
            arg_values,
        }
    }

    // ---- inst_constant_value_for_counterexample --------------------------

    #[test]
    fn const_value_renders_each_scalar_kind() {
        assert_eq!(
            inst_constant_value_for_counterexample(&inst(
                0,
                Opcode::ConstI32,
                &[],
                InstData::ConstI32(-5)
            )),
            Some("-5".to_string())
        );
        assert_eq!(
            inst_constant_value_for_counterexample(&inst(
                0,
                Opcode::ConstI64,
                &[],
                InstData::ConstI64(42)
            )),
            Some("42".to_string())
        );
        assert_eq!(
            inst_constant_value_for_counterexample(&inst(
                0,
                Opcode::ConstU64,
                &[],
                InstData::ConstU64(u64::MAX)
            )),
            Some("18446744073709551615".to_string())
        );
        assert_eq!(
            inst_constant_value_for_counterexample(&inst(
                0,
                Opcode::ConstBool,
                &[],
                InstData::ConstBool(true)
            )),
            Some("true".to_string())
        );
    }

    #[test]
    fn const_value_is_none_for_non_constant() {
        assert_eq!(
            inst_constant_value_for_counterexample(&inst(
                0,
                Opcode::GetArg,
                &[],
                InstData::ArgIndex(0)
            )),
            None
        );
    }

    // ---- eval_const_i64_for_counterexample -------------------------------

    #[test]
    fn counterexample_folds_i32_and_i64_constants() {
        let insts = [
            inst(0, Opcode::ConstI32, &[], InstData::ConstI32(7)),
            inst(1, Opcode::ConstI64, &[], InstData::ConstI64(-9)),
        ];
        let map = index(&insts);
        assert_eq!(eval_const_i64_for_counterexample(0, &map), Some(7));
        assert_eq!(eval_const_i64_for_counterexample(1, &map), Some(-9));
    }

    #[test]
    fn counterexample_does_not_fold_u64_or_getarg() {
        // The counterexample path deliberately omits ConstU64 and GetArg; those
        // are the callee evaluator's responsibility. Pinning this guards the
        // shared-arithmetic dedup from silently widening the leaf set.
        let insts = [
            inst(0, Opcode::ConstU64, &[], InstData::ConstU64(3)),
            inst(1, Opcode::GetArg, &[], InstData::ArgIndex(0)),
        ];
        let map = index(&insts);
        assert_eq!(eval_const_i64_for_counterexample(0, &map), None);
        assert_eq!(eval_const_i64_for_counterexample(1, &map), None);
    }

    #[test]
    fn counterexample_folds_arithmetic_with_wraparound() {
        let insts = [
            inst(0, Opcode::ConstI64, &[], InstData::ConstI64(2)),
            inst(1, Opcode::ConstI64, &[], InstData::ConstI64(3)),
            inst(2, Opcode::WrappingAddI64, &[0, 1], InstData::None),
            inst(3, Opcode::ConstI64, &[], InstData::ConstI64(10)),
            inst(4, Opcode::ConstI64, &[], InstData::ConstI64(4)),
            inst(5, Opcode::CheckedSubI64, &[3, 4], InstData::None),
            inst(6, Opcode::ConstI64, &[], InstData::ConstI64(6)),
            inst(7, Opcode::ConstI64, &[], InstData::ConstI64(7)),
            inst(8, Opcode::WrappingMulI64, &[6, 7], InstData::None),
            inst(9, Opcode::ConstI64, &[], InstData::ConstI64(i64::MAX)),
            inst(10, Opcode::ConstI64, &[], InstData::ConstI64(1)),
            inst(11, Opcode::WrappingAddI64, &[9, 10], InstData::None),
        ];
        let map = index(&insts);
        assert_eq!(eval_const_i64_for_counterexample(2, &map), Some(5));
        assert_eq!(eval_const_i64_for_counterexample(5, &map), Some(6));
        assert_eq!(eval_const_i64_for_counterexample(8, &map), Some(42));
        assert_eq!(eval_const_i64_for_counterexample(11, &map), Some(i64::MIN));
    }

    #[test]
    fn counterexample_folds_nested_arithmetic() {
        let insts = [
            inst(0, Opcode::ConstI64, &[], InstData::ConstI64(2)),
            inst(1, Opcode::ConstI64, &[], InstData::ConstI64(3)),
            inst(2, Opcode::WrappingMulI64, &[0, 1], InstData::None),
            inst(3, Opcode::ConstI64, &[], InstData::ConstI64(4)),
            inst(4, Opcode::WrappingAddI64, &[2, 3], InstData::None),
        ];
        let map = index(&insts);
        assert_eq!(eval_const_i64_for_counterexample(4, &map), Some(10));
    }

    #[test]
    fn counterexample_arithmetic_over_u64_operand_is_none() {
        // A ConstU64 operand does not fold in the counterexample path, so the
        // whole arithmetic expression stays unfolded — the behaviour the shared
        // fold must preserve.
        let insts = [
            inst(0, Opcode::ConstU64, &[], InstData::ConstU64(2)),
            inst(1, Opcode::ConstI64, &[], InstData::ConstI64(3)),
            inst(2, Opcode::WrappingAddI64, &[0, 1], InstData::None),
        ];
        let map = index(&insts);
        assert_eq!(eval_const_i64_for_counterexample(2, &map), None);
    }

    #[test]
    fn counterexample_is_none_for_unknown_op_or_missing_operand() {
        let insts = [
            inst(0, Opcode::Call, &[], InstData::None),
            inst(1, Opcode::WrappingAddI64, &[0, 99], InstData::None),
        ];
        let map = index(&insts);
        assert_eq!(eval_const_i64_for_counterexample(0, &map), None);
        assert_eq!(eval_const_i64_for_counterexample(1, &map), None);
        assert_eq!(eval_const_i64_for_counterexample(99, &map), None);
    }

    // ---- eval_callee_i64_for_call_site -----------------------------------

    #[test]
    fn callee_i64_folds_u64_that_fits_and_rejects_overflow() {
        let fits = [inst(0, Opcode::ConstU64, &[], InstData::ConstU64(5))];
        let map = index(&fits);
        assert_eq!(
            eval_callee_i64_for_call_site(0, &map, &call_site(vec![])),
            Some(5)
        );
        let too_big = [inst(0, Opcode::ConstU64, &[], InstData::ConstU64(u64::MAX))];
        let map = index(&too_big);
        assert_eq!(
            eval_callee_i64_for_call_site(0, &map, &call_site(vec![])),
            None
        );
    }

    #[test]
    fn callee_i64_resolves_getarg_from_call_site() {
        let insts = [inst(0, Opcode::GetArg, &[], InstData::ArgIndex(0))];
        let map = index(&insts);
        assert_eq!(
            eval_callee_i64_for_call_site(0, &map, &call_site(vec![Some("7".to_string())])),
            Some(7)
        );
        // out of range, absent, and non-numeric all yield None.
        assert_eq!(
            eval_callee_i64_for_call_site(0, &map, &call_site(vec![])),
            None
        );
        assert_eq!(
            eval_callee_i64_for_call_site(0, &map, &call_site(vec![None])),
            None
        );
        assert_eq!(
            eval_callee_i64_for_call_site(0, &map, &call_site(vec![Some("nope".to_string())])),
            None
        );
    }

    #[test]
    fn callee_i64_folds_arithmetic_over_getarg_and_const() {
        let insts = [
            inst(0, Opcode::GetArg, &[], InstData::ArgIndex(0)),
            inst(1, Opcode::ConstI64, &[], InstData::ConstI64(3)),
            inst(2, Opcode::WrappingAddI64, &[0, 1], InstData::None),
        ];
        let map = index(&insts);
        assert_eq!(
            eval_callee_i64_for_call_site(2, &map, &call_site(vec![Some("5".to_string())])),
            Some(8)
        );
    }

    // ---- eval_callee_bool_for_call_site ----------------------------------

    #[test]
    fn callee_bool_handles_const_and_getarg() {
        let cb = [inst(0, Opcode::ConstBool, &[], InstData::ConstBool(true))];
        let map = index(&cb);
        assert_eq!(
            eval_callee_bool_for_call_site(0, &map, &call_site(vec![])),
            Some(true)
        );
        let ga = [inst(0, Opcode::GetArg, &[], InstData::ArgIndex(0))];
        let map = index(&ga);
        assert_eq!(
            eval_callee_bool_for_call_site(0, &map, &call_site(vec![Some("true".to_string())])),
            Some(true)
        );
        assert_eq!(
            eval_callee_bool_for_call_site(0, &map, &call_site(vec![Some("false".to_string())])),
            Some(false)
        );
        assert_eq!(
            eval_callee_bool_for_call_site(0, &map, &call_site(vec![Some("1".to_string())])),
            None
        );
    }

    #[test]
    fn callee_bool_evaluates_comparisons() {
        let base = |op: Opcode, a: i64, b: i64| {
            let insts = [
                inst(0, Opcode::ConstI64, &[], InstData::ConstI64(a)),
                inst(1, Opcode::ConstI64, &[], InstData::ConstI64(b)),
                inst(2, op, &[0, 1], InstData::None),
            ];
            let map = index(&insts);
            eval_callee_bool_for_call_site(2, &map, &call_site(vec![]))
        };
        assert_eq!(base(Opcode::EqI64, 3, 3), Some(true));
        assert_eq!(base(Opcode::EqI64, 3, 4), Some(false));
        assert_eq!(base(Opcode::NeI64, 3, 4), Some(true));
        assert_eq!(base(Opcode::LtI64, 3, 4), Some(true));
        assert_eq!(base(Opcode::LeI64, 4, 4), Some(true));
        assert_eq!(base(Opcode::GtI64, 5, 4), Some(true));
        assert_eq!(base(Opcode::GeI64, 4, 4), Some(true));
    }

    #[test]
    fn callee_bool_evaluates_logical_connectives() {
        let t = inst(0, Opcode::ConstBool, &[], InstData::ConstBool(true));
        let f = inst(1, Opcode::ConstBool, &[], InstData::ConstBool(false));
        let not = inst(2, Opcode::Not, &[1], InstData::None);
        let and = inst(3, Opcode::And, &[0, 1], InstData::None);
        let or = inst(4, Opcode::Or, &[1, 0], InstData::None);
        let insts = [t, f, not, and, or];
        let map = index(&insts);
        let cs = call_site(vec![]);
        assert_eq!(eval_callee_bool_for_call_site(2, &map, &cs), Some(true));
        assert_eq!(eval_callee_bool_for_call_site(3, &map, &cs), Some(false));
        assert_eq!(eval_callee_bool_for_call_site(4, &map, &cs), Some(true));
    }

    #[test]
    fn callee_bool_compares_getarg_against_const() {
        let insts = [
            inst(0, Opcode::GetArg, &[], InstData::ArgIndex(0)),
            inst(1, Opcode::ConstI64, &[], InstData::ConstI64(5)),
            inst(2, Opcode::LtI64, &[0, 1], InstData::None),
        ];
        let map = index(&insts);
        assert_eq!(
            eval_callee_bool_for_call_site(2, &map, &call_site(vec![Some("3".to_string())])),
            Some(true)
        );
    }

    #[test]
    fn callee_bool_is_none_for_unknown_op() {
        let insts = [inst(0, Opcode::Call, &[], InstData::None)];
        let map = index(&insts);
        assert_eq!(
            eval_callee_bool_for_call_site(0, &map, &call_site(vec![])),
            None
        );
    }
}
