use crate::types::{BasicBlock, Function, Inst, InstData, Module, Opcode, Ty};
use std::fmt;
use vow_syntax::ast::Effect;

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Ty::I32 => "i32",
            Ty::I64 => "i64",
            Ty::F32 => "f32",
            Ty::F64 => "f64",
            Ty::Bool => "Bool",
            Ty::Unit => "Void",
            Ty::Ptr => "ptr",
            Ty::LinearPtr => "linear_ptr",
        };
        write!(f, "{s}")
    }
}

fn opcode_name(opcode: &Opcode) -> &'static str {
    match opcode {
        Opcode::ConstI32 => "ConstI32",
        Opcode::ConstI64 => "ConstI64",
        Opcode::ConstF32 => "ConstF32",
        Opcode::ConstF64 => "ConstF64",
        Opcode::ConstBool => "ConstBool",
        Opcode::ConstStr => "ConstStr",
        Opcode::ConstUnit => "ConstUnit",
        Opcode::GetArg => "GetArg",
        Opcode::WrappingAddI32 => "WrappingAddI32",
        Opcode::WrappingSubI32 => "WrappingSubI32",
        Opcode::WrappingMulI32 => "WrappingMulI32",
        Opcode::WrappingDivI32 => "WrappingDivI32",
        Opcode::WrappingRemI32 => "WrappingRemI32",
        Opcode::CheckedAddI32 => "CheckedAddI32",
        Opcode::CheckedSubI32 => "CheckedSubI32",
        Opcode::CheckedMulI32 => "CheckedMulI32",
        Opcode::CheckedDivI32 => "CheckedDivI32",
        Opcode::CheckedRemI32 => "CheckedRemI32",
        Opcode::EqI32 => "EqI32",
        Opcode::NeI32 => "NeI32",
        Opcode::LtI32 => "LtI32",
        Opcode::LeI32 => "LeI32",
        Opcode::GtI32 => "GtI32",
        Opcode::GeI32 => "GeI32",
        Opcode::WrappingAddI64 => "WrappingAddI64",
        Opcode::WrappingSubI64 => "WrappingSubI64",
        Opcode::WrappingMulI64 => "WrappingMulI64",
        Opcode::WrappingDivI64 => "WrappingDivI64",
        Opcode::WrappingRemI64 => "WrappingRemI64",
        Opcode::CheckedAddI64 => "CheckedAddI64",
        Opcode::CheckedSubI64 => "CheckedSubI64",
        Opcode::CheckedMulI64 => "CheckedMulI64",
        Opcode::CheckedDivI64 => "CheckedDivI64",
        Opcode::CheckedRemI64 => "CheckedRemI64",
        Opcode::EqI64 => "EqI64",
        Opcode::NeI64 => "NeI64",
        Opcode::LtI64 => "LtI64",
        Opcode::LeI64 => "LeI64",
        Opcode::GtI64 => "GtI64",
        Opcode::GeI64 => "GeI64",
        Opcode::AddF32 => "AddF32",
        Opcode::SubF32 => "SubF32",
        Opcode::MulF32 => "MulF32",
        Opcode::DivF32 => "DivF32",
        Opcode::RemF32 => "RemF32",
        Opcode::EqF32 => "EqF32",
        Opcode::NeF32 => "NeF32",
        Opcode::LtF32 => "LtF32",
        Opcode::LeF32 => "LeF32",
        Opcode::GtF32 => "GtF32",
        Opcode::GeF32 => "GeF32",
        Opcode::AddF64 => "AddF64",
        Opcode::SubF64 => "SubF64",
        Opcode::MulF64 => "MulF64",
        Opcode::DivF64 => "DivF64",
        Opcode::RemF64 => "RemF64",
        Opcode::EqF64 => "EqF64",
        Opcode::NeF64 => "NeF64",
        Opcode::LtF64 => "LtF64",
        Opcode::LeF64 => "LeF64",
        Opcode::GtF64 => "GtF64",
        Opcode::GeF64 => "GeF64",
        Opcode::Not => "Not",
        Opcode::And => "And",
        Opcode::Or => "Or",
        Opcode::Load => "Load",
        Opcode::Store => "Store",
        Opcode::Branch => "Branch",
        Opcode::Jump => "Jump",
        Opcode::Return => "Return",
        Opcode::Unreachable => "Unreachable",
        Opcode::Phi => "Phi",
        Opcode::Upsilon => "Upsilon",
        Opcode::VowRequires => "VowRequires",
        Opcode::VowEnsures => "VowEnsures",
        Opcode::VowInvariant => "VowInvariant",
        Opcode::Call => "Call",
        Opcode::RegionAlloc => "RegionAlloc",
        Opcode::RegionFree => "RegionFree",
        Opcode::LinearConsume => "LinearConsume",
        Opcode::LinearBorrow => "LinearBorrow",
        Opcode::FieldGet => "FieldGet",
        Opcode::FieldSet => "FieldSet",
    }
}

fn format_data(data: &InstData) -> Option<String> {
    match data {
        InstData::None => None,
        InstData::ConstI32(v) => Some(v.to_string()),
        InstData::ConstI64(v) => Some(v.to_string()),
        InstData::ConstF32(v) => Some(v.to_string()),
        InstData::ConstF64(v) => Some(v.to_string()),
        InstData::ConstBool(v) => Some(v.to_string()),
        InstData::ConstStr(idx) => Some(format!("@{idx}")),
        InstData::ArgIndex(n) => Some(n.to_string()),
        InstData::PhiTarget(id) => Some(format!("%{}", id.0)),
        InstData::CallTarget(fid) => Some(format!("func{}", fid.0)),
        InstData::CallExtern(sym) => Some(format!("extern:{sym}")),
        InstData::BranchTargets {
            then_block,
            else_block,
        } => Some(format!("block_{}, block_{}", then_block.0, else_block.0)),
        InstData::JumpTarget(bid) => Some(format!("block_{}", bid.0)),
        InstData::RegionId(rid) => Some(format!("region_{}", rid.0)),
        InstData::VowId(vid) => Some(format!("vow_{}", vid.0)),
        InstData::AllocSize { size, align } => Some(format!("size={size},align={align}")),
        InstData::FieldIndex(idx) => Some(format!("field_{idx}")),
    }
}

fn print_inst(inst: &Inst) -> String {
    let args: Vec<String> = inst.args.iter().map(|id| format!("%{}", id.0)).collect();
    let args_str = args.join(", ");
    let name = match format_data(&inst.data) {
        Some(d) => format!("{}[{}]", opcode_name(&inst.opcode), d),
        None => opcode_name(&inst.opcode).to_string(),
    };
    format!(
        "    {:<10}  %{} = {}({})",
        inst.ty.to_string(),
        inst.id.0,
        name,
        args_str
    )
}

fn print_block(block: &BasicBlock, is_entry: bool) -> String {
    let header = if is_entry {
        format!("  entry (block {}):", block.id.0)
    } else {
        format!("  block_{}:", block.id.0)
    };
    let mut lines = vec![header];
    for inst in &block.insts {
        lines.push(print_inst(inst));
    }
    lines.join("\n")
}

pub fn print_function(func: &Function) -> String {
    let params: Vec<String> = func.params.iter().map(|t| t.to_string()).collect();
    let effects: Vec<&str> = func.effects.iter().map(effect_str).collect();
    let header = format!(
        "fn {}({}) -> {} [{}]:",
        func.name,
        params.join(", "),
        func.return_ty,
        effects.join(", ")
    );
    let mut parts = vec![header];
    for (i, block) in func.blocks.iter().enumerate() {
        parts.push(print_block(block, i == 0));
    }
    parts.join("\n")
}

pub fn print_module(module: &Module) -> String {
    let mut parts = Vec::new();
    if !module.strings.is_empty() {
        let pool: Vec<String> = module
            .strings
            .iter()
            .enumerate()
            .map(|(i, s)| format!("  @{i} = {:?}", s))
            .collect();
        parts.push(format!("strings:\n{}", pool.join("\n")));
    }
    for sl in &module.struct_layouts {
        let fields: Vec<String> = sl
            .fields
            .iter()
            .map(|f| format!("{}: {}", f.name, f.ty))
            .collect();
        parts.push(format!("struct {} {{ {} }}", sl.name, fields.join(", ")));
    }
    for el in &module.enum_layouts {
        let variants: Vec<String> = el
            .variants
            .iter()
            .map(|v| {
                if v.payload.is_empty() {
                    format!("{}(tag={})", v.name, v.tag)
                } else {
                    let fields: Vec<String> = v.payload.iter().map(|f| f.ty.to_string()).collect();
                    format!("{}(tag={}, {})", v.name, v.tag, fields.join(", "))
                }
            })
            .collect();
        parts.push(format!("enum {} {{ {} }}", el.name, variants.join(", ")));
    }
    for func in &module.functions {
        parts.push(print_function(func));
    }
    parts.join("\n\n")
}

fn effect_str(e: &Effect) -> &'static str {
    match e {
        Effect::IO => "IO",
        Effect::Panic => "Panic",
        Effect::Read => "Read",
        Effect::Unsafe => "Unsafe",
        Effect::Write => "Write",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        BasicBlock, BlockId, EnumLayout, FieldLayout, FuncId, Function, Inst, InstData, InstId,
        Module, Opcode, RegionId, StructLayout, Ty, VariantLayout, VowId,
    };
    use vow_syntax::span::Span;

    fn dummy_span() -> Span {
        Span::new(0, 0)
    }

    fn make_inst(id: u32, opcode: Opcode, ty: Ty, args: Vec<InstId>, data: InstData) -> Inst {
        Inst {
            id: InstId(id),
            opcode,
            ty,
            args,
            data,
            origin: dummy_span(),
        }
    }

    fn make_func(
        id: u32,
        name: &str,
        params: Vec<Ty>,
        return_ty: Ty,
        effects: Vec<Effect>,
        blocks: Vec<BasicBlock>,
    ) -> Function {
        Function {
            id: FuncId(id),
            name: name.to_string(),
            params,
            param_names: vec![],
            return_ty,
            effects,
            vows: vec![],
            blocks,
            local_names: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn print_const_function() {
        let insts = vec![
            make_inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(42)),
            make_inst(1, Opcode::Return, Ty::Unit, vec![InstId(0)], InstData::None),
        ];
        let block = BasicBlock {
            id: BlockId(0),
            insts,
        };
        let func = make_func(0, "const_fn", vec![], Ty::I64, vec![], vec![block]);
        let output = print_function(&func);
        assert!(output.contains("fn const_fn()"));
        assert!(output.contains("-> i64"));
        assert!(output.contains("ConstI64[42]"));
        assert!(output.contains("%0"));
        assert!(output.contains("Return"));
    }

    #[test]
    fn print_function_with_args() {
        let insts = vec![
            make_inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
            make_inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
            make_inst(
                2,
                Opcode::WrappingAddI64,
                Ty::I64,
                vec![InstId(0), InstId(1)],
                InstData::None,
            ),
            make_inst(3, Opcode::Return, Ty::Unit, vec![InstId(2)], InstData::None),
        ];
        let block = BasicBlock {
            id: BlockId(0),
            insts,
        };
        let func = make_func(
            0,
            "add",
            vec![Ty::I64, Ty::I64],
            Ty::I64,
            vec![],
            vec![block],
        );
        let output = print_function(&func);
        assert!(output.contains("fn add(i64, i64)"));
        assert!(output.contains("GetArg[0]"));
        assert!(output.contains("GetArg[1]"));
        assert!(output.contains("WrappingAddI64(%0, %1)"));
        assert!(output.contains("Return(%2)"));
    }

    #[test]
    fn print_block_header() {
        let entry_insts = vec![make_inst(
            0,
            Opcode::Return,
            Ty::Unit,
            vec![],
            InstData::None,
        )];
        let second_insts = vec![make_inst(
            1,
            Opcode::Unreachable,
            Ty::Unit,
            vec![],
            InstData::None,
        )];
        let func = make_func(
            0,
            "blocks",
            vec![],
            Ty::Unit,
            vec![],
            vec![
                BasicBlock {
                    id: BlockId(0),
                    insts: entry_insts,
                },
                BasicBlock {
                    id: BlockId(1),
                    insts: second_insts,
                },
            ],
        );
        let output = print_function(&func);
        assert!(output.contains("entry (block 0):"));
        assert!(output.contains("block_1:"));
    }

    #[test]
    fn print_module_with_effects() {
        let insts = vec![make_inst(
            0,
            Opcode::Return,
            Ty::Unit,
            vec![],
            InstData::None,
        )];
        let block = BasicBlock {
            id: BlockId(0),
            insts,
        };
        let func = make_func(
            0,
            "io_fn",
            vec![],
            Ty::Unit,
            vec![Effect::IO, Effect::Write],
            vec![block],
        );
        let module = Module {
            name: "m".to_string(),
            functions: vec![func],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
        };
        let output = print_module(&module);
        assert!(output.contains("IO"));
        assert!(output.contains("Write"));
    }

    #[test]
    fn ty_display_all_variants() {
        assert_eq!(Ty::I32.to_string(), "i32");
        assert_eq!(Ty::I64.to_string(), "i64");
        assert_eq!(Ty::F32.to_string(), "f32");
        assert_eq!(Ty::F64.to_string(), "f64");
        assert_eq!(Ty::Bool.to_string(), "Bool");
        assert_eq!(Ty::Unit.to_string(), "Void");
        assert_eq!(Ty::Ptr.to_string(), "ptr");
        assert_eq!(Ty::LinearPtr.to_string(), "linear_ptr");
    }

    #[test]
    fn opcode_name_all_variants() {
        let pairs = [
            (Opcode::ConstI32, "ConstI32"),
            (Opcode::ConstF32, "ConstF32"),
            (Opcode::ConstF64, "ConstF64"),
            (Opcode::ConstBool, "ConstBool"),
            (Opcode::ConstStr, "ConstStr"),
            (Opcode::ConstUnit, "ConstUnit"),
            (Opcode::WrappingAddI32, "WrappingAddI32"),
            (Opcode::WrappingSubI32, "WrappingSubI32"),
            (Opcode::WrappingMulI32, "WrappingMulI32"),
            (Opcode::WrappingDivI32, "WrappingDivI32"),
            (Opcode::WrappingRemI32, "WrappingRemI32"),
            (Opcode::CheckedAddI32, "CheckedAddI32"),
            (Opcode::CheckedSubI32, "CheckedSubI32"),
            (Opcode::CheckedMulI32, "CheckedMulI32"),
            (Opcode::CheckedDivI32, "CheckedDivI32"),
            (Opcode::CheckedRemI32, "CheckedRemI32"),
            (Opcode::EqI32, "EqI32"),
            (Opcode::NeI32, "NeI32"),
            (Opcode::LtI32, "LtI32"),
            (Opcode::LeI32, "LeI32"),
            (Opcode::GtI32, "GtI32"),
            (Opcode::GeI32, "GeI32"),
            (Opcode::WrappingSubI64, "WrappingSubI64"),
            (Opcode::WrappingMulI64, "WrappingMulI64"),
            (Opcode::WrappingDivI64, "WrappingDivI64"),
            (Opcode::WrappingRemI64, "WrappingRemI64"),
            (Opcode::CheckedAddI64, "CheckedAddI64"),
            (Opcode::CheckedSubI64, "CheckedSubI64"),
            (Opcode::CheckedMulI64, "CheckedMulI64"),
            (Opcode::CheckedDivI64, "CheckedDivI64"),
            (Opcode::CheckedRemI64, "CheckedRemI64"),
            (Opcode::EqI64, "EqI64"),
            (Opcode::NeI64, "NeI64"),
            (Opcode::LtI64, "LtI64"),
            (Opcode::LeI64, "LeI64"),
            (Opcode::GtI64, "GtI64"),
            (Opcode::GeI64, "GeI64"),
            (Opcode::AddF32, "AddF32"),
            (Opcode::SubF32, "SubF32"),
            (Opcode::MulF32, "MulF32"),
            (Opcode::DivF32, "DivF32"),
            (Opcode::RemF32, "RemF32"),
            (Opcode::EqF32, "EqF32"),
            (Opcode::NeF32, "NeF32"),
            (Opcode::LtF32, "LtF32"),
            (Opcode::LeF32, "LeF32"),
            (Opcode::GtF32, "GtF32"),
            (Opcode::GeF32, "GeF32"),
            (Opcode::AddF64, "AddF64"),
            (Opcode::SubF64, "SubF64"),
            (Opcode::MulF64, "MulF64"),
            (Opcode::DivF64, "DivF64"),
            (Opcode::RemF64, "RemF64"),
            (Opcode::EqF64, "EqF64"),
            (Opcode::NeF64, "NeF64"),
            (Opcode::LtF64, "LtF64"),
            (Opcode::LeF64, "LeF64"),
            (Opcode::GtF64, "GtF64"),
            (Opcode::GeF64, "GeF64"),
            (Opcode::Not, "Not"),
            (Opcode::And, "And"),
            (Opcode::Or, "Or"),
            (Opcode::Load, "Load"),
            (Opcode::Store, "Store"),
            (Opcode::Branch, "Branch"),
            (Opcode::Jump, "Jump"),
            (Opcode::Phi, "Phi"),
            (Opcode::Upsilon, "Upsilon"),
            (Opcode::VowRequires, "VowRequires"),
            (Opcode::VowEnsures, "VowEnsures"),
            (Opcode::VowInvariant, "VowInvariant"),
            (Opcode::Call, "Call"),
            (Opcode::RegionAlloc, "RegionAlloc"),
            (Opcode::RegionFree, "RegionFree"),
            (Opcode::LinearConsume, "LinearConsume"),
            (Opcode::LinearBorrow, "LinearBorrow"),
            (Opcode::FieldGet, "FieldGet"),
            (Opcode::FieldSet, "FieldSet"),
        ];
        for (op, expected) in pairs {
            assert_eq!(opcode_name(&op), expected);
        }
    }

    #[test]
    fn format_data_all_variants() {
        assert_eq!(format_data(&InstData::None), None);
        assert_eq!(format_data(&InstData::ConstI32(7)), Some("7".to_string()));
        assert_eq!(format_data(&InstData::ConstI64(-1)), Some("-1".to_string()));
        assert_eq!(
            format_data(&InstData::ConstBool(false)),
            Some("false".to_string())
        );
        assert_eq!(format_data(&InstData::ConstStr(3)), Some("@3".to_string()));
        assert_eq!(format_data(&InstData::ArgIndex(2)), Some("2".to_string()));
        assert_eq!(
            format_data(&InstData::PhiTarget(InstId(5))),
            Some("%5".to_string())
        );
        assert_eq!(
            format_data(&InstData::CallTarget(FuncId(1))),
            Some("func1".to_string())
        );
        assert_eq!(
            format_data(&InstData::CallExtern("__foo".to_string())),
            Some("extern:__foo".to_string())
        );
        assert_eq!(
            format_data(&InstData::BranchTargets {
                then_block: BlockId(2),
                else_block: BlockId(3)
            }),
            Some("block_2, block_3".to_string())
        );
        assert_eq!(
            format_data(&InstData::JumpTarget(BlockId(4))),
            Some("block_4".to_string())
        );
        assert_eq!(
            format_data(&InstData::RegionId(RegionId(0))),
            Some("region_0".to_string())
        );
        assert_eq!(
            format_data(&InstData::VowId(VowId(1))),
            Some("vow_1".to_string())
        );
        assert_eq!(
            format_data(&InstData::AllocSize { size: 8, align: 8 }),
            Some("size=8,align=8".to_string())
        );
        assert_eq!(
            format_data(&InstData::FieldIndex(0)),
            Some("field_0".to_string())
        );
    }

    #[test]
    fn print_module_with_strings_structs_enums() {
        let module = Module {
            name: "m".to_string(),
            functions: vec![],
            strings: vec!["hello".to_string(), "world".to_string()],
            struct_layouts: vec![StructLayout {
                name: "Point".to_string(),
                fields: vec![
                    FieldLayout {
                        name: "x".to_string(),
                        ty: Ty::I64,
                    },
                    FieldLayout {
                        name: "y".to_string(),
                        ty: Ty::I64,
                    },
                ],
                is_linear: false,
            }],
            enum_layouts: vec![EnumLayout {
                name: "Color".to_string(),
                variants: vec![
                    VariantLayout {
                        name: "Red".to_string(),
                        tag: 0,
                        payload: vec![],
                    },
                    VariantLayout {
                        name: "Rgb".to_string(),
                        tag: 1,
                        payload: vec![
                            FieldLayout {
                                name: "r".to_string(),
                                ty: Ty::I64,
                            },
                            FieldLayout {
                                name: "g".to_string(),
                                ty: Ty::I64,
                            },
                        ],
                    },
                ],
            }],
        };
        let out = print_module(&module);
        assert!(out.contains("strings:"), "strings section: {out}");
        assert!(out.contains("@0 = \"hello\""), "string 0: {out}");
        assert!(out.contains("@1 = \"world\""), "string 1: {out}");
        assert!(out.contains("struct Point"), "struct: {out}");
        assert!(out.contains("x: i64"), "field x: {out}");
        assert!(out.contains("enum Color"), "enum: {out}");
        assert!(out.contains("Red(tag=0)"), "Red variant: {out}");
        assert!(out.contains("Rgb(tag=1"), "Rgb variant: {out}");
    }

    #[test]
    fn print_function_with_float_const() {
        let insts = vec![
            make_inst(
                0,
                Opcode::ConstF32,
                Ty::F32,
                vec![],
                InstData::ConstF32(1.5),
            ),
            make_inst(1, Opcode::Return, Ty::Unit, vec![InstId(0)], InstData::None),
        ];
        let block = BasicBlock {
            id: BlockId(0),
            insts,
        };
        let func = make_func(0, "f32_fn", vec![Ty::F32], Ty::F32, vec![], vec![block]);
        let out = print_function(&func);
        assert!(out.contains("ConstF32[1.5]"), "f32 const: {out}");
        assert!(out.contains("f32"), "f32 type: {out}");
    }
}
