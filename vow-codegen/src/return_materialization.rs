//! Return-materialization analysis (spec §5.1).
//!
//! A deep module answering one question — *does a `FreshInCaller`
//! function's return value need to be deep-copied into the caller's
//! `target_region` before the return edge?* — behind a two-function
//! interface. All of the IR-walk complexity (transitive `Phi`/`Upsilon`
//! traversal, the `__vow_string_literal` descriptor recognition, and the
//! memory-safety "all leaves must be clone-safe" rule) is hidden here.
//!
//! Callers in `cranelift_backend`:
//! - [`module_uses_return_materialization`] gates importing
//!   `__vow_string_clone_into_arena` at module-load time.
//! - [`return_source_needs_materialization`] decides, per `Return`
//!   during lowering, whether to emit the clone call.

use crate::cranelift_backend::build_inst_index;
use std::collections::{HashMap, HashSet};
use vow_ir::{
    Function as IrFunction, Inst, InstData, InstId, Opcode, RegionConstraint, Ty as IrTy,
};

/// True when the function may emit a return-materialisation clone call —
/// i.e. it has `return_region == FreshInCaller` and at least one `Return`
/// inst whose source path reaches a `.rodata` literal or a heap-typed
/// parameter alias. Used at module-load time to decide whether to import
/// `__vow_string_clone_into_arena`.
pub(crate) fn module_uses_return_materialization(func: &IrFunction) -> bool {
    if func.summary.return_region != RegionConstraint::FreshInCaller {
        return false;
    }
    if func.return_ty != IrTy::Ptr {
        return false;
    }
    let inst_index = build_inst_index(func);
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Return
                && let Some(&val_id) = inst.args.first()
                && return_source_needs_materialization(func, &inst_index, val_id)
            {
                return true;
            }
        }
    }
    false
}

fn is_string_literal_descriptor_call(inst: &Inst, inst_index: &HashMap<InstId, &Inst>) -> bool {
    if !matches!(
        &inst.data,
        InstData::CallExtern(sym) if sym == "__vow_string_literal"
    ) {
        return false;
    }
    let Some(cstr_id) = inst.args.first() else {
        return false;
    };
    inst_index
        .get(cstr_id)
        .is_some_and(|arg| arg.opcode == Opcode::ConstStr)
}

/// Decide whether a `FreshInCaller` function's return value needs to be
/// deep-copied into `target_region` to satisfy the §5.1 representation
/// promise.
///
/// Walks the value's source transitively through `Phi` (via `Upsilon`
/// arms). Returns true iff at least one reachable leaf is a
/// `__vow_string_literal(literal)` call AND every reachable leaf is
/// the same shape (a known-safe `VowString` / `Vec<u8>` descriptor
/// producer).
///
/// **Why `__vow_string_literal`, not `ConstStr` directly.**
/// `ConstStr` lowers to a pointer to a NUL-terminated byte blob in a
/// `.rodata` data section — it is *not* a `VowVec` descriptor.
/// `__vow_string_clone_into_arena` reads `(ptr, len, cap)` from its
/// source and copies `len` bytes; firing it on a raw `ConstStr` would
/// reinterpret arbitrary literal bytes as `{ptr, len, cap}` and corrupt
/// memory. The Vow lowerer wraps every `String` literal in
/// `Call __vow_string_literal(ConstStr)` (`vow-ir/src/lower/mod.rs`
/// `Lit::String`), and codegen turns that synthetic call into a static
/// `VowVec` descriptor — that's the shape we recognise here.
///
/// **Why the all-leaves rule still applies.** A Phi mixing a
/// `__vow_string_literal` arm with any other leaf shape (a
/// `RegionAlloc`, a `GetArg` of a heap-typed param, a generic call,
/// …) cannot be safely cloned via the `VowString`-typed primitive: at
/// runtime the Phi-selected value might not have descriptor layout.
/// Mixed Phis fall back to no-clone; correctness over §5.1
/// compliance for those paths. A generic deep-clone intrinsic in
/// Phase 7 / #202 lifts the restriction.
///
/// `GetArg`, `RegionAlloc`, plain `Call`s, and other leaves disqualify
/// the walk for the same reason — their layout isn't statically
/// known to be `VowString`-shaped.
pub(crate) fn return_source_needs_materialization(
    func: &IrFunction,
    inst_index: &HashMap<InstId, &Inst>,
    return_val: InstId,
) -> bool {
    let mut visited: HashSet<InstId> = HashSet::new();
    let mut stack: Vec<InstId> = vec![return_val];
    let mut saw_safe_leaf = false;
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        let Some(&src) = inst_index.get(&id) else {
            // Unknown source — be conservative.
            return false;
        };
        match src.opcode {
            Opcode::Call if is_string_literal_descriptor_call(src, inst_index) => {
                saw_safe_leaf = true;
            }
            Opcode::Phi => {
                // Walk every Upsilon arm that targets this Phi. Upsilons
                // can live in any block, so we iterate the whole function
                // looking for matching `PhiTarget(id)` writes.
                //
                // Complexity: O(n_insts) per visited Phi. `inst_index`
                // doesn't help (it's keyed by InstId, not by PhiTarget),
                // and the existing `PhiUpsilonData` pre-pass indexes
                // Upsilons by source block, not by Phi ID, so it can't
                // be reused either. Phase 7 / #202 — when materialisation
                // becomes load-bearing on synthesised functions with
                // deeper Phi chains — should add a `phi_id → [arm_ids]`
                // inverted index (either inside `PhiUpsilonData` or as a
                // standalone helper) to bring this to O(k_arms) per Phi.
                // For Phase 4 this scan rarely fires (materialisation
                // requires a `FreshInCaller` summary AND a
                // `__vow_string_literal` leaf, which only String-literal
                // returns produce).
                for block in &func.blocks {
                    for inst in &block.insts {
                        if inst.opcode == Opcode::Upsilon
                            && let InstData::PhiTarget(target) = inst.data
                            && target == id
                            && let Some(&arm_id) = inst.args.first()
                        {
                            stack.push(arm_id);
                        }
                    }
                }
            }
            // Any other leaf shape disqualifies the walk. This includes
            // raw `ConstStr` (a `.rodata` byte blob, NOT a `VowVec`
            // descriptor — running the clone runtime on it would
            // corrupt memory).
            _ => return false,
        }
    }
    saw_safe_leaf
}

#[cfg(test)]
mod tests {
    use super::{module_uses_return_materialization, return_source_needs_materialization};
    use crate::cranelift_backend::build_inst_index;
    use std::collections::HashMap;
    use vow_ir::{
        BasicBlock, BlockId, FuncId, Function, Inst, InstData, InstId, Opcode, RegionConstraint,
        RegionId, RegionSummary, Ty,
    };
    use vow_syntax::span::Span;

    fn inst(id: u32, op: Opcode, ty: Ty, args: Vec<u32>, data: InstData) -> Inst {
        Inst {
            id: InstId(id),
            opcode: op,
            ty,
            args: args.into_iter().map(InstId).collect(),
            data,
            origin: Span::new(0, 0),
            region: RegionId::Root,
        }
    }

    /// A `__vow_string_literal(ConstStr)` descriptor call — the shape the
    /// Vow lowerer produces for a String literal. `cstr_id`/`call_id` are
    /// the two instruction ids; the call is the clone-safe leaf.
    fn descriptor_call(cstr_id: u32, call_id: u32) -> Vec<Inst> {
        vec![
            inst(
                cstr_id,
                Opcode::ConstStr,
                Ty::Ptr,
                vec![],
                InstData::ConstStr(0),
            ),
            inst(
                call_id,
                Opcode::Call,
                Ty::Ptr,
                vec![cstr_id],
                InstData::CallExtern("__vow_string_literal".to_string()),
            ),
        ]
    }

    /// Wrap instructions in a single-block function with the given return
    /// shape. All fixtures put every instruction in block 0, which is
    /// sufficient for both the Phi/Upsilon scan and the module-load scan.
    fn func(return_ty: Ty, return_region: RegionConstraint, insts: Vec<Inst>) -> Function {
        Function {
            id: FuncId(0),
            name: "fixture".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts,
            }],
            local_names: HashMap::new(),
            summary: RegionSummary {
                param_regions: vec![],
                return_region,
                store_effects: vec![],
            },
            source_file: String::new(),
        }
    }

    // -- return_source_needs_materialization: leaf shapes --------------

    #[test]
    fn descriptor_call_leaf_needs_materialization() {
        let f = func(
            Ty::Ptr,
            RegionConstraint::FreshInCaller,
            descriptor_call(1, 2),
        );
        let index = build_inst_index(&f);
        assert!(return_source_needs_materialization(&f, &index, InstId(2)));
    }

    #[test]
    fn raw_const_str_leaf_rejected() {
        // A raw ConstStr is a `.rodata` c-string, NOT a VowVec descriptor.
        // Cloning it would reinterpret literal bytes as {ptr,len,cap}.
        let f = func(
            Ty::Ptr,
            RegionConstraint::FreshInCaller,
            vec![inst(
                1,
                Opcode::ConstStr,
                Ty::Ptr,
                vec![],
                InstData::ConstStr(0),
            )],
        );
        let index = build_inst_index(&f);
        assert!(!return_source_needs_materialization(&f, &index, InstId(1)));
    }

    #[test]
    fn region_alloc_leaf_rejected() {
        let f = func(
            Ty::Ptr,
            RegionConstraint::FreshInCaller,
            vec![inst(
                1,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 24, align: 8 },
            )],
        );
        let index = build_inst_index(&f);
        assert!(!return_source_needs_materialization(&f, &index, InstId(1)));
    }

    #[test]
    fn unknown_source_id_is_conservative() {
        // The return_val is not present in the index — be conservative.
        let f = func(Ty::Ptr, RegionConstraint::FreshInCaller, vec![]);
        let index = build_inst_index(&f);
        assert!(!return_source_needs_materialization(&f, &index, InstId(99)));
    }

    // -- return_source_needs_materialization: Phi walk ----------------

    /// Phi(id=10) fed by two Upsilon arms (ids 20/21) writing the two
    /// descriptor calls. All-safe leaves → materialise.
    #[test]
    fn phi_all_descriptor_arms_needs_materialization() {
        let mut insts = descriptor_call(1, 2);
        insts.extend(descriptor_call(3, 4));
        insts.push(inst(10, Opcode::Phi, Ty::Ptr, vec![], InstData::None));
        insts.push(inst(
            20,
            Opcode::Upsilon,
            Ty::Unit,
            vec![2],
            InstData::PhiTarget(InstId(10)),
        ));
        insts.push(inst(
            21,
            Opcode::Upsilon,
            Ty::Unit,
            vec![4],
            InstData::PhiTarget(InstId(10)),
        ));
        let f = func(Ty::Ptr, RegionConstraint::FreshInCaller, insts);
        let index = build_inst_index(&f);
        assert!(return_source_needs_materialization(&f, &index, InstId(10)));
    }

    /// Phi mixing a descriptor arm (safe) with a raw ConstStr arm
    /// (unsafe). The unsafe leaf disqualifies the whole walk.
    #[test]
    fn phi_mixed_arms_rejected() {
        let mut insts = descriptor_call(1, 2);
        insts.push(inst(
            3,
            Opcode::ConstStr,
            Ty::Ptr,
            vec![],
            InstData::ConstStr(0),
        ));
        insts.push(inst(10, Opcode::Phi, Ty::Ptr, vec![], InstData::None));
        insts.push(inst(
            20,
            Opcode::Upsilon,
            Ty::Unit,
            vec![2],
            InstData::PhiTarget(InstId(10)),
        ));
        insts.push(inst(
            21,
            Opcode::Upsilon,
            Ty::Unit,
            vec![3],
            InstData::PhiTarget(InstId(10)),
        ));
        let f = func(Ty::Ptr, RegionConstraint::FreshInCaller, insts);
        let index = build_inst_index(&f);
        assert!(!return_source_needs_materialization(&f, &index, InstId(10)));
    }

    /// A Phi whose only arm targets itself must terminate (the visited
    /// set breaks the cycle) and, with no safe leaf reached, report false.
    #[test]
    fn self_referential_phi_terminates_and_is_rejected() {
        let insts = vec![
            inst(10, Opcode::Phi, Ty::Ptr, vec![], InstData::None),
            inst(
                20,
                Opcode::Upsilon,
                Ty::Unit,
                vec![10],
                InstData::PhiTarget(InstId(10)),
            ),
        ];
        let f = func(Ty::Ptr, RegionConstraint::FreshInCaller, insts);
        let index = build_inst_index(&f);
        assert!(!return_source_needs_materialization(&f, &index, InstId(10)));
    }

    // -- module_uses_return_materialization: the module-load gate ------

    #[test]
    fn fresh_in_caller_descriptor_return_uses_materialization() {
        let mut insts = descriptor_call(0, 1);
        insts.push(inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None));
        let f = func(Ty::Ptr, RegionConstraint::FreshInCaller, insts);
        assert!(module_uses_return_materialization(&f));
    }

    #[test]
    fn fresh_in_caller_region_alloc_return_no_materialization() {
        let insts = vec![
            inst(
                0,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 24, align: 8 },
            ),
            inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
        ];
        let f = func(Ty::Ptr, RegionConstraint::FreshInCaller, insts);
        assert!(!module_uses_return_materialization(&f));
    }

    #[test]
    fn non_ptr_return_type_skips_materialization() {
        // Same descriptor return, but a non-Ptr return type never
        // materialises — the early-out fires before the IR walk.
        let mut insts = descriptor_call(0, 1);
        insts.push(inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None));
        let f = func(Ty::I64, RegionConstraint::FreshInCaller, insts);
        assert!(!module_uses_return_materialization(&f));
    }

    #[test]
    fn non_fresh_in_caller_skips_materialization() {
        // ConstantGlobal (the default summary) never materialises, even
        // with a descriptor return path.
        let mut insts = descriptor_call(0, 1);
        insts.push(inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None));
        let f = func(Ty::Ptr, RegionConstraint::ConstantGlobal, insts);
        assert!(!module_uses_return_materialization(&f));
    }
}
