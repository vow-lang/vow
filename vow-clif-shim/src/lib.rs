#![allow(clippy::missing_safety_doc)]

// ---------------------------------------------------------------------------
// User trap code registry
// ---------------------------------------------------------------------------
//
// The shim emits Cranelift user trap codes to signal runtime conditions that
// should fail loudly. Codes are allocated centrally here so collisions with
// `vow-codegen` (which emits the same numeric space) are visible at a glance:
//
//   1  — VowViolation fallthrough (after `__vow_violation` is called, a trap
//         prevents execution from continuing; see line ~1906).
//   2  — `Unreachable` opcode hit at runtime (e.g. pattern-match exhaustion;
//         line ~1371).
//   3  — Unimplemented region opcode (Phase 2 `RegionOpen`/`RegionClose`
//         guard; line ~1584). Intentionally asymmetric with
//         `vow-codegen/src/cranelift_backend.rs`, which panics via
//         `unreachable!("not emitted in Phase 2")` at **compile time** for
//         the same opcodes — the shim cannot panic across the FFI boundary
//         so it traps at **runtime** instead.
//
// When adding a new trap code, update this registry and the corresponding
// `vow-codegen` site (if any) so agents reading either file see the full
// picture.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    AbiParam, Block, FuncRef, GlobalValue, InstBuilder, MemFlags, Signature, StackSlot,
    StackSlotData, StackSlotKind, TrapCode, Value, types,
};
use cranelift_codegen::isa::TargetIsa;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{
    DataDescription, DataId, FuncId as CraneliftFuncId, Linkage, Module as CraneliftModule,
};
use cranelift_object::{ObjectBuilder, ObjectModule};

// ---------------------------------------------------------------------------
// VowVec layout: { ptr: *mut u8, len: usize, cap: usize } = 24 bytes
// ---------------------------------------------------------------------------

#[repr(C)]
struct VowVec {
    ptr: *mut u8,
    len: usize,
    cap: usize,
}

const _: () = assert!(size_of::<VowVec>() == 24);

unsafe fn read_i64_slice(vow_vec_ptr: i64) -> &'static [i64] {
    let v = unsafe { &*(vow_vec_ptr as *const VowVec) };
    if v.ptr.is_null() || v.len == 0 {
        return &[];
    }
    unsafe { std::slice::from_raw_parts(v.ptr as *const i64, v.len) }
}

unsafe fn read_vow_string(vow_vec_ptr: i64) -> &'static str {
    let v = unsafe { &*(vow_vec_ptr as *const VowVec) };
    if v.ptr.is_null() || v.len == 0 {
        return "";
    }
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    std::str::from_utf8(bytes).unwrap_or("")
}

// ---------------------------------------------------------------------------
// IR type/opcode constants (must match compiler/ir.vow)
// ---------------------------------------------------------------------------

const ITY_I32: i64 = 0;
const ITY_I64: i64 = 1;
const ITY_F32: i64 = 2;
const ITY_F64: i64 = 3;
const ITY_BOOL: i64 = 4;
const ITY_UNIT: i64 = 5;
const ITY_PTR: i64 = 6;
const ITY_LPTR: i64 = 7;
const ITY_U64: i64 = 8;

fn ity_to_cranelift(ty: i64) -> Option<types::Type> {
    match ty {
        ITY_I32 => Some(types::I32),
        ITY_I64 => Some(types::I64),
        ITY_F32 => Some(types::F32),
        ITY_F64 => Some(types::F64),
        ITY_BOOL => Some(types::I64),
        ITY_U64 => Some(types::I64),
        ITY_UNIT => None,
        ITY_PTR | ITY_LPTR => Some(types::I64),
        _ => None,
    }
}

const IOP_CONST_I32: i64 = 0;
const IOP_CONST_I64: i64 = 1;
const IOP_CONST_F32: i64 = 2;
const IOP_CONST_F64: i64 = 3;
const IOP_CONST_BOOL: i64 = 4;
const IOP_CONST_STR: i64 = 5;
const IOP_CONST_UNIT: i64 = 6;
const IOP_GET_ARG: i64 = 7;

const IOP_WADD_I32: i64 = 8;
const IOP_WSUB_I32: i64 = 9;
const IOP_WMUL_I32: i64 = 10;
const IOP_WDIV_I32: i64 = 11;
const IOP_WREM_I32: i64 = 12;
const IOP_CADD_I32: i64 = 13;
const IOP_CSUB_I32: i64 = 14;
const IOP_CMUL_I32: i64 = 15;
const IOP_CDIV_I32: i64 = 16;
const IOP_CREM_I32: i64 = 17;
const IOP_EQ_I32: i64 = 18;
const IOP_NE_I32: i64 = 19;
const IOP_LT_I32: i64 = 20;
const IOP_LE_I32: i64 = 21;
const IOP_GT_I32: i64 = 22;
const IOP_GE_I32: i64 = 23;

const IOP_WADD_I64: i64 = 24;
const IOP_WSUB_I64: i64 = 25;
const IOP_WMUL_I64: i64 = 26;
const IOP_WDIV_I64: i64 = 27;
const IOP_WREM_I64: i64 = 28;
const IOP_CADD_I64: i64 = 29;
const IOP_CSUB_I64: i64 = 30;
const IOP_CMUL_I64: i64 = 31;
const IOP_CDIV_I64: i64 = 32;
const IOP_CREM_I64: i64 = 33;
const IOP_EQ_I64: i64 = 34;
const IOP_NE_I64: i64 = 35;
const IOP_LT_I64: i64 = 36;
const IOP_LE_I64: i64 = 37;
const IOP_GT_I64: i64 = 38;
const IOP_GE_I64: i64 = 39;

const IOP_ADD_F32: i64 = 40;
const IOP_SUB_F32: i64 = 41;
const IOP_MUL_F32: i64 = 42;
const IOP_DIV_F32: i64 = 43;
const IOP_REM_F32: i64 = 44;
const IOP_EQ_F32: i64 = 45;
const IOP_NE_F32: i64 = 46;
const IOP_LT_F32: i64 = 47;
const IOP_LE_F32: i64 = 48;
const IOP_GT_F32: i64 = 49;
const IOP_GE_F32: i64 = 50;

const IOP_ADD_F64: i64 = 51;
const IOP_SUB_F64: i64 = 52;
const IOP_MUL_F64: i64 = 53;
const IOP_DIV_F64: i64 = 54;
const IOP_REM_F64: i64 = 55;
const IOP_EQ_F64: i64 = 56;
const IOP_NE_F64: i64 = 57;
const IOP_LT_F64: i64 = 58;
const IOP_LE_F64: i64 = 59;
const IOP_GT_F64: i64 = 60;
const IOP_GE_F64: i64 = 61;

const IOP_NOT: i64 = 62;
const IOP_AND: i64 = 63;
const IOP_OR: i64 = 64;

const IOP_LOAD: i64 = 65;
const IOP_STORE: i64 = 66;

const IOP_BRANCH: i64 = 67;
const IOP_JUMP: i64 = 68;
const IOP_RETURN: i64 = 69;
const IOP_UNREACHABLE: i64 = 70;

const IOP_PHI: i64 = 71;
const IOP_UPSILON: i64 = 72;

const IOP_VOW_REQ: i64 = 73;
const IOP_VOW_ENS: i64 = 74;
const IOP_VOW_INV: i64 = 75;

const IOP_CALL: i64 = 76;

const IOP_REGION_ALLOC: i64 = 77;
const IOP_REGION_FREE: i64 = 78;

// Region-kind discriminants. Mirror `compiler/ir.vow::REGION_KIND_*` and
// `RegionId` in `vow-ir/src/types.rs`. The packed i64 is `val * 4 + kind`
// (see `region_pack`).
const REGION_KIND_BLOCK: i64 = 0;
const REGION_KIND_CALLER: i64 = 1;
const REGION_KIND_ROOT: i64 = 2;
const REGION_KIND_RODATA: i64 = 3;

const IOP_LINEAR_CONSUME: i64 = 79;
const IOP_LINEAR_BORROW: i64 = 80;

const IOP_FIELD_GET: i64 = 81;
const IOP_FIELD_SET: i64 = 82;

const IOP_XOR_I32: i64 = 83;
const IOP_XOR_I64: i64 = 84;

const IOP_WADD_U64: i64 = 85;
const IOP_WSUB_U64: i64 = 86;
const IOP_WMUL_U64: i64 = 87;
const IOP_WDIV_U64: i64 = 88;
const IOP_WREM_U64: i64 = 89;
const IOP_CADD_U64: i64 = 90;
const IOP_CSUB_U64: i64 = 91;
const IOP_CMUL_U64: i64 = 92;
const IOP_CDIV_U64: i64 = 93;
const IOP_CREM_U64: i64 = 94;
const IOP_EQ_U64: i64 = 95;
const IOP_NE_U64: i64 = 96;
const IOP_LT_U64: i64 = 97;
const IOP_LE_U64: i64 = 98;
const IOP_GT_U64: i64 = 99;
const IOP_GE_U64: i64 = 100;
const IOP_XOR_U64: i64 = 101;
const IOP_CONST_U64: i64 = 102;
const IOP_CAST_I64_TO_U64: i64 = 103;
const IOP_CAST_U64_TO_I64: i64 = 104;
const IOP_DEBUG_CALL: i64 = 105;
const IOP_BITAND_I64: i64 = 106;
const IOP_BITOR_I64: i64 = 107;
const IOP_SHL_I64: i64 = 108;
const IOP_SHR_I64: i64 = 109;
const IOP_BITAND_U64: i64 = 110;
const IOP_BITOR_U64: i64 = 111;
const IOP_SHL_U64: i64 = 112;
const IOP_SHR_U64: i64 = 113;
// Phase 2: declared but never emitted. Phase 4 wires arena open/close to
// __vow_arena_open / __vow_arena_close. If one leaks into the shim today
// it is a defensive no-op rather than a misdispatch (see compile_function
// below).
#[allow(dead_code)]
const IOP_REGION_OPEN: i64 = 114;
#[allow(dead_code)]
const IOP_REGION_CLOSE: i64 = 115;

// InstData kind constants (match compiler/ir.vow IDATA_*)
#[allow(dead_code)]
const IDATA_NONE: i64 = 0;
const IDATA_CONST_I32: i64 = 1;
const IDATA_CONST_I64: i64 = 2;
const IDATA_CONST_F32: i64 = 3;
const IDATA_CONST_F64: i64 = 4;
const IDATA_CONST_BOOL: i64 = 5;
const IDATA_ARG_INDEX: i64 = 6;
const IDATA_PHI_TARGET: i64 = 7;
const IDATA_CONST_STR: i64 = 8;
const IDATA_CALL_TARGET: i64 = 9;
const IDATA_CALL_EXTERN: i64 = 10;
const IDATA_BRANCH_TARGETS: i64 = 11;
const IDATA_JUMP_TARGET: i64 = 12;
#[allow(dead_code)]
const IDATA_REGION_ID: i64 = 13;
const IDATA_VOW_ID: i64 = 14;
const IDATA_ALLOC_SIZE: i64 = 15;
const IDATA_FIELD: i64 = 16;
const IDATA_CONST_U64: i64 = 17;

// ---------------------------------------------------------------------------
// Module context (opaque handle passed through FFI)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct FuncDecl {
    cl_id: CraneliftFuncId,
    name: String,
    param_tys: Vec<i64>,
    ret_ty: i64,
    is_main: bool,
}

struct ModuleContext {
    isa: Arc<dyn TargetIsa>,
    obj_module: ObjectModule,
    builder_ctx: FunctionBuilderContext,
    string_data_ids: Vec<DataId>,
    func_decls: Vec<FuncDecl>,
    extern_func_ids: HashMap<String, CraneliftFuncId>,
    mode: i64,       // 0=release, 1=debug, 2=profile, 3=sanitize
    trace_mode: i64, // 0=off, 1=calls, 2=full
    fn_scratch: FnScratch,
}

// Per-function scratch accumulated via incremental FFI. Buffers are reused
// across functions — Rust's `Vec::clear()` preserves capacity, so amortized
// cost converges to the peak function's working set.
//
// The `*_ptrs` fields below hold raw VowVec pointers aliased from caller-owned
// strings (the live `IrModule`). They are only valid within one
// `fn_begin → fn_end` cycle, since `fn_end` resets the scratch. Do not retain
// these values across FFI boundaries.
#[derive(Default)]
struct FnScratch {
    // True between a successful `fn_begin` and the matching `fn_end`. Gates
    // `fn_block`/`fn_inst`/`fn_vow`/`fn_end` so an out-of-order caller hits a
    // clean `-1` return instead of silently accumulating into `func_idx == 0`
    // or panicking Cranelift mid-seal.
    began: bool,
    func_idx: i64,
    ret_ty: i64,
    param_tys: Vec<i64>,
    // Per-block:
    block_starts: Vec<i64>, // offsets into inst_* arrays
    block_lengths: Vec<i64>,
    // Per-instruction (flattened across all blocks of this function):
    inst_ids: Vec<i64>,
    inst_ops: Vec<i64>,
    inst_tys: Vec<i64>,
    inst_dks: Vec<i64>,
    inst_dvs: Vec<i64>,
    inst_dv2s: Vec<i64>,
    inst_ds_ptrs: Vec<i64>, // raw VowVec ptrs — see struct-level doc
    // Region tag per instruction, packed as (val * 4 + kind) — mirrors
    // `region_pack` in `compiler/ir.vow` and `RegionId` in `vow-ir`. Today
    // only the kind byte matters for codegen; the payload is read so
    // `.vmod`-produced `Caller(idx)` regions carry through for error
    // reporting when the shim refuses them (see REGION_KIND_ROOT check
    // in IOP_REGION_ALLOC).
    inst_rgns: Vec<i64>,
    all_args: Vec<i64>,
    arg_offsets: Vec<i64>,
    arg_lengths: Vec<i64>,
    // Per vow entry:
    vow_ids: Vec<i64>,
    vow_desc_ptrs: Vec<i64>, // raw VowVec ptrs — see struct-level doc
    binding_counts: Vec<i64>,
    binding_inst_ids_all: Vec<i64>,
    binding_names_ptrs: Vec<i64>, // raw VowVec ptrs — see struct-level doc
}

impl FnScratch {
    // Reset len=0 on all fields while preserving each Vec's allocated capacity.
    fn reset(&mut self) {
        self.began = false;
        self.func_idx = 0;
        self.ret_ty = 0;
        self.param_tys.clear();
        self.block_starts.clear();
        self.block_lengths.clear();
        self.inst_ids.clear();
        self.inst_ops.clear();
        self.inst_tys.clear();
        self.inst_dks.clear();
        self.inst_dvs.clear();
        self.inst_dv2s.clear();
        self.inst_ds_ptrs.clear();
        self.inst_rgns.clear();
        self.all_args.clear();
        self.arg_offsets.clear();
        self.arg_lengths.clear();
        self.vow_ids.clear();
        self.vow_desc_ptrs.clear();
        self.binding_counts.clear();
        self.binding_inst_ids_all.clear();
        self.binding_names_ptrs.clear();
    }
}

// ---------------------------------------------------------------------------
// FFI: create / destroy
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn __vow_clif_create(mode: i64, trace_mode: i64) -> i64 {
    let mut flag_builder = settings::builder();
    if let Err(e) = flag_builder.set("use_colocated_libcalls", "false") {
        eprintln!("clif_shim: error setting use_colocated_libcalls: {e}");
        return 0;
    }
    if let Err(e) = flag_builder.set("is_pic", "true") {
        eprintln!("clif_shim: error setting is_pic: {e}");
        return 0;
    }
    if (mode == 0 || mode == 2)
        && let Err(e) = flag_builder.set("opt_level", "speed")
    {
        eprintln!("clif_shim: error setting opt_level: {e}");
        return 0;
    }
    let flags = settings::Flags::new(flag_builder);
    let isa = match cranelift_native::builder() {
        Ok(b) => match b.finish(flags) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("clif_shim: ISA build error: {e}");
                return 0;
            }
        },
        Err(e) => {
            eprintln!("clif_shim: native builder error: {e}");
            return 0;
        }
    };

    let obj_builder = match ObjectBuilder::new(
        isa.clone(),
        b"vow_module".to_vec(),
        cranelift_module::default_libcall_names(),
    ) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("clif_shim: ObjectBuilder error: {e}");
            return 0;
        }
    };

    let ctx = Box::new(ModuleContext {
        isa,
        obj_module: ObjectModule::new(obj_builder),
        builder_ctx: FunctionBuilderContext::new(),
        string_data_ids: Vec::new(),
        func_decls: Vec::new(),
        extern_func_ids: HashMap::new(),
        mode,
        trace_mode,
        fn_scratch: FnScratch::default(),
    });

    Box::into_raw(ctx) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_clif_destroy(ctx_ptr: i64) {
    if ctx_ptr != 0 {
        let _ = unsafe { Box::from_raw(ctx_ptr as *mut ModuleContext) };
    }
}

// ---------------------------------------------------------------------------
// FFI: add_string — register a string constant as a data section
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_clif_add_string(ctx_ptr: i64, str_ptr: i64) {
    let ctx = unsafe { &mut *(ctx_ptr as *mut ModuleContext) };
    let s = unsafe { read_vow_string(str_ptr) };
    let mut bytes = s.as_bytes().to_vec();
    bytes.push(0); // null terminate
    let mut desc = DataDescription::new();
    desc.define(bytes.into_boxed_slice());
    let data_id = ctx
        .obj_module
        .declare_anonymous_data(false, false)
        .expect("declare string data");
    ctx.obj_module
        .define_data(data_id, &desc)
        .expect("define string data");
    ctx.string_data_ids.push(data_id);
}

// ---------------------------------------------------------------------------
// FFI: declare_extern — declare an external runtime symbol
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_clif_declare_extern(ctx_ptr: i64, sym_ptr: i64) {
    let ctx = unsafe { &mut *(ctx_ptr as *mut ModuleContext) };
    let sym = unsafe { read_vow_string(sym_ptr) }.to_string();
    if ctx.extern_func_ids.contains_key(&sym) {
        return;
    }
    let sig = make_extern_sig(&sym, &ctx.obj_module);
    let cl_id = ctx
        .obj_module
        .declare_function(&sym, Linkage::Import, &sig)
        .expect("declare extern");
    ctx.extern_func_ids.insert(sym, cl_id);
}

// ---------------------------------------------------------------------------
// FFI: declare_function
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_clif_declare_function(
    ctx_ptr: i64,
    _idx: i64,
    name_ptr: i64,
    param_tys_ptr: i64,
    n_params: i64,
    ret_ty: i64,
    is_main: i64,
) {
    let ctx = unsafe { &mut *(ctx_ptr as *mut ModuleContext) };
    let name = unsafe { read_vow_string(name_ptr) };
    let param_slice = if param_tys_ptr != 0 && n_params > 0 {
        unsafe { read_i64_slice(param_tys_ptr) }
    } else {
        &[]
    };
    let param_tys: Vec<i64> = param_slice.to_vec();

    let call_conv = ctx.isa.default_call_conv();
    let mut sig = Signature::new(call_conv);
    for &pty in &param_tys {
        if let Some(cl_ty) = ity_to_cranelift(pty) {
            sig.params.push(AbiParam::new(cl_ty));
        }
    }
    if let Some(cl_ty) = ity_to_cranelift(ret_ty) {
        sig.returns.push(AbiParam::new(cl_ty));
    }

    let linkage = if is_main != 0 {
        Linkage::Export
    } else {
        Linkage::Local
    };
    let cl_id = ctx
        .obj_module
        .declare_function(name, linkage, &sig)
        .expect("declare function");

    ctx.func_decls.push(FuncDecl {
        cl_id,
        name: name.to_string(),
        param_tys,
        ret_ty,
        is_main: is_main != 0,
    });
}

// ---------------------------------------------------------------------------
// FFI: per-function incremental compilation (fn_begin / fn_block / fn_inst /
// fn_vow / fn_end)
// ---------------------------------------------------------------------------

// Begin accumulating a new function. Clears any prior scratch state.
//
// The scratch is also reset by `fn_end` and by the next `fn_begin`, so any
// partial state from a mid-stream `fn_block`/`fn_inst`/`fn_vow` error is
// discarded at the next `fn_begin`. Cranelift's module state is separate: if
// a previous `fn_end` failed partway, destroy the context rather than compile
// more functions into it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_clif_fn_begin(
    ctx_ptr: i64,
    func_idx: i64,
    ret_ty: i64,
    param_tys_vec: i64,
) -> i64 {
    let ctx = unsafe { &mut *(ctx_ptr as *mut ModuleContext) };
    if (func_idx as usize) >= ctx.func_decls.len() {
        eprintln!("clif_shim: func_idx {func_idx} out of range");
        return -1;
    }
    ctx.fn_scratch.reset();
    ctx.fn_scratch.began = true;
    ctx.fn_scratch.func_idx = func_idx;
    ctx.fn_scratch.ret_ty = ret_ty;
    if param_tys_vec != 0 {
        let slice = unsafe { read_i64_slice(param_tys_vec) };
        ctx.fn_scratch.param_tys.extend_from_slice(slice);
    }
    0
}

// Start a new block in the current function. Must be called before any
// `__vow_clif_fn_inst` calls for that block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_clif_fn_block(ctx_ptr: i64) -> i64 {
    let ctx = unsafe { &mut *(ctx_ptr as *mut ModuleContext) };
    let s = &mut ctx.fn_scratch;
    if !s.began {
        eprintln!("clif_shim: __vow_clif_fn_block without matching fn_begin");
        return -1;
    }
    s.block_starts.push(s.inst_ids.len() as i64);
    s.block_lengths.push(0);
    0
}

// Add an instruction to the current block.
// `ds_vec` / `args_vec` are VowVec pointers owned by the caller (the IrModule)
// — the shim reads their contents without taking ownership.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_clif_fn_inst(
    ctx_ptr: i64,
    id: i64,
    op: i64,
    ty: i64,
    dk: i64,
    dv: i64,
    dv2: i64,
    ds_vec: i64,
    args_vec: i64,
    rgn: i64,
) -> i64 {
    let ctx = unsafe { &mut *(ctx_ptr as *mut ModuleContext) };
    let args_start;
    {
        let s = &mut ctx.fn_scratch;
        if !s.began {
            eprintln!("clif_shim: __vow_clif_fn_inst without matching fn_begin");
            return -1;
        }
        // Validate we are inside a block BEFORE touching any inst arrays, so
        // an out-of-order call leaves the scratch arrays aligned.
        if let Some(last) = s.block_lengths.last_mut() {
            *last += 1;
        } else {
            eprintln!("clif_shim: __vow_clif_fn_inst before __vow_clif_fn_block");
            return -1;
        }
        s.inst_ids.push(id);
        s.inst_ops.push(op);
        s.inst_tys.push(ty);
        s.inst_dks.push(dk);
        s.inst_dvs.push(dv);
        s.inst_dv2s.push(dv2);
        s.inst_ds_ptrs.push(ds_vec);
        s.inst_rgns.push(rgn);
        args_start = s.all_args.len() as i64;
    }
    // Copy arg inst IDs into the flat all_args buffer.
    let args_len = if args_vec != 0 {
        let args = unsafe { read_i64_slice(args_vec) };
        ctx.fn_scratch.all_args.extend_from_slice(args);
        args.len() as i64
    } else {
        0
    };
    ctx.fn_scratch.arg_offsets.push(args_start);
    ctx.fn_scratch.arg_lengths.push(args_len);
    0
}

// Add a vow entry to the current function. The `blame` field (Caller vs
// Callee) is not a parameter here because the shim derives it from the IR
// opcode (`IOP_VOW_REQ` → Caller, else Callee) — see `blame_byte` below.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_clif_fn_vow(
    ctx_ptr: i64,
    id: i64,
    desc_vec: i64,
    binding_inst_ids_vec: i64,
    binding_names_vec: i64,
) -> i64 {
    let ctx = unsafe { &mut *(ctx_ptr as *mut ModuleContext) };
    if !ctx.fn_scratch.began {
        eprintln!("clif_shim: __vow_clif_fn_vow without matching fn_begin");
        return -1;
    }
    let bids: &[i64] = if binding_inst_ids_vec != 0 {
        unsafe { read_i64_slice(binding_inst_ids_vec) }
    } else {
        &[]
    };
    let bnames: &[i64] = if binding_names_vec != 0 {
        unsafe { read_i64_slice(binding_names_vec) }
    } else {
        &[]
    };
    if bids.len() != bnames.len() {
        eprintln!(
            "clif_shim: __vow_clif_fn_vow: binding_inst_ids len ({}) != binding_names len ({})",
            bids.len(),
            bnames.len()
        );
        return -1;
    }
    ctx.fn_scratch.vow_ids.push(id);
    ctx.fn_scratch.vow_desc_ptrs.push(desc_vec);
    ctx.fn_scratch.binding_counts.push(bids.len() as i64);
    ctx.fn_scratch.binding_inst_ids_all.extend_from_slice(bids);
    ctx.fn_scratch.binding_names_ptrs.extend_from_slice(bnames);
    0
}

// Finalize the current function: drive Cranelift codegen using the accumulated
// scratch, then reset the scratch (preserving allocated capacity for reuse).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_clif_fn_end(ctx_ptr: i64) -> i64 {
    let ctx = unsafe { &mut *(ctx_ptr as *mut ModuleContext) };
    let result = compile_current_function(ctx);
    ctx.fn_scratch.reset();
    result
}

// Compiles the function described by `ctx.fn_scratch` via Cranelift.
// This is the former body of the monolithic `__vow_clif_compile_function` FFI
// entry, now reading its inputs from the per-context scratch instead of
// rebuilt-per-call parameter arrays.
fn compile_current_function(ctx: &mut ModuleContext) -> i64 {
    // Guarded by `began` so an fn_end without a matching fn_begin (or a
    // duplicate fn_end) returns a clean FFI error rather than silently
    // compiling into func_idx 0 or panicking Cranelift on an unsealed block.
    if !ctx.fn_scratch.began {
        eprintln!("clif_shim: __vow_clif_fn_end without matching fn_begin");
        return -1;
    }
    let func_idx = ctx.fn_scratch.func_idx;
    let fi = func_idx as usize;
    debug_assert!(fi < ctx.func_decls.len(), "fn_begin should have rejected");
    let ret_ty = ctx.fn_scratch.ret_ty;
    // Alias scratch fields into locals so the existing body reads the same names.
    // Rust's field-disjoint borrow checking lets us keep these immutable borrows
    // alive alongside mutations of `ctx.obj_module`, `ctx.builder_ctx`, etc.
    let param_tys: &[i64] = &ctx.fn_scratch.param_tys;
    let block_starts: &[i64] = &ctx.fn_scratch.block_starts;
    let block_lengths: &[i64] = &ctx.fn_scratch.block_lengths;
    let inst_ids: &[i64] = &ctx.fn_scratch.inst_ids;
    let inst_ops: &[i64] = &ctx.fn_scratch.inst_ops;
    let inst_tys: &[i64] = &ctx.fn_scratch.inst_tys;
    let inst_dks: &[i64] = &ctx.fn_scratch.inst_dks;
    let inst_dvs: &[i64] = &ctx.fn_scratch.inst_dvs;
    let inst_dv2s: &[i64] = &ctx.fn_scratch.inst_dv2s;
    let inst_ds_ptrs: &[i64] = &ctx.fn_scratch.inst_ds_ptrs;
    let inst_rgns: &[i64] = &ctx.fn_scratch.inst_rgns;
    let all_args: &[i64] = &ctx.fn_scratch.all_args;
    let arg_offsets: &[i64] = &ctx.fn_scratch.arg_offsets;
    let arg_lengths: &[i64] = &ctx.fn_scratch.arg_lengths;
    let vow_ids: &[i64] = &ctx.fn_scratch.vow_ids;
    let vow_desc_ptrs: &[i64] = &ctx.fn_scratch.vow_desc_ptrs;
    let binding_counts: &[i64] = &ctx.fn_scratch.binding_counts;
    let binding_inst_ids_all: &[i64] = &ctx.fn_scratch.binding_inst_ids_all;
    let binding_names_ptrs: &[i64] = &ctx.fn_scratch.binding_names_ptrs;
    let nb = block_starts.len();
    let n_insts = inst_ids.len();

    // Build inst_id → block_index map
    let mut inst_block: HashMap<i64, usize> = HashMap::new();
    for bi in 0..nb {
        let start = block_starts[bi] as usize;
        let len = block_lengths[bi] as usize;
        for &iid in &inst_ids[start..start + len] {
            inst_block.insert(iid, bi);
        }
    }

    // Declare arena runtime functions.
    let mut arena_alloc_sig = ctx.obj_module.make_signature();
    arena_alloc_sig.params.push(AbiParam::new(types::I64)); // *VowArena
    arena_alloc_sig.params.push(AbiParam::new(types::I64)); // size
    arena_alloc_sig.params.push(AbiParam::new(types::I64)); // align
    arena_alloc_sig.returns.push(AbiParam::new(types::I64));
    let arena_alloc_id = ctx
        .obj_module
        .declare_function("__vow_arena_alloc", Linkage::Import, &arena_alloc_sig)
        .expect("declare __vow_arena_alloc");

    let mut arena_open_close_sig = ctx.obj_module.make_signature();
    arena_open_close_sig.params.push(AbiParam::new(types::I64)); // *VowArena
    let arena_open_id = ctx
        .obj_module
        .declare_function("__vow_arena_open", Linkage::Import, &arena_open_close_sig)
        .expect("declare __vow_arena_open");
    let arena_close_id = ctx
        .obj_module
        .declare_function("__vow_arena_close", Linkage::Import, &arena_open_close_sig)
        .expect("declare __vow_arena_close");

    let root_arena_id = ctx
        .obj_module
        .declare_data("__vow_root_arena", Linkage::Import, true, false)
        .expect("declare __vow_root_arena");
    let runtime_start_sig = ctx.obj_module.make_signature();
    let runtime_start_id = ctx
        .obj_module
        .declare_function("__vow_runtime_start", Linkage::Import, &runtime_start_sig)
        .expect("declare __vow_runtime_start");

    // Debug-only runtime functions (debug=1 or sanitize=3)
    let vow_violation_id = if ctx.mode == 1 || ctx.mode == 3 {
        let mut sig = ctx.obj_module.make_signature();
        sig.params.push(AbiParam::new(types::I32)); // vow_id
        sig.params.push(AbiParam::new(types::I8)); // blame
        sig.params.push(AbiParam::new(types::I64)); // desc_ptr
        sig.params.push(AbiParam::new(types::I64)); // bindings_ptr
        sig.params.push(AbiParam::new(types::I32)); // binding_count
        sig.params.push(AbiParam::new(types::I64)); // file_ptr
        sig.params.push(AbiParam::new(types::I32)); // offset
        Some(
            ctx.obj_module
                .declare_function("__vow_violation", Linkage::Import, &sig)
                .expect("declare vow_violation"),
        )
    } else {
        None
    };
    let overflow_id = if ctx.mode == 1 || ctx.mode == 3 {
        let sig = ctx.obj_module.make_signature();
        Some(
            ctx.obj_module
                .declare_function("__vow_arithmetic_overflow", Linkage::Import, &sig)
                .expect("declare overflow"),
        )
    } else {
        None
    };

    // Trace runtime functions
    let trace_enter_id = if ctx.trace_mode != 0 {
        let mut sig = ctx.obj_module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        Some(
            ctx.obj_module
                .declare_function("__vow_trace_enter", Linkage::Import, &sig)
                .expect("declare trace_enter"),
        )
    } else {
        None
    };
    let trace_exit_id = if ctx.trace_mode != 0 {
        let mut sig = ctx.obj_module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        Some(
            ctx.obj_module
                .declare_function("__vow_trace_exit", Linkage::Import, &sig)
                .expect("declare trace_exit"),
        )
    } else {
        None
    };
    let trace_vow_id = if ctx.trace_mode >= 2 && (ctx.mode == 1 || ctx.mode == 3) {
        let mut sig = ctx.obj_module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        Some(
            ctx.obj_module
                .declare_function("__vow_trace_vow", Linkage::Import, &sig)
                .expect("declare trace_vow"),
        )
    } else {
        None
    };

    // Profile runtime functions (mode=2)
    let profile_enter_id = if ctx.mode == 2 {
        let mut sig = ctx.obj_module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        Some(
            ctx.obj_module
                .declare_function("__vow_profile_enter", Linkage::Import, &sig)
                .expect("declare profile_enter"),
        )
    } else {
        None
    };
    let profile_init_id = if ctx.mode == 2 {
        let sig = ctx.obj_module.make_signature();
        Some(
            ctx.obj_module
                .declare_function("__vow_profile_init", Linkage::Import, &sig)
                .expect("declare profile_init"),
        )
    } else {
        None
    };

    // Build function signature
    let call_conv = ctx.isa.default_call_conv();
    let mut sig = Signature::new(call_conv);
    for &pty in param_tys {
        if let Some(cl_ty) = ity_to_cranelift(pty) {
            sig.params.push(AbiParam::new(cl_ty));
        }
    }
    if let Some(cl_ty) = ity_to_cranelift(ret_ty) {
        sig.returns.push(AbiParam::new(cl_ty));
    }

    let cl_func_id = ctx.func_decls[fi].cl_id;
    let mut cl_ctx = ctx.obj_module.make_context();
    cl_ctx.func.signature = sig;

    let mut builder = FunctionBuilder::new(&mut cl_ctx.func, &mut ctx.builder_ctx);

    // Create Cranelift blocks
    let mut cl_blocks: Vec<Block> = Vec::new();
    for _ in 0..nb {
        cl_blocks.push(builder.create_block());
    }

    // Entry block: add function params
    if nb > 0 {
        builder.append_block_params_for_function_params(cl_blocks[0]);
    }

    // Declare func refs for function-to-function calls
    let mut ir_func_idx_to_ref: HashMap<i64, FuncRef> = HashMap::new();
    for (idx, decl) in ctx.func_decls.iter().enumerate() {
        let fref = ctx
            .obj_module
            .declare_func_in_func(decl.cl_id, builder.func);
        ir_func_idx_to_ref.insert(idx as i64, fref);
    }

    // Declare string global values
    let mut string_global_values: HashMap<i64, GlobalValue> = HashMap::new();
    for (idx, &data_id) in ctx.string_data_ids.iter().enumerate() {
        let gv = ctx.obj_module.declare_data_in_func(data_id, builder.func);
        string_global_values.insert(idx as i64, gv);
    }

    // Declare extern function refs
    let mut extern_func_refs: HashMap<String, FuncRef> = HashMap::new();
    for (sym, &cl_id) in &ctx.extern_func_ids {
        let fref = ctx.obj_module.declare_func_in_func(cl_id, builder.func);
        extern_func_refs.insert(sym.clone(), fref);
    }

    let arena_alloc_ref = ctx
        .obj_module
        .declare_func_in_func(arena_alloc_id, builder.func);
    let arena_open_ref = ctx
        .obj_module
        .declare_func_in_func(arena_open_id, builder.func);
    let arena_close_ref = ctx
        .obj_module
        .declare_func_in_func(arena_close_id, builder.func);
    let root_arena_gv = ctx
        .obj_module
        .declare_data_in_func(root_arena_id, builder.func);
    // Per-block VowArena stack-slot map. Lazily populated on first use of
    // a given BlockId by `IOP_REGION_OPEN` / `IOP_REGION_CLOSE` /
    // `IOP_REGION_ALLOC` with `REGION_KIND_BLOCK`. `VowArena` is 48 bytes,
    // 8-byte aligned (asserted in `vow-runtime/src/lib.rs`).
    //
    // BTreeMap (not HashMap) for the same reason `slot_map` below uses one
    // (CLAUDE.md): deterministic iteration order is required for binary
    // fixed-point reproducibility under the bootstrap triple. Today the
    // map is only accessed via `.entry()` so the codegen order is
    // incidentally deterministic, but any future iteration over it
    // (diagnostics, init pass) MUST preserve that property by default.
    let mut block_arena_slots: std::collections::BTreeMap<i64, StackSlot> =
        std::collections::BTreeMap::new();
    let vow_violation_ref =
        vow_violation_id.map(|id| ctx.obj_module.declare_func_in_func(id, builder.func));
    let overflow_ref = overflow_id.map(|id| ctx.obj_module.declare_func_in_func(id, builder.func));
    let trace_enter_ref =
        trace_enter_id.map(|id| ctx.obj_module.declare_func_in_func(id, builder.func));
    let trace_exit_ref =
        trace_exit_id.map(|id| ctx.obj_module.declare_func_in_func(id, builder.func));
    let trace_vow_ref =
        trace_vow_id.map(|id| ctx.obj_module.declare_func_in_func(id, builder.func));
    let profile_enter_ref =
        profile_enter_id.map(|id| ctx.obj_module.declare_func_in_func(id, builder.func));
    let profile_init_ref =
        profile_init_id.map(|id| ctx.obj_module.declare_func_in_func(id, builder.func));

    // Sanitize init (mode==3 only, main function only)
    let sanitize_init_ref = if ctx.mode == 3 && ctx.func_decls[fi].is_main {
        let sig = ctx.obj_module.make_signature();
        let id = ctx
            .obj_module
            .declare_function("__vow_sanitize_init", Linkage::Import, &sig)
            .expect("declare sanitize_init");
        Some(ctx.obj_module.declare_func_in_func(id, builder.func))
    } else {
        None
    };

    // Stack guard init (always, main function only)
    let stack_guard_init_ref = if ctx.func_decls[fi].is_main {
        let sig = ctx.obj_module.make_signature();
        let id = ctx
            .obj_module
            .declare_function("__vow_init_stack_guard", Linkage::Import, &sig)
            .expect("declare stack_guard_init");
        Some(ctx.obj_module.declare_func_in_func(id, builder.func))
    } else {
        None
    };

    // Stack depth tracking (debug/sanitize mode only)
    let stack_enter_ref = if ctx.mode == 1 || ctx.mode == 3 {
        let mut sig = ctx.obj_module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        let id = ctx
            .obj_module
            .declare_function("__vow_stack_enter", Linkage::Import, &sig)
            .expect("declare stack_enter");
        Some(ctx.obj_module.declare_func_in_func(id, builder.func))
    } else {
        None
    };
    let stack_exit_ref = if ctx.mode == 1 || ctx.mode == 3 {
        let sig = ctx.obj_module.make_signature();
        let id = ctx
            .obj_module
            .declare_function("__vow_stack_exit", Linkage::Import, &sig)
            .expect("declare stack_exit");
        Some(ctx.obj_module.declare_func_in_func(id, builder.func))
    } else {
        None
    };

    // Create function name data section for trace/profile/stack instrumentation
    let fn_name_gv = if ctx.trace_mode != 0 || ctx.mode == 2 || ctx.mode == 1 || ctx.mode == 3 {
        let name = &ctx.func_decls[fi].name;
        let mut name_bytes = name.as_bytes().to_vec();
        name_bytes.push(0);
        let mut desc = DataDescription::new();
        desc.define(name_bytes.into_boxed_slice());
        let data_id = ctx
            .obj_module
            .declare_anonymous_data(false, false)
            .expect("fn name data");
        ctx.obj_module
            .define_data(data_id, &desc)
            .expect("define fn name");
        Some(ctx.obj_module.declare_data_in_func(data_id, builder.func))
    } else {
        None
    };

    // Build inst_id → ty map for all instructions (for vow checks)
    let mut inst_ty_map: HashMap<i64, i64> = HashMap::new();
    for ii in 0..n_insts {
        inst_ty_map.insert(inst_ids[ii], inst_tys[ii]);
    }

    // Create vow description data sections (debug/sanitize mode)
    let mut vow_desc_gvs: HashMap<i64, GlobalValue> = HashMap::new();
    // We don't have file info from the self-hosted IR, so skip file/offset vow metadata
    if ctx.mode == 1 || ctx.mode == 3 {
        for (vi, &vow_id) in vow_ids.iter().enumerate() {
            let desc_str = unsafe { read_vow_string(vow_desc_ptrs[vi]) };
            let mut bytes = desc_str.as_bytes().to_vec();
            bytes.push(0);
            let mut desc = DataDescription::new();
            desc.define(bytes.into_boxed_slice());
            let data_id = ctx
                .obj_module
                .declare_anonymous_data(false, false)
                .expect("declare vow desc");
            ctx.obj_module
                .define_data(data_id, &desc)
                .expect("define vow desc");
            let gv = ctx.obj_module.declare_data_in_func(data_id, builder.func);
            vow_desc_gvs.insert(vow_id, gv);
        }
    }

    // Build binding info for vow entries
    struct VowBindingInfo {
        name_gv: GlobalValue,
        inst_id: i64,
    }
    let mut vow_bindings: HashMap<i64, Vec<VowBindingInfo>> = HashMap::new();
    if ctx.mode == 1 || ctx.mode == 3 {
        let mut bind_offset = 0usize;
        for (vi, &vow_id) in vow_ids.iter().enumerate() {
            let bc = binding_counts[vi] as usize;
            let mut bindings = Vec::new();
            for bi in 0..bc {
                let name_str = unsafe { read_vow_string(binding_names_ptrs[bind_offset + bi]) };
                let mut name_bytes = name_str.as_bytes().to_vec();
                name_bytes.push(0);
                let mut name_desc = DataDescription::new();
                name_desc.define(name_bytes.into_boxed_slice());
                let name_data_id = ctx
                    .obj_module
                    .declare_anonymous_data(false, false)
                    .expect("declare binding name");
                ctx.obj_module
                    .define_data(name_data_id, &name_desc)
                    .expect("define binding name");
                let name_gv = ctx
                    .obj_module
                    .declare_data_in_func(name_data_id, builder.func);
                bindings.push(VowBindingInfo {
                    name_gv,
                    inst_id: binding_inst_ids_all[bind_offset + bi],
                });
            }
            vow_bindings.insert(vow_id, bindings);
            bind_offset += bc;
        }
    }

    // Set up entry block arg values
    let mut arg_values: HashMap<i64, Value> = HashMap::new(); // arg_index → Value
    if nb > 0 {
        builder.switch_to_block(cl_blocks[0]);
        let entry_params = builder.block_params(cl_blocks[0]).to_vec();
        let mut cl_idx = 0usize;
        for (ir_idx, &pty) in param_tys.iter().enumerate() {
            if ity_to_cranelift(pty).is_some() {
                if cl_idx < entry_params.len() {
                    arg_values.insert(ir_idx as i64, entry_params[cl_idx]);
                }
                cl_idx += 1;
            }
        }
    }

    let mut value_map: HashMap<i64, Value> = HashMap::new();

    // Find instruction IDs that are referenced cross-block.
    // These use stack slots to bypass SSA dominance requirements, since the
    // self-hosted IR lowerer produces cross-block references between sibling
    // branches (valid for C codegen but not SSA).
    let mut cross_block_refs: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for bi in 0..nb {
        let start = block_starts[bi] as usize;
        let len = block_lengths[bi] as usize;
        for ii in start..start + len {
            let aoff = arg_offsets[ii] as usize;
            let alen = arg_lengths[ii] as usize;
            for ai in 0..alen {
                let arg_id = all_args[aoff + ai];
                if inst_block.get(&arg_id).is_some_and(|&b| b != bi) {
                    cross_block_refs.insert(arg_id);
                }
            }
            // Phi nodes need cross-block storage (fed by Upsilons from other blocks)
            if inst_ops[ii] == IOP_PHI {
                cross_block_refs.insert(inst_ids[ii]);
            }
        }
    }

    // Allocate stack slots for cross-block referenced values.
    // Each slot holds one i64 (8 bytes), which is sufficient for all Vow types.
    let mut slot_map: BTreeMap<i64, StackSlot> = BTreeMap::new();
    for &iid in &inst_ids[..n_insts] {
        if cross_block_refs.contains(&iid) {
            let slot = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                8,
                3, // align to 8 bytes (2^3)
            ));
            slot_map.insert(iid, slot);
        }
    }

    // Zero-initialize all stack slots to match C's typical behavior (GCC
    // zero-initializes locals). The self-hosted IR has uninitialized cross-block
    // refs that happen to work in C codegen due to this.
    if nb > 0 {
        let zero = builder.ins().iconst(types::I64, 0);
        for &slot in slot_map.values() {
            builder.ins().stack_store(zero, slot, 0);
        }
        // Emit stack_guard_init at main entry (all modes)
        if ctx.func_decls[fi].is_main {
            let runtime_start_ref = ctx
                .obj_module
                .declare_func_in_func(runtime_start_id, builder.func);
            builder.ins().call(runtime_start_ref, &[]);
        }
        if let Some(init_ref) = stack_guard_init_ref {
            builder.ins().call(init_ref, &[]);
        }
        // Emit trace_enter at function entry
        if let (Some(gv), Some(enter_ref)) = (fn_name_gv, trace_enter_ref) {
            let name_ptr = builder.ins().global_value(types::I64, gv);
            builder.ins().call(enter_ref, &[name_ptr]);
        }
        // Emit profile_init in main, profile_enter at all function entries
        if ctx.func_decls[fi].is_main
            && let Some(init_ref) = profile_init_ref
        {
            builder.ins().call(init_ref, &[]);
        }
        if let (Some(gv), Some(prof_ref)) = (fn_name_gv, profile_enter_ref) {
            let name_ptr = builder.ins().global_value(types::I64, gv);
            builder.ins().call(prof_ref, &[name_ptr]);
        }
        // Emit stack_enter at function entry (debug/sanitize mode)
        if let (Some(gv), Some(se_ref)) = (fn_name_gv, stack_enter_ref) {
            let name_ptr = builder.ins().global_value(types::I64, gv);
            builder.ins().call(se_ref, &[name_ptr]);
        }
        // Emit sanitize_init at main entry
        if let Some(init_ref) = sanitize_init_ref {
            builder.ins().call(init_ref, &[]);
        }
    }
    // Emit blocks
    let mut first_block = true;
    for bi in 0..nb {
        let cl_block = cl_blocks[bi];
        if !first_block {
            builder.switch_to_block(cl_block);
        }
        first_block = false;

        let start = block_starts[bi] as usize;
        let len = block_lengths[bi] as usize;
        for ii in start..start + len {
            let iid = inst_ids[ii];
            let op = inst_ops[ii];
            let ity = inst_tys[ii];
            let dk = inst_dks[ii];
            let dv = inst_dvs[ii];
            let dv2 = inst_dv2s[ii];
            let aoff = arg_offsets[ii] as usize;
            let alen = arg_lengths[ii] as usize;

            // Pre-resolve cross-block arg references via stack slot loads.
            // Always reload from stack for slotted args (value_map may hold
            // a stale SSA value from a non-dominating block).
            // After loading I64 from the slot, narrow to the arg's original type.
            for ai in 0..alen {
                let arg_inst_id = all_args[aoff + ai];
                if let Some(&slot) = slot_map.get(&arg_inst_id) {
                    let raw = builder.ins().stack_load(types::I64, slot, 0);
                    let orig_ty = inst_ty_map
                        .get(&arg_inst_id)
                        .and_then(|&t| ity_to_cranelift(t))
                        .unwrap_or(types::I64);
                    let val = match orig_ty {
                        types::I32 => builder.ins().ireduce(types::I32, raw),
                        types::I8 => builder.ins().ireduce(types::I8, raw),
                        _ => raw,
                    };
                    value_map.insert(arg_inst_id, val);
                }
            }

            macro_rules! arg {
                ($i:expr) => {{
                    let arg_inst_id = all_args[aoff + $i];
                    match value_map.get(&arg_inst_id) {
                        Some(&v) => v,
                        None => panic!(
                            "clif shim: value_map miss: inst_id={} references arg_inst_id={} (op={}, block={}, inst_idx={}, func_idx={})",
                            iid, arg_inst_id, op, bi, ii, func_idx
                        ),
                    }
                }};
            }

            macro_rules! set_val {
                ($id:expr, $val:expr) => {{
                    let id_ = $id;
                    let val_ = $val;
                    let src_ty = builder.func.dfg.value_type(val_);
                    // Widen i8 (booleans from icmp/fcmp/const_bool) to i64 so
                    // value_map always holds i64 for booleans, matching slot loads.
                    let norm = match src_ty {
                        types::I8 => builder.ins().uextend(types::I64, val_),
                        _ => val_,
                    };
                    value_map.insert(id_, norm);
                    if let Some(&slot) = slot_map.get(&id_) {
                        let store_val = match builder.func.dfg.value_type(norm) {
                            types::I32 => builder.ins().sextend(types::I64, norm),
                            _ => norm,
                        };
                        builder.ins().stack_store(store_val, slot, 0);
                    }
                }};
            }

            match op {
                IOP_CONST_I32 => {
                    if dk == IDATA_CONST_I32 {
                        let val = builder.ins().iconst(types::I32, dv as i32 as i64);
                        set_val!(iid, val);
                    }
                }
                IOP_CONST_I64 => {
                    if dk == IDATA_CONST_I64 {
                        let val = builder.ins().iconst(types::I64, dv);
                        set_val!(iid, val);
                    }
                }
                IOP_CONST_F32 => {
                    if dk == IDATA_CONST_F32 {
                        let val = builder.ins().f32const(f32::from_bits(dv as u32));
                        set_val!(iid, val);
                    }
                }
                IOP_CONST_F64 => {
                    if dk == IDATA_CONST_F64 {
                        let val = builder.ins().f64const(f64::from_bits(dv as u64));
                        set_val!(iid, val);
                    }
                }
                IOP_CONST_BOOL => {
                    let b = if dk == IDATA_CONST_BOOL { dv } else { 0 };
                    let val = builder.ins().iconst(types::I64, b);
                    set_val!(iid, val);
                }
                IOP_CONST_STR => {
                    if dk == IDATA_CONST_STR {
                        let str_idx = dv;
                        if let Some(&gv) = string_global_values.get(&str_idx) {
                            let ptr = builder.ins().global_value(types::I64, gv);
                            set_val!(iid, ptr);
                        } else {
                            let null = builder.ins().iconst(types::I64, 0);
                            set_val!(iid, null);
                        }
                    }
                }
                IOP_CONST_UNIT => {
                    let val = builder.ins().iconst(types::I32, 0);
                    set_val!(iid, val);
                }
                IOP_GET_ARG => {
                    if dk == IDATA_ARG_INDEX {
                        let idx = dv;
                        let val = if let Some(&v) = arg_values.get(&idx) {
                            v
                        } else {
                            builder.ins().iconst(types::I32, 0)
                        };
                        set_val!(iid, val);
                    }
                }

                // Wrapping arithmetic
                IOP_WADD_I32 | IOP_WADD_I64 => {
                    let val = builder.ins().iadd(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_WSUB_I32 | IOP_WSUB_I64 => {
                    let val = builder.ins().isub(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_WMUL_I32 | IOP_WMUL_I64 => {
                    let val = builder.ins().imul(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_WDIV_I32 | IOP_WDIV_I64 => {
                    let val = builder.ins().sdiv(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_WREM_I32 | IOP_WREM_I64 => {
                    let val = builder.ins().srem(arg!(0), arg!(1));
                    set_val!(iid, val);
                }

                // Checked arithmetic
                IOP_CADD_I32 | IOP_CADD_I64 => {
                    let (result, overflow) = builder.ins().sadd_overflow(arg!(0), arg!(1));
                    emit_overflow_check(&mut builder, overflow, overflow_ref);
                    set_val!(iid, result);
                }
                IOP_CSUB_I32 | IOP_CSUB_I64 => {
                    let (result, overflow) = builder.ins().ssub_overflow(arg!(0), arg!(1));
                    emit_overflow_check(&mut builder, overflow, overflow_ref);
                    set_val!(iid, result);
                }
                IOP_CMUL_I32 | IOP_CMUL_I64 => {
                    let (result, overflow) = builder.ins().smul_overflow(arg!(0), arg!(1));
                    emit_overflow_check(&mut builder, overflow, overflow_ref);
                    set_val!(iid, result);
                }
                IOP_CDIV_I32 | IOP_CDIV_I64 => {
                    let cl_ty = ity_to_cranelift(ity).unwrap_or(types::I64);
                    let zero = builder.ins().iconst(cl_ty, 0);
                    let is_zero = builder.ins().icmp(IntCC::Equal, arg!(1), zero);
                    emit_overflow_check(&mut builder, is_zero, overflow_ref);
                    let val = builder.ins().sdiv(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_CREM_I32 | IOP_CREM_I64 => {
                    let cl_ty = ity_to_cranelift(ity).unwrap_or(types::I64);
                    let zero = builder.ins().iconst(cl_ty, 0);
                    let is_zero = builder.ins().icmp(IntCC::Equal, arg!(1), zero);
                    emit_overflow_check(&mut builder, is_zero, overflow_ref);
                    let val = builder.ins().srem(arg!(0), arg!(1));
                    set_val!(iid, val);
                }

                // Integer comparisons
                IOP_EQ_I32 | IOP_EQ_I64 => {
                    let val = builder.ins().icmp(IntCC::Equal, arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_NE_I32 | IOP_NE_I64 => {
                    let val = builder.ins().icmp(IntCC::NotEqual, arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_LT_I32 | IOP_LT_I64 => {
                    let val = builder.ins().icmp(IntCC::SignedLessThan, arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_LE_I32 | IOP_LE_I64 => {
                    let val = builder
                        .ins()
                        .icmp(IntCC::SignedLessThanOrEqual, arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_GT_I32 | IOP_GT_I64 => {
                    let val = builder
                        .ins()
                        .icmp(IntCC::SignedGreaterThan, arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_GE_I32 | IOP_GE_I64 => {
                    let val = builder
                        .ins()
                        .icmp(IntCC::SignedGreaterThanOrEqual, arg!(0), arg!(1));
                    set_val!(iid, val);
                }

                // Float arithmetic
                IOP_ADD_F32 | IOP_ADD_F64 => {
                    let val = builder.ins().fadd(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_SUB_F32 | IOP_SUB_F64 => {
                    let val = builder.ins().fsub(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_MUL_F32 | IOP_MUL_F64 => {
                    let val = builder.ins().fmul(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_DIV_F32 | IOP_DIV_F64 => {
                    let val = builder.ins().fdiv(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_REM_F32 | IOP_REM_F64 => {
                    eprintln!("clif_shim: float remainder not supported");
                    return -1;
                }

                // Float comparisons
                IOP_EQ_F32 | IOP_EQ_F64 => {
                    let val = builder.ins().fcmp(FloatCC::Equal, arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_NE_F32 | IOP_NE_F64 => {
                    let val = builder.ins().fcmp(FloatCC::NotEqual, arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_LT_F32 | IOP_LT_F64 => {
                    let val = builder.ins().fcmp(FloatCC::LessThan, arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_LE_F32 | IOP_LE_F64 => {
                    let val = builder
                        .ins()
                        .fcmp(FloatCC::LessThanOrEqual, arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_GT_F32 | IOP_GT_F64 => {
                    let val = builder.ins().fcmp(FloatCC::GreaterThan, arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_GE_F32 | IOP_GE_F64 => {
                    let val = builder
                        .ins()
                        .fcmp(FloatCC::GreaterThanOrEqual, arg!(0), arg!(1));
                    set_val!(iid, val);
                }

                // Boolean
                IOP_NOT => {
                    let one = builder.ins().iconst(types::I64, 1);
                    let val = builder.ins().bxor(arg!(0), one);
                    set_val!(iid, val);
                }
                IOP_AND => {
                    let val = builder.ins().band(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_OR => {
                    let val = builder.ins().bor(arg!(0), arg!(1));
                    set_val!(iid, val);
                }

                IOP_BITAND_I64 | IOP_BITAND_U64 => {
                    let val = builder.ins().band(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_BITOR_I64 | IOP_BITOR_U64 => {
                    let val = builder.ins().bor(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_XOR_I32 | IOP_XOR_I64 | IOP_XOR_U64 => {
                    let val = builder.ins().bxor(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_SHL_I64 | IOP_SHL_U64 => {
                    let val = builder.ins().ishl(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_SHR_I64 => {
                    let val = builder.ins().sshr(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_SHR_U64 => {
                    let val = builder.ins().ushr(arg!(0), arg!(1));
                    set_val!(iid, val);
                }

                // U64 wrapping arithmetic
                IOP_WADD_U64 => {
                    let val = builder.ins().iadd(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_WSUB_U64 => {
                    let val = builder.ins().isub(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_WMUL_U64 => {
                    let val = builder.ins().imul(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_WDIV_U64 => {
                    let val = builder.ins().udiv(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_WREM_U64 => {
                    let val = builder.ins().urem(arg!(0), arg!(1));
                    set_val!(iid, val);
                }

                // U64 checked arithmetic
                IOP_CADD_U64 => {
                    let (result, overflow) = builder.ins().uadd_overflow(arg!(0), arg!(1));
                    emit_overflow_check(&mut builder, overflow, overflow_ref);
                    set_val!(iid, result);
                }
                IOP_CSUB_U64 => {
                    let (result, overflow) = builder.ins().usub_overflow(arg!(0), arg!(1));
                    emit_overflow_check(&mut builder, overflow, overflow_ref);
                    set_val!(iid, result);
                }
                IOP_CMUL_U64 => {
                    let (result, overflow) = builder.ins().umul_overflow(arg!(0), arg!(1));
                    emit_overflow_check(&mut builder, overflow, overflow_ref);
                    set_val!(iid, result);
                }
                IOP_CDIV_U64 => {
                    let zero = builder.ins().iconst(types::I64, 0);
                    let is_zero = builder.ins().icmp(IntCC::Equal, arg!(1), zero);
                    emit_overflow_check(&mut builder, is_zero, overflow_ref);
                    let val = builder.ins().udiv(arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_CREM_U64 => {
                    let zero = builder.ins().iconst(types::I64, 0);
                    let is_zero = builder.ins().icmp(IntCC::Equal, arg!(1), zero);
                    emit_overflow_check(&mut builder, is_zero, overflow_ref);
                    let val = builder.ins().urem(arg!(0), arg!(1));
                    set_val!(iid, val);
                }

                // U64 comparisons
                IOP_EQ_U64 => {
                    let val = builder.ins().icmp(IntCC::Equal, arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_NE_U64 => {
                    let val = builder.ins().icmp(IntCC::NotEqual, arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_LT_U64 => {
                    let val = builder
                        .ins()
                        .icmp(IntCC::UnsignedLessThan, arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_LE_U64 => {
                    let val = builder
                        .ins()
                        .icmp(IntCC::UnsignedLessThanOrEqual, arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_GT_U64 => {
                    let val = builder
                        .ins()
                        .icmp(IntCC::UnsignedGreaterThan, arg!(0), arg!(1));
                    set_val!(iid, val);
                }
                IOP_GE_U64 => {
                    let val =
                        builder
                            .ins()
                            .icmp(IntCC::UnsignedGreaterThanOrEqual, arg!(0), arg!(1));
                    set_val!(iid, val);
                }

                // ConstU64
                IOP_CONST_U64 => {
                    if dk == IDATA_CONST_U64 {
                        let val = builder.ins().iconst(types::I64, dv);
                        set_val!(iid, val);
                    }
                }

                // Cast (no-op at machine level)
                IOP_CAST_I64_TO_U64 | IOP_CAST_U64_TO_I64 => {
                    let val = arg!(0);
                    set_val!(iid, val);
                }

                // Memory
                IOP_LOAD => {
                    let cl_ty = ity_to_cranelift(ity).unwrap_or(types::I64);
                    let val = builder.ins().load(cl_ty, MemFlags::new(), arg!(0), 0);
                    set_val!(iid, val);
                }
                IOP_STORE => {
                    builder.ins().store(MemFlags::new(), arg!(1), arg!(0), 0);
                    let unit = builder.ins().iconst(types::I32, 0);
                    set_val!(iid, unit);
                }

                // Control flow
                IOP_BRANCH => {
                    let cond = arg!(0);
                    if dk == IDATA_BRANCH_TARGETS {
                        let then_bi = dv as usize;
                        let else_bi = dv2 as usize;
                        let then_cl = cl_blocks[then_bi];
                        let else_cl = cl_blocks[else_bi];
                        builder.ins().brif(cond, then_cl, &[], else_cl, &[]);
                    }
                }
                IOP_JUMP => {
                    if dk == IDATA_JUMP_TARGET {
                        let target_bi = dv as usize;
                        let target_cl = cl_blocks[target_bi];
                        builder.ins().jump(target_cl, &[]);
                    }
                }
                IOP_RETURN => {
                    if let (Some(gv), Some(exit_ref)) = (fn_name_gv, trace_exit_ref) {
                        let name_ptr = builder.ins().global_value(types::I64, gv);
                        builder.ins().call(exit_ref, &[name_ptr]);
                    }
                    if let Some(se_ref) = stack_exit_ref {
                        builder.ins().call(se_ref, &[]);
                    }
                    if ret_ty == ITY_UNIT {
                        builder.ins().return_(&[]);
                    } else if alen > 0 {
                        let val_id = all_args[aoff];
                        if let Some(&val) = value_map.get(&val_id) {
                            let val = coerce_return_value(&mut builder, val, ret_ty);
                            builder.ins().return_(&[val]);
                        } else {
                            builder.ins().return_(&[]);
                        }
                    } else {
                        builder.ins().return_(&[]);
                    }
                }
                IOP_UNREACHABLE => {
                    builder.ins().trap(TrapCode::unwrap_user(2));
                }

                IOP_PHI => {}
                IOP_UPSILON => {
                    // Store to the Phi's stack slot so cross-block references work.
                    // The pre-resolve loop above already reloaded cross-block args
                    // into value_map, so value_map[val_id] is valid here.
                    if dk == IDATA_PHI_TARGET && alen > 0 {
                        let phi_id = dv;
                        let val_id = all_args[aoff];
                        if let Some(&slot) = slot_map.get(&phi_id)
                            && let Some(&val) = value_map.get(&val_id)
                        {
                            let src_ty = builder.func.dfg.value_type(val);
                            let store_val = match src_ty {
                                types::I32 => builder.ins().sextend(types::I64, val),
                                types::I8 => builder.ins().uextend(types::I64, val),
                                _ => val,
                            };
                            builder.ins().stack_store(store_val, slot, 0);
                        }
                    }
                }

                // Vow checks
                IOP_VOW_REQ | IOP_VOW_ENS | IOP_VOW_INV => {
                    if (ctx.mode == 1 || ctx.mode == 3) && alen > 0 {
                        let pred_id = all_args[aoff];
                        if let Some(&pred) = value_map.get(&pred_id) {
                            let vow_id = if dk == IDATA_VOW_ID { dv } else { 0 };
                            let blame_byte: i64 = if op == IOP_VOW_REQ { 0 } else { 1 };

                            // Collect captures
                            let captures: Vec<(GlobalValue, Value, i64)> =
                                if let Some(bindings) = vow_bindings.get(&vow_id) {
                                    bindings
                                        .iter()
                                        .filter_map(|b| {
                                            let ir_ty = *inst_ty_map.get(&b.inst_id)?;
                                            if matches!(ir_ty, ITY_PTR | ITY_LPTR | ITY_UNIT) {
                                                return None;
                                            }
                                            let cl_val = *value_map.get(&b.inst_id)?;
                                            Some((b.name_gv, cl_val, ir_ty))
                                        })
                                        .collect()
                                } else {
                                    vec![]
                                };

                            emit_vow_check(
                                &mut builder,
                                pred,
                                vow_id,
                                blame_byte,
                                &captures,
                                vow_violation_ref,
                                &vow_desc_gvs,
                                trace_vow_ref,
                                fn_name_gv,
                            );
                        }
                    }
                }

                // Function calls
                IOP_CALL => {
                    let func_ref = match dk {
                        IDATA_CALL_TARGET => {
                            let target_idx = dv;
                            if let Some(&fr) = ir_func_idx_to_ref.get(&target_idx) {
                                fr
                            } else {
                                eprintln!("clif_shim: unknown call target func_idx={target_idx}");
                                return -1;
                            }
                        }
                        IDATA_CALL_EXTERN => {
                            let sym = unsafe { read_vow_string(inst_ds_ptrs[ii]) };
                            if let Some(&fr) = extern_func_refs.get(sym) {
                                fr
                            } else {
                                eprintln!("clif_shim: unknown extern symbol: {sym}");
                                return -1;
                            }
                        }
                        _ => {
                            eprintln!("clif_shim: Call without target data");
                            return -1;
                        }
                    };
                    let sig_ref = builder.func.dfg.ext_funcs[func_ref].signature;
                    let expected_types: Vec<types::Type> = builder.func.dfg.signatures[sig_ref]
                        .params
                        .iter()
                        .map(|p| p.value_type)
                        .collect();
                    let call_args: Vec<Value> = (0..alen)
                        .map(|i| {
                            let arg_id = all_args[aoff + i];
                            let v = *value_map.get(&arg_id).unwrap_or_else(|| {
                                panic!(
                                    "clif shim: IOP_CALL value_map miss: inst_id={iid} arg_id={arg_id} (block={bi}, inst_idx={ii}, func_idx={func_idx})"
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
                        set_val!(iid, unit);
                    } else {
                        set_val!(iid, results[0]);
                    }
                }

                // Debug calls (only emitted in debug/sanitize mode)
                IOP_DEBUG_CALL => {
                    if (ctx.mode == 1 || ctx.mode == 3) && dk == IDATA_CALL_EXTERN {
                        let sym = unsafe { read_vow_string(inst_ds_ptrs[ii]) };
                        if let Some(&fr) = extern_func_refs.get(sym) {
                            let sig_ref = builder.func.dfg.ext_funcs[fr].signature;
                            let expected_types: Vec<types::Type> = builder.func.dfg.signatures
                                [sig_ref]
                                .params
                                .iter()
                                .map(|p| p.value_type)
                                .collect();
                            let call_args: Vec<Value> = (0..alen)
                                .map(|i| {
                                    let arg_id = all_args[aoff + i];
                                    let v = *value_map.get(&arg_id).unwrap_or_else(|| {
                                        panic!(
                                            "clif shim: IOP_DEBUG_CALL value_map miss: inst_id={iid} arg_id={arg_id}"
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
                            builder.ins().call(fr, &call_args);
                        }
                    }
                    let unit = builder.ins().iconst(types::I32, 0);
                    set_val!(iid, unit);
                }

                // Region / linear
                IOP_REGION_ALLOC => {
                    let rgn = inst_rgns[ii];
                    let kind = rgn & 3;
                    let payload = rgn >> 2;
                    let (size, align) = if dk == IDATA_ALLOC_SIZE {
                        (dv, dv2)
                    } else {
                        (0, 8)
                    };
                    let arena = match kind {
                        REGION_KIND_ROOT => builder.ins().global_value(types::I64, root_arena_gv),
                        REGION_KIND_BLOCK => {
                            let slot = *block_arena_slots.entry(payload).or_insert_with(|| {
                                builder.create_sized_stack_slot(StackSlotData::new(
                                    StackSlotKind::ExplicitSlot,
                                    48, // sizeof VowArena
                                    3,  // log2(8)
                                ))
                            });
                            builder.ins().stack_addr(types::I64, slot, 0)
                        }
                        REGION_KIND_CALLER => {
                            // Hidden-region plumbing (spec §5.2) is not yet
                            // wired in the shim — `__vow_clif_declare_function`
                            // does not accept a hidden-param count, so the
                            // self-hosted compiler cannot emit any
                            // `Caller`-routed allocation regardless of `k`.
                            // Reject all `k` values rather than silently
                            // routing into the root arena. Phase 5 (#200)
                            // extends the declare API and lifts this reject.
                            eprintln!(
                                "clif_shim: IOP_REGION_ALLOC with REGION_KIND_CALLER \
                                 (k={}) is not yet wired — self-hosted compiler must \
                                 not emit Caller-routed allocations until hidden-param \
                                 plumbing lands",
                                payload,
                            );
                            return -1;
                        }
                        REGION_KIND_RODATA => {
                            eprintln!(
                                "clif_shim: IOP_REGION_ALLOC with REGION_KIND_RODATA \
                                 is invalid — rodata-backed values are static, not \
                                 allocated"
                            );
                            return -1;
                        }
                        _ => {
                            eprintln!(
                                "clif_shim: IOP_REGION_ALLOC with unknown region kind {kind}"
                            );
                            return -1;
                        }
                    };
                    let size_val = builder.ins().iconst(types::I64, size);
                    let align_val = builder.ins().iconst(types::I64, align);
                    let call_inst = builder
                        .ins()
                        .call(arena_alloc_ref, &[arena, size_val, align_val]);
                    let ptr = builder.inst_results(call_inst)[0];
                    set_val!(iid, ptr);
                }
                IOP_REGION_FREE => {
                    let unit = builder.ins().iconst(types::I32, 0);
                    set_val!(iid, unit);
                }
                IOP_LINEAR_CONSUME | IOP_LINEAR_BORROW => {
                    let unit = builder.ins().iconst(types::I32, 0);
                    set_val!(iid, unit);
                }

                // RegionOpen / RegionClose: bracket a block-region's lifetime
                // by calling `__vow_arena_open` / `__vow_arena_close` on the
                // BlockId-keyed stack slot. The region payload (encoded in
                // `inst_rgns[ii]`) names the block being opened/closed, which
                // need not match the containing IR block — opens/closes can
                // straddle basic-block boundaries.
                IOP_REGION_OPEN | IOP_REGION_CLOSE => {
                    let rgn = inst_rgns[ii];
                    let kind = rgn & 3;
                    if kind != REGION_KIND_BLOCK {
                        eprintln!(
                            "clif_shim: IOP_REGION_{{OPEN,CLOSE}} requires \
                             REGION_KIND_BLOCK, got kind {kind}"
                        );
                        return -1;
                    }
                    let payload = rgn >> 2;
                    let slot = *block_arena_slots.entry(payload).or_insert_with(|| {
                        builder.create_sized_stack_slot(StackSlotData::new(
                            StackSlotKind::ExplicitSlot,
                            48,
                            3,
                        ))
                    });
                    let arena_addr = builder.ins().stack_addr(types::I64, slot, 0);
                    let func_ref = if op == IOP_REGION_OPEN {
                        arena_open_ref
                    } else {
                        arena_close_ref
                    };
                    builder.ins().call(func_ref, &[arena_addr]);
                    let unit = builder.ins().iconst(types::I32, 0);
                    set_val!(iid, unit);
                }

                // Struct / enum field access
                IOP_FIELD_GET => {
                    if dk == IDATA_FIELD {
                        let idx = dv;
                        let base = arg!(0);
                        let offset = (idx as i32) * 8;
                        let raw = builder
                            .ins()
                            .load(types::I64, MemFlags::trusted(), base, offset);
                        let result = match ity_to_cranelift(ity) {
                            Some(types::I64) | None => raw,
                            Some(types::I32) => builder.ins().ireduce(types::I32, raw),
                            Some(types::I8) => builder.ins().ireduce(types::I8, raw),
                            Some(types::F64) => {
                                builder.ins().bitcast(types::F64, MemFlags::new(), raw)
                            }
                            Some(types::F32) => {
                                let i32v = builder.ins().ireduce(types::I32, raw);
                                builder.ins().bitcast(types::F32, MemFlags::new(), i32v)
                            }
                            Some(other) => builder.ins().ireduce(other, raw),
                        };
                        set_val!(iid, result);
                    }
                }
                IOP_FIELD_SET => {
                    if dk == IDATA_FIELD {
                        let idx = dv;
                        let base = arg!(0);
                        let new_val = arg!(1);
                        let offset = (idx as i32) * 8;
                        let src_ty = builder.func.dfg.value_type(new_val);
                        let store_val = match src_ty {
                            types::I32 => builder.ins().sextend(types::I64, new_val),
                            types::I8 => builder.ins().uextend(types::I64, new_val),
                            types::F32 => {
                                let bits =
                                    builder.ins().bitcast(types::I32, MemFlags::new(), new_val);
                                builder.ins().uextend(types::I64, bits)
                            }
                            types::F64 => {
                                builder.ins().bitcast(types::I64, MemFlags::new(), new_val)
                            }
                            _ => new_val,
                        };
                        builder
                            .ins()
                            .store(MemFlags::trusted(), store_val, base, offset);
                        let unit = builder.ins().iconst(types::I32, 0);
                        set_val!(iid, unit);
                    }
                }

                _ => {
                    eprintln!("clif_shim: unknown opcode {op}");
                    return -1;
                }
            }
        }
    }

    builder.seal_all_blocks();
    builder.finalize();

    // Verify function for debugging
    if let Err(errs) = cranelift_codegen::verify_function(&cl_ctx.func, ctx.isa.as_ref()) {
        eprintln!("clif_shim: verifier errors for func_idx={func_idx}:");
        eprintln!("{errs}");
        eprintln!("--- CLIF IR ---");
        eprintln!("{}", cl_ctx.func.display());
        eprintln!("--- END CLIF IR ---");
        return -1;
    }
    if let Err(e) = ctx.obj_module.define_function(cl_func_id, &mut cl_ctx) {
        eprintln!("clif_shim: define_function error: {e:?}");
        return -1;
    }
    ctx.obj_module.clear_context(&mut cl_ctx);

    0
}

// ---------------------------------------------------------------------------
// FFI: finish — emit object file
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_clif_finish(ctx_ptr: i64, obj_path_ptr: i64) -> i64 {
    let ctx = unsafe { *Box::from_raw(ctx_ptr as *mut ModuleContext) };
    let obj_path = unsafe { read_vow_string(obj_path_ptr) };

    let product = ctx.obj_module.finish();
    let bytes = match product.emit() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("clif_shim: emit error: {e}");
            return -1;
        }
    };

    if let Err(e) = std::fs::write(obj_path, &bytes) {
        eprintln!("clif_shim: write {obj_path}: {e}");
        return -1;
    }

    0
}

// ---------------------------------------------------------------------------
// FFI: link — invoke cc to link object + runtime into executable
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_clif_link(obj_path_ptr: i64, output_path_ptr: i64) -> i64 {
    let obj_path = unsafe { read_vow_string(obj_path_ptr) };
    let output_path = unsafe { read_vow_string(output_path_ptr) };

    let runtime_lib = find_lib("libvow_runtime.a");
    let shim_lib = find_lib("libvow_clif_shim.a");

    let mut cmd = std::process::Command::new("cc");
    cmd.arg(obj_path);
    if let Some(ref rt) = runtime_lib {
        cmd.arg(rt);
    } else {
        eprintln!("clif_shim: warning: could not find libvow_runtime.a");
    }
    if let Some(ref sl) = shim_lib {
        cmd.arg(sl);
    }
    cmd.arg("-o").arg(output_path);
    cmd.args(["-lpthread", "-ldl", "-lm"]);

    match cmd.status() {
        Ok(s) if s.success() => {
            let _ = std::fs::remove_file(obj_path);
            0
        }
        Ok(s) => {
            eprintln!("clif_shim: cc exited with {s}");
            -1
        }
        Err(e) => {
            eprintln!("clif_shim: failed to invoke cc: {e}");
            -1
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn find_lib(name: &str) -> Option<String> {
    // Check env var
    let env_key = if name.contains("runtime") {
        "VOW_RUNTIME_PATH"
    } else {
        "VOW_CLIF_SHIM_PATH"
    };
    if let Ok(p) = std::env::var(env_key)
        && std::path::Path::new(&p).exists()
    {
        return Some(p);
    }

    // Adjacent to current exe
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let p = dir.join(name);
        if p.exists() {
            return Some(p.to_string_lossy().into_owned());
        }
    }

    // Cargo target directories (for development)
    for profile in &["release", "debug"] {
        let p = format!(
            "{}/../target/{}/{}",
            env!("CARGO_MANIFEST_DIR"),
            profile,
            name
        );
        if std::path::Path::new(&p).exists() {
            return Some(p);
        }
    }

    None
}

fn emit_overflow_check(
    builder: &mut FunctionBuilder,
    overflow: Value,
    overflow_ref: Option<FuncRef>,
) {
    let trap_block = builder.create_block();
    let cont_block = builder.create_block();
    builder
        .ins()
        .brif(overflow, trap_block, &[], cont_block, &[]);

    builder.switch_to_block(trap_block);
    builder.seal_block(trap_block);
    if let Some(ov_ref) = overflow_ref {
        builder.ins().call(ov_ref, &[]);
    }
    builder.ins().trap(TrapCode::INTEGER_OVERFLOW);

    builder.switch_to_block(cont_block);
    builder.seal_block(cont_block);
}

fn tag_for_ir_ty(ty: i64) -> i64 {
    match ty {
        ITY_I32 => 0,
        ITY_I64 => 1,
        ITY_F32 => 2,
        ITY_F64 => 3,
        ITY_BOOL => 4,
        _ => 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_vow_check(
    builder: &mut FunctionBuilder,
    predicate: Value,
    vow_id: i64,
    blame: i64,
    captures: &[(GlobalValue, Value, i64)],
    vow_violation_ref: Option<FuncRef>,
    vow_desc_gvs: &HashMap<i64, GlobalValue>,
    trace_vow_ref: Option<FuncRef>,
    fn_name_gv: Option<GlobalValue>,
) {
    let one = builder.ins().iconst(types::I64, 1);
    let inv = builder.ins().bxor(predicate, one);

    let violation_block = builder.create_block();
    let cont_block = builder.create_block();
    builder
        .ins()
        .brif(inv, violation_block, &[], cont_block, &[]);

    builder.switch_to_block(violation_block);
    builder.seal_block(violation_block);
    // Trace vow failure (full mode)
    if let (Some(tv_ref), Some(gv)) = (trace_vow_ref, fn_name_gv) {
        let name_ptr = builder.ins().global_value(types::I64, gv);
        let vid = builder.ins().iconst(types::I64, vow_id);
        let passed = builder.ins().iconst(types::I64, 0);
        builder.ins().call(tv_ref, &[name_ptr, vid, passed]);
    }
    if let Some(vr) = vow_violation_ref {
        let vow_id_val = builder.ins().iconst(types::I32, vow_id);
        let blame_val = builder.ins().iconst(types::I8, blame);
        let desc_ptr = if let Some(&gv) = vow_desc_gvs.get(&vow_id) {
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
                let tag_val = builder.ins().iconst(types::I8, tag_for_ir_ty(*ir_ty));
                builder
                    .ins()
                    .stack_store(tag_val, slot, (i * 24 + 8) as i32);
                let payload: Value = match *ir_ty {
                    ITY_I32 => builder.ins().sextend(types::I64, *cl_val),
                    ITY_I64 => *cl_val,
                    ITY_F32 => {
                        let bits = builder.ins().bitcast(types::I32, MemFlags::new(), *cl_val);
                        builder.ins().uextend(types::I64, bits)
                    }
                    ITY_F64 => builder.ins().bitcast(types::I64, MemFlags::new(), *cl_val),
                    ITY_BOOL => *cl_val,
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

        // No file/offset info from self-hosted IR
        let file_ptr = builder.ins().iconst(types::I64, 0);
        let offset_val = builder.ins().iconst(types::I32, 0);

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
    builder.ins().trap(TrapCode::unwrap_user(1));

    builder.switch_to_block(cont_block);
    builder.seal_block(cont_block);
    // Trace vow pass (full mode)
    if let (Some(tv_ref), Some(gv)) = (trace_vow_ref, fn_name_gv) {
        let name_ptr = builder.ins().global_value(types::I64, gv);
        let vid = builder.ins().iconst(types::I64, vow_id);
        let passed = builder.ins().iconst(types::I64, 1);
        builder.ins().call(tv_ref, &[name_ptr, vid, passed]);
    }
}

fn coerce_return_value(builder: &mut FunctionBuilder<'_>, val: Value, ret_ty: i64) -> Value {
    let val_ty = builder.func.dfg.value_type(val);
    let target = ity_to_cranelift(ret_ty);
    match (val_ty, target) {
        (types::I64, Some(types::I32)) => builder.ins().ireduce(types::I32, val),
        (types::I64, Some(types::I8)) => builder.ins().ireduce(types::I8, val),
        (types::I32, Some(types::I64)) => builder.ins().sextend(types::I64, val),
        (types::I8, Some(types::I64)) => builder.ins().uextend(types::I64, val),
        (types::I32, Some(types::I8)) => builder.ins().ireduce(types::I8, val),
        (types::I8, Some(types::I32)) => builder.ins().uextend(types::I32, val),
        _ => val,
    }
}

fn make_extern_sig(sym: &str, obj_module: &ObjectModule) -> Signature {
    let call_conv = obj_module.isa().default_call_conv();
    let mut sig = Signature::new(call_conv);
    match sym {
        "__vow_print_str" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_print_i64" | "__vow_print_u64" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_vec_new" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_vec_new_val" => {
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_vec_len" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_vec_push_val" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_vec_get_val" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_vec_set_val" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_vec_pop" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_vec_clear" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_vec_truncate" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_string_new" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_string_from_cstr" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_string_len" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_string_clear" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_string_eq" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I8));
        }
        "__vow_string_contains" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I8));
        }
        "__vow_string_push_str" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_string_byte_at" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_string_push_byte" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_string_from_i64" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_string_print" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_fs_read" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_fs_write" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_fs_exists" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_fs_mkdir" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_fs_listdir" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_fs_remove" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_fs_remove_dir" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_fs_is_dir" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_fs_rename" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_string_substr" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_string_substring" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_string_parse_i64_opt" | "__vow_string_parse_u64_opt" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_string_split" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_string_starts_with" | "__vow_string_ends_with" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_string_trim" | "__vow_string_to_upper" | "__vow_string_to_lower" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_string_replace" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_string_join" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_parse_i64" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_vec_sort" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_time_unix" => {
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_time_unix_ms" => {
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_num_cpus" => {
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_hex_encode" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_hex_decode" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_eprintln_str" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_debug_str" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_debug_i64" | "__vow_debug_u64" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_args" => {
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_stdin_read" => {
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_stdin_read_line" => {
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_stdin_ready" => {
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_process_exit" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_process_run" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_process_get_stdout" | "__vow_process_get_stderr" => {
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_process_start" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_process_wait" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_process_wait_timeout" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_process_kill" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_process_stdout_for" | "__vow_process_stderr_for" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_string_free" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_vec_free_val" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_map_free" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_map_new" => {
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_map_insert" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_map_get" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_map_contains" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I8));
        }
        "__vow_map_remove" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_map_len" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_unwrap_panic" => {}
        // Cranelift shim FFI (for self-hosting: the binary calls back into the shim)
        "__vow_trace_enter" | "__vow_trace_exit" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_trace_vow" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_profile_enter" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_profile_init" | "__vow_init_stack_guard" | "__vow_stack_exit" => {}
        "__vow_stack_enter" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_clif_create" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_clif_add_string" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_clif_declare_extern" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
        }
        "__vow_clif_declare_function" => {
            for _ in 0..7 {
                sig.params.push(AbiParam::new(types::I64));
            }
        }
        "__vow_clif_fn_begin" => {
            // ctx, func_idx, ret_ty, param_tys_vec
            for _ in 0..4 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_clif_fn_block" => {
            // ctx
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_clif_fn_inst" => {
            // ctx, id, op, ty, dk, dv, dv2, ds_vec, args_vec, rgn
            for _ in 0..10 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_clif_fn_vow" => {
            // ctx, id, desc_vec, binding_inst_ids_vec, binding_names_vec
            for _ in 0..5 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_clif_fn_end" => {
            // ctx
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_clif_finish" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_clif_link" => {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        }
        "__vow_clif_destroy" => {
            sig.params.push(AbiParam::new(types::I64));
        }
        _ => {
            eprintln!("clif_shim: unknown extern sig for '{sym}', using no-arg no-return");
        }
    }
    sig
}
