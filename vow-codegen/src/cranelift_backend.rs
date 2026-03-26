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
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use vow_ir::{
    BlockId, FuncId as IrFuncId, Function as IrFunction, Inst, InstData, InstId,
    Module as IrModule, Opcode, Ty as IrTy,
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
                Opcode::Phi => {
                    // Only track phis that have a Cranelift representation.
                    // Unit-typed phis don't become block params so must be excluded.
                    if ir_ty_to_cranelift(inst.ty).is_some() {
                        block_phis.entry(block.id).or_default().push(inst.id);
                        phi_home.insert(inst.id, block.id);
                    }
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
        IrTy::Bool => Some(types::I64),
        IrTy::U64 => Some(types::I64),
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
    return_ty: IrTy,
    ir_func_id_to_ref: &'a HashMap<IrFuncId, FuncRef>,
    vow_violation_ref: Option<FuncRef>,
    overflow_ref: Option<FuncRef>,
    arena_alloc_ref: FuncRef,
    arena_free_ref: FuncRef,
    mode: BuildMode,
    trace: TraceMode,
    current_ir_block: BlockId,
    string_global_values: &'a HashMap<u32, GlobalValue>,
    extern_func_refs: &'a HashMap<String, FuncRef>,
    vow_desc_global_values: &'a HashMap<u32, GlobalValue>,
    vow_file_global_values: &'a HashMap<u32, GlobalValue>,
    vow_binding_name_gvs: &'a HashMap<(u32, u32), GlobalValue>,
    inst_ty_map: &'a HashMap<InstId, IrTy>,
    ir_func: &'a IrFunction,
    trace_exit_ref: Option<FuncRef>,
    trace_vow_ref: Option<FuncRef>,
    fn_name_gv: Option<GlobalValue>,
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
        // Bitwise XOR
        // ------------------------------------------------------------------
        Opcode::XorI32 | Opcode::XorI64 | Opcode::XorU64 => {
            let val = builder.ins().bxor(arg!(0), arg!(1));
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
            if ctx.return_ty == IrTy::Unit {
                builder.ins().return_(&[]);
            } else if let Some(&val_id) = inst.args.first() {
                if let Some(&val) = ctx.value_map.get(&val_id) {
                    let val = coerce_return_value(builder, val, ctx.return_ty);
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
                .map(|(i, id)| {
                    let v = *ctx.value_map.get(id).unwrap_or_else(|| {
                        panic!(
                            "cranelift backend: Call value_map miss for arg {id:?} in inst {:?}",
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
            let size_val = builder.ins().iconst(types::I64, size);
            let align_val = builder.ins().iconst(types::I64, align);
            let call_inst = builder
                .ins()
                .call(ctx.arena_alloc_ref, &[size_val, align_val]);
            let ptr = builder.inst_results(call_inst)[0];
            ctx.value_map.insert(inst.id, ptr);
        }
        Opcode::RegionFree => {
            let ptr_id = *inst.args.first().expect("RegionFree missing arg");
            let ptr_val = *ctx.value_map.get(&ptr_id).unwrap_or_else(|| {
                panic!(
                    "cranelift backend: RegionFree value_map miss for {:?}",
                    inst.id
                )
            });
            let (size, align) = if let InstData::AllocSize { size, align } = inst.data {
                (size as i64, align as i64)
            } else {
                (0, 8)
            };
            let size_val = builder.ins().iconst(types::I64, size);
            let align_val = builder.ins().iconst(types::I64, align);
            builder
                .ins()
                .call(ctx.arena_free_ref, &[ptr_val, size_val, align_val]);
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
    arena_free_id: CraneliftFuncId,
    trace_enter_id: Option<CraneliftFuncId>,
    trace_exit_id: Option<CraneliftFuncId>,
    trace_vow_id: Option<CraneliftFuncId>,
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
    let arena_free_ref = obj_module.declare_func_in_func(runtime.arena_free_id, builder.func);

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
    let mut vow_file_global_values: HashMap<u32, GlobalValue> = HashMap::new();
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
    let fn_name_gv = if trace != TraceMode::Off {
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
        if trace != TraceMode::Off
            && let (Some(enter_ref), Some(gv)) = (trace_enter_ref, fn_name_gv)
        {
            let name_ptr = builder.ins().global_value(types::I64, gv);
            builder.ins().call(enter_ref, &[name_ptr]);
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
            trace,
            current_ir_block: ir_block.id,
            string_global_values: &string_global_values,
            extern_func_refs: &extern_func_refs,
            vow_desc_global_values: &vow_desc_global_values,
            vow_file_global_values: &vow_file_global_values,
            vow_binding_name_gvs: &vow_binding_name_gvs,
            inst_ty_map: &inst_ty_map,
            ir_func,
            trace_exit_ref,
            trace_vow_ref,
            fn_name_gv,
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
        "__vow_string_from_cstr" => {
            sig.params.push(AbiParam::new(types::I64)); // C-string ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
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
        "__vow_string_push_str" => {
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
        "__vow_string_from_i64" => {
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
        "__vow_string_substring" => {
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
        "__vow_string_to_upper" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_to_lower" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        "__vow_string_replace" => {
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
        "__vow_args" => {
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<String>
        }
        "__vow_stdin_read" => {
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
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
        "__vow_process_stdout_for" | "__vow_process_stderr_for" => {
            sig.params.push(AbiParam::new(types::I64)); // handle
            sig.returns.push(AbiParam::new(types::I64)); // *VowVec<u8>
        }
        // Typed deallocation
        "__vow_string_free" => {
            sig.params.push(AbiParam::new(types::I64)); // string ptr
        }
        "__vow_vec_free_val" => {
            sig.params.push(AbiParam::new(types::I64)); // vec ptr
        }
        "__vow_map_free" => {
            sig.params.push(AbiParam::new(types::I64)); // map ptr
        }
        // HashMap runtime
        "__vow_map_new" => {
            sig.returns.push(AbiParam::new(types::I64)); // *VowMap
        }
        "__vow_map_insert" => {
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
        "__vow_map_len" => {
            sig.params.push(AbiParam::new(types::I64)); // map ptr
            sig.returns.push(AbiParam::new(types::I64)); // len
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
        }
        "__vow_clif_compile_function" => {
            for _ in 0..23 {
                sig.params.push(AbiParam::new(types::I64));
            }
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
            for block in &func.blocks {
                for inst in &block.insts {
                    if let InstData::CallExtern(sym) = &inst.data {
                        extern_syms.insert(sym.clone());
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
        arena_free_sig.params.push(AbiParam::new(types::I64)); // size
        arena_free_sig.params.push(AbiParam::new(types::I64)); // align
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
                    arena_free_id,
                    trace_enter_id,
                    trace_exit_id,
                    trace_vow_id,
                },
                &string_data_ids,
                &extern_func_ids,
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
        BasicBlock, BlockId, FuncId, Function, InstData, InstId, Module, Opcode, Ty, VowEntry,
        VowId,
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
                local_names: std::collections::HashMap::new(),
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
        };
        let sig = build_signature(&ir_func, CallConv::SystemV);
        assert_eq!(sig.params.len(), 2);
        assert_eq!(sig.returns.len(), 1);
        assert_eq!(sig.params[0].value_type, types::I64);
        assert_eq!(sig.returns[0].value_type, types::I64);
    }

    #[test]
    fn cranelift_backend_default_impl() {
        let _ = CraneliftBackend::default();
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
                        InstData::ConstF64(2.718f64),
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
                    inst(
                        1,
                        Opcode::RegionFree,
                        Ty::Unit,
                        vec![0],
                        InstData::AllocSize { size: 64, align: 8 },
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
                        InstData::ConstF64(2.718f64),
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
                },
            ],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
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
            }],
        );
        let result =
            CraneliftBackend::new().compile_module(&module, BuildMode::Debug, TraceMode::Off);
        assert!(result.is_ok(), "{:?}", result.err());
    }
}
