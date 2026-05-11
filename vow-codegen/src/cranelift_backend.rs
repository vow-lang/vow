use cranelift_codegen::Context;
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    AbiParam, Block, BlockArg, FuncRef, GlobalValue, InstBuilder, MemFlags, Signature, StackSlot,
    StackSlotData, StackSlotKind, TrapCode, Value, types,
};
use cranelift_codegen::isa::TargetIsa;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{
    DataDescription, DataId, FuncId as CraneliftFuncId, Linkage, Module as CraneliftModule,
};
use cranelift_object::{ObjectBuilder, ObjectModule};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use vow_ir::{
    BlockId, FuncId as IrFuncId, Function as IrFunction, HiddenRegionIdx, Inst, InstData, InstId,
    Module as IrModule, Opcode, RegionConstraint, RegionId, RegionSummary, Ty as IrTy,
};

use crate::{Backend, BuildMode, CodegenError, CompiledObject, TraceMode};

pub struct CraneliftBackend;

impl CraneliftBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CraneliftBackend {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Phi / Upsilon pre-pass data
// ---------------------------------------------------------------------------

struct PhiUpsilonData {
    // BlockId → ordered list of Phi InstIds (one per block param)
    block_phis: HashMap<BlockId, Vec<InstId>>,
    // Phi InstId → BlockId that contains it
    phi_home: HashMap<InstId, BlockId>,
    // Source BlockId → [(phi_id, value_inst_id)]
    block_upsilons: HashMap<BlockId, Vec<(InstId, InstId)>>,
}

fn build_phi_upsilon_data(ir_func: &IrFunction) -> PhiUpsilonData {
    let mut block_phis: HashMap<BlockId, Vec<InstId>> = HashMap::new();
    let mut phi_home: HashMap<InstId, BlockId> = HashMap::new();
    let mut block_upsilons: HashMap<BlockId, Vec<(InstId, InstId)>> = HashMap::new();

    for block in &ir_func.blocks {
        for inst in &block.insts {
            match inst.opcode {
                Opcode::Phi if ir_ty_to_cranelift(inst.ty).is_some() => {
                    block_phis.entry(block.id).or_default().push(inst.id);
                    phi_home.insert(inst.id, block.id);
                }
                Opcode::Upsilon => {
                    if let InstData::PhiTarget(phi_id) = inst.data
                        && let Some(&val_id) = inst.args.first()
                    {
                        block_upsilons
                            .entry(block.id)
                            .or_default()
                            .push((phi_id, val_id));
                    }
                }
                _ => {}
            }
        }
    }

    if std::env::var("VOW_DEBUG_IR").is_ok() {
        for block in &ir_func.blocks {
            eprintln!("IR block {:?}:", block.id);
            for inst in &block.insts {
                eprintln!(
                    "  {:?} {:?} args={:?} data={:?}",
                    inst.id, inst.opcode, inst.args, inst.data
                );
            }
        }
        eprintln!("block_phis: {:?}", block_phis);
        eprintln!("block_upsilons: {:?}", block_upsilons);
    }

    PhiUpsilonData {
        block_phis,
        phi_home,
        block_upsilons,
    }
}

fn collect_target_block_args(
    from_block_id: BlockId,
    to_block_id: BlockId,
    phi_data: &PhiUpsilonData,
    value_map: &HashMap<InstId, Value>,
) -> Vec<BlockArg> {
    let Some(phi_ids) = phi_data.block_phis.get(&to_block_id) else {
        return vec![];
    };
    let upsilons = phi_data
        .block_upsilons
        .get(&from_block_id)
        .map(|v| v.as_slice())
        .unwrap_or(&[]);

    let upsil_map: HashMap<InstId, InstId> = upsilons
        .iter()
        .filter(|(phi_id, _)| {
            phi_data
                .phi_home
                .get(phi_id)
                .is_some_and(|&b| b == to_block_id)
        })
        .map(|&(phi_id, val_id)| (phi_id, val_id))
        .collect();

    phi_ids
        .iter()
        .filter_map(|phi_id| {
            upsil_map
                .get(phi_id)
                .and_then(|val_id| value_map.get(val_id).copied())
                .map(BlockArg::Value)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// ISA and type helpers
// ---------------------------------------------------------------------------

fn make_isa(mode: BuildMode) -> Result<Arc<dyn TargetIsa>, CodegenError> {
    let mut flag_builder = settings::builder();
    flag_builder
        .set("use_colocated_libcalls", "false")
        .map_err(|e| CodegenError::IsaBuild(e.to_string()))?;
    flag_builder
        .set("is_pic", "true")
        .map_err(|e| CodegenError::IsaBuild(e.to_string()))?;
    if mode == BuildMode::Release || mode == BuildMode::Profile {
        flag_builder
            .set("opt_level", "speed")
            .map_err(|e| CodegenError::IsaBuild(e.to_string()))?;
    }
    let flags = settings::Flags::new(flag_builder);
    cranelift_native::builder()
        .map_err(|e| CodegenError::IsaBuild(e.to_string()))?
        .finish(flags)
        .map_err(|e| CodegenError::IsaBuild(e.to_string()))
}

fn ir_ty_to_cranelift(ty: IrTy) -> Option<types::Type> {
    match ty {
        IrTy::I32 => Some(types::I32),
        IrTy::I64 => Some(types::I64),
        IrTy::F32 => Some(types::F32),
        IrTy::F64 => Some(types::F64),
        IrTy::Bool => Some(types::I64),
        IrTy::U64 => Some(types::I64),
        IrTy::Unit => None,
        IrTy::Ptr | IrTy::LinearPtr => Some(types::I64),
    }
}

fn signature_return_ty(ir_func: &IrFunction) -> Option<types::Type> {
    if ir_func.name == "main" && ir_func.return_ty == IrTy::Unit {
        Some(types::I32)
    } else {
        ir_ty_to_cranelift(ir_func.return_ty)
    }
}

fn build_signature(ir_func: &IrFunction, call_conv: cranelift_codegen::isa::CallConv) -> Signature {
    let mut sig = Signature::new(call_conv);
    for &param_ty in &ir_func.params {
        if let Some(cl_ty) = ir_ty_to_cranelift(param_ty) {
            sig.params.push(AbiParam::new(cl_ty));
        }
    }
    for _ in 0..hidden_region_param_count(ir_func) {
        sig.params.push(AbiParam::new(types::I64));
    }
    if let Some(cl_ty) = signature_return_ty(ir_func) {
        sig.returns.push(AbiParam::new(cl_ty));
    }
    sig
}

fn hidden_region_store_targets(ir_func: &IrFunction) -> Vec<u32> {
    // Keep this slot order in sync with compiler/clif.vow's clif_hidden_store_targets.
    // The self-hosted path dedups before sorting; both paths must return sorted unique targets.
    let mut targets: Vec<u32> = ir_func
        .summary
        .store_effects
        .iter()
        .map(|effect| effect.target)
        .collect();
    targets.sort_unstable();
    targets.dedup();
    targets
}

fn hidden_region_param_count(ir_func: &IrFunction) -> usize {
    if ir_func.name == "main" {
        return 0;
    }
    let mut count = 0;
    if ir_func.summary.return_region == RegionConstraint::FreshInCaller {
        count += 1;
    }
    count + hidden_region_store_targets(ir_func).len()
}

fn hidden_region_idx_for_store_target(
    summary: &RegionSummary,
    target_param: u32,
) -> Option<HiddenRegionIdx> {
    let mut idx = 0u32;
    if summary.return_region == RegionConstraint::FreshInCaller {
        idx += 1;
    }
    let mut targets: Vec<u32> = summary
        .store_effects
        .iter()
        .map(|effect| effect.target)
        .collect();
    targets.sort_unstable();
    targets.dedup();
    for target in targets {
        if target == target_param {
            return Some(HiddenRegionIdx(idx));
        }
        idx += 1;
    }
    None
}

fn coerce_return_value(builder: &mut FunctionBuilder<'_>, val: Value, return_ty: IrTy) -> Value {
    let val_ty = builder.func.dfg.value_type(val);
    match (val_ty, ir_ty_to_cranelift(return_ty)) {
        (types::I64, Some(types::I32)) => builder.ins().ireduce(types::I32, val),
        (types::I32, Some(types::I64)) => builder.ins().sextend(types::I64, val),
        _ => val,
    }
}

// ---------------------------------------------------------------------------
// Instruction lowering
// ---------------------------------------------------------------------------

struct LowerCtx<'a> {
    value_map: &'a mut HashMap<InstId, Value>,
    block_map: &'a HashMap<BlockId, Block>,
    phi_data: &'a PhiUpsilonData,
    arg_values: &'a HashMap<u32, Value>,
    hidden_region_values: &'a [Value],
    return_ty: IrTy,
    ir_func_id_to_ref: &'a HashMap<IrFuncId, FuncRef>,
    vow_violation_ref: Option<FuncRef>,
    overflow_ref: Option<FuncRef>,
    arena_alloc_ref: FuncRef,
    arena_open_ref: FuncRef,
    arena_close_ref: FuncRef,
    string_clone_ref: Option<FuncRef>,
    /// Stack slots holding the `VowArena` header for each block whose region
    /// is non-empty (spec §3.5). Lazily populated on first use of a given
    /// `BlockId` by `RegionOpen` / `RegionClose` / `RegionAlloc{Block}`.
    /// `BTreeMap` (not `HashMap`) for deterministic iteration order — the
    /// same rule the existing `slot_map` follows (see CLAUDE.md). Required
    /// for binary fixed-point reproducibility under the bootstrap triple.
    block_arena_slots: &'a mut BTreeMap<BlockId, StackSlot>,
    /// Per-callable region summaries, indexed by IR `FuncId`. Read by the
    /// `Opcode::Call` lowering to project hidden region parameters from the
    /// caller's frame (spec §5.2).
    func_summaries: &'a HashMap<IrFuncId, RegionSummary>,
    mode: BuildMode,
    trace: TraceMode,
    current_ir_block: BlockId,
    string_global_values: &'a HashMap<u32, GlobalValue>,
    extern_func_refs: &'a HashMap<String, FuncRef>,
    vow_desc_global_values: &'a HashMap<u32, GlobalValue>,
    vow_file_global_values: &'a HashMap<u32, GlobalValue>,
    vow_binding_name_gvs: &'a HashMap<(u32, u32), GlobalValue>,
    inst_ty_map: &'a HashMap<InstId, IrTy>,
    /// `InstId → &Inst` lookup used by return-materialisation analysis to
    /// stay O(n) per `Return` rather than O(n²). Built once per function
    /// in `compile_ir_function`.
    inst_index: &'a HashMap<InstId, &'a Inst>,
    ir_func: &'a IrFunction,
    trace_exit_ref: Option<FuncRef>,
    trace_vow_ref: Option<FuncRef>,
    fn_name_gv: Option<GlobalValue>,
    stack_exit_ref: Option<FuncRef>,
    root_arena_gv: GlobalValue,
}

/// Size of `vow_runtime::VowArena` in bytes — asserted in
/// `vow-runtime/src/lib.rs` (`assert!(size_of::<VowArena>() == 56)`).
const VOW_ARENA_HEADER_SIZE: u32 = 56;
/// Alignment for the `VowArena` header (contains pointers).
const VOW_ARENA_HEADER_ALIGN_LOG2: u8 = 3;

/// Look up or allocate the stack slot holding the `VowArena` header for
/// `block_id`. Slots are reused across `RegionOpen` / `RegionAlloc{Block}`
/// / `RegionClose` references to the same block within a single function.
fn block_arena_slot(
    builder: &mut FunctionBuilder,
    slots: &mut BTreeMap<BlockId, StackSlot>,
    block_id: BlockId,
) -> StackSlot {
    let (slot, created) = block_arena_slot_with_created(builder, slots, block_id);
    if created {
        let zero = builder.ins().iconst(types::I64, 0);
        for offset in (0..VOW_ARENA_HEADER_SIZE as i32).step_by(8) {
            builder.ins().stack_store(zero, slot, offset);
        }
    }
    slot
}

fn block_arena_slot_with_created(
    builder: &mut FunctionBuilder,
    slots: &mut BTreeMap<BlockId, StackSlot>,
    block_id: BlockId,
) -> (StackSlot, bool) {
    if let Some(&slot) = slots.get(&block_id) {
        return (slot, false);
    }
    let slot = builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        VOW_ARENA_HEADER_SIZE,
        VOW_ARENA_HEADER_ALIGN_LOG2,
    ));
    slots.insert(block_id, slot);
    (slot, true)
}

fn collect_block_arena_ids(func: &IrFunction) -> BTreeSet<BlockId> {
    let mut ids = BTreeSet::new();
    for block in &func.blocks {
        for inst in &block.insts {
            if let RegionId::Block(block_id) = inst.region {
                ids.insert(block_id);
            }
        }
    }
    ids
}

/// Materialise the Cranelift `Value` (`*VowArena`) representing `region` in
/// the current function's frame. Used by:
/// - `Opcode::RegionAlloc` lowering (§5.3) to pick the arena to allocate
///   into.
/// - `Opcode::Call` lowering (§5.2) to project hidden region parameters
///   for an internal callee.
fn region_to_arena_value(
    builder: &mut FunctionBuilder,
    region: RegionId,
    hidden_region_values: &[Value],
    block_arena_slots: &mut BTreeMap<BlockId, StackSlot>,
    root_arena_gv: GlobalValue,
) -> Result<Value, CodegenError> {
    match region {
        RegionId::Root => Ok(builder.ins().global_value(types::I64, root_arena_gv)),
        RegionId::Caller(idx) => {
            // Issue #317: AMBIGUOUS is the slot-aware-inference sentinel
            // for "this value's marker set resolves to multiple distinct
            // slots". The post-inference store-conflict check rejects
            // any store whose source has this region — that's the
            // soundness gate. Codegen still has to lower the alloc itself
            // (which may sit on an unrelated path that never reaches a
            // store), so fall back to slot 0 as a safe placeholder. If
            // the value is actually used at a store, the build has
            // already been rejected.
            let resolved_idx = if idx.is_ambiguous() {
                0
            } else {
                idx.0 as usize
            };
            let Some(&arena) = hidden_region_values.get(resolved_idx) else {
                return Err(CodegenError::UnsupportedOpcode(format!(
                    "missing hidden arena parameter {:?}",
                    idx
                )));
            };
            Ok(arena)
        }
        RegionId::Block(block_id) => {
            let slot = block_arena_slot(builder, block_arena_slots, block_id);
            Ok(builder.ins().stack_addr(types::I64, slot, 0))
        }
        RegionId::Rodata => Err(CodegenError::UnsupportedOpcode(
            "cannot derive arena pointer for Rodata region".to_string(),
        )),
    }
}

fn source_value_region(
    source: &Inst,
    inst_index: &HashMap<InstId, &Inst>,
    current_summary: &RegionSummary,
    phi_data: &PhiUpsilonData,
    seen: &mut BTreeSet<InstId>,
) -> RegionId {
    if !seen.insert(source.id) {
        return source.region;
    }
    if let (Opcode::GetArg, InstData::ArgIndex(param_idx)) = (&source.opcode, &source.data)
        && let Some(hidden_idx) = hidden_region_idx_for_store_target(current_summary, *param_idx)
    {
        return RegionId::Caller(hidden_idx);
    }
    if matches!(source.opcode, Opcode::FieldGet | Opcode::Load)
        && let Some(source_id) = source.args.first()
    {
        return arg_region_inner(*source_id, inst_index, current_summary, phi_data, seen);
    }
    if let (Opcode::Call, InstData::CallExtern(sym)) = (&source.opcode, &source.data)
        && matches!(sym.as_str(), "__vow_vec_get_val" | "__vow_vec_get")
        && let Some(source_id) = source.args.first()
    {
        return arg_region_inner(*source_id, inst_index, current_summary, phi_data, seen);
    }
    if source.opcode == Opcode::Phi {
        let mut merged: Option<RegionId> = None;
        for upsilons in phi_data.block_upsilons.values() {
            for &(phi_id, val_id) in upsilons {
                if phi_id != source.id {
                    continue;
                }
                let mut arm_seen = seen.clone();
                let arm_region =
                    arg_region_inner(val_id, inst_index, current_summary, phi_data, &mut arm_seen);
                match merged {
                    Some(existing) if existing != arm_region => return source.region,
                    Some(_) => {}
                    None => merged = Some(arm_region),
                }
            }
        }
        if let Some(region) = merged {
            return region;
        }
    }
    source.region
}

fn arg_region_inner(
    arg_id: InstId,
    inst_index: &HashMap<InstId, &Inst>,
    current_summary: &RegionSummary,
    phi_data: &PhiUpsilonData,
    seen: &mut BTreeSet<InstId>,
) -> RegionId {
    inst_index
        .get(&arg_id)
        .map(|src| source_value_region(src, inst_index, current_summary, phi_data, seen))
        .unwrap_or(RegionId::Root)
}

fn arg_region(
    arg_id: InstId,
    inst_index: &HashMap<InstId, &Inst>,
    current_summary: &RegionSummary,
    phi_data: &PhiUpsilonData,
) -> RegionId {
    let mut seen = BTreeSet::new();
    arg_region_inner(arg_id, inst_index, current_summary, phi_data, &mut seen)
}

fn first_arg_region(
    inst: &Inst,
    inst_index: &HashMap<InstId, &Inst>,
    current_summary: &RegionSummary,
    phi_data: &PhiUpsilonData,
) -> RegionId {
    inst.args
        .first()
        .map(|arg_id| arg_region(*arg_id, inst_index, current_summary, phi_data))
        .unwrap_or(RegionId::Root)
}

fn routed_vec_extern<'a>(
    sym: &'a str,
    inst: &Inst,
    inst_index: &HashMap<InstId, &Inst>,
    current_summary: &RegionSummary,
    phi_data: &PhiUpsilonData,
) -> (&'a str, Option<RegionId>) {
    match sym {
        "__vow_vec_new" => match inst.region {
            RegionId::Root => (sym, None),
            region => ("__vow_vec_new_in_arena", Some(region)),
        },
        "__vow_vec_new_val" => match inst.region {
            RegionId::Root => (sym, None),
            region => ("__vow_vec_new_val_in_arena", Some(region)),
        },
        "__vow_vec_push_val" => {
            let region = first_arg_region(inst, inst_index, current_summary, phi_data);
            match region {
                RegionId::Block(_) | RegionId::Caller(_) => {
                    ("__vow_vec_push_val_in_arena", Some(region))
                }
                _ => (sym, None),
            }
        }
        "__vow_vec_push" => {
            let region = first_arg_region(inst, inst_index, current_summary, phi_data);
            match region {
                RegionId::Block(_) | RegionId::Caller(_) => {
                    ("__vow_vec_push_in_arena", Some(region))
                }
                _ => (sym, None),
            }
        }
        "__vow_string_new" => match inst.region {
            RegionId::Root => (sym, None),
            region => ("__vow_string_new_in_arena", Some(region)),
        },
        "__vow_string_from_cstr" => match inst.region {
            RegionId::Root => (sym, None),
            region => ("__vow_string_from_cstr_in_arena", Some(region)),
        },
        "__vow_string_substr" => match inst.region {
            RegionId::Root => (sym, None),
            region => ("__vow_string_substr_in_arena", Some(region)),
        },
        "__vow_string_substring" => match inst.region {
            RegionId::Root => (sym, None),
            region => ("__vow_string_substring_in_arena", Some(region)),
        },
        "__vow_string_from_i64" => match inst.region {
            RegionId::Root => (sym, None),
            region => ("__vow_string_from_i64_in_arena", Some(region)),
        },
        "__vow_string_split" => match inst.region {
            RegionId::Root => (sym, None),
            region => ("__vow_string_split_in_arena", Some(region)),
        },
        "__vow_string_trim" => match inst.region {
            RegionId::Root => (sym, None),
            region => ("__vow_string_trim_in_arena", Some(region)),
        },
        "__vow_string_to_upper" => match inst.region {
            RegionId::Root => (sym, None),
            region => ("__vow_string_to_upper_in_arena", Some(region)),
        },
        "__vow_string_to_lower" => match inst.region {
            RegionId::Root => (sym, None),
            region => ("__vow_string_to_lower_in_arena", Some(region)),
        },
        "__vow_string_replace" => match inst.region {
            RegionId::Root => (sym, None),
            region => ("__vow_string_replace_in_arena", Some(region)),
        },
        "__vow_string_join" => match inst.region {
            RegionId::Root => (sym, None),
            region => ("__vow_string_join_in_arena", Some(region)),
        },
        "__vow_string_push_str" => {
            let region = first_arg_region(inst, inst_index, current_summary, phi_data);
            match region {
                RegionId::Block(_) | RegionId::Caller(_) => {
                    ("__vow_string_push_str_in_arena", Some(region))
                }
                _ => (sym, None),
            }
        }
        "__vow_string_push_byte" => {
            let region = first_arg_region(inst, inst_index, current_summary, phi_data);
            match region {
                RegionId::Block(_) | RegionId::Caller(_) => {
                    ("__vow_string_push_byte_in_arena", Some(region))
                }
                _ => (sym, None),
            }
        }
        "__vow_map_new" => match inst.region {
            RegionId::Root => (sym, None),
            region => ("__vow_map_new_in_arena", Some(region)),
        },
        "__vow_map_insert" => {
            let region = first_arg_region(inst, inst_index, current_summary, phi_data);
            match region {
                RegionId::Block(_) | RegionId::Caller(_) => {
                    ("__vow_map_insert_in_arena", Some(region))
                }
                _ => (sym, None),
            }
        }
        _ => {
            if extern_uses_target_region(sym) {
                (sym, Some(inst.region))
            } else {
                (sym, None)
            }
        }
    }
}

/// True when the function may emit a return-materialisation clone call —
/// i.e. it has `return_region == FreshInCaller` and at least one `Return`
/// inst whose source path reaches a `.rodata` literal or a heap-typed
/// parameter alias. Used at module-load time to decide whether to import
/// `__vow_string_clone_into_arena`.
fn module_uses_return_materialization(func: &IrFunction) -> bool {
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

/// Build an `InstId → &Inst` lookup for a function. Used by
/// `return_source_needs_materialization` (and its module-level scan) to
/// avoid an O(insts) `find` per visited value: the materialization walk
/// is called both at module-load time and per `Return` during lowering,
/// so the linear-scan version was O(n²) in functions with many
/// `Return`-reachable Phi arms.
fn build_inst_index(func: &IrFunction) -> HashMap<InstId, &Inst> {
    let mut index: HashMap<InstId, &Inst> = HashMap::new();
    for block in &func.blocks {
        for inst in &block.insts {
            index.insert(inst.id, inst);
        }
    }
    index
}

/// Decide whether a `FreshInCaller` function's return value needs to be
/// deep-copied into `target_region` to satisfy the §5.1 representation
/// promise.
///
/// Walks the value's source transitively through `Phi` (via `Upsilon`
/// arms). Returns true iff at least one reachable leaf is a
/// `__vow_string_from_cstr(literal)` call AND every reachable leaf is
/// the same shape (a known-safe `VowString` / `Vec<u8>` descriptor
/// producer).
///
/// **Why `__vow_string_from_cstr`, not `ConstStr` directly.**
/// `ConstStr` lowers to a pointer to a NUL-terminated byte blob in a
/// `.rodata` data section — it is *not* a `VowVec` descriptor.
/// `__vow_string_clone_into_arena` reads `(ptr, len, cap)` from its
/// source and copies `len` bytes; firing it on a raw `ConstStr` would
/// reinterpret arbitrary literal bytes as `{ptr, len, cap}` and corrupt
/// memory. The Vow lowerer wraps every `String` literal in
/// `Call __vow_string_from_cstr(ConstStr)` (`vow-ir/src/lower/mod.rs`
/// `Lit::String`), and that Call's *result* is the actual `VowVec`
/// descriptor — that's the shape we recognise here.
///
/// **Why the all-leaves rule still applies.** A Phi mixing a
/// `__vow_string_from_cstr` arm with any other leaf shape (a
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
fn return_source_needs_materialization(
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
            Opcode::Call
                if matches!(
                    &src.data,
                    InstData::CallExtern(sym) if sym == "__vow_string_from_cstr"
                ) =>
            {
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
                // `__vow_string_from_cstr` leaf, which only String-literal
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

fn lower_inst(
    builder: &mut FunctionBuilder,
    inst: &Inst,
    ctx: &mut LowerCtx,
) -> Result<(), CodegenError> {
    macro_rules! arg {
        ($i:expr) => {
            ctx.value_map[&inst.args[$i]]
        };
    }

    match inst.opcode {
        // ------------------------------------------------------------------
        // Constants
        // ------------------------------------------------------------------
        Opcode::ConstI32 => {
            if let InstData::ConstI32(v) = inst.data {
                let val = builder.ins().iconst(types::I32, v as i64);
                ctx.value_map.insert(inst.id, val);
            }
        }
        Opcode::ConstI64 => {
            if let InstData::ConstI64(v) = inst.data {
                let val = builder.ins().iconst(types::I64, v);
                ctx.value_map.insert(inst.id, val);
            }
        }
        Opcode::ConstU64 => {
            if let InstData::ConstU64(v) = inst.data {
                let val = builder.ins().iconst(types::I64, v as i64);
                ctx.value_map.insert(inst.id, val);
            }
        }
        Opcode::ConstF32 => {
            if let InstData::ConstF32(v) = inst.data {
                let val = builder.ins().f32const(v);
                ctx.value_map.insert(inst.id, val);
            }
        }
        Opcode::ConstF64 => {
            if let InstData::ConstF64(v) = inst.data {
                let val = builder.ins().f64const(v);
                ctx.value_map.insert(inst.id, val);
            }
        }
        Opcode::ConstBool => {
            let b = matches!(inst.data, InstData::ConstBool(true));
            let val = builder.ins().iconst(types::I64, b as i64);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::ConstStr => {
            if let InstData::ConstStr(idx) = inst.data {
                if let Some(&gv) = ctx.string_global_values.get(&idx) {
                    let ptr = builder.ins().global_value(types::I64, gv);
                    ctx.value_map.insert(inst.id, ptr);
                } else {
                    let null = builder.ins().iconst(types::I64, 0);
                    ctx.value_map.insert(inst.id, null);
                }
            }
        }
        Opcode::ConstUnit => {
            let val = builder.ins().iconst(types::I32, 0);
            ctx.value_map.insert(inst.id, val);
        }

        // ------------------------------------------------------------------
        // Arguments
        // ------------------------------------------------------------------
        Opcode::GetArg => {
            if let InstData::ArgIndex(idx) = inst.data {
                let val = if let Some(&v) = ctx.arg_values.get(&idx) {
                    v
                } else {
                    builder.ins().iconst(types::I32, 0) // Unit arg
                };
                ctx.value_map.insert(inst.id, val);
            }
        }

        // ------------------------------------------------------------------
        // Wrapping integer arithmetic
        // ------------------------------------------------------------------
        Opcode::WrappingAddI32 | Opcode::WrappingAddI64 | Opcode::WrappingAddU64 => {
            let val = builder.ins().iadd(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::WrappingSubI32 | Opcode::WrappingSubI64 | Opcode::WrappingSubU64 => {
            let val = builder.ins().isub(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::WrappingMulI32 | Opcode::WrappingMulI64 | Opcode::WrappingMulU64 => {
            let val = builder.ins().imul(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::WrappingDivI32 | Opcode::WrappingDivI64 => {
            let val = builder.ins().sdiv(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::WrappingDivU64 => {
            let val = builder.ins().udiv(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::WrappingRemI32 | Opcode::WrappingRemI64 => {
            let val = builder.ins().srem(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::WrappingRemU64 => {
            let val = builder.ins().urem(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }

        // ------------------------------------------------------------------
        // Checked integer arithmetic
        // ------------------------------------------------------------------
        Opcode::CheckedAddI32 | Opcode::CheckedAddI64 => {
            let (result, overflow) = builder.ins().sadd_overflow(arg!(0), arg!(1));
            emit_overflow_check(builder, overflow, ctx)?;
            ctx.value_map.insert(inst.id, result);
        }
        Opcode::CheckedAddU64 => {
            let (result, overflow) = builder.ins().uadd_overflow(arg!(0), arg!(1));
            emit_overflow_check(builder, overflow, ctx)?;
            ctx.value_map.insert(inst.id, result);
        }
        Opcode::CheckedSubI32 | Opcode::CheckedSubI64 => {
            let (result, overflow) = builder.ins().ssub_overflow(arg!(0), arg!(1));
            emit_overflow_check(builder, overflow, ctx)?;
            ctx.value_map.insert(inst.id, result);
        }
        Opcode::CheckedSubU64 => {
            let (result, overflow) = builder.ins().usub_overflow(arg!(0), arg!(1));
            emit_overflow_check(builder, overflow, ctx)?;
            ctx.value_map.insert(inst.id, result);
        }
        Opcode::CheckedMulI32 | Opcode::CheckedMulI64 => {
            let (result, overflow) = builder.ins().smul_overflow(arg!(0), arg!(1));
            emit_overflow_check(builder, overflow, ctx)?;
            ctx.value_map.insert(inst.id, result);
        }
        Opcode::CheckedMulU64 => {
            let (result, overflow) = builder.ins().umul_overflow(arg!(0), arg!(1));
            emit_overflow_check(builder, overflow, ctx)?;
            ctx.value_map.insert(inst.id, result);
        }
        Opcode::CheckedDivI32 | Opcode::CheckedDivI64 => {
            let cl_ty = ir_ty_to_cranelift(inst.ty).unwrap_or(types::I64);
            let zero = builder.ins().iconst(cl_ty, 0);
            let is_zero = builder.ins().icmp(IntCC::Equal, arg!(1), zero);
            emit_overflow_check(builder, is_zero, ctx)?;
            let val = builder.ins().sdiv(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::CheckedDivU64 => {
            let zero = builder.ins().iconst(types::I64, 0);
            let is_zero = builder.ins().icmp(IntCC::Equal, arg!(1), zero);
            emit_overflow_check(builder, is_zero, ctx)?;
            let val = builder.ins().udiv(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::CheckedRemI32 | Opcode::CheckedRemI64 => {
            let cl_ty = ir_ty_to_cranelift(inst.ty).unwrap_or(types::I64);
            let zero = builder.ins().iconst(cl_ty, 0);
            let is_zero = builder.ins().icmp(IntCC::Equal, arg!(1), zero);
            emit_overflow_check(builder, is_zero, ctx)?;
            let val = builder.ins().srem(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::CheckedRemU64 => {
            let zero = builder.ins().iconst(types::I64, 0);
            let is_zero = builder.ins().icmp(IntCC::Equal, arg!(1), zero);
            emit_overflow_check(builder, is_zero, ctx)?;
            let val = builder.ins().urem(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }

        // ------------------------------------------------------------------
        // Integer comparisons (return Bool)
        // ------------------------------------------------------------------
        Opcode::EqI32 | Opcode::EqI64 | Opcode::EqU64 => {
            let cmp = builder.ins().icmp(IntCC::Equal, arg!(0), arg!(1));
            let val = builder.ins().uextend(types::I64, cmp);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::NeI32 | Opcode::NeI64 | Opcode::NeU64 => {
            let cmp = builder.ins().icmp(IntCC::NotEqual, arg!(0), arg!(1));
            let val = builder.ins().uextend(types::I64, cmp);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::LtI32 | Opcode::LtI64 => {
            let cmp = builder.ins().icmp(IntCC::SignedLessThan, arg!(0), arg!(1));
            let val = builder.ins().uextend(types::I64, cmp);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::LtU64 => {
            let cmp = builder
                .ins()
                .icmp(IntCC::UnsignedLessThan, arg!(0), arg!(1));
            let val = builder.ins().uextend(types::I64, cmp);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::LeI32 | Opcode::LeI64 => {
            let cmp = builder
                .ins()
                .icmp(IntCC::SignedLessThanOrEqual, arg!(0), arg!(1));
            let val = builder.ins().uextend(types::I64, cmp);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::LeU64 => {
            let cmp = builder
                .ins()
                .icmp(IntCC::UnsignedLessThanOrEqual, arg!(0), arg!(1));
            let val = builder.ins().uextend(types::I64, cmp);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::GtI32 | Opcode::GtI64 => {
            let cmp = builder
                .ins()
                .icmp(IntCC::SignedGreaterThan, arg!(0), arg!(1));
            let val = builder.ins().uextend(types::I64, cmp);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::GtU64 => {
            let cmp = builder
                .ins()
                .icmp(IntCC::UnsignedGreaterThan, arg!(0), arg!(1));
            let val = builder.ins().uextend(types::I64, cmp);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::GeI32 | Opcode::GeI64 => {
            let cmp = builder
                .ins()
                .icmp(IntCC::SignedGreaterThanOrEqual, arg!(0), arg!(1));
            let val = builder.ins().uextend(types::I64, cmp);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::GeU64 => {
            let cmp = builder
                .ins()
                .icmp(IntCC::UnsignedGreaterThanOrEqual, arg!(0), arg!(1));
            let val = builder.ins().uextend(types::I64, cmp);
            ctx.value_map.insert(inst.id, val);
        }

        // ------------------------------------------------------------------
        // Float arithmetic
        // ------------------------------------------------------------------
        Opcode::AddF32 | Opcode::AddF64 => {
            let val = builder.ins().fadd(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::SubF32 | Opcode::SubF64 => {
            let val = builder.ins().fsub(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::MulF32 | Opcode::MulF64 => {
            let val = builder.ins().fmul(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::DivF32 | Opcode::DivF64 => {
            let val = builder.ins().fdiv(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::RemF32 | Opcode::RemF64 => {
            return Err(CodegenError::UnsupportedOpcode(
                "float remainder not yet supported".to_string(),
            ));
        }

        // ------------------------------------------------------------------
        // Float comparisons
        // ------------------------------------------------------------------
        Opcode::EqF32 | Opcode::EqF64 => {
            let cmp = builder.ins().fcmp(FloatCC::Equal, arg!(0), arg!(1));
            let val = builder.ins().uextend(types::I64, cmp);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::NeF32 | Opcode::NeF64 => {
            let cmp = builder.ins().fcmp(FloatCC::NotEqual, arg!(0), arg!(1));
            let val = builder.ins().uextend(types::I64, cmp);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::LtF32 | Opcode::LtF64 => {
            let cmp = builder.ins().fcmp(FloatCC::LessThan, arg!(0), arg!(1));
            let val = builder.ins().uextend(types::I64, cmp);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::LeF32 | Opcode::LeF64 => {
            let cmp = builder
                .ins()
                .fcmp(FloatCC::LessThanOrEqual, arg!(0), arg!(1));
            let val = builder.ins().uextend(types::I64, cmp);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::GtF32 | Opcode::GtF64 => {
            let cmp = builder.ins().fcmp(FloatCC::GreaterThan, arg!(0), arg!(1));
            let val = builder.ins().uextend(types::I64, cmp);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::GeF32 | Opcode::GeF64 => {
            let cmp = builder
                .ins()
                .fcmp(FloatCC::GreaterThanOrEqual, arg!(0), arg!(1));
            let val = builder.ins().uextend(types::I64, cmp);
            ctx.value_map.insert(inst.id, val);
        }

        // ------------------------------------------------------------------
        // Boolean operations
        // ------------------------------------------------------------------
        Opcode::Not => {
            let one = builder.ins().iconst(types::I64, 1);
            let val = builder.ins().bxor(arg!(0), one);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::And => {
            let val = builder.ins().band(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::Or => {
            let val = builder.ins().bor(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }

        // ------------------------------------------------------------------
        // Integer bitwise operations
        // ------------------------------------------------------------------
        Opcode::BitAndI64 | Opcode::BitAndU64 => {
            let val = builder.ins().band(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::BitOrI64 | Opcode::BitOrU64 => {
            let val = builder.ins().bor(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        // ------------------------------------------------------------------
        // Bitwise XOR
        // ------------------------------------------------------------------
        Opcode::XorI32 | Opcode::XorI64 | Opcode::XorU64 => {
            let val = builder.ins().bxor(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::ShlI64 | Opcode::ShlU64 => {
            let val = builder.ins().ishl(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::ShrI64 => {
            let val = builder.ins().sshr(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::ShrU64 => {
            let val = builder.ins().ushr(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }

        // ------------------------------------------------------------------
        // Memory
        // ------------------------------------------------------------------
        Opcode::Load => {
            let cl_ty = ir_ty_to_cranelift(inst.ty).unwrap_or(types::I64);
            let val = builder.ins().load(cl_ty, MemFlags::new(), arg!(0), 0);
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::Store => {
            builder.ins().store(MemFlags::new(), arg!(1), arg!(0), 0);
            let unit = builder.ins().iconst(types::I32, 0);
            ctx.value_map.insert(inst.id, unit);
        }

        // ------------------------------------------------------------------
        // Control flow
        // ------------------------------------------------------------------
        Opcode::Branch => {
            let cond = arg!(0);
            let (then_block_id, else_block_id) = match inst.data {
                InstData::BranchTargets {
                    then_block,
                    else_block,
                } => (then_block, else_block),
                _ => unreachable!("Branch must have BranchTargets data"),
            };
            let then_cl = ctx.block_map[&then_block_id];
            let else_cl = ctx.block_map[&else_block_id];
            let then_args = collect_target_block_args(
                ctx.current_ir_block,
                then_block_id,
                ctx.phi_data,
                ctx.value_map,
            );
            let else_args = collect_target_block_args(
                ctx.current_ir_block,
                else_block_id,
                ctx.phi_data,
                ctx.value_map,
            );
            builder
                .ins()
                .brif(cond, then_cl, &then_args, else_cl, &else_args);
        }
        Opcode::Jump => {
            let target_id = match inst.data {
                InstData::JumpTarget(b) => b,
                _ => unreachable!("Jump must have JumpTarget data"),
            };
            let target_cl = ctx.block_map[&target_id];
            let args = collect_target_block_args(
                ctx.current_ir_block,
                target_id,
                ctx.phi_data,
                ctx.value_map,
            );
            builder.ins().jump(target_cl, &args);
        }
        Opcode::Return => {
            if ctx.trace != TraceMode::Off
                && let (Some(exit_ref), Some(gv)) = (ctx.trace_exit_ref, ctx.fn_name_gv)
            {
                let name_ptr = builder.ins().global_value(types::I64, gv);
                builder.ins().call(exit_ref, &[name_ptr]);
            }
            if let Some(se_ref) = ctx.stack_exit_ref {
                builder.ins().call(se_ref, &[]);
            }
            if ctx.return_ty == IrTy::Unit {
                if ctx.ir_func.name == "main" {
                    let zero = builder.ins().iconst(types::I32, 0);
                    builder.ins().return_(&[zero]);
                } else {
                    builder.ins().return_(&[]);
                }
            } else if let Some(&val_id) = inst.args.first() {
                if let Some(&val) = ctx.value_map.get(&val_id) {
                    // Phase 4 / S5 return materialization (spec §5.1).
                    // For a `FreshInCaller` function, the returned heap-typed
                    // value MUST be in `target_region` at the return edge. If
                    // the value's source path includes a `.rodata` literal or
                    // a parameter alias, deep-copy it into `target_region`
                    // (slot 0 of `hidden_region_values`).
                    let needs_clone = ctx.return_ty == IrTy::Ptr
                        && ctx.ir_func.summary.return_region == RegionConstraint::FreshInCaller
                        && return_source_needs_materialization(ctx.ir_func, ctx.inst_index, val_id);
                    let materialized = if needs_clone {
                        let target_region =
                            ctx.hidden_region_values.first().copied().ok_or_else(|| {
                                CodegenError::UnsupportedOpcode(
                                    "FreshInCaller function missing target_region hidden param"
                                        .to_string(),
                                )
                            })?;
                        let clone_ref = ctx.string_clone_ref.ok_or_else(|| {
                            CodegenError::UnsupportedOpcode(
                                "return materialisation needed but \
                                 __vow_string_clone_into_arena import missing — \
                                 module-level scan in compile_module is out of sync"
                                    .to_string(),
                            )
                        })?;
                        let call_inst = builder.ins().call(clone_ref, &[target_region, val]);
                        builder.inst_results(call_inst)[0]
                    } else {
                        val
                    };
                    let materialized = coerce_return_value(builder, materialized, ctx.return_ty);
                    builder.ins().return_(&[materialized]);
                } else {
                    builder.ins().return_(&[]);
                }
            } else {
                builder.ins().return_(&[]);
            }
        }
        Opcode::Unreachable => {
            builder.ins().trap(TrapCode::unwrap_user(2));
        }

        // ------------------------------------------------------------------
        // Phi / Upsilon — handled via block params; skip instruction emit
        // ------------------------------------------------------------------
        Opcode::Phi => {
            // Value already in value_map from block param setup
        }
        Opcode::Upsilon => {
            // Handled at Branch/Jump sites
        }

        // ------------------------------------------------------------------
        // Vow checks
        // ------------------------------------------------------------------
        Opcode::VowRequires | Opcode::VowEnsures | Opcode::VowInvariant => {
            if ctx.mode.has_debug_checks()
                && let Some(&pred_id) = inst.args.first()
                && let Some(&pred) = ctx.value_map.get(&pred_id)
            {
                let vow_id = match inst.data {
                    InstData::VowId(v) => v.0,
                    _ => 0,
                };
                let blame_byte = if inst.opcode == Opcode::VowRequires {
                    0u8 // Caller
                } else {
                    1u8 // Callee
                };
                let vow_offset = ctx
                    .ir_func
                    .vows
                    .get(vow_id as usize)
                    .map(|v| v.offset)
                    .unwrap_or(0);
                let captures: Vec<(GlobalValue, Value, IrTy)> = if let Some(vow_entry) =
                    ctx.ir_func.vows.get(vow_id as usize)
                {
                    vow_entry
                        .bindings
                        .iter()
                        .enumerate()
                        .filter_map(|(idx, (_, inst_id))| {
                            let ir_ty = *ctx.inst_ty_map.get(inst_id)?;
                            if matches!(ir_ty, IrTy::Ptr | IrTy::LinearPtr | IrTy::Unit) {
                                return None;
                            }
                            let cl_val = *ctx.value_map.get(inst_id)?;
                            let name_gv = *ctx.vow_binding_name_gvs.get(&(vow_id, idx as u32))?;
                            Some((name_gv, cl_val, ir_ty))
                        })
                        .collect()
                } else {
                    vec![]
                };
                emit_vow_check(
                    builder, pred, vow_id, blame_byte, &captures, vow_offset, ctx,
                )?;
            }
            // In Release mode: no-op
        }

        // ------------------------------------------------------------------
        // Debug calls (only emitted in debug/sanitize mode)
        // ------------------------------------------------------------------
        Opcode::DebugCall => {
            if ctx.mode.has_debug_checks() {
                let InstData::CallExtern(ref sym) = inst.data else {
                    return Err(CodegenError::UnsupportedOpcode(
                        "DebugCall without CallExtern data".to_string(),
                    ));
                };
                let Some(&func_ref) = ctx.extern_func_refs.get(sym.as_str()) else {
                    return Err(CodegenError::UnsupportedOpcode(format!(
                        "unknown debug extern symbol: {sym}"
                    )));
                };
                let sig_ref = builder.func.dfg.ext_funcs[func_ref].signature;
                let expected_types: Vec<types::Type> = builder.func.dfg.signatures[sig_ref]
                    .params
                    .iter()
                    .map(|p| p.value_type)
                    .collect();
                let call_args: Vec<Value> = inst
                    .args
                    .iter()
                    .enumerate()
                    .map(|(i, id)| {
                        let v = *ctx.value_map.get(id).unwrap_or_else(|| {
                            panic!(
                                "cranelift backend: DebugCall value_map miss for arg {id:?} in inst {:?}",
                                inst.id
                            )
                        });
                        if let Some(&expected_ty) = expected_types.get(i) {
                            let actual_ty = builder.func.dfg.value_type(v);
                            if actual_ty == types::I32 && expected_ty == types::I64 {
                                return builder.ins().sextend(types::I64, v);
                            }
                            if actual_ty == types::I8 && expected_ty == types::I64 {
                                return builder.ins().uextend(types::I64, v);
                            }
                        }
                        v
                    })
                    .collect();
                builder.ins().call(func_ref, &call_args);
            }
            let unit = builder.ins().iconst(types::I32, 0);
            ctx.value_map.insert(inst.id, unit);
        }

        // ------------------------------------------------------------------
        // Function calls
        // ------------------------------------------------------------------
        Opcode::Call => {
            let mut internal_call = false;
            let mut external_target_region = None;
            let func_ref = match &inst.data {
                InstData::CallTarget(f) => {
                    internal_call = true;
                    let Some(&fr) = ctx.ir_func_id_to_ref.get(f) else {
                        return Err(CodegenError::UnsupportedOpcode(format!(
                            "unknown call target FuncId({:?})",
                            f
                        )));
                    };
                    fr
                }
                InstData::CallExtern(sym) => {
                    let (routed_sym, target_region) = routed_vec_extern(
                        sym,
                        inst,
                        ctx.inst_index,
                        &ctx.ir_func.summary,
                        ctx.phi_data,
                    );
                    external_target_region = target_region;
                    let Some(&fr) = ctx.extern_func_refs.get(routed_sym) else {
                        return Err(CodegenError::UnsupportedOpcode(format!(
                            "unknown extern symbol: {routed_sym}"
                        )));
                    };
                    fr
                }
                _ => {
                    return Err(CodegenError::UnsupportedOpcode(
                        "Call without CallTarget or CallExtern data".to_string(),
                    ));
                }
            };
            let sig_ref = builder.func.dfg.ext_funcs[func_ref].signature;
            let expected_types: Vec<types::Type> = builder.func.dfg.signatures[sig_ref]
                .params
                .iter()
                .map(|p| p.value_type)
                .collect();
            let mut call_args: Vec<Value> = Vec::new();
            if let Some(region) = external_target_region {
                let arena = region_to_arena_value(
                    builder,
                    region,
                    ctx.hidden_region_values,
                    ctx.block_arena_slots,
                    ctx.root_arena_gv,
                )?;
                call_args.push(arena);
            }
            let hidden_arg_offset = call_args.len();
            for (i, id) in inst.args.iter().enumerate() {
                let v = *ctx.value_map.get(id).unwrap_or_else(|| {
                    panic!(
                        "cranelift backend: Call value_map miss for arg {id:?} in inst {:?}",
                        inst.id
                    )
                });
                let v = if let Some(&expected_ty) = expected_types.get(i + hidden_arg_offset) {
                    let actual_ty = builder.func.dfg.value_type(v);
                    if actual_ty == types::I32 && expected_ty == types::I64 {
                        builder.ins().sextend(types::I64, v)
                    } else if actual_ty == types::I8 && expected_ty == types::I64 {
                        builder.ins().uextend(types::I64, v)
                    } else {
                        v
                    }
                } else {
                    v
                };
                call_args.push(v);
            }
            if internal_call {
                // Project the callee's hidden region parameters from the
                // caller's frame (spec §5.2). The order MUST match
                // `build_signature` / `hidden_region_param_count`:
                //   1. `target_region` first iff callee has
                //      `return_region == FreshInCaller`. Sourced from the
                //      Call's `inst.region` — the caller's view of where
                //      the result must live.
                //   2. One `*VowArena` per distinct `StoreEffect.target` in
                //      ascending callee-param-index order. Sourced from the
                //      region of the matching Vow argument.
                //
                // The projection MUST NOT overshoot the callee's declared
                // signature. `hidden_region_param_count` special-cases
                // `main` to 0 hidden params (C ABI), so the summary's
                // `FreshInCaller` / `store_effects` may say "two hidden
                // args" while the signature has none. Bound the loop by
                // `expected_types.len()` (the callee's actual signature
                // arity) so a call to `main` with a non-empty summary
                // doesn't push extra args and break Cranelift verification.
                let callee_id = match &inst.data {
                    InstData::CallTarget(f) => *f,
                    _ => unreachable!("internal_call is true only for CallTarget"),
                };
                let callee_summary = ctx.func_summaries.get(&callee_id).ok_or_else(|| {
                    CodegenError::UnsupportedOpcode(format!(
                        "missing region summary for callee FuncId({:?})",
                        callee_id
                    ))
                })?;

                let mut push_hidden = |builder: &mut FunctionBuilder<'_>,
                                       call_args: &mut Vec<Value>,
                                       region: RegionId|
                 -> Result<(), CodegenError> {
                    if call_args.len() >= expected_types.len() {
                        return Ok(());
                    }
                    let arena = region_to_arena_value(
                        builder,
                        region,
                        ctx.hidden_region_values,
                        ctx.block_arena_slots,
                        ctx.root_arena_gv,
                    )?;
                    call_args.push(arena);
                    Ok(())
                };

                if callee_summary.return_region == RegionConstraint::FreshInCaller {
                    push_hidden(builder, &mut call_args, inst.region)?;
                }

                let mut store_targets: Vec<u32> = callee_summary
                    .store_effects
                    .iter()
                    .map(|e| e.target)
                    .collect();
                store_targets.sort_unstable();
                store_targets.dedup();
                for target_idx in store_targets {
                    // The arena to thread is the region of the Vow
                    // argument at `target_idx` in this call. For
                    // `RegionAlloc`-derived arguments the region pass set
                    // `inst.region` precisely; for other heap-typed
                    // arguments (`GetArg`, `Phi`, `FieldGet`, ...) the
                    // region currently defaults to `Root`. That fallback
                    // matches today's external behavior and is the same
                    // gap S5 / future work tightens via per-value region
                    // tracking.
                    let arg_region = inst
                        .args
                        .get(target_idx as usize)
                        .map(|arg_id| {
                            arg_region(*arg_id, ctx.inst_index, &ctx.ir_func.summary, ctx.phi_data)
                        })
                        .unwrap_or(RegionId::Root);
                    push_hidden(builder, &mut call_args, arg_region)?;
                }
            }
            let call_inst = builder.ins().call(func_ref, &call_args);
            let results = builder.inst_results(call_inst);
            if results.is_empty() {
                let unit = builder.ins().iconst(types::I32, 0);
                ctx.value_map.insert(inst.id, unit);
            } else {
                let r = results[0];
                let rt = builder.func.dfg.value_type(r);
                let norm = if rt == types::I8 {
                    builder.ins().uextend(types::I64, r)
                } else {
                    r
                };
                ctx.value_map.insert(inst.id, norm);
            }
        }

        // ------------------------------------------------------------------
        // Region / linear
        // ------------------------------------------------------------------
        Opcode::RegionAlloc => {
            let (size, align) = if let InstData::AllocSize { size, align } = inst.data {
                (size as i64, align as i64)
            } else {
                (0, 8)
            };
            let arena = region_to_arena_value(
                builder,
                inst.region,
                ctx.hidden_region_values,
                ctx.block_arena_slots,
                ctx.root_arena_gv,
            )?;
            let size_val = builder.ins().iconst(types::I64, size);
            let align_val = builder.ins().iconst(types::I64, align);
            let call_inst = builder
                .ins()
                .call(ctx.arena_alloc_ref, &[arena, size_val, align_val]);
            let ptr = builder.inst_results(call_inst)[0];
            ctx.value_map.insert(inst.id, ptr);
        }
        Opcode::LinearConsume | Opcode::LinearBorrow => {
            let unit = builder.ins().iconst(types::I32, 0);
            ctx.value_map.insert(inst.id, unit);
        }

        Opcode::RegionOpen | Opcode::RegionClose => {
            let RegionId::Block(block_id) = inst.region else {
                return Err(CodegenError::UnsupportedOpcode(format!(
                    "{:?} requires region = Block(_), got {:?}",
                    inst.opcode, inst.region
                )));
            };
            let slot = block_arena_slot(builder, ctx.block_arena_slots, block_id);
            let arena_addr = builder.ins().stack_addr(types::I64, slot, 0);

            if inst.opcode == Opcode::RegionOpen {
                builder.ins().call(ctx.arena_open_ref, &[arena_addr]);
            } else {
                builder.ins().call(ctx.arena_close_ref, &[arena_addr]);
            }
            let unit = builder.ins().iconst(types::I32, 0);
            ctx.value_map.insert(inst.id, unit);
        }

        // ------------------------------------------------------------------
        // Struct / enum field access
        // ------------------------------------------------------------------
        Opcode::FieldGet => {
            if let InstData::FieldIndex(idx) = inst.data {
                let base = ctx.value_map[&inst.args[0]];
                let offset = (idx as i32) * 8;
                let raw = builder
                    .ins()
                    .load(types::I64, MemFlags::trusted(), base, offset);
                let result = match ir_ty_to_cranelift(inst.ty) {
                    Some(types::I64) | None => raw,
                    Some(types::I32) => builder.ins().ireduce(types::I32, raw),
                    Some(types::I8) => builder.ins().ireduce(types::I8, raw),
                    Some(types::F64) => builder.ins().bitcast(types::F64, MemFlags::new(), raw),
                    Some(types::F32) => {
                        let i32v = builder.ins().ireduce(types::I32, raw);
                        builder.ins().bitcast(types::F32, MemFlags::new(), i32v)
                    }
                    Some(other) => builder.ins().ireduce(other, raw),
                };
                ctx.value_map.insert(inst.id, result);
            }
        }
        Opcode::FieldSet => {
            if let InstData::FieldIndex(idx) = inst.data {
                let base = ctx.value_map[&inst.args[0]];
                let new_val = ctx.value_map[&inst.args[1]];
                let offset = (idx as i32) * 8;
                let src_ty = builder.func.dfg.value_type(new_val);
                let store_val = match src_ty {
                    types::I32 => builder.ins().sextend(types::I64, new_val),
                    types::I8 => builder.ins().uextend(types::I64, new_val),
                    types::F32 => {
                        let bits = builder.ins().bitcast(types::I32, MemFlags::new(), new_val);
                        builder.ins().uextend(types::I64, bits)
                    }
                    types::F64 => builder.ins().bitcast(types::I64, MemFlags::new(), new_val),
                    _ => new_val,
                };
                builder
                    .ins()
                    .store(MemFlags::trusted(), store_val, base, offset);
                let unit = builder.ins().iconst(types::I32, 0);
                ctx.value_map.insert(inst.id, unit);
            }
        }

        // ------------------------------------------------------------------
        // Casts (i64 <-> u64 — no-op at machine level, both are I64)
        // ------------------------------------------------------------------
        Opcode::CastI64ToU64 | Opcode::CastU64ToI64 => {
            let val = arg!(0);
            ctx.value_map.insert(inst.id, val);
        }
    }
    Ok(())
}

fn emit_overflow_check(
    builder: &mut FunctionBuilder,
    overflow: Value,
    ctx: &mut LowerCtx,
) -> Result<(), CodegenError> {
    let trap_block = builder.create_block();
    let cont_block = builder.create_block();
    builder
        .ins()
        .brif(overflow, trap_block, &[], cont_block, &[]);

    builder.switch_to_block(trap_block);
    builder.seal_block(trap_block);
    if let Some(overflow_ref) = ctx.overflow_ref {
        builder.ins().call(overflow_ref, &[]);
    }
    builder.ins().trap(TrapCode::INTEGER_OVERFLOW);

    builder.switch_to_block(cont_block);
    builder.seal_block(cont_block);
    Ok(())
}

fn tag_for_ir_ty(ty: IrTy) -> u8 {
    match ty {
        IrTy::I32 => 0,
        IrTy::I64 => 1,
        IrTy::F32 => 2,
        IrTy::F64 => 3,
        IrTy::Bool => 4,
        _ => 0,
    }
}

fn emit_vow_check(
    builder: &mut FunctionBuilder,
    predicate: Value,
    vow_id: u32,
    blame: u8,
    captures: &[(GlobalValue, Value, IrTy)],
    vow_offset: u32,
    ctx: &mut LowerCtx,
) -> Result<(), CodegenError> {
    let one = builder.ins().iconst(types::I64, 1);
    let inv = builder.ins().bxor(predicate, one);

    let violation_block = builder.create_block();
    let cont_block = builder.create_block();
    builder
        .ins()
        .brif(inv, violation_block, &[], cont_block, &[]);

    // Trace vow check result in Full mode (pass branch)
    if ctx.trace == TraceMode::Full
        && let (Some(vow_ref), Some(gv)) = (ctx.trace_vow_ref, ctx.fn_name_gv)
    {
        // Emit on cont_block (pass)
        builder.switch_to_block(cont_block);
        builder.seal_block(cont_block);
        let name_ptr = builder.ins().global_value(types::I64, gv);
        let vid = builder.ins().iconst(types::I64, vow_id as i64);
        let passed = builder.ins().iconst(types::I64, 1);
        builder.ins().call(vow_ref, &[name_ptr, vid, passed]);
        let cont2 = builder.create_block();
        builder.ins().jump(cont2, &[]);

        // Emit on violation_block (fail) before violation call
        builder.switch_to_block(violation_block);
        builder.seal_block(violation_block);
        let name_ptr2 = builder.ins().global_value(types::I64, gv);
        let vid2 = builder.ins().iconst(types::I64, vow_id as i64);
        let failed = builder.ins().iconst(types::I64, 0);
        builder.ins().call(vow_ref, &[name_ptr2, vid2, failed]);
        emit_vow_violation_body(builder, vow_id, blame, captures, vow_offset, ctx)?;
        builder.ins().trap(TrapCode::unwrap_user(1));

        builder.switch_to_block(cont2);
        builder.seal_block(cont2);
        return Ok(());
    }

    builder.switch_to_block(violation_block);
    builder.seal_block(violation_block);
    emit_vow_violation_body(builder, vow_id, blame, captures, vow_offset, ctx)?;
    builder.ins().trap(TrapCode::unwrap_user(1));

    builder.switch_to_block(cont_block);
    builder.seal_block(cont_block);
    Ok(())
}

fn emit_vow_violation_body(
    builder: &mut FunctionBuilder,
    vow_id: u32,
    blame: u8,
    captures: &[(GlobalValue, Value, IrTy)],
    vow_offset: u32,
    ctx: &mut LowerCtx,
) -> Result<(), CodegenError> {
    if let Some(vr) = ctx.vow_violation_ref {
        let vow_id_val = builder.ins().iconst(types::I32, vow_id as i64);
        let blame_val = builder.ins().iconst(types::I8, blame as i64);
        let desc_ptr = if let Some(&gv) = ctx.vow_desc_global_values.get(&vow_id) {
            builder.ins().global_value(types::I64, gv)
        } else {
            builder.ins().iconst(types::I64, 0)
        };

        let n = captures.len();
        let (bindings_ptr, count_val) = if n > 0 {
            let slot: StackSlot = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                (24 * n) as u32,
                3,
            ));
            for (i, (name_gv, cl_val, ir_ty)) in captures.iter().enumerate() {
                let name_ptr = builder.ins().global_value(types::I64, *name_gv);
                builder.ins().stack_store(name_ptr, slot, (i * 24) as i32);
                let tag_val = builder
                    .ins()
                    .iconst(types::I8, tag_for_ir_ty(*ir_ty) as i64);
                builder
                    .ins()
                    .stack_store(tag_val, slot, (i * 24 + 8) as i32);
                let payload: Value = match ir_ty {
                    IrTy::I32 => builder.ins().sextend(types::I64, *cl_val),
                    IrTy::I64 => *cl_val,
                    IrTy::F32 => {
                        let bits = builder.ins().bitcast(types::I32, MemFlags::new(), *cl_val);
                        builder.ins().uextend(types::I64, bits)
                    }
                    IrTy::F64 => builder.ins().bitcast(types::I64, MemFlags::new(), *cl_val),
                    IrTy::Bool => *cl_val,
                    _ => builder.ins().iconst(types::I64, 0),
                };
                builder
                    .ins()
                    .stack_store(payload, slot, (i * 24 + 16) as i32);
            }
            let base = builder.ins().stack_addr(types::I64, slot, 0);
            let cnt = builder.ins().iconst(types::I32, n as i64);
            (base, cnt)
        } else {
            let null = builder.ins().iconst(types::I64, 0);
            let zero = builder.ins().iconst(types::I32, 0);
            (null, zero)
        };

        let file_ptr = if let Some(&gv) = ctx.vow_file_global_values.get(&vow_id) {
            builder.ins().global_value(types::I64, gv)
        } else {
            builder.ins().iconst(types::I64, 0)
        };
        let offset_val = builder.ins().iconst(types::I32, vow_offset as i64);

        builder.ins().call(
            vr,
            &[
                vow_id_val,
                blame_val,
                desc_ptr,
                bindings_ptr,
                count_val,
                file_ptr,
                offset_val,
            ],
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Function compilation
// ---------------------------------------------------------------------------

struct RuntimeIds {
    vow_violation_id: Option<CraneliftFuncId>,
    overflow_id: Option<CraneliftFuncId>,
    arena_alloc_id: CraneliftFuncId,
    arena_init_id: CraneliftFuncId,
    arena_open_id: CraneliftFuncId,
    arena_close_id: CraneliftFuncId,
    string_clone_id: Option<CraneliftFuncId>,
    runtime_start_id: CraneliftFuncId,
    root_arena_id: DataId,
    trace_enter_id: Option<CraneliftFuncId>,
    trace_exit_id: Option<CraneliftFuncId>,
    trace_vow_id: Option<CraneliftFuncId>,
    profile_enter_id: Option<CraneliftFuncId>,
    profile_init_id: Option<CraneliftFuncId>,
    sanitize_init_id: Option<CraneliftFuncId>,
    stack_guard_init_id: CraneliftFuncId,
    stack_enter_id: Option<CraneliftFuncId>,
    stack_exit_id: Option<CraneliftFuncId>,
}

#[allow(clippy::too_many_arguments)]
fn compile_ir_function(
    ctx: &mut Context,
    ir_func: &IrFunction,
    builder_ctx: &mut FunctionBuilderContext,
    mode: BuildMode,
    trace: TraceMode,
    obj_module: &mut ObjectModule,
    ir_to_cl: &[(IrFuncId, CraneliftFuncId)],
    runtime: &RuntimeIds,
    string_data_ids: &[DataId],
    extern_func_ids: &HashMap<String, CraneliftFuncId>,
    func_summaries: &HashMap<IrFuncId, RegionSummary>,
) -> Result<(), CodegenError> {
    let vow_violation_id = runtime.vow_violation_id;
    let overflow_id = runtime.overflow_id;
    let phi_data = build_phi_upsilon_data(ir_func);

    let mut builder = FunctionBuilder::new(&mut ctx.func, builder_ctx);

    // Create all IR blocks in Cranelift
    let mut block_map: HashMap<BlockId, Block> = HashMap::new();
    for ir_block in &ir_func.blocks {
        let cl_block = builder.create_block();
        block_map.insert(ir_block.id, cl_block);
    }

    // Add block params for Phi nodes
    for ir_block in &ir_func.blocks {
        if let Some(phi_ids) = phi_data.block_phis.get(&ir_block.id) {
            let cl_block = block_map[&ir_block.id];
            for &phi_id in phi_ids {
                let phi_inst = ir_block.insts.iter().find(|i| i.id == phi_id).unwrap();
                if let Some(cl_ty) = ir_ty_to_cranelift(phi_inst.ty) {
                    builder.append_block_param(cl_block, cl_ty);
                }
            }
        }
    }

    // Set up entry block with function parameters
    if let Some(first) = ir_func.blocks.first() {
        let entry_cl = block_map[&first.id];
        builder.append_block_params_for_function_params(entry_cl);
    }

    // Pre-declare external func refs (before value_map / instruction loop)
    let vow_violation_ref =
        vow_violation_id.map(|id| obj_module.declare_func_in_func(id, builder.func));
    let overflow_ref = overflow_id.map(|id| obj_module.declare_func_in_func(id, builder.func));
    let arena_alloc_ref = obj_module.declare_func_in_func(runtime.arena_alloc_id, builder.func);
    let arena_init_ref = obj_module.declare_func_in_func(runtime.arena_init_id, builder.func);
    let arena_open_ref = obj_module.declare_func_in_func(runtime.arena_open_id, builder.func);
    let arena_close_ref = obj_module.declare_func_in_func(runtime.arena_close_id, builder.func);
    let string_clone_ref = runtime
        .string_clone_id
        .map(|id| obj_module.declare_func_in_func(id, builder.func));
    let root_arena_gv = obj_module.declare_data_in_func(runtime.root_arena_id, builder.func);

    let mut ir_func_id_to_ref: HashMap<IrFuncId, FuncRef> = HashMap::new();
    for &(ir_id, cl_id) in ir_to_cl {
        let fref = obj_module.declare_func_in_func(cl_id, builder.func);
        ir_func_id_to_ref.insert(ir_id, fref);
    }

    // Declare string data globals in this function
    let mut string_global_values: HashMap<u32, GlobalValue> = HashMap::new();
    for (idx, &data_id) in string_data_ids.iter().enumerate() {
        let gv = obj_module.declare_data_in_func(data_id, builder.func);
        string_global_values.insert(idx as u32, gv);
    }

    // Build InstId → IrTy map for all instructions
    let mut inst_ty_map: HashMap<InstId, IrTy> = HashMap::new();
    for ir_block in &ir_func.blocks {
        for inst in &ir_block.insts {
            inst_ty_map.insert(inst.id, inst.ty);
        }
    }

    // Build InstId → &Inst map for return-materialisation analysis.
    // Reused per `Return` to keep the walk O(n) (see
    // `return_source_needs_materialization`).
    let inst_index: HashMap<InstId, &Inst> = build_inst_index(ir_func);

    // Create data sections for vow description strings and map VowId → GlobalValue
    let mut vow_desc_global_values: HashMap<u32, GlobalValue> = HashMap::new();
    let mut vow_file_global_values: HashMap<u32, GlobalValue> = HashMap::new();
    let mut vow_binding_name_gvs: HashMap<(u32, u32), GlobalValue> = HashMap::new();
    if mode.has_debug_checks() {
        for vow_entry in &ir_func.vows {
            let mut bytes = vow_entry.description.as_bytes().to_vec();
            bytes.push(0);
            let mut desc = DataDescription::new();
            desc.define(bytes.into_boxed_slice());
            let data_id = obj_module
                .declare_anonymous_data(false, false)
                .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
            obj_module
                .define_data(data_id, &desc)
                .map_err(|e| CodegenError::FunctionDefine(e.to_string()))?;
            let gv = obj_module.declare_data_in_func(data_id, builder.func);
            vow_desc_global_values.insert(vow_entry.id.0, gv);

            let mut file_bytes = vow_entry.file.as_bytes().to_vec();
            file_bytes.push(0);
            let mut file_desc = DataDescription::new();
            file_desc.define(file_bytes.into_boxed_slice());
            let file_data_id = obj_module
                .declare_anonymous_data(false, false)
                .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
            obj_module
                .define_data(file_data_id, &file_desc)
                .map_err(|e| CodegenError::FunctionDefine(e.to_string()))?;
            let file_gv = obj_module.declare_data_in_func(file_data_id, builder.func);
            vow_file_global_values.insert(vow_entry.id.0, file_gv);

            for (idx, (name, _)) in vow_entry.bindings.iter().enumerate() {
                let mut name_bytes = name.as_bytes().to_vec();
                name_bytes.push(0);
                let mut name_desc = DataDescription::new();
                name_desc.define(name_bytes.into_boxed_slice());
                let name_data_id = obj_module
                    .declare_anonymous_data(false, false)
                    .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
                obj_module
                    .define_data(name_data_id, &name_desc)
                    .map_err(|e| CodegenError::FunctionDefine(e.to_string()))?;
                let name_gv = obj_module.declare_data_in_func(name_data_id, builder.func);
                vow_binding_name_gvs.insert((vow_entry.id.0, idx as u32), name_gv);
            }
        }
    }

    // Declare extern function refs in this function
    let mut extern_func_refs: HashMap<String, FuncRef> = HashMap::new();
    for (sym, &cl_id) in extern_func_ids {
        let fref = obj_module.declare_func_in_func(cl_id, builder.func);
        extern_func_refs.insert(sym.clone(), fref);
    }

    // Trace function refs and function name data
    let trace_enter_ref = runtime
        .trace_enter_id
        .map(|id| obj_module.declare_func_in_func(id, builder.func));
    let trace_exit_ref = runtime
        .trace_exit_id
        .map(|id| obj_module.declare_func_in_func(id, builder.func));
    let trace_vow_ref = runtime
        .trace_vow_id
        .map(|id| obj_module.declare_func_in_func(id, builder.func));
    let profile_enter_ref = runtime
        .profile_enter_id
        .map(|id| obj_module.declare_func_in_func(id, builder.func));
    let profile_init_ref = runtime
        .profile_init_id
        .map(|id| obj_module.declare_func_in_func(id, builder.func));
    let stack_enter_ref = runtime
        .stack_enter_id
        .map(|id| obj_module.declare_func_in_func(id, builder.func));
    let stack_exit_ref = runtime
        .stack_exit_id
        .map(|id| obj_module.declare_func_in_func(id, builder.func));
    let needs_fn_name =
        trace != TraceMode::Off || mode == BuildMode::Profile || mode.has_debug_checks();
    let fn_name_gv = if needs_fn_name {
        let mut name_bytes = ir_func.name.as_bytes().to_vec();
        name_bytes.push(0);
        let mut desc = DataDescription::new();
        desc.define(name_bytes.into_boxed_slice());
        let data_id = obj_module
            .declare_anonymous_data(false, false)
            .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
        obj_module
            .define_data(data_id, &desc)
            .map_err(|e| CodegenError::FunctionDefine(e.to_string()))?;
        Some(obj_module.declare_data_in_func(data_id, builder.func))
    } else {
        None
    };

    // Collect entry block arg Values → ArgIndex map
    let mut arg_values: HashMap<u32, Value> = HashMap::new();
    let mut hidden_region_values: Vec<Value> = Vec::new();
    if let Some(first) = ir_func.blocks.first() {
        let entry_cl = block_map[&first.id];
        builder.switch_to_block(entry_cl);
        let entry_params = builder.block_params(entry_cl).to_vec();
        let mut cl_idx = 0usize;
        for (ir_idx, &param_ty) in ir_func.params.iter().enumerate() {
            if ir_ty_to_cranelift(param_ty).is_some() {
                arg_values.insert(ir_idx as u32, entry_params[cl_idx]);
                cl_idx += 1;
            }
        }
        hidden_region_values.extend(entry_params[cl_idx..].iter().copied());
        if ir_func.name == "main" {
            let root_arena = builder.ins().global_value(types::I64, root_arena_gv);
            hidden_region_values.push(root_arena);
            let runtime_start_ref =
                obj_module.declare_func_in_func(runtime.runtime_start_id, builder.func);
            builder.ins().call(runtime_start_ref, &[]);
            let guard_ref =
                obj_module.declare_func_in_func(runtime.stack_guard_init_id, builder.func);
            builder.ins().call(guard_ref, &[]);
        }
        if trace != TraceMode::Off
            && let (Some(enter_ref), Some(gv)) = (trace_enter_ref, fn_name_gv)
        {
            let name_ptr = builder.ins().global_value(types::I64, gv);
            builder.ins().call(enter_ref, &[name_ptr]);
        }
        if ir_func.name == "main"
            && let Some(init_ref) = profile_init_ref
        {
            builder.ins().call(init_ref, &[]);
        }
        if let (Some(prof_ref), Some(gv)) = (profile_enter_ref, fn_name_gv) {
            let name_ptr = builder.ins().global_value(types::I64, gv);
            builder.ins().call(prof_ref, &[name_ptr]);
        }
        if let (Some(se_ref), Some(gv)) = (stack_enter_ref, fn_name_gv) {
            let name_ptr = builder.ins().global_value(types::I64, gv);
            builder.ins().call(se_ref, &[name_ptr]);
        }
        if ir_func.name == "main"
            && let Some(init_id) = runtime.sanitize_init_id
        {
            let init_ref = obj_module.declare_func_in_func(init_id, builder.func);
            builder.ins().call(init_ref, &[]);
        }
    }

    let mut value_map: HashMap<InstId, Value> = HashMap::new();
    let mut block_arena_slots: BTreeMap<BlockId, StackSlot> = BTreeMap::new();
    for block_id in collect_block_arena_ids(ir_func) {
        let slot = block_arena_slot(&mut builder, &mut block_arena_slots, block_id);
        let arena_addr = builder.ins().stack_addr(types::I64, slot, 0);
        builder.ins().call(arena_init_ref, &[arena_addr]);
    }

    // Emit each block
    let mut first_block = true;
    for ir_block in &ir_func.blocks {
        let cl_block = block_map[&ir_block.id];
        if !first_block {
            builder.switch_to_block(cl_block);
        }
        first_block = false;

        // Populate value_map with Phi block param values
        if let Some(phi_ids) = phi_data.block_phis.get(&ir_block.id) {
            let params = builder.block_params(cl_block).to_vec();
            for (i, &phi_id) in phi_ids.iter().enumerate() {
                if let Some(&v) = params.get(i) {
                    value_map.insert(phi_id, v);
                }
            }
        }

        let mut lctx = LowerCtx {
            value_map: &mut value_map,
            block_map: &block_map,
            phi_data: &phi_data,
            arg_values: &arg_values,
            hidden_region_values: &hidden_region_values,
            return_ty: ir_func.return_ty,
            ir_func_id_to_ref: &ir_func_id_to_ref,
            vow_violation_ref,
            overflow_ref,
            arena_alloc_ref,
            arena_open_ref,
            arena_close_ref,
            string_clone_ref,
            block_arena_slots: &mut block_arena_slots,
            func_summaries,
            mode,
            trace,
            current_ir_block: ir_block.id,
            string_global_values: &string_global_values,
            extern_func_refs: &extern_func_refs,
            vow_desc_global_values: &vow_desc_global_values,
            vow_file_global_values: &vow_file_global_values,
            vow_binding_name_gvs: &vow_binding_name_gvs,
            inst_ty_map: &inst_ty_map,
            inst_index: &inst_index,
            ir_func,
            trace_exit_ref,
            trace_vow_ref,
            fn_name_gv,
            stack_exit_ref,
            root_arena_gv,
        };

        for inst in &ir_block.insts {
            lower_inst(&mut builder, inst, &mut lctx)?;
        }
    }

    builder.seal_all_blocks();
    builder.finalize();
    Ok(())
}

fn make_extern_sig(sym: &str, obj_module: &ObjectModule) -> Signature {
    let call_conv = obj_module.isa().default_call_conv();
    let mut sig = Signature::new(call_conv);
    match sym {
        "__vow_print_str" => {
            sig.params.push(AbiParam::new(types::I64)); // ptr
        }
        "__vow_print_i64" | "__vow_print_u64" => {
            sig.params.push(AbiParam::new(types::I64)); // value
        }
        "__vow_vec_new" => {
            sig.params.push(AbiParam::new(types::I64)); // elem_size
            sig.params.push(AbiParam::new(types::I64)); // elem_align
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec
        }
        "__vow_vec_new_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // elem_size
            sig.params.push(AbiParam::new(types::I64)); // elem_align
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec
        }
        "__vow_vec_new_val" => {
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<i64>
        }
        "__vow_vec_new_val_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<i64>
        }
        "__vow_vec_from_raw_parts_copy_val" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // raw i64 ptr
            sig.params.push(AbiParam::new(types::I64)); // len
            sig.returns.push(AbiParam::new(types::I64)); // copied *VowVec
        }
        "__vow_vec_pin_to_root_val" => {
            sig.params.push(AbiParam::new(types::I64)); // source vec ptr
            sig.returns.push(AbiParam::new(types::I64)); // copied *VowVec
        }
        "__vow_vec_len" => {
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
            sig.returns.push(AbiParam::new(types::I64)); // len
        }
        "__vow_vec_push_val" => {
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
            sig.params.push(AbiParam::new(types::I64)); // value (i64)
        }
        "__vow_vec_push_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
            sig.params.push(AbiParam::new(types::I64)); // value ptr
            sig.params.push(AbiParam::new(types::I64)); // elem_size
            sig.params.push(AbiParam::new(types::I64)); // elem_align
        }
        "__vow_vec_push_val_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
            sig.params.push(AbiParam::new(types::I64)); // value (i64)
        }
        "__vow_vec_reserve_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
            sig.params.push(AbiParam::new(types::I64)); // additional
            sig.params.push(AbiParam::new(types::I64)); // elem_size
            sig.params.push(AbiParam::new(types::I64)); // elem_align
        }
        "__vow_vec_get_val" => {
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
            sig.params.push(AbiParam::new(types::I64)); // index
            sig.returns.push(AbiParam::new(types::I64)); // element value
        }
        "__vow_vec_set_val" => {
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
            sig.params.push(AbiParam::new(types::I64)); // index
            sig.params.push(AbiParam::new(types::I64)); // value
        }
        "__vow_vec_pop" => {
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
        }
        "__vow_vec_clear" => {
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
        }
        "__vow_vec_truncate" => {
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
            sig.params.push(AbiParam::new(types::I64)); // new_len
        }
        // String runtime
        "__vow_string_new" => {
            sig.params.push(AbiParam::new(types::I64)); // ptr
            sig.params.push(AbiParam::new(types::I64)); // len
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_new_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // ptr
            sig.params.push(AbiParam::new(types::I64)); // len
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_from_cstr" => {
            sig.params.push(AbiParam::new(types::I64)); // C-string ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_from_cstr_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // C-string ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_pin_to_root" => {
            sig.params.push(AbiParam::new(types::I64)); // source string ptr
            sig.returns.push(AbiParam::new(types::I64)); // root-pinned *VowVec<u8>
        }
        "__vow_string_from_raw_parts_copy" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // raw bytes ptr
            sig.params.push(AbiParam::new(types::I64)); // len
            sig.returns.push(AbiParam::new(types::I64)); // copied *VowVec<u8>
        }
        "__vow_string_len" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.returns.push(AbiParam::new(types::I64)); // len
        }
        "__vow_string_clear" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
        }
        "__vow_string_eq" => {
            sig.params.push(AbiParam::new(types::I64)); // a ptr
            sig.params.push(AbiParam::new(types::I64)); // b ptr
            sig.returns.push(AbiParam::new(types::I8)); // bool
        }
        "__vow_string_contains" => {
            sig.params.push(AbiParam::new(types::I64)); // haystack ptr
            sig.params.push(AbiParam::new(types::I64)); // needle ptr
            sig.returns.push(AbiParam::new(types::I8)); // bool
        }
        "__vow_string_matches_literal_at" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.params.push(AbiParam::new(types::I64)); // offset
            sig.params.push(AbiParam::new(types::I64)); // literal bytes ptr
            sig.params.push(AbiParam::new(types::I64)); // literal byte len
            sig.returns.push(AbiParam::new(types::I64)); // 1 if matched, else 0
        }
        "__vow_string_push_str" => {
            sig.params.push(AbiParam::new(types::I64)); // dest ptr
            sig.params.push(AbiParam::new(types::I64)); // src ptr
        }
        "__vow_string_push_str_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // dest ptr
            sig.params.push(AbiParam::new(types::I64)); // src ptr
        }
        "__vow_string_byte_at" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.params.push(AbiParam::new(types::I64)); // index
            sig.returns.push(AbiParam::new(types::I64)); // byte value or -1
        }
        "__vow_string_push_byte" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.params.push(AbiParam::new(types::I64)); // byte value
        }
        "__vow_string_push_byte_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.params.push(AbiParam::new(types::I64)); // byte value
        }
        "__vow_string_from_i64" => {
            sig.params.push(AbiParam::new(types::I64)); // value
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_from_i64_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // value
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_print" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
        }
        // File I/O runtime
        "__vow_fs_read" => {
            sig.params.push(AbiParam::new(types::I64)); // path C-string
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8> or null
        }
        "__vow_fs_open" => {
            sig.params.push(AbiParam::new(types::I64)); // path *VowVec<u8>
            sig.returns.push(AbiParam::new(types::I64)); // positive handle or -1
        }
        "__vow_fs_read_line" => {
            sig.params.push(AbiParam::new(types::I64)); // file handle
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_fs_status" => {
            sig.params.push(AbiParam::new(types::I64)); // file handle
            sig.returns.push(AbiParam::new(types::I64)); // 0=active, 1=EOF, -1=err
        }
        "__vow_fs_close" => {
            sig.params.push(AbiParam::new(types::I64)); // file handle
            sig.returns.push(AbiParam::new(types::I64)); // 0=ok, -1=err
        }
        "__vow_fs_write" => {
            sig.params.push(AbiParam::new(types::I64)); // path C-string
            sig.params.push(AbiParam::new(types::I64)); // data *VowVec<u8>
            sig.returns.push(AbiParam::new(types::I64)); // 0=ok, -1=err
        }
        "__vow_fs_exists" => {
            sig.params.push(AbiParam::new(types::I64)); // path *VowVec<u8>
            sig.returns.push(AbiParam::new(types::I64)); // 1=exists, 0=not
        }
        "__vow_fs_mkdir" => {
            sig.params.push(AbiParam::new(types::I64)); // path *VowVec<u8>
            sig.returns.push(AbiParam::new(types::I64)); // 0=ok, -1=err
        }
        "__vow_fs_listdir" => {
            sig.params.push(AbiParam::new(types::I64)); // path *VowVec<u8>
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<String>
        }
        "__vow_fs_remove" => {
            sig.params.push(AbiParam::new(types::I64)); // path *VowVec<u8>
            sig.returns.push(AbiParam::new(types::I64)); // 0=ok, -1=err
        }
        "__vow_fs_remove_dir" => {
            sig.params.push(AbiParam::new(types::I64)); // path *VowVec<u8>
            sig.returns.push(AbiParam::new(types::I64)); // 0=ok, -1=err
        }
        "__vow_fs_is_dir" => {
            sig.params.push(AbiParam::new(types::I64)); // path *VowVec<u8>
            sig.returns.push(AbiParam::new(types::I64)); // 1=dir, 0=not
        }
        "__vow_fs_rename" => {
            sig.params.push(AbiParam::new(types::I64)); // old path *VowVec<u8>
            sig.params.push(AbiParam::new(types::I64)); // new path *VowVec<u8>
            sig.returns.push(AbiParam::new(types::I64)); // 0=ok, -1=err
        }
        "__vow_string_substr" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.params.push(AbiParam::new(types::I64)); // start
            sig.params.push(AbiParam::new(types::I64)); // len
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_substr_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.params.push(AbiParam::new(types::I64)); // start
            sig.params.push(AbiParam::new(types::I64)); // len
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_substring" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.params.push(AbiParam::new(types::I64)); // start
            sig.params.push(AbiParam::new(types::I64)); // end (exclusive)
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_substring_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.params.push(AbiParam::new(types::I64)); // start
            sig.params.push(AbiParam::new(types::I64)); // end (exclusive)
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_parse_i64_opt" | "__vow_string_parse_u64_opt" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.returns.push(AbiParam::new(types::I64)); // *Option enum (16 bytes: tag+payload)
        }
        "__vow_string_split" => {
            sig.params.push(AbiParam::new(types::I64)); // haystack ptr
            sig.params.push(AbiParam::new(types::I64)); // separator ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<String>
        }
        "__vow_string_split_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // haystack ptr
            sig.params.push(AbiParam::new(types::I64)); // separator ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<String>
        }
        "__vow_string_starts_with" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.params.push(AbiParam::new(types::I64)); // prefix ptr
            sig.returns.push(AbiParam::new(types::I64)); // 1/0
        }
        "__vow_string_ends_with" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.params.push(AbiParam::new(types::I64)); // suffix ptr
            sig.returns.push(AbiParam::new(types::I64)); // 1/0
        }
        "__vow_string_trim" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_trim_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_to_upper" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_to_upper_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_to_lower" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_to_lower_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_replace" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.params.push(AbiParam::new(types::I64)); // from ptr
            sig.params.push(AbiParam::new(types::I64)); // to ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_replace_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.params.push(AbiParam::new(types::I64)); // from ptr
            sig.params.push(AbiParam::new(types::I64)); // to ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_join" => {
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
            sig.params.push(AbiParam::new(types::I64)); // separator ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_join_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
            sig.params.push(AbiParam::new(types::I64)); // separator ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_parse_i64" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.returns.push(AbiParam::new(types::I64)); // parsed value
        }
        "__vow_vec_sort" => {
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
            sig.returns.push(AbiParam::new(types::I64)); // new sorted vec ptr
        }
        "__vow_time_unix" => {
            sig.returns.push(AbiParam::new(types::I64)); // unix timestamp
        }
        "__vow_time_unix_ms" => {
            sig.returns.push(AbiParam::new(types::I64)); // unix timestamp ms
        }
        "__vow_num_cpus" => {
            sig.returns.push(AbiParam::new(types::I64)); // available CPU count
        }
        "__vow_memory_root_arena_bytes"
        | "__vow_memory_peak_bytes"
        | "__vow_memory_alloc_count_since_start" => {
            sig.returns.push(AbiParam::new(types::I64)); // u64 counter
        }
        "__vow_hex_encode" => {
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
            sig.returns.push(AbiParam::new(types::I64)); // string ptr
        }
        "__vow_hex_decode" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.returns.push(AbiParam::new(types::I64)); // vec ptr
        }
        "__vow_eprintln_str" => {
            sig.params.push(AbiParam::new(types::I64)); // C-string ptr
        }
        "__vow_debug_str" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
        }
        "__vow_debug_i64" | "__vow_debug_u64" => {
            sig.params.push(AbiParam::new(types::I64)); // value
        }
        "__vow_args" => {
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<String>
        }
        "__vow_stdin_read" => {
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_stdin_read_line" => {
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_stdin_ready" => {
            sig.returns.push(AbiParam::new(types::I64)); // bool as i64
        }
        "__vow_process_exit" => {
            sig.params.push(AbiParam::new(types::I64)); // exit code
        }
        "__vow_process_run" => {
            sig.params.push(AbiParam::new(types::I64)); // cmd *VowVec<u8>
            sig.params.push(AbiParam::new(types::I64)); // args *VowVec<i64>
            sig.returns.push(AbiParam::new(types::I64)); // exit code
        }
        "__vow_process_get_stdout" | "__vow_process_get_stderr" => {
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_process_start" => {
            sig.params.push(AbiParam::new(types::I64)); // cmd *VowVec<u8>
            sig.params.push(AbiParam::new(types::I64)); // args *VowVec<i64>
            sig.returns.push(AbiParam::new(types::I64)); // handle
        }
        "__vow_process_wait" => {
            sig.params.push(AbiParam::new(types::I64)); // handle
            sig.returns.push(AbiParam::new(types::I64)); // exit code
        }
        "__vow_process_wait_timeout" => {
            sig.params.push(AbiParam::new(types::I64)); // handle
            sig.params.push(AbiParam::new(types::I64)); // timeout_ms
            sig.returns.push(AbiParam::new(types::I64)); // exit code or -2 timeout
        }
        "__vow_process_kill" => {
            sig.params.push(AbiParam::new(types::I64)); // handle
            sig.returns.push(AbiParam::new(types::I64)); // 0 success, -1 error
        }
        "__vow_process_stdout_for" | "__vow_process_stderr_for" => {
            sig.params.push(AbiParam::new(types::I64)); // handle
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        // HashMap runtime
        "__vow_map_new" => {
            sig.returns.push(AbiParam::new(types::I64)); // *VowMap
        }
        "__vow_map_new_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.returns.push(AbiParam::new(types::I64)); // *VowMap
        }
        "__vow_map_insert" => {
            sig.params.push(AbiParam::new(types::I64)); // map ptr
            sig.params.push(AbiParam::new(types::I64)); // key
            sig.params.push(AbiParam::new(types::I64)); // value
        }
        "__vow_map_insert_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // map ptr
            sig.params.push(AbiParam::new(types::I64)); // key
            sig.params.push(AbiParam::new(types::I64)); // value
        }
        "__vow_map_get" => {
            sig.params.push(AbiParam::new(types::I64)); // map ptr
            sig.params.push(AbiParam::new(types::I64)); // key
            sig.returns.push(AbiParam::new(types::I64)); // value (0 if not found)
        }
        "__vow_map_contains" => {
            sig.params.push(AbiParam::new(types::I64)); // map ptr
            sig.params.push(AbiParam::new(types::I64)); // key
            sig.returns.push(AbiParam::new(types::I8)); // bool
        }
        "__vow_map_remove" => {
            sig.params.push(AbiParam::new(types::I64)); // map ptr
            sig.params.push(AbiParam::new(types::I64)); // key
        }
        "__vow_map_remove_in_arena" => {
            sig.params.push(AbiParam::new(types::I64)); // target arena
            sig.params.push(AbiParam::new(types::I64)); // map ptr
            sig.params.push(AbiParam::new(types::I64)); // key
        }
        "__vow_map_len" => {
            sig.params.push(AbiParam::new(types::I64)); // map ptr
            sig.returns.push(AbiParam::new(types::I64)); // len
        }
        // BTreeMap runtime — sorted parallel-Vec backing
        "__vow_btreemap_new" => {
            sig.returns.push(AbiParam::new(types::I64)); // *VowBTreeMap
        }
        "__vow_btreemap_len" => {
            sig.params.push(AbiParam::new(types::I64)); // map ptr
            sig.returns.push(AbiParam::new(types::I64)); // len
        }
        "__vow_btreemap_insert" => {
            sig.params.push(AbiParam::new(types::I64)); // map ptr
            sig.params.push(AbiParam::new(types::I64)); // key
            sig.params.push(AbiParam::new(types::I64)); // value
            sig.returns.push(AbiParam::new(types::I64)); // *VowOption (prev)
        }
        "__vow_btreemap_get" => {
            sig.params.push(AbiParam::new(types::I64)); // map ptr
            sig.params.push(AbiParam::new(types::I64)); // key
            sig.returns.push(AbiParam::new(types::I64)); // *VowOption
        }
        "__vow_btreemap_contains" => {
            sig.params.push(AbiParam::new(types::I64)); // map ptr
            sig.params.push(AbiParam::new(types::I64)); // key
            sig.returns.push(AbiParam::new(types::I8)); // bool
        }
        // Cranelift shim FFI (used by self-hosted compiler)
        "__vow_clif_create" => {
            sig.params.push(AbiParam::new(types::I64)); // mode
            sig.params.push(AbiParam::new(types::I64)); // trace_mode
            sig.returns.push(AbiParam::new(types::I64)); // ctx handle
        }
        "__vow_clif_add_string" => {
            sig.params.push(AbiParam::new(types::I64)); // ctx
            sig.params.push(AbiParam::new(types::I64)); // str VowVec ptr
        }
        "__vow_clif_declare_extern" => {
            sig.params.push(AbiParam::new(types::I64)); // ctx
            sig.params.push(AbiParam::new(types::I64)); // sym VowVec ptr
        }
        "__vow_clif_declare_function" => {
            sig.params.push(AbiParam::new(types::I64)); // ctx
            sig.params.push(AbiParam::new(types::I64)); // idx
            sig.params.push(AbiParam::new(types::I64)); // name VowVec ptr
            sig.params.push(AbiParam::new(types::I64)); // param_tys Vec
            sig.params.push(AbiParam::new(types::I64)); // n_params
            sig.params.push(AbiParam::new(types::I64)); // ret_ty
            sig.params.push(AbiParam::new(types::I64)); // is_main
            sig.params.push(AbiParam::new(types::I64)); // return summary kind
            sig.params.push(AbiParam::new(types::I64)); // store_effects Vec
        }
        // Incremental per-function FFI (replaces the batched
        // __vow_clif_compile_function). Signatures must match
        // vow-clif-shim/src/lib.rs.
        "__vow_clif_fn_begin" => {
            for _ in 0..4 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_clif_fn_block" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_clif_fn_inst" => {
            for _ in 0..10 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_clif_fn_vow" => {
            for _ in 0..5 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_clif_fn_end" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_clif_finish" => {
            sig.params.push(AbiParam::new(types::I64)); // ctx
            sig.params.push(AbiParam::new(types::I64)); // obj_path VowVec ptr
            sig.returns.push(AbiParam::new(types::I64)); // result
        }
        "__vow_clif_link" => {
            sig.params.push(AbiParam::new(types::I64)); // obj_path VowVec ptr
            sig.params.push(AbiParam::new(types::I64)); // output_path VowVec ptr
            sig.returns.push(AbiParam::new(types::I64)); // result
        }
        "__vow_clif_destroy" => {
            sig.params.push(AbiParam::new(types::I64)); // ctx
        }
        // Trace instrumentation
        "__vow_trace_enter" | "__vow_trace_exit" => {
            sig.params.push(AbiParam::new(types::I64)); // fn_name C-string ptr
        }
        "__vow_trace_vow" => {
            sig.params.push(AbiParam::new(types::I64)); // fn_name C-string ptr
            sig.params.push(AbiParam::new(types::I64)); // vow_id
            sig.params.push(AbiParam::new(types::I64)); // passed (0 or 1)
        }
        // Profile instrumentation
        "__vow_profile_enter" => {
            sig.params.push(AbiParam::new(types::I64)); // fn_name C-string ptr
        }
        "__vow_profile_init" | "__vow_init_stack_guard" | "__vow_stack_exit" => {}
        "__vow_stack_enter" => {
            sig.params.push(AbiParam::new(types::I64)); // fn_name C-string ptr
        }
        _ => {}
    }
    sig
}

fn extern_uses_target_region(sym: &str) -> bool {
    matches!(
        sym,
        "__vow_string_from_raw_parts_copy" | "__vow_vec_from_raw_parts_copy_val"
    )
}

// ---------------------------------------------------------------------------
// Backend trait implementation
// ---------------------------------------------------------------------------

impl Backend for CraneliftBackend {
    fn compile_module(
        &self,
        module: &IrModule,
        mode: BuildMode,
        trace: TraceMode,
    ) -> Result<CompiledObject, CodegenError> {
        let isa = make_isa(mode)?;

        let obj_builder = ObjectBuilder::new(
            isa.clone(),
            module.name.as_bytes().to_vec(),
            cranelift_module::default_libcall_names(),
        )
        .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;

        let mut obj_module = ObjectModule::new(obj_builder);

        // Create data sections for string constants
        let mut string_data_ids: Vec<DataId> = Vec::new();
        for s in &module.strings {
            let mut bytes = s.as_bytes().to_vec();
            bytes.push(0); // null terminate
            let mut desc = DataDescription::new();
            desc.define(bytes.into_boxed_slice());
            let data_id = obj_module
                .declare_anonymous_data(false, false)
                .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
            obj_module
                .define_data(data_id, &desc)
                .map_err(|e| CodegenError::FunctionDefine(e.to_string()))?;
            string_data_ids.push(data_id);
        }

        // Scan all functions for CallExtern symbols and declare them as imports
        let mut extern_syms = HashSet::new();
        for func in &module.functions {
            let inst_index = build_inst_index(func);
            let phi_data = build_phi_upsilon_data(func);
            for block in &func.blocks {
                for inst in &block.insts {
                    if let InstData::CallExtern(sym) = &inst.data {
                        if inst.opcode == Opcode::DebugCall && !mode.has_debug_checks() {
                            continue;
                        }
                        let (routed_sym, _) =
                            routed_vec_extern(sym, inst, &inst_index, &func.summary, &phi_data);
                        extern_syms.insert(routed_sym.to_string());
                    }
                }
            }
        }
        let mut extern_func_ids: HashMap<String, CraneliftFuncId> = HashMap::new();
        for sym in &extern_syms {
            let sig = make_extern_sig(sym, &obj_module);
            let cl_id = obj_module
                .declare_function(sym, Linkage::Import, &sig)
                .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
            extern_func_ids.insert(sym.clone(), cl_id);
        }

        // Declare all Vow functions first (needed for Call resolution)
        let mut ir_to_cl: Vec<(IrFuncId, CraneliftFuncId)> = Vec::new();
        for ir_func in &module.functions {
            let sig = build_signature(ir_func, isa.default_call_conv());
            let linkage = if ir_func.name == "main" {
                Linkage::Export
            } else {
                Linkage::Local
            };
            let cl_id = obj_module
                .declare_function(&ir_func.name, linkage, &sig)
                .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
            ir_to_cl.push((ir_func.id, cl_id));
        }

        // Declare arena runtime functions (always needed by RegionAlloc and main startup).
        let mut arena_alloc_sig = obj_module.make_signature();
        arena_alloc_sig.params.push(AbiParam::new(types::I64)); // *VowArena
        arena_alloc_sig.params.push(AbiParam::new(types::I64)); // size
        arena_alloc_sig.params.push(AbiParam::new(types::I64)); // align
        arena_alloc_sig.returns.push(AbiParam::new(types::I64)); // *mut u8
        let arena_alloc_id = obj_module
            .declare_function("__vow_arena_alloc", Linkage::Import, &arena_alloc_sig)
            .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;

        let mut arena_open_close_sig = obj_module.make_signature();
        arena_open_close_sig.params.push(AbiParam::new(types::I64)); // *VowArena
        let arena_open_id = obj_module
            .declare_function("__vow_arena_open", Linkage::Import, &arena_open_close_sig)
            .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
        let arena_close_id = obj_module
            .declare_function("__vow_arena_close", Linkage::Import, &arena_open_close_sig)
            .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
        let arena_init_id = obj_module
            .declare_function(
                "__vow_arena_init_closed",
                Linkage::Import,
                &arena_open_close_sig,
            )
            .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;

        // `__vow_string_clone_into_arena` is only imported when some function
        // in the module actually emits a return-materialisation call. This
        // keeps the symbol out of object files for modules whose
        // FreshInCaller paths are all already in `target_region` (spec §5.1).
        //
        // Phase 7 / #202 follow-up: this scan builds a fresh `inst_index`
        // per qualifying function (inside `module_uses_return_materialization`),
        // and `compile_ir_function` builds another `inst_index` for the
        // same function during lowering. A `Vec<bool>` parallel to
        // `module.functions` recording the per-function flag — populated
        // here once and threaded through to `compile_ir_function` — would
        // skip the second scan. Negligible at Phase 4 scale (few functions
        // qualify); becomes worth doing once the FFI-wrapper stdlib lands
        // and synthesised functions exercise materialisation more broadly.
        let needs_string_clone = module
            .functions
            .iter()
            .any(module_uses_return_materialization);
        let string_clone_id = if needs_string_clone {
            let mut string_clone_sig = obj_module.make_signature();
            string_clone_sig.params.push(AbiParam::new(types::I64)); // *VowArena
            string_clone_sig.params.push(AbiParam::new(types::I64)); // *const u8 (source)
            string_clone_sig.returns.push(AbiParam::new(types::I64)); // *mut u8 (clone)
            Some(
                obj_module
                    .declare_function(
                        "__vow_string_clone_into_arena",
                        Linkage::Import,
                        &string_clone_sig,
                    )
                    .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?,
            )
        } else {
            None
        };

        let runtime_start_sig = obj_module.make_signature();
        let runtime_start_id = obj_module
            .declare_function("__vow_runtime_start", Linkage::Import, &runtime_start_sig)
            .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;

        let root_arena_id = obj_module
            .declare_data("__vow_root_arena", Linkage::Import, true, false)
            .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;

        // Declare external runtime functions (debug/sanitize mode only)
        let (vow_violation_id, overflow_id) = if mode.has_debug_checks() {
            let mut violation_sig = obj_module.make_signature();
            violation_sig.params.push(AbiParam::new(types::I32)); // vow_id
            violation_sig.params.push(AbiParam::new(types::I8)); // blame
            violation_sig.params.push(AbiParam::new(types::I64)); // desc_ptr
            violation_sig.params.push(AbiParam::new(types::I64)); // bindings_ptr
            violation_sig.params.push(AbiParam::new(types::I32)); // binding_count
            violation_sig.params.push(AbiParam::new(types::I64)); // file_ptr
            violation_sig.params.push(AbiParam::new(types::I32)); // offset
            let vv_id = obj_module
                .declare_function("__vow_violation", Linkage::Import, &violation_sig)
                .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;

            let overflow_sig = obj_module.make_signature();
            let ov_id = obj_module
                .declare_function("__vow_arithmetic_overflow", Linkage::Import, &overflow_sig)
                .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;

            (Some(vv_id), Some(ov_id))
        } else {
            (None, None)
        };

        // Declare trace runtime functions
        let (trace_enter_id, trace_exit_id, trace_vow_id) = if trace != TraceMode::Off {
            let enter_sig = make_extern_sig("__vow_trace_enter", &obj_module);
            let enter_id = obj_module
                .declare_function("__vow_trace_enter", Linkage::Import, &enter_sig)
                .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
            let exit_sig = make_extern_sig("__vow_trace_exit", &obj_module);
            let exit_id = obj_module
                .declare_function("__vow_trace_exit", Linkage::Import, &exit_sig)
                .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
            let vow_sig = if trace == TraceMode::Full {
                let s = make_extern_sig("__vow_trace_vow", &obj_module);
                let id = obj_module
                    .declare_function("__vow_trace_vow", Linkage::Import, &s)
                    .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
                Some(id)
            } else {
                None
            };
            (Some(enter_id), Some(exit_id), vow_sig)
        } else {
            (None, None, None)
        };

        // Declare profile runtime functions
        let (profile_enter_id, profile_init_id) = if mode == BuildMode::Profile {
            let enter_sig = make_extern_sig("__vow_profile_enter", &obj_module);
            let enter_id = obj_module
                .declare_function("__vow_profile_enter", Linkage::Import, &enter_sig)
                .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
            let init_sig = make_extern_sig("__vow_profile_init", &obj_module);
            let init_id = obj_module
                .declare_function("__vow_profile_init", Linkage::Import, &init_sig)
                .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
            (Some(enter_id), Some(init_id))
        } else {
            (None, None)
        };

        // Declare sanitize init function (sanitize mode only)
        let sanitize_init_id = if mode == BuildMode::Sanitize {
            let sig = obj_module.make_signature();
            let id = obj_module
                .declare_function("__vow_sanitize_init", Linkage::Import, &sig)
                .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
            Some(id)
        } else {
            None
        };

        // Declare stack guard init (always — signal handler works in all modes)
        let stack_guard_init_id = {
            let sig = make_extern_sig("__vow_init_stack_guard", &obj_module);
            obj_module
                .declare_function("__vow_init_stack_guard", Linkage::Import, &sig)
                .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?
        };

        // Declare stack depth tracking (debug/sanitize mode only)
        let (stack_enter_id, stack_exit_id) = if mode.has_debug_checks() {
            let enter_sig = make_extern_sig("__vow_stack_enter", &obj_module);
            let enter_id = obj_module
                .declare_function("__vow_stack_enter", Linkage::Import, &enter_sig)
                .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
            let exit_sig = make_extern_sig("__vow_stack_exit", &obj_module);
            let exit_id = obj_module
                .declare_function("__vow_stack_exit", Linkage::Import, &exit_sig)
                .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;
            (Some(enter_id), Some(exit_id))
        } else {
            (None, None)
        };

        // Pre-collect each callable's region summary, keyed by IR FuncId.
        // Internal call lowering reads this to project the callee's hidden
        // region parameters (target_region for FreshInCaller; one per
        // distinct StoreEffect.target) from the caller's frame.
        let func_summaries: HashMap<IrFuncId, RegionSummary> = module
            .functions
            .iter()
            .map(|f| (f.id, f.summary.clone()))
            .collect();

        // Compile each function
        let mut builder_ctx = FunctionBuilderContext::new();
        for (ir_func, &(_, cl_id)) in module.functions.iter().zip(ir_to_cl.iter()) {
            let sig = build_signature(ir_func, isa.default_call_conv());
            let mut cl_ctx = obj_module.make_context();
            cl_ctx.func.signature = sig;

            compile_ir_function(
                &mut cl_ctx,
                ir_func,
                &mut builder_ctx,
                mode,
                trace,
                &mut obj_module,
                &ir_to_cl,
                &RuntimeIds {
                    vow_violation_id,
                    overflow_id,
                    arena_alloc_id,
                    arena_init_id,
                    arena_open_id,
                    arena_close_id,
                    string_clone_id,
                    runtime_start_id,
                    root_arena_id,
                    trace_enter_id,
                    trace_exit_id,
                    trace_vow_id,
                    profile_enter_id,
                    profile_init_id,
                    sanitize_init_id,
                    stack_guard_init_id,
                    stack_enter_id,
                    stack_exit_id,
                },
                &string_data_ids,
                &extern_func_ids,
                &func_summaries,
            )?;

            if let Err(e) = obj_module.define_function(cl_id, &mut cl_ctx) {
                return Err(CodegenError::FunctionDefine(format!(
                    "in function '{}': {}",
                    ir_func.name, e
                )));
            }
            obj_module.clear_context(&mut cl_ctx);
        }

        let product = obj_module.finish();
        let bytes = product
            .emit()
            .map_err(|e| CodegenError::Emit(e.to_string()))?;

        Ok(CompiledObject { bytes })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use vow_ir::{
        BasicBlock, BlockId, FuncId, Function, HiddenRegionIdx, InstData, InstId, Module, Opcode,
        RegionConstraint, RegionId, RegionSummary, StoreEffect, Ty, VowEntry, VowId,
    };
    use vow_syntax::span::Span;

    fn sp() -> Span {
        Span::new(0, 0)
    }

    fn make_module(name: &str, funcs: Vec<Function>) -> Module {
        Module {
            name: name.to_string(),
            functions: funcs,
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        }
    }

    fn inst(id: u32, op: Opcode, ty: Ty, args: Vec<u32>, data: InstData) -> Inst {
        Inst {
            id: InstId(id),
            opcode: op,
            ty,
            args: args.into_iter().map(InstId).collect(),
            data,
            origin: sp(),
            region: RegionId::Root,
        }
    }

    #[test]
    fn compile_empty_void_function() {
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "empty".to_string(),
                params: vec![],
                param_names: vec![],
                return_ty: Ty::Unit,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![inst(0, Opcode::Return, Ty::Unit, vec![], InstData::None)],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
        assert!(!result.unwrap().bytes.is_empty());
    }

    #[test]
    fn compile_constant_return_i64() {
        // fn answer() -> i64 { 42 }
        //   block0: v0 = const_i64(42); return v0
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "answer".to_string(),
                params: vec![],
                param_names: vec![],
                return_ty: Ty::I64,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(42)),
                        inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_add_two_i64_params() {
        // fn add(a: i64, b: i64) -> i64 { a + b }
        //   block0: v0=get_arg(0); v1=get_arg(1); v2=wrap_add(v0,v1); return v2
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "add".to_string(),
                params: vec![Ty::I64, Ty::I64],
                param_names: vec![],
                return_ty: Ty::I64,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                        inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                        inst(
                            2,
                            Opcode::WrappingAddI64,
                            Ty::I64,
                            vec![0, 1],
                            InstData::None,
                        ),
                        inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_if_else_with_phi() {
        // fn choose(cond: bool, a: i64, b: i64) -> i64 { if cond { a } else { b } }
        //   block0: v0=get_arg(0); v1=get_arg(1); v2=get_arg(2); branch(v0, block1, block2)
        //   block1: upsilon(v1 → phi); jump(block3)
        //   block2: upsilon(v2 → phi); jump(block3)
        //   block3: phi; return phi
        use vow_ir::{VowEntry, VowId};
        let phi_id = InstId(6);
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "choose".to_string(),
                params: vec![Ty::Bool, Ty::I64, Ty::I64],
                param_names: vec![],
                return_ty: Ty::I64,
                effects: vec![],
                vows: vec![],
                blocks: vec![
                    BasicBlock {
                        id: BlockId(0),
                        insts: vec![
                            inst(0, Opcode::GetArg, Ty::Bool, vec![], InstData::ArgIndex(0)),
                            inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                            inst(2, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(2)),
                            Inst {
                                id: InstId(3),
                                opcode: Opcode::Branch,
                                ty: Ty::Unit,
                                args: vec![InstId(0)],
                                data: InstData::BranchTargets {
                                    then_block: BlockId(1),
                                    else_block: BlockId(2),
                                },
                                origin: sp(),
                                region: RegionId::Root,
                            },
                        ],
                    },
                    BasicBlock {
                        id: BlockId(1),
                        insts: vec![
                            inst(
                                4,
                                Opcode::Upsilon,
                                Ty::Unit,
                                vec![1],
                                InstData::PhiTarget(phi_id),
                            ),
                            inst(
                                5,
                                Opcode::Jump,
                                Ty::Unit,
                                vec![],
                                InstData::JumpTarget(BlockId(3)),
                            ),
                        ],
                    },
                    BasicBlock {
                        id: BlockId(2),
                        insts: vec![
                            inst(
                                7,
                                Opcode::Upsilon,
                                Ty::Unit,
                                vec![2],
                                InstData::PhiTarget(phi_id),
                            ),
                            inst(
                                8,
                                Opcode::Jump,
                                Ty::Unit,
                                vec![],
                                InstData::JumpTarget(BlockId(3)),
                            ),
                        ],
                    },
                    BasicBlock {
                        id: BlockId(3),
                        insts: vec![
                            Inst {
                                id: phi_id,
                                opcode: Opcode::Phi,
                                ty: Ty::I64,
                                args: vec![],
                                data: InstData::None,
                                origin: sp(),
                                region: RegionId::Root,
                            },
                            inst(9, Opcode::Return, Ty::Unit, vec![6], InstData::None),
                        ],
                    },
                ],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let _ = VowId(0); // suppress unused import
        let _ = VowEntry {
            id: vow_ir::VowId(0),
            description: String::new(),
            blame: vow_diag::Blame::None,
            bindings: vec![],
            file: String::new(),
            offset: 0,
        };
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn type_mapping_covers_all_ir_types() {
        assert!(ir_ty_to_cranelift(IrTy::I32).is_some());
        assert!(ir_ty_to_cranelift(IrTy::I64).is_some());
        assert!(ir_ty_to_cranelift(IrTy::F32).is_some());
        assert!(ir_ty_to_cranelift(IrTy::F64).is_some());
        assert!(ir_ty_to_cranelift(IrTy::Bool).is_some());
        assert!(ir_ty_to_cranelift(IrTy::Unit).is_none());
        assert!(ir_ty_to_cranelift(IrTy::Ptr).is_some());
        assert!(ir_ty_to_cranelift(IrTy::LinearPtr).is_some());
    }

    #[test]
    fn signature_with_params_and_return() {
        use cranelift_codegen::isa::CallConv;
        let ir_func = Function {
            id: FuncId(0),
            name: "divide".to_string(),
            params: vec![Ty::I64, Ty::I64],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let sig = build_signature(&ir_func, CallConv::SystemV);
        assert_eq!(sig.params.len(), 2);
        assert_eq!(sig.returns.len(), 1);
        assert_eq!(sig.params[0].value_type, types::I64);
        assert_eq!(sig.returns[0].value_type, types::I64);
    }

    /// Acceptance test 1 from issue #198: a `FreshInCaller` return + one
    /// store-effect parameter gets two hidden `*VowArena` params, with
    /// `target_region` first and the store-target hidden region second.
    #[test]
    fn signature_projects_hidden_regions_from_full_summary() {
        use cranelift_codegen::isa::CallConv;
        let ir_func = Function {
            id: FuncId(0),
            name: "mutating_builder".to_string(),
            params: vec![Ty::Ptr, Ty::I64],
            param_names: vec![],
            return_ty: Ty::Ptr,
            effects: vec![],
            vows: vec![],
            blocks: vec![],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary {
                param_regions: vec![],
                return_region: RegionConstraint::FreshInCaller,
                store_effects: vec![StoreEffect {
                    target: 0,
                    source: RegionConstraint::FreshInCaller,
                }],
            },
            source_file: String::new(),
        };

        let sig = build_signature(&ir_func, CallConv::SystemV);

        assert_eq!(
            sig.params.len(),
            4,
            "two user params plus target_region and one store-target hidden region"
        );
        assert_eq!(sig.params[2].value_type, types::I64);
        assert_eq!(sig.params[3].value_type, types::I64);
        assert_eq!(sig.returns.len(), 1);
    }

    #[test]
    fn signature_omits_hidden_regions_for_exported_main() {
        use cranelift_codegen::isa::CallConv;
        let ir_func = Function {
            id: FuncId(0),
            name: "main".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I32,
            effects: vec![],
            vows: vec![],
            blocks: vec![],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary {
                param_regions: vec![],
                return_region: RegionConstraint::FreshInCaller,
                store_effects: vec![StoreEffect {
                    target: 0,
                    source: RegionConstraint::FreshInCaller,
                }],
            },
            source_file: String::new(),
        };

        let sig = build_signature(&ir_func, CallConv::SystemV);

        assert_eq!(
            sig.params.len(),
            0,
            "C ABI main must not take hidden regions"
        );
        assert_eq!(sig.returns.len(), 1);
    }

    #[test]
    fn cranelift_backend_default_impl() {
        let _ = CraneliftBackend {};
    }

    /// Regression for the Codex review on PR #230: `build_signature`
    /// special-cases `main` to 0 hidden region params (C ABI), so a call
    /// site that targets `main` must not project hidden args from
    /// `main`'s summary — even when the summary says `FreshInCaller` or
    /// has non-empty `store_effects`. Otherwise the caller's call args
    /// overshoot `main`'s declared signature and Cranelift verification
    /// fails.
    #[test]
    fn call_to_main_with_nonempty_summary_does_not_overshoot_signature() {
        // helper(): calls main(); returns i32.
        // main(): summary says FreshInCaller + store_effect on param 0,
        //   but signature is 0-arg per the §5.4 / `hidden_region_param_count`
        //   exception. The call site MUST NOT push hidden args.
        // `main`'s summary deliberately mixes `FreshInCaller` AND a
        // `store_effect` — region inference wouldn't normally produce both
        // for `main`, but the codegen MUST be robust to whatever summary
        // it sees, since the caller's projection reads from the summary.
        let main_fn = Function {
            id: FuncId(0),
            name: "main".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I32,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::ConstI32, Ty::I32, vec![], InstData::ConstI32(0)),
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary {
                param_regions: vec![],
                return_region: RegionConstraint::FreshInCaller,
                store_effects: vec![StoreEffect {
                    target: 0,
                    source: RegionConstraint::FreshInCaller,
                }],
            },
            source_file: String::new(),
        };

        let helper = Function {
            id: FuncId(1),
            name: "helper".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I32,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(
                        0,
                        Opcode::Call,
                        Ty::I32,
                        vec![],
                        InstData::CallTarget(FuncId(0)),
                    ),
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };

        let module = make_module("test", vec![main_fn, helper]);
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(
            result.is_ok(),
            "calling main with a non-empty summary must not break codegen: {:?}",
            result.err()
        );
    }

    fn simple_fn(
        id: u32,
        name: &str,
        params: Vec<Ty>,
        return_ty: Ty,
        insts: Vec<Inst>,
    ) -> Function {
        Function {
            id: FuncId(id),
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

    #[test]
    fn compile_f32_constant() {
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::F32,
                vec![
                    inst(
                        0,
                        Opcode::ConstF32,
                        Ty::F32,
                        vec![],
                        InstData::ConstF32(1.5f32),
                    ),
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Release, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_f64_constant() {
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::F64,
                vec![
                    inst(
                        0,
                        Opcode::ConstF64,
                        Ty::F64,
                        vec![],
                        InstData::ConstF64(std::f64::consts::E),
                    ),
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Release, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_const_bool_and_unit() {
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::Bool,
                vec![
                    inst(
                        0,
                        Opcode::ConstBool,
                        Ty::Bool,
                        vec![],
                        InstData::ConstBool(false),
                    ),
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_const_unit_opcode() {
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::Unit,
                vec![
                    inst(0, Opcode::ConstUnit, Ty::Unit, vec![], InstData::None),
                    inst(1, Opcode::Return, Ty::Unit, vec![], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_const_str_opcode() {
        let mut module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::Ptr,
                vec![
                    inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            )],
        );
        module.strings.push("hello".to_string());
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_const_str_missing_idx() {
        // ConstStr with idx not in strings → null ptr fallback
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::Ptr,
                vec![
                    inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(99)),
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_unit_type_param() {
        // GetArg where param ty is Unit → no cranelift value, fallback to iconst
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::Unit],
                Ty::Unit,
                vec![
                    inst(0, Opcode::GetArg, Ty::Unit, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::Return, Ty::Unit, vec![], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_wrapping_arithmetic_i64() {
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "arith".to_string(),
                params: vec![Ty::I64, Ty::I64],
                param_names: vec![],
                return_ty: Ty::I64,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                        inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                        inst(
                            2,
                            Opcode::WrappingSubI64,
                            Ty::I64,
                            vec![0, 1],
                            InstData::None,
                        ),
                        inst(
                            3,
                            Opcode::WrappingMulI64,
                            Ty::I64,
                            vec![0, 1],
                            InstData::None,
                        ),
                        inst(
                            4,
                            Opcode::WrappingDivI64,
                            Ty::I64,
                            vec![0, 1],
                            InstData::None,
                        ),
                        inst(
                            5,
                            Opcode::WrappingRemI64,
                            Ty::I64,
                            vec![0, 1],
                            InstData::None,
                        ),
                        inst(6, Opcode::Return, Ty::Unit, vec![5], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_checked_arithmetic_i64() {
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "checked".to_string(),
                params: vec![Ty::I64, Ty::I64],
                param_names: vec![],
                return_ty: Ty::I64,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                        inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                        inst(
                            2,
                            Opcode::CheckedAddI64,
                            Ty::I64,
                            vec![0, 1],
                            InstData::None,
                        ),
                        inst(
                            3,
                            Opcode::CheckedSubI64,
                            Ty::I64,
                            vec![0, 1],
                            InstData::None,
                        ),
                        inst(
                            4,
                            Opcode::CheckedMulI64,
                            Ty::I64,
                            vec![0, 1],
                            InstData::None,
                        ),
                        inst(
                            5,
                            Opcode::CheckedDivI64,
                            Ty::I64,
                            vec![0, 1],
                            InstData::None,
                        ),
                        inst(
                            6,
                            Opcode::CheckedRemI64,
                            Ty::I64,
                            vec![0, 1],
                            InstData::None,
                        ),
                        inst(7, Opcode::Return, Ty::Unit, vec![6], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_integer_comparisons_i64() {
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "cmp".to_string(),
                params: vec![Ty::I64, Ty::I64],
                param_names: vec![],
                return_ty: Ty::Bool,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                        inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                        inst(2, Opcode::EqI64, Ty::Bool, vec![0, 1], InstData::None),
                        inst(3, Opcode::NeI64, Ty::Bool, vec![0, 1], InstData::None),
                        inst(4, Opcode::LtI64, Ty::Bool, vec![0, 1], InstData::None),
                        inst(5, Opcode::LeI64, Ty::Bool, vec![0, 1], InstData::None),
                        inst(6, Opcode::GtI64, Ty::Bool, vec![0, 1], InstData::None),
                        inst(7, Opcode::GeI64, Ty::Bool, vec![0, 1], InstData::None),
                        inst(8, Opcode::Return, Ty::Unit, vec![7], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_float_arithmetic() {
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "farith".to_string(),
                params: vec![Ty::F64, Ty::F64],
                param_names: vec![],
                return_ty: Ty::F64,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::F64, vec![], InstData::ArgIndex(0)),
                        inst(1, Opcode::GetArg, Ty::F64, vec![], InstData::ArgIndex(1)),
                        inst(2, Opcode::AddF64, Ty::F64, vec![0, 1], InstData::None),
                        inst(3, Opcode::SubF64, Ty::F64, vec![0, 1], InstData::None),
                        inst(4, Opcode::MulF64, Ty::F64, vec![0, 1], InstData::None),
                        inst(5, Opcode::DivF64, Ty::F64, vec![0, 1], InstData::None),
                        inst(6, Opcode::Return, Ty::Unit, vec![5], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_float_arithmetic_f32() {
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "farith32".to_string(),
                params: vec![Ty::F32, Ty::F32],
                param_names: vec![],
                return_ty: Ty::F32,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::F32, vec![], InstData::ArgIndex(0)),
                        inst(1, Opcode::GetArg, Ty::F32, vec![], InstData::ArgIndex(1)),
                        inst(2, Opcode::AddF32, Ty::F32, vec![0, 1], InstData::None),
                        inst(3, Opcode::SubF32, Ty::F32, vec![0, 1], InstData::None),
                        inst(4, Opcode::MulF32, Ty::F32, vec![0, 1], InstData::None),
                        inst(5, Opcode::DivF32, Ty::F32, vec![0, 1], InstData::None),
                        inst(6, Opcode::Return, Ty::Unit, vec![5], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_float_rem_returns_error() {
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::F64, Ty::F64],
                Ty::F64,
                vec![
                    inst(0, Opcode::GetArg, Ty::F64, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::GetArg, Ty::F64, vec![], InstData::ArgIndex(1)),
                    inst(2, Opcode::RemF64, Ty::F64, vec![0, 1], InstData::None),
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(matches!(result, Err(CodegenError::UnsupportedOpcode(_))));
    }

    #[test]
    fn compile_float_comparisons() {
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "fcmp".to_string(),
                params: vec![Ty::F64, Ty::F64],
                param_names: vec![],
                return_ty: Ty::Bool,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::F64, vec![], InstData::ArgIndex(0)),
                        inst(1, Opcode::GetArg, Ty::F64, vec![], InstData::ArgIndex(1)),
                        inst(2, Opcode::EqF64, Ty::Bool, vec![0, 1], InstData::None),
                        inst(3, Opcode::NeF64, Ty::Bool, vec![0, 1], InstData::None),
                        inst(4, Opcode::LtF64, Ty::Bool, vec![0, 1], InstData::None),
                        inst(5, Opcode::LeF64, Ty::Bool, vec![0, 1], InstData::None),
                        inst(6, Opcode::GtF64, Ty::Bool, vec![0, 1], InstData::None),
                        inst(7, Opcode::GeF64, Ty::Bool, vec![0, 1], InstData::None),
                        inst(8, Opcode::Return, Ty::Unit, vec![7], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_f32_comparisons() {
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "fcmp32".to_string(),
                params: vec![Ty::F32, Ty::F32],
                param_names: vec![],
                return_ty: Ty::Bool,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::F32, vec![], InstData::ArgIndex(0)),
                        inst(1, Opcode::GetArg, Ty::F32, vec![], InstData::ArgIndex(1)),
                        inst(2, Opcode::EqF32, Ty::Bool, vec![0, 1], InstData::None),
                        inst(3, Opcode::NeF32, Ty::Bool, vec![0, 1], InstData::None),
                        inst(4, Opcode::LtF32, Ty::Bool, vec![0, 1], InstData::None),
                        inst(5, Opcode::LeF32, Ty::Bool, vec![0, 1], InstData::None),
                        inst(6, Opcode::GtF32, Ty::Bool, vec![0, 1], InstData::None),
                        inst(7, Opcode::GeF32, Ty::Bool, vec![0, 1], InstData::None),
                        inst(8, Opcode::Return, Ty::Unit, vec![7], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_boolean_ops() {
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "bools".to_string(),
                params: vec![Ty::Bool, Ty::Bool],
                param_names: vec![],
                return_ty: Ty::Bool,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::Bool, vec![], InstData::ArgIndex(0)),
                        inst(1, Opcode::GetArg, Ty::Bool, vec![], InstData::ArgIndex(1)),
                        inst(2, Opcode::Not, Ty::Bool, vec![0], InstData::None),
                        inst(3, Opcode::And, Ty::Bool, vec![0, 1], InstData::None),
                        inst(4, Opcode::Or, Ty::Bool, vec![0, 1], InstData::None),
                        inst(5, Opcode::Return, Ty::Unit, vec![4], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_integer_bitwise_ops() {
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "bits".to_string(),
                params: vec![Ty::I64, Ty::I64],
                param_names: vec![],
                return_ty: Ty::I64,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                        inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                        inst(2, Opcode::BitAndI64, Ty::I64, vec![0, 1], InstData::None),
                        inst(3, Opcode::BitOrI64, Ty::I64, vec![0, 1], InstData::None),
                        inst(4, Opcode::XorI64, Ty::I64, vec![0, 1], InstData::None),
                        inst(5, Opcode::ShlI64, Ty::I64, vec![0, 1], InstData::None),
                        inst(6, Opcode::ShrI64, Ty::I64, vec![0, 1], InstData::None),
                        inst(7, Opcode::Return, Ty::Unit, vec![6], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_load_store_ops() {
        // fn f(ptr: Ptr) -> i64 { store(ptr, 42); load(ptr) }
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "memops".to_string(),
                params: vec![Ty::Ptr],
                param_names: vec![],
                return_ty: Ty::I64,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                        inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(42)),
                        inst(2, Opcode::Store, Ty::Unit, vec![0, 1], InstData::None),
                        inst(3, Opcode::Load, Ty::I64, vec![0], InstData::None),
                        inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_unreachable_opcode() {
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::Unit,
                vec![inst(
                    0,
                    Opcode::Unreachable,
                    Ty::Unit,
                    vec![],
                    InstData::None,
                )],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_vow_requires_debug_mode() {
        // VowRequires: predicate(cond) → if false, call violation handler
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "f".to_string(),
                params: vec![],
                param_names: vec![],
                return_ty: Ty::Unit,
                effects: vec![],
                vows: vec![VowEntry {
                    id: VowId(0),
                    description: "x > 0".to_string(),
                    blame: vow_diag::Blame::Caller,
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
                        inst(
                            1,
                            Opcode::VowRequires,
                            Ty::Unit,
                            vec![0],
                            InstData::VowId(VowId(0)),
                        ),
                        inst(2, Opcode::Return, Ty::Unit, vec![], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_vow_ensures_debug_mode() {
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "f".to_string(),
                params: vec![],
                param_names: vec![],
                return_ty: Ty::Unit,
                effects: vec![],
                vows: vec![VowEntry {
                    id: VowId(0),
                    description: "result >= 0".to_string(),
                    blame: vow_diag::Blame::Callee,
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
                        inst(
                            1,
                            Opcode::VowEnsures,
                            Ty::Unit,
                            vec![0],
                            InstData::VowId(VowId(0)),
                        ),
                        inst(2, Opcode::Return, Ty::Unit, vec![], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_vow_invariant_debug_mode() {
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "f".to_string(),
                params: vec![],
                param_names: vec![],
                return_ty: Ty::Unit,
                effects: vec![],
                vows: vec![VowEntry {
                    id: VowId(0),
                    description: "i >= 0".to_string(),
                    blame: vow_diag::Blame::Callee,
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
                        inst(
                            1,
                            Opcode::VowInvariant,
                            Ty::Unit,
                            vec![0],
                            InstData::VowId(VowId(0)),
                        ),
                        inst(2, Opcode::Return, Ty::Unit, vec![], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_vow_with_bindings_debug_mode() {
        // VowRequires with a binding so captures loop is hit (n > 0 path)
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "f".to_string(),
                params: vec![Ty::I64],
                param_names: vec![],
                return_ty: Ty::Unit,
                effects: vec![],
                vows: vec![VowEntry {
                    id: VowId(0),
                    description: "x > 0".to_string(),
                    blame: vow_diag::Blame::Caller,
                    bindings: vec![("x".to_string(), InstId(0))],
                    file: String::new(),
                    offset: 0,
                }],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                        inst(
                            1,
                            Opcode::ConstBool,
                            Ty::Bool,
                            vec![],
                            InstData::ConstBool(true),
                        ),
                        inst(
                            2,
                            Opcode::VowRequires,
                            Ty::Unit,
                            vec![1],
                            InstData::VowId(VowId(0)),
                        ),
                        inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_extern_call() {
        // Call __vow_print_i64 via CallExtern
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::I64],
                Ty::Unit,
                vec![
                    inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                    inst(
                        1,
                        Opcode::Call,
                        Ty::Unit,
                        vec![0],
                        InstData::CallExtern("__vow_print_i64".to_string()),
                    ),
                    inst(2, Opcode::Return, Ty::Unit, vec![], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_extern_call_with_return() {
        // Call __vow_vec_new which returns a ptr (covers non-empty results path)
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::Ptr,
                vec![
                    inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                    inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                    inst(
                        2,
                        Opcode::Call,
                        Ty::Ptr,
                        vec![0, 1],
                        InstData::CallExtern("__vow_vec_new".to_string()),
                    ),
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn block_region_vec_new_imports_arena_variant() {
        let mut vec_new = inst(
            2,
            Opcode::Call,
            Ty::Ptr,
            vec![0, 1],
            InstData::CallExtern("__vow_vec_new".to_string()),
        );
        vec_new.region = RegionId::Block(BlockId(0));
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::Ptr,
                vec![
                    inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                    inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                    vec_new,
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());

        let bytes = result.unwrap().bytes;
        use object::{Object, ObjectSymbol};
        let object = object::File::parse(bytes.as_slice()).expect("compiled object should parse");
        let symbols: HashSet<String> = object
            .symbols()
            .filter_map(|symbol| symbol.name().ok().map(str::to_string))
            .collect();
        assert!(symbols.contains("__vow_vec_new_in_arena"));
        assert!(!symbols.contains("__vow_vec_new"));
    }

    #[test]
    fn root_region_vec_new_keeps_wrapper_symbol() {
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::Ptr,
                vec![
                    inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                    inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                    inst(
                        2,
                        Opcode::Call,
                        Ty::Ptr,
                        vec![0, 1],
                        InstData::CallExtern("__vow_vec_new".to_string()),
                    ),
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());

        let bytes = result.unwrap().bytes;
        use object::{Object, ObjectSymbol};
        let object = object::File::parse(bytes.as_slice()).expect("compiled object should parse");
        let symbols: HashSet<String> = object
            .symbols()
            .filter_map(|symbol| symbol.name().ok().map(str::to_string))
            .collect();
        assert!(symbols.contains("__vow_vec_new"));
        assert!(!symbols.contains("__vow_vec_new_in_arena"));
    }

    #[test]
    fn block_region_string_from_cstr_imports_arena_variant() {
        let mut string_new = inst(
            1,
            Opcode::Call,
            Ty::Ptr,
            vec![0],
            InstData::CallExtern("__vow_string_from_cstr".to_string()),
        );
        string_new.region = RegionId::Block(BlockId(0));
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::I64,
                vec![
                    inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                    string_new,
                    inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());

        let bytes = result.unwrap().bytes;
        use object::{Object, ObjectSymbol};
        let object = object::File::parse(bytes.as_slice()).expect("compiled object should parse");
        let symbols: HashSet<String> = object
            .symbols()
            .filter_map(|symbol| symbol.name().ok().map(str::to_string))
            .collect();
        assert!(symbols.contains("__vow_string_from_cstr_in_arena"));
        assert!(!symbols.contains("__vow_string_from_cstr"));
    }

    #[test]
    fn block_region_fresh_string_helpers_import_arena_variants() {
        let cases = [
            ("__vow_string_split", "__vow_string_split_in_arena", 2),
            ("__vow_string_trim", "__vow_string_trim_in_arena", 1),
            ("__vow_string_to_upper", "__vow_string_to_upper_in_arena", 1),
            ("__vow_string_to_lower", "__vow_string_to_lower_in_arena", 1),
            ("__vow_string_replace", "__vow_string_replace_in_arena", 3),
            ("__vow_string_join", "__vow_string_join_in_arena", 2),
        ];

        let mut insts = vec![
            inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
            inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
            inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
        ];
        for (idx, (sym, _, arity)) in cases.iter().enumerate() {
            let mut call = inst(
                10 + idx as u32,
                Opcode::Call,
                Ty::Ptr,
                (0..*arity).collect(),
                InstData::CallExtern((*sym).to_string()),
            );
            call.region = RegionId::Block(BlockId(0));
            insts.push(call);
        }
        insts.push(inst(90, Opcode::Return, Ty::Unit, vec![], InstData::None));

        let module = make_module("test", vec![simple_fn(0, "f", vec![], Ty::Unit, insts)]);
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());

        let bytes = result.unwrap().bytes;
        use object::{Object, ObjectSymbol};
        let object = object::File::parse(bytes.as_slice()).expect("compiled object should parse");
        let symbols: HashSet<String> = object
            .symbols()
            .filter_map(|symbol| symbol.name().ok().map(str::to_string))
            .collect();

        for (root, routed, _) in cases {
            assert!(symbols.contains(routed), "{routed} should be imported");
            assert!(!symbols.contains(root), "{root} should not be imported");
        }
    }

    #[test]
    fn root_region_string_from_cstr_keeps_wrapper_symbol() {
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::I64,
                vec![
                    inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                    inst(
                        1,
                        Opcode::Call,
                        Ty::Ptr,
                        vec![0],
                        InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    ),
                    inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());

        let bytes = result.unwrap().bytes;
        use object::{Object, ObjectSymbol};
        let object = object::File::parse(bytes.as_slice()).expect("compiled object should parse");
        let symbols: HashSet<String> = object
            .symbols()
            .filter_map(|symbol| symbol.name().ok().map(str::to_string))
            .collect();
        assert!(symbols.contains("__vow_string_from_cstr"));
        assert!(!symbols.contains("__vow_string_from_cstr_in_arena"));
    }

    #[test]
    fn block_region_string_push_str_imports_arena_variant() {
        let mut dest = inst(
            1,
            Opcode::Call,
            Ty::Ptr,
            vec![0],
            InstData::CallExtern("__vow_string_from_cstr".to_string()),
        );
        dest.region = RegionId::Block(BlockId(0));
        let mut src = inst(
            3,
            Opcode::Call,
            Ty::Ptr,
            vec![2],
            InstData::CallExtern("__vow_string_from_cstr".to_string()),
        );
        src.region = RegionId::Block(BlockId(0));
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::I64,
                vec![
                    inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                    dest,
                    inst(2, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(1)),
                    src,
                    inst(
                        4,
                        Opcode::Call,
                        Ty::Unit,
                        vec![1, 3],
                        InstData::CallExtern("__vow_string_push_str".to_string()),
                    ),
                    inst(5, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    inst(6, Opcode::Return, Ty::Unit, vec![5], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());

        let bytes = result.unwrap().bytes;
        use object::{Object, ObjectSymbol};
        let object = object::File::parse(bytes.as_slice()).expect("compiled object should parse");
        let symbols: HashSet<String> = object
            .symbols()
            .filter_map(|symbol| symbol.name().ok().map(str::to_string))
            .collect();
        assert!(symbols.contains("__vow_string_push_str_in_arena"));
        assert!(!symbols.contains("__vow_string_push_str"));
    }

    #[test]
    fn parameter_region_string_push_byte_imports_arena_variant() {
        let grow_param = Function {
            id: FuncId(0),
            name: "grow_param".to_string(),
            params: vec![Ty::Ptr],
            param_names: vec!["s".to_string()],
            return_ty: Ty::Unit,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(
                        1,
                        Opcode::ConstI64,
                        Ty::I64,
                        vec![],
                        InstData::ConstI64(120),
                    ),
                    inst(
                        2,
                        Opcode::Call,
                        Ty::Unit,
                        vec![0, 1],
                        InstData::CallExtern("__vow_string_push_byte".to_string()),
                    ),
                    inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary {
                param_regions: vec![],
                return_region: RegionConstraint::ConstantGlobal,
                store_effects: vec![StoreEffect {
                    target: 0,
                    source: RegionConstraint::ConstantGlobal,
                }],
            },
            source_file: String::new(),
        };
        let module = make_module("test", vec![grow_param]);
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());

        let bytes = result.unwrap().bytes;
        use object::{Object, ObjectSymbol};
        let object = object::File::parse(bytes.as_slice()).expect("compiled object should parse");
        let symbols: HashSet<String> = object
            .symbols()
            .filter_map(|symbol| symbol.name().ok().map(str::to_string))
            .collect();
        assert!(symbols.contains("__vow_string_push_byte_in_arena"));
        assert!(!symbols.contains("__vow_string_push_byte"));
    }

    #[test]
    fn projected_parameter_region_string_push_byte_imports_arena_variant() {
        let grow_projection = Function {
            id: FuncId(0),
            name: "grow_projection".to_string(),
            params: vec![Ty::Ptr, Ty::Ptr],
            param_names: vec!["strings".to_string(), "holder".to_string()],
            return_ty: Ty::Unit,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    inst(
                        2,
                        Opcode::Call,
                        Ty::Ptr,
                        vec![0, 1],
                        InstData::CallExtern("__vow_vec_get_val".to_string()),
                    ),
                    inst(3, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
                    inst(
                        4,
                        Opcode::FieldGet,
                        Ty::Ptr,
                        vec![3],
                        InstData::FieldIndex(0),
                    ),
                    inst(
                        5,
                        Opcode::ConstI64,
                        Ty::I64,
                        vec![],
                        InstData::ConstI64(120),
                    ),
                    inst(
                        6,
                        Opcode::Call,
                        Ty::Unit,
                        vec![2, 5],
                        InstData::CallExtern("__vow_string_push_byte".to_string()),
                    ),
                    inst(
                        7,
                        Opcode::ConstI64,
                        Ty::I64,
                        vec![],
                        InstData::ConstI64(121),
                    ),
                    inst(
                        8,
                        Opcode::Call,
                        Ty::Unit,
                        vec![4, 7],
                        InstData::CallExtern("__vow_string_push_byte".to_string()),
                    ),
                    inst(9, Opcode::Return, Ty::Unit, vec![], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary {
                param_regions: vec![],
                return_region: RegionConstraint::ConstantGlobal,
                store_effects: vec![
                    StoreEffect {
                        target: 0,
                        source: RegionConstraint::ConstantGlobal,
                    },
                    StoreEffect {
                        target: 1,
                        source: RegionConstraint::ConstantGlobal,
                    },
                ],
            },
            source_file: String::new(),
        };
        let module = make_module("test", vec![grow_projection]);
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());

        let bytes = result.unwrap().bytes;
        use object::{Object, ObjectSymbol};
        let object = object::File::parse(bytes.as_slice()).expect("compiled object should parse");
        let symbols: HashSet<String> = object
            .symbols()
            .filter_map(|symbol| symbol.name().ok().map(str::to_string))
            .collect();
        assert!(symbols.contains("__vow_string_push_byte_in_arena"));
        assert!(!symbols.contains("__vow_string_push_byte"));
    }

    #[test]
    fn phi_parameter_region_string_push_byte_imports_arena_variant() {
        let phi_id = InstId(8);
        let grow_phi = Function {
            id: FuncId(0),
            name: "grow_phi".to_string(),
            params: vec![Ty::Bool, Ty::Ptr],
            param_names: vec!["cond".to_string(), "s".to_string()],
            return_ty: Ty::Unit,
            effects: vec![],
            vows: vec![],
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::Bool, vec![], InstData::ArgIndex(0)),
                        inst(1, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
                        inst(
                            2,
                            Opcode::Branch,
                            Ty::Unit,
                            vec![0],
                            InstData::BranchTargets {
                                then_block: BlockId(1),
                                else_block: BlockId(2),
                            },
                        ),
                    ],
                },
                BasicBlock {
                    id: BlockId(1),
                    insts: vec![
                        inst(
                            3,
                            Opcode::Upsilon,
                            Ty::Unit,
                            vec![1],
                            InstData::PhiTarget(phi_id),
                        ),
                        inst(
                            4,
                            Opcode::Jump,
                            Ty::Unit,
                            vec![],
                            InstData::JumpTarget(BlockId(3)),
                        ),
                    ],
                },
                BasicBlock {
                    id: BlockId(2),
                    insts: vec![
                        inst(
                            5,
                            Opcode::Upsilon,
                            Ty::Unit,
                            vec![1],
                            InstData::PhiTarget(phi_id),
                        ),
                        inst(
                            6,
                            Opcode::Jump,
                            Ty::Unit,
                            vec![],
                            InstData::JumpTarget(BlockId(3)),
                        ),
                    ],
                },
                BasicBlock {
                    id: BlockId(3),
                    insts: vec![
                        Inst {
                            id: phi_id,
                            opcode: Opcode::Phi,
                            ty: Ty::Ptr,
                            args: vec![],
                            data: InstData::None,
                            origin: sp(),
                            region: RegionId::Root,
                        },
                        inst(
                            9,
                            Opcode::ConstI64,
                            Ty::I64,
                            vec![],
                            InstData::ConstI64(120),
                        ),
                        inst(
                            10,
                            Opcode::Call,
                            Ty::Unit,
                            vec![8, 9],
                            InstData::CallExtern("__vow_string_push_byte".to_string()),
                        ),
                        inst(11, Opcode::Return, Ty::Unit, vec![], InstData::None),
                    ],
                },
            ],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary {
                param_regions: vec![],
                return_region: RegionConstraint::ConstantGlobal,
                store_effects: vec![StoreEffect {
                    target: 1,
                    source: RegionConstraint::ConstantGlobal,
                }],
            },
            source_file: String::new(),
        };
        let module = make_module("test", vec![grow_phi]);
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());

        let bytes = result.unwrap().bytes;
        use object::{Object, ObjectSymbol};
        let object = object::File::parse(bytes.as_slice()).expect("compiled object should parse");
        let symbols: HashSet<String> = object
            .symbols()
            .filter_map(|symbol| symbol.name().ok().map(str::to_string))
            .collect();
        assert!(symbols.contains("__vow_string_push_byte_in_arena"));
        assert!(!symbols.contains("__vow_string_push_byte"));
    }

    #[test]
    fn block_region_vec_push_val_imports_arena_variant() {
        let mut vec_new = inst(
            2,
            Opcode::Call,
            Ty::Ptr,
            vec![0, 1],
            InstData::CallExtern("__vow_vec_new".to_string()),
        );
        vec_new.region = RegionId::Block(BlockId(0));
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::Unit,
                vec![
                    inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                    inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                    vec_new,
                    inst(3, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(42)),
                    inst(
                        4,
                        Opcode::Call,
                        Ty::Unit,
                        vec![2, 3],
                        InstData::CallExtern("__vow_vec_push_val".to_string()),
                    ),
                    inst(5, Opcode::Return, Ty::Unit, vec![], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());

        let bytes = result.unwrap().bytes;
        use object::{Object, ObjectSymbol};
        let object = object::File::parse(bytes.as_slice()).expect("compiled object should parse");
        let symbols: HashSet<String> = object
            .symbols()
            .filter_map(|symbol| symbol.name().ok().map(str::to_string))
            .collect();
        assert!(symbols.contains("__vow_vec_push_val_in_arena"));
        assert!(!symbols.contains("__vow_vec_push_val"));
    }

    #[test]
    fn compile_region_ops() {
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::Unit,
                vec![
                    inst(
                        0,
                        Opcode::RegionAlloc,
                        Ty::Ptr,
                        vec![],
                        InstData::AllocSize { size: 64, align: 8 },
                    ),
                    inst(1, Opcode::Return, Ty::Unit, vec![], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
        let bytes = result.unwrap().bytes;
        use object::{Object, ObjectSymbol};
        let object = object::File::parse(bytes.as_slice()).expect("compiled object should parse");
        let symbols: HashSet<String> = object
            .symbols()
            .filter_map(|symbol| symbol.name().ok().map(str::to_string))
            .collect();
        assert!(
            symbols.contains("__vow_arena_alloc"),
            "RegionAlloc must lower through __vow_arena_alloc"
        );
        assert!(
            !symbols.contains("__vow_malloc"),
            "RegionAlloc must not import the legacy __vow_malloc shim"
        );
    }

    #[test]
    fn compile_block_arena_open_alloc_close() {
        // Phase 4 / S2: a function with a single block that opens its own
        // block-region arena, allocates from it, then closes it.
        //
        //   block0:
        //     v0 = RegionOpen(block0)
        //     v1 = RegionAlloc { region: Block(block0), size: 64, align: 8 }
        //     v2 = RegionClose(block0)
        //     return
        //
        // The codegen MUST:
        //   - reserve a stack slot for the block's VowArena header,
        //   - lower RegionOpen  → __vow_arena_open(&slot),
        //   - lower RegionAlloc → __vow_arena_alloc(&slot, size, align),
        //   - lower RegionClose → __vow_arena_close(&slot),
        //   - import all three runtime symbols.
        let mut open = inst(0, Opcode::RegionOpen, Ty::Unit, vec![], InstData::None);
        open.region = RegionId::Block(BlockId(0));
        let mut alloc = inst(
            1,
            Opcode::RegionAlloc,
            Ty::Ptr,
            vec![],
            InstData::AllocSize { size: 64, align: 8 },
        );
        alloc.region = RegionId::Block(BlockId(0));
        let mut close = inst(2, Opcode::RegionClose, Ty::Unit, vec![], InstData::None);
        close.region = RegionId::Block(BlockId(0));
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::Unit,
                vec![
                    open,
                    alloc,
                    close,
                    inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
        let bytes = result.unwrap().bytes;
        use object::{Object, ObjectSymbol};
        let object = object::File::parse(bytes.as_slice()).expect("compiled object should parse");
        let symbols: HashSet<String> = object
            .symbols()
            .filter_map(|symbol| symbol.name().ok().map(str::to_string))
            .collect();
        assert!(
            symbols.contains("__vow_arena_alloc"),
            "RegionAlloc{{Block}} must import __vow_arena_alloc"
        );
        assert!(
            symbols.contains("__vow_arena_open"),
            "RegionOpen must import __vow_arena_open"
        );
        assert!(
            symbols.contains("__vow_arena_close"),
            "RegionClose must import __vow_arena_close"
        );
    }

    #[test]
    fn compile_linear_ops() {
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::LinearPtr],
                Ty::Unit,
                vec![
                    inst(
                        0,
                        Opcode::GetArg,
                        Ty::LinearPtr,
                        vec![],
                        InstData::ArgIndex(0),
                    ),
                    inst(1, Opcode::LinearConsume, Ty::Unit, vec![0], InstData::None),
                    inst(2, Opcode::LinearBorrow, Ty::Unit, vec![], InstData::None),
                    inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_field_get_set_i64() {
        // Struct with one i64 field at index 0
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::Ptr],
                Ty::I64,
                vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(99)),
                    inst(
                        2,
                        Opcode::FieldSet,
                        Ty::Unit,
                        vec![0, 1],
                        InstData::FieldIndex(0),
                    ),
                    inst(
                        3,
                        Opcode::FieldGet,
                        Ty::I64,
                        vec![0],
                        InstData::FieldIndex(0),
                    ),
                    inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_field_get_set_bool() {
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::Ptr],
                Ty::Bool,
                vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(
                        1,
                        Opcode::ConstBool,
                        Ty::Bool,
                        vec![],
                        InstData::ConstBool(true),
                    ),
                    inst(
                        2,
                        Opcode::FieldSet,
                        Ty::Unit,
                        vec![0, 1],
                        InstData::FieldIndex(0),
                    ),
                    inst(
                        3,
                        Opcode::FieldGet,
                        Ty::Bool,
                        vec![0],
                        InstData::FieldIndex(0),
                    ),
                    inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_field_get_set_i32() {
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::Ptr],
                Ty::I32,
                vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::ConstI32, Ty::I32, vec![], InstData::ConstI32(7)),
                    inst(
                        2,
                        Opcode::FieldSet,
                        Ty::Unit,
                        vec![0, 1],
                        InstData::FieldIndex(1),
                    ),
                    inst(
                        3,
                        Opcode::FieldGet,
                        Ty::I32,
                        vec![0],
                        InstData::FieldIndex(1),
                    ),
                    inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_field_get_set_f32() {
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::Ptr],
                Ty::F32,
                vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(
                        1,
                        Opcode::ConstF32,
                        Ty::F32,
                        vec![],
                        InstData::ConstF32(1.5f32),
                    ),
                    inst(
                        2,
                        Opcode::FieldSet,
                        Ty::Unit,
                        vec![0, 1],
                        InstData::FieldIndex(0),
                    ),
                    inst(
                        3,
                        Opcode::FieldGet,
                        Ty::F32,
                        vec![0],
                        InstData::FieldIndex(0),
                    ),
                    inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_field_get_set_f64() {
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::Ptr],
                Ty::F64,
                vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(
                        1,
                        Opcode::ConstF64,
                        Ty::F64,
                        vec![],
                        InstData::ConstF64(std::f64::consts::E),
                    ),
                    inst(
                        2,
                        Opcode::FieldSet,
                        Ty::Unit,
                        vec![0, 1],
                        InstData::FieldIndex(0),
                    ),
                    inst(
                        3,
                        Opcode::FieldGet,
                        Ty::F64,
                        vec![0],
                        InstData::FieldIndex(0),
                    ),
                    inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_release_mode() {
        // Release mode: no violation/overflow handler declarations
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
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
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Release, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_string_new_extern() {
        // __vow_string_new(ptr: I64, len: I64) -> Ptr
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::Ptr, Ty::I64],
                Ty::Ptr,
                vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                    inst(
                        2,
                        Opcode::Call,
                        Ty::Ptr,
                        vec![0, 1],
                        InstData::CallExtern("__vow_string_new".to_string()),
                    ),
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_map_new_and_fs_read_extern() {
        // __vow_map_new() -> Ptr
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::Ptr,
                vec![
                    inst(
                        0,
                        Opcode::Call,
                        Ty::Ptr,
                        vec![],
                        InstData::CallExtern("__vow_map_new".to_string()),
                    ),
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            )],
        );
        let r = CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(r.is_ok(), "{:?}", r.err());

        // __vow_fs_read(path: Ptr) -> Ptr
        let module2 = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::Ptr],
                Ty::Ptr,
                vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(
                        1,
                        Opcode::Call,
                        Ty::Ptr,
                        vec![0],
                        InstData::CallExtern("__vow_fs_read".to_string()),
                    ),
                    inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
                ],
            )],
        );
        let r2 = CraneliftBackend::new().compile_module(&module2, BuildMode::Debug, TraceMode::Off);
        assert!(r2.is_ok(), "{:?}", r2.err());
    }

    #[test]
    fn compile_extern_single_param_no_return() {
        // __vow_eprintln_str(ptr: Ptr) -> Unit
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::Ptr],
                Ty::Unit,
                vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(
                        1,
                        Opcode::Call,
                        Ty::Unit,
                        vec![0],
                        InstData::CallExtern("__vow_eprintln_str".to_string()),
                    ),
                    inst(2, Opcode::Return, Ty::Unit, vec![], InstData::None),
                ],
            )],
        );
        let r = CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(r.is_ok(), "{:?}", r.err());
    }

    #[test]
    fn compile_map_remove_two_params_no_return() {
        // __vow_map_remove(map: Ptr, key: I64) -> Unit
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::Ptr, Ty::I64],
                Ty::Unit,
                vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                    inst(
                        2,
                        Opcode::Call,
                        Ty::Unit,
                        vec![0, 1],
                        InstData::CallExtern("__vow_map_remove".to_string()),
                    ),
                    inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
                ],
            )],
        );
        let r = CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(r.is_ok(), "{:?}", r.err());
    }

    #[test]
    fn compile_string_len_and_eq_externs() {
        // __vow_string_len(str: Ptr) -> I64
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::Ptr],
                Ty::I64,
                vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(
                        1,
                        Opcode::Call,
                        Ty::I64,
                        vec![0],
                        InstData::CallExtern("__vow_string_len".to_string()),
                    ),
                    inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
                ],
            )],
        );
        let r = CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(r.is_ok(), "{:?}", r.err());

        // __vow_string_eq(a: Ptr, b: Ptr) -> Bool (I8)
        let module2 = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::Ptr, Ty::Ptr],
                Ty::Bool,
                vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
                    inst(
                        2,
                        Opcode::Call,
                        Ty::Bool,
                        vec![0, 1],
                        InstData::CallExtern("__vow_string_eq".to_string()),
                    ),
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            )],
        );
        let r2 = CraneliftBackend::new().compile_module(&module2, BuildMode::Debug, TraceMode::Off);
        assert!(r2.is_ok(), "{:?}", r2.err());
    }

    #[test]
    fn compile_map_get_contains_externs() {
        // __vow_map_get(map: Ptr, key: I64) -> I64
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::Ptr, Ty::I64],
                Ty::I64,
                vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                    inst(
                        2,
                        Opcode::Call,
                        Ty::I64,
                        vec![0, 1],
                        InstData::CallExtern("__vow_map_get".to_string()),
                    ),
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            )],
        );
        let r = CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(r.is_ok(), "{:?}", r.err());

        // __vow_map_contains(map: Ptr, key: I64) -> Bool
        let module2 = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![Ty::Ptr, Ty::I64],
                Ty::Bool,
                vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                    inst(
                        2,
                        Opcode::Call,
                        Ty::Bool,
                        vec![0, 1],
                        InstData::CallExtern("__vow_map_contains".to_string()),
                    ),
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            )],
        );
        let r2 = CraneliftBackend::new().compile_module(&module2, BuildMode::Debug, TraceMode::Off);
        assert!(r2.is_ok(), "{:?}", r2.err());
    }

    #[test]
    fn compile_two_functions_with_call() {
        // Two functions: callee + caller that calls callee (covers Call with CallTarget)
        let module = make_module(
            "test",
            vec![
                Function {
                    id: FuncId(0),
                    name: "callee".to_string(),
                    params: vec![Ty::I64],
                    param_names: vec![],
                    return_ty: Ty::I64,
                    effects: vec![],
                    vows: vec![],
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        insts: vec![
                            inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                            inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                        ],
                    }],
                    local_names: std::collections::HashMap::new(),
                    summary: RegionSummary::default(),
                    source_file: String::new(),
                },
                Function {
                    id: FuncId(1),
                    name: "caller".to_string(),
                    params: vec![Ty::I64],
                    param_names: vec![],
                    return_ty: Ty::I64,
                    effects: vec![],
                    vows: vec![],
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        insts: vec![
                            inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                            inst(
                                1,
                                Opcode::Call,
                                Ty::I64,
                                vec![0],
                                InstData::CallTarget(FuncId(0)),
                            ),
                            inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
                        ],
                    }],
                    local_names: std::collections::HashMap::new(),
                    summary: RegionSummary::default(),
                    source_file: String::new(),
                },
            ],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    /// Build a 4-block FreshInCaller function whose return is a Phi
    /// merging two arm sources. Each arm is a `Vec<Inst>` (so multi-step
    /// arms like `ConstStr → Call __vow_string_from_cstr(cstr)` can be
    /// modelled); the source value fed into the Upsilon is the LAST
    /// inst's id.
    fn fresh_in_caller_phi_function(arm_a: Vec<Inst>, arm_b: Vec<Inst>) -> Function {
        let arm_a_src = arm_a.last().unwrap().id.0;
        let arm_b_src = arm_b.last().unwrap().id.0;
        let mut block1 = arm_a;
        block1.extend([
            inst(
                100,
                Opcode::Upsilon,
                Ty::Unit,
                vec![arm_a_src],
                InstData::PhiTarget(InstId(3)),
            ),
            inst(
                101,
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(BlockId(3)),
            ),
        ]);
        let mut block2 = arm_b;
        block2.extend([
            inst(
                102,
                Opcode::Upsilon,
                Ty::Unit,
                vec![arm_b_src],
                InstData::PhiTarget(InstId(3)),
            ),
            inst(
                103,
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(BlockId(3)),
            ),
        ]);
        Function {
            id: FuncId(0),
            name: "phi_test".to_string(),
            params: vec![Ty::Bool],
            param_names: vec![],
            return_ty: Ty::Ptr,
            effects: vec![],
            vows: vec![],
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::Bool, vec![], InstData::ArgIndex(0)),
                        inst(
                            10,
                            Opcode::Branch,
                            Ty::Unit,
                            vec![0],
                            InstData::BranchTargets {
                                then_block: BlockId(1),
                                else_block: BlockId(2),
                            },
                        ),
                    ],
                },
                BasicBlock {
                    id: BlockId(1),
                    insts: block1,
                },
                BasicBlock {
                    id: BlockId(2),
                    insts: block2,
                },
                BasicBlock {
                    id: BlockId(3),
                    insts: vec![
                        inst(3, Opcode::Phi, Ty::Ptr, vec![], InstData::None),
                        inst(13, Opcode::Return, Ty::Unit, vec![3], InstData::None),
                    ],
                },
            ],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary {
                param_regions: vec![],
                return_region: RegionConstraint::FreshInCaller,
                store_effects: vec![],
            },
            source_file: String::new(),
        }
    }

    /// Build a `__vow_string_from_cstr(ConstStr)` arm — the actual shape
    /// the Vow lowerer produces for a String literal (see
    /// `vow-ir/src/lower/mod.rs` `Lit::String`). The Call's result is a
    /// `VowVec` descriptor and is the only kind of leaf the
    /// materialisation analysis treats as clone-safe.
    fn string_literal_arm(base_id: u32) -> Vec<Inst> {
        let cstr = inst(
            base_id,
            Opcode::ConstStr,
            Ty::Ptr,
            vec![],
            InstData::ConstStr(0),
        );
        let call = inst(
            base_id + 1,
            Opcode::Call,
            Ty::Ptr,
            vec![base_id],
            InstData::CallExtern("__vow_string_from_cstr".to_string()),
        );
        vec![cstr, call]
    }

    fn module_imports_string_clone(bytes: &[u8]) -> bool {
        use object::{Object, ObjectSymbol};
        let object = object::File::parse(bytes).expect("compiled object should parse");
        object
            .symbols()
            .filter_map(|s| s.name().ok().map(str::to_string))
            .any(|n| n == "__vow_string_clone_into_arena")
    }

    /// Acceptance test 2 from issue #198 (safe variant): a Phi whose
    /// arms are both `__vow_string_from_cstr(literal)` — the actual
    /// lowered shape of a Vow String literal — produces `VowVec`
    /// descriptors on every path. Materialising via
    /// `__vow_string_clone_into_arena` is sound; the clone runtime is
    /// imported.
    #[test]
    fn fresh_in_caller_phi_all_string_literal_arms_materialises() {
        let func = fresh_in_caller_phi_function(string_literal_arm(20), string_literal_arm(40));
        let mut module = make_module("test", vec![func]);
        module.strings.push("hello".to_string());
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
        assert!(
            module_imports_string_clone(&result.unwrap().bytes),
            "Phi with all `__vow_string_from_cstr` arms must materialise"
        );
    }

    /// Defense-in-depth (codex P1 from PR #230 round-2 review): a Phi
    /// mixing a string-literal arm with a `RegionAlloc` arm has at
    /// least one path producing a non-`VowString` descriptor. The clone
    /// runtime is type-specialised for `VowString` / `Vec<u8>`, so
    /// firing it on the `RegionAlloc` arm at runtime would reinterpret
    /// the struct's first 24 bytes as `{ptr, len, cap}` and copy
    /// arbitrary bytes — corrupting memory. Materialisation MUST NOT
    /// fire; the clone primitive MUST NOT be imported.
    ///
    /// (In well-typed Vow, the type checker forbids mixed-kind Phi —
    /// but this analysis stays sound under any IR a future pass might
    /// synthesise. A generic deep-clone intrinsic in Phase 7 / #202
    /// lifts this restriction.)
    #[test]
    fn fresh_in_caller_phi_mixed_arms_skips_materialisation() {
        let mut arm_b = inst(
            40,
            Opcode::RegionAlloc,
            Ty::Ptr,
            vec![],
            InstData::AllocSize { size: 16, align: 8 },
        );
        arm_b.region = RegionId::Caller(HiddenRegionIdx(0));
        let func = fresh_in_caller_phi_function(string_literal_arm(20), vec![arm_b]);
        let mut module = make_module("test", vec![func]);
        module.strings.push("hello".to_string());
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
        assert!(
            !module_imports_string_clone(&result.unwrap().bytes),
            "Phi mixing string literal with non-string-layout arms must \
             NOT trigger materialisation — the clone primitive only \
             handles VowString descriptors and would corrupt memory on \
             the non-string arm"
        );
    }

    /// Defense-in-depth (codex P1 from PR #230 round-3 review): a Phi
    /// arm that is a *raw* `ConstStr` (not wrapped in
    /// `__vow_string_from_cstr`) is a c-string pointer, not a `VowVec`
    /// descriptor. Treating it as clone-safe would corrupt memory.
    /// The well-formed Vow lowerer never produces this shape — every
    /// String literal is wrapped in the `from_cstr` Call — but the
    /// analysis must reject it under any IR a future pass might
    /// synthesise.
    #[test]
    fn fresh_in_caller_raw_const_str_arm_skips_materialisation() {
        let cstr_arm = vec![inst(
            20,
            Opcode::ConstStr,
            Ty::Ptr,
            vec![],
            InstData::ConstStr(0),
        )];
        let func = fresh_in_caller_phi_function(cstr_arm, string_literal_arm(40));
        let mut module = make_module("test", vec![func]);
        module.strings.push("hello".to_string());
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
        assert!(
            !module_imports_string_clone(&result.unwrap().bytes),
            "Raw ConstStr arm (c-string, not VowVec descriptor) must NOT \
             trigger materialisation"
        );
    }

    #[test]
    fn fresh_in_caller_string_literal_return_materialises_via_clone() {
        // Phase 4 / S5: a `FreshInCaller` function whose return path is the
        // lowered shape of a Vow String literal —
        // `Call __vow_string_from_cstr(ConstStr)` — produces a `VowVec`
        // descriptor that must be cloned into `target_region` to satisfy
        // §5.1. The produced object must import
        // `__vow_string_clone_into_arena`.
        let mut module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "literal_returner".to_string(),
                params: vec![],
                param_names: vec![],
                return_ty: Ty::Ptr,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                        inst(
                            1,
                            Opcode::Call,
                            Ty::Ptr,
                            vec![0],
                            InstData::CallExtern("__vow_string_from_cstr".to_string()),
                        ),
                        inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary {
                    param_regions: vec![],
                    return_region: RegionConstraint::FreshInCaller,
                    store_effects: vec![],
                },
                source_file: String::new(),
            }],
        );
        module.strings.push("hello".to_string());
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
        assert!(
            module_imports_string_clone(&result.unwrap().bytes),
            "FreshInCaller String-literal return path must import __vow_string_clone_into_arena"
        );
    }

    #[test]
    fn fresh_in_caller_region_alloc_return_skips_materialisation() {
        // Inverse of the above: when the return source is a `RegionAlloc`
        // (placed in `Caller(0)` by the region pass), no clone is needed —
        // the value is already in `target_region`.
        let mut alloc = inst(
            0,
            Opcode::RegionAlloc,
            Ty::Ptr,
            vec![],
            InstData::AllocSize { size: 24, align: 8 },
        );
        alloc.region = RegionId::Caller(HiddenRegionIdx(0));
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "alloc_returner".to_string(),
                params: vec![],
                param_names: vec![],
                return_ty: Ty::Ptr,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        alloc,
                        inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary {
                    param_regions: vec![],
                    return_region: RegionConstraint::FreshInCaller,
                    store_effects: vec![],
                },
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
        let bytes = result.unwrap().bytes;
        use object::{Object, ObjectSymbol};
        let object = object::File::parse(bytes.as_slice()).expect("compiled object should parse");
        let symbols: HashSet<String> = object
            .symbols()
            .filter_map(|symbol| symbol.name().ok().map(str::to_string))
            .collect();
        assert!(
            !symbols.contains("__vow_string_clone_into_arena"),
            "FreshInCaller RegionAlloc(Caller) return path must not emit a clone"
        );
    }

    #[test]
    fn compile_call_threads_block_arena_for_fresh_in_caller_callee() {
        // Phase 4 / S4: when caller has a Block(0) region and calls a
        // FreshInCaller callee whose result lives in Block(0), the call
        // site must project the callee's hidden `target_region` from the
        // caller's frame — i.e. pass the caller's block-arena stack slot,
        // not pad the call with `__vow_root_arena`.
        //
        // This test exercises the projection path end-to-end: compilation
        // must succeed and the produced object must import all three
        // arena runtime symbols (open/close/alloc), confirming the call
        // site reaches the projection helper rather than the legacy
        // root-arena padding (which would never declare arena_open / _close
        // for a caller that opens its own block region).
        //
        // PIPELINE NOTE: the test hand-sets `caller_call.region =
        // Block(BlockId(0))`. The IR lowerer in Phase 4 does NOT tag
        // `Call` insts with non-Root regions — the region pass only
        // touches `RegionAlloc` (`vow-ir/src/types.rs` `Inst.region`
        // doc, `vow-ir/src/region.rs::is_heap_producing`). Phase 9
        // (#204) wires the lowerer to tag `Call` insts whose result
        // lives in a block arena; this test pre-validates that the
        // codegen projection consumes that tag correctly when it
        // arrives.
        let mut callee_alloc = inst(
            0,
            Opcode::RegionAlloc,
            Ty::Ptr,
            vec![],
            InstData::AllocSize { size: 16, align: 8 },
        );
        callee_alloc.region = RegionId::Caller(HiddenRegionIdx(0));

        let callee = Function {
            id: FuncId(0),
            name: "callee".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::Ptr,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    callee_alloc,
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary {
                param_regions: vec![],
                return_region: RegionConstraint::FreshInCaller,
                store_effects: vec![],
            },
            source_file: String::new(),
        };

        let mut caller_open = inst(0, Opcode::RegionOpen, Ty::Unit, vec![], InstData::None);
        caller_open.region = RegionId::Block(BlockId(0));
        let mut caller_call = inst(
            1,
            Opcode::Call,
            Ty::Ptr,
            vec![],
            InstData::CallTarget(FuncId(0)),
        );
        caller_call.region = RegionId::Block(BlockId(0));
        let mut caller_close = inst(2, Opcode::RegionClose, Ty::Unit, vec![], InstData::None);
        caller_close.region = RegionId::Block(BlockId(0));

        let caller = Function {
            id: FuncId(1),
            name: "caller".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::Unit,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    caller_open,
                    caller_call,
                    caller_close,
                    inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };

        let module = make_module("test", vec![callee, caller]);
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
        let bytes = result.unwrap().bytes;
        use object::{Object, ObjectSymbol};
        let object = object::File::parse(bytes.as_slice()).expect("compiled object should parse");
        let symbols: HashSet<String> = object
            .symbols()
            .filter_map(|symbol| symbol.name().ok().map(str::to_string))
            .collect();
        assert!(symbols.contains("__vow_arena_alloc"));
        assert!(symbols.contains("__vow_arena_open"));
        assert!(symbols.contains("__vow_arena_close"));
    }

    #[test]
    fn compile_const_i32_opcode() {
        let module = make_module(
            "test",
            vec![simple_fn(
                0,
                "f",
                vec![],
                Ty::I32,
                vec![
                    inst(0, Opcode::ConstI32, Ty::I32, vec![], InstData::ConstI32(42)),
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            )],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_wrapping_i32_arithmetic() {
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "i32arith".to_string(),
                params: vec![Ty::I32, Ty::I32],
                param_names: vec![],
                return_ty: Ty::I32,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::I32, vec![], InstData::ArgIndex(0)),
                        inst(1, Opcode::GetArg, Ty::I32, vec![], InstData::ArgIndex(1)),
                        inst(
                            2,
                            Opcode::WrappingAddI32,
                            Ty::I32,
                            vec![0, 1],
                            InstData::None,
                        ),
                        inst(
                            3,
                            Opcode::WrappingSubI32,
                            Ty::I32,
                            vec![0, 1],
                            InstData::None,
                        ),
                        inst(
                            4,
                            Opcode::WrappingMulI32,
                            Ty::I32,
                            vec![0, 1],
                            InstData::None,
                        ),
                        inst(
                            5,
                            Opcode::WrappingDivI32,
                            Ty::I32,
                            vec![0, 1],
                            InstData::None,
                        ),
                        inst(
                            6,
                            Opcode::WrappingRemI32,
                            Ty::I32,
                            vec![0, 1],
                            InstData::None,
                        ),
                        inst(7, Opcode::Return, Ty::Unit, vec![6], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn compile_i32_comparisons() {
        let module = make_module(
            "test",
            vec![Function {
                id: FuncId(0),
                name: "cmpi32".to_string(),
                params: vec![Ty::I32, Ty::I32],
                param_names: vec![],
                return_ty: Ty::Bool,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::I32, vec![], InstData::ArgIndex(0)),
                        inst(1, Opcode::GetArg, Ty::I32, vec![], InstData::ArgIndex(1)),
                        inst(2, Opcode::EqI32, Ty::Bool, vec![0, 1], InstData::None),
                        inst(3, Opcode::NeI32, Ty::Bool, vec![0, 1], InstData::None),
                        inst(4, Opcode::LtI32, Ty::Bool, vec![0, 1], InstData::None),
                        inst(5, Opcode::LeI32, Ty::Bool, vec![0, 1], InstData::None),
                        inst(6, Opcode::GtI32, Ty::Bool, vec![0, 1], InstData::None),
                        inst(7, Opcode::GeI32, Ty::Bool, vec![0, 1], InstData::None),
                        inst(8, Opcode::Return, Ty::Unit, vec![7], InstData::None),
                    ],
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }
}
