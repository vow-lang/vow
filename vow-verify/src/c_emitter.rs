use std::collections::{HashMap, HashSet};

use vow_ir::{FuncId, Function, Inst, InstData, Module, Opcode, Ty};

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
// Verification limits for bounded model checking
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct VerifyLimits {
    pub max_k_step: u32,
    pub vec_max: usize,
    pub string_max: usize,
    pub hashmap_max: usize,
}

impl Default for VerifyLimits {
    fn default() -> Self {
        Self {
            max_k_step: 50,
            vec_max: 128,
            string_max: 256,
            hashmap_max: 64,
        }
    }
}

// ---------------------------------------------------------------------------
// Type mapping
// ---------------------------------------------------------------------------

fn ir_ty_to_c(ty: Ty) -> &'static str {
    match ty {
        Ty::I32 => "int32_t",
        Ty::I64 => "int64_t",
        Ty::U64 => "uint64_t",
        Ty::F32 => "float",
        Ty::F64 => "double",
        Ty::Bool => "_Bool",
        Ty::Unit => "int32_t",
        Ty::Ptr | Ty::LinearPtr => "void*",
    }
}

// ---------------------------------------------------------------------------
// Typed variable analysis (Vec, String, HashMap)
// ---------------------------------------------------------------------------

fn collect_typed_vars(func: &Function, creator: &str, prefix: &str) -> HashSet<u32> {
    let mut vars = HashSet::new();

    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Call
                && let InstData::CallExtern(ref name) = inst.data
            {
                let is_alt_creator = (prefix == "__vow_vec_"
                    && (name == "__vow_vec_from_raw_parts_copy_val"
                        || name == "__vow_vec_pin_to_root_val"))
                    || (prefix == "__vow_string_"
                        && (name == "__vow_string_from_raw_parts_copy"
                            || name == "__vow_string_pin_to_root"));
                if name == creator || is_alt_creator {
                    vars.insert(inst.id.0);
                } else if name.starts_with(prefix) && !inst.args.is_empty() {
                    vars.insert(inst.args[0].0);
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
                && (name == "__vow_string_parse_i64_opt" || name == "__vow_string_parse_u64_opt")
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
    matches!(
        name,
        "__vow_vec_new"
            | "__vow_vec_push_val"
            | "__vow_vec_get_val"
            | "__vow_vec_from_raw_parts_copy_val"
            | "__vow_vec_pin_to_root_val"
            | "__vow_vec_len"
            | "__vow_vec_pop"
            | "__vow_vec_set_val"
            | "__vow_string_from_cstr"
            | "__vow_string_from_raw_parts_copy"
            | "__vow_string_pin_to_root"
            | "__vow_string_len"
            | "__vow_string_push_str"
            | "__vow_string_push_byte"
            | "__vow_string_byte_at"
            | "__vow_string_eq"
            | "__vow_string_contains"
            | "__vow_string_substring"
            | "__vow_string_parse_i64_opt"
            | "__vow_string_parse_u64_opt"
            | "__vow_string_print"
            | "__vow_map_new"
            | "__vow_map_len"
            | "__vow_map_insert"
            | "__vow_map_get"
            | "__vow_map_contains"
            | "__vow_map_remove"
    )
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
    if let Some(&cached) = cache.get(&func.id) {
        return cached;
    }
    cache.insert(func.id, false); // prevent infinite recursion

    if !func.effects.is_empty() {
        return false;
    }

    let vec_vars = collect_typed_vars(func, "__vow_vec_new", "__vow_vec_");
    let string_vars = collect_typed_vars(func, "__vow_string_from_cstr", "__vow_string_");
    let hashmap_vars = collect_typed_vars(func, "__vow_map_new", "__vow_map_");
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
                | Opcode::WrappingAddI32
                | Opcode::WrappingAddI64
                | Opcode::CheckedAddI32
                | Opcode::CheckedAddI64
                | Opcode::WrappingSubI32
                | Opcode::WrappingSubI64
                | Opcode::CheckedSubI32
                | Opcode::CheckedSubI64
                | Opcode::WrappingMulI32
                | Opcode::WrappingMulI64
                | Opcode::CheckedMulI32
                | Opcode::CheckedMulI64
                | Opcode::WrappingDivI32
                | Opcode::WrappingDivI64
                | Opcode::CheckedDivI32
                | Opcode::CheckedDivI64
                | Opcode::WrappingRemI32
                | Opcode::WrappingRemI64
                | Opcode::CheckedRemI32
                | Opcode::CheckedRemI64
                | Opcode::AddF32
                | Opcode::AddF64
                | Opcode::SubF32
                | Opcode::SubF64
                | Opcode::MulF32
                | Opcode::MulF64
                | Opcode::DivF32
                | Opcode::DivF64
                | Opcode::EqI32
                | Opcode::EqI64
                | Opcode::EqF32
                | Opcode::EqF64
                | Opcode::NeI32
                | Opcode::NeI64
                | Opcode::NeF32
                | Opcode::NeF64
                | Opcode::LtI32
                | Opcode::LtI64
                | Opcode::LtF32
                | Opcode::LtF64
                | Opcode::LeI32
                | Opcode::LeI64
                | Opcode::LeF32
                | Opcode::LeF64
                | Opcode::GtI32
                | Opcode::GtI64
                | Opcode::GtF32
                | Opcode::GtF64
                | Opcode::GeI32
                | Opcode::GeI64
                | Opcode::GeF32
                | Opcode::GeF64
                | Opcode::Not
                | Opcode::And
                | Opcode::Or
                | Opcode::BitAndI64
                | Opcode::BitOrI64
                | Opcode::XorI32
                | Opcode::XorI64
                | Opcode::ShlI64
                | Opcode::ShrI64
                | Opcode::WrappingAddU64
                | Opcode::WrappingSubU64
                | Opcode::WrappingMulU64
                | Opcode::WrappingDivU64
                | Opcode::WrappingRemU64
                | Opcode::CheckedAddU64
                | Opcode::CheckedSubU64
                | Opcode::CheckedMulU64
                | Opcode::CheckedDivU64
                | Opcode::CheckedRemU64
                | Opcode::EqU64
                | Opcode::NeU64
                | Opcode::LtU64
                | Opcode::LeU64
                | Opcode::GtU64
                | Opcode::GeU64
                | Opcode::BitAndU64
                | Opcode::BitOrU64
                | Opcode::XorU64
                | Opcode::ShlU64
                | Opcode::ShrU64
                | Opcode::ConstU64
                | Opcode::CastI64ToU64
                | Opcode::CastU64ToI64
                | Opcode::VowRequires
                | Opcode::VowEnsures
                | Opcode::VowInvariant
                | Opcode::Branch
                | Opcode::Jump
                | Opcode::Return
                | Opcode::Unreachable
                | Opcode::Phi
                | Opcode::Upsilon => true,

                Opcode::Call => match &inst.data {
                    InstData::CallExtern(name) => is_known_builtin(name),
                    InstData::CallTarget(fid) => {
                        const_fns.contains_key(fid)
                            || module.functions.iter().find(|f| f.id == *fid).is_some_and(
                                |callee| is_modelable(callee, module, const_fns, cache),
                            )
                    }
                    _ => false,
                },

                Opcode::FieldGet => {
                    let iid = inst.id.0;
                    vec_vars.contains(&iid)
                        || string_vars.contains(&iid)
                        || hashmap_vars.contains(&iid)
                        || option_vars.contains(&inst.args.first().map_or(u32::MAX, |a| a.0))
                }

                Opcode::RemF32
                | Opcode::RemF64
                | Opcode::Load
                | Opcode::Store
                | Opcode::RegionAlloc
                | Opcode::RegionOpen
                | Opcode::RegionClose
                | Opcode::LinearConsume
                | Opcode::LinearBorrow
                | Opcode::FieldSet => false,

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
    option_vars: &HashSet<u32>,
    const_fns: &HashMap<FuncId, ConstantValue>,
    modelable_fns: &HashSet<FuncId>,
    module: &Module,
    limits: &VerifyLimits,
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

        // Arithmetic
        Opcode::WrappingAddI32
        | Opcode::WrappingAddI64
        | Opcode::CheckedAddI32
        | Opcode::CheckedAddI64
        | Opcode::WrappingAddU64
        | Opcode::CheckedAddU64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = v{} + v{};\n", id, a, b));
        }
        Opcode::WrappingSubI32
        | Opcode::WrappingSubI64
        | Opcode::CheckedSubI32
        | Opcode::CheckedSubI64
        | Opcode::WrappingSubU64
        | Opcode::CheckedSubU64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = v{} - v{};\n", id, a, b));
        }
        Opcode::WrappingMulI32
        | Opcode::WrappingMulI64
        | Opcode::CheckedMulI32
        | Opcode::CheckedMulI64
        | Opcode::WrappingMulU64
        | Opcode::CheckedMulU64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = v{} * v{};\n", id, a, b));
        }
        Opcode::WrappingDivI32
        | Opcode::WrappingDivI64
        | Opcode::CheckedDivI32
        | Opcode::CheckedDivI64
        | Opcode::WrappingDivU64
        | Opcode::CheckedDivU64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = v{} / v{};\n", id, a, b));
        }
        Opcode::WrappingRemI32
        | Opcode::WrappingRemI64
        | Opcode::CheckedRemI32
        | Opcode::CheckedRemI64
        | Opcode::WrappingRemU64
        | Opcode::CheckedRemU64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = v{} % v{};\n", id, a, b));
        }

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
        Opcode::EqI32 | Opcode::EqI64 | Opcode::EqF32 | Opcode::EqF64 | Opcode::EqU64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} == v{});\n", id, a, b));
        }
        Opcode::NeI32 | Opcode::NeI64 | Opcode::NeF32 | Opcode::NeF64 | Opcode::NeU64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} != v{});\n", id, a, b));
        }
        Opcode::LtI32 | Opcode::LtI64 | Opcode::LtF32 | Opcode::LtF64 | Opcode::LtU64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} < v{});\n", id, a, b));
        }
        Opcode::LeI32 | Opcode::LeI64 | Opcode::LeF32 | Opcode::LeF64 | Opcode::LeU64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} <= v{});\n", id, a, b));
        }
        Opcode::GtI32 | Opcode::GtI64 | Opcode::GtF32 | Opcode::GtF64 | Opcode::GtU64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} > v{});\n", id, a, b));
        }
        Opcode::GeI32 | Opcode::GeI64 | Opcode::GeF32 | Opcode::GeF64 | Opcode::GeU64 => {
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
        Opcode::BitAndI64 | Opcode::BitAndU64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} & v{});\n", id, a, b));
        }
        Opcode::BitOrI64 | Opcode::BitOrU64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} | v{});\n", id, a, b));
        }
        Opcode::XorI32 | Opcode::XorI64 | Opcode::XorU64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = (v{} ^ v{});\n", id, a, b));
        }
        Opcode::ShlI64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = __vow_shl_i64(v{}, v{});\n", id, a, b));
        }
        Opcode::ShrI64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = __vow_shr_i64(v{}, v{});\n", id, a, b));
        }
        Opcode::ShlU64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = __vow_shl_u64(v{}, v{});\n", id, a, b));
        }
        Opcode::ShrU64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  v{} = __vow_shr_u64(v{}, v{});\n", id, a, b));
        }

        Opcode::ConstU64 => {
            if let InstData::ConstU64(v) = inst.data {
                out.push_str(&format!("  v{} = {}ULL;\n", id, v));
            }
        }

        Opcode::CastI64ToU64 => {
            let a = inst.args[0].0;
            out.push_str(&format!("  v{} = (uint64_t)v{};\n", id, a));
        }
        Opcode::CastU64ToI64 => {
            let a = inst.args[0].0;
            out.push_str(&format!("  v{} = (int64_t)v{};\n", id, a));
        }

        // Vow checks → ESBMC intrinsics
        Opcode::VowRequires => {
            let pred = inst.args[0].0;
            out.push_str(&format!("  __ESBMC_assume(v{});\n", pred));
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
            if let Some(&val_id) = inst.args.first() {
                if vec_vars.contains(&val_id.0)
                    || string_vars.contains(&val_id.0)
                    || hashmap_vars.contains(&val_id.0)
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

        // Phi — already pre-declared at function top; nothing to emit here
        Opcode::Phi => {}
        Opcode::Upsilon => {
            if let InstData::PhiTarget(phi_id) = inst.data {
                let val = inst.args[0].0;
                out.push_str(&format!("  v{} = v{};\n", phi_id.0, val));
            }
        }

        // Vec operations — modeled as abstract struct with len + data array
        Opcode::Call if matches!(&inst.data, InstData::CallExtern(n) if n.starts_with("__vow_vec_")) => {
            if let InstData::CallExtern(ref name) = inst.data {
                match name.as_str() {
                    "__vow_vec_new" => {
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
                    "__vow_vec_push_val" => {
                        let vec = inst.args[0].0;
                        let val = inst.args[1].0;
                        let vec_max = limits.vec_max;
                        out.push_str(&format!(
                            "  __ESBMC_assert(v{vec}.len < {vec_max}, \"vec capacity\");\n\
                             \x20 v{vec}.data[v{vec}.len] = v{val};\n  v{vec}.len++;\n",
                        ));
                    }
                    "__vow_vec_get_val" => {
                        let vec = inst.args[0].0;
                        let idx = inst.args[1].0;
                        out.push_str(&format!(
                            "  __ESBMC_assert(v{idx} >= 0 && v{idx} < v{vec}.len, \"vec bounds\");\n\
                             \x20 v{id} = v{vec}.data[v{idx}];\n"
                        ));
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
                        out.push_str(&format!(
                            "  __ESBMC_assert(v{idx} >= 0 && v{idx} < v{vec}.len, \"vec bounds\");\n\
                             \x20 v{vec}.data[v{idx}] = v{val};\n"
                        ));
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
                    "__vow_string_from_cstr" => {
                        let string_max = limits.string_max;
                        out.push_str(&format!(
                            "  v{id}.len = __VERIFIER_nondet_long();\n\
                             \x20 __ESBMC_assume(v{id}.len >= 0 && v{id}.len < {string_max});\n",
                        ));
                    }
                    "__vow_string_from_raw_parts_copy" => {
                        let len = inst.args[1].0;
                        let string_max = limits.string_max;
                        out.push_str(&format!(
                            "  __ESBMC_assume(v{len} >= 0 && v{len} < {string_max});\n  v{id}.len = v{len};\n"
                        ));
                    }
                    "__vow_string_pin_to_root" => {
                        let source = inst.args[0].0;
                        out.push_str(&format!("  v{id} = v{source};\n"));
                    }
                    "__vow_string_len" => {
                        let s = inst.args[0].0;
                        out.push_str(&format!("  v{id} = v{s}.len;\n"));
                    }
                    "__vow_string_push_str" => {
                        let dest = inst.args[0].0;
                        let src = inst.args[1].0;
                        out.push_str(&format!("  v{dest}.len += v{src}.len;\n"));
                    }
                    "__vow_string_push_byte" => {
                        let s = inst.args[0].0;
                        let byte = inst.args[1].0;
                        let string_max = limits.string_max;
                        out.push_str(&format!(
                            "  __ESBMC_assert(v{s}.len < {string_max}, \"string capacity\");\n\
                             \x20 v{s}.data[v{s}.len] = (int8_t)v{byte};\n  v{s}.len++;\n",
                        ));
                    }
                    "__vow_string_byte_at" => {
                        let s = inst.args[0].0;
                        let idx = inst.args[1].0;
                        out.push_str(&format!(
                            "  __ESBMC_assert(v{idx} >= 0 && v{idx} < v{s}.len, \"string bounds\");\n\
                             \x20 v{id} = (int64_t)(unsigned char)v{s}.data[v{idx}];\n\
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
                    "__vow_string_substring" => {
                        let s = inst.args[0].0;
                        let start = inst.args[1].0;
                        let end = inst.args[2].0;
                        let string_max = limits.string_max;
                        out.push_str(&format!(
                            "  __ESBMC_assert(v{start} >= 0 && v{start} <= v{s}.len, \"substring start\");\n\
                             \x20 __ESBMC_assert(v{end} >= v{start} && v{end} <= v{s}.len, \"substring end\");\n\
                             \x20 v{id}.len = v{end} - v{start};\n\
                             \x20 for (int64_t __i = 0; __i < v{id}.len && __i < {string_max}; __i++) {{\n\
                             \x20   v{id}.data[__i] = v{s}.data[v{start} + __i];\n\
                             \x20 }}\n",
                        ));
                    }
                    "__vow_string_parse_i64_opt" | "__vow_string_parse_u64_opt" => {
                        out.push_str(&format!(
                            "  v{id}.tag = __VERIFIER_nondet_long();\n\
                             \x20 __ESBMC_assume(v{id}.tag == 0 || v{id}.tag == 1);\n\
                             \x20 if (v{id}.tag == 1) {{ v{id}.payload = __VERIFIER_nondet_long(); }}\n"
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
        Opcode::Call if matches!(&inst.data, InstData::CallExtern(n) if n.starts_with("__vow_map_")) => {
            if let InstData::CallExtern(ref name) = inst.data {
                match name.as_str() {
                    "__vow_map_new" => {
                        out.push_str(&format!("  v{id}.len = 0;\n"));
                    }
                    "__vow_map_len" => {
                        let m = inst.args[0].0;
                        out.push_str(&format!("  v{id} = v{m}.len;\n"));
                    }
                    "__vow_map_insert" => {
                        let m = inst.args[0].0;
                        let k = inst.args[1].0;
                        let v = inst.args[2].0;
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
                        let m = inst.args[0].0;
                        let k = inst.args[1].0;
                        out.push_str(&format!(
                            "  v{id} = 0;\n\
                             \x20 for (int64_t __i = 0; __i < v{m}.len; __i++) {{\n\
                             \x20   if (v{m}.keys[__i] == v{k}) {{ v{id} = v{m}.vals[__i]; break; }}\n\
                             \x20 }}\n"
                        ));
                    }
                    "__vow_map_contains" => {
                        let m = inst.args[0].0;
                        let k = inst.args[1].0;
                        out.push_str(&format!(
                            "  v{id} = 0;\n\
                             \x20 for (int64_t __i = 0; __i < v{m}.len; __i++) {{\n\
                             \x20   if (v{m}.keys[__i] == v{k}) {{ v{id} = 1; break; }}\n\
                             \x20 }}\n"
                        ));
                    }
                    "__vow_map_remove" => {
                        let m = inst.args[0].0;
                        let k = inst.args[1].0;
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
                        callee.name,
                        args_str.join(", ")
                    ));
                } else {
                    out.push_str(&format!("  {}({});\n", callee.name, args_str.join(", ")));
                }
            }
        }

        // Other calls, memory, region/linear/field ops — not yet supported for verification
        Opcode::Call
        | Opcode::Load
        | Opcode::Store
        | Opcode::RegionAlloc
        | Opcode::RegionOpen
        | Opcode::RegionClose
        | Opcode::LinearConsume
        | Opcode::LinearBorrow
        | Opcode::FieldSet => {
            emit_unsupported_for_verification(inst, out);
        }
        Opcode::FieldGet => {
            if vec_vars.contains(&id) {
                out.push_str(&format!("  /* FieldGet -> vec */ v{}.len = 0;\n", id));
            } else if string_vars.contains(&id) {
                out.push_str(&format!("  /* FieldGet -> string */ v{}.len = 0;\n", id));
            } else if hashmap_vars.contains(&id) {
                out.push_str(&format!("  /* FieldGet -> hashmap */ v{}.len = 0;\n", id));
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

fn emit_unsupported_for_verification(inst: &Inst, out: &mut String) {
    let id = inst.id.0;
    out.push_str(&format!(
        "  __ESBMC_assert(0, \"unsupported opcode in verifier model: {:?}\");\n",
        inst.opcode
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
        Ty::I32 => "int",
        Ty::I64 => "long",
        Ty::U64 => "unsigned_long",
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
    )
}

pub fn emit_c_function_full(
    func: &Function,
    const_fns: &HashMap<FuncId, ConstantValue>,
    modelable_fns: &HashSet<FuncId>,
    module: &Module,
    limits: &VerifyLimits,
) -> String {
    let mut out = String::new();
    let vec_vars = collect_typed_vars(func, "__vow_vec_new", "__vow_vec_");
    let string_vars = collect_typed_vars(func, "__vow_string_from_cstr", "__vow_string_");
    let hashmap_vars = collect_typed_vars(func, "__vow_map_new", "__vow_map_");
    let option_vars = collect_option_vars(func);

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

    out.push_str(&format!("{} {}({}) {{\n", ret_c, func.name, param_str));

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
                    } else if option_vars.contains(&id) {
                        out.push_str(&format!("  __vow_option_t v{};\n  v{}.tag = 0;\n", id, id));
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
            out.push_str(&format!("  int64_t __ups_{};\n", src));
        }
    }

    // Per-pair nondet cache for abstract __vow_string_eq. A fresh
    // __VERIFIER_nondet_bool() on every call would let ESBMC pick different
    // values for the same (a,b) pair, breaking determinism (e.g. body proves
    // `a.eq(b)` then `ensures: a.eq(b)` fails). Declare one shared bool per
    // unordered pair (min, max) and reuse it at every call site.
    {
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
        eq_pairs.sort();
        for (lo, hi) in eq_pairs {
            out.push_str(&format!(
                "  _Bool __str_eq_{lo}_{hi} = __VERIFIER_nondet_bool();\n"
            ));
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
                &option_vars,
                const_fns,
                modelable_fns,
                module,
                limits,
            );
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
                &option_vars,
                const_fns,
                modelable_fns,
                module,
                limits,
            );
        }
    }

    out.push_str("}\n");
    out
}

#[derive(Default)]
struct ShiftNeeds {
    shl_i64: bool,
    shr_i64: bool,
    shl_u64: bool,
    shr_u64: bool,
}

fn scan_shift_needs(funcs: &[&Function]) -> ShiftNeeds {
    let mut needs = ShiftNeeds::default();
    for func in funcs {
        for block in &func.blocks {
            for inst in &block.insts {
                match inst.opcode {
                    Opcode::ShlI64 => needs.shl_i64 = true,
                    Opcode::ShrI64 => needs.shr_i64 = true,
                    Opcode::ShlU64 => needs.shl_u64 = true,
                    Opcode::ShrU64 => needs.shr_u64 = true,
                    _ => {}
                }
            }
        }
    }
    needs
}

fn emit_c_preamble(out: &mut String, shifts: &ShiftNeeds, limits: &VerifyLimits) {
    out.push_str("#include <stdint.h>\n");
    out.push_str("#include <stdlib.h>\n");
    out.push_str("#include <stdbool.h>\n");
    out.push_str("extern void __ESBMC_assume(_Bool);\n");
    out.push_str("extern void __ESBMC_assert(_Bool, const char*);\n");
    out.push_str("extern int __VERIFIER_nondet_int(void);\n");
    out.push_str("extern long __VERIFIER_nondet_long(void);\n");
    out.push_str("extern float __VERIFIER_nondet_float(void);\n");
    out.push_str("extern double __VERIFIER_nondet_double(void);\n");
    out.push_str("extern _Bool __VERIFIER_nondet_bool(void);\n\n");
    let vec_max = limits.vec_max;
    let string_max = limits.string_max;
    let hashmap_max = limits.hashmap_max;
    out.push_str(&format!(
        "typedef struct {{ int64_t len; int64_t data[{vec_max}]; }} __vow_vec_t;\n",
    ));
    out.push_str(&format!(
        "typedef struct {{ int64_t len; int8_t data[{string_max}]; }} __vow_string_t;\n",
    ));
    out.push_str(&format!(
        "typedef struct {{ int64_t len; int64_t keys[{hashmap_max}]; int64_t vals[{hashmap_max}]; }} __vow_hashmap_t;\n",
    ));
    out.push_str("typedef struct { int64_t tag; int64_t payload; } __vow_option_t;\n");
    if shifts.shl_i64 {
        out.push_str(
            "static inline int64_t __vow_shl_i64(int64_t value, int64_t count) {\n\
             \x20 uint64_t shift = ((uint64_t)count) & 63ULL;\n\
             \x20 return (int64_t)(((uint64_t)value) << shift);\n\
             }\n",
        );
    }
    if shifts.shr_i64 {
        out.push_str(
            "static inline int64_t __vow_shr_i64(int64_t value, int64_t count) {\n\
             \x20 uint64_t shift = ((uint64_t)count) & 63ULL;\n\
             \x20 uint64_t bits = (uint64_t)value;\n\
             \x20 uint64_t logical = bits >> shift;\n\
             \x20 uint64_t sign_fill = value < 0 ? ~(~0ULL >> shift) : 0ULL;\n\
             \x20 return (int64_t)(logical | sign_fill);\n\
             }\n",
        );
    }
    if shifts.shl_u64 {
        out.push_str(
            "static inline uint64_t __vow_shl_u64(uint64_t value, uint64_t count) {\n\
             \x20 return value << (count & 63ULL);\n\
             }\n",
        );
    }
    if shifts.shr_u64 {
        out.push_str(
            "static inline uint64_t __vow_shr_u64(uint64_t value, uint64_t count) {\n\
             \x20 return value >> (count & 63ULL);\n\
             }\n",
        );
    }
    if shifts.shl_i64 || shifts.shr_i64 || shifts.shl_u64 || shifts.shr_u64 {
        out.push('\n');
    }
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
    out.push_str(&format!("{} {}({});\n", ret_c, func.name, param_str));
}

pub fn emit_c_module(
    funcs: &[&Function],
    const_fns: &HashMap<FuncId, ConstantValue>,
    limits: &VerifyLimits,
) -> String {
    let mut out = String::new();
    let shifts = scan_shift_needs(funcs);
    emit_c_preamble(&mut out, &shifts, limits);
    for func in funcs {
        out.push_str(&emit_c_function(func, const_fns, limits));
        out.push('\n');
    }
    out
}

/// Emit C code for a target function and its modelable callees.
/// Callee functions are emitted in topological order (callees first).
pub fn emit_c_module_with_callees(
    target: &Function,
    module: &Module,
    const_fns: &HashMap<FuncId, ConstantValue>,
    callee_ids: &[FuncId],
    modelable_fns: &HashSet<FuncId>,
    limits: &VerifyLimits,
) -> String {
    let mut out = String::new();

    // Collect all functions (target + callees) for shift scanning
    let mut all_funcs: Vec<&Function> = vec![target];
    for fid in callee_ids {
        if let Some(callee) = module.functions.iter().find(|f| f.id == *fid) {
            all_funcs.push(callee);
        }
    }
    let shifts = scan_shift_needs(&all_funcs);
    emit_c_preamble(&mut out, &shifts, limits);

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
                limits,
            ));
            out.push('\n');
        }
    }

    // Target function
    out.push_str(&emit_c_function_full(
        target,
        const_fns,
        modelable_fns,
        module,
        limits,
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
        };
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("int64_t add("), "signature: {c}");
        assert!(c.contains("v2 = v0 + v1"), "add: {c}");
        assert!(c.contains("return v2"), "return: {c}");
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
                    inst(3, Opcode::NeI64, Ty::Bool, vec![1, 2], InstData::None),
                    Inst {
                        id: InstId(4),
                        opcode: Opcode::VowRequires,
                        ty: Ty::Unit,
                        args: vec![InstId(3)],
                        data: InstData::VowId(VowId(0)),
                        origin: sp(),
                        region: RegionId::Root,
                    },
                    inst(
                        5,
                        Opcode::WrappingDivI64,
                        Ty::I64,
                        vec![0, 1],
                        InstData::None,
                    ),
                    inst(6, Opcode::Return, Ty::Unit, vec![5], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
        };
        let c = emit_c_function(&func, &HashMap::new(), &VerifyLimits::default());
        assert!(c.contains("__ESBMC_assume(v3)"), "requires: {c}");
        assert!(!c.contains("__ESBMC_assert"), "no assert for requires: {c}");
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
        }
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
                inst(7, Opcode::Return, Ty::Unit, vec![], InstData::None),
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
                inst(
                    6,
                    Opcode::WrappingAddI32,
                    Ty::I32,
                    vec![0, 1],
                    InstData::None,
                ),
                inst(
                    7,
                    Opcode::WrappingSubI32,
                    Ty::I32,
                    vec![0, 1],
                    InstData::None,
                ),
                inst(
                    8,
                    Opcode::WrappingMulI32,
                    Ty::I32,
                    vec![0, 1],
                    InstData::None,
                ),
                inst(
                    9,
                    Opcode::WrappingDivI32,
                    Ty::I32,
                    vec![0, 1],
                    InstData::None,
                ),
                inst(
                    10,
                    Opcode::WrappingRemI32,
                    Ty::I32,
                    vec![0, 1],
                    InstData::None,
                ),
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
                inst(2, Opcode::EqI64, Ty::Bool, vec![0, 1], InstData::None),
                inst(3, Opcode::NeI64, Ty::Bool, vec![0, 1], InstData::None),
                inst(4, Opcode::LtI64, Ty::Bool, vec![0, 1], InstData::None),
                inst(5, Opcode::LeI64, Ty::Bool, vec![0, 1], InstData::None),
                inst(6, Opcode::GtI64, Ty::Bool, vec![0, 1], InstData::None),
                inst(7, Opcode::GeI64, Ty::Bool, vec![0, 1], InstData::None),
                inst(8, Opcode::EqI32, Ty::Bool, vec![0, 1], InstData::None),
                inst(9, Opcode::NeI32, Ty::Bool, vec![0, 1], InstData::None),
                inst(10, Opcode::LtI32, Ty::Bool, vec![0, 1], InstData::None),
                inst(11, Opcode::LeI32, Ty::Bool, vec![0, 1], InstData::None),
                inst(12, Opcode::GtI32, Ty::Bool, vec![0, 1], InstData::None),
                inst(13, Opcode::GeI32, Ty::Bool, vec![0, 1], InstData::None),
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
                inst(2, Opcode::BitAndI64, Ty::I64, vec![0, 1], InstData::None),
                inst(3, Opcode::BitOrI64, Ty::I64, vec![0, 1], InstData::None),
                inst(4, Opcode::XorI64, Ty::I64, vec![0, 1], InstData::None),
                inst(5, Opcode::ShlI64, Ty::I64, vec![0, 1], InstData::None),
                inst(6, Opcode::ShrI64, Ty::I64, vec![0, 1], InstData::None),
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
                inst(2, Opcode::ShlI64, Ty::I64, vec![0, 1], InstData::None),
                inst(3, Opcode::ShrU64, Ty::U64, vec![0, 1], InstData::None),
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
        assert!(c.contains("return 0;"), "void return: {c}");
    }

    #[test]
    fn emit_c_module_wraps_multiple_functions() {
        let f1 = make_func(
            "f1",
            vec![],
            Ty::Unit,
            vec![inst(0, Opcode::Return, Ty::Unit, vec![], InstData::None)],
        );
        let f2 = make_func(
            "f2",
            vec![Ty::I64],
            Ty::I64,
            vec![
                inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let out = emit_c_module(&[&f1, &f2], &HashMap::new(), &VerifyLimits::default());
        assert!(out.contains("#include <stdint.h>"), "includes: {out}");
        assert!(out.contains("__ESBMC_assume"), "esbmc assume: {out}");
        assert!(out.contains("void f1(void)"), "f1 signature: {out}");
        assert!(out.contains("f2("), "f2 signature: {out}");
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
        assert!(c.contains("v1.len += v3.len;"), "push_str: {c}");
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
            &const_fns,
            &HashSet::new(),
            &empty_module,
            &VerifyLimits::default(),
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
            &HashMap::new(),
            &HashSet::new(),
            &empty_module,
            &VerifyLimits::default(),
        );
        assert!(
            out.contains("__VERIFIER_nondet_long()"),
            "nondet fallback: {out}"
        );
    }
}
