use vow_ir::{Function, Inst, InstData, Opcode, Ty};

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
// Expression / statement emission
// ---------------------------------------------------------------------------

fn emit_inst(inst: &Inst, out: &mut String) {
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
                out.push_str(&format!("  return v{};\n", val_id.0));
            } else {
                out.push_str("  return 0;\n");
            }
        }
        Opcode::Unreachable => {
            out.push_str("  __ESBMC_assume(0); /* unreachable */\n");
        }

        // Phi / Upsilon — translated as variable copies
        Opcode::Phi => {
            out.push_str(&format!("  {} v{};\n", ir_ty_to_c(inst.ty), id));
        }
        Opcode::Upsilon => {
            if let InstData::PhiTarget(phi_id) = inst.data {
                let val = inst.args[0].0;
                out.push_str(&format!("  v{} = v{};\n", phi_id.0, val));
            }
        }

        // Calls, memory, region/linear ops — not yet supported for verification
        Opcode::Call
        | Opcode::Load
        | Opcode::Store
        | Opcode::RegionAlloc
        | Opcode::RegionFree
        | Opcode::LinearConsume
        | Opcode::LinearBorrow => {
            out.push_str(&format!("  /* opcode {:?} not modelled */\n", inst.opcode));
            if inst.ty != Ty::Unit {
                out.push_str(&format!(
                    "  {} v{} = __VERIFIER_nondet_{}();\n",
                    ir_ty_to_c(inst.ty),
                    id,
                    c_nondet_suffix(inst.ty)
                ));
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

fn c_nondet_suffix(ty: Ty) -> &'static str {
    match ty {
        Ty::I32 => "int",
        Ty::I64 => "long",
        Ty::F32 => "float",
        Ty::F64 => "double",
        Ty::Bool => "bool",
        _ => "int",
    }
}

// ---------------------------------------------------------------------------
// Function emission
// ---------------------------------------------------------------------------

pub fn emit_c_function(func: &Function) -> String {
    let mut out = String::new();

    // Return type
    let ret_c = if func.return_ty == Ty::Unit {
        "void"
    } else {
        ir_ty_to_c(func.return_ty)
    };

    // Parameters (skip Unit params)
    let params: Vec<String> = func
        .params
        .iter()
        .enumerate()
        .filter(|&(_, &ty)| ty != Ty::Unit)
        .map(|(i, &ty)| format!("{} p{}", ir_ty_to_c(ty), i))
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
                if let Some(&(_, cl)) = arg_var_map.iter().find(|(ir, _)| *ir == idx) {
                    out.push_str(&format!(
                        "  {} v{} = p{};\n",
                        ir_ty_to_c(inst.ty),
                        inst.id.0,
                        cl
                    ));
                } else {
                    out.push_str(&format!("  int32_t v{} = 0; /* unit arg */\n", inst.id.0));
                }
            }
        }
    }

    // Emit blocks
    let first_block_id = func.blocks.first().map(|b| b.id);
    for block in &func.blocks {
        if Some(block.id) != first_block_id {
            out.push_str(&format!("block{}:;\n", block.id.0));
        }
        for inst in &block.insts {
            if inst.opcode != Opcode::GetArg {
                emit_inst(inst, &mut out);
            }
        }
    }

    out.push_str("}\n");
    out
}

pub fn emit_c_module(funcs: &[&Function]) -> String {
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

    for func in funcs {
        out.push_str(&emit_c_function(func));
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
        };
        let c = emit_c_function(&func);
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
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "y != 0".to_string(),
                blame: Blame::Caller,
                bindings: vec![],
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
        };
        let c = emit_c_function(&func);
        assert!(c.contains("__ESBMC_assume(v3)"), "requires: {c}");
        assert!(!c.contains("__ESBMC_assert"), "no assert for requires: {c}");
    }

    #[test]
    fn emit_vow_ensures_as_assert() {
        let func = Function {
            id: FuncId(0),
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Bool,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "result".to_string(),
                blame: Blame::Callee,
                bindings: vec![],
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
        };
        let c = emit_c_function(&func);
        assert!(c.contains("__ESBMC_assert(v0"), "ensures: {c}");
    }
}
