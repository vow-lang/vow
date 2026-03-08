use vow_diag::Blame;
use vow_syntax::ast::Effect;
use vow_syntax::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FuncId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegionId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VowId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    ConstI32,
    ConstI64,
    ConstF32,
    ConstF64,
    ConstBool,
    ConstStr,
    ConstUnit,

    GetArg,

    WrappingAddI32,
    WrappingSubI32,
    WrappingMulI32,
    WrappingDivI32,
    WrappingRemI32,
    CheckedAddI32,
    CheckedSubI32,
    CheckedMulI32,
    CheckedDivI32,
    CheckedRemI32,
    EqI32,
    NeI32,
    LtI32,
    LeI32,
    GtI32,
    GeI32,

    WrappingAddI64,
    WrappingSubI64,
    WrappingMulI64,
    WrappingDivI64,
    WrappingRemI64,
    CheckedAddI64,
    CheckedSubI64,
    CheckedMulI64,
    CheckedDivI64,
    CheckedRemI64,
    EqI64,
    NeI64,
    LtI64,
    LeI64,
    GtI64,
    GeI64,

    AddF32,
    SubF32,
    MulF32,
    DivF32,
    RemF32,
    EqF32,
    NeF32,
    LtF32,
    LeF32,
    GtF32,
    GeF32,

    AddF64,
    SubF64,
    MulF64,
    DivF64,
    RemF64,
    EqF64,
    NeF64,
    LtF64,
    LeF64,
    GtF64,
    GeF64,

    Not,
    And,
    Or,

    Load,
    Store,

    Branch,
    Jump,
    Return,
    Unreachable,

    Phi,
    Upsilon,

    VowRequires,
    VowEnsures,
    VowInvariant,
    Assert,
    Assume,

    Call,

    RegionAlloc,
    RegionFree,

    LinearConsume,
    LinearBorrow,

    FieldGet,
    FieldSet,
}

impl Opcode {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Opcode::Branch | Opcode::Jump | Opcode::Return | Opcode::Unreachable
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum InstData {
    None,
    ConstI32(i32),
    ConstI64(i64),
    ConstF32(f32),
    ConstF64(f64),
    ConstBool(bool),
    ArgIndex(u32),
    PhiTarget(InstId),
    ConstStr(u32),
    CallTarget(FuncId),
    CallExtern(String),
    BranchTargets {
        then_block: BlockId,
        else_block: BlockId,
    },
    JumpTarget(BlockId),
    RegionId(RegionId),
    VowId(VowId),
    AllocSize {
        size: u32,
        align: u32,
    },
    FieldIndex(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    I32,
    I64,
    F32,
    F64,
    Bool,
    Unit,
    Ptr,
    LinearPtr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Inst {
    pub id: InstId,
    pub opcode: Opcode,
    pub ty: Ty,
    pub args: Vec<InstId>,
    pub data: InstData,
    pub origin: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BasicBlock {
    pub id: BlockId,
    pub insts: Vec<Inst>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VowEntry {
    pub id: VowId,
    pub description: String,
    pub blame: Blame,
    pub bindings: Vec<(String, InstId)>,
    pub file: String,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub id: FuncId,
    pub name: String,
    pub params: Vec<Ty>,
    pub param_names: Vec<String>,
    pub return_ty: Ty,
    pub effects: Vec<Effect>,
    pub vows: Vec<VowEntry>,
    pub blocks: Vec<BasicBlock>,
    pub local_names: std::collections::HashMap<u32, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldLayout {
    pub name: String,
    pub ty: Ty,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructLayout {
    pub name: String,
    pub fields: Vec<FieldLayout>,
    pub is_linear: bool,
}

impl StructLayout {
    pub fn size_bytes(&self) -> u32 {
        (self.fields.len() as u32) * 8
    }

    pub fn field_index(&self, field_name: &str) -> Option<u32> {
        self.fields
            .iter()
            .position(|f| f.name == field_name)
            .map(|i| i as u32)
    }

    pub fn field_ty(&self, idx: u32) -> Option<Ty> {
        self.fields.get(idx as usize).map(|f| f.ty)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariantLayout {
    pub name: String,
    pub tag: u64,
    pub payload: Vec<FieldLayout>,
}

impl VariantLayout {
    pub fn payload_size_bytes(&self) -> u32 {
        (self.payload.len() as u32) * 8
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumLayout {
    pub name: String,
    pub variants: Vec<VariantLayout>,
}

impl EnumLayout {
    pub fn size_bytes(&self) -> u32 {
        let max_payload = self
            .variants
            .iter()
            .map(|v| v.payload.len())
            .max()
            .unwrap_or(0);
        (1 + max_payload as u32) * 8
    }

    pub fn variant_index(&self, variant_name: &str) -> Option<u32> {
        self.variants
            .iter()
            .position(|v| v.name == variant_name)
            .map(|i| i as u32)
    }

    pub fn variant_tag(&self, variant_name: &str) -> Option<u64> {
        self.variants
            .iter()
            .find(|v| v.name == variant_name)
            .map(|v| v.tag)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub name: String,
    pub functions: Vec<Function>,
    pub strings: Vec<String>,
    pub struct_layouts: Vec<StructLayout>,
    pub enum_layouts: Vec<EnumLayout>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_span() -> Span {
        Span::new(0, 0)
    }

    #[test]
    fn inst_id_newtype_wrapping() {
        let a = InstId(0);
        let b = InstId(1);
        let c = InstId(0);
        assert_ne!(a, b);
        assert_eq!(a, c);
        assert_eq!(a.0, 0);
        assert!(a < b);
    }

    #[test]
    fn basic_inst_construction() {
        let inst = Inst {
            id: InstId(0),
            opcode: Opcode::ConstI32,
            ty: Ty::I32,
            args: vec![],
            data: InstData::ConstI32(42),
            origin: dummy_span(),
        };
        assert_eq!(inst.id, InstId(0));
        assert_eq!(inst.opcode, Opcode::ConstI32);
        assert_eq!(inst.ty, Ty::I32);
        assert!(inst.args.is_empty());
        assert_eq!(inst.data, InstData::ConstI32(42));
    }

    #[test]
    fn module_function_basicblock_construction() {
        let block = BasicBlock {
            id: BlockId(0),
            insts: vec![Inst {
                id: InstId(0),
                opcode: Opcode::ConstUnit,
                ty: Ty::Unit,
                args: vec![],
                data: InstData::None,
                origin: dummy_span(),
            }],
        };
        let func = Function {
            id: FuncId(0),
            name: "main".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::Unit,
            effects: vec![],
            vows: vec![],
            blocks: vec![block],
            local_names: std::collections::HashMap::new(),
        };
        let module = Module {
            name: "test_module".to_string(),
            functions: vec![func],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
        };
        assert_eq!(module.functions.len(), 1);
        assert_eq!(module.functions[0].blocks.len(), 1);
        assert_eq!(module.functions[0].blocks[0].insts.len(), 1);
    }

    #[test]
    fn simple_function_with_instructions() {
        let const_inst = Inst {
            id: InstId(0),
            opcode: Opcode::ConstI32,
            ty: Ty::I32,
            args: vec![],
            data: InstData::ConstI32(10),
            origin: dummy_span(),
        };
        let return_inst = Inst {
            id: InstId(1),
            opcode: Opcode::Return,
            ty: Ty::Unit,
            args: vec![InstId(0)],
            data: InstData::None,
            origin: dummy_span(),
        };
        let block = BasicBlock {
            id: BlockId(0),
            insts: vec![const_inst, return_inst],
        };
        let func = Function {
            id: FuncId(1),
            name: "answer".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I32,
            effects: vec![],
            vows: vec![],
            blocks: vec![block],
            local_names: std::collections::HashMap::new(),
        };
        assert_eq!(func.blocks[0].insts[0].data, InstData::ConstI32(10));
        assert_eq!(func.blocks[0].insts[1].args, vec![InstId(0)]);
    }

    #[test]
    fn ty_and_opcode_partial_eq() {
        assert_eq!(Ty::I32, Ty::I32);
        assert_ne!(Ty::I32, Ty::I64);
        assert_eq!(Opcode::ConstBool, Opcode::ConstBool);
        assert_ne!(Opcode::ConstBool, Opcode::ConstI32);
        assert_eq!(Ty::LinearPtr, Ty::LinearPtr);
        assert_ne!(Ty::Ptr, Ty::LinearPtr);
    }

    #[test]
    fn struct_layout_methods() {
        let layout = StructLayout {
            name: "Pair".to_string(),
            fields: vec![
                FieldLayout {
                    name: "x".to_string(),
                    ty: Ty::I64,
                },
                FieldLayout {
                    name: "y".to_string(),
                    ty: Ty::Bool,
                },
            ],
            is_linear: false,
        };
        assert_eq!(layout.size_bytes(), 16);
        assert_eq!(layout.field_index("x"), Some(0));
        assert_eq!(layout.field_index("y"), Some(1));
        assert_eq!(layout.field_index("z"), None);
        assert_eq!(layout.field_ty(0), Some(Ty::I64));
        assert_eq!(layout.field_ty(1), Some(Ty::Bool));
        assert_eq!(layout.field_ty(2), None);
    }

    #[test]
    fn variant_layout_payload_size() {
        let empty = VariantLayout {
            name: "None".to_string(),
            tag: 0,
            payload: vec![],
        };
        assert_eq!(empty.payload_size_bytes(), 0);
        let with_payload = VariantLayout {
            name: "Some".to_string(),
            tag: 1,
            payload: vec![FieldLayout {
                name: "v".to_string(),
                ty: Ty::I64,
            }],
        };
        assert_eq!(with_payload.payload_size_bytes(), 8);
    }

    #[test]
    fn enum_layout_methods() {
        let layout = EnumLayout {
            name: "Option".to_string(),
            variants: vec![
                VariantLayout {
                    name: "None".to_string(),
                    tag: 0,
                    payload: vec![],
                },
                VariantLayout {
                    name: "Some".to_string(),
                    tag: 1,
                    payload: vec![FieldLayout {
                        name: "v".to_string(),
                        ty: Ty::I64,
                    }],
                },
            ],
        };
        assert_eq!(layout.size_bytes(), 16); // (1 discriminant + 1 payload field) * 8
        assert_eq!(layout.variant_index("None"), Some(0));
        assert_eq!(layout.variant_index("Some"), Some(1));
        assert_eq!(layout.variant_index("Other"), None);
        assert_eq!(layout.variant_tag("None"), Some(0));
        assert_eq!(layout.variant_tag("Some"), Some(1));
        assert_eq!(layout.variant_tag("Other"), None);
    }

    #[test]
    fn branch_inst_data() {
        let branch = Inst {
            id: InstId(5),
            opcode: Opcode::Branch,
            ty: Ty::Unit,
            args: vec![InstId(3)],
            data: InstData::BranchTargets {
                then_block: BlockId(1),
                else_block: BlockId(2),
            },
            origin: dummy_span(),
        };
        match branch.data {
            InstData::BranchTargets {
                then_block,
                else_block,
            } => {
                assert_eq!(then_block, BlockId(1));
                assert_eq!(else_block, BlockId(2));
            }
            _ => panic!("expected BranchTargets"),
        }
    }
}
