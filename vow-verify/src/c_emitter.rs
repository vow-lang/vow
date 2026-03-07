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
// Vec model capacity for bounded model checking
// ---------------------------------------------------------------------------

const VOW_VEC_MAX: usize = 128;
const VOW_STRING_MAX: usize = 256;
const VOW_HASHMAP_MAX: usize = 64;

// ---------------------------------------------------------------------------
// Type mapping
// ---------------------------------------------------------------------------

fn ir_ty_to_c(ty: Ty) -> &'static str {
    match ty {
        Ty::I32 => "int32_t",
        Ty::I64 => "int64_t",
        Ty::F32 => "float",
        Ty::F64 => "double",
        Ty::Bool => "_Bool",
        Ty::Unit => "int32_t", // treated as void internally, but needs a type for vars
        Ty::Ptr | Ty::LinearPtr => "void*",
    }
}

// ---------------------------------------------------------------------------
// Vec variable analysis
// ---------------------------------------------------------------------------

fn collect_vec_vars(func: &Function) -> HashSet<u32> {
    let mut vec_vars = HashSet::new();

    // Pass 1: collect all __vow_vec_new results
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Call
                && let InstData::CallExtern(ref name) = inst.data
                && name == "__vow_vec_new"
            {
                vec_vars.insert(inst.id.0);
            }
        }
    }

    // Pass 2: reverse-propagate — if a value is used as first arg of __vow_vec_*, it's a vec
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Call
                && let InstData::CallExtern(ref name) = inst.data
                && name.starts_with("__vow_vec_")
                && name != "__vow_vec_new"
                && !inst.args.is_empty()
            {
                vec_vars.insert(inst.args[0].0);
            }
        }
    }

    // Pass 3: propagate through Upsilon→Phi until fixed point
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.insts {
                if inst.opcode == Opcode::Upsilon
                    && let InstData::PhiTarget(phi_id) = inst.data
                    && !inst.args.is_empty()
                {
                    // Forward: Upsilon value → Phi target
                    if vec_vars.contains(&inst.args[0].0) && vec_vars.insert(phi_id.0) {
                        changed = true;
                    }
                    // Reverse: Phi target → Upsilon value
                    if vec_vars.contains(&phi_id.0) && vec_vars.insert(inst.args[0].0) {
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    vec_vars
}

// ---------------------------------------------------------------------------
// String variable analysis
// ---------------------------------------------------------------------------

fn collect_string_vars(func: &Function) -> HashSet<u32> {
    let mut string_vars = HashSet::new();

    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Call
                && let InstData::CallExtern(ref name) = inst.data
                && name == "__vow_string_from_cstr"
            {
                string_vars.insert(inst.id.0);
            }
        }
    }

    // Reverse-propagate: first arg of __vow_string_* calls is a string
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Call
                && let InstData::CallExtern(ref name) = inst.data
                && name.starts_with("__vow_string_")
                && name != "__vow_string_from_cstr"
                && !inst.args.is_empty()
            {
                string_vars.insert(inst.args[0].0);
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
                    // Forward: Upsilon value → Phi target
                    if string_vars.contains(&inst.args[0].0) && string_vars.insert(phi_id.0) {
                        changed = true;
                    }
                    // Reverse: Phi target → Upsilon value
                    if string_vars.contains(&phi_id.0) && string_vars.insert(inst.args[0].0) {
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    string_vars
}

// ---------------------------------------------------------------------------
// HashMap variable analysis
// ---------------------------------------------------------------------------

fn collect_hashmap_vars(func: &Function) -> HashSet<u32> {
    let mut hashmap_vars = HashSet::new();

    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Call
                && let InstData::CallExtern(ref name) = inst.data
                && name == "__vow_map_new"
            {
                hashmap_vars.insert(inst.id.0);
            }
        }
    }

    // Reverse-propagate: first arg of __vow_map_* calls is a hashmap
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Call
                && let InstData::CallExtern(ref name) = inst.data
                && name.starts_with("__vow_map_")
                && name != "__vow_map_new"
                && !inst.args.is_empty()
            {
                hashmap_vars.insert(inst.args[0].0);
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
                    // Forward: Upsilon value → Phi target
                    if hashmap_vars.contains(&inst.args[0].0) && hashmap_vars.insert(phi_id.0) {
                        changed = true;
                    }
                    // Reverse: Phi target → Upsilon value
                    if hashmap_vars.contains(&phi_id.0) && hashmap_vars.insert(inst.args[0].0) {
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    hashmap_vars
}

// ---------------------------------------------------------------------------
// Expression / statement emission
// ---------------------------------------------------------------------------

fn emit_inst(
    inst: &Inst,
    out: &mut String,
    vec_vars: &HashSet<u32>,
    string_vars: &HashSet<u32>,
    hashmap_vars: &HashSet<u32>,
    const_fns: &HashMap<FuncId, ConstantValue>,
) {
    let id = inst.id.0;
    match inst.opcode {
        // Constants
        Opcode::ConstI32 => {
            if let InstData::ConstI32(v) = inst.data {
                out.push_str(&format!("  {} v{} = {};\n", ir_ty_to_c(inst.ty), id, v));
            }
        }
        Opcode::ConstI64 => {
            if let InstData::ConstI64(v) = inst.data {
                out.push_str(&format!("  {} v{} = {}LL;\n", ir_ty_to_c(inst.ty), id, v));
            }
        }
        Opcode::ConstF32 => {
            if let InstData::ConstF32(v) = inst.data {
                out.push_str(&format!("  float v{} = {}f;\n", id, v));
            }
        }
        Opcode::ConstF64 => {
            if let InstData::ConstF64(v) = inst.data {
                out.push_str(&format!("  double v{} = {};\n", id, v));
            }
        }
        Opcode::ConstBool => {
            let b = matches!(inst.data, InstData::ConstBool(true));
            out.push_str(&format!("  _Bool v{} = {};\n", id, b as i32));
        }
        Opcode::ConstUnit => {
            out.push_str(&format!("  int32_t v{} = 0;\n", id));
        }
        Opcode::ConstStr => {
            out.push_str(&format!("  void* v{} = 0; /* string not modelled */\n", id));
        }

        // Arguments — emitted as parameter names at function top
        Opcode::GetArg => {}

        // Arithmetic
        Opcode::WrappingAddI32
        | Opcode::WrappingAddI64
        | Opcode::CheckedAddI32
        | Opcode::CheckedAddI64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!(
                "  {} v{} = v{} + v{};\n",
                ir_ty_to_c(inst.ty),
                id,
                a,
                b
            ));
        }
        Opcode::WrappingSubI32
        | Opcode::WrappingSubI64
        | Opcode::CheckedSubI32
        | Opcode::CheckedSubI64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!(
                "  {} v{} = v{} - v{};\n",
                ir_ty_to_c(inst.ty),
                id,
                a,
                b
            ));
        }
        Opcode::WrappingMulI32
        | Opcode::WrappingMulI64
        | Opcode::CheckedMulI32
        | Opcode::CheckedMulI64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!(
                "  {} v{} = v{} * v{};\n",
                ir_ty_to_c(inst.ty),
                id,
                a,
                b
            ));
        }
        Opcode::WrappingDivI32
        | Opcode::WrappingDivI64
        | Opcode::CheckedDivI32
        | Opcode::CheckedDivI64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!(
                "  {} v{} = v{} / v{};\n",
                ir_ty_to_c(inst.ty),
                id,
                a,
                b
            ));
        }
        Opcode::WrappingRemI32
        | Opcode::WrappingRemI64
        | Opcode::CheckedRemI32
        | Opcode::CheckedRemI64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!(
                "  {} v{} = v{} % v{};\n",
                ir_ty_to_c(inst.ty),
                id,
                a,
                b
            ));
        }

        // Float arithmetic
        Opcode::AddF32 | Opcode::AddF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!(
                "  {} v{} = v{} + v{};\n",
                ir_ty_to_c(inst.ty),
                id,
                a,
                b
            ));
        }
        Opcode::SubF32 | Opcode::SubF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!(
                "  {} v{} = v{} - v{};\n",
                ir_ty_to_c(inst.ty),
                id,
                a,
                b
            ));
        }
        Opcode::MulF32 | Opcode::MulF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!(
                "  {} v{} = v{} * v{};\n",
                ir_ty_to_c(inst.ty),
                id,
                a,
                b
            ));
        }
        Opcode::DivF32 | Opcode::DivF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!(
                "  {} v{} = v{} / v{};\n",
                ir_ty_to_c(inst.ty),
                id,
                a,
                b
            ));
        }

        // Integer comparisons
        Opcode::EqI32 | Opcode::EqI64 | Opcode::EqF32 | Opcode::EqF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  _Bool v{} = (v{} == v{});\n", id, a, b));
        }
        Opcode::NeI32 | Opcode::NeI64 | Opcode::NeF32 | Opcode::NeF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  _Bool v{} = (v{} != v{});\n", id, a, b));
        }
        Opcode::LtI32 | Opcode::LtI64 | Opcode::LtF32 | Opcode::LtF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  _Bool v{} = (v{} < v{});\n", id, a, b));
        }
        Opcode::LeI32 | Opcode::LeI64 | Opcode::LeF32 | Opcode::LeF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  _Bool v{} = (v{} <= v{});\n", id, a, b));
        }
        Opcode::GtI32 | Opcode::GtI64 | Opcode::GtF32 | Opcode::GtF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  _Bool v{} = (v{} > v{});\n", id, a, b));
        }
        Opcode::GeI32 | Opcode::GeI64 | Opcode::GeF32 | Opcode::GeF64 => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  _Bool v{} = (v{} >= v{});\n", id, a, b));
        }

        // Boolean ops
        Opcode::Not => {
            let a = inst.args[0].0;
            out.push_str(&format!("  _Bool v{} = !v{};\n", id, a));
        }
        Opcode::And => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  _Bool v{} = (v{} && v{});\n", id, a, b));
        }
        Opcode::Or => {
            let (a, b) = (inst.args[0].0, inst.args[1].0);
            out.push_str(&format!("  _Bool v{} = (v{} || v{});\n", id, a, b));
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
                        out.push_str(&format!("  __vow_vec_t v{};\n  v{}.len = 0;\n", id, id));
                    }
                    "__vow_vec_push_val" => {
                        let vec = inst.args[0].0;
                        let val = inst.args[1].0;
                        out.push_str(&format!(
                            "  __ESBMC_assert(v{vec}.len < {}, \"vec capacity\");\n\
                             \x20 v{vec}.data[v{vec}.len] = v{val};\n  v{vec}.len++;\n",
                            VOW_VEC_MAX
                        ));
                    }
                    "__vow_vec_get_val" => {
                        let vec = inst.args[0].0;
                        let idx = inst.args[1].0;
                        out.push_str(&format!(
                            "  __ESBMC_assert(v{idx} >= 0 && v{idx} < v{vec}.len, \"vec bounds\");\n\
                             \x20 int64_t v{id} = v{vec}.data[v{idx}];\n"
                        ));
                    }
                    "__vow_vec_len" => {
                        let vec = inst.args[0].0;
                        out.push_str(&format!("  int64_t v{id} = v{vec}.len;\n"));
                    }
                    "__vow_vec_pop" => {
                        let vec = inst.args[0].0;
                        out.push_str(&format!("  v{vec}.len--;\n"));
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
                        out.push_str(&format!(
                            "  __vow_string_t v{id};\n\
                             \x20 v{id}.len = __VERIFIER_nondet_long();\n\
                             \x20 __ESBMC_assume(v{id}.len >= 0 && v{id}.len < {});\n",
                            VOW_STRING_MAX
                        ));
                    }
                    "__vow_string_len" => {
                        let s = inst.args[0].0;
                        out.push_str(&format!("  int64_t v{id} = v{s}.len;\n"));
                    }
                    "__vow_string_push_str" => {
                        let dest = inst.args[0].0;
                        let src = inst.args[1].0;
                        out.push_str(&format!("  v{dest}.len += v{src}.len;\n"));
                    }
                    "__vow_string_push_byte" => {
                        let s = inst.args[0].0;
                        let byte = inst.args[1].0;
                        out.push_str(&format!(
                            "  __ESBMC_assert(v{s}.len < {}, \"string capacity\");\n\
                             \x20 v{s}.data[v{s}.len] = (int8_t)v{byte};\n  v{s}.len++;\n",
                            VOW_STRING_MAX
                        ));
                    }
                    "__vow_string_byte_at" => {
                        let s = inst.args[0].0;
                        let idx = inst.args[1].0;
                        out.push_str(&format!(
                            "  __ESBMC_assert(v{idx} >= 0 && v{idx} < v{s}.len, \"string bounds\");\n\
                             \x20 int64_t v{id} = (int64_t)(unsigned char)v{s}.data[v{idx}];\n\
                             \x20 __ESBMC_assume(v{id} >= 0 && v{id} <= 255);\n"
                        ));
                    }
                    "__vow_string_eq" => {
                        let a = inst.args[0].0;
                        let b = inst.args[1].0;
                        out.push_str(&format!(
                            "  _Bool v{id} = (v{a}.len == v{b}.len);\n\
                             \x20 if (v{id}) {{\n\
                             \x20   for (int64_t __i = 0; __i < v{a}.len; __i++) {{\n\
                             \x20     if (v{a}.data[__i] != v{b}.data[__i]) {{ v{id} = 0; break; }}\n\
                             \x20   }}\n\
                             \x20 }}\n"
                        ));
                    }
                    "__vow_string_contains" => {
                        let h = inst.args[0].0;
                        let n = inst.args[1].0;
                        out.push_str(&format!(
                            "  _Bool v{id} = 0;\n\
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
                        out.push_str(&format!("  __vow_hashmap_t v{id};\n  v{id}.len = 0;\n"));
                    }
                    "__vow_map_len" => {
                        let m = inst.args[0].0;
                        out.push_str(&format!("  int64_t v{id} = v{m}.len;\n"));
                    }
                    "__vow_map_insert" => {
                        let m = inst.args[0].0;
                        let k = inst.args[1].0;
                        let v = inst.args[2].0;
                        out.push_str(&format!(
                            "  {{\n\
                             \x20   _Bool __found = 0;\n\
                             \x20   for (int64_t __i = 0; __i < v{m}.len; __i++) {{\n\
                             \x20     if (v{m}.keys[__i] == v{k}) {{ v{m}.vals[__i] = v{v}; __found = 1; break; }}\n\
                             \x20   }}\n\
                             \x20   if (!__found) {{\n\
                             \x20     __ESBMC_assert(v{m}.len < {VOW_HASHMAP_MAX}, \"hashmap capacity\");\n\
                             \x20     v{m}.keys[v{m}.len] = v{k}; v{m}.vals[v{m}.len] = v{v}; v{m}.len++;\n\
                             \x20   }}\n\
                             \x20 }}\n"
                        ));
                    }
                    "__vow_map_get" => {
                        let m = inst.args[0].0;
                        let k = inst.args[1].0;
                        out.push_str(&format!(
                            "  int64_t v{id} = 0;\n\
                             \x20 for (int64_t __i = 0; __i < v{m}.len; __i++) {{\n\
                             \x20   if (v{m}.keys[__i] == v{k}) {{ v{id} = v{m}.vals[__i]; break; }}\n\
                             \x20 }}\n"
                        ));
                    }
                    "__vow_map_contains" => {
                        let m = inst.args[0].0;
                        let k = inst.args[1].0;
                        out.push_str(&format!(
                            "  _Bool v{id} = 0;\n\
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
                        out.push_str(&format!("  int32_t v{} = {};\n", id, v));
                    }
                    ConstantValue::I64(v) => {
                        out.push_str(&format!("  int64_t v{} = {}LL;\n", id, v));
                    }
                    ConstantValue::Bool(v) => {
                        out.push_str(&format!("  _Bool v{} = {};\n", id, *v as i32));
                    }
                }
            }
        }

        // Other calls, memory, region/linear/field ops — not yet supported for verification
        Opcode::Call
        | Opcode::Load
        | Opcode::Store
        | Opcode::RegionAlloc
        | Opcode::RegionFree
        | Opcode::LinearConsume
        | Opcode::LinearBorrow
        | Opcode::FieldSet => {
            emit_unmodelled(inst, out);
        }
        Opcode::FieldGet => {
            if vec_vars.contains(&id) {
                out.push_str(&format!(
                    "  /* FieldGet -> vec */ __vow_vec_t v{};\n  v{}.len = 0;\n",
                    id, id
                ));
            } else if string_vars.contains(&id) {
                out.push_str(&format!(
                    "  /* FieldGet -> string */ __vow_string_t v{};\n  v{}.len = 0;\n",
                    id, id
                ));
            } else if hashmap_vars.contains(&id) {
                out.push_str(&format!(
                    "  /* FieldGet -> hashmap */ __vow_hashmap_t v{};\n  v{}.len = 0;\n",
                    id, id
                ));
            } else {
                emit_unmodelled(inst, out);
            }
        }

        Opcode::RemF32 | Opcode::RemF64 => {
            out.push_str(&format!(
                "  /* float rem not modelled */ {} v{} = 0;\n",
                ir_ty_to_c(inst.ty),
                id
            ));
        }
    }
}

fn emit_unmodelled(inst: &Inst, out: &mut String) {
    let id = inst.id.0;
    out.push_str(&format!("  /* opcode {:?} not modelled */\n", inst.opcode));
    if inst.ty != Ty::Unit {
        let c_ty = match inst.ty {
            Ty::Ptr | Ty::LinearPtr => "int64_t",
            other => ir_ty_to_c(other),
        };
        out.push_str(&format!(
            "  {} v{} = __VERIFIER_nondet_{}();\n",
            c_ty,
            id,
            c_nondet_suffix(inst.ty)
        ));
    }
}

fn c_nondet_suffix(ty: Ty) -> &'static str {
    match ty {
        Ty::I32 => "int",
        Ty::I64 => "long",
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

pub fn emit_c_function(func: &Function, const_fns: &HashMap<FuncId, ConstantValue>) -> String {
    let mut out = String::new();
    let vec_vars = collect_vec_vars(func);
    let string_vars = collect_string_vars(func);
    let hashmap_vars = collect_hashmap_vars(func);

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
                        out.push_str(&format!("  __vow_vec_t v{};\n  v{}.len = 0;\n", id, id));
                    } else if string_vars.contains(&id) {
                        out.push_str(&format!("  __vow_string_t v{};\n  v{}.len = 0;\n", id, id));
                    } else if hashmap_vars.contains(&id) {
                        out.push_str(&format!("  __vow_hashmap_t v{};\n  v{}.len = 0;\n", id, id));
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

    // Pre-declare Phi variables (Upsilon writes may precede the Phi block)
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Phi {
                let id = inst.id.0;
                if vec_vars.contains(&id) {
                    out.push_str(&format!("  __vow_vec_t v{};\n", id));
                } else if string_vars.contains(&id) {
                    out.push_str(&format!("  __vow_string_t v{};\n", id));
                } else if hashmap_vars.contains(&id) {
                    out.push_str(&format!("  __vow_hashmap_t v{};\n", id));
                } else {
                    out.push_str(&format!("  {} v{};\n", ir_ty_to_c(inst.ty), id));
                }
            }
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
        for inst in &block.insts {
            if inst.opcode != Opcode::GetArg {
                emit_inst(
                    inst,
                    &mut out,
                    &vec_vars,
                    &string_vars,
                    &hashmap_vars,
                    const_fns,
                );
            }
        }
    }

    out.push_str("}\n");
    out
}

pub fn emit_c_module(funcs: &[&Function], const_fns: &HashMap<FuncId, ConstantValue>) -> String {
    let mut out = String::new();
    out.push_str("#include <stdint.h>\n");
    out.push_str("#include <stdbool.h>\n");
    out.push_str("extern void __ESBMC_assume(_Bool);\n");
    out.push_str("extern void __ESBMC_assert(_Bool, const char*);\n");
    out.push_str("extern int __VERIFIER_nondet_int(void);\n");
    out.push_str("extern long __VERIFIER_nondet_long(void);\n");
    out.push_str("extern float __VERIFIER_nondet_float(void);\n");
    out.push_str("extern double __VERIFIER_nondet_double(void);\n");
    out.push_str("extern _Bool __VERIFIER_nondet_bool(void);\n\n");
    out.push_str(&format!(
        "typedef struct {{ int64_t len; int64_t data[{}]; }} __vow_vec_t;\n",
        VOW_VEC_MAX
    ));
    out.push_str(&format!(
        "typedef struct {{ int64_t len; int8_t data[{}]; }} __vow_string_t;\n",
        VOW_STRING_MAX
    ));
    out.push_str(&format!(
        "typedef struct {{ int64_t len; int64_t keys[{}]; int64_t vals[{}]; }} __vow_hashmap_t;\n\n",
        VOW_HASHMAP_MAX, VOW_HASHMAP_MAX
    ));

    for func in funcs {
        out.push_str(&emit_c_function(func, const_fns));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use vow_diag::Blame;
    use vow_ir::{BasicBlock, BlockId, FuncId, InstId, VowEntry, VowId};
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
        };
        let c = emit_c_function(&func, &HashMap::new());
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
        };
        let c = emit_c_function(&func, &HashMap::new());
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
        let c = emit_c_function(&func, &HashMap::new());
        assert!(c.contains("int32_t v0 = 7"), "ConstI32: {c}");
        assert!(c.contains("float v1 = 1.5f"), "ConstF32: {c}");
        assert!(c.contains("double v2 = 2"), "ConstF64: {c}");
        assert!(c.contains("_Bool v3 = 1"), "ConstBool true: {c}");
        assert!(c.contains("_Bool v4 = 0"), "ConstBool false: {c}");
        assert!(c.contains("int32_t v5 = 0"), "ConstUnit: {c}");
        assert!(c.contains("void* v6 = 0"), "ConstStr: {c}");
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
        let c = emit_c_function(&func, &HashMap::new());
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
        let c = emit_c_function(&func, &HashMap::new());
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
        let c = emit_c_function(&func, &HashMap::new());
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
        let c = emit_c_function(&func, &HashMap::new());
        assert!(c.contains("!v0"), "not: {c}");
        assert!(c.contains("v0 && v1"), "and: {c}");
        assert!(c.contains("v0 || v1"), "or: {c}");
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
        };
        let c = emit_c_function(&func, &HashMap::new());
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
                },
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(42)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Upsilon,
                    ty: Ty::Unit,
                    args: vec![InstId(1)],
                    data: InstData::PhiTarget(InstId(0)),
                    origin: sp(),
                },
                inst(3, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(c.contains("int64_t v0;"), "phi declaration: {c}");
        assert!(c.contains("v0 = v1;"), "upsilon assignment: {c}");
    }

    #[test]
    fn emit_not_modelled_ops_produce_nondet() {
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
                },
                Inst {
                    id: InstId(1),
                    opcode: Opcode::FieldGet,
                    ty: Ty::I64,
                    args: vec![InstId(0)],
                    data: InstData::FieldIndex(0),
                    origin: sp(),
                },
                inst(2, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(c.contains("not modelled"), "not modelled comment: {c}");
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
                },
                inst(2, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
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
        let c = emit_c_function(&func, &HashMap::new());
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
        let out = emit_c_module(&[&f1, &f2], &HashMap::new());
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
                    },
                    inst(2, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
        };
        let c = emit_c_function(&func, &HashMap::new());
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
        let out = emit_c_module(&[&f], &HashMap::new());
        assert!(out.contains("__vow_vec_t"), "vec typedef: {out}");
        assert!(out.contains("int64_t len"), "vec len field: {out}");
        assert!(out.contains("int64_t data["), "vec data field: {out}");
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
                },
                inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
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
                },
                inst(3, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(42)),
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(2), InstId(3)],
                    data: InstData::CallExtern("__vow_vec_push_val".to_string()),
                    origin: sp(),
                },
                inst(5, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(
            c.contains("__ESBMC_assert(v2.len < 128, \"vec capacity\")"),
            "push capacity: {c}"
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
                },
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![InstId(2)],
                    data: InstData::CallExtern("__vow_vec_len".to_string()),
                    origin: sp(),
                },
                inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(c.contains("int64_t v3 = v2.len;"), "vec len: {c}");
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
                },
                inst(3, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![InstId(2), InstId(3)],
                    data: InstData::CallExtern("__vow_vec_get_val".to_string()),
                    origin: sp(),
                },
                inst(5, Opcode::Return, Ty::Unit, vec![4], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(
            c.contains("__ESBMC_assert(v3 >= 0 && v3 < v2.len"),
            "bounds check: {c}"
        );
        assert!(c.contains("int64_t v4 = v2.data[v3]"), "get access: {c}");
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
                },
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(2)],
                    data: InstData::CallExtern("__vow_vec_pop".to_string()),
                    origin: sp(),
                },
                inst(4, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(c.contains("v2.len--;"), "pop decrement: {c}");
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
                },
                inst(6, Opcode::Return, Ty::Unit, vec![], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
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
                    },
                    Inst {
                        id: InstId(4),
                        opcode: Opcode::Upsilon,
                        ty: Ty::Unit,
                        args: vec![InstId(3)],
                        data: InstData::PhiTarget(InstId(0)),
                        origin: sp(),
                    },
                    inst(5, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
        };
        let c = emit_c_function(&func, &HashMap::new());
        assert!(c.contains("__vow_vec_t v0;"), "phi uses vec type: {c}");
    }

    #[test]
    fn emit_non_vec_call_still_nondet() {
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
                },
                inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(c.contains("not modelled"), "non-vec call still nondet: {c}");
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
        let out = emit_c_module(&[&f], &HashMap::new());
        assert!(out.contains("__vow_string_t"), "string typedef: {out}");
        assert!(out.contains("int8_t data["), "string data field: {out}");
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
                },
                inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(c.contains("__vow_string_t v1;"), "string struct decl: {c}");
        assert!(
            c.contains("v1.len = __VERIFIER_nondet_long()"),
            "nondet len: {c}"
        );
        assert!(
            c.contains("__ESBMC_assume(v1.len >= 0 && v1.len < 256)"),
            "len bounded: {c}"
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
                },
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![InstId(1)],
                    data: InstData::CallExtern("__vow_string_len".to_string()),
                    origin: sp(),
                },
                inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(c.contains("int64_t v2 = v1.len;"), "string len: {c}");
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
                },
                inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(65)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(1), InstId(2)],
                    data: InstData::CallExtern("__vow_string_push_byte".to_string()),
                    origin: sp(),
                },
                inst(4, Opcode::Return, Ty::Unit, vec![1], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(
            c.contains("__ESBMC_assert(v1.len < 256, \"string capacity\")"),
            "push_byte capacity: {c}"
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
                },
                inst(2, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(1)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(2)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                },
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(1), InstId(3)],
                    data: InstData::CallExtern("__vow_string_push_str".to_string()),
                    origin: sp(),
                },
                inst(5, Opcode::Return, Ty::Unit, vec![1], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
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
                },
                inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![InstId(1), InstId(2)],
                    data: InstData::CallExtern("__vow_string_byte_at".to_string()),
                    origin: sp(),
                },
                inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(
            c.contains("__ESBMC_assert(v2 >= 0 && v2 < v1.len"),
            "bounds check: {c}"
        );
        assert!(
            c.contains("int64_t v3 = (int64_t)(unsigned char)v1.data[v2]"),
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
                },
                inst(2, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(1)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(2)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                },
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::Bool,
                    args: vec![InstId(1), InstId(3)],
                    data: InstData::CallExtern("__vow_string_eq".to_string()),
                    origin: sp(),
                },
                inst(5, Opcode::Return, Ty::Unit, vec![4], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(
            c.contains("_Bool v4 = (v1.len == v3.len)"),
            "string eq length check: {c}"
        );
        assert!(
            c.contains("v1.data[__i] != v3.data[__i]"),
            "string eq byte comparison: {c}"
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
                },
                inst(2, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(1)),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::Call,
                    ty: Ty::Ptr,
                    args: vec![InstId(2)],
                    data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    origin: sp(),
                },
                Inst {
                    id: InstId(4),
                    opcode: Opcode::Call,
                    ty: Ty::Bool,
                    args: vec![InstId(1), InstId(3)],
                    data: InstData::CallExtern("__vow_string_contains".to_string()),
                    origin: sp(),
                },
                inst(5, Opcode::Return, Ty::Unit, vec![4], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(c.contains("_Bool v4 = 0;"), "contains init: {c}");
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
                    },
                    inst(1, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::Call,
                        ty: Ty::Ptr,
                        args: vec![InstId(1)],
                        data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                        origin: sp(),
                    },
                    Inst {
                        id: InstId(3),
                        opcode: Opcode::Upsilon,
                        ty: Ty::Unit,
                        args: vec![InstId(2)],
                        data: InstData::PhiTarget(InstId(0)),
                        origin: sp(),
                    },
                    inst(4, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
        };
        let c = emit_c_function(&func, &HashMap::new());
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
                },
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(1)],
                    data: InstData::CallExtern("__vow_string_print".to_string()),
                    origin: sp(),
                },
                inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
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
                },
                inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
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
                },
                Inst {
                    id: InstId(1),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![InstId(0)],
                    data: InstData::CallExtern("__vow_map_len".to_string()),
                    origin: sp(),
                },
                inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(c.contains("int64_t v1 = v0.len;"), "hashmap len: {c}");
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
                },
                inst(4, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(c.contains("v0.keys[__i] == v1"), "key search: {c}");
        assert!(c.contains("v0.vals[__i] = v2"), "update existing: {c}");
        assert!(
            c.contains("__ESBMC_assert(v0.len < 64, \"hashmap capacity\")"),
            "insert capacity: {c}"
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
                },
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(5)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_map_get".to_string()),
                    origin: sp(),
                },
                inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(c.contains("int64_t v2 = 0;"), "get default: {c}");
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
                },
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(7)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Bool,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_map_contains".to_string()),
                    origin: sp(),
                },
                inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
        assert!(c.contains("_Bool v2 = 0;"), "contains default: {c}");
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
                },
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(3)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Call,
                    ty: Ty::Unit,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::CallExtern("__vow_map_remove".to_string()),
                    origin: sp(),
                },
                inst(3, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        );
        let c = emit_c_function(&func, &HashMap::new());
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
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::Call,
                        ty: Ty::Ptr,
                        args: vec![],
                        data: InstData::CallExtern("__vow_map_new".to_string()),
                        origin: sp(),
                    },
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::Upsilon,
                        ty: Ty::Unit,
                        args: vec![InstId(1)],
                        data: InstData::PhiTarget(InstId(0)),
                        origin: sp(),
                    },
                    inst(3, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
        };
        let c = emit_c_function(&func, &HashMap::new());
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
        let c = emit_c_module(&[&func], &HashMap::new());
        assert!(
            c.contains("__vow_hashmap_t"),
            "hashmap typedef in header: {c}"
        );
        assert!(c.contains("int64_t keys["), "keys array in typedef: {c}");
        assert!(c.contains("int64_t vals["), "vals array in typedef: {c}");
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
        };
        let c = emit_c_function(&func, &HashMap::new());
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
        };
        let module = Module {
            name: "test".to_string(),
            functions: vec![func],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
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
        };
        let module = Module {
            name: "test".to_string(),
            functions: vec![func],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
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
        };
        let mut out = String::new();
        emit_inst(
            &call_inst,
            &mut out,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &const_fns,
        );
        assert!(
            out.contains("int64_t v5 = 42LL;"),
            "inlined constant: {out}"
        );
    }

    #[test]
    fn emit_falls_back_for_unknown_call_target() {
        let call_inst = Inst {
            id: InstId(5),
            opcode: Opcode::Call,
            ty: Ty::I64,
            args: vec![],
            data: InstData::CallTarget(FuncId(99)),
            origin: sp(),
        };
        let mut out = String::new();
        emit_inst(
            &call_inst,
            &mut out,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
        );
        assert!(
            out.contains("__VERIFIER_nondet_long()"),
            "nondet fallback: {out}"
        );
    }
}
