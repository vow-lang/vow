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
use std::collections::HashMap;
use std::sync::Arc;
use vow_ir::{
    BlockId, FuncId as IrFuncId, Function as IrFunction, Inst, InstData, InstId,
    Module as IrModule, Opcode, Ty as IrTy,
};

use crate::{Backend, BuildMode, CodegenError, CompiledObject};

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
                Opcode::Phi => {
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
    if mode == BuildMode::Release {
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
        IrTy::Bool => Some(types::I8),
        IrTy::Unit => None,
        IrTy::Ptr | IrTy::LinearPtr => Some(types::I64),
    }
}

fn build_signature(ir_func: &IrFunction, call_conv: cranelift_codegen::isa::CallConv) -> Signature {
    let mut sig = Signature::new(call_conv);
    for &param_ty in &ir_func.params {
        if let Some(cl_ty) = ir_ty_to_cranelift(param_ty) {
            sig.params.push(AbiParam::new(cl_ty));
        }
    }
    if let Some(cl_ty) = ir_ty_to_cranelift(ir_func.return_ty) {
        sig.returns.push(AbiParam::new(cl_ty));
    }
    sig
}

// ---------------------------------------------------------------------------
// Instruction lowering
// ---------------------------------------------------------------------------

struct LowerCtx<'a> {
    value_map: &'a mut HashMap<InstId, Value>,
    block_map: &'a HashMap<BlockId, Block>,
    phi_data: &'a PhiUpsilonData,
    arg_values: &'a HashMap<u32, Value>,
    return_ty: IrTy,
    ir_func_id_to_ref: &'a HashMap<IrFuncId, FuncRef>,
    vow_violation_ref: Option<FuncRef>,
    overflow_ref: Option<FuncRef>,
    arena_alloc_ref: FuncRef,
    arena_free_ref: FuncRef,
    mode: BuildMode,
    current_ir_block: BlockId,
    // index into module.strings → Cranelift GlobalValue (address of null-terminated string)
    string_global_values: &'a HashMap<u32, GlobalValue>,
    // extern symbol name → FuncRef declared in the current function
    extern_func_refs: &'a HashMap<String, FuncRef>,
    // VowId → GlobalValue for description strings
    vow_desc_global_values: &'a HashMap<u32, GlobalValue>,
    // (vow_id, binding_index) → GlobalValue for the binding's name C-string
    vow_binding_name_gvs: &'a HashMap<(u32, u32), GlobalValue>,
    // InstId → IrTy for all instructions in the current function
    inst_ty_map: &'a HashMap<InstId, IrTy>,
    // reference to current IrFunction for accessing vow entries
    ir_func: &'a IrFunction,
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
            let val = builder.ins().iconst(types::I8, b as i64);
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
        Opcode::WrappingAddI32 | Opcode::WrappingAddI64 => {
            let val = builder.ins().iadd(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::WrappingSubI32 | Opcode::WrappingSubI64 => {
            let val = builder.ins().isub(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::WrappingMulI32 | Opcode::WrappingMulI64 => {
            let val = builder.ins().imul(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::WrappingDivI32 | Opcode::WrappingDivI64 => {
            let val = builder.ins().sdiv(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::WrappingRemI32 | Opcode::WrappingRemI64 => {
            let val = builder.ins().srem(arg!(0), arg!(1));
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
        Opcode::CheckedSubI32 | Opcode::CheckedSubI64 => {
            let (result, overflow) = builder.ins().ssub_overflow(arg!(0), arg!(1));
            emit_overflow_check(builder, overflow, ctx)?;
            ctx.value_map.insert(inst.id, result);
        }
        Opcode::CheckedMulI32 | Opcode::CheckedMulI64 => {
            let (result, overflow) = builder.ins().smul_overflow(arg!(0), arg!(1));
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
        Opcode::CheckedRemI32 | Opcode::CheckedRemI64 => {
            let cl_ty = ir_ty_to_cranelift(inst.ty).unwrap_or(types::I64);
            let zero = builder.ins().iconst(cl_ty, 0);
            let is_zero = builder.ins().icmp(IntCC::Equal, arg!(1), zero);
            emit_overflow_check(builder, is_zero, ctx)?;
            let val = builder.ins().srem(arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }

        // ------------------------------------------------------------------
        // Integer comparisons (return Bool / I8)
        // ------------------------------------------------------------------
        Opcode::EqI32 | Opcode::EqI64 => {
            let val = builder.ins().icmp(IntCC::Equal, arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::NeI32 | Opcode::NeI64 => {
            let val = builder.ins().icmp(IntCC::NotEqual, arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::LtI32 | Opcode::LtI64 => {
            let val = builder.ins().icmp(IntCC::SignedLessThan, arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::LeI32 | Opcode::LeI64 => {
            let val = builder
                .ins()
                .icmp(IntCC::SignedLessThanOrEqual, arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::GtI32 | Opcode::GtI64 => {
            let val = builder
                .ins()
                .icmp(IntCC::SignedGreaterThan, arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::GeI32 | Opcode::GeI64 => {
            let val = builder
                .ins()
                .icmp(IntCC::SignedGreaterThanOrEqual, arg!(0), arg!(1));
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
            let val = builder.ins().fcmp(FloatCC::Equal, arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::NeF32 | Opcode::NeF64 => {
            let val = builder.ins().fcmp(FloatCC::NotEqual, arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::LtF32 | Opcode::LtF64 => {
            let val = builder.ins().fcmp(FloatCC::LessThan, arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::LeF32 | Opcode::LeF64 => {
            let val = builder
                .ins()
                .fcmp(FloatCC::LessThanOrEqual, arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::GtF32 | Opcode::GtF64 => {
            let val = builder.ins().fcmp(FloatCC::GreaterThan, arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }
        Opcode::GeF32 | Opcode::GeF64 => {
            let val = builder
                .ins()
                .fcmp(FloatCC::GreaterThanOrEqual, arg!(0), arg!(1));
            ctx.value_map.insert(inst.id, val);
        }

        // ------------------------------------------------------------------
        // Boolean operations (I8)
        // ------------------------------------------------------------------
        Opcode::Not => {
            let one = builder.ins().iconst(types::I8, 1);
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
            if ctx.return_ty == IrTy::Unit {
                builder.ins().return_(&[]);
            } else if let Some(&val_id) = inst.args.first() {
                if let Some(&val) = ctx.value_map.get(&val_id) {
                    builder.ins().return_(&[val]);
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
            if ctx.mode == BuildMode::Debug
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
                emit_vow_check(builder, pred, vow_id, blame_byte, &captures, ctx)?;
            }
            // In Release mode: no-op
        }

        // ------------------------------------------------------------------
        // Function calls
        // ------------------------------------------------------------------
        Opcode::Call => {
            let func_ref = match &inst.data {
                InstData::CallTarget(f) => {
                    let Some(&fr) = ctx.ir_func_id_to_ref.get(f) else {
                        return Err(CodegenError::UnsupportedOpcode(format!(
                            "unknown call target FuncId({:?})",
                            f
                        )));
                    };
                    fr
                }
                InstData::CallExtern(sym) => {
                    let Some(&fr) = ctx.extern_func_refs.get(sym.as_str()) else {
                        return Err(CodegenError::UnsupportedOpcode(format!(
                            "unknown extern symbol: {sym}"
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
            let call_args: Vec<Value> = inst
                .args
                .iter()
                .enumerate()
                .filter_map(|(i, id)| {
                    let v = ctx.value_map.get(id).copied()?;
                    if let Some(&expected_ty) = expected_types.get(i) {
                        let actual_ty = builder.func.dfg.value_type(v);
                        if actual_ty == types::I32 && expected_ty == types::I64 {
                            return Some(builder.ins().sextend(types::I64, v));
                        }
                        if actual_ty == types::I8 && expected_ty == types::I64 {
                            return Some(builder.ins().uextend(types::I64, v));
                        }
                    }
                    Some(v)
                })
                .collect();
            let call_inst = builder.ins().call(func_ref, &call_args);
            let results = builder.inst_results(call_inst);
            if results.is_empty() {
                let unit = builder.ins().iconst(types::I32, 0);
                ctx.value_map.insert(inst.id, unit);
            } else {
                ctx.value_map.insert(inst.id, results[0]);
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
            let size_val = builder.ins().iconst(types::I64, size);
            let align_val = builder.ins().iconst(types::I64, align);
            let call_inst = builder.ins().call(ctx.arena_alloc_ref, &[size_val, align_val]);
            let ptr = builder.inst_results(call_inst)[0];
            ctx.value_map.insert(inst.id, ptr);
        }
        Opcode::RegionFree => {
            if let Some(&ptr_id) = inst.args.first()
                && let Some(&ptr_val) = ctx.value_map.get(&ptr_id)
            {
                builder.ins().call(ctx.arena_free_ref, &[ptr_val]);
            }
            let unit = builder.ins().iconst(types::I32, 0);
            ctx.value_map.insert(inst.id, unit);
        }
        Opcode::LinearConsume | Opcode::LinearBorrow => {
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
                let raw = builder.ins().load(types::I64, MemFlags::trusted(), base, offset);
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
                builder.ins().store(MemFlags::trusted(), store_val, base, offset);
                let unit = builder.ins().iconst(types::I32, 0);
                ctx.value_map.insert(inst.id, unit);
            }
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
    ctx: &mut LowerCtx,
) -> Result<(), CodegenError> {
    let one = builder.ins().iconst(types::I8, 1);
    let inv = builder.ins().bxor(predicate, one);

    let violation_block = builder.create_block();
    let cont_block = builder.create_block();
    builder
        .ins()
        .brif(inv, violation_block, &[], cont_block, &[]);

    builder.switch_to_block(violation_block);
    builder.seal_block(violation_block);
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
                    IrTy::Bool => builder.ins().uextend(types::I64, *cl_val),
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

        builder.ins().call(
            vr,
            &[vow_id_val, blame_val, desc_ptr, bindings_ptr, count_val],
        );
    }
    builder.ins().trap(TrapCode::unwrap_user(1));

    builder.switch_to_block(cont_block);
    builder.seal_block(cont_block);
    Ok(())
}

// ---------------------------------------------------------------------------
// Function compilation
// ---------------------------------------------------------------------------

struct RuntimeIds {
    vow_violation_id: Option<CraneliftFuncId>,
    overflow_id: Option<CraneliftFuncId>,
    arena_alloc_id: CraneliftFuncId,
    arena_free_id: CraneliftFuncId,
}

#[allow(clippy::too_many_arguments)]
fn compile_ir_function(
    ctx: &mut Context,
    ir_func: &IrFunction,
    builder_ctx: &mut FunctionBuilderContext,
    mode: BuildMode,
    obj_module: &mut ObjectModule,
    ir_to_cl: &[(IrFuncId, CraneliftFuncId)],
    runtime: &RuntimeIds,
    string_data_ids: &[DataId],
    extern_func_ids: &HashMap<String, CraneliftFuncId>,
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
    let arena_alloc_ref =
        obj_module.declare_func_in_func(runtime.arena_alloc_id, builder.func);
    let arena_free_ref =
        obj_module.declare_func_in_func(runtime.arena_free_id, builder.func);

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

    // Create data sections for vow description strings and map VowId → GlobalValue
    let mut vow_desc_global_values: HashMap<u32, GlobalValue> = HashMap::new();
    let mut vow_binding_name_gvs: HashMap<(u32, u32), GlobalValue> = HashMap::new();
    if mode == BuildMode::Debug {
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

    // Collect entry block arg Values → ArgIndex map
    let mut arg_values: HashMap<u32, Value> = HashMap::new();
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
    }

    let mut value_map: HashMap<InstId, Value> = HashMap::new();

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
            return_ty: ir_func.return_ty,
            ir_func_id_to_ref: &ir_func_id_to_ref,
            vow_violation_ref,
            overflow_ref,
            arena_alloc_ref,
            arena_free_ref,
            mode,
            current_ir_block: ir_block.id,
            string_global_values: &string_global_values,
            extern_func_refs: &extern_func_refs,
            vow_desc_global_values: &vow_desc_global_values,
            vow_binding_name_gvs: &vow_binding_name_gvs,
            inst_ty_map: &inst_ty_map,
            ir_func,
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
        "__vow_print_i64" => {
            sig.params.push(AbiParam::new(types::I64)); // value
        }
        "__vow_vec_new" => {
            sig.params.push(AbiParam::new(types::I64)); // elem_size
            sig.params.push(AbiParam::new(types::I64)); // elem_align
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec
        }
        "__vow_vec_len" => {
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
            sig.returns.push(AbiParam::new(types::I64)); // len
        }
        "__vow_vec_push_val" => {
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
            sig.params.push(AbiParam::new(types::I64)); // value (i64)
        }
        "__vow_vec_get_val" => {
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
            sig.params.push(AbiParam::new(types::I64)); // index
            sig.returns.push(AbiParam::new(types::I64)); // element value
        }
        _ => {}
    }
    sig
}

// ---------------------------------------------------------------------------
// Backend trait implementation
// ---------------------------------------------------------------------------

impl Backend for CraneliftBackend {
    fn compile_module(
        &self,
        module: &IrModule,
        mode: BuildMode,
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
        let mut extern_syms: Vec<String> = Vec::new();
        for func in &module.functions {
            for block in &func.blocks {
                for inst in &block.insts {
                    if let InstData::CallExtern(sym) = &inst.data
                        && !extern_syms.contains(sym)
                    {
                        extern_syms.push(sym.clone());
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

        // Declare arena runtime functions (always needed)
        let mut arena_alloc_sig = obj_module.make_signature();
        arena_alloc_sig.params.push(AbiParam::new(types::I64)); // size
        arena_alloc_sig.params.push(AbiParam::new(types::I64)); // align
        arena_alloc_sig.returns.push(AbiParam::new(types::I64)); // *mut u8
        let arena_alloc_id = obj_module
            .declare_function("__vow_arena_alloc", Linkage::Import, &arena_alloc_sig)
            .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;

        let mut arena_free_sig = obj_module.make_signature();
        arena_free_sig.params.push(AbiParam::new(types::I64)); // *mut u8
        let arena_free_id = obj_module
            .declare_function("__vow_arena_free", Linkage::Import, &arena_free_sig)
            .map_err(|e| CodegenError::FunctionDeclare(e.to_string()))?;

        // Declare external runtime functions (debug mode only)
        let (vow_violation_id, overflow_id) = if mode == BuildMode::Debug {
            let mut violation_sig = obj_module.make_signature();
            violation_sig.params.push(AbiParam::new(types::I32)); // vow_id
            violation_sig.params.push(AbiParam::new(types::I8)); // blame
            violation_sig.params.push(AbiParam::new(types::I64)); // desc_ptr
            violation_sig.params.push(AbiParam::new(types::I64)); // bindings_ptr
            violation_sig.params.push(AbiParam::new(types::I32)); // binding_count
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
                &mut obj_module,
                &ir_to_cl,
                &RuntimeIds {
                    vow_violation_id,
                    overflow_id,
                    arena_alloc_id,
                    arena_free_id,
                },
                &string_data_ids,
                &extern_func_ids,
            )?;

            obj_module
                .define_function(cl_id, &mut cl_ctx)
                .map_err(|e| CodegenError::FunctionDefine(e.to_string()))?;
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
    use vow_ir::{BasicBlock, BlockId, FuncId, Function, InstData, InstId, Module, Opcode, Ty};
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
                return_ty: Ty::Unit,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts: vec![inst(0, Opcode::Return, Ty::Unit, vec![], InstData::None)],
                }],
            }],
        );
        let result = CraneliftBackend::new().compile_module(&module, BuildMode::Debug);
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
            }],
        );
        let result = CraneliftBackend::new().compile_module(&module, BuildMode::Debug);
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
            }],
        );
        let result = CraneliftBackend::new().compile_module(&module, BuildMode::Debug);
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
                            },
                            inst(9, Opcode::Return, Ty::Unit, vec![6], InstData::None),
                        ],
                    },
                ],
            }],
        );
        let _ = VowId(0); // suppress unused import
        let _ = VowEntry {
            id: vow_ir::VowId(0),
            description: String::new(),
            blame: vow_diag::Blame::None,
            bindings: vec![],
        };
        let result = CraneliftBackend::new().compile_module(&module, BuildMode::Debug);
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
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![],
        };
        let sig = build_signature(&ir_func, CallConv::SystemV);
        assert_eq!(sig.params.len(), 2);
        assert_eq!(sig.returns.len(), 1);
        assert_eq!(sig.params[0].value_type, types::I64);
        assert_eq!(sig.returns[0].value_type, types::I64);
    }
}
