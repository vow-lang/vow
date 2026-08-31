use std::collections::{HashMap, HashSet};

use vow_ir::{
    FuncId, Function, Inst, InstData, IntegerSignedness, IntegerType, IntegerWidth, Module, Opcode,
    Ty,
};

// ---------------------------------------------------------------------------
// Constant-function detection (for cross-function verification inlining)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ConstantValue {
    I32(i32),
    I64(i64),
    Bool(bool),
}

pub fn detect_constant_functions(module: &Module) -> HashMap<FuncId, ConstantValue> {
    let mut result = HashMap::new();
    for func in &module.functions {
        if func.blocks.len() != 1 {
            continue;
        }
        let block = &func.blocks[0];
        let non_arg: Vec<_> = block
            .insts
            .iter()
            .filter(|i| i.opcode != Opcode::GetArg)
            .collect();
        if non_arg.len() != 2 {
            continue;
        }
        let const_inst = non_arg[0];
        let ret_inst = non_arg[1];
        if ret_inst.opcode != Opcode::Return {
            continue;
        }
        if ret_inst.args.first().copied() != Some(const_inst.id) {
            continue;
        }
        let val = match (&const_inst.opcode, &const_inst.data) {
            (Opcode::ConstI32, InstData::ConstI32(v)) => ConstantValue::I32(*v),
            (Opcode::ConstI64, InstData::ConstI64(v)) => ConstantValue::I64(*v),
            (Opcode::ConstBool, InstData::ConstBool(v)) => ConstantValue::Bool(*v),
            _ => continue,
        };
        result.insert(func.id, val);
    }
    result
}

// ---------------------------------------------------------------------------
// Verification model bounds for bounded model checking
//
// ESBMC models Vec/String/HashMap/BTreeMap as fixed-size C arrays, so the model
// needs a finite capacity per collection. These are *internal verifier-model*
// parameters with safe defaults — NOT language properties and NOT user-tunable.
// The bound is a fact about the prover, not about Vow: a program that needs more
// proof power is a prover concern (a stronger or unbounded backend), never a
// language knob. Swap in an unbounded checker and the same source, contracts,
// and CLI verify unchanged. See docs/design/verifier-model-bounds.md.
// ---------------------------------------------------------------------------

/// Safe default model capacity for `Vec<T>` under bounded model checking.
pub const VEC_MODEL_CAP: usize = 128;
/// Safe default model capacity for `String` under bounded model checking. The
/// verifier transparently raises this per function when a string literal is
/// longer (see `limits_with_literal_string_capacity`).
pub const STRING_MODEL_CAP: usize = 256;
/// Safe default model capacity for `HashMap<K, V>` under bounded model checking.
pub const HASHMAP_MODEL_CAP: usize = 64;
/// Safe default model capacity for `BTreeMap<K, V>` under bounded model checking.
pub const BTREEMAP_MODEL_CAP: usize = 64;

/// Safe default slot capacity for the user-struct heap model under bounded model
/// checking. Each `RegionAlloc` bump-allocates `(n_fields + 1)` int64 slots; this
/// caps total live struct slots per verified function. Model artifact, not a
/// contract bound (mirrors `vec_max` et al.).
pub const HEAP_MODEL_CAP: usize = 1024;

// A capacity of zero would emit a zero-length C array and an unsatisfiable
// `len < CAP` assertion, silently breaking all collection verification. Pin the
// positivity invariant the (now-removed) CLI `validate_limits` used to enforce.
const _: () = assert!(
    VEC_MODEL_CAP > 0 && STRING_MODEL_CAP > 0 && HASHMAP_MODEL_CAP > 0 && BTREEMAP_MODEL_CAP > 0,
    "collection model capacities must be positive",
);

#[derive(Debug, Clone, Copy)]
pub struct VerifyLimits {
    pub max_k_step: u32,
    pub vec_max: usize,
    pub string_max: usize,
    pub hashmap_max: usize,
    pub btreemap_max: usize,
    pub heap_max: usize,
}

impl Default for VerifyLimits {
    fn default() -> Self {
        Self {
            max_k_step: 50,
            vec_max: VEC_MODEL_CAP,
            string_max: STRING_MODEL_CAP,
            hashmap_max: HASHMAP_MODEL_CAP,
            btreemap_max: BTREEMAP_MODEL_CAP,
            heap_max: HEAP_MODEL_CAP,
        }
    }
}

// ---------------------------------------------------------------------------
// Type mapping
// ---------------------------------------------------------------------------

fn ir_ty_to_c(ty: Ty) -> &'static str {
    match ty {
        Ty::I8 => "int8_t",
        Ty::U8 => "uint8_t",
        Ty::I16 => "int16_t",
        Ty::U16 => "uint16_t",
        Ty::I32 => "int32_t",
        Ty::U32 => "uint32_t",
        Ty::I64 => "int64_t",
        Ty::U64 => "uint64_t",
        Ty::I128 => "__int128",
        Ty::U128 => "unsigned __int128",
        Ty::F32 => "float",
        Ty::F64 => "double",
        Ty::Bool => "_Bool",
        Ty::Unit => "int32_t",
        Ty::Ptr | Ty::LinearPtr => "void*",
    }
}

fn ty_is_unsigned(ty: Ty) -> bool {
    matches!(ty, Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64 | Ty::U128)
}

/// Emit the bounds assert for an indexed container access (`Vec` or `String`).
///
/// The comparison is keyed off the index instruction's IR type so that the
/// emitted C says what it means. For an unsigned index `v{idx} >= 0` is
/// vacuous, and comparing it against the `int64_t` `.len` field would leave
/// the conversion to C's usual arithmetic conversions; emit a single explicit
/// comparison instead. Signed indices keep the two-sided form unchanged.
fn emit_bounds_assert(idx: u32, container: u32, idx_ty: Ty, label: &str, out: &mut String) {
    if ty_is_unsigned(idx_ty) {
        out.push_str(&format!(
            "  __ESBMC_assert(v{idx} < (uint64_t)v{container}.len, \"{label}\");\n"
        ));
    } else {
        out.push_str(&format!(
            "  __ESBMC_assert(v{idx} >= 0 && v{idx} < v{container}.len, \"{label}\");\n"
        ));
    }
}

/// IR type of the instruction producing `id`, defaulting to the signed form
/// when the operand cannot be resolved.
fn operand_ty(id: u32, inst_by_id: &HashMap<u32, &Inst>) -> Ty {
    inst_by_id.get(&id).map_or(Ty::I64, |i| i.ty)
}

// ---------------------------------------------------------------------------
// Typed variable analysis (Vec, String, HashMap)
// ---------------------------------------------------------------------------

fn is_vec_model_creator(name: &str) -> bool {
    matches!(
        name,
        "__vow_vec_new"
            | "__vow_vec_new_val"
            | "__vow_vec_new_in_arena"
            | "__vow_vec_new_val_in_arena"
            | "__vow_vec_from_raw_parts_copy_val"
            | "__vow_vec_pin_to_root_val"
    )
}

fn is_string_fresh_helper(name: &str) -> bool {
    matches!(
        name,
        "__vow_string_trim"
            | "__vow_string_trim_in_arena"
            | "__vow_string_to_upper"
            | "__vow_string_to_upper_in_arena"
            | "__vow_string_to_lower"
            | "__vow_string_to_lower_in_arena"
    )
}

fn is_string_model_creator(name: &str) -> bool {
    is_string_fresh_helper(name)
        || matches!(
            name,
            "__vow_string_new"
                | "__vow_string_new_in_arena"
                | "__vow_string_literal"
                | "__vow_string_from_cstr"
                | "__vow_string_from_cstr_in_arena"
                | "__vow_string_clone"
                | "__vow_string_clone_in_arena"
                | "__vow_string_from_raw_parts_copy"
                | "__vow_string_pin_to_root"
                | "__vow_string_substr"
                | "__vow_string_substr_in_arena"
                | "__vow_string_substring"
                | "__vow_string_substring_in_arena"
                | "__vow_string_from_i64"
                | "__vow_string_from_i64_in_arena"
                | "__vow_string_from_u64"
                | "__vow_string_from_u64_in_arena"
        )
}

fn vec_model_receiver_arg(name: &str) -> Option<usize> {
    match name {
        "__vow_vec_push_val" | "__vow_vec_get_val" | "__vow_vec_len" | "__vow_vec_pop"
        | "__vow_vec_set_val" => Some(0),
        "__vow_vec_push_in_arena"
        | "__vow_vec_push_val_in_arena"
        | "__vow_vec_reserve_in_arena" => Some(1),
        _ => None,
    }
}

/// Argument index of the value being stored into `__vow_vec_t::data[]` for
/// vec store-ops.  `__vow_vec_get_val` reads from `data[]` into the call's
/// *result* id; callers handle it via the result id instead and this
/// function returns `None` for that case.
fn vec_op_value_arg(name: &str) -> Option<usize> {
    match name {
        "__vow_vec_push_val" => Some(1),
        "__vow_vec_push_val_in_arena" => Some(2),
        "__vow_vec_set_val" => Some(2),
        _ => None,
    }
}

/// True when `id` represents a model struct value (`__vow_vec_t`,
/// `__vow_string_t`, etc.) rather than an `int64_t`.
fn is_structured_value_id(
    id: u32,
    vec_vars: &HashSet<u32>,
    string_vars: &HashSet<u32>,
    hashmap_vars: &HashSet<u32>,
    btreemap_vars: &HashSet<u32>,
    option_vars: &HashSet<u32>,
) -> bool {
    vec_vars.contains(&id)
        || string_vars.contains(&id)
        || hashmap_vars.contains(&id)
        || btreemap_vars.contains(&id)
        || option_vars.contains(&id)
}

/// True when `inst` is a vec store/load op whose element side is a model
/// struct rather than a scalar — the configuration that produces
/// `int64_t = __vow_vec_t` (issue #505) in the emitted C model.
fn vec_op_carries_non_scalar(
    name: &str,
    inst: &Inst,
    vec_vars: &HashSet<u32>,
    string_vars: &HashSet<u32>,
    hashmap_vars: &HashSet<u32>,
    btreemap_vars: &HashSet<u32>,
    option_vars: &HashSet<u32>,
) -> bool {
    if name == "__vow_vec_get_val" {
        return is_structured_value_id(
            inst.id.0,
            vec_vars,
            string_vars,
            hashmap_vars,
            btreemap_vars,
            option_vars,
        );
    }
    if let Some(arg_idx) = vec_op_value_arg(name)
        && let Some(arg) = inst.args.get(arg_idx)
    {
        return is_structured_value_id(
            arg.0,
            vec_vars,
            string_vars,
            hashmap_vars,
            btreemap_vars,
            option_vars,
        );
    }
    false
}

fn string_model_receiver_arg(name: &str) -> Option<usize> {
    match name {
        "__vow_string_push_str_in_arena"
        | "__vow_string_push_byte_in_arena"
        | "__vow_string_substr_in_arena"
        | "__vow_string_substring_in_arena" => Some(1),
        "__vow_string_push_str"
        | "__vow_string_push_byte"
        | "__vow_string_substr"
        | "__vow_string_substring"
        | "__vow_string_clone"
        | "__vow_string_pin_to_root"
        | "__vow_string_len"
        | "__vow_string_clear"
        | "__vow_string_byte_at"
        | "__vow_string_eq"
        | "__vow_string_contains"
        | "__vow_string_matches_literal_at"
        | "__vow_string_print" => Some(0),
        "__vow_string_clone_in_arena" => Some(1),
        _ => None,
    }
}

fn string_model_extra_arg(name: &str) -> Option<usize> {
    match name {
        "__vow_string_push_str_in_arena" => Some(2),
        "__vow_string_push_str" | "__vow_string_eq" | "__vow_string_contains" => Some(1),
        _ => None,
    }
}

fn is_map_model_creator(name: &str) -> bool {
    matches!(name, "__vow_map_new" | "__vow_map_new_in_arena")
}

fn map_model_receiver_arg(name: &str) -> Option<usize> {
    match name {
        "__vow_map_insert_in_arena" | "__vow_map_remove_in_arena" => Some(1),
        "__vow_map_insert" | "__vow_map_remove" | "__vow_map_get" | "__vow_map_contains"
        | "__vow_map_len" => Some(0),
        _ => None,
    }
}

fn collect_typed_vars(func: &Function, creator: &str, prefix: &str) -> HashSet<u32> {
    let mut vars = HashSet::new();

    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Call
                && let InstData::CallExtern(ref name) = inst.data
            {
                let is_alt_creator = (prefix == "__vow_vec_" && is_vec_model_creator(name))
                    || (prefix == "__vow_string_" && is_string_model_creator(name))
                    || (prefix == "__vow_map_" && is_map_model_creator(name));
                let is_creator = name == creator || is_alt_creator;
                if is_creator {
                    vars.insert(inst.id.0);
                }
                if name.starts_with(prefix) {
                    let receiver_arg = if prefix == "__vow_vec_" {
                        vec_model_receiver_arg(name)
                    } else if prefix == "__vow_string_" {
                        string_model_receiver_arg(name)
                    } else if prefix == "__vow_map_" {
                        map_model_receiver_arg(name)
                    } else {
                        Some(0)
                    };
                    if let Some(arg_idx) = receiver_arg
                        && let Some(arg) = inst.args.get(arg_idx)
                    {
                        vars.insert(arg.0);
                    }
                    if prefix == "__vow_string_"
                        && let Some(arg_idx) = string_model_extra_arg(name)
                        && let Some(arg) = inst.args.get(arg_idx)
                    {
                        vars.insert(arg.0);
                    }
                }
            }
        }
    }

    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.insts {
                if inst.opcode == Opcode::Upsilon
                    && let InstData::PhiTarget(phi_id) = inst.data
                    && !inst.args.is_empty()
                {
                    if vars.contains(&inst.args[0].0) && vars.insert(phi_id.0) {
                        changed = true;
                    }
                    if vars.contains(&phi_id.0) && vars.insert(inst.args[0].0) {
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    vars
}

fn collect_option_vars(func: &Function) -> HashSet<u32> {
    let mut vars = HashSet::new();

    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Call
                && let InstData::CallExtern(ref name) = inst.data
                && (name == "__vow_string_parse_i64_opt"
                    || name == "__vow_string_parse_i64_opt_in_arena"
                    || name == "__vow_string_parse_u64_opt"
                    || name == "__vow_string_parse_i8_opt"
                    || name == "__vow_string_parse_u8_opt"
                    || name == "__vow_string_parse_i16_opt"
                    || name == "__vow_string_parse_u16_opt"
                    || name == "__vow_string_parse_u32_opt"
                    || name == "__vow_string_parse_i32_opt"
                    || (narrow_target_model(name).is_some() && name.ends_with("_try"))
                    || name == "__vow_btreemap_insert"
                    || name == "__vow_btreemap_get")
            {
                vars.insert(inst.id.0);
            }
        }
    }

    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.insts {
                if inst.opcode == Opcode::Upsilon
                    && let InstData::PhiTarget(phi_id) = inst.data
                    && !inst.args.is_empty()
                {
                    if vars.contains(&inst.args[0].0) && vars.insert(phi_id.0) {
                        changed = true;
                    }
                    if vars.contains(&phi_id.0) && vars.insert(inst.args[0].0) {
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    vars
}

// ---------------------------------------------------------------------------
// Modelable function detection (for cross-function spec verification)
// ---------------------------------------------------------------------------

fn is_known_builtin(name: &str) -> bool {
    if is_string_fresh_helper(name) || is_numeric_intrinsic(name) {
        return true;
    }

    matches!(
        name,
        "__vow_unwrap_panic"
            | "__vow_vec_new"
            | "__vow_vec_new_val"
            | "__vow_vec_new_in_arena"
            | "__vow_vec_new_val_in_arena"
            | "__vow_vec_push_val"
            | "__vow_vec_push_in_arena"
            | "__vow_vec_push_val_in_arena"
            | "__vow_vec_reserve_in_arena"
            | "__vow_vec_get_val"
            | "__vow_vec_from_raw_parts_copy_val"
            | "__vow_vec_pin_to_root_val"
            | "__vow_vec_len"
            | "__vow_vec_pop"
            | "__vow_vec_set_val"
            | "__vow_string_new"
            | "__vow_string_new_in_arena"
            | "__vow_string_literal"
            | "__vow_string_from_cstr"
            | "__vow_string_from_cstr_in_arena"
            | "__vow_string_clone"
            | "__vow_string_clone_in_arena"
            | "__vow_string_from_raw_parts_copy"
            | "__vow_string_pin_to_root"
            | "__vow_string_len"
            | "__vow_string_push_str"
            | "__vow_string_push_str_in_arena"
            | "__vow_string_push_byte"
            | "__vow_string_push_byte_in_arena"
            | "__vow_string_clear"
            | "__vow_string_byte_at"
            | "__vow_string_eq"
            | "__vow_string_contains"
            | "__vow_string_matches_literal_at"
            | "__vow_string_substr"
            | "__vow_string_substr_in_arena"
            | "__vow_string_substring"
            | "__vow_string_substring_in_arena"
            | "__vow_string_from_i64"
            | "__vow_string_from_i64_in_arena"
            | "__vow_string_from_u64"
            | "__vow_string_from_u64_in_arena"
            | "__vow_string_parse_i64_opt"
            | "__vow_string_parse_i64_opt_in_arena"
            | "__vow_string_parse_u64_opt"
            | "__vow_string_parse_i8_opt"
            | "__vow_string_parse_u8_opt"
            | "__vow_string_parse_i16_opt"
            | "__vow_string_parse_u16_opt"
            | "__vow_string_parse_u32_opt"
            | "__vow_string_parse_i32_opt"
            | "__vow_string_print"
            | "__vow_map_new"
            | "__vow_map_new_in_arena"
            | "__vow_map_len"
            | "__vow_map_insert"
            | "__vow_map_insert_in_arena"
            | "__vow_map_get"
            | "__vow_map_contains"
            | "__vow_map_remove"
            | "__vow_map_remove_in_arena"
            | "__vow_btreemap_new"
            | "__vow_btreemap_len"
            | "__vow_btreemap_insert"
            | "__vow_btreemap_get"
            | "__vow_btreemap_contains"
    )
}

#[derive(Clone, Copy)]
struct NarrowTargetModel {
    c_ty: &'static str,
    min: &'static str,
    max: &'static str,
}

fn narrow_target_model(name: &str) -> Option<NarrowTargetModel> {
    if !(name.starts_with("__vow_")
        && (name.ends_with("_try") || name.ends_with("_wrap") || name.ends_with("_sat")))
    {
        return None;
    }
    if name.contains("_to_i8_") {
        Some(NarrowTargetModel {
            c_ty: "int8_t",
            min: "-128",
            max: "127",
        })
    } else if name.contains("_to_u8_") {
        Some(NarrowTargetModel {
            c_ty: "uint8_t",
            min: "0",
            max: "255",
        })
    } else if name.contains("_to_i16_") {
        Some(NarrowTargetModel {
            c_ty: "int16_t",
            min: "-32768",
            max: "32767",
        })
    } else if name.contains("_to_u16_") {
        Some(NarrowTargetModel {
            c_ty: "uint16_t",
            min: "0",
            max: "65535",
        })
    } else if name.contains("_to_i32_") {
        Some(NarrowTargetModel {
            c_ty: "int32_t",
            min: "-2147483648",
            max: "2147483647",
        })
    } else if name.contains("_to_u32_") {
        Some(NarrowTargetModel {
            c_ty: "uint32_t",
            min: "0",
            max: "4294967295ULL",
        })
    } else {
        None
    }
}

fn is_numeric_intrinsic(name: &str) -> bool {
    narrow_target_model(name).is_some()
        || matches!(
            name,
            "__vow_add_sat_u8" | "__vow_sub_sat_u8" | "__vow_mul_sat_u8"
        )
}

fn is_reserved_verifier_symbol(name: &str) -> bool {
    // User functions are namespaced to `vow_user_fn_<id>` in the emitted C, so a
    // user symbol named after a libc function (e.g. `abs`) cannot collide with the
    // stdlib declaration; only the verifier's own intrinsics are truly reserved.
    name.starts_with("__ESBMC_") || name.starts_with("__VERIFIER_")
}

pub(crate) fn verifier_c_func_name(func: &Function) -> String {
    format!("vow_user_fn_{}", func.id.0)
}

/// Check whether a function can be precisely modeled in the C emitter.
/// Modelable functions are pure (no effects) and use only opcodes that the
/// C emitter handles without resorting to `__VERIFIER_nondet`.
pub fn is_modelable(
    func: &Function,
    module: &Module,
    const_fns: &HashMap<FuncId, ConstantValue>,
    cache: &mut HashMap<FuncId, bool>,
) -> bool {
    if is_reserved_verifier_symbol(&func.name) {
        return false;
    }

    if let Some(&cached) = cache.get(&func.id) {
        return cached;
    }
    cache.insert(func.id, false); // prevent infinite recursion

    if !func.effects.is_empty() {
        return false;
    }

    let vec_vars = collect_typed_vars(func, "__vow_vec_new", "__vow_vec_");
    let string_vars = collect_typed_vars(func, "__vow_string_new", "__vow_string_");
    let hashmap_vars = collect_typed_vars(func, "__vow_map_new", "__vow_map_");
    let btreemap_vars = collect_typed_vars(func, "__vow_btreemap_new", "__vow_btreemap_");
    let option_vars = collect_option_vars(func);

    for block in &func.blocks {
        for inst in &block.insts {
            let ok = match inst.opcode {
                Opcode::ConstI32
                | Opcode::ConstI64
                | Opcode::ConstF32
                | Opcode::ConstF64
                | Opcode::ConstBool
                | Opcode::ConstUnit
                | Opcode::ConstStr
                | Opcode::GetArg
                | Opcode::WrappingAdd
                | Opcode::WrappingSub
                | Opcode::WrappingMul
                | Opcode::WrappingDiv
                | Opcode::WrappingRem
                | Opcode::Eq
                | Opcode::Ne
                | Opcode::Lt
                | Opcode::Le
                | Opcode::Gt
                | Opcode::Ge
                | Opcode::BitAnd
                | Opcode::BitOr
                | Opcode::BitXor
                | Opcode::Shl
                | Opcode::Shr
                | Opcode::IntCast
                | Opcode::AddF32
                | Opcode::AddF64
                | Opcode::SubF32
                | Opcode::SubF64
                | Opcode::MulF32
                | Opcode::MulF64
                | Opcode::DivF32
                | Opcode::DivF64
                | Opcode::EqF32
                | Opcode::EqF64
                | Opcode::NeF32
                | Opcode::NeF64
                | Opcode::LtF32
                | Opcode::LtF64
                | Opcode::LeF32
                | Opcode::LeF64
                | Opcode::GtF32
                | Opcode::GtF64
                | Opcode::GeF32
                | Opcode::GeF64
                | Opcode::Not
                | Opcode::And
                | Opcode::Or
                | Opcode::ConstU64
                | Opcode::ConstU8
                | Opcode::VowRequires
                | Opcode::VowEnsures
                | Opcode::VowInvariant
                | Opcode::ComplexityDescriptor
                | Opcode::Branch
                | Opcode::Jump
                | Opcode::Return
                | Opcode::Unreachable
                | Opcode::Phi
                | Opcode::Upsilon
                | Opcode::RegionOpen
                | Opcode::RegionClose => true,

                Opcode::Call => match &inst.data {
                    InstData::CallExtern(name) => {
                        is_known_builtin(name)
                            && !vec_op_carries_non_scalar(
                                name,
                                inst,
                                &vec_vars,
                                &string_vars,
                                &hashmap_vars,
                                &btreemap_vars,
                                &option_vars,
                            )
                    }
                    InstData::CallTarget(fid) => {
                        const_fns.contains_key(fid)
                            || module.functions.iter().find(|f| f.id == *fid).is_some_and(
                                |callee| is_modelable(callee, module, const_fns, cache),
                            )
                    }
                    _ => false,
                },

                // Collection/Option field reads have dedicated models; all other
                // FieldGets are user-struct slot reads under the heap model.
                Opcode::FieldGet => true,

                // User-struct heap model: allocation and field writes are slot ops.
                Opcode::RegionAlloc | Opcode::FieldSet => true,

                // #585: a checked operator aborts on overflow, and the model
                // only reproduces that abort for the widths
                // `emit_checked_arith` has a guard for. 128-bit sites fail
                // closed here (reported `Skipped`) rather than silently
                // reverting to the wrapping model.
                Opcode::CheckedAdd
                | Opcode::CheckedSub
                | Opcode::CheckedMul
                | Opcode::CheckedDiv
                | Opcode::CheckedRem => checked_integer_type(inst).is_some(),

                Opcode::RemF32
                | Opcode::RemF64
                | Opcode::ConstI128
                | Opcode::ConstU128
                | Opcode::Load
                | Opcode::Store
                | Opcode::LinearConsume
                | Opcode::LinearBorrow => false,

                Opcode::DebugCall => true,
            };
            if !ok {
                return false;
            }
        }
    }

    cache.insert(func.id, true);
    true
}

/// Gate: if `func` is non-modelable, return a human-readable reason; `None` means modelable. Mirror of `compiler/c_emitter.vow::non_modelable_reason`; must stay in sync.
pub fn non_modelable_reason(
    func: &Function,
    module: &Module,
    const_fns: &HashMap<FuncId, ConstantValue>,
) -> Option<String> {
    if is_reserved_verifier_symbol(&func.name) {
        return Some(format!(
            "function `{}` shadows a reserved verifier symbol",
            func.name
        ));
    }
    if !func.effects.is_empty() {
        return Some(format!(
            "function `{}` has effects; the verifier model is restricted to pure functions",
            func.name
        ));
    }
    let mut cache = HashMap::new();
    if is_modelable(func, module, const_fns, &mut cache) {
        return None;
    }
    let offender = first_unsupported_opcode(func, module, const_fns);
    let detail = match offender {
        Some(name) => format!("contains unsupported opcode `{name}`"),
        None => "calls a non-modelable callee".to_string(),
    };
    Some(format!(
        "function `{}` is not modelable in the verifier ({detail})",
        func.name
    ))
}

fn first_unsupported_opcode(
    func: &Function,
    module: &Module,
    const_fns: &HashMap<FuncId, ConstantValue>,
) -> Option<String> {
    let vec_vars = collect_typed_vars(func, "__vow_vec_new", "__vow_vec_");
    let string_vars = collect_typed_vars(func, "__vow_string_new", "__vow_string_");
    let hashmap_vars = collect_typed_vars(func, "__vow_map_new", "__vow_map_");
    let btreemap_vars = collect_typed_vars(func, "__vow_btreemap_new", "__vow_btreemap_");
    let option_vars = collect_option_vars(func);
    for block in &func.blocks {
        for inst in &block.insts {
            match inst.opcode {
                Opcode::RemF32
                | Opcode::RemF64
                | Opcode::ConstI128
                | Opcode::ConstU128
                | Opcode::Load
                | Opcode::Store
                | Opcode::LinearConsume
                | Opcode::LinearBorrow => return Some(format!("{:?}", inst.opcode)),
                Opcode::CheckedAdd
                | Opcode::CheckedSub
                | Opcode::CheckedMul
                | Opcode::CheckedDiv
                | Opcode::CheckedRem
                    if checked_integer_type(inst).is_none() =>
                {
                    return Some(format!("{:?} at 128-bit width", inst.opcode));
                }
                Opcode::Call => match &inst.data {
                    InstData::CallExtern(name) => {
                        if !is_known_builtin(name) {
                            return Some(format!("Call extern `{name}`"));
                        }
                        if vec_op_carries_non_scalar(
                            name,
                            inst,
                            &vec_vars,
                            &string_vars,
                            &hashmap_vars,
                            &btreemap_vars,
                            &option_vars,
                        ) {
                            return Some(format!("Call extern `{name}` with non-scalar element"));
                        }
                    }
                    InstData::CallTarget(fid) => {
                        if !const_fns.contains_key(fid) {
                            if let Some(callee) = module.functions.iter().find(|f| f.id == *fid) {
                                let mut cache = HashMap::new();
                                if !is_modelable(callee, module, const_fns, &mut cache) {
                                    return Some(format!("Call target `{}`", callee.name));
                                }
                            } else {
                                return Some(format!("Call target `FuncId({})`", fid.0));
                            }
                        }
                    }
                    _ => return Some(format!("Call ({:?})", inst.data)),
                },
                _ => {}
            }
        }
    }
    None
}

/// Collect all modelable callees reachable from `func`, excluding constant
/// functions (which are inlined). Returns FuncIds in topological order
/// (callees before callers).
pub fn collect_modelable_callees(
    func: &Function,
    module: &Module,
    const_fns: &HashMap<FuncId, ConstantValue>,
    modelable_cache: &mut HashMap<FuncId, bool>,
) -> Vec<FuncId> {
    let mut visited = HashSet::new();
    let mut order = Vec::new();
    collect_callees_dfs(
        func,
        module,
        const_fns,
        modelable_cache,
        &mut visited,
        &mut order,
    );
    order
}

fn collect_callees_dfs(
    func: &Function,
    module: &Module,
    const_fns: &HashMap<FuncId, ConstantValue>,
    modelable_cache: &mut HashMap<FuncId, bool>,
    visited: &mut HashSet<FuncId>,
    order: &mut Vec<FuncId>,
) {
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Call
                && let InstData::CallTarget(fid) = &inst.data
            {
                if const_fns.contains_key(fid) || visited.contains(fid) {
                    continue;
                }
                if let Some(callee) = module.functions.iter().find(|f| f.id == *fid)
                    && is_modelable(callee, module, const_fns, modelable_cache)
                {
                    visited.insert(*fid);
                    collect_callees_dfs(callee, module, const_fns, modelable_cache, visited, order);
                    order.push(*fid);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Expression / statement emission
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn emit_inst(
    inst: &Inst,
    out: &mut String,
    vec_vars: &HashSet<u32>,
    string_vars: &HashSet<u32>,
    hashmap_vars: &HashSet<u32>,
    btreemap_vars: &HashSet<u32>,
    option_vars: &HashSet<u32>,
    const_fns: &HashMap<FuncId, ConstantValue>,
    modelable_fns: &HashSet<FuncId>,
    const_str_indices: &HashMap<u32, u32>,
    eq_pairs: &[(u32, u32)],
    inst_by_id: &HashMap<u32, &Inst>,
    module: &Module,
    limits: &VerifyLimits,
    func_return_ty: Ty,
    current_func_id: FuncId,
    requires_as_assert: bool,
) {
    let id = inst.id.0;
    match inst.opcode {
        // Constants
        Opcode::ConstI32 => {
            if let InstData::ConstI32(v) = inst.data {
                out.push_str(&format!("  v{} = {};\n", id, v));
            }
        }
        Opcode::ConstI64 => {
            if let InstData::ConstI64(v) = inst.data {
                out.push_str(&format!("  v{} = {}LL;\n", id, v));
            }
        }
        // Epic #526: wide constant verification is implemented by a later seam.
        Opcode::ConstI128 | Opcode::ConstU128 => emit_unmodelled(inst, out),
        Opcode::ConstF32 => {
            if let InstData::ConstF32(v) = inst.data {
                out.push_str(&format!("  v{} = {}f;\n", id, v));
            }
        }
        Opcode::ConstF64 => {
            if let InstData::ConstF64(v) = inst.data {
                out.push_str(&format!("  v{} = {};\n", id, v));
            }
        }
        Opcode::ConstBool => {
            let b = matches!(inst.data, InstData::ConstBool(true));
            out.push_str(&format!("  v{} = {};\n", id, b as i32));
        }
        Opcode::ConstUnit => {
            out.push_str(&format!("  v{} = 0;\n", id));
        }
        Opcode::ConstStr => {
            out.push_str(&format!("  v{} = 0; /* string not modelled */\n", id));
        }

        // Arguments — emitted as parameter names at function top
        Opcode::GetArg => {}

        Opcode::IntCast => {
            out.push_str(&format!(
                "  v{} = ({})v{};\n",
                id,
                ir_ty_to_c(inst.ty),
                inst.args[0].0
            ));
        }

        // Arithmetic. Wrapping operators are *specified* to wrap, so a plain C
        // operator under the bit-vector encoding is already their exact model.
        // The checked operators abort instead, which `emit_checked_arith` models.
        Opcode::WrappingAdd => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = v{} + v{};\n", id, a, b));
        }
        Opcode::WrappingSub => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = v{} - v{};\n", id, a, b));
        }
        Opcode::WrappingMul => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = v{} * v{};\n", id, a, b));
        }
        Opcode::WrappingDiv => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = v{} / v{};\n", id, a, b));
        }
        Opcode::WrappingRem => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = v{} % v{};\n", id, a, b));
        }
        Opcode::CheckedAdd
        | Opcode::CheckedSub
        | Opcode::CheckedMul
        | Opcode::CheckedDiv
        | Opcode::CheckedRem => emit_checked_arith(inst, out),

        // Float arithmetic
        Opcode::AddF32 | Opcode::AddF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = v{} + v{};\n", id, a, b));
        }
        Opcode::SubF32 | Opcode::SubF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = v{} - v{};\n", id, a, b));
        }
        Opcode::MulF32 | Opcode::MulF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = v{} * v{};\n", id, a, b));
        }
        Opcode::DivF32 | Opcode::DivF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = v{} / v{};\n", id, a, b));
        }

        // Integer comparisons
        Opcode::Eq | Opcode::EqF32 | Opcode::EqF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} == v{});\n", id, a, b));
        }
        Opcode::Ne | Opcode::NeF32 | Opcode::NeF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} != v{});\n", id, a, b));
        }
        Opcode::Lt | Opcode::LtF32 | Opcode::LtF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} < v{});\n", id, a, b));
        }
        Opcode::Le | Opcode::LeF32 | Opcode::LeF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} <= v{});\n", id, a, b));
        }
        Opcode::Gt | Opcode::GtF32 | Opcode::GtF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} > v{});\n", id, a, b));
        }
        Opcode::Ge | Opcode::GeF32 | Opcode::GeF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} >= v{});\n", id, a, b));
        }

        // Boolean ops
        Opcode::Not => {
            let a = inst.args[0].0;
            out.push_str(&format!("  v{} = !v{};\n", id, a));
        }
        Opcode::And => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} && v{});\n", id, a, b));
        }
        Opcode::Or => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} || v{});\n", id, a, b));
        }
        Opcode::BitAnd => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} & v{});\n", id, a, b));
        }
        Opcode::BitOr => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} | v{});\n", id, a, b));
        }
        Opcode::BitXor => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} ^ v{});\n", id, a, b));
        }
        Opcode::Shl | Opcode::Shr => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            let int_ty = match inst.data {
                InstData::Integer(ty) => ty,
                _ => IntegerType::I64,
            };
            out.push_str(&format!(
                "  __ESBMC_assert(v{b} < {}, \"integer shift count\");\n",
                int_ty.width.bits()
            ));
            let prefix = match int_ty.signedness {
                IntegerSignedness::Signed => "i",
                IntegerSignedness::Unsigned => "u",
            };
            let op = if inst.opcode == Opcode::Shl {
                "shl"
            } else {
                "shr"
            };
            out.push_str(&format!(
                "  v{} = __vow_{}_{}{}(v{}, v{});\n",
                id,
                op,
                prefix,
                int_ty.width.bits(),
                a,
                b
            ));
        }

        Opcode::ConstU64 => {
            if let InstData::ConstU64(v) = inst.data {
                out.push_str(&format!("  v{} = {}ULL;\n", id, v));
            }
        }
        Opcode::ConstU8 => {
            if let InstData::ConstU8(v) = inst.data {
                out.push_str(&format!("  v{} = UINT8_C({});\n", id, v));
            }
        }

        // Vow checks → ESBMC intrinsics
        Opcode::VowRequires => {
            let pred = inst.args[0].0;
            if requires_as_assert {
                // Callee precondition: the caller is responsible for satisfying
                // it (requires → Caller blame). Asserting it here — rather than
                // assuming it — is what makes a caller that passes a violating
                // argument fail, instead of injecting `assume(false)` and
                // vacuously verifying the rest of the caller's body. The label
                // carries both the callee function id and the callee-local vow id
                // so diagnostics can resolve the exact precondition without
                // colliding with the target function's vow ids.
                let vow_id = match inst.data {
                    InstData::VowId(v) => v.0,
                    _ => 0,
                };
                out.push_str(&format!(
                    "  __ESBMC_assert(v{}, \"vow:pre:{}:{}\");\n",
                    pred, current_func_id.0, vow_id
                ));
            } else {
                // Target function precondition: assumed for the function under
                // verification (its caller is out of scope of this query).
                out.push_str(&format!("  __ESBMC_assume(v{});\n", pred));
            }
        }
        Opcode::VowEnsures | Opcode::VowInvariant => {
            let pred = inst.args[0].0;
            let vow_id = match inst.data {
                InstData::VowId(v) => v.0,
                _ => 0,
            };
            out.push_str(&format!(
                "  __ESBMC_assert(v{}, \"vow:{}\");\n",
                pred, vow_id
            ));
        }

        Opcode::ComplexityDescriptor => {
            // Performance-contract metadata belongs to vow-perf and produces
            // no correctness verification condition.
        }

        // Control flow
        Opcode::Branch => {
            let cond = inst.args[0].0;
            let (then_b, else_b) = match inst.data {
                InstData::BranchTargets {
                    then_block,
                    else_block,
                } => (then_block.0, else_block.0),
                _ => unreachable!(),
            };
            out.push_str(&format!(
                "  if (v{}) goto block{}; else goto block{};\n",
                cond, then_b, else_b
            ));
        }
        Opcode::Jump => {
            let target = match inst.data {
                InstData::JumpTarget(b) => b.0,
                _ => unreachable!(),
            };
            out.push_str(&format!("  goto block{};\n", target));
        }
        Opcode::Return => {
            if func_return_ty == Ty::Unit {
                out.push_str("  return;\n");
            } else if let Some(&val_id) = inst.args.first() {
                if vec_vars.contains(&val_id.0)
                    || string_vars.contains(&val_id.0)
                    || hashmap_vars.contains(&val_id.0)
                    || btreemap_vars.contains(&val_id.0)
                    || option_vars.contains(&val_id.0)
                {
                    out.push_str("  return 0; /* modelled type return */\n");
                } else {
                    out.push_str(&format!("  return v{};\n", val_id.0));
                }
            } else {
                out.push_str("  return 0;\n");
            }
        }
        Opcode::Unreachable => {
            out.push_str("  __ESBMC_assume(0); /* unreachable */\n");
        }

        // `.unwrap()` on the empty variant. The lowerer guards this call with a
        // discriminant branch, so reaching it *is* the unwrap-on-None failure.
        Opcode::Call if matches!(&inst.data, InstData::CallExtern(name) if name == "__vow_unwrap_panic") =>
        {
            out.push_str("  __ESBMC_assert(0, \"unwrap-none\");\n");
        }

        // Phi — already pre-declared at function top; nothing to emit here
        Opcode::Phi => {}
        Opcode::Upsilon => {
            if let InstData::PhiTarget(phi_id) = inst.data {
                let val = inst.args[0].0;
                out.push_str(&format!("  v{} = v{};\n", phi_id.0, val));
            }
        }

        // Vec operations — modeled as abstract struct with len + data array
        Opcode::Call
            if matches!(
                &inst.data,
                InstData::CallExtern(name)
                    if matches!(
                        name.as_str(),
                        "__vow_add_sat_u8" | "__vow_sub_sat_u8" | "__vow_mul_sat_u8"
                    )
            ) =>
        {
            let InstData::CallExtern(name) = &inst.data else {
                unreachable!()
            };
            let a = inst.args[0].0;
            let b = inst.args[1].0;
            let expression = match name.as_str() {
                "__vow_add_sat_u8" => format!("(uint16_t)v{a} + (uint16_t)v{b}"),
                "__vow_sub_sat_u8" => format!("v{a} < v{b} ? 0 : v{a} - v{b}"),
                "__vow_mul_sat_u8" => format!("(uint16_t)v{a} * (uint16_t)v{b}"),
                _ => unreachable!(),
            };
            if name == "__vow_sub_sat_u8" {
                out.push_str(&format!("  v{id} = (uint8_t)({expression});\n"));
            } else {
                out.push_str(&format!(
                    "  uint16_t __sat_{id} = {expression};\n  v{id} = __sat_{id} > 255 ? 255 : (uint8_t)__sat_{id};\n"
                ));
            }
        }

        Opcode::Call if matches!(&inst.data, InstData::CallExtern(name) if narrow_target_model(name).is_some()) =>
        {
            let InstData::CallExtern(name) = &inst.data else {
                unreachable!()
            };
            let target = narrow_target_model(name).expect("guarded by match");
            let a = inst.args[0].0;
            let source_signed = name.starts_with("__vow_i");
            if name.ends_with("_try") {
                let guard = if source_signed {
                    format!("v{a} >= {} && v{a} <= {}", target.min, target.max)
                } else {
                    format!("v{a} <= {}", target.max)
                };
                out.push_str(&format!(
                    "  v{id}.tag = ({guard});\n  v{id}.payload = v{id}.tag ? ({})v{a} : 0;\n",
                    target.c_ty
                ));
            } else if name.ends_with("_wrap") {
                out.push_str(&format!("  v{id} = ({})v{a};\n", target.c_ty));
            } else if source_signed {
                out.push_str(&format!(
                    "  v{id} = v{a} < {} ? {} : (v{a} > {} ? {} : ({})v{a});\n",
                    target.min, target.min, target.max, target.max, target.c_ty
                ));
            } else {
                out.push_str(&format!(
                    "  v{id} = v{a} > {} ? {} : ({})v{a};\n",
                    target.max, target.max, target.c_ty
                ));
            }
        }

        Opcode::Call if matches!(&inst.data, InstData::CallExtern(n) if n.starts_with("__vow_vec_")) => {
            if let InstData::CallExtern(ref name) = inst.data {
                match name.as_str() {
                    "__vow_vec_new"
                    | "__vow_vec_new_val"
                    | "__vow_vec_new_in_arena"
                    | "__vow_vec_new_val_in_arena" => {
                        out.push_str(&format!("  v{id}.len = 0;\n"));
                    }
                    "__vow_vec_from_raw_parts_copy_val" => {
                        let len = inst.args[1].0;
                        let vec_max = limits.vec_max;
                        out.push_str(&format!(
                            "  __ESBMC_assume(v{len} >= 0 && v{len} < {vec_max});\n  v{id}.len = v{len};\n"
                        ));
                    }
                    "__vow_vec_pin_to_root_val" => {
                        let source = inst.args[0].0;
                        out.push_str(&format!("  v{id} = v{source};\n"));
                    }
                    "__vow_vec_push_val" | "__vow_vec_push_val_in_arena" => {
                        let (vec_arg, val_arg) = if name == "__vow_vec_push_val_in_arena" {
                            (1, 2)
                        } else {
                            (0, 1)
                        };
                        let vec = inst.args[vec_arg].0;
                        let val = inst.args[val_arg].0;
                        let vec_max = limits.vec_max;
                        out.push_str(&format!(
                            "  __ESBMC_assert(v{vec}.len < {vec_max}, \"vec capacity\");\n\
                             \x20 v{vec}.data[v{vec}.len] = v{val};\n  v{vec}.len++;\n",
                        ));
                    }
                    "__vow_vec_push_in_arena" => {
                        let vec = inst.args[1].0;
                        let vec_max = limits.vec_max;
                        out.push_str(&format!(
                            "  __ESBMC_assert(v{vec}.len < {vec_max}, \"vec capacity\");\n\
                             \x20 v{vec}.data[v{vec}.len] = __VERIFIER_nondet_long();\n  v{vec}.len++;\n",
                        ));
                    }
                    "__vow_vec_reserve_in_arena" => {}
                    "__vow_vec_get_val" => {
                        let vec = inst.args[0].0;
                        let idx = inst.args[1].0;
                        emit_bounds_assert(
                            idx,
                            vec,
                            operand_ty(idx, inst_by_id),
                            "vec bounds",
                            out,
                        );
                        out.push_str(&format!("  v{id} = v{vec}.data[v{idx}];\n"));
                    }
                    "__vow_vec_len" => {
                        let vec = inst.args[0].0;
                        out.push_str(&format!("  v{id} = v{vec}.len;\n"));
                    }
                    "__vow_vec_pop" => {
                        let vec = inst.args[0].0;
                        out.push_str(&format!("  if (v{vec}.len > 0) {{ v{vec}.len--; }}\n"));
                    }
                    "__vow_vec_set_val" => {
                        let vec = inst.args[0].0;
                        let idx = inst.args[1].0;
                        let val = inst.args[2].0;
                        emit_bounds_assert(
                            idx,
                            vec,
                            operand_ty(idx, inst_by_id),
                            "vec bounds",
                            out,
                        );
                        out.push_str(&format!("  v{vec}.data[v{idx}] = v{val};\n"));
                    }
                    _ => {
                        emit_unmodelled(inst, out);
                    }
                }
            }
        }

        // String operations — modeled as abstract struct with len + data array
        Opcode::Call if matches!(&inst.data, InstData::CallExtern(n) if n.starts_with("__vow_string_")) => {
            if let InstData::CallExtern(ref name) = inst.data {
                match name.as_str() {
                    "__vow_string_new" | "__vow_string_new_in_arena" => {
                        let len_arg = if name == "__vow_string_new_in_arena" {
                            2
                        } else {
                            1
                        };
                        let len = inst.args[len_arg].0;
                        let string_max = limits.string_max;
                        out.push_str(&format!(
                            "  __ESBMC_assume(v{len} >= 0 && v{len} < {string_max});\n  v{id}.len = v{len};\n"
                        ));
                    }
                    "__vow_string_from_cstr" | "__vow_string_from_cstr_in_arena" => {
                        emit_nondet_string_len(id, limits.string_max, out);
                    }
                    "__vow_string_literal" => {
                        let literal = inst
                            .args
                            .first()
                            .and_then(|arg| const_str_indices.get(&arg.0))
                            .and_then(|idx| module.strings.get(*idx as usize));
                        if let Some(literal) = literal {
                            out.push_str(&format!("  v{id}.len = {};\n", literal.len()));
                            for (idx, byte) in literal.as_bytes().iter().enumerate() {
                                out.push_str(&format!("  v{id}.data[{idx}] = (int8_t){byte};\n"));
                            }
                        } else {
                            emit_nondet_string_len(id, limits.string_max, out);
                        }
                    }
                    "__vow_string_from_raw_parts_copy" => {
                        let len = inst.args[1].0;
                        let string_max = limits.string_max;
                        out.push_str(&format!(
                            "  __ESBMC_assume(v{len} >= 0 && v{len} < {string_max});\n  v{id}.len = v{len};\n"
                        ));
                    }
                    "__vow_string_clone"
                    | "__vow_string_clone_in_arena"
                    | "__vow_string_pin_to_root" => {
                        let source_arg = if name == "__vow_string_clone_in_arena" {
                            1
                        } else {
                            0
                        };
                        let source = inst.args[source_arg].0;
                        out.push_str(&format!("  v{id} = v{source};\n"));
                    }
                    "__vow_string_len" => {
                        let s = inst.args[0].0;
                        out.push_str(&format!("  v{id} = v{s}.len;\n"));
                    }
                    "__vow_string_from_i64"
                    | "__vow_string_from_i64_in_arena"
                    | "__vow_string_from_u64"
                    | "__vow_string_from_u64_in_arena" => {
                        emit_nondet_string_len(id, limits.string_max, out);
                    }
                    name if is_string_fresh_helper(name) => {
                        emit_nondet_string_len(id, limits.string_max, out);
                    }
                    "__vow_string_push_str" | "__vow_string_push_str_in_arena" => {
                        let (dest_arg, src_arg) = if name == "__vow_string_push_str_in_arena" {
                            (1, 2)
                        } else {
                            (0, 1)
                        };
                        let dest = inst.args[dest_arg].0;
                        let src = inst.args[src_arg].0;
                        let string_max = limits.string_max;
                        out.push_str(&format!(
                            "  __ESBMC_assert(v{dest}.len + v{src}.len <= {string_max}, \"string capacity\");\n\
                             \x20 v{dest}.len += v{src}.len;\n",
                        ));
                        emit_string_eq_invalidate(dest, eq_pairs, out);
                    }
                    "__vow_string_push_byte" | "__vow_string_push_byte_in_arena" => {
                        let (s_arg, byte_arg) = if name == "__vow_string_push_byte_in_arena" {
                            (1, 2)
                        } else {
                            (0, 1)
                        };
                        let s = inst.args[s_arg].0;
                        let byte = inst.args[byte_arg].0;
                        let string_max = limits.string_max;
                        out.push_str(&format!(
                            "  __ESBMC_assert(v{s}.len < {string_max}, \"string capacity\");\n\
                             \x20 v{s}.data[v{s}.len] = (int8_t)v{byte};\n  v{s}.len++;\n",
                        ));
                        emit_string_eq_invalidate(s, eq_pairs, out);
                    }
                    "__vow_string_clear" => {
                        let s = inst.args[0].0;
                        out.push_str(&format!("  v{s}.len = 0;\n"));
                        emit_string_eq_invalidate(s, eq_pairs, out);
                    }
                    "__vow_string_byte_at" => {
                        let s = inst.args[0].0;
                        let idx = inst.args[1].0;
                        emit_bounds_assert(
                            idx,
                            s,
                            operand_ty(idx, inst_by_id),
                            "string bounds",
                            out,
                        );
                        out.push_str(&format!(
                            "  v{id} = (int64_t)(unsigned char)v{s}.data[v{idx}];\n\
                             \x20 __ESBMC_assume(v{id} >= 0 && v{id} <= 255);\n"
                        ));
                    }
                    "__vow_string_eq" => {
                        let a = inst.args[0].0;
                        let b = inst.args[1].0;
                        if a == b {
                            out.push_str(&format!("  v{id} = 1;\n"));
                        } else {
                            let lo = a.min(b);
                            let hi = a.max(b);
                            out.push_str(&format!(
                                "  v{id} = (v{a}.len == v{b}.len) ? __str_eq_{lo}_{hi} : 0;\n"
                            ));
                        }
                    }
                    "__vow_string_contains" => {
                        let h = inst.args[0].0;
                        let n = inst.args[1].0;
                        out.push_str(&format!(
                            "  v{id} = 0;\n\
                             \x20 if (v{n}.len == 0) {{ v{id} = 1; }}\n\
                             \x20 else if (v{n}.len <= v{h}.len) {{\n\
                             \x20   for (int64_t __i = 0; __i <= v{h}.len - v{n}.len; __i++) {{\n\
                             \x20     _Bool __match = 1;\n\
                             \x20     for (int64_t __j = 0; __j < v{n}.len; __j++) {{\n\
                             \x20       if (v{h}.data[__i + __j] != v{n}.data[__j]) {{ __match = 0; break; }}\n\
                             \x20     }}\n\
                             \x20     if (__match) {{ v{id} = 1; break; }}\n\
                             \x20   }}\n\
                            \x20 }}\n"
                        ));
                    }
                    "__vow_string_matches_literal_at" => {
                        let s = inst.args[0].0;
                        let pos = inst.args[1].0;
                        let literal_ptr = inst.args[2].0;
                        let literal = inst_by_id.get(&literal_ptr).and_then(|literal_inst| {
                            if let InstData::ConstStr(idx) = &literal_inst.data {
                                module.strings.get(*idx as usize)
                            } else {
                                None
                            }
                        });
                        if let Some(literal) = literal {
                            let bytes = literal.as_bytes();
                            let len = bytes.len();
                            out.push_str(&format!("  v{id} = 0;\n"));
                            out.push_str(&format!(
                                "  if (v{pos} >= 0 && v{pos} <= v{s}.len && {len}LL <= v{s}.len - v{pos}) {{\n"
                            ));
                            if bytes.is_empty() {
                                out.push_str(&format!("    v{id} = 1;\n"));
                            } else {
                                out.push_str("    _Bool __match = 1;\n");
                                for (idx, byte) in bytes.iter().enumerate() {
                                    out.push_str(&format!(
                                        "    if ((unsigned char)v{s}.data[v{pos} + {idx}] != {byte}) {{ __match = 0; }}\n"
                                    ));
                                }
                                out.push_str(&format!("    v{id} = __match ? 1 : 0;\n"));
                            }
                            out.push_str("  }\n");
                        } else {
                            emit_unmodelled(inst, out);
                        }
                    }
                    "__vow_string_substr" | "__vow_string_substr_in_arena" => {
                        let (s_arg, start_arg, len_arg) = if name == "__vow_string_substr_in_arena"
                        {
                            (1, 2, 3)
                        } else {
                            (0, 1, 2)
                        };
                        let s = inst.args[s_arg].0;
                        let start = inst.args[start_arg].0;
                        let len = inst.args[len_arg].0;
                        let string_max = limits.string_max;
                        out.push_str(&format!(
                            "  int64_t __substr_start_{id} = v{start};\n\
                             \x20 if (__substr_start_{id} < 0) {{ __substr_start_{id} = 0; }}\n\
                             \x20 if (__substr_start_{id} > v{s}.len) {{ __substr_start_{id} = v{s}.len; }}\n\
                             \x20 int64_t __substr_len_{id} = v{len};\n\
                             \x20 if (__substr_len_{id} < 0) {{ __substr_len_{id} = 0; }}\n\
                             \x20 int64_t __substr_max_len_{id} = v{s}.len - __substr_start_{id};\n\
                             \x20 if (__substr_len_{id} > __substr_max_len_{id}) {{ __substr_len_{id} = __substr_max_len_{id}; }}\n\
                             \x20 v{id}.len = __substr_len_{id};\n\
                             \x20 for (int64_t __i = 0; __i < v{id}.len && __i < {string_max}; __i++) {{\n\
                             \x20   v{id}.data[__i] = v{s}.data[__substr_start_{id} + __i];\n\
                             \x20 }}\n",
                        ));
                    }
                    "__vow_string_substring" | "__vow_string_substring_in_arena" => {
                        let (s_arg, start_arg, end_arg) =
                            if name == "__vow_string_substring_in_arena" {
                                (1, 2, 3)
                            } else {
                                (0, 1, 2)
                            };
                        let s = inst.args[s_arg].0;
                        let start = inst.args[start_arg].0;
                        let end = inst.args[end_arg].0;
                        let string_max = limits.string_max;
                        out.push_str(&format!(
                            "  int64_t __substring_start_{id} = v{start};\n\
                             \x20 if (__substring_start_{id} < 0) {{ __substring_start_{id} = 0; }}\n\
                             \x20 if (__substring_start_{id} > v{s}.len) {{ __substring_start_{id} = v{s}.len; }}\n\
                             \x20 int64_t __substring_end_{id} = v{end};\n\
                             \x20 if (__substring_end_{id} < __substring_start_{id}) {{ __substring_end_{id} = __substring_start_{id}; }}\n\
                             \x20 if (__substring_end_{id} > v{s}.len) {{ __substring_end_{id} = v{s}.len; }}\n\
                             \x20 v{id}.len = __substring_end_{id} - __substring_start_{id};\n\
                             \x20 for (int64_t __i = 0; __i < v{id}.len && __i < {string_max}; __i++) {{\n\
                             \x20   v{id}.data[__i] = v{s}.data[__substring_start_{id} + __i];\n\
                             \x20 }}\n",
                        ));
                    }
                    "__vow_string_parse_i64_opt"
                    | "__vow_string_parse_i64_opt_in_arena"
                    | "__vow_string_parse_u64_opt" => {
                        out.push_str(&format!(
                            "  v{id}.tag = __VERIFIER_nondet_long();\n\
                             \x20 __ESBMC_assume(v{id}.tag == 0 || v{id}.tag == 1);\n\
                             \x20 if (v{id}.tag == 1) {{ v{id}.payload = __VERIFIER_nondet_long(); }}\n"
                        ));
                    }
                    "__vow_string_parse_u8_opt" => {
                        out.push_str(&format!(
                            "  v{id}.tag = __VERIFIER_nondet_long();\n\
                             \x20 __ESBMC_assume(v{id}.tag == 0 || v{id}.tag == 1);\n\
                             \x20 if (v{id}.tag == 1) {{ v{id}.payload = __VERIFIER_nondet_long(); __ESBMC_assume(v{id}.payload >= 0 && v{id}.payload <= 255); }}\n"
                        ));
                    }
                    "__vow_string_parse_i8_opt" => {
                        out.push_str(&format!(
                            "  v{id}.tag = __VERIFIER_nondet_long();\n\
                             \x20 __ESBMC_assume(v{id}.tag == 0 || v{id}.tag == 1);\n\
                             \x20 if (v{id}.tag == 1) {{ v{id}.payload = __VERIFIER_nondet_long(); __ESBMC_assume(v{id}.payload >= -128 && v{id}.payload <= 127); }}\n"
                        ));
                    }
                    "__vow_string_parse_i16_opt" => {
                        out.push_str(&format!(
                            "  v{id}.tag = __VERIFIER_nondet_long();\n\
                             \x20 __ESBMC_assume(v{id}.tag == 0 || v{id}.tag == 1);\n\
                             \x20 if (v{id}.tag == 1) {{ v{id}.payload = __VERIFIER_nondet_long(); __ESBMC_assume(v{id}.payload >= -32768 && v{id}.payload <= 32767); }}\n"
                        ));
                    }
                    "__vow_string_parse_u16_opt" => {
                        out.push_str(&format!(
                            "  v{id}.tag = __VERIFIER_nondet_long();\n\
                             \x20 __ESBMC_assume(v{id}.tag == 0 || v{id}.tag == 1);\n\
                             \x20 if (v{id}.tag == 1) {{ v{id}.payload = __VERIFIER_nondet_long(); __ESBMC_assume(v{id}.payload >= 0 && v{id}.payload <= 65535); }}\n"
                        ));
                    }
                    "__vow_string_parse_u32_opt" => {
                        out.push_str(&format!(
                            "  v{id}.tag = __VERIFIER_nondet_long();\n\
                             \x20 __ESBMC_assume(v{id}.tag == 0 || v{id}.tag == 1);\n\
                             \x20 if (v{id}.tag == 1) {{ v{id}.payload = __VERIFIER_nondet_ulong(); __ESBMC_assume(v{id}.payload >= 0 && v{id}.payload <= 4294967295ULL); }}\n"
                        ));
                    }
                    "__vow_string_parse_i32_opt" => {
                        out.push_str(&format!(
                            "  v{id}.tag = __VERIFIER_nondet_long();\n\
                             \x20 __ESBMC_assume(v{id}.tag == 0 || v{id}.tag == 1);\n\
                             \x20 if (v{id}.tag == 1) {{ v{id}.payload = __VERIFIER_nondet_long(); __ESBMC_assume(v{id}.payload >= -2147483648 && v{id}.payload <= 2147483647); }}\n"
                        ));
                    }
                    "__vow_string_print" => {
                        out.push_str("  /* string print not modelled */\n");
                    }
                    _ => {
                        emit_unmodelled(inst, out);
                    }
                }
            }
        }

        // HashMap operations — modeled as abstract struct with len + keys/vals arrays
        Opcode::Call if matches!(&inst.data, InstData::CallExtern(n) if n.starts_with("__vow_map_")) =>
        {
            if let InstData::CallExtern(ref name) = inst.data {
                // _in_arena variants share their bodies with the root forms;
                // they accept an extra leading arena pointer that the verifier
                // C model ignores (arenas are opaque).
                let arena_offset = if matches!(
                    name.as_str(),
                    "__vow_map_new_in_arena"
                        | "__vow_map_insert_in_arena"
                        | "__vow_map_remove_in_arena"
                ) {
                    1
                } else {
                    0
                };
                let arg = |i: usize| inst.args[arena_offset + i].0;
                match name.as_str() {
                    "__vow_map_new" | "__vow_map_new_in_arena" => {
                        out.push_str(&format!("  v{id}.len = 0;\n"));
                    }
                    "__vow_map_len" => {
                        let m = arg(0);
                        out.push_str(&format!("  v{id} = v{m}.len;\n"));
                    }
                    "__vow_map_insert" | "__vow_map_insert_in_arena" => {
                        let m = arg(0);
                        let k = arg(1);
                        let v = arg(2);
                        let hashmap_max = limits.hashmap_max;
                        out.push_str(&format!(
                            "  {{\n\
                             \x20   _Bool __found = 0;\n\
                             \x20   for (int64_t __i = 0; __i < v{m}.len; __i++) {{\n\
                             \x20     if (v{m}.keys[__i] == v{k}) {{ v{m}.vals[__i] = v{v}; __found = 1; break; }}\n\
                             \x20   }}\n\
                             \x20   if (!__found) {{\n\
                             \x20     __ESBMC_assert(v{m}.len < {hashmap_max}, \"hashmap capacity\");\n\
                             \x20     v{m}.keys[v{m}.len] = v{k}; v{m}.vals[v{m}.len] = v{v}; v{m}.len++;\n\
                             \x20   }}\n\
                             \x20 }}\n"
                        ));
                    }
                    "__vow_map_get" => {
                        let m = arg(0);
                        let k = arg(1);
                        out.push_str(&format!(
                            "  v{id} = 0;\n\
                             \x20 for (int64_t __i = 0; __i < v{m}.len; __i++) {{\n\
                             \x20   if (v{m}.keys[__i] == v{k}) {{ v{id} = v{m}.vals[__i]; break; }}\n\
                             \x20 }}\n"
                        ));
                    }
                    "__vow_map_contains" => {
                        let m = arg(0);
                        let k = arg(1);
                        out.push_str(&format!(
                            "  v{id} = 0;\n\
                             \x20 for (int64_t __i = 0; __i < v{m}.len; __i++) {{\n\
                             \x20   if (v{m}.keys[__i] == v{k}) {{ v{id} = 1; break; }}\n\
                             \x20 }}\n"
                        ));
                    }
                    "__vow_map_remove" | "__vow_map_remove_in_arena" => {
                        let m = arg(0);
                        let k = arg(1);
                        out.push_str(&format!(
                            "  for (int64_t __i = 0; __i < v{m}.len; __i++) {{\n\
                             \x20   if (v{m}.keys[__i] == v{k}) {{\n\
                             \x20     v{m}.keys[__i] = v{m}.keys[v{m}.len - 1];\n\
                             \x20     v{m}.vals[__i] = v{m}.vals[v{m}.len - 1];\n\
                             \x20     v{m}.len--;\n\
                             \x20     break;\n\
                             \x20   }}\n\
                             \x20 }}\n"
                        ));
                    }
                    _ => {
                        emit_unmodelled(inst, out);
                    }
                }
            }
        }

        // BTreeMap ops — linear-scan C model (sorted early-exit, cheaper to unwind than binary search); insert/get return Option.
        Opcode::Call if matches!(&inst.data, InstData::CallExtern(n) if n.starts_with("__vow_btreemap_")) =>
        {
            if let InstData::CallExtern(ref name) = inst.data {
                match name.as_str() {
                    "__vow_btreemap_new" => {
                        out.push_str(&format!("  v{id}.len = 0;\n"));
                    }
                    "__vow_btreemap_len" => {
                        let m = inst.args[0].0;
                        out.push_str(&format!("  v{id} = v{m}.len;\n"));
                    }
                    "__vow_btreemap_insert" => {
                        let m = inst.args[0].0;
                        let k = inst.args[1].0;
                        let v = inst.args[2].0;
                        let btreemap_max = limits.btreemap_max;
                        // Sorted-insert: replace value if key found (return Some(prev)), else shift+insert (return None).
                        out.push_str(&format!(
                            "  v{id}.tag = 0; v{id}.payload = 0;\n\
                             \x20 {{\n\
                             \x20   _Bool __found = 0;\n\
                             \x20   int64_t __pos = v{m}.len;\n\
                             \x20   for (int64_t __i = 0; __i < v{m}.len; __i++) {{\n\
                             \x20     if (v{m}.keys[__i] == v{k}) {{\n\
                             \x20       v{id}.tag = 1; v{id}.payload = v{m}.vals[__i];\n\
                             \x20       v{m}.vals[__i] = v{v};\n\
                             \x20       __found = 1; break;\n\
                             \x20     }}\n\
                             \x20     if (v{m}.keys[__i] > v{k}) {{ __pos = __i; break; }}\n\
                             \x20   }}\n\
                             \x20   if (!__found) {{\n\
                             \x20     __ESBMC_assert(v{m}.len < {btreemap_max}, \"btreemap capacity\");\n\
                             \x20     for (int64_t __j = v{m}.len; __j > __pos; __j--) {{\n\
                             \x20       v{m}.keys[__j] = v{m}.keys[__j - 1];\n\
                             \x20       v{m}.vals[__j] = v{m}.vals[__j - 1];\n\
                             \x20     }}\n\
                             \x20     v{m}.keys[__pos] = v{k}; v{m}.vals[__pos] = v{v}; v{m}.len++;\n\
                             \x20   }}\n\
                             \x20 }}\n"
                        ));
                    }
                    "__vow_btreemap_get" => {
                        let m = inst.args[0].0;
                        let k = inst.args[1].0;
                        out.push_str(&format!(
                            "  v{id}.tag = 0; v{id}.payload = 0;\n\
                             \x20 for (int64_t __i = 0; __i < v{m}.len; __i++) {{\n\
                             \x20   if (v{m}.keys[__i] == v{k}) {{ v{id}.tag = 1; v{id}.payload = v{m}.vals[__i]; break; }}\n\
                             \x20   if (v{m}.keys[__i] > v{k}) {{ break; }}\n\
                             \x20 }}\n"
                        ));
                    }
                    "__vow_btreemap_contains" => {
                        let m = inst.args[0].0;
                        let k = inst.args[1].0;
                        out.push_str(&format!(
                            "  v{id} = 0;\n\
                             \x20 for (int64_t __i = 0; __i < v{m}.len; __i++) {{\n\
                             \x20   if (v{m}.keys[__i] == v{k}) {{ v{id} = 1; break; }}\n\
                             \x20   if (v{m}.keys[__i] > v{k}) {{ break; }}\n\
                             \x20 }}\n"
                        ));
                    }
                    _ => {
                        emit_unmodelled(inst, out);
                    }
                }
            }
        }

        // Constant-function inlining: replace CallTarget with the known constant
        Opcode::Call if matches!(&inst.data, InstData::CallTarget(fid) if const_fns.contains_key(fid)) => {
            if let InstData::CallTarget(fid) = &inst.data {
                let val = &const_fns[fid];
                match val {
                    ConstantValue::I32(v) => {
                        out.push_str(&format!("  v{} = {};\n", id, v));
                    }
                    ConstantValue::I64(v) => {
                        out.push_str(&format!("  v{} = {}LL;\n", id, v));
                    }
                    ConstantValue::Bool(v) => {
                        out.push_str(&format!("  v{} = {};\n", id, *v as i32));
                    }
                }
            }
        }

        // Modelable function calls: emit actual C function call
        Opcode::Call if matches!(&inst.data, InstData::CallTarget(fid) if modelable_fns.contains(fid)) => {
            if let InstData::CallTarget(fid) = &inst.data
                && let Some(callee) = module.functions.iter().find(|f| f.id == *fid)
            {
                let mut args_str = Vec::new();
                for (i, arg) in inst.args.iter().enumerate() {
                    if i < callee.params.len() && callee.params[i] != Ty::Unit {
                        args_str.push(format!("v{}", arg.0));
                    }
                }
                if inst.ty != Ty::Unit {
                    out.push_str(&format!(
                        "  v{} = {}({});\n",
                        id,
                        verifier_c_func_name(callee),
                        args_str.join(", ")
                    ));
                } else {
                    out.push_str(&format!(
                        "  {}({});\n",
                        verifier_c_func_name(callee),
                        args_str.join(", ")
                    ));
                }
            }
        }

        Opcode::RegionOpen | Opcode::RegionClose => {
            out.push_str("  /* verifier no-op: region scope marker */\n");
        }

        // Other calls, memory, linear ops — not yet supported for verification
        Opcode::Call
        | Opcode::Load
        | Opcode::Store
        | Opcode::LinearConsume
        | Opcode::LinearBorrow => {
            emit_unsupported_for_verification(inst, out);
        }

        // User-struct heap model: bump-allocate `size/8` int64 slots.
        Opcode::RegionAlloc => {
            let slots = match inst.data {
                InstData::AllocSize { size, .. } => (size / 8).max(1),
                _ => 1,
            };
            let heap_max = limits.heap_max;
            out.push_str(&format!(
                "  v{id} = __vow_heap_top;\n  __vow_heap_top += {slots};\n\
                 \x20 __ESBMC_assume(__vow_heap_top <= {heap_max});\n"
            ));
        }

        // User-struct heap model: store into base's field slot.
        Opcode::FieldSet => {
            let idx = match inst.data {
                InstData::FieldIndex(i) => i,
                _ => 0,
            };
            let base = inst.args.first().map_or(0, |a| a.0);
            let val = inst.args.get(1).map_or(0, |a| a.0);
            out.push_str(&format!("  __vow_heap[v{base} + {idx}] = v{val};\n"));
        }
        Opcode::FieldGet => {
            if vec_vars.contains(&id) {
                let vec_max = limits.vec_max;
                out.push_str(&format!(
                    "  /* FieldGet -> vec */ v{id}.len = __VERIFIER_nondet_long();\n\
                     \x20 __ESBMC_assume(v{id}.len >= 0 && v{id}.len <= {vec_max});\n"
                ));
            } else if string_vars.contains(&id) {
                let string_max = limits.string_max;
                out.push_str(&format!(
                    "  /* FieldGet -> string */ v{id}.len = __VERIFIER_nondet_long();\n\
                     \x20 __ESBMC_assume(v{id}.len >= 0 && v{id}.len <= {string_max});\n"
                ));
            } else if hashmap_vars.contains(&id) {
                let hashmap_max = limits.hashmap_max;
                out.push_str(&format!(
                    "  /* FieldGet -> hashmap */ v{id}.len = __VERIFIER_nondet_long();\n\
                     \x20 __ESBMC_assume(v{id}.len >= 0 && v{id}.len <= {hashmap_max});\n"
                ));
            } else if btreemap_vars.contains(&id) {
                let btreemap_max = limits.btreemap_max;
                // Sorted-keys assume: get/contains/insert C model requires ascending-key state.
                out.push_str(&format!(
                    "  /* FieldGet -> btreemap */ v{id}.len = __VERIFIER_nondet_long();\n\
                     \x20 __ESBMC_assume(v{id}.len >= 0 && v{id}.len <= {btreemap_max});\n\
                     \x20 for (int64_t __si = 0; __si + 1 < v{id}.len; __si++)\n\
                     \x20   __ESBMC_assume(v{id}.keys[__si] < v{id}.keys[__si + 1]);\n"
                ));
            } else if let Some(&src_id) = inst.args.first() {
                if option_vars.contains(&src_id.0) {
                    if let InstData::FieldIndex(idx) = inst.data {
                        if idx == 0 {
                            out.push_str(&format!("  v{id} = v{}.tag;\n", src_id.0));
                        } else {
                            out.push_str(&format!("  v{id} = v{}.payload;\n", src_id.0));
                        }
                    } else {
                        emit_unmodelled(inst, out);
                    }
                } else if let InstData::FieldIndex(idx) = inst.data {
                    // User-struct heap model: load base's field slot.
                    out.push_str(&format!("  v{id} = __vow_heap[v{} + {idx}];\n", src_id.0));
                } else {
                    emit_unmodelled(inst, out);
                }
            } else {
                emit_unmodelled(inst, out);
            }
        }

        Opcode::RemF32 | Opcode::RemF64 => {
            out.push_str(&format!("  /* float rem not modelled */ v{} = 0;\n", id));
        }

        Opcode::DebugCall => {
            // Debug prints are no-ops for verification
        }
    }
}

fn emit_unmodelled(inst: &Inst, out: &mut String) {
    let id = inst.id.0;
    out.push_str(&format!("  /* opcode {:?} not modelled */\n", inst.opcode));
    if inst.ty != Ty::Unit {
        out.push_str(&format!(
            "  v{} = __VERIFIER_nondet_{}();\n",
            id,
            c_nondet_suffix(inst.ty)
        ));
    }
}

/// Collect every unordered (lo, hi) operand pair appearing as arguments to
/// `__vow_string_eq`, in IR-traversal order with linear deduplication. The
/// caller emits one shared `_Bool __str_eq_<lo>_<hi>` per pair, and re-samples
/// it whenever a modeled mutation touches `lo` or `hi`.
fn compute_string_eq_pairs(func: &Function) -> Vec<(u32, u32)> {
    let mut eq_pairs: Vec<(u32, u32)> = Vec::new();
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Call
                && let InstData::CallExtern(ref name) = inst.data
                && name == "__vow_string_eq"
                && inst.args.len() == 2
            {
                let a = inst.args[0].0;
                let b = inst.args[1].0;
                if a != b {
                    let pair = (a.min(b), a.max(b));
                    if !eq_pairs.contains(&pair) {
                        eq_pairs.push(pair);
                    }
                }
            }
        }
    }
    eq_pairs
}

/// Emit `__str_eq_<lo>_<hi> = __VERIFIER_nondet_bool();` for every cached pair
/// involving `operand`. Called immediately after each modeled string mutation
/// so the verifier cannot prove a stale equality across a mutation.
fn emit_string_eq_invalidate(operand: u32, eq_pairs: &[(u32, u32)], out: &mut String) {
    for &(lo, hi) in eq_pairs {
        if lo == operand || hi == operand {
            out.push_str(&format!(
                "  __str_eq_{lo}_{hi} = __VERIFIER_nondet_bool();\n"
            ));
        }
    }
}

fn emit_nondet_string_len(id: u32, string_max: usize, out: &mut String) {
    out.push_str(&format!(
        "  v{id}.len = __VERIFIER_nondet_long();\n\
         \x20 __ESBMC_assume(v{id}.len >= 0 && v{id}.len < {string_max});\n",
    ));
}

/// Sentinel `vow_id` reported when ESBMC fails on an
/// `emit_unsupported_for_verification` assertion. Reserved so the diagnostic
/// pipeline can distinguish a verifier-limitation failure from a real vow
/// violation; never assigned to a user-authored vow.
pub const UNSUPPORTED_OP_VOW_ID: u32 = u32::MAX;

/// Sentinel `vow_id` reported when a co-emitted callee's `requires` is violated
/// by its caller. Reserved (never a user vow id) so the diagnostic pipeline maps
/// it to a Caller-blamed "callee precondition violated" without a per-function
/// vow lookup. Using the callee's own (function-local) vow id here would collide
/// with the target function's vow of the same index and mislabel the violation.
pub const CALLER_PRECONDITION_VOW_ID: u32 = u32::MAX - 1;

fn emit_unsupported_for_verification(inst: &Inst, out: &mut String) {
    let id = inst.id.0;
    // The assertion text must be exactly `vow:<id>` — `extract_vow_id` calls
    // `parse::<u32>()` on the suffix, which rejects anything but pure digits.
    // The descriptive text goes in a separate C comment so the verifier model
    // stays human-readable while the violated-property line remains parseable.
    out.push_str(&format!(
        "  /* unsupported opcode in verifier model: {:?} */\n",
        inst.opcode
    ));
    out.push_str(&format!(
        "  __ESBMC_assert(0, \"vow:{UNSUPPORTED_OP_VOW_ID}\");\n",
    ));
    if inst.ty != Ty::Unit {
        out.push_str(&format!(
            "  v{} = __VERIFIER_nondet_{}();\n",
            id,
            c_nondet_suffix(inst.ty)
        ));
    }
}

fn c_nondet_suffix(ty: Ty) -> &'static str {
    match ty {
        Ty::I8 => "char",
        Ty::U8 => "unsigned_char",
        Ty::I16 => "short",
        Ty::U16 => "unsigned_short",
        Ty::I32 => "int",
        Ty::U32 => "unsigned_int",
        Ty::I64 => "long",
        Ty::U64 => "unsigned_long",
        Ty::I128 => "int128",
        Ty::U128 => "uint128",
        Ty::F32 => "float",
        Ty::F64 => "double",
        Ty::Bool => "bool",
        Ty::Ptr | Ty::LinearPtr => "long",
        Ty::Unit => "int",
    }
}

// ---------------------------------------------------------------------------
// Function emission
// ---------------------------------------------------------------------------

pub fn emit_c_function(
    func: &Function,
    const_fns: &HashMap<FuncId, ConstantValue>,
    limits: &VerifyLimits,
) -> String {
    emit_c_function_full(
        func,
        const_fns,
        &HashSet::new(),
        &Module {
            name: String::new(),
            functions: vec![],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        },
        limits,
        // Standalone single-function verification: the function is the target,
        // so it carries no vacuity `vow_reach` label, no body-replace rewrite,
        // and its `requires` are assumed (not asserted).
        false, // reach_label
        false, // body_replace
        false, // requires_as_assert
    )
}

#[allow(clippy::too_many_arguments)]
pub fn emit_c_function_full(
    func: &Function,
    const_fns: &HashMap<FuncId, ConstantValue>,
    modelable_fns: &HashSet<FuncId>,
    module: &Module,
    limits: &VerifyLimits,
    reach_label: bool,
    body_replace: bool,
    requires_as_assert: bool,
) -> String {
    let mut out = String::new();

    // Vacuity detection (#81 PR-B): when `reach_label` is set, emit a `vow_reach`
    // label immediately after the last `requires` assume in the entry block.
    // The label is reachable iff the preconditions are satisfiable; ESBMC
    // `--error-label vow_reach` then maps unreachable -> contradictory requires
    // -> vacuous contract. Placing it right after the requires prefix (not at the
    // function end) keeps body divergence — unbounded loops, assume(0) — from
    // making the label spuriously unreachable.
    let reach_after: Option<u32> = if reach_label {
        func.blocks.first().and_then(|b| {
            b.insts
                .iter()
                .rfind(|i| i.opcode == Opcode::VowRequires)
                .map(|i| i.id.0)
        })
    } else {
        None
    };

    // Weakness detection (#81 PR-C): when `body_replace` is set, overwrite the
    // returned value with the type-default right after it is computed, so each
    // `ensures` is checked against a trivial `return <default>` implementation.
    // If ESBMC still proves the ensures, a constant-returning body satisfies the
    // contract — it is too weak to pin down the implementation. `result_after`
    // is the id of the value the `Return` yields (which the `ensures` predicate
    // references). Callers only set `body_replace` when this id names a regular
    // body instruction of scalar type (see `emit_bodyreplace_c_source`).
    let result_after: Option<u32> = if body_replace {
        func.blocks
            .iter()
            .flat_map(|b| &b.insts)
            .find(|i| i.opcode == Opcode::Return)
            .and_then(|ret| ret.args.first().map(|a| a.0))
    } else {
        None
    };
    let vec_vars = collect_typed_vars(func, "__vow_vec_new", "__vow_vec_");
    let string_vars = collect_typed_vars(func, "__vow_string_new", "__vow_string_");
    let hashmap_vars = collect_typed_vars(func, "__vow_map_new", "__vow_map_");
    let btreemap_vars = collect_typed_vars(func, "__vow_btreemap_new", "__vow_btreemap_");
    let option_vars = collect_option_vars(func);
    let mut const_str_indices: HashMap<u32, u32> = HashMap::new();
    for block in &func.blocks {
        for inst in &block.insts {
            if let InstData::ConstStr(idx) = inst.data {
                const_str_indices.insert(inst.id.0, idx);
            }
        }
    }

    // Return type (use int64_t for Ptr since structs are opaque in verification)
    let ret_c = match func.return_ty {
        Ty::Unit => "void",
        Ty::Ptr | Ty::LinearPtr => "int64_t",
        other => ir_ty_to_c(other),
    };

    // Parameters (skip Unit params; use int64_t for Ptr)
    let params: Vec<String> = func
        .params
        .iter()
        .enumerate()
        .filter(|&(_, &ty)| ty != Ty::Unit)
        .map(|(i, &ty)| {
            let c_ty = match ty {
                Ty::Ptr | Ty::LinearPtr => "int64_t",
                other => ir_ty_to_c(other),
            };
            format!("{} p{}", c_ty, i)
        })
        .collect();
    let param_str = if params.is_empty() {
        "void".to_string()
    } else {
        params.join(", ")
    };

    out.push_str(&format!(
        "{} {}({}) {{\n",
        ret_c,
        verifier_c_func_name(func),
        param_str
    ));

    // Map arg index to parameter name at the top of the function
    // GetArg(idx) refers to p{cl_idx} where cl_idx skips Unit params
    let mut arg_var_map: Vec<(u32, u32)> = Vec::new(); // (ir_idx, cl_idx)
    let mut cl_idx = 0u32;
    for (ir_idx, &ty) in func.params.iter().enumerate() {
        if ty != Ty::Unit {
            arg_var_map.push((ir_idx as u32, cl_idx));
            cl_idx += 1;
        }
    }

    // Emit GetArg aliases
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::GetArg
                && let InstData::ArgIndex(idx) = inst.data
            {
                let id = inst.id.0;
                if let Some(&(_, cl)) = arg_var_map.iter().find(|(ir, _)| *ir == idx) {
                    if vec_vars.contains(&id) {
                        let vec_max = limits.vec_max;
                        out.push_str(&format!(
                            "  __vow_vec_t v{id};\n  v{id}.len = __VERIFIER_nondet_long();\n\
                             \x20 __ESBMC_assume(v{id}.len >= 0 && v{id}.len <= {vec_max});\n"
                        ));
                    } else if string_vars.contains(&id) {
                        let string_max = limits.string_max;
                        out.push_str(&format!(
                            "  __vow_string_t v{id};\n  v{id}.len = __VERIFIER_nondet_long();\n\
                             \x20 __ESBMC_assume(v{id}.len >= 0 && v{id}.len <= {string_max});\n"
                        ));
                    } else if hashmap_vars.contains(&id) {
                        let hashmap_max = limits.hashmap_max;
                        out.push_str(&format!(
                            "  __vow_hashmap_t v{id};\n  v{id}.len = __VERIFIER_nondet_long();\n\
                             \x20 __ESBMC_assume(v{id}.len >= 0 && v{id}.len <= {hashmap_max});\n"
                        ));
                    } else if btreemap_vars.contains(&id) {
                        let btreemap_max = limits.btreemap_max;
                        // Sorted-keys assume: get/contains/insert C model requires ascending-key state.
                        out.push_str(&format!(
                            "  __vow_btreemap_t v{id};\n  v{id}.len = __VERIFIER_nondet_long();\n\
                             \x20 __ESBMC_assume(v{id}.len >= 0 && v{id}.len <= {btreemap_max});\n\
                             \x20 for (int64_t __si = 0; __si + 1 < v{id}.len; __si++)\n\
                             \x20   __ESBMC_assume(v{id}.keys[__si] < v{id}.keys[__si + 1]);\n"
                        ));
                    } else {
                        let c_ty = match inst.ty {
                            Ty::Ptr | Ty::LinearPtr => "int64_t",
                            other => ir_ty_to_c(other),
                        };
                        out.push_str(&format!("  {} v{} = p{};\n", c_ty, id, cl));
                    }
                } else {
                    out.push_str(&format!("  int32_t v{} = 0; /* unit arg */\n", id));
                }
            }
        }
    }

    // Pre-declare ALL instruction variables at function scope.
    // This prevents C99 goto/scope errors when declarations appear inside
    // goto-labeled blocks (e.g. `let mut` inside loop bodies).
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::GetArg
                || inst.opcode == Opcode::Upsilon
                || inst.opcode.is_terminal()
                || inst.opcode == Opcode::VowRequires
                || inst.opcode == Opcode::VowEnsures
                || inst.opcode == Opcode::VowInvariant
            {
                continue;
            }
            if inst.ty == Ty::Unit && inst.opcode != Opcode::ConstUnit && inst.opcode != Opcode::Phi
            {
                continue;
            }
            let id = inst.id.0;
            if vec_vars.contains(&id) {
                out.push_str(&format!("  __vow_vec_t v{};\n", id));
            } else if string_vars.contains(&id) {
                out.push_str(&format!("  __vow_string_t v{};\n", id));
            } else if hashmap_vars.contains(&id) {
                out.push_str(&format!("  __vow_hashmap_t v{};\n", id));
            } else if btreemap_vars.contains(&id) {
                out.push_str(&format!("  __vow_btreemap_t v{};\n", id));
            } else if option_vars.contains(&id) {
                out.push_str(&format!("  __vow_option_t v{};\n", id));
            } else {
                let c_ty = match inst.ty {
                    Ty::Unit => "int32_t",
                    Ty::Ptr | Ty::LinearPtr => "int64_t",
                    other => ir_ty_to_c(other),
                };
                out.push_str(&format!("  {} v{};\n", c_ty, id));
            }
        }
    }

    // User-struct heap model: a per-function slot array + bump pointer, emitted
    // only when the body has struct allocation/field ops. Pointers are int64
    // slot indices, so equal addresses alias the same slots (sound aliasing);
    // each RegionAlloc bumps to a fresh region (distinct objects).
    let needs_heap = func.blocks.iter().flat_map(|b| &b.insts).any(|i| {
        matches!(i.opcode, Opcode::RegionAlloc | Opcode::FieldSet)
            || (i.opcode == Opcode::FieldGet
                && !vec_vars.contains(&i.id.0)
                && !string_vars.contains(&i.id.0)
                && !hashmap_vars.contains(&i.id.0)
                && !btreemap_vars.contains(&i.id.0)
                && !option_vars.contains(&i.args.first().map_or(u32::MAX, |a| a.0)))
    });
    if needs_heap {
        out.push_str(&format!(
            "  int64_t __vow_heap[{heap}];\n  int64_t __vow_heap_top = 0;\n",
            heap = limits.heap_max
        ));
    }

    // Pre-declare Upsilon temporaries at function scope
    {
        let mut ups_sources: Vec<u32> = Vec::new();
        for block in &func.blocks {
            for inst in &block.insts {
                if inst.opcode == Opcode::Upsilon
                    && let InstData::PhiTarget(_) = inst.data
                    && !inst.args.is_empty()
                    && !ups_sources.contains(&inst.args[0].0)
                {
                    ups_sources.push(inst.args[0].0);
                }
            }
        }
        ups_sources.sort();
        for src in ups_sources {
            // Upsilon temps must share the source's struct type; struct-to-int64 assignment corrupts the payload.
            if option_vars.contains(&src) {
                out.push_str(&format!("  __vow_option_t __ups_{};\n", src));
            } else if vec_vars.contains(&src) {
                out.push_str(&format!("  __vow_vec_t __ups_{};\n", src));
            } else if string_vars.contains(&src) {
                out.push_str(&format!("  __vow_string_t __ups_{};\n", src));
            } else if hashmap_vars.contains(&src) {
                out.push_str(&format!("  __vow_hashmap_t __ups_{};\n", src));
            } else if btreemap_vars.contains(&src) {
                out.push_str(&format!("  __vow_btreemap_t __ups_{};\n", src));
            } else {
                out.push_str(&format!("  int64_t __ups_{};\n", src));
            }
        }
    }

    // Per-pair nondet cache for abstract __vow_string_eq. A fresh
    // __VERIFIER_nondet_bool() on every call would let ESBMC pick different
    // values for the same (a,b) pair, breaking determinism (e.g. body proves
    // `a.eq(b)` then `ensures: a.eq(b)` fails). Declare one shared bool per
    // unordered pair (min, max) and reuse it at every call site, then
    // re-sample after each modeled mutation on either operand.
    let eq_pairs = compute_string_eq_pairs(func);
    for &(lo, hi) in &eq_pairs {
        out.push_str(&format!(
            "  _Bool __str_eq_{lo}_{hi} = __VERIFIER_nondet_bool();\n"
        ));
    }
    let mut inst_by_id: HashMap<u32, &Inst> = HashMap::new();
    for block in &func.blocks {
        for inst in &block.insts {
            inst_by_id.insert(inst.id.0, inst);
        }
    }

    // Block-visit tracking variables
    for block in &func.blocks {
        out.push_str(&format!("  int __blk_{} = 0;\n", block.id.0));
    }

    // Emit blocks
    let first_block_id = func.blocks.first().map(|b| b.id);
    for block in &func.blocks {
        if Some(block.id) != first_block_id {
            out.push_str(&format!("block{}:;\n", block.id.0));
        }
        out.push_str(&format!("  __blk_{} = 1;\n", block.id.0));
        // Partition block instructions into: regular, upsilons, terminal.
        // In Pizlo-style IR, Upsilons can appear after the terminal and
        // multiple Upsilons can conflict (one writes a Phi that another
        // reads).  Fix both by: (1) moving post-terminal Upsilons before
        // the terminal, and (2) reading all Upsilon sources into temps
        // before writing any targets.
        let mut regular: Vec<&Inst> = Vec::new();
        let mut upsilons: Vec<(u32, u32)> = Vec::new(); // (phi_id, source_val)
        let mut terminal: Option<&Inst> = None;
        for inst in &block.insts {
            if inst.opcode == Opcode::GetArg {
                continue;
            }
            if inst.opcode == Opcode::Upsilon {
                if let InstData::PhiTarget(phi_id) = inst.data {
                    upsilons.push((phi_id.0, inst.args[0].0));
                }
                continue;
            }
            if inst.opcode.is_terminal() {
                terminal = Some(inst);
                continue;
            }
            regular.push(inst);
        }
        for inst in &regular {
            emit_inst(
                inst,
                &mut out,
                &vec_vars,
                &string_vars,
                &hashmap_vars,
                &btreemap_vars,
                &option_vars,
                const_fns,
                modelable_fns,
                &const_str_indices,
                &eq_pairs,
                &inst_by_id,
                module,
                limits,
                func.return_ty,
                func.id,
                requires_as_assert,
            );
            if reach_after == Some(inst.id.0) {
                out.push_str("vow_reach:;\n");
            }
            if result_after == Some(inst.id.0) {
                out.push_str(&format!("  v{} = 0;\n", inst.id.0));
            }
        }
        // Emit Upsilons: read all sources first, then write all targets.
        if !upsilons.is_empty() {
            for &(_, src) in &upsilons {
                out.push_str(&format!("  __ups_{src} = v{src};\n"));
            }
            for &(phi, src) in &upsilons {
                out.push_str(&format!("  v{phi} = __ups_{src};\n"));
            }
        }
        if let Some(term) = terminal {
            emit_inst(
                term,
                &mut out,
                &vec_vars,
                &string_vars,
                &hashmap_vars,
                &btreemap_vars,
                &option_vars,
                const_fns,
                modelable_fns,
                &const_str_indices,
                &eq_pairs,
                &inst_by_id,
                module,
                limits,
                func.return_ty,
                func.id,
                requires_as_assert,
            );
        }
    }

    out.push_str("}\n");
    out
}

/// Set of `(is_shl, signedness, width)` shift-helper flavors actually used by
/// the module.
type ShiftNeeds = std::collections::BTreeSet<(bool, IntegerSignedness, IntegerWidth)>;

fn scan_shift_needs(funcs: &[&Function]) -> ShiftNeeds {
    let mut needs = ShiftNeeds::new();
    for func in funcs {
        for block in &func.blocks {
            for inst in &block.insts {
                let is_shl = match inst.opcode {
                    Opcode::Shl => true,
                    Opcode::Shr => false,
                    _ => continue,
                };
                if let InstData::Integer(IntegerType { width, signedness }) = inst.data {
                    needs.insert((is_shl, signedness, width));
                }
            }
        }
    }
    needs
}

/// The width/signedness-specialized C helpers a module's model needs in its
/// preamble. Both sets are derived from one IR walk per emitted module, so the
/// preamble emits exactly the flavors used and nothing more.
struct ModelHelpers {
    shifts: ShiftNeeds,
    arith: ArithNeeds,
}

impl ModelHelpers {
    fn scan(funcs: &[&Function]) -> Self {
        Self {
            shifts: scan_shift_needs(funcs),
            arith: scan_arith_needs(funcs),
        }
    }
}

fn c_uint_type(width: IntegerWidth) -> &'static str {
    match width {
        IntegerWidth::W8 => "uint8_t",
        IntegerWidth::W16 => "uint16_t",
        IntegerWidth::W32 => "uint32_t",
        IntegerWidth::W64 => "uint64_t",
        IntegerWidth::W128 => "unsigned __int128",
    }
}

fn c_int_type(width: IntegerWidth) -> &'static str {
    match width {
        IntegerWidth::W8 => "int8_t",
        IntegerWidth::W16 => "int16_t",
        IntegerWidth::W32 => "int32_t",
        IntegerWidth::W64 => "int64_t",
        IntegerWidth::W128 => "__int128",
    }
}

/// Emits one `__vow_{shl,shr}_{i,u}<bits>` helper. Mirrors hardware shift
/// semantics for any shift count (masked to `width - 1`, matching Cranelift's
/// `ishl`/`sshr`/`ushr`) rather than C's undefined behavior for
/// out-of-range/negative shift counts.
fn emit_shift_helper(
    out: &mut String,
    is_shl: bool,
    signedness: IntegerSignedness,
    width: IntegerWidth,
) {
    let bits = width.bits();
    let mask = bits - 1;
    let uint_ty = c_uint_type(width);
    let int_ty = c_int_type(width);
    let prefix = match signedness {
        IntegerSignedness::Signed => "i",
        IntegerSignedness::Unsigned => "u",
    };
    let op = if is_shl { "shl" } else { "shr" };
    match (is_shl, signedness) {
        (true, IntegerSignedness::Signed) => out.push_str(&format!(
            "static inline {int_ty} __vow_{op}_{prefix}{bits}({int_ty} value, {int_ty} count) {{\n\
             \x20 {uint_ty} shift = (({uint_ty})count) & {mask};\n\
             \x20 return ({int_ty})((({uint_ty})value) << shift);\n\
             }}\n"
        )),
        (false, IntegerSignedness::Signed) => out.push_str(&format!(
            "static inline {int_ty} __vow_{op}_{prefix}{bits}({int_ty} value, {int_ty} count) {{\n\
             \x20 {uint_ty} shift = (({uint_ty})count) & {mask};\n\
             \x20 {uint_ty} raw_bits = ({uint_ty})value;\n\
             \x20 {uint_ty} logical = raw_bits >> shift;\n\
             \x20 {uint_ty} ones = ~(({uint_ty})0);\n\
             \x20 {uint_ty} sign_fill = value < 0 ? (({uint_ty})~(ones >> shift)) : ({uint_ty})0;\n\
             \x20 return ({int_ty})(logical | sign_fill);\n\
             }}\n"
        )),
        (true, IntegerSignedness::Unsigned) => out.push_str(&format!(
            "static inline {uint_ty} __vow_{op}_{prefix}{bits}({uint_ty} value, {uint_ty} count) {{\n\
             \x20 return value << (count & {mask});\n\
             }}\n"
        )),
        (false, IntegerSignedness::Unsigned) => out.push_str(&format!(
            "static inline {uint_ty} __vow_{op}_{prefix}{bits}({uint_ty} value, {uint_ty} count) {{\n\
             \x20 return value >> (count & {mask});\n\
             }}\n"
        )),
    }
}

// ---------------------------------------------------------------------------
// Checked-arithmetic abort model (#585)
// ---------------------------------------------------------------------------

/// Which abort a checked-arithmetic site can take. Vow's checked operators
/// abort with `ArithmeticOverflow` instead of wrapping (`docs/spec/grammar.md`,
/// "Checked Arithmetic"), and the cause is worth distinguishing in the
/// diagnostic: a divisor of zero is a different authoring mistake from a product
/// that does not fit.
///
/// The `label` strings are the second field of the `arith:` property label and
/// are parsed back by `vow-verify::esbmc::parse_arith_label`; they are a wire
/// format shared with `compiler/c_emitter.vow` and must not be renamed casually.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ArithAbort {
    Add,
    Sub,
    Mul,
    /// `/!` or `%!` with a zero divisor.
    DivZero,
    /// Signed `/!` on `MIN / -1`, whose quotient is not representable.
    DivOverflow,
}

impl ArithAbort {
    pub fn label(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Sub => "sub",
            Self::Mul => "mul",
            Self::DivZero => "div-zero",
            Self::DivOverflow => "div-overflow",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "add" => Some(Self::Add),
            "sub" => Some(Self::Sub),
            "mul" => Some(Self::Mul),
            "div-zero" => Some(Self::DivZero),
            "div-overflow" => Some(Self::DivOverflow),
            _ => None,
        }
    }

    /// Human-readable cause, used verbatim in the `ArithOverflowReachable`
    /// diagnostic message.
    pub fn description(self) -> &'static str {
        match self {
            Self::Add => "addition overflows",
            Self::Sub => "subtraction overflows",
            Self::Mul => "multiplication overflows",
            Self::DivZero => "divisor is zero",
            Self::DivOverflow => "quotient is not representable (MIN / -1)",
        }
    }
}

/// Preprocessor macro that compiles the `arith:` obligations out of an emitted
/// model, leaving the contract properties as the only checkable ones. Defined by
/// [`contracts_only_source`]; the same name is emitted by
/// `compiler/c_emitter.vow`, so it is a wire format shared by both compilers.
pub const ARITH_ASSERT_SUPPRESS_MACRO: &str = "VOW_NO_ARITH_ASSERT";

/// Project an emitted model onto its *contract* obligations alone, by defining
/// [`ARITH_ASSERT_SUPPRESS_MACRO`].
///
/// ESBMC reports one violated property per run, so a function whose checked
/// arithmetic can abort would otherwise mask its own contract verdict: the
/// `arith:` property fails first and the run never says whether the `ensures`
/// holds. Re-running this projection answers that second question. The abort
/// semantics are unchanged — only the asserts are suppressed, never the assumes
/// — so the verdict it yields is the verdict on returning executions, which is
/// exactly what a contract constrains.
///
/// Returning a distinct source (rather than passing `-D` on the command line)
/// keeps the verify cache honest: the two runs ask different questions and hash
/// to different keys.
pub fn contracts_only_source(c_src: &str) -> String {
    format!("#define {ARITH_ASSERT_SUPPRESS_MACRO} 1\n{c_src}")
}

/// Set of `(op, signedness, width)` overflow-guard helper flavors the module
/// actually uses. Mirrors [`ShiftNeeds`]; only `+!`/`-!`/`*!` need a helper,
/// since the `/!`/`%!` guards are single comparisons emitted inline.
type ArithNeeds = std::collections::BTreeSet<(ArithAbort, IntegerSignedness, IntegerWidth)>;

/// The helper flavor a checked opcode needs, or `None` for the div/rem forms
/// whose guards are emitted inline.
fn checked_helper_abort(opcode: Opcode) -> Option<ArithAbort> {
    match opcode {
        Opcode::CheckedAdd => Some(ArithAbort::Add),
        Opcode::CheckedSub => Some(ArithAbort::Sub),
        Opcode::CheckedMul => Some(ArithAbort::Mul),
        _ => None,
    }
}

fn scan_arith_needs(funcs: &[&Function]) -> ArithNeeds {
    let mut needs = ArithNeeds::new();
    for func in funcs {
        for block in &func.blocks {
            for inst in &block.insts {
                if let Some(abort) = checked_helper_abort(inst.opcode)
                    && let Some(int_ty) = checked_integer_type(inst)
                {
                    needs.insert((abort, int_ty.signedness, int_ty.width));
                }
            }
        }
    }
    needs
}

/// The integer type of a checked-arithmetic site. `InstData::Integer` is what
/// the lowerer attaches (`vow-ir/src/lower/mod.rs::binop_opcode`); the `inst.ty`
/// fallback keeps a hand-built or deserialized IR working. `None` means the site
/// is not a modelable integer op — 128-bit widths included, which the
/// `is_modelable` gate rejects so the function is reported `Skipped` rather than
/// modelled with the wrong guard.
fn checked_integer_type(inst: &Inst) -> Option<IntegerType> {
    let int_ty = match inst.data {
        InstData::Integer(int_ty) => int_ty,
        _ => ir_ty_to_integer_type(inst.ty)?,
    };
    match int_ty.width {
        IntegerWidth::W128 => None,
        _ => Some(int_ty),
    }
}

fn ir_ty_to_integer_type(ty: Ty) -> Option<IntegerType> {
    let parts = match ty {
        Ty::I8 => (IntegerWidth::W8, IntegerSignedness::Signed),
        Ty::U8 => (IntegerWidth::W8, IntegerSignedness::Unsigned),
        Ty::I16 => (IntegerWidth::W16, IntegerSignedness::Signed),
        Ty::U16 => (IntegerWidth::W16, IntegerSignedness::Unsigned),
        Ty::I32 => (IntegerWidth::W32, IntegerSignedness::Signed),
        Ty::U32 => (IntegerWidth::W32, IntegerSignedness::Unsigned),
        Ty::I64 => (IntegerWidth::W64, IntegerSignedness::Signed),
        Ty::U64 => (IntegerWidth::W64, IntegerSignedness::Unsigned),
        Ty::I128 => (IntegerWidth::W128, IntegerSignedness::Signed),
        Ty::U128 => (IntegerWidth::W128, IntegerSignedness::Unsigned),
        _ => return None,
    };
    Some(IntegerType::new(parts.0, parts.1))
}

/// Largest / smallest value of an integer type, as a C expression. Written as
/// literals rather than `<limits.h>` macros so the emitted model stays
/// self-contained, and `MIN` is spelled `(-MAX - 1)` because `-9223372036854775808`
/// is not a valid C integer literal (it parses as a negated `unsigned long long`).
fn c_int_max_literal(int_ty: IntegerType) -> String {
    let bits = int_ty.width.bits();
    match int_ty.signedness {
        IntegerSignedness::Unsigned => format!(
            "(({}){}ULL)",
            c_uint_type(int_ty.width),
            (u128::MAX >> (128 - bits))
        ),
        IntegerSignedness::Signed => format!(
            "(({}){}LL)",
            c_int_type(int_ty.width),
            (u128::MAX >> (128 - bits + 1))
        ),
    }
}

fn c_int_min_literal(int_ty: IntegerType) -> String {
    match int_ty.signedness {
        IntegerSignedness::Unsigned => format!("(({})0)", c_uint_type(int_ty.width)),
        IntegerSignedness::Signed => format!("(-{} - 1)", c_int_max_literal(int_ty)),
    }
}

fn arith_helper_name(abort: ArithAbort, int_ty: IntegerType) -> String {
    let prefix = match int_ty.signedness {
        IntegerSignedness::Signed => "i",
        IntegerSignedness::Unsigned => "u",
    };
    format!(
        "__vow_ovf_{}_{}{}",
        abort.label(),
        prefix,
        int_ty.width.bits()
    )
}

/// Emits one `__vow_ovf_{add,sub,mul}_{i,u}<bits>` predicate: `true` when the
/// operation would overflow, i.e. when the real program would abort.
///
/// Every guard is itself overflow-free in C, so the model never trips a nested
/// obligation on its own arithmetic: the `MAX - b` / `MIN - b` forms are only
/// evaluated for the sign of `b` that keeps them in range, and the multiply form
/// special-cases `0` and `-1` before dividing so it never evaluates `MIN / -1`.
/// The guards were validated against exhaustive ground truth over the whole
/// `i8`/`u8` domain; the formulas are width-generic.
fn emit_arith_helper(
    out: &mut String,
    abort: ArithAbort,
    signedness: IntegerSignedness,
    width: IntegerWidth,
) {
    let int_ty = IntegerType::new(width, signedness);
    let name = arith_helper_name(abort, int_ty);
    let c_ty = match signedness {
        IntegerSignedness::Signed => c_int_type(width),
        IntegerSignedness::Unsigned => c_uint_type(width),
    };
    let max = c_int_max_literal(int_ty);
    let min = c_int_min_literal(int_ty);
    let body = match (abort, signedness) {
        (ArithAbort::Add, IntegerSignedness::Unsigned) => format!("  return a > {max} - b;\n"),
        (ArithAbort::Sub, IntegerSignedness::Unsigned) => "  return a < b;\n".to_string(),
        (ArithAbort::Mul, IntegerSignedness::Unsigned) => {
            format!("  return b != 0 && a > {max} / b;\n")
        }
        (ArithAbort::Add, IntegerSignedness::Signed) => format!(
            "  if (b > 0) return a > {max} - b;\n  if (b < 0) return a < {min} - b;\n  return 0;\n"
        ),
        (ArithAbort::Sub, IntegerSignedness::Signed) => format!(
            "  if (b > 0) return a < {min} + b;\n  if (b < 0) return a > {max} + b;\n  return 0;\n"
        ),
        (ArithAbort::Mul, IntegerSignedness::Signed) => format!(
            "  if (a == 0 || b == 0) return 0;\n\
             \x20 if (a == -1) return b == {min};\n\
             \x20 if (b == -1) return a == {min};\n\
             \x20 if (a > 0) return b > 0 ? a > {max} / b : b < {min} / a;\n\
             \x20 return b > 0 ? a < {min} / b : a < {max} / b;\n"
        ),
        (ArithAbort::DivZero | ArithAbort::DivOverflow, _) => {
            unreachable!("div guards are emitted inline, not as helpers")
        }
    };
    out.push_str(&format!(
        "static inline _Bool {name}({c_ty} a, {c_ty} b) {{\n{body}}}\n"
    ));
}

/// Emits the abort-on-overflow model for one checked-arithmetic instruction.
///
/// Vow's `+!`/`-!`/`*!`/`/!`/`%!` abort rather than wrap, so each site carries
/// two obligations, and both are emitted here:
///
/// * `__ESBMC_assert` on the no-overflow guard, labelled `arith:<cause>:<span>`.
///   A *reachable* abort is a real program behaviour; without this assert the
///   assume below would silently prove it away. The `arith:` prefix keeps it a
///   distinct property class from `vow:N`, so a reachable abort is reported as a
///   diagnostic rather than mislabelled as a contract violation.
/// * `__ESBMC_assume` on the same guard. An overflowing execution aborts, so it
///   never returns and can never witness an `ensures` or a loop `invariant`.
///   Pruning it models *termination*, not a real behaviour — the carve-out
///   `docs/verifier-discipline.md` grants this obligation. This is what makes
///   `docs/spec/contracts.md`'s standing advice ("use `+!`") actually change the
///   verdict.
///
/// The assert precedes the assume so ESBMC checks the property on the
/// unpruned state. Earlier sites' assumes do constrain later sites, so the probe
/// reports the *first* reachable abort in a function rather than all of them.
fn emit_checked_arith(inst: &Inst, out: &mut String) {
    let id = inst.id.0;
    let Some(int_ty) = checked_integer_type(inst) else {
        // 128-bit and non-integer checked arithmetic is rejected by the
        // `is_modelable` gate, so the function never reaches the emitter. Fail
        // closed rather than emit a guard for the wrong width.
        emit_unsupported_for_verification(inst, out);
        return;
    };
    let (a, b) = (inst.args[0].0, inst.args[1].0);
    let span = inst.origin;

    let mut guards: Vec<(ArithAbort, String)> = Vec::new();
    if let Some(abort) = checked_helper_abort(inst.opcode) {
        let helper = arith_helper_name(abort, int_ty);
        guards.push((abort, format!("!{helper}(v{a}, v{b})")));
    } else {
        guards.push((ArithAbort::DivZero, format!("v{b} != 0")));
        // Cranelift's `sdiv` traps on `MIN / -1` at every width, and the
        // 128-bit routed path reproduces it (`cranelift_backend.rs`), so `/!`
        // aborts there too. `%!` does not: `MIN % -1` is `0` (grammar.md).
        if inst.opcode == Opcode::CheckedDiv && int_ty.signedness == IntegerSignedness::Signed {
            let min = c_int_min_literal(int_ty);
            guards.push((
                ArithAbort::DivOverflow,
                format!("!(v{a} == {min} && v{b} == -1)"),
            ));
        }
    }

    // The asserts are compiled out under `ARITH_ASSERT_SUPPRESS_MACRO`, which is
    // how `contracts_only_source` produces the contract-only variant of this
    // model from the very same emitted text. The assumes are never suppressed:
    // they *are* the abort semantics, and dropping them would resurrect the
    // wrapping model this whole path exists to replace.
    out.push_str(&format!("#ifndef {ARITH_ASSERT_SUPPRESS_MACRO}\n"));
    for (abort, guard) in &guards {
        out.push_str(&format!(
            "  __ESBMC_assert({guard}, \"arith:{}:{}:{}\");\n",
            abort.label(),
            span.start,
            span.len
        ));
    }
    out.push_str("#endif\n");
    for (_, guard) in &guards {
        out.push_str(&format!("  __ESBMC_assume({guard});\n"));
    }
    let op = match inst.opcode {
        Opcode::CheckedAdd => "+",
        Opcode::CheckedSub => "-",
        Opcode::CheckedMul => "*",
        Opcode::CheckedDiv => "/",
        Opcode::CheckedRem => "%",
        _ => unreachable!("emit_checked_arith called on {:?}", inst.opcode),
    };
    out.push_str(&format!("  v{id} = v{a} {op} v{b};\n"));
}

fn emit_c_preamble(out: &mut String, helpers: &ModelHelpers, limits: &VerifyLimits) {
    out.push_str("#include <stdint.h>\n");
    out.push_str("#include <stdlib.h>\n");
    out.push_str("#include <stdbool.h>\n");
    out.push_str("extern void __ESBMC_assume(_Bool);\n");
    out.push_str("extern void __ESBMC_assert(_Bool, const char*);\n");
    out.push_str("extern int __VERIFIER_nondet_int(void);\n");
    out.push_str("extern char __VERIFIER_nondet_char(void);\n");
    out.push_str("extern unsigned char __VERIFIER_nondet_uchar(void);\n");
    out.push_str("extern unsigned char __VERIFIER_nondet_unsigned_char(void);\n");
    out.push_str("extern short __VERIFIER_nondet_short(void);\n");
    out.push_str("extern unsigned short __VERIFIER_nondet_ushort(void);\n");
    out.push_str("extern unsigned short __VERIFIER_nondet_unsigned_short(void);\n");
    out.push_str("extern unsigned int __VERIFIER_nondet_uint(void);\n");
    out.push_str("extern unsigned int __VERIFIER_nondet_unsigned_int(void);\n");
    out.push_str("extern long __VERIFIER_nondet_long(void);\n");
    out.push_str("extern unsigned long __VERIFIER_nondet_unsigned_long(void);\n");
    out.push_str("extern __int128 __VERIFIER_nondet_int128(void);\n");
    out.push_str("extern unsigned __int128 __VERIFIER_nondet_uint128(void);\n");
    out.push_str("extern float __VERIFIER_nondet_float(void);\n");
    out.push_str("extern double __VERIFIER_nondet_double(void);\n");
    out.push_str("extern _Bool __VERIFIER_nondet_bool(void);\n\n");
    let vec_max = limits.vec_max;
    let string_max = limits.string_max;
    let hashmap_max = limits.hashmap_max;
    let btreemap_max = limits.btreemap_max;
    out.push_str(&format!(
        "typedef struct {{ int64_t len; int64_t data[{vec_max}]; }} __vow_vec_t;\n",
    ));
    out.push_str(&format!(
        "typedef struct {{ int64_t len; int8_t data[{string_max}]; }} __vow_string_t;\n",
    ));
    out.push_str(&format!(
        "typedef struct {{ int64_t len; int64_t keys[{hashmap_max}]; int64_t vals[{hashmap_max}]; }} __vow_hashmap_t;\n",
    ));
    out.push_str(&format!(
        "typedef struct {{ int64_t len; int64_t keys[{btreemap_max}]; int64_t vals[{btreemap_max}]; }} __vow_btreemap_t;\n",
    ));
    out.push_str("typedef struct { int64_t tag; int64_t payload; } __vow_option_t;\n");
    for &(is_shl, signedness, width) in &helpers.shifts {
        emit_shift_helper(out, is_shl, signedness, width);
    }
    for &(abort, signedness, width) in &helpers.arith {
        emit_arith_helper(out, abort, signedness, width);
    }
    if !helpers.shifts.is_empty() || !helpers.arith.is_empty() {
        out.push('\n');
    }
}

fn limits_with_literal_string_capacity(module: &Module, limits: &VerifyLimits) -> VerifyLimits {
    let mut effective = *limits;
    if let Some(max_literal_len) = module.strings.iter().map(|s| s.len()).max() {
        effective.string_max = effective.string_max.max(max_literal_len);
    }
    effective
}

fn emit_forward_declaration(func: &Function, out: &mut String) {
    let ret_c = match func.return_ty {
        Ty::Unit => "void",
        Ty::Ptr | Ty::LinearPtr => "int64_t",
        other => ir_ty_to_c(other),
    };
    let params: Vec<String> = func
        .params
        .iter()
        .enumerate()
        .filter(|&(_, &ty)| ty != Ty::Unit)
        .map(|(i, &ty)| {
            let c_ty = match ty {
                Ty::Ptr | Ty::LinearPtr => "int64_t",
                other => ir_ty_to_c(other),
            };
            format!("{} p{}", c_ty, i)
        })
        .collect();
    let param_str = if params.is_empty() {
        "void".to_string()
    } else {
        params.join(", ")
    };
    out.push_str(&format!(
        "{} {}({});\n",
        ret_c,
        verifier_c_func_name(func),
        param_str
    ));
}

pub fn emit_c_module(
    funcs: &[&Function],
    const_fns: &HashMap<FuncId, ConstantValue>,
    limits: &VerifyLimits,
) -> String {
    let mut out = String::new();
    let helpers = ModelHelpers::scan(funcs);
    emit_c_preamble(&mut out, &helpers, limits);
    for func in funcs {
        out.push_str(&emit_c_function(func, const_fns, limits));
        out.push('\n');
    }
    out
}

/// Emit C code for a target function and its modelable callees.
/// Callee functions are emitted in topological order (callees first).
#[allow(clippy::too_many_arguments)]
pub fn emit_c_module_with_callees(
    target: &Function,
    module: &Module,
    const_fns: &HashMap<FuncId, ConstantValue>,
    callee_ids: &[FuncId],
    modelable_fns: &HashSet<FuncId>,
    limits: &VerifyLimits,
    target_reach_label: bool,
    target_body_replace: bool,
) -> String {
    let mut out = String::new();
    let effective_limits = limits_with_literal_string_capacity(module, limits);

    // Collect all functions (target + callees) for shift scanning
    let mut all_funcs: Vec<&Function> = vec![target];
    for fid in callee_ids {
        if let Some(callee) = module.functions.iter().find(|f| f.id == *fid) {
            all_funcs.push(callee);
        }
    }
    let helpers = ModelHelpers::scan(&all_funcs);
    emit_c_preamble(&mut out, &helpers, &effective_limits);

    // Forward declarations for all callees
    for fid in callee_ids {
        if let Some(callee) = module.functions.iter().find(|f| f.id == *fid) {
            emit_forward_declaration(callee, &mut out);
        }
    }
    if !callee_ids.is_empty() {
        out.push('\n');
    }

    // Callee function bodies in topological order
    for fid in callee_ids {
        if let Some(callee) = module.functions.iter().find(|f| f.id == *fid) {
            out.push_str(&emit_c_function_full(
                callee,
                const_fns,
                modelable_fns,
                module,
                &effective_limits,
                false, // reach_label: callees never carry the vacuity label
                false, // body_replace: callees are never the weakness target
                // Callee: assert its `requires` at the boundary so the caller is
                // blamed for violating it (caller blame), instead of assuming it
                // and vacuously verifying the caller past the call.
                true, // requires_as_assert
            ));
            out.push('\n');
        }
    }

    // Target function — the only one that carries the vacuity `vow_reach` label
    // or the weakness body-replace rewrite. Its own `requires` are the
    // assumptions of this query (assumed, not asserted).
    out.push_str(&emit_c_function_full(
        target,
        const_fns,
        modelable_fns,
        module,
        &effective_limits,
        target_reach_label,
        target_body_replace,
        false, // requires_as_assert
    ));
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use vow_diag::Blame;
    use vow_ir::{
        BasicBlock, BlockId, FuncId, InstId, Module, RegionId, RegionSummary, VowEntry, VowId,
    };
    use vow_syntax::span::Span;

    fn sp() -> Span {
        Span::new(0, 0)
    }

    fn inst(id: u32, op: Opcode, ty: Ty, args: Vec<u32>, data: InstData) -> Inst {
        let data = integer_test_data(op, ty, data);
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

    fn integer_test_data(op: Opcode, ty: Ty, data: InstData) -> InstData {
        if data != InstData::None
            || !matches!(
                op,
                Opcode::WrappingAdd
                    | Opcode::WrappingSub
                    | Opcode::WrappingMul
                    | Opcode::WrappingDiv
                    | Opcode::WrappingRem
                    | Opcode::CheckedAdd
                    | Opcode::CheckedSub
                    | Opcode::CheckedMul
                    | Opcode::CheckedDiv
                    | Opcode::CheckedRem
                    | Opcode::Eq
                    | Opcode::Ne
                    | Opcode::Lt
                    | Opcode::Le
                    | Opcode::Gt
                    | Opcode::Ge
                    | Opcode::BitAnd
                    | Opcode::BitOr
                    | Opcode::BitXor
                    | Opcode::Shl
                    | Opcode::Shr
            )
        {
            return data;
        }
        InstData::Integer(match ty {
            Ty::I32 => IntegerType::I32,
            Ty::U64 => IntegerType::U64,
            Ty::U8 => IntegerType::U8,
            _ => IntegerType::I64,
        })
    }

    // -----------------------------------------------------------------------
    // Checked-arithmetic abort model (#585)
    // -----------------------------------------------------------------------

    /// A one-block function computing `p0 <op> p1` at `ty`, so a test can read
    /// the emitted model for a single arithmetic site.
    fn arith_func(op: Opcode, ty: Ty, origin: Span) -> Function {
        // Attach the true integer type; `integer_test_data`'s fallback would
        // claim I64 for the widths it does not enumerate, which is exactly the
        // lie the 128-bit gate must not be tested against.
        let int_ty = ir_ty_to_integer_type(ty).expect("arith_func needs an integer type");
        let mut binop = inst(2, op, ty, vec![0, 1], InstData::Integer(int_ty));
        binop.origin = origin;
        Function {
            id: FuncId(0),
            name: "f".to_string(),
            params: vec![ty, ty],
            param_names: vec!["a".to_string(), "b".to_string()],
            return_ty: ty,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::GetArg, ty, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::GetArg, ty, vec![], InstData::ArgIndex(1)),
                    binop,
                    inst(3, Opcode::Return, ty, vec![2], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        }
    }

    /// A module holding just `f`, for the non-modelable gate.
    fn arith_module(f: &Function) -> Module {
        Module {
            name: String::new(),
            functions: vec![f.clone()],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        }
    }

    fn arith_c(op: Opcode, ty: Ty) -> String {
        let f = arith_func(op, ty, Span::new(92, 6));
        emit_c_module(&[&f], &HashMap::new(), &VerifyLimits::default())
    }

    // The whole point of #585: a checked operator must not emit the same model
    // as its wrapping sibling. Wrapping is specified to wrap, so a bare C
    // operator is exact; checked aborts, so it carries an obligation.
    #[test]
    fn checked_arithmetic_is_modelled_differently_from_wrapping() {
        for (wrapping, checked) in [
            (Opcode::WrappingAdd, Opcode::CheckedAdd),
            (Opcode::WrappingSub, Opcode::CheckedSub),
            (Opcode::WrappingMul, Opcode::CheckedMul),
            (Opcode::WrappingDiv, Opcode::CheckedDiv),
            (Opcode::WrappingRem, Opcode::CheckedRem),
        ] {
            let w = arith_c(wrapping, Ty::I64);
            let c = arith_c(checked, Ty::I64);
            assert!(
                !w.contains("arith:") && !w.contains("__vow_ovf_"),
                "{wrapping:?} must emit no overflow obligation:\n{w}"
            );
            assert!(
                c.contains("arith:"),
                "{checked:?} must emit an arith obligation:\n{c}"
            );
            assert_ne!(w, c, "{wrapping:?} and {checked:?} must not share a model");
        }
    }

    // Each site emits the assert *before* the assume, so the property is checked
    // on the unpruned state. Both name the same guard.
    #[test]
    fn checked_site_asserts_then_assumes_the_same_guard() {
        let c = arith_c(Opcode::CheckedAdd, Ty::I64);
        let assert_at = c
            .find(r#"__ESBMC_assert(!__vow_ovf_add_i64(v0, v1), "arith:add:92:6");"#)
            .unwrap_or_else(|| panic!("missing labelled assert:\n{c}"));
        let assume_at = c
            .find("__ESBMC_assume(!__vow_ovf_add_i64(v0, v1));")
            .unwrap_or_else(|| panic!("missing assume:\n{c}"));
        assert!(
            assert_at < assume_at,
            "the assert must precede the pruning assume:\n{c}"
        );
        assert!(c.contains("v2 = v0 + v1;"), "operation still emitted:\n{c}");
    }

    // The label carries the operator's own span, which is what points the
    // ArithOverflowReachable diagnostic at the source.
    #[test]
    fn arith_label_carries_cause_and_span() {
        for (op, ty, expected) in [
            (Opcode::CheckedAdd, Ty::I64, "arith:add:7:3"),
            (Opcode::CheckedSub, Ty::U64, "arith:sub:7:3"),
            (Opcode::CheckedMul, Ty::I32, "arith:mul:7:3"),
            (Opcode::CheckedRem, Ty::I64, "arith:div-zero:7:3"),
        ] {
            let f = arith_func(op, ty, Span::new(7, 3));
            let c = emit_c_module(&[&f], &HashMap::new(), &VerifyLimits::default());
            assert!(c.contains(expected), "expected {expected} in:\n{c}");
        }
    }

    // `/!` aborts on MIN / -1 (Cranelift's sdiv traps there, and the routed
    // 128-bit path reproduces it); `%!` does not — grammar.md fixes MIN % -1 at
    // 0. The model must not invent an obligation the runtime does not have.
    #[test]
    fn signed_div_guards_min_over_minus_one_but_rem_does_not() {
        let div = arith_c(Opcode::CheckedDiv, Ty::I64);
        assert!(
            div.contains("arith:div-zero:92:6") && div.contains("arith:div-overflow:92:6"),
            "signed /! needs both a zero-divisor and a MIN/-1 guard:\n{div}"
        );
        let rem = arith_c(Opcode::CheckedRem, Ty::I64);
        assert!(
            rem.contains("arith:div-zero:92:6"),
            "%! needs the zero-divisor guard:\n{rem}"
        );
        assert!(
            !rem.contains("div-overflow"),
            "MIN % -1 is 0 and does not abort; %! must carry no overflow guard:\n{rem}"
        );
        // Unsigned division has no MIN/-1 case at all.
        let udiv = arith_c(Opcode::CheckedDiv, Ty::U64);
        assert!(
            !udiv.contains("div-overflow"),
            "unsigned /! has no MIN/-1 case:\n{udiv}"
        );
    }

    // `contracts_only_source` must suppress the asserts and keep the assumes:
    // dropping the assumes would resurrect the wrapping model this path
    // replaces, and keeping the asserts would leave the contract verdict masked.
    #[test]
    fn contracts_only_source_drops_asserts_and_keeps_assumes() {
        let c = arith_c(Opcode::CheckedAdd, Ty::I64);
        assert!(
            c.contains(&format!("#ifndef {ARITH_ASSERT_SUPPRESS_MACRO}")) && c.contains("#endif"),
            "the arith asserts must be conditionally compiled:\n{c}"
        );
        let only = contracts_only_source(&c);
        assert!(
            only.starts_with(&format!("#define {ARITH_ASSERT_SUPPRESS_MACRO} 1\n")),
            "projection must define the suppression macro:\n{only}"
        );
        assert!(
            only.contains("__ESBMC_assume(!__vow_ovf_add_i64(v0, v1));"),
            "the abort assume must survive the projection:\n{only}"
        );
    }

    // Only the flavors actually used are emitted, and each is emitted once.
    #[test]
    fn preamble_emits_exactly_the_overflow_helpers_used() {
        let c = arith_c(Opcode::CheckedAdd, Ty::U64);
        assert_eq!(
            c.matches("__vow_ovf_add_u64(uint64_t a, uint64_t b)")
                .count(),
            1,
            "the used helper is defined exactly once:\n{c}"
        );
        for unused in [
            "__vow_ovf_add_i64(",
            "__vow_ovf_sub_u64(",
            "__vow_ovf_mul_u64(",
        ] {
            assert!(
                !c.contains(unused),
                "unused helper {unused} must not be emitted:\n{c}"
            );
        }
        // Div/rem guards are inline comparisons, so they pull in no helper.
        let d = arith_c(Opcode::CheckedDiv, Ty::I64);
        assert!(!d.contains("__vow_ovf_"), "div guards need no helper:\n{d}");
    }

    // 128-bit checked arithmetic has no guard, so it must fail closed as
    // non-modelable (reported `Skipped`) rather than silently fall back to the
    // wrapping model. `ConstI128` already set this precedent.
    #[test]
    fn checked_arithmetic_at_128_bits_is_not_modelable() {
        for ty in [Ty::I128, Ty::U128] {
            for op in [
                Opcode::CheckedAdd,
                Opcode::CheckedSub,
                Opcode::CheckedMul,
                Opcode::CheckedDiv,
                Opcode::CheckedRem,
            ] {
                let f = arith_func(op, ty, sp());
                let module = arith_module(&f);
                let reason = non_modelable_reason(&f, &module, &HashMap::new())
                    .unwrap_or_else(|| panic!("{op:?} at {ty:?} must be non-modelable"));
                assert!(
                    reason.contains("128-bit width"),
                    "{op:?} at {ty:?}: unexpected reason {reason}"
                );
            }
        }
        // The narrower widths stay modelable.
        for ty in [Ty::I32, Ty::I64, Ty::U64] {
            let f = arith_func(Opcode::CheckedAdd, ty, sp());
            let module = arith_module(&f);
            assert!(
                non_modelable_reason(&f, &module, &HashMap::new()).is_none(),
                "CheckedAdd at {ty:?} must be modelable"
            );
        }
    }

    /// Compiles the *emitted* overflow guards and checks them against
    /// `__builtin_*_overflow` ground truth: exhaustively over the whole 8-bit
    /// domain, and over boundary plus deterministic pseudo-random values at 16,
    /// 32 and 64 bits.
    ///
    /// This is the test that matters most for #585. A guard that is merely
    /// *close* to right is a silent unsoundness in both directions — too strict
    /// and the assume prunes real returning executions (false proofs); too loose
    /// and a reachable abort goes unreported. Asserting on the emitted C text
    /// cannot catch an off-by-one in the formula, so the C is actually run.
    ///
    /// Skipped (not failed) when no C compiler is on PATH.
    #[test]
    #[cfg(unix)]
    fn emitted_overflow_guards_match_builtin_ground_truth() {
        let Some(cc) = ["cc", "gcc", "clang"].into_iter().find(|c| {
            std::process::Command::new(c)
                .arg("--version")
                .output()
                .is_ok()
        }) else {
            eprintln!("no C compiler on PATH; skipping guard brute-force");
            return;
        };

        let mut src =
            String::from("#include <stdint.h>\n#include <stdio.h>\n#include <stdbool.h>\n");
        // Emit every guard flavor the emitter can produce for the modelled widths.
        let widths = [
            IntegerWidth::W8,
            IntegerWidth::W16,
            IntegerWidth::W32,
            IntegerWidth::W64,
        ];
        for &width in &widths {
            for signedness in [IntegerSignedness::Signed, IntegerSignedness::Unsigned] {
                for abort in [ArithAbort::Add, ArithAbort::Sub, ArithAbort::Mul] {
                    emit_arith_helper(&mut src, abort, signedness, width);
                }
            }
        }

        src.push_str(
            "static long bad = 0;\n\
             static uint64_t rng_state = 0x243f6a8885a308d3ULL;\n\
             static uint64_t rng(void) {\n\
             \x20 rng_state ^= rng_state << 13; rng_state ^= rng_state >> 7;\n\
             \x20 rng_state ^= rng_state << 17; return rng_state;\n\
             }\n",
        );

        // One checker per (width, signedness): compares each emitted guard
        // against the corresponding __builtin_*_overflow on the same types.
        for &width in &widths {
            for signedness in [IntegerSignedness::Signed, IntegerSignedness::Unsigned] {
                let int_ty = IntegerType::new(width, signedness);
                let bits = width.bits();
                let c_ty = match signedness {
                    IntegerSignedness::Signed => c_int_type(width),
                    IntegerSignedness::Unsigned => c_uint_type(width),
                };
                let tag = match signedness {
                    IntegerSignedness::Signed => "i",
                    IntegerSignedness::Unsigned => "u",
                };
                let add = arith_helper_name(ArithAbort::Add, int_ty);
                let sub = arith_helper_name(ArithAbort::Sub, int_ty);
                let mul = arith_helper_name(ArithAbort::Mul, int_ty);
                src.push_str(&format!(
                    "static void check_{tag}{bits}({c_ty} a, {c_ty} b) {{\n\
                     \x20 {c_ty} r;\n\
                     \x20 if ({add}(a, b) != __builtin_add_overflow(a, b, &r)) {{ printf(\"ADD {tag}{bits}\\n\"); bad++; }}\n\
                     \x20 if ({sub}(a, b) != __builtin_sub_overflow(a, b, &r)) {{ printf(\"SUB {tag}{bits}\\n\"); bad++; }}\n\
                     \x20 if ({mul}(a, b) != __builtin_mul_overflow(a, b, &r)) {{ printf(\"MUL {tag}{bits}\\n\"); bad++; }}\n\
                     }}\n"
                ));
            }
        }

        src.push_str(
            "int main(void) {\n\
             \x20 /* Exhaustive over the whole 8-bit domain. */\n\
             \x20 for (int a = -128; a <= 127; a++) for (int b = -128; b <= 127; b++)\n\
             \x20   check_i8((int8_t)a, (int8_t)b);\n\
             \x20 for (int a = 0; a <= 255; a++) for (int b = 0; b <= 255; b++)\n\
             \x20   check_u8((uint8_t)a, (uint8_t)b);\n",
        );
        // Boundary values plus random draws at the wider widths.
        for (bits, signed_bounds, unsigned_bounds) in [
            (
                16u16,
                "INT16_MIN, INT16_MIN+1, -256, -255, -2, -1, 0, 1, 2, 181, 182, 255, 256, INT16_MAX-1, INT16_MAX",
                "0, 1, 2, 255, 256, 257, 65534, UINT16_MAX",
            ),
            (
                32,
                "INT32_MIN, INT32_MIN+1, -65536, -46341, -46340, -2, -1, 0, 1, 2, 46340, 46341, 65536, INT32_MAX-1, INT32_MAX",
                "0, 1, 2, 65535, 65536, 65537, 4294967294u, UINT32_MAX",
            ),
            (
                64,
                "INT64_MIN, INT64_MIN+1, -4294967296LL, -3037000500LL, -3037000499LL, -2, -1, 0, 1, 2, 3037000499LL, 3037000500LL, 4294967296LL, INT64_MAX-1, INT64_MAX",
                "0, 1, 2, 4294967295ULL, 4294967296ULL, 6074000999ULL, 18446744073709551614ULL, UINT64_MAX",
            ),
        ] {
            src.push_str(&format!(
                "  {{\n\
                 \x20   int{bits}_t sv[] = {{{signed_bounds}}};\n\
                 \x20   uint{bits}_t uv[] = {{{unsigned_bounds}}};\n\
                 \x20   size_t sn = sizeof(sv)/sizeof(sv[0]), un = sizeof(uv)/sizeof(uv[0]);\n\
                 \x20   for (size_t i = 0; i < sn; i++) for (size_t j = 0; j < sn; j++) check_i{bits}(sv[i], sv[j]);\n\
                 \x20   for (size_t i = 0; i < un; i++) for (size_t j = 0; j < un; j++) check_u{bits}(uv[i], uv[j]);\n\
                 \x20   for (int k = 0; k < 20000; k++) {{\n\
                 \x20     check_i{bits}((int{bits}_t)rng(), (int{bits}_t)rng());\n\
                 \x20     check_u{bits}((uint{bits}_t)rng(), (uint{bits}_t)rng());\n\
                 \x20     /* mix a boundary against a random value, both ways */\n\
                 \x20     check_i{bits}(sv[k %% sn], (int{bits}_t)rng());\n\
                 \x20     check_i{bits}((int{bits}_t)rng(), sv[k %% sn]);\n\
                 \x20     check_u{bits}(uv[k %% un], (uint{bits}_t)rng());\n\
                 \x20     check_u{bits}((uint{bits}_t)rng(), uv[k %% un]);\n\
                 \x20   }}\n\
                 \x20 }}\n"
            ).replace("%%", "%"));
        }
        src.push_str("  printf(\"mismatches=%ld\\n\", bad);\n  return bad != 0;\n}\n");

        let dir = std::env::temp_dir().join(format!("vow-arith-guards-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let c_path = dir.join("guards.c");
        let bin_path = dir.join("guards");
        std::fs::write(&c_path, &src).expect("write guard harness");

        let build = std::process::Command::new(cc)
            .arg("-O1")
            .arg("-Werror")
            .arg("-o")
            .arg(&bin_path)
            .arg(&c_path)
            .output()
            .expect("invoke C compiler");
        assert!(
            build.status.success(),
            "the emitted guards must compile cleanly:\n{}\n--- source ---\n{src}",
            String::from_utf8_lossy(&build.stderr)
        );

        let run = std::process::Command::new(&bin_path)
            .output()
            .expect("run guard harness");
        let stdout = String::from_utf8_lossy(&run.stdout);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            run.status.success() && stdout.contains("mismatches=0"),
            "emitted overflow guards disagree with __builtin_*_overflow:\n{stdout}"
        );
    }

    #[test]
    fn arith_abort_labels_round_trip() {
        for abort in [
            ArithAbort::Add,
            ArithAbort::Sub,
            ArithAbort::Mul,
            ArithAbort::DivZero,
            ArithAbort::DivOverflow,
        ] {
            assert_eq!(ArithAbort::from_label(abort.label()), Some(abort));
            assert!(!abort.description().is_empty());
        }
        assert_eq!(ArithAbort::from_label("nope"), None);
    }

    // The MIN/MAX literals the guards are built from, per width. `MIN` is spelled
    // `(-MAX - 1)` because `-9223372036854775808` is not a valid C literal.
    #[test]
    fn integer_bound_literals_are_width_exact() {
        for (width, signed_max, unsigned_max) in [
            (IntegerWidth::W8, "127", "255"),
            (IntegerWidth::W16, "32767", "65535"),
            (IntegerWidth::W32, "2147483647", "4294967295"),
            (
                IntegerWidth::W64,
                "9223372036854775807",
                "18446744073709551615",
            ),
        ] {
            let signed = IntegerType::new(width, IntegerSignedness::Signed);
            let unsigned = IntegerType::new(width, IntegerSignedness::Unsigned);
            assert!(
                c_int_max_literal(signed).contains(signed_max),
                "{width:?} signed max"
            );
            assert!(
                c_int_max_literal(unsigned).contains(unsigned_max),
                "{width:?} unsigned max"
            );
            assert_eq!(
                c_int_min_literal(signed),
                format!("(-{} - 1)", c_int_max_literal(signed))
            );
            assert_eq!(
                c_int_min_literal(unsigned),
                format!("(({})0)", c_uint_type(width))
            );
        }
    }

    #[test]
    fn emit_c_module_declares_unsigned_long_nondet() {
        let c = emit_c_module(&[], &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("extern unsigned long __VERIFIER_nondet_unsigned_long(void);"),
            "generated C preamble must declare the u64 nondet intrinsic:\n{c}"
        );
    }

    #[test]
    fn collect_modelable_callees_skips_reserved_verifier_names() {
        let caller = Function {
            id: FuncId(0),
            name: "caller".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(
                        0,
                        Opcode::Call,
                        Ty::I64,
                        vec![],
                        InstData::CallTarget(FuncId(1)),
                    ),
                    inst(
                        1,
                        Opcode::Call,
                        Ty::I64,
                        vec![],
                        InstData::CallTarget(FuncId(2)),
                    ),
                    inst(
                        2,
                        Opcode::Call,
                        Ty::I64,
                        vec![],
                        InstData::CallTarget(FuncId(3)),
                    ),
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let nondet_reserved = Function {
            id: FuncId(1),
            name: "__VERIFIER_nondet_int".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(7)),
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let assume_reserved = Function {
            id: FuncId(2),
            name: "__ESBMC_assume".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let safe = Function {
            id: FuncId(3),
            name: "safe".to_string(),
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
        };
        let module = Module {
            name: "test".to_string(),
            functions: vec![caller.clone(), nondet_reserved, assume_reserved, safe],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };

        let mut cache = HashMap::new();
        let callees = collect_modelable_callees(&caller, &module, &HashMap::new(), &mut cache);
        assert_eq!(callees, vec![FuncId(3)]);
    }

    #[test]
    fn emit_simple_function() {
        let func = Function {
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
                    inst(2, Opcode::WrappingAdd, Ty::I64, vec![0, 1], InstData::None),
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("int64_t vow_user_fn_0("), "signature: {c}");
        assert!(c.contains("v2 = v0 + v1"), "add: {c}");
        assert!(c.contains("return v2"), "return: {c}");
    }

    #[test]
    fn emit_function_uses_verifier_private_symbol_for_source_name() {
        for (idx, name) in ["abs", "labs", "div", "ldiv"].into_iter().enumerate() {
            let mut func = make_func(
                name,
                vec![Ty::I64],
                Ty::I64,
                vec![
                    inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            );
            let func_id = FuncId(7 + idx as u32);
            func.id = func_id;

            let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());

            assert!(
                c.contains(&format!("int64_t vow_user_fn_{}(", func_id.0)),
                "mangled signature for {name}: {c}"
            );
            assert!(
                !c.contains(&format!("int64_t {name}(")),
                "raw libc name leaked for {name}: {c}"
            );
        }
    }

    #[test]
    fn libc_named_user_function_is_modelable() {
        // A user function named like a libc/ESBMC stdlib symbol must
        // still be modeled: it is emitted under a mangled `vow_user_fn_<id>`
        // name, so it cannot collide with ESBMC's C stdlib model. Regression
        // test for the `verify/libc_name_collision` verifier-eval fixture.
        for name in ["abs", "labs", "div", "ldiv"] {
            let func = make_func(
                name,
                vec![Ty::I64],
                Ty::I64,
                vec![
                    inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            );
            let module = Module {
                name: "test".to_string(),
                functions: vec![func.clone()],
                strings: vec![],
                struct_layouts: vec![],
                enum_layouts: vec![],
                warnings: vec![],
            };
            assert_eq!(
                non_modelable_reason(&func, &module, &HashMap::new()),
                None,
                "a libc-named user function should be modelable, not skipped as reserved: {name}"
            );
            assert!(
                !is_reserved_verifier_symbol(name),
                "ordinary user function should not be reserved: {name}"
            );
        }
        // The genuine ESBMC/verifier intrinsic namespaces remain reserved.
        assert!(is_reserved_verifier_symbol("__VERIFIER_nondet_int"));
        assert!(is_reserved_verifier_symbol("__ESBMC_assume"));
    }

    #[test]
    fn emit_string_matches_literal_at_uses_static_bytes() {
        let func = Function {
            id: FuncId(0),
            name: "matches_literal".to_string(),
            params: vec![Ty::Ptr, Ty::I64],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                    inst(2, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                    inst(3, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(3)),
                    inst(
                        4,
                        Opcode::Call,
                        Ty::I64,
                        vec![0, 1, 2, 3],
                        InstData::CallExtern("__vow_string_matches_literal_at".to_string()),
                    ),
                    inst(5, Opcode::Return, Ty::Unit, vec![4], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let module = Module {
            name: "test".to_string(),
            functions: vec![func.clone()],
            strings: vec!["a\0b".to_string()],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };

        let c = emit_c_function_full(
            &func,
            &HashMap::new(),
            &HashSet::new(),
            &module,
            &VerifyLimits::default(),
            false,
            false,
            false,
        );

        assert!(
            c.contains("v4 = 0;") && !c.contains("v4 = __VERIFIER_nondet_long"),
            "literal helper should be modeled deterministically: {c}"
        );
        assert!(c.contains("3LL <= v0.len - v1"), "byte length guard: {c}");
        assert!(
            c.contains("(unsigned char)v0.data[v1 + 0] != 97")
                && c.contains("(unsigned char)v0.data[v1 + 1] != 0")
                && c.contains("(unsigned char)v0.data[v1 + 2] != 98"),
            "literal bytes should be embedded as constants: {c}"
        );
    }

    #[test]
    fn emit_vow_requires_as_assume() {
        let func = Function {
            id: FuncId(0),
            name: "divide".to_string(),
            params: vec![Ty::I64, Ty::I64],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "y != 0".to_string(),
                blame: Blame::Caller,
                bindings: vec![],
                file: String::new(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                    inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    inst(3, Opcode::Ne, Ty::Bool, vec![1, 2], InstData::None),
                    Inst {
                        id: InstId(4),
                        opcode: Opcode::VowRequires,
                        ty: Ty::Unit,
                        args: vec![InstId(3)],
                        data: InstData::VowId(VowId(0)),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(5, Opcode::WrappingDiv, Ty::I64, vec![0, 1], InstData::None),
                    inst(6, Opcode::Return, Ty::Unit, vec![5], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("__ESBMC_assume(v3)"), "requires: {c}");
        assert!(!c.contains("__ESBMC_assert"), "no assert for requires: {c}");
    }

    #[test]
    fn emit_callee_requires_as_structured_precondition_assert() {
        let func = Function {
            id: FuncId(7),
            name: "divide".to_string(),
            params: vec![Ty::I64, Ty::I64],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(2),
                description: "y != 0".to_string(),
                blame: Blame::Caller,
                bindings: vec![],
                file: String::new(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                    inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    inst(3, Opcode::Ne, Ty::Bool, vec![1, 2], InstData::None),
                    Inst {
                        id: InstId(4),
                        opcode: Opcode::VowRequires,
                        ty: Ty::Unit,
                        args: vec![InstId(3)],
                        data: InstData::VowId(VowId(2)),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(5, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let module = Module {
            name: String::new(),
            functions: vec![],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };

        let c = emit_c_function_full(
            &func,
            &HashMap::new(),
            &HashSet::new(),
            &module,
            &VerifyLimits::default(),
            false,
            false,
            true,
        );

        assert!(
            c.contains("__ESBMC_assert(v3, \"vow:pre:7:2\")"),
            "callee requires assert should carry function-local disambiguation:\n{c}"
        );
    }

    #[test]
    fn emit_reach_label_after_requires() {
        // #81 PR-B: in reach mode the `vow_reach` label is planted immediately
        // after the last `requires` assume (so body divergence can't make it
        // spuriously unreachable), and it is absent on the normal verify path.
        let func = Function {
            id: FuncId(0),
            name: "divide".to_string(),
            params: vec![Ty::I64, Ty::I64],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "y != 0".to_string(),
                blame: Blame::Caller,
                bindings: vec![],
                file: String::new(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                    inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    inst(3, Opcode::Ne, Ty::Bool, vec![1, 2], InstData::None),
                    Inst {
                        id: InstId(4),
                        opcode: Opcode::VowRequires,
                        ty: Ty::Unit,
                        args: vec![InstId(3)],
                        data: InstData::VowId(VowId(0)),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(5, Opcode::WrappingDiv, Ty::I64, vec![0, 1], InstData::None),
                    inst(6, Opcode::Return, Ty::Unit, vec![5], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let module = Module {
            name: String::new(),
            functions: vec![],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        let c = emit_c_function_full(
            &func,
            &HashMap::new(),
            &HashSet::new(),
            &module,
            &VerifyLimits::default(),
            true,
            false,
            false,
        );
        let assume_pos = c
            .find("__ESBMC_assume(v3)")
            .expect("requires assume present");
        let label_pos = c.find("vow_reach:").expect("reach label present");
        assert!(
            label_pos > assume_pos,
            "label must follow the requires assume:\n{c}"
        );
        let c_no = emit_c_function_full(
            &func,
            &HashMap::new(),
            &HashSet::new(),
            &module,
            &VerifyLimits::default(),
            false,
            false,
            false,
        );
        assert!(
            !c_no.contains("vow_reach"),
            "no reach label on the normal verify path:\n{c_no}"
        );
    }

    #[test]
    fn emit_body_replace_overwrites_result_with_default() {
        // #81 PR-C: in body-replace mode the returned value is overwritten with
        // the type-default right after it is computed, so the `ensures` is
        // checked against a trivial `return 0` body; absent on the normal path.
        // g(x) { x + 1 }: result is %5 (WrappingAdd[i64]), referenced by the
        // ensures and the Return.
        let func = Function {
            id: FuncId(0),
            name: "g".to_string(),
            params: vec![Ty::I64],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(1),
                description: "result >= 0".to_string(),
                blame: Blame::Callee,
                bindings: vec![],
                file: String::new(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                    inst(4, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(1)),
                    inst(5, Opcode::WrappingAdd, Ty::I64, vec![0, 4], InstData::None),
                    inst(6, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    inst(7, Opcode::Ge, Ty::Bool, vec![5, 6], InstData::None),
                    Inst {
                        id: InstId(8),
                        opcode: Opcode::VowEnsures,
                        ty: Ty::Unit,
                        args: vec![InstId(7)],
                        data: InstData::VowId(VowId(1)),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(9, Opcode::Return, Ty::Unit, vec![5], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let module = Module {
            name: String::new(),
            functions: vec![],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        let c = emit_c_function_full(
            &func,
            &HashMap::new(),
            &HashSet::new(),
            &module,
            &VerifyLimits::default(),
            false,
            true,
            false,
        );
        let add_pos = c.find("v5 = v0 + v4").expect("body computes the result");
        let zero_pos = c.find("v5 = 0;").expect("result overwritten with default");
        assert!(
            zero_pos > add_pos,
            "default overwrite must follow the body's result computation:\n{c}"
        );
        let c_no = emit_c_function_full(
            &func,
            &HashMap::new(),
            &HashSet::new(),
            &module,
            &VerifyLimits::default(),
            false,
            false,
            false,
        );
        assert!(
            !c_no.contains("v5 = 0;"),
            "no result overwrite on the normal verify path:\n{c_no}"
        );
    }

    #[test]
    fn ir_ty_to_c_all_variants() {
        assert_eq!(ir_ty_to_c(Ty::I32), "int32_t");
        assert_eq!(ir_ty_to_c(Ty::I64), "int64_t");
        assert_eq!(ir_ty_to_c(Ty::F32), "float");
        assert_eq!(ir_ty_to_c(Ty::F64), "double");
        assert_eq!(ir_ty_to_c(Ty::Bool), "_Bool");
        assert_eq!(ir_ty_to_c(Ty::Unit), "int32_t");
        assert_eq!(ir_ty_to_c(Ty::Ptr), "void*");
        assert_eq!(ir_ty_to_c(Ty::LinearPtr), "void*");
    }

    #[test]
    fn c_nondet_suffix_all_variants() {
        assert_eq!(c_nondet_suffix(Ty::I32), "int");
        assert_eq!(c_nondet_suffix(Ty::I64), "long");
        assert_eq!(c_nondet_suffix(Ty::F32), "float");
        assert_eq!(c_nondet_suffix(Ty::F64), "double");
        assert_eq!(c_nondet_suffix(Ty::Bool), "bool");
        assert_eq!(c_nondet_suffix(Ty::Ptr), "long");
        assert_eq!(c_nondet_suffix(Ty::LinearPtr), "long");
        assert_eq!(c_nondet_suffix(Ty::Unit), "int");
    }

    fn make_func(name: &str, params: Vec<Ty>, ret: Ty, insts: Vec<Inst>) -> Function {
        Function {
            id: FuncId(0),
            name: name.to_string(),
            params,
            param_names: vec![],
            return_ty: ret,
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
    fn unwrap_panic_call_asserts_unwrap_none() {
        let insts = vec![
            inst(
                0,
                Opcode::Call,
                Ty::Unit,
                vec![],
                InstData::CallExtern("__vow_unwrap_panic".to_string()),
            ),
            inst(1, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let func = make_func("unwrap_none", vec![], Ty::Unit, insts);

        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains(r#"__ESBMC_assert(0, "unwrap-none")"#),
            "the guarded unwrap panic must become a verification obligation:\n{c}"
        );
        assert!(
            !c.contains("not modelled"),
            "__vow_unwrap_panic must be modelled, not skipped:\n{c}"
        );
    }

    #[test]
    fn phase3_narrowing_targets_have_exact_c_ranges() {
        let cases = [
            ("__vow_i16_to_i8_try", "int8_t", "-128", "127"),
            ("__vow_i16_to_u8_try", "uint8_t", "0", "255"),
            ("__vow_i32_to_i16_try", "int16_t", "-32768", "32767"),
            ("__vow_i32_to_u16_try", "uint16_t", "0", "65535"),
            (
                "__vow_i64_to_i32_try",
                "int32_t",
                "-2147483648",
                "2147483647",
            ),
            ("__vow_i64_to_u32_try", "uint32_t", "0", "4294967295ULL"),
        ];

        for (name, c_ty, min, max) in cases {
            let model = narrow_target_model(name).expect(name);
            assert_eq!(model.c_ty, c_ty, "{name}");
            assert_eq!(model.min, min, "{name}");
            assert_eq!(model.max, max, "{name}");
        }
        for name in [
            "i16_to_i8_try",
            "__vow_i16_to_i8_checked",
            "__vow_i16_to_i64_try",
        ] {
            assert!(narrow_target_model(name).is_none(), "{name}");
        }
    }

    #[test]
    fn emit_phase3_narrowing_modes_and_saturating_u8_arithmetic() {
        let func = make_func(
            "narrow",
            vec![Ty::I64, Ty::U64],
            Ty::Unit,
            vec![
                inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::GetArg, Ty::U64, vec![], InstData::ArgIndex(1)),
                inst(
                    2,
                    Opcode::Call,
                    Ty::Ptr,
                    vec![0],
                    InstData::CallExtern("__vow_i64_to_i8_try".to_string()),
                ),
                inst(
                    3,
                    Opcode::Call,
                    Ty::Ptr,
                    vec![1],
                    InstData::CallExtern("__vow_u64_to_i8_try".to_string()),
                ),
                inst(
                    4,
                    Opcode::Call,
                    Ty::U16,
                    vec![0],
                    InstData::CallExtern("__vow_i64_to_u16_wrap".to_string()),
                ),
                inst(
                    5,
                    Opcode::Call,
                    Ty::I16,
                    vec![0],
                    InstData::CallExtern("__vow_i64_to_i16_sat".to_string()),
                ),
                inst(
                    6,
                    Opcode::Call,
                    Ty::U32,
                    vec![1],
                    InstData::CallExtern("__vow_u64_to_u32_sat".to_string()),
                ),
                inst(7, Opcode::ConstU8, Ty::U8, vec![], InstData::ConstU8(250)),
                inst(8, Opcode::ConstU8, Ty::U8, vec![], InstData::ConstU8(10)),
                inst(
                    9,
                    Opcode::Call,
                    Ty::U8,
                    vec![7, 8],
                    InstData::CallExtern("__vow_add_sat_u8".to_string()),
                ),
                inst(
                    10,
                    Opcode::Call,
                    Ty::U8,
                    vec![7, 8],
                    InstData::CallExtern("__vow_sub_sat_u8".to_string()),
                ),
                inst(
                    11,
                    Opcode::Call,
                    Ty::U8,
                    vec![7, 8],
                    InstData::CallExtern("__vow_mul_sat_u8".to_string()),
                ),
                inst(12, Opcode::Return, Ty::Unit, vec![], InstData::None),
            ],
        );

        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        for expected in [
            "v2.tag = (v0 >= -128 && v0 <= 127);",
            "v3.tag = (v1 <= 127);",
            "v4 = (uint16_t)v0;",
            "v5 = v0 < -32768 ? -32768 : (v0 > 32767 ? 32767 : (int16_t)v0);",
            "v6 = v1 > 4294967295ULL ? 4294967295ULL : (uint32_t)v1;",
            "uint16_t __sat_9 = (uint16_t)v7 + (uint16_t)v8;",
            "v10 = (uint8_t)(v7 < v8 ? 0 : v7 - v8);",
            "uint16_t __sat_11 = (uint16_t)v7 * (uint16_t)v8;",
        ] {
            assert!(c.contains(expected), "missing `{expected}` in:\n{c}");
        }
    }

    #[test]
    fn emit_phase3_parser_models_bound_each_payload() {
        let mut insts = vec![inst(
            0,
            Opcode::GetArg,
            Ty::Ptr,
            vec![],
            InstData::ArgIndex(0),
        )];
        for (id, name) in [
            "__vow_string_parse_i8_opt",
            "__vow_string_parse_i16_opt",
            "__vow_string_parse_u16_opt",
            "__vow_string_parse_u32_opt",
        ]
        .into_iter()
        .enumerate()
        {
            insts.push(inst(
                id as u32 + 1,
                Opcode::Call,
                Ty::Ptr,
                vec![0],
                InstData::CallExtern(name.to_string()),
            ));
        }
        insts.push(inst(5, Opcode::Return, Ty::Unit, vec![], InstData::None));
        let func = make_func("parse_narrow", vec![Ty::Ptr], Ty::Unit, insts);

        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        for expected in [
            "v1.payload >= -128 && v1.payload <= 127",
            "v2.payload >= -32768 && v2.payload <= 32767",
            "v3.payload >= 0 && v3.payload <= 65535",
            "v4.payload >= 0 && v4.payload <= 4294967295ULL",
        ] {
            assert!(c.contains(expected), "missing `{expected}` in:\n{c}");
        }
        assert!(
            c.contains("v4.payload = __VERIFIER_nondet_ulong()"),
            "u32 parse payload must use the unsigned nondeterministic model:\n{c}"
        );
    }

    #[test]
    fn emit_arena_parse_i64_as_an_option() {
        let func = make_func(
            "parse",
            vec![],
            Ty::I64,
            vec![
                inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                inst(1, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                inst(
                    2,
                    Opcode::Call,
                    Ty::Ptr,
                    vec![0, 1],
                    InstData::CallExtern("__vow_string_parse_i64_opt_in_arena".to_string()),
                ),
                inst(
                    3,
                    Opcode::FieldGet,
                    Ty::I64,
                    vec![2],
                    InstData::FieldIndex(0),
                ),
                inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
            ],
        );

        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(is_known_builtin("__vow_string_parse_i64_opt_in_arena"));
        assert!(c.contains("__vow_option_t v2;"), "option declaration: {c}");
        assert!(
            c.contains("v2.tag = __VERIFIER_nondet_long();"),
            "parse model: {c}"
        );
        assert!(c.contains("v3 = v2.tag;"), "option projection: {c}");
        assert!(
            !c.contains("/* opcode Call not modelled */"),
            "arena parse must be modeled: {c}"
        );
    }

    #[test]
    fn emit_const_variants() {
        let func = make_func(
            "f",
            vec![],
            Ty::Unit,
            vec![
                inst(0, Opcode::ConstI32, Ty::I32, vec![], InstData::ConstI32(7)),
                inst(
                    1,
                    Opcode::ConstF32,
                    Ty::F32,
                    vec![],
                    InstData::ConstF32(1.5),
                ),
                inst(
                    2,
                    Opcode::ConstF64,
                    Ty::F64,
                    vec![],
                    InstData::ConstF64(2.0),
                ),
                inst(
                    3,
                    Opcode::ConstBool,
                    Ty::Bool,
                    vec![],
                    InstData::ConstBool(true),
                ),
                inst(
                    4,
                    Opcode::ConstBool,
                    Ty::Bool,
                    vec![],
                    InstData::ConstBool(false),
                ),
                inst(5, Opcode::ConstUnit, Ty::Unit, vec![], InstData::None),
                inst(6, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                inst(
                    7,
                    Opcode::ConstI128,
                    Ty::I128,
                    vec![],
                    InstData::ConstI128(i128::MAX),
                ),
                inst(
                    8,
                    Opcode::ConstU128,
                    Ty::U128,
                    vec![],
                    InstData::ConstU128(u128::MAX),
                ),
                inst(9, Opcode::Return, Ty::Unit, vec![], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("int32_t v0;"), "ConstI32 decl: {c}");
        assert!(c.contains("v0 = 7;"), "ConstI32 assign: {c}");
        assert!(c.contains("float v1;"), "ConstF32 decl: {c}");
        assert!(c.contains("v1 = 1.5f;"), "ConstF32 assign: {c}");
        assert!(c.contains("double v2;"), "ConstF64 decl: {c}");
        assert!(c.contains("v2 = 2;"), "ConstF64 assign: {c}");
        assert!(c.contains("_Bool v3;"), "ConstBool true decl: {c}");
        assert!(c.contains("v3 = 1;"), "ConstBool true assign: {c}");
        assert!(c.contains("_Bool v4;"), "ConstBool false decl: {c}");
        assert!(c.contains("v4 = 0;"), "ConstBool false assign: {c}");
        assert!(c.contains("int32_t v5;"), "ConstUnit decl: {c}");
        assert!(c.contains("v5 = 0;"), "ConstUnit assign: {c}");
        assert!(c.contains("int64_t v6;"), "ConstStr decl: {c}");
        assert!(c.contains("v6 = 0;"), "ConstStr assign: {c}");
        assert!(
            c.contains("/* opcode ConstI128 not modelled */"),
            "ConstI128 must use the deferred verifier fallback: {c}"
        );
        assert!(
            c.contains("/* opcode ConstU128 not modelled */"),
            "ConstU128 must use the deferred verifier fallback: {c}"
        );
    }

    #[test]
    fn wide_constants_report_the_unsupported_opcode() {
        for (opcode, ty, data, name) in [
            (
                Opcode::ConstI128,
                Ty::I128,
                InstData::ConstI128(i128::MAX),
                "ConstI128",
            ),
            (
                Opcode::ConstU128,
                Ty::U128,
                InstData::ConstU128(u128::MAX),
                "ConstU128",
            ),
        ] {
            let (func, module) = one_block_func_module(
                "wide",
                ty,
                vec![
                    inst(0, opcode, ty, vec![], data),
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            );
            let reason = non_modelable_reason(&func, &module, &HashMap::new());
            assert!(
                matches!(reason.as_deref(), Some(reason) if reason.contains(name)),
                "wide constant reason should name {name}: {reason:?}"
            );
        }
    }

    #[test]
    fn emit_void_function_returns_bare() {
        // Regression for issue #506: void functions must emit `return;`,
        // not `return v{N};` or `return 0;`. The IR lowerer always attaches
        // a ConstUnit value arg to Return, even for source-level `return;`
        // and implicit fall-through; so the gate must be on the function's
        // declared return type, not on whether the Return inst has args.
        let func = make_func(
            "void_fn",
            vec![],
            Ty::Unit,
            vec![
                inst(0, Opcode::ConstUnit, Ty::Unit, vec![], InstData::None),
                inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("void vow_user_fn_0("), "void signature: {c}");
        assert!(c.contains("  return;\n"), "bare return: {c}");
        assert!(!c.contains("return v0;"), "no value returned: {c}");
        assert!(!c.contains("return 0;"), "no value returned: {c}");
    }

    #[test]
    fn emit_arithmetic_ops() {
        let func = make_func(
            "arith",
            vec![Ty::I64, Ty::I64],
            Ty::I64,
            vec![
                inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                inst(2, Opcode::WrappingSub, Ty::I64, vec![0, 1], InstData::None),
                inst(3, Opcode::WrappingMul, Ty::I64, vec![0, 1], InstData::None),
                inst(4, Opcode::WrappingDiv, Ty::I64, vec![0, 1], InstData::None),
                inst(5, Opcode::WrappingRem, Ty::I64, vec![0, 1], InstData::None),
                inst(6, Opcode::WrappingAdd, Ty::I32, vec![0, 1], InstData::None),
                inst(7, Opcode::WrappingSub, Ty::I32, vec![0, 1], InstData::None),
                inst(8, Opcode::WrappingMul, Ty::I32, vec![0, 1], InstData::None),
                inst(9, Opcode::WrappingDiv, Ty::I32, vec![0, 1], InstData::None),
                inst(10, Opcode::WrappingRem, Ty::I32, vec![0, 1], InstData::None),
                inst(11, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("v0 - v1"), "sub: {c}");
        assert!(c.contains("v0 * v1"), "mul: {c}");
        assert!(c.contains("v0 / v1"), "div: {c}");
        assert!(c.contains("v0 % v1"), "rem: {c}");
    }

    #[test]
    fn emit_float_arithmetic() {
        let func = make_func(
            "floats",
            vec![Ty::F64, Ty::F64],
            Ty::F64,
            vec![
                inst(0, Opcode::GetArg, Ty::F64, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::GetArg, Ty::F64, vec![], InstData::ArgIndex(1)),
                inst(2, Opcode::AddF64, Ty::F64, vec![0, 1], InstData::None),
                inst(3, Opcode::SubF64, Ty::F64, vec![0, 1], InstData::None),
                inst(4, Opcode::MulF64, Ty::F64, vec![0, 1], InstData::None),
                inst(5, Opcode::DivF64, Ty::F64, vec![0, 1], InstData::None),
                inst(6, Opcode::AddF32, Ty::F32, vec![0, 1], InstData::None),
                inst(7, Opcode::SubF32, Ty::F32, vec![0, 1], InstData::None),
                inst(8, Opcode::MulF32, Ty::F32, vec![0, 1], InstData::None),
                inst(9, Opcode::DivF32, Ty::F32, vec![0, 1], InstData::None),
                inst(10, Opcode::RemF32, Ty::F32, vec![0, 1], InstData::None),
                inst(11, Opcode::RemF64, Ty::F64, vec![0, 1], InstData::None),
                inst(12, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("v0 + v1"), "fadd: {c}");
        assert!(c.contains("v0 - v1"), "fsub: {c}");
        assert!(c.contains("v0 * v1"), "fmul: {c}");
        assert!(c.contains("v0 / v1"), "fdiv: {c}");
        assert!(c.contains("float rem not modelled"), "frem32: {c}");
        assert!(c.contains("float rem not modelled"), "frem64: {c}");
    }

    #[test]
    fn emit_comparisons() {
        let func = make_func(
            "cmp",
            vec![Ty::I64, Ty::I64],
            Ty::Bool,
            vec![
                inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                inst(2, Opcode::Eq, Ty::Bool, vec![0, 1], InstData::None),
                inst(3, Opcode::Ne, Ty::Bool, vec![0, 1], InstData::None),
                inst(4, Opcode::Lt, Ty::Bool, vec![0, 1], InstData::None),
                inst(5, Opcode::Le, Ty::Bool, vec![0, 1], InstData::None),
                inst(6, Opcode::Gt, Ty::Bool, vec![0, 1], InstData::None),
                inst(7, Opcode::Ge, Ty::Bool, vec![0, 1], InstData::None),
                inst(8, Opcode::Eq, Ty::Bool, vec![0, 1], InstData::None),
                inst(9, Opcode::Ne, Ty::Bool, vec![0, 1], InstData::None),
                inst(10, Opcode::Lt, Ty::Bool, vec![0, 1], InstData::None),
                inst(11, Opcode::Le, Ty::Bool, vec![0, 1], InstData::None),
                inst(12, Opcode::Gt, Ty::Bool, vec![0, 1], InstData::None),
                inst(13, Opcode::Ge, Ty::Bool, vec![0, 1], InstData::None),
                inst(14, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("v0 == v1"), "eq: {c}");
        assert!(c.contains("v0 != v1"), "ne: {c}");
        assert!(c.contains("v0 < v1"), "lt: {c}");
        assert!(c.contains("v0 <= v1"), "le: {c}");
        assert!(c.contains("v0 > v1"), "gt: {c}");
        assert!(c.contains("v0 >= v1"), "ge: {c}");
    }

    #[test]
    fn emit_boolean_ops() {
        let func = make_func(
            "bools",
            vec![Ty::Bool, Ty::Bool],
            Ty::Bool,
            vec![
                inst(0, Opcode::GetArg, Ty::Bool, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::GetArg, Ty::Bool, vec![], InstData::ArgIndex(1)),
                inst(2, Opcode::Not, Ty::Bool, vec![0], InstData::None),
                inst(3, Opcode::And, Ty::Bool, vec![0, 1], InstData::None),
                inst(4, Opcode::Or, Ty::Bool, vec![0, 1], InstData::None),
                inst(5, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("!v0"), "not: {c}");
        assert!(c.contains("v0 && v1"), "and: {c}");
        assert!(c.contains("v0 || v1"), "or: {c}");
    }

    #[test]
    fn emit_integer_bitwise_ops() {
        let func = make_func(
            "bits",
            vec![Ty::I64, Ty::I64],
            Ty::I64,
            vec![
                inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                inst(2, Opcode::BitAnd, Ty::I64, vec![0, 1], InstData::None),
                inst(3, Opcode::BitOr, Ty::I64, vec![0, 1], InstData::None),
                inst(4, Opcode::BitXor, Ty::I64, vec![0, 1], InstData::None),
                inst(5, Opcode::Shl, Ty::I64, vec![0, 1], InstData::None),
                inst(6, Opcode::Shr, Ty::I64, vec![0, 1], InstData::None),
                inst(7, Opcode::Return, Ty::Unit, vec![6], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("v0 & v1"), "bitand: {c}");
        assert!(c.contains("v0 | v1"), "bitor: {c}");
        assert!(c.contains("v0 ^ v1"), "xor: {c}");
        assert!(c.contains("__vow_shl_i64(v0, v1)"), "shl: {c}");
        assert!(c.contains("__vow_shr_i64(v0, v1)"), "shr: {c}");
    }

    #[test]
    fn emit_c_module_includes_only_needed_shift_helpers() {
        let func = make_func(
            "bits",
            vec![Ty::I64, Ty::I64],
            Ty::I64,
            vec![
                inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                inst(2, Opcode::Shl, Ty::I64, vec![0, 1], InstData::None),
                inst(3, Opcode::Shr, Ty::U64, vec![0, 1], InstData::None),
                inst(4, Opcode::Return, Ty::Unit, vec![], InstData::None),
            ],
        );
        let c = emit_c_module(&[&func], &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("static inline int64_t __vow_shl_i64"),
            "shl_i64 should be present: {c}"
        );
        assert!(
            !c.contains("static inline int64_t __vow_shr_i64"),
            "shr_i64 should NOT be present: {c}"
        );
        assert!(
            !c.contains("static inline uint64_t __vow_shl_u64"),
            "shl_u64 should NOT be present: {c}"
        );
        assert!(
            c.contains("static inline uint64_t __vow_shr_u64"),
            "shr_u64 should be present: {c}"
        );
    }

    #[test]
    fn emit_c_module_omits_shift_helpers_when_unused() {
        let func = make_func(
            "add",
            vec![Ty::I64, Ty::I64],
            Ty::I64,
            vec![
                inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                inst(2, Opcode::WrappingAdd, Ty::I64, vec![0, 1], InstData::None),
                inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
            ],
        );
        let c = emit_c_module(&[&func], &HashMap::new(), &VerifyLimits::default());
        assert!(
            !c.contains("__vow_shl_i64"),
            "no shift helpers should be present: {c}"
        );
        assert!(
            !c.contains("__vow_shr_i64"),
            "no shift helpers should be present: {c}"
        );
        assert!(
            !c.contains("__vow_shl_u64"),
            "no shift helpers should be present: {c}"
        );
        assert!(
            !c.contains("__vow_shr_u64"),
            "no shift helpers should be present: {c}"
        );
    }

    #[test]
    fn emit_control_flow_branch_jump_unreachable() {
        use vow_ir::InstId;
        let func = Function {
            id: FuncId(0),
            name: "cfg".to_string(),
            params: vec![Ty::Bool],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::Bool, vec![], InstData::ArgIndex(0)),
                        Inst {
                            id: InstId(1),
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
                        inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(1)),
                        inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                    ],
                },
                BasicBlock {
                    id: BlockId(2),
                    insts: vec![inst(
                        4,
                        Opcode::Unreachable,
                        Ty::Unit,
                        vec![],
                        InstData::None,
                    )],
                },
            ],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("if (v0) goto block1; else goto block2;"),
            "branch: {c}"
        );
        assert!(c.contains("block2:;"), "block label: {c}");
        assert!(c.contains("__ESBMC_assume(0)"), "unreachable: {c}");
    }

    #[test]
    fn emit_phi_upsilon() {
        use vow_ir::InstId;
        let func = make_func(
            "phi_fn",
            vec![],
            Ty::I64,
            vec![
                Inst {
                    id: InstId(0),
                    opcode: Opcode::Phi,
                    ty: Ty::I64,
                    args: vec![],
                    data: InstData::None,
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(42)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Upsilon,
                    ty: Ty::Unit,
                    args: vec![InstId(1)],
                    data: InstData::PhiTarget(InstId(0)),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(3, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("int64_t v0;"), "phi declaration: {c}");
        assert!(c.contains("v0 = __ups_1;"), "upsilon assignment: {c}");
    }

    #[test]
    fn emit_unsupported_ops_fail_closed() {
        use vow_ir::InstId;
        let func = make_func(
            "nd",
            vec![],
            Ty::I64,
            vec![
                Inst {
                    id: InstId(0),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![],
                    data: InstData::CallTarget(FuncId(1)),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(1),
                    opcode: Opcode::FieldGet,
                    ty: Ty::I64,
                    args: vec![InstId(0)],
                    data: InstData::FieldIndex(0),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(2, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("unsupported opcode in verifier model"),
            "fail-closed assert: {c}"
        );
        assert!(
            c.contains(&format!("vow:{UNSUPPORTED_OP_VOW_ID}")),
            "sentinel vow id in assert: {c}"
        );
        assert!(c.contains("__VERIFIER_nondet_long"), "nondet for I64: {c}");
    }

    #[test]
    fn emit_vow_invariant_as_assert() {
        use vow_ir::{InstId, VowId};
        let func = make_func(
            "inv",
            vec![],
            Ty::Bool,
            vec![
                inst(
                    0,
                    Opcode::ConstBool,
                    Ty::Bool,
                    vec![],
                    InstData::ConstBool(true),
                ),
                Inst {
                    id: InstId(1),
                    opcode: Opcode::VowInvariant,
                    ty: Ty::Unit,
                    args: vec![InstId(0)],
                    data: InstData::VowId(VowId(2)),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(2, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("__ESBMC_assert(v0, \"vow:2\")"),
            "invariant assert: {c}"
        );
    }

    #[test]
    fn emit_return_no_value() {
        let func = make_func(
            "void_fn",
            vec![],
            Ty::Unit,
            vec![inst(0, Opcode::Return, Ty::Unit, vec![], InstData::None)],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("return;"), "bare void return: {c}");
        assert!(!c.contains("return 0;"), "no value in void return: {c}");
    }

    #[test]
    fn emit_c_module_wraps_multiple_functions() {
        let mut f1 = make_func(
            "f1",
            vec![],
            Ty::Unit,
            vec![inst(0, Opcode::Return, Ty::Unit, vec![], InstData::None)],
        );
        f1.id = FuncId(1);
        let mut f2 = make_func(
            "f2",
            vec![Ty::I64],
            Ty::I64,
            vec![
                inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        f2.id = FuncId(2);
        let out = emit_c_module(&[&f1, &f2], &HashMap::new(), &VerifyLimits::default());
        assert!(out.contains("#include <stdint.h>"), "includes: {out}");
        assert!(out.contains("__ESBMC_assume"), "esbmc assume: {out}");
        assert!(
            out.contains("void vow_user_fn_1(void)"),
            "f1 signature: {out}"
        );
        assert!(out.contains("vow_user_fn_2("), "f2 signature: {out}");
    }

    #[test]
    fn emit_vow_ensures_as_assert() {
        let func = Function {
            id: FuncId(0),
            name: "f".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::Bool,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "result".to_string(),
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
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(2, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("__ESBMC_assert(v0"), "ensures: {c}");
    }

    #[test]
    fn emit_c_module_includes_vec_typedef() {
        let f = make_func(
            "f",
            vec![],
            Ty::Unit,
            vec![inst(0, Opcode::Return, Ty::Unit, vec![], InstData::None)],
        );
        let out = emit_c_module(&[&f], &HashMap::new(), &VerifyLimits::default());
        assert!(out.contains("__vow_vec_t"), "vec typedef: {out}");
        assert!(out.contains("int64_t len"), "vec len field: {out}");
        assert!(
            out.contains("int64_t data[128]"),
            "vec data array field: {out}"
        );
    }

    #[test]
    fn emit_vec_new() {
        use vow_ir::InstId;
        let func = make_func(
            "make_vec",
            vec![],
            Ty::Ptr,
            vec![
                inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_vec_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("__vow_vec_t v2;"), "vec struct decl: {c}");
        assert!(c.contains("v2.len = 0;"), "vec len init: {c}");
        assert!(
            c.contains("return 0; /* modelled type return */"),
            "vec return: {c}"
        );
    }

    #[test]
    fn emit_vec_push() {
        use vow_ir::InstId;
        let func = make_func(
            "push_one",
            vec![],
            Ty::Ptr,
            vec![
                inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_vec_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(3, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(42)),
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(2), InstId(3)],
                    data: InstData::CallExtern("__vow_vec_push_val".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(5, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("vec capacity"),
            "push must have capacity assertion: {c}"
        );
        assert!(c.contains("v2.data[v2.len] = v3;"), "push store: {c}");
        assert!(c.contains("v2.len++;"), "push increment: {c}");
    }

    #[test]
    fn emit_explicit_arena_vec_push_val() {
        use vow_ir::InstId;
        let func = make_func(
            "push_one_in_arena",
            vec![],
            Ty::Ptr,
            vec![
                inst(
                    0,
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(1000),
                ),
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0), InstId(1), InstId(2)],
                    data: InstData::CallExtern("__vow_vec_new_in_arena".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(4, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(42)),
                Inst {
                    id: InstId(5),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(0), InstId(3), InstId(4)],
                    data: InstData::CallExtern("__vow_vec_push_val_in_arena".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(6, Opcode::Return, Ty::Unit, vec![3], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("__vow_vec_t v3;"), "arena vec decl: {c}");
        assert!(
            !c.contains("__vow_vec_t v0;"),
            "arena pointer must not be tracked as vec: {c}"
        );
        assert!(c.contains("v3.len = 0;"), "arena vec init: {c}");
        assert!(
            c.contains("v3.data[v3.len] = v4;"),
            "arena push value store: {c}"
        );
        assert!(c.contains("v3.len++;"), "arena push increment: {c}");
        assert!(
            !c.contains("not modelled"),
            "arena vec calls should be modelled: {c}"
        );
    }

    #[test]
    fn emit_explicit_arena_vec_reserve_and_generic_push() {
        use vow_ir::InstId;
        let func = make_func(
            "reserve_and_push_in_arena",
            vec![],
            Ty::Ptr,
            vec![
                inst(
                    0,
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(1000),
                ),
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_vec_new_val_in_arena".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(4, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(1)),
                Inst {
                    id: InstId(5),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(0), InstId(3), InstId(4), InstId(1), InstId(2)],
                    data: InstData::CallExtern("__vow_vec_reserve_in_arena".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(6),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(0), InstId(3), InstId(4), InstId(1), InstId(2)],
                    data: InstData::CallExtern("__vow_vec_push_in_arena".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(7, Opcode::Return, Ty::Unit, vec![3], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("v3.data[v3.len] = __VERIFIER_nondet_long();"),
            "generic arena push should over-approximate element value: {c}"
        );
        assert!(c.contains("v3.len++;"), "generic arena push increment: {c}");
        assert!(
            !c.contains("not modelled"),
            "reserve/generic arena push should be modelled: {c}"
        );
    }

    #[test]
    fn emit_vec_len() {
        use vow_ir::InstId;
        let func = make_func(
            "get_len",
            vec![],
            Ty::I64,
            vec![
                inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_vec_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![InstId(2)],
                    data: InstData::CallExtern("__vow_vec_len".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("v3 = v2.len;"), "vec len: {c}");
    }

    #[test]
    fn emit_vec_get_with_bounds() {
        use vow_ir::InstId;
        let func = make_func(
            "get_elem",
            vec![],
            Ty::I64,
            vec![
                inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_vec_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(3, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![InstId(2), InstId(3)],
                    data: InstData::CallExtern("__vow_vec_get_val".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(5, Opcode::Return, Ty::Unit, vec![4], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("__ESBMC_assert(v3 >= 0 && v3 < v2.len"),
            "bounds check: {c}"
        );
        assert!(c.contains("v4 = v2.data[v3]"), "get access: {c}");
    }

    #[test]
    fn emit_vec_pop() {
        use vow_ir::InstId;
        let func = make_func(
            "pop_one",
            vec![],
            Ty::Ptr,
            vec![
                inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_vec_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(2)],
                    data: InstData::CallExtern("__vow_vec_pop".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(4, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("if (v2.len > 0) { v2.len--; }"),
            "pop decrement: {c}"
        );
    }

    #[test]
    fn emit_vec_set_with_bounds() {
        use vow_ir::InstId;
        let func = make_func(
            "set_elem",
            vec![],
            Ty::Unit,
            vec![
                inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_vec_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(3, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                inst(4, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(99)),
                Inst {
                    id: InstId(5),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(2), InstId(3), InstId(4)],
                    data: InstData::CallExtern("__vow_vec_set_val".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(6, Opcode::Return, Ty::Unit, vec![], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("__ESBMC_assert(v3 >= 0 && v3 < v2.len"),
            "bounds check: {c}"
        );
        assert!(c.contains("v2.data[v3] = v4"), "set store: {c}");
    }

    #[test]
    fn emit_vec_phi_propagation() {
        use vow_ir::InstId;
        let func = Function {
            id: FuncId(0),
            name: "vec_phi".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::Ptr,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(0),
                        opcode: Opcode::Phi,
                        ty: Ty::Ptr,
                        args: vec![],
                        data: InstData::None,
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                    inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                    Inst {
                        id: InstId(3),
                        opcode: Opcode::Call,
                        ty: Ty::Ptr,
                        args: vec![InstId(1), InstId(2)],
                        data: InstData::CallExtern("__vow_vec_new".to_string()),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(4),
                        opcode: Opcode::Upsilon,
                        ty: Ty::Unit,
                        args: vec![InstId(3)],
                        data: InstData::PhiTarget(InstId(0)),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(5, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("__vow_vec_t v0;"), "phi uses vec type: {c}");
    }

    #[test]
    fn emit_non_vec_call_is_unsupported_for_verification() {
        use vow_ir::InstId;
        let func = make_func(
            "other",
            vec![],
            Ty::I64,
            vec![
                Inst {
                    id: InstId(0),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![],
                    data: InstData::CallExtern("__some_other_func".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("unsupported opcode in verifier model: Call"),
            "non-vec call must fail closed: {c}"
        );
        assert!(
            c.contains(&format!("vow:{UNSUPPORTED_OP_VOW_ID}")),
            "sentinel vow id in assert: {c}"
        );
        assert!(
            c.contains("__VERIFIER_nondet_long"),
            "nondet for non-vec: {c}"
        );
    }

    #[test]
    fn emit_c_module_includes_string_typedef() {
        let f = make_func(
            "f",
            vec![],
            Ty::Unit,
            vec![inst(0, Opcode::Return, Ty::Unit, vec![], InstData::None)],
        );
        let out = emit_c_module(&[&f], &HashMap::new(), &VerifyLimits::default());
        assert!(out.contains("__vow_string_t"), "string typedef: {out}");
        assert!(
            out.contains("int8_t data[256]"),
            "string data array field: {out}"
        );
    }

    #[test]
    fn emit_getarg_container_bounds() {
        use vow_ir::InstId;
        // A String parameter: GetArg(0) followed by __vow_string_len to mark it.
        let str_func = Function {
            id: FuncId(0),
            name: "str_arg".to_string(),
            params: vec![Ty::Ptr],
            param_names: vec!["s".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(0)],
                        data: InstData::CallExtern("__vow_string_len".to_string()),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&str_func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("__ESBMC_assume(v0.len >= 0 && v0.len <= 256)"),
            "GetArg bound must include len == string_max (reachable via push_byte): {c}"
        );
        assert!(
            !c.contains("INT64_MAX"),
            "GetArg bound must not use INT64_MAX: {c}"
        );
        assert!(
            !c.contains("v0.data = "),
            "GetArg must not assign to fixed-array data field: {c}"
        );

        // A Vec<i64> parameter: GetArg(0) followed by __vow_vec_len.
        let vec_func = Function {
            id: FuncId(0),
            name: "vec_arg".to_string(),
            params: vec![Ty::Ptr],
            param_names: vec!["v".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(0)],
                        data: InstData::CallExtern("__vow_vec_len".to_string()),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&vec_func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("__ESBMC_assume(v0.len >= 0 && v0.len <= 128)"),
            "GetArg bound must include len == vec_max: {c}"
        );
        assert!(!c.contains("v0.data = "), "no data assignment: {c}");

        // A HashMap parameter: GetArg(0) followed by __vow_map_len.
        let map_func = Function {
            id: FuncId(0),
            name: "map_arg".to_string(),
            params: vec![Ty::Ptr],
            param_names: vec!["m".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(0)],
                        data: InstData::CallExtern("__vow_map_len".to_string()),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&map_func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("__ESBMC_assume(v0.len >= 0 && v0.len <= 64)"),
            "GetArg bound must include len == hashmap_max: {c}"
        );
        assert!(!c.contains("v0.keys = "), "no keys assignment: {c}");
        assert!(!c.contains("v0.vals = "), "no vals assignment: {c}");
    }

    #[test]
    fn emit_string_substring_marks_parameter_receiver_as_string() {
        use vow_ir::InstId;
        let func = Function {
            id: FuncId(0),
            name: "slice_param".to_string(),
            params: vec![Ty::Ptr],
            param_names: vec!["s".to_string()],
            return_ty: Ty::Ptr,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(1)),
                    Inst {
                        id: InstId(3),
                        opcode: Opcode::Call,
                        ty: Ty::Ptr,
                        args: vec![InstId(0), InstId(1), InstId(2)],
                        data: InstData::CallExtern("__vow_string_substring".to_string()),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("__vow_string_t v0;"),
            "substring source receiver must be emitted as a String model value: {c}"
        );
        assert!(
            !c.contains("int64_t v0;"),
            "substring source receiver must not be emitted as scalar int64_t: {c}"
        );
        assert!(
            c.contains("v3.len = __substring_end_3 - __substring_start_3"),
            "substring result model should still be emitted: {c}"
        );
    }

    #[test]
    fn emit_string_substr_in_arena_marks_shifted_receiver_as_string() {
        use vow_ir::InstId;
        let func = Function {
            id: FuncId(0),
            name: "arena_slice_param".to_string(),
            params: vec![Ty::Ptr],
            param_names: vec!["s".to_string()],
            return_ty: Ty::Ptr,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    inst(1, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    inst(3, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(1)),
                    Inst {
                        id: InstId(4),
                        opcode: Opcode::Call,
                        ty: Ty::Ptr,
                        args: vec![InstId(0), InstId(1), InstId(2), InstId(3)],
                        data: InstData::CallExtern("__vow_string_substr_in_arena".to_string()),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(5, Opcode::Return, Ty::Unit, vec![4], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("__vow_string_t v1;"),
            "arena substr source receiver must be emitted as a String model value: {c}"
        );
        assert!(
            !c.contains("int64_t v1;"),
            "arena substr source receiver must not be emitted as scalar int64_t: {c}"
        );
        assert!(
            c.contains("v4.len = __substr_len_4"),
            "substr result model should still be emitted: {c}"
        );
    }

    #[test]
    fn emit_string_substr_models_runtime_clamping() {
        use vow_ir::InstId;
        let func = Function {
            id: FuncId(0),
            name: "substr_clamp".to_string(),
            params: vec![Ty::Ptr],
            param_names: vec!["s".to_string()],
            return_ty: Ty::Ptr,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(-1)),
                    inst(
                        2,
                        Opcode::ConstI64,
                        Ty::I64,
                        vec![],
                        InstData::ConstI64(999),
                    ),
                    Inst {
                        id: InstId(3),
                        opcode: Opcode::Call,
                        ty: Ty::Ptr,
                        args: vec![InstId(0), InstId(1), InstId(2)],
                        data: InstData::CallExtern("__vow_string_substr".to_string()),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            !c.contains("\"substr start\"") && !c.contains("\"substr len\""),
            "substr model should not assert inputs the runtime clamps: {c}"
        );
        assert!(
            c.contains("__substr_start_3") && c.contains("__substr_len_3"),
            "substr model should introduce clamped start/len temporaries: {c}"
        );
        assert!(
            c.contains("v3.data[__i] = v0.data[__substr_start_3 + __i];"),
            "substr copy should index with the clamped start: {c}"
        );
    }

    #[test]
    fn emit_string_substring_models_runtime_clamping() {
        use vow_ir::InstId;
        let func = Function {
            id: FuncId(0),
            name: "substring_clamp".to_string(),
            params: vec![Ty::Ptr],
            param_names: vec!["s".to_string()],
            return_ty: Ty::Ptr,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(-5)),
                    inst(
                        2,
                        Opcode::ConstI64,
                        Ty::I64,
                        vec![],
                        InstData::ConstI64(999),
                    ),
                    Inst {
                        id: InstId(3),
                        opcode: Opcode::Call,
                        ty: Ty::Ptr,
                        args: vec![InstId(0), InstId(1), InstId(2)],
                        data: InstData::CallExtern("__vow_string_substring".to_string()),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            !c.contains("\"substring start\"") && !c.contains("\"substring end\""),
            "substring model should not assert inputs the runtime clamps: {c}"
        );
        assert!(
            c.contains("__substring_start_3") && c.contains("__substring_end_3"),
            "substring model should introduce clamped start/end temporaries: {c}"
        );
        assert!(
            c.contains("v3.data[__i] = v0.data[__substring_start_3 + __i];"),
            "substring copy should index with the clamped start: {c}"
        );
    }

    #[test]
    fn emit_non_expanding_string_helpers_are_modelable() {
        use vow_ir::InstId;

        let cases = [
            ("__vow_string_trim", 1),
            ("__vow_string_trim_in_arena", 2),
            ("__vow_string_to_upper", 1),
            ("__vow_string_to_upper_in_arena", 2),
            ("__vow_string_to_lower", 1),
            ("__vow_string_to_lower_in_arena", 2),
        ];

        for (idx, (name, argc)) in cases.iter().enumerate() {
            let call_id = 20 + idx as u32;
            let args: Vec<InstId> = (0..*argc).map(|i| InstId(i as u32)).collect();
            let mut insts = Vec::new();
            for i in 0..*argc {
                insts.push(inst(
                    i as u32,
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(0),
                ));
            }
            insts.push(Inst {
                id: InstId(call_id),
                opcode: Opcode::Call,
                ty: Ty::Ptr,
                args,
                data: InstData::CallExtern((*name).to_string()),
                origin: sp(),
                region: RegionId::Root,
            });
            insts.push(inst(
                call_id + 1,
                Opcode::Return,
                Ty::Unit,
                vec![call_id],
                InstData::None,
            ));

            let func = Function {
                id: FuncId(idx as u32),
                name: format!("helper_{idx}"),
                params: vec![],
                param_names: vec![],
                return_ty: Ty::Ptr,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts,
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            };
            let module = Module {
                name: "test".to_string(),
                functions: vec![func.clone()],
                strings: vec![],
                struct_layouts: vec![],
                enum_layouts: vec![],
                warnings: vec![],
            };
            assert_eq!(
                non_modelable_reason(&func, &module, &HashMap::new()),
                None,
                "{name} should be modelable"
            );

            let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
            assert!(
                !c.contains("unsupported opcode in verifier model"),
                "{name} should not fall through to unsupported: {c}"
            );
            assert!(
                c.contains(&format!("__vow_string_t v{call_id};")),
                "{name} result should use the String model: {c}"
            );
            assert!(
                c.contains(&format!(
                    "__ESBMC_assume(v{call_id}.len >= 0 && v{call_id}.len < 256)"
                )),
                "{name} result length should be bounded by string_max: {c}"
            );
        }
    }

    #[test]
    fn emit_expanding_string_helpers_are_not_modelable() {
        use vow_ir::InstId;

        let cases = [
            ("__vow_string_replace", 3),
            ("__vow_string_replace_in_arena", 4),
            ("__vow_string_join", 2),
            ("__vow_string_join_in_arena", 3),
            ("__vow_string_split", 2),
            ("__vow_string_split_in_arena", 3),
        ];

        for (idx, (name, argc)) in cases.iter().enumerate() {
            let call_id = 40 + idx as u32;
            let args: Vec<InstId> = (0..*argc).map(|i| InstId(i as u32)).collect();
            let mut insts = Vec::new();
            for i in 0..*argc {
                insts.push(inst(
                    i as u32,
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(0),
                ));
            }
            insts.push(Inst {
                id: InstId(call_id),
                opcode: Opcode::Call,
                ty: Ty::Ptr,
                args,
                data: InstData::CallExtern((*name).to_string()),
                origin: sp(),
                region: RegionId::Root,
            });
            insts.push(inst(
                call_id + 1,
                Opcode::Return,
                Ty::Unit,
                vec![call_id],
                InstData::None,
            ));

            let func = Function {
                id: FuncId(idx as u32),
                name: format!("expanding_helper_{idx}"),
                params: vec![],
                param_names: vec![],
                return_ty: Ty::Ptr,
                effects: vec![],
                vows: vec![],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    insts,
                }],
                local_names: std::collections::HashMap::new(),
                summary: RegionSummary::default(),
                source_file: String::new(),
            };
            let module = Module {
                name: "test".to_string(),
                functions: vec![func.clone()],
                strings: vec![],
                struct_layouts: vec![],
                enum_layouts: vec![],
                warnings: vec![],
            };
            let reason = non_modelable_reason(&func, &module, &HashMap::new());
            assert!(
                matches!(reason.as_deref(), Some(text) if text.contains(name)),
                "{name} should stay non-modelable until its expanding length semantics are modeled; reason: {reason:?}"
            );

            let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
            assert!(
                c.contains("/* opcode Call not modelled */"),
                "{name} should fall through to the unmodelled-call path: {c}"
            );
        }
    }

    fn one_block_func_module(name: &str, ret: Ty, insts: Vec<Inst>) -> (Function, Module) {
        let func = Function {
            id: FuncId(0),
            name: name.to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: ret,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts,
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let module = Module {
            name: "test".to_string(),
            functions: vec![func.clone()],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        (func, module)
    }

    #[test]
    fn nested_vec_push_marked_non_modelable() {
        let (func, module) = one_block_func_module(
            "nested_push",
            Ty::Ptr,
            vec![
                inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_vec_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(3, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                inst(4, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                Inst {
                    id: InstId(5),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(3), InstId(4)],
                    data: InstData::CallExtern("__vow_vec_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(6),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(2), InstId(5)],
                    data: InstData::CallExtern("__vow_vec_push_val".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(7, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let reason = non_modelable_reason(&func, &module, &HashMap::new());
        assert!(
            matches!(reason.as_deref(), Some(text) if text.contains("non-scalar element")),
            "nested Vec<Vec<...>> push must be non-modelable with non-scalar element reason; got: {reason:?}"
        );
    }

    #[test]
    fn nested_vec_push_in_arena_marked_non_modelable() {
        // outer = __vow_vec_new_in_arena(arena, size, align)   // in vec_vars (creator)
        // inner = __vow_vec_new_in_arena(arena, size, align)   // in vec_vars (creator)
        // __vow_vec_push_val_in_arena(arena, outer, inner)     // value arg is at index 2
        let (func, module) = one_block_func_module(
            "nested_push_arena",
            Ty::Ptr,
            vec![
                inst(
                    0,
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(1000),
                ),
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0), InstId(1), InstId(2)],
                    data: InstData::CallExtern("__vow_vec_new_in_arena".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0), InstId(1), InstId(2)],
                    data: InstData::CallExtern("__vow_vec_new_in_arena".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(5),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(0), InstId(3), InstId(4)],
                    data: InstData::CallExtern("__vow_vec_push_val_in_arena".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(6, Opcode::Return, Ty::Unit, vec![3], InstData::None),
            ],
        );
        let reason = non_modelable_reason(&func, &module, &HashMap::new());
        assert!(
            matches!(reason.as_deref(), Some(text) if text.contains("__vow_vec_push_val_in_arena") && text.contains("non-scalar element")),
            "arena push of nested vec must be flagged with non-scalar element; got: {reason:?}"
        );
    }

    #[test]
    fn vec_get_val_of_string_marked_non_modelable() {
        // v       = __vow_vec_new(...)           // outer Vec, in vec_vars
        // got     = __vow_vec_get_val(v, 0)      // result type at IR is Ptr/i64
        // _       = __vow_string_len(got)        // forces `got` into string_vars
        // The get_val emit would be `v{got} = v{v}.data[..];` — a __vow_string_t
        // would be loaded from int64_t[] storage, which is ill-typed.
        let (func, module) = one_block_func_module(
            "vec_get_string",
            Ty::I64,
            vec![
                inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_vec_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(3, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(2), InstId(3)],
                    data: InstData::CallExtern("__vow_vec_get_val".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(5),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![InstId(4)],
                    data: InstData::CallExtern("__vow_string_len".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(6, Opcode::Return, Ty::Unit, vec![5], InstData::None),
            ],
        );
        let reason = non_modelable_reason(&func, &module, &HashMap::new());
        assert!(
            matches!(reason.as_deref(), Some(text) if text.contains("__vow_vec_get_val") && text.contains("non-scalar element")),
            "vec_get_val whose result is a string must be flagged with non-scalar element; got: {reason:?}"
        );
    }

    #[test]
    fn vec_set_val_with_structured_value_marked_non_modelable() {
        // outer = __vow_vec_new                       // in vec_vars
        // inner = __vow_vec_new                       // in vec_vars
        // __vow_vec_set_val(outer, idx, inner)        // value arg at index 2
        let (func, module) = one_block_func_module(
            "vec_set_struct",
            Ty::Ptr,
            vec![
                inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_vec_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_vec_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(4, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                Inst {
                    id: InstId(5),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(2), InstId(4), InstId(3)],
                    data: InstData::CallExtern("__vow_vec_set_val".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(6, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let reason = non_modelable_reason(&func, &module, &HashMap::new());
        assert!(
            matches!(reason.as_deref(), Some(text) if text.contains("__vow_vec_set_val") && text.contains("non-scalar element")),
            "vec_set_val with structured value must be flagged; got: {reason:?}"
        );
    }

    #[test]
    fn flat_vec_i64_push_still_modelable() {
        // Regression: flat Vec<i64> with scalar pushes must remain modelable.
        let (func, module) = one_block_func_module(
            "flat_push",
            Ty::Ptr,
            vec![
                inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_vec_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(3, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(42)),
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(2), InstId(3)],
                    data: InstData::CallExtern("__vow_vec_push_val".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(5, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        assert_eq!(
            non_modelable_reason(&func, &module, &HashMap::new()),
            None,
            "flat Vec<i64> push of an i64 const must remain modelable"
        );
    }

    #[test]
    fn emit_fieldget_container_bounds() {
        use vow_ir::InstId;
        // FieldGet result used as a string should be nondeterministic within model bounds.
        let str_func = Function {
            id: FuncId(0),
            name: "str_field".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::FieldGet,
                        ty: Ty::Ptr,
                        args: vec![InstId(0)],
                        data: InstData::FieldIndex(0),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(1)],
                        data: InstData::CallExtern("__vow_string_len".to_string()),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&str_func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("__ESBMC_assume(v1.len >= 0 && v1.len <= 256)"),
            "FieldGet string bound must include string_max: {c}"
        );
        assert!(
            !c.contains("/* FieldGet -> string */ v1.len = 0;"),
            "FieldGet string must not force empty value: {c}"
        );

        // FieldGet result used as a vec should be nondeterministic within model bounds.
        let vec_func = Function {
            id: FuncId(0),
            name: "vec_field".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::FieldGet,
                        ty: Ty::Ptr,
                        args: vec![InstId(0)],
                        data: InstData::FieldIndex(0),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(1)],
                        data: InstData::CallExtern("__vow_vec_len".to_string()),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&vec_func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("__ESBMC_assume(v1.len >= 0 && v1.len <= 128)"),
            "FieldGet vec bound must include vec_max: {c}"
        );
        assert!(
            !c.contains("/* FieldGet -> vec */ v1.len = 0;"),
            "FieldGet vec must not force empty value: {c}"
        );

        // FieldGet result used as a hashmap should be nondeterministic within model bounds.
        let map_func = Function {
            id: FuncId(0),
            name: "map_field".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::FieldGet,
                        ty: Ty::Ptr,
                        args: vec![InstId(0)],
                        data: InstData::FieldIndex(0),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(1)],
                        data: InstData::CallExtern("__vow_map_len".to_string()),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&map_func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("__ESBMC_assume(v1.len >= 0 && v1.len <= 64)"),
            "FieldGet map bound must include hashmap_max: {c}"
        );
        assert!(
            !c.contains("/* FieldGet -> hashmap */ v1.len = 0;"),
            "FieldGet map must not force empty value: {c}"
        );
    }

    #[test]
    fn emit_string_eq_self_comparison_is_reflexive() {
        use vow_ir::InstId;
        let func = make_func(
            "self_eq",
            vec![],
            Ty::Bool,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                Inst {
                    id: InstId(1),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Bool,
                    args: vec![InstId(1), InstId(1)],
                    data: InstData::CallExtern("__vow_string_eq".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("v2 = 1;"),
            "string_eq(x, x) must be reflexive (emit `= 1`): {c}"
        );
        assert!(
            !c.contains("__VERIFIER_nondet_bool"),
            "self-compare should not use nondet: {c}"
        );
    }

    #[test]
    fn emit_string_from_cstr() {
        use vow_ir::InstId;
        let func = make_func(
            "make_str",
            vec![],
            Ty::Ptr,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                Inst {
                    id: InstId(1),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("__vow_string_t v1;"), "string struct decl: {c}");
        assert!(
            c.contains("v1.len = __VERIFIER_nondet_long()"),
            "nondet len: {c}"
        );
        assert!(
            c.contains("__ESBMC_assume(v1.len >= 0 && v1.len < 256)"),
            "len bounded by string_max: {c}"
        );
        assert!(
            c.contains("return 0; /* modelled type return */"),
            "string return: {c}"
        );
    }

    #[test]
    fn emit_string_literal_uses_pool_length() {
        let func = make_func(
            "make_literal",
            vec![],
            Ty::Ptr,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                inst(
                    1,
                    Opcode::Call,
                    Ty::Ptr,
                    vec![0],
                    InstData::CallExtern("__vow_string_literal".to_string()),
                ),
                inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
            ],
        );
        let module = Module {
            name: String::new(),
            functions: vec![func.clone()],
            strings: vec!["hello".to_string()],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        let c = emit_c_function_full(
            &func,
            &HashMap::new(),
            &HashSet::new(),
            &module,
            &VerifyLimits::default(),
            false,
            false,
            false,
        );
        assert!(c.contains("__vow_string_t v1;"), "string struct decl: {c}");
        assert!(c.contains("v1.len = 5;"), "literal len from pool: {c}");
        assert!(
            c.contains("v1.data[0] = (int8_t)104;"),
            "literal byte h from pool: {c}"
        );
        assert!(
            c.contains("v1.data[4] = (int8_t)111;"),
            "literal byte o from pool: {c}"
        );
    }

    #[test]
    fn emit_c_module_grows_string_model_for_literal_bytes() {
        let func = make_func(
            "make_literal",
            vec![],
            Ty::Ptr,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                inst(
                    1,
                    Opcode::Call,
                    Ty::Ptr,
                    vec![0],
                    InstData::CallExtern("__vow_string_literal".to_string()),
                ),
                inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
            ],
        );
        let module = Module {
            name: String::new(),
            functions: vec![func.clone()],
            strings: vec!["hello".to_string()],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        let limits = VerifyLimits {
            string_max: 4,
            ..VerifyLimits::default()
        };
        let c = emit_c_module_with_callees(
            &func,
            &module,
            &HashMap::new(),
            &[],
            &HashSet::new(),
            &limits,
            false,
            false,
        );
        assert!(
            c.contains("typedef struct { int64_t len; int8_t data[5]; } __vow_string_t;"),
            "literal bytes must fit in the string model: {c}"
        );
        assert!(
            c.contains("v1.data[4] = (int8_t)111;"),
            "last literal byte should be emitted without exceeding the model: {c}"
        );
    }

    #[test]
    fn emit_string_clone_copies_source_model() {
        let func = make_func(
            "clone_literal",
            vec![],
            Ty::Ptr,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                inst(
                    1,
                    Opcode::Call,
                    Ty::Ptr,
                    vec![0],
                    InstData::CallExtern("__vow_string_literal".to_string()),
                ),
                inst(
                    2,
                    Opcode::Call,
                    Ty::Ptr,
                    vec![1],
                    InstData::CallExtern("__vow_string_clone".to_string()),
                ),
                inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let module = Module {
            name: String::new(),
            functions: vec![func.clone()],
            strings: vec!["hello".to_string()],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        let c = emit_c_function_full(
            &func,
            &HashMap::new(),
            &HashSet::new(),
            &module,
            &VerifyLimits::default(),
            false,
            false,
            false,
        );
        assert!(c.contains("__vow_string_t v2;"), "clone decl: {c}");
        assert!(c.contains("v2 = v1;"), "clone preserves source model: {c}");
    }

    #[test]
    fn emit_unsigned_integer_formatters_as_bounded_nondet_strings() {
        let (func, module) = one_block_func_module(
            "format_unsigned",
            Ty::Ptr,
            vec![
                inst(
                    0,
                    Opcode::ConstU64,
                    Ty::U64,
                    vec![],
                    InstData::ConstU64(u64::MAX),
                ),
                inst(
                    1,
                    Opcode::Call,
                    Ty::Ptr,
                    vec![0],
                    InstData::CallExtern("__vow_string_from_u64".to_string()),
                ),
                inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                inst(
                    3,
                    Opcode::Call,
                    Ty::Ptr,
                    vec![2, 0],
                    InstData::CallExtern("__vow_string_from_u64_in_arena".to_string()),
                ),
                inst(4, Opcode::Return, Ty::Unit, vec![1], InstData::None),
            ],
        );

        assert_eq!(
            non_modelable_reason(&func, &module, &HashMap::new()),
            None,
            "unsigned formatters must remain modelable"
        );

        let limits = VerifyLimits::default();
        let c = emit_c_function(&func, &HashMap::new(), &limits);
        for id in [1, 3] {
            assert!(
                c.contains(&format!("__vow_string_t v{id};")),
                "string result: {c}"
            );
            assert!(
                c.contains(&format!("v{id}.len = __VERIFIER_nondet_long();")),
                "nondeterministic length: {c}"
            );
            assert!(
                c.contains(&format!(
                    "__ESBMC_assume(v{id}.len >= 0 && v{id}.len < {});",
                    limits.string_max
                )),
                "bounded length: {c}"
            );
        }
    }

    #[test]
    fn emit_string_push_str_models_source_param_as_string() {
        let func = make_func(
            "append_param",
            vec![Ty::Ptr, Ty::Ptr],
            Ty::Ptr,
            vec![
                inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
                inst(
                    2,
                    Opcode::Call,
                    Ty::Unit,
                    vec![0, 1],
                    InstData::CallExtern("__vow_string_push_str".to_string()),
                ),
                inst(3, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let c = emit_c_function_full(
            &func,
            &HashMap::new(),
            &HashSet::new(),
            &Module {
                name: String::new(),
                functions: vec![func.clone()],
                strings: vec![],
                struct_layouts: vec![],
                enum_layouts: vec![],
                warnings: vec![],
            },
            &VerifyLimits::default(),
            false,
            false,
            false,
        );
        assert!(c.contains("__vow_string_t v0;"), "dest param model: {c}");
        assert!(c.contains("__vow_string_t v1;"), "source param model: {c}");
        assert!(
            !c.contains("int64_t v1 = p1;"),
            "source param must not be scalar: {c}"
        );
    }

    #[test]
    fn emit_string_len() {
        use vow_ir::InstId;
        let func = make_func(
            "str_len",
            vec![],
            Ty::I64,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                Inst {
                    id: InstId(1),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![InstId(1)],
                    data: InstData::CallExtern("__vow_string_len".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("v2 = v1.len;"), "string len: {c}");
    }

    #[test]
    fn emit_string_push_byte() {
        use vow_ir::InstId;
        let func = make_func(
            "push_byte",
            vec![],
            Ty::Ptr,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                Inst {
                    id: InstId(1),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(65)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(1), InstId(2)],
                    data: InstData::CallExtern("__vow_string_push_byte".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(4, Opcode::Return, Ty::Unit, vec![1], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("string capacity"),
            "push_byte must have capacity assertion: {c}"
        );
        assert!(
            c.contains("v1.data[v1.len] = (int8_t)v2;"),
            "push_byte store: {c}"
        );
        assert!(c.contains("v1.len++;"), "push_byte increment: {c}");
    }

    #[test]
    fn emit_string_push_str() {
        use vow_ir::InstId;
        let func = make_func(
            "cat",
            vec![],
            Ty::Ptr,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                Inst {
                    id: InstId(1),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(2, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(1)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(2)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(1), InstId(3)],
                    data: InstData::CallExtern("__vow_string_push_str".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(5, Opcode::Return, Ty::Unit, vec![1], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("string capacity"),
            "push_str must have capacity assertion: {c}"
        );
        assert!(
            c.contains("v1.len + v3.len <= 256"),
            "push_str capacity expression: {c}"
        );
        assert!(c.contains("v1.len += v3.len;"), "push_str mutation: {c}");
    }

    #[test]
    fn emit_explicit_arena_string_from_cstr_and_push_str() {
        use vow_ir::InstId;
        let func = make_func(
            "cat_in_arena",
            vec![],
            Ty::Ptr,
            vec![
                inst(
                    0,
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(1000),
                ),
                inst(1, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_string_from_cstr_in_arena".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(3, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(1)),
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0), InstId(3)],
                    data: InstData::CallExtern("__vow_string_from_cstr_in_arena".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(5),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(0), InstId(2), InstId(4)],
                    data: InstData::CallExtern("__vow_string_push_str_in_arena".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(6, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("__vow_string_t v2;"), "dest string decl: {c}");
        assert!(c.contains("__vow_string_t v4;"), "src string decl: {c}");
        assert!(
            !c.contains("__vow_string_t v0;"),
            "arena pointer must not be tracked as a string: {c}"
        );
        assert!(
            c.contains("v2.len += v4.len;"),
            "arena push_str must use shifted dest/src args: {c}"
        );
    }

    #[test]
    fn emit_string_byte_at_with_bounds() {
        use vow_ir::InstId;
        let func = make_func(
            "get_byte",
            vec![],
            Ty::I64,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                Inst {
                    id: InstId(1),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![InstId(1), InstId(2)],
                    data: InstData::CallExtern("__vow_string_byte_at".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("__ESBMC_assert(v2 >= 0 && v2 < v1.len"),
            "bounds check: {c}"
        );
        assert!(
            c.contains("v3 = (int64_t)(unsigned char)v1.data[v2]"),
            "byte_at access: {c}"
        );
        assert!(
            c.contains("__ESBMC_assume(v3 >= 0 && v3 <= 255)"),
            "byte_at range postcondition: {c}"
        );
    }

    #[test]
    fn emit_string_eq() {
        use vow_ir::InstId;
        let func = make_func(
            "cmp_str",
            vec![],
            Ty::Bool,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                Inst {
                    id: InstId(1),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(2, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(1)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(2)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::Bool,
                    args: vec![InstId(1), InstId(3)],
                    data: InstData::CallExtern("__vow_string_eq".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(5, Opcode::Return, Ty::Unit, vec![4], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("_Bool __str_eq_1_3 = __VERIFIER_nondet_bool();"),
            "string eq must declare shared per-pair nondet: {c}"
        );
        assert!(
            c.contains("v4 = (v1.len == v3.len) ? __str_eq_1_3 : 0"),
            "string eq abstract model must reference shared nondet: {c}"
        );
    }

    #[test]
    fn emit_string_eq_is_deterministic_per_pair() {
        // Two __vow_string_eq calls on the same (a, b) pair must read the same
        // cached nondet — otherwise ESBMC can pick different values and reject
        // contracts like `ensures: a.eq(b)` after the body established it.
        use vow_ir::InstId;
        let func = make_func(
            "two_compares",
            vec![],
            Ty::Bool,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                Inst {
                    id: InstId(1),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(2, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(1)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(2)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::Bool,
                    args: vec![InstId(1), InstId(3)],
                    data: InstData::CallExtern("__vow_string_eq".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                // Second call with swapped arg order — must hash to the same pair.
                Inst {
                    id: InstId(5),
                    opcode: Opcode::Call,
                    ty: Ty::Bool,
                    args: vec![InstId(3), InstId(1)],
                    data: InstData::CallExtern("__vow_string_eq".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(6, Opcode::Return, Ty::Unit, vec![5], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        // Exactly one shared nondet declaration for the (1, 3) pair.
        let decls = c
            .matches("_Bool __str_eq_1_3 = __VERIFIER_nondet_bool();")
            .count();
        assert_eq!(decls, 1, "expected exactly one shared nondet decl: {c}");
        // Both call sites reference the same cached name.
        assert!(
            c.contains("v4 = (v1.len == v3.len) ? __str_eq_1_3 : 0"),
            "first eq call must use cached nondet: {c}"
        );
        assert!(
            c.contains("v5 = (v3.len == v1.len) ? __str_eq_1_3 : 0"),
            "swapped-order eq call must use the same cached nondet: {c}"
        );
    }

    #[test]
    fn emit_string_eq_invalidates_on_push_byte() {
        // After `__vow_string_push_byte(a, ...)` mutates `a`, every cached
        // pair touching `a` must be re-sampled with __VERIFIER_nondet_bool().
        // The cache itself stays — the pair is still shared by call sites
        // before and after the mutation — but its value can change.
        use vow_ir::InstId;
        let func = make_func(
            "push_then_compare",
            vec![],
            Ty::Bool,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                Inst {
                    id: InstId(1),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(2, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(1)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(2)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::Bool,
                    args: vec![InstId(1), InstId(3)],
                    data: InstData::CallExtern("__vow_string_eq".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(5, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(65)),
                Inst {
                    id: InstId(6),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(1), InstId(5)],
                    data: InstData::CallExtern("__vow_string_push_byte".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(7),
                    opcode: Opcode::Call,
                    ty: Ty::Bool,
                    args: vec![InstId(1), InstId(3)],
                    data: InstData::CallExtern("__vow_string_eq".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(8, Opcode::Return, Ty::Unit, vec![7], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        // Exactly one shared declaration for the (1, 3) pair.
        let decls = c
            .matches("_Bool __str_eq_1_3 = __VERIFIER_nondet_bool();")
            .count();
        assert_eq!(decls, 1, "expected exactly one shared decl: {c}");
        // Both reads share the cached name.
        assert!(
            c.contains("v4 = (v1.len == v3.len) ? __str_eq_1_3 : 0"),
            "first eq call must use cached nondet: {c}"
        );
        assert!(
            c.contains("v7 = (v1.len == v3.len) ? __str_eq_1_3 : 0"),
            "second eq call must reuse cached nondet: {c}"
        );
        // Re-sample line follows the mutation (`v1.len++;`) and precedes the
        // second read.
        let resample = "__str_eq_1_3 = __VERIFIER_nondet_bool();";
        let len_inc_pos = c.find("v1.len++;").expect("push_byte must emit len++");
        let resample_pos = c[len_inc_pos..]
            .find(resample)
            .map(|p| p + len_inc_pos)
            .unwrap_or_else(|| panic!("re-sample must follow v1.len++: {c}"));
        let second_read_pos = c
            .find("v7 = (v1.len == v3.len)")
            .expect("second read must exist");
        assert!(
            resample_pos < second_read_pos,
            "re-sample must precede second read: {c}"
        );
    }

    #[test]
    fn emit_string_eq_invalidates_on_push_str() {
        // `__vow_string_push_str(dest, src)` mutates dest's len; src is
        // read-only. Cached pairs touching dest must be re-sampled; pairs
        // not touching dest must not.
        use vow_ir::InstId;
        let func = make_func(
            "push_str_then_compare",
            vec![],
            Ty::Bool,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                Inst {
                    id: InstId(1),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(2, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(1)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(2)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::Bool,
                    args: vec![InstId(1), InstId(3)],
                    data: InstData::CallExtern("__vow_string_eq".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(5),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(1), InstId(3)],
                    data: InstData::CallExtern("__vow_string_push_str".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(6),
                    opcode: Opcode::Call,
                    ty: Ty::Bool,
                    args: vec![InstId(1), InstId(3)],
                    data: InstData::CallExtern("__vow_string_eq".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(7, Opcode::Return, Ty::Unit, vec![6], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        // Cache shared across both reads.
        let decls = c
            .matches("_Bool __str_eq_1_3 = __VERIFIER_nondet_bool();")
            .count();
        assert_eq!(decls, 1, "expected exactly one shared decl: {c}");
        // Re-sample line follows v1.len += v3.len; (the push_str body) and
        // precedes the second read.
        let push_pos = c
            .find("v1.len += v3.len;")
            .expect("push_str must emit `dest.len += src.len;`");
        let resample = "__str_eq_1_3 = __VERIFIER_nondet_bool();";
        let resample_pos = c[push_pos..]
            .find(resample)
            .map(|p| p + push_pos)
            .unwrap_or_else(|| panic!("re-sample must follow push_str body: {c}"));
        let second_read_pos = c
            .find("v6 = (v1.len == v3.len)")
            .expect("second read must exist");
        assert!(
            resample_pos < second_read_pos,
            "re-sample must precede second read: {c}"
        );
    }

    #[test]
    fn emit_string_clear_emits_len_zero_and_invalidates() {
        // `__vow_string_clear` was previously elided as unmodelled, which
        // both lost the `len = 0` post-condition and skipped cache
        // invalidation. The new model must emit both.
        use vow_ir::InstId;
        let func = make_func(
            "clear_then_compare",
            vec![],
            Ty::Bool,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                Inst {
                    id: InstId(1),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(2, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(1)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(2)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::Bool,
                    args: vec![InstId(1), InstId(3)],
                    data: InstData::CallExtern("__vow_string_eq".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(5),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(1)],
                    data: InstData::CallExtern("__vow_string_clear".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(6),
                    opcode: Opcode::Call,
                    ty: Ty::Bool,
                    args: vec![InstId(1), InstId(3)],
                    data: InstData::CallExtern("__vow_string_eq".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(7, Opcode::Return, Ty::Unit, vec![6], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        // Clear must NOT fall through to the unmodelled handler.
        assert!(
            !c.contains("/* opcode Call not modelled */"),
            "clear must be modeled, not unmodelled: {c}"
        );
        // The new model emits `len = 0`.
        let zero_pos = c
            .find("v1.len = 0;")
            .unwrap_or_else(|| panic!("clear must emit `v1.len = 0;`: {c}"));
        // Re-sample for the (1, 3) pair follows the clear and precedes the
        // second read.
        let resample = "__str_eq_1_3 = __VERIFIER_nondet_bool();";
        let resample_pos = c[zero_pos..]
            .find(resample)
            .map(|p| p + zero_pos)
            .unwrap_or_else(|| panic!("re-sample must follow clear: {c}"));
        let second_read_pos = c
            .find("v6 = (v1.len == v3.len)")
            .expect("second read must exist");
        assert!(
            resample_pos < second_read_pos,
            "re-sample must precede second read: {c}"
        );
    }

    #[test]
    fn emit_string_eq_no_invalidation_for_unrelated_operand() {
        // The cache key is the (lo, hi) operand pair. A mutation on a string
        // that is NOT in any cached pair must emit no re-sample.
        use vow_ir::InstId;
        let func = make_func(
            "mutate_unrelated",
            vec![],
            Ty::Bool,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                Inst {
                    id: InstId(1),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(2, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(1)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(2)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(4, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(2)),
                Inst {
                    id: InstId(5),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(4)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(6),
                    opcode: Opcode::Call,
                    ty: Ty::Bool,
                    args: vec![InstId(1), InstId(3)],
                    data: InstData::CallExtern("__vow_string_eq".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(7, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(65)),
                Inst {
                    id: InstId(8),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(5), InstId(7)],
                    data: InstData::CallExtern("__vow_string_push_byte".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(9, Opcode::Return, Ty::Unit, vec![6], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        // Cached pair is (1, 3); the push_byte is on operand 5.
        // Cache decl is the only place __str_eq_1_3 is assigned; no re-sample.
        let assignments = c.matches("__str_eq_1_3").count();
        // Two occurrences expected: the decl line and the read at v6.
        assert_eq!(
            assignments, 2,
            "no re-sample for unrelated operand (decl + 1 read = 2): {c}"
        );
    }

    #[test]
    fn emit_string_contains() {
        use vow_ir::InstId;
        let func = make_func(
            "has_sub",
            vec![],
            Ty::Bool,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                Inst {
                    id: InstId(1),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(2, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(1)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(2)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::Bool,
                    args: vec![InstId(1), InstId(3)],
                    data: InstData::CallExtern("__vow_string_contains".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(5, Opcode::Return, Ty::Unit, vec![4], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("v4 = 0;"), "contains init: {c}");
        assert!(
            c.contains("v1.data[__i + __j] != v3.data[__j]"),
            "contains byte comparison: {c}"
        );
    }

    #[test]
    fn emit_string_phi_propagation() {
        use vow_ir::InstId;
        let func = Function {
            id: FuncId(0),
            name: "str_phi".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::Ptr,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(0),
                        opcode: Opcode::Phi,
                        ty: Ty::Ptr,
                        args: vec![],
                        data: InstData::None,
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(1, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::Call,
                        ty: Ty::Ptr,
                        args: vec![InstId(1)],
                        data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(3),
                        opcode: Opcode::Upsilon,
                        ty: Ty::Unit,
                        args: vec![InstId(2)],
                        data: InstData::PhiTarget(InstId(0)),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(4, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("__vow_string_t v0;"),
            "phi uses string type: {c}"
        );
    }

    #[test]
    fn emit_string_print_not_modelled() {
        use vow_ir::InstId;
        let func = make_func(
            "print_it",
            vec![],
            Ty::Unit,
            vec![
                inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                Inst {
                    id: InstId(1),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(1)],
                    data: InstData::CallExtern("__vow_string_print".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("string print not modelled"),
            "print not modelled: {c}"
        );
    }

    // --- HashMap unit tests ---

    #[test]
    fn emit_hashmap_new() {
        use vow_ir::InstId;
        let func = make_func(
            "make_map",
            vec![],
            Ty::Ptr,
            vec![
                Inst {
                    id: InstId(0),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![],
                    data: InstData::CallExtern("__vow_map_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("__vow_hashmap_t v0;"), "hashmap decl: {c}");
        assert!(c.contains("v0.len = 0;"), "hashmap len init: {c}");
        assert!(
            c.contains("return 0; /* modelled type return */"),
            "hashmap return: {c}"
        );
    }

    #[test]
    fn emit_hashmap_len() {
        use vow_ir::InstId;
        let func = make_func(
            "get_len",
            vec![],
            Ty::I64,
            vec![
                Inst {
                    id: InstId(0),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![],
                    data: InstData::CallExtern("__vow_map_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(1),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_map_len".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("v1 = v0.len;"), "hashmap len: {c}");
    }

    #[test]
    fn emit_hashmap_insert() {
        use vow_ir::InstId;
        let func = make_func(
            "insert_one",
            vec![],
            Ty::Ptr,
            vec![
                Inst {
                    id: InstId(0),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![],
                    data: InstData::CallExtern("__vow_map_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(10)),
                inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(20)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(0), InstId(1), InstId(2)],
                    data: InstData::CallExtern("__vow_map_insert".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(4, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("v0.keys[__i] == v1"), "key search: {c}");
        assert!(c.contains("v0.vals[__i] = v2"), "update existing: {c}");
        assert!(
            c.contains("hashmap capacity"),
            "insert must have capacity assertion: {c}"
        );
        assert!(c.contains("v0.keys[v0.len] = v1"), "insert new key: {c}");
        assert!(c.contains("v0.vals[v0.len] = v2"), "insert new val: {c}");
        assert!(c.contains("v0.len++"), "insert increments len: {c}");
    }

    #[test]
    fn emit_hashmap_get() {
        use vow_ir::InstId;
        let func = make_func(
            "get_val",
            vec![],
            Ty::I64,
            vec![
                Inst {
                    id: InstId(0),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![],
                    data: InstData::CallExtern("__vow_map_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(5)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_map_get".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("v2 = 0;"), "get default: {c}");
        assert!(c.contains("v0.keys[__i] == v1"), "get key search: {c}");
        assert!(c.contains("v2 = v0.vals[__i]"), "get reads value: {c}");
    }

    #[test]
    fn emit_hashmap_contains_key() {
        use vow_ir::InstId;
        let func = make_func(
            "has_key",
            vec![],
            Ty::Bool,
            vec![
                Inst {
                    id: InstId(0),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![],
                    data: InstData::CallExtern("__vow_map_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(7)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Bool,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_map_contains".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("v2 = 0;"), "contains default: {c}");
        assert!(c.contains("v0.keys[__i] == v1"), "contains key search: {c}");
        assert!(c.contains("v2 = 1"), "contains sets true: {c}");
    }

    #[test]
    fn emit_hashmap_remove() {
        use vow_ir::InstId;
        let func = make_func(
            "remove_key",
            vec![],
            Ty::Ptr,
            vec![
                Inst {
                    id: InstId(0),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![],
                    data: InstData::CallExtern("__vow_map_new".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(3)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_map_remove".to_string()),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(3, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("v0.keys[__i] == v1"), "remove key search: {c}");
        assert!(c.contains("v0.len--"), "remove decrements len: {c}");
    }

    #[test]
    fn emit_hashmap_phi_propagation() {
        use vow_ir::InstId;
        let func = Function {
            id: FuncId(0),
            name: "map_phi".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::Ptr,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(0),
                        opcode: Opcode::Phi,
                        ty: Ty::Ptr,
                        args: vec![],
                        data: InstData::None,
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::Call,
                        ty: Ty::Ptr,
                        args: vec![],
                        data: InstData::CallExtern("__vow_map_new".to_string()),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::Upsilon,
                        ty: Ty::Unit,
                        args: vec![InstId(1)],
                        data: InstData::PhiTarget(InstId(0)),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(3, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("__vow_hashmap_t v0;"),
            "phi uses hashmap type: {c}"
        );
    }

    #[test]
    fn emit_hashmap_module_header() {
        let func = make_func(
            "f",
            vec![],
            Ty::Unit,
            vec![inst(0, Opcode::Return, Ty::Unit, vec![], InstData::None)],
        );
        let c = emit_c_module(&[&func], &HashMap::new(), &VerifyLimits::default());
        assert!(
            c.contains("__vow_hashmap_t"),
            "hashmap typedef in header: {c}"
        );
        assert!(c.contains("int64_t keys[64]"), "keys array in typedef: {c}");
        assert!(c.contains("int64_t vals[64]"), "vals array in typedef: {c}");
    }

    #[test]
    fn emit_block_visit_instrumentation() {
        use vow_ir::InstId;
        let func = Function {
            id: FuncId(0),
            name: "branchy".to_string(),
            params: vec![Ty::Bool],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        inst(0, Opcode::GetArg, Ty::Bool, vec![], InstData::ArgIndex(0)),
                        Inst {
                            id: InstId(1),
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
                        inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(1)),
                        inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                    ],
                },
                BasicBlock {
                    id: BlockId(2),
                    insts: vec![
                        inst(4, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                        inst(5, Opcode::Return, Ty::Unit, vec![4], InstData::None),
                    ],
                },
            ],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("int __blk_0 = 0;"), "blk_0 decl: {c}");
        assert!(c.contains("int __blk_1 = 0;"), "blk_1 decl: {c}");
        assert!(c.contains("int __blk_2 = 0;"), "blk_2 decl: {c}");
        assert!(c.contains("__blk_0 = 1;"), "blk_0 set: {c}");
        assert!(c.contains("__blk_1 = 1;"), "blk_1 set: {c}");
        assert!(c.contains("__blk_2 = 1;"), "blk_2 set: {c}");
    }

    // --- Constant-function detection tests ---

    fn make_constant_func(fid: u32, name: &str, val: i64) -> Function {
        Function {
            id: FuncId(fid),
            name: name.to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(
                        0,
                        Opcode::ConstI64,
                        Ty::I64,
                        vec![],
                        InstData::ConstI64(val),
                    ),
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        }
    }

    #[test]
    fn detect_constant_functions_finds_simple() {
        use vow_ir::Module;
        let module = Module {
            name: "test".to_string(),
            functions: vec![make_constant_func(0, "forty_two", 42)],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        let result = detect_constant_functions(&module);
        assert_eq!(result.len(), 1);
        assert!(result.contains_key(&FuncId(0)));
        assert!(matches!(result[&FuncId(0)], ConstantValue::I64(42)));
    }

    #[test]
    fn detect_constant_functions_skips_multi_block() {
        use vow_ir::Module;
        let func = Function {
            id: FuncId(0),
            name: "multi".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    insts: vec![inst(
                        0,
                        Opcode::ConstI64,
                        Ty::I64,
                        vec![],
                        InstData::ConstI64(1),
                    )],
                },
                BasicBlock {
                    id: BlockId(1),
                    insts: vec![inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None)],
                },
            ],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let module = Module {
            name: "test".to_string(),
            functions: vec![func],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        assert!(detect_constant_functions(&module).is_empty());
    }

    #[test]
    fn detect_constant_functions_skips_non_trivial() {
        use vow_ir::Module;
        let func = Function {
            id: FuncId(0),
            name: "adder".to_string(),
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
                    inst(2, Opcode::WrappingAdd, Ty::I64, vec![0, 1], InstData::None),
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let module = Module {
            name: "test".to_string(),
            functions: vec![func],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        assert!(detect_constant_functions(&module).is_empty());
    }

    #[test]
    fn emit_inlines_constant_call_target() {
        let mut const_fns = HashMap::new();
        const_fns.insert(FuncId(1), ConstantValue::I64(42));

        let call_inst = Inst {
            id: InstId(5),
            opcode: Opcode::Call,
            ty: Ty::I64,
            args: vec![],
            data: InstData::CallTarget(FuncId(1)),
            origin: sp(),
            region: RegionId::Root,
        };
        let empty_module = Module {
            name: String::new(),
            functions: vec![],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        let mut out = String::new();
        emit_inst(
            &call_inst,
            &mut out,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &const_fns,
            &HashSet::new(),
            &HashMap::new(),
            &[],
            &HashMap::new(),
            &empty_module,
            &VerifyLimits::default(),
            Ty::I64,
            FuncId(0),
            false,
        );
        assert!(out.contains("v5 = 42LL;"), "inlined constant: {out}");
    }

    #[test]
    fn emit_falls_back_for_unknown_call_target() {
        let empty_module = Module {
            name: String::new(),
            functions: vec![],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        let call_inst = Inst {
            id: InstId(5),
            opcode: Opcode::Call,
            ty: Ty::I64,
            args: vec![],
            data: InstData::CallTarget(FuncId(99)),
            origin: sp(),
            region: RegionId::Root,
        };
        let mut out = String::new();
        emit_inst(
            &call_inst,
            &mut out,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
            &[],
            &HashMap::new(),
            &empty_module,
            &VerifyLimits::default(),
            Ty::I64,
            FuncId(0),
            false,
        );
        assert!(
            out.contains("__VERIFIER_nondet_long()"),
            "nondet fallback: {out}"
        );
    }

    /// Build a one-block function that indexes a container parameter, marking
    /// the container with `len_extern` so the emitter classifies it, and
    /// indexing it with `idx_ty` via `index_extern`.
    fn indexed_container_fn(len_extern: &str, index_extern: &str, idx_ty: Ty) -> Function {
        use vow_ir::InstId;
        let call = |id: u32, name: &str, args: Vec<u32>, ty: Ty| Inst {
            id: InstId(id),
            opcode: Opcode::Call,
            ty,
            args: args.into_iter().map(InstId).collect(),
            data: InstData::CallExtern(name.to_string()),
            origin: sp(),
            region: RegionId::Root,
        };
        let mut insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            call(1, len_extern, vec![0], Ty::I64),
            inst(2, Opcode::GetArg, idx_ty, vec![], InstData::ArgIndex(1)),
        ];
        if index_extern == "__vow_vec_set_val" {
            insts.push(inst(
                3,
                Opcode::ConstI64,
                Ty::I64,
                vec![],
                InstData::ConstI64(7),
            ));
            insts.push(call(4, index_extern, vec![0, 2, 3], Ty::Unit));
        } else {
            insts.push(call(4, index_extern, vec![0, 2], Ty::I64));
        }
        insts.push(inst(5, Opcode::Return, Ty::Unit, vec![1], InstData::None));
        Function {
            id: FuncId(0),
            name: "indexed".to_string(),
            params: vec![Ty::Ptr, idx_ty],
            param_names: vec!["c".to_string(), "i".to_string()],
            return_ty: Ty::I64,
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

    /// An unsigned index drops the vacuous `>= 0` conjunct and converts the
    /// signed `.len` field explicitly, so the emitted C carries no
    /// mixed-signedness comparison (#1113).
    #[test]
    fn bounds_assert_unsigned_index_is_explicitly_converted() {
        let cases = [
            ("__vow_vec_len", "__vow_vec_get_val", "vec bounds"),
            ("__vow_vec_len", "__vow_vec_set_val", "vec bounds"),
            ("__vow_string_len", "__vow_string_byte_at", "string bounds"),
        ];
        for (len_extern, index_extern, label) in cases {
            for idx_ty in [Ty::U8, Ty::U16, Ty::U32, Ty::U64] {
                let func = indexed_container_fn(len_extern, index_extern, idx_ty);
                let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
                let expected = format!("__ESBMC_assert(v2 < (uint64_t)v0.len, \"{label}\");");
                assert!(
                    c.contains(&expected),
                    "{index_extern} with {idx_ty:?} index must emit `{expected}`: {c}"
                );
                assert!(
                    !c.contains(&format!("v2 >= 0 && v2 < v0.len, \"{label}\"")),
                    "{index_extern} with {idx_ty:?} index must not emit the vacuous \
                     signed form: {c}"
                );
            }
        }
    }

    /// A signed index keeps the two-sided form byte-for-byte (#1113).
    #[test]
    fn bounds_assert_signed_index_is_unchanged() {
        let cases = [
            ("__vow_vec_len", "__vow_vec_get_val", "vec bounds"),
            ("__vow_vec_len", "__vow_vec_set_val", "vec bounds"),
            ("__vow_string_len", "__vow_string_byte_at", "string bounds"),
        ];
        for (len_extern, index_extern, label) in cases {
            for idx_ty in [Ty::I8, Ty::I16, Ty::I32, Ty::I64] {
                let func = indexed_container_fn(len_extern, index_extern, idx_ty);
                let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
                let expected = format!("__ESBMC_assert(v2 >= 0 && v2 < v0.len, \"{label}\");");
                assert!(
                    c.contains(&expected),
                    "{index_extern} with {idx_ty:?} index must emit `{expected}`: {c}"
                );
                assert!(
                    !c.contains("(uint64_t)v0.len"),
                    "{index_extern} with {idx_ty:?} index must not cast .len: {c}"
                );
            }
        }
    }
}
