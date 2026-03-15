use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub name: String,
    pub uses: Vec<UseDecl>,
    pub items: Vec<Item>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UseDecl {
    pub path: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Fn(FnDef),
    Struct(StructDef),
    Enum(EnumDef),
    Trait(TraitDef),
    Impl(ImplBlock),
    TypeAlias(TypeAlias),
    Extern(ExternBlock),
    Const(ConstDef),
}

impl Item {
    pub fn span(&self) -> Span {
        match self {
            Item::Fn(f) => f.span,
            Item::Struct(s) => s.span,
            Item::Enum(e) => e.span,
            Item::Trait(t) => t.span,
            Item::Impl(i) => i.span,
            Item::TypeAlias(t) => t.span,
            Item::Extern(e) => e.span,
            Item::Const(c) => c.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConstDef {
    pub vis: Visibility,
    pub name: String,
    pub ty: Type,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnDef {
    pub vis: Visibility,
    pub name: String,
    pub params: Vec<Param>,
    pub return_ty: Type,
    pub effects: Vec<Effect>,
    pub vow: Option<VowBlock>,
    pub body: Block,
    pub span: Span,
    pub is_declaration: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub refinement: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Visibility {
    Public,
    Private,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Effect {
    IO,
    Panic,
    Read,
    Unsafe,
    Write,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VowBlock {
    pub clauses: Vec<VowClause>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VowClause {
    Requires { expr: Expr, span: Span },
    Ensures { expr: Expr, span: Span },
    Invariant { expr: Expr, span: Span },
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDef {
    pub vis: Visibility,
    pub is_linear: bool,
    pub name: String,
    pub fields: Vec<FieldDef>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDef {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef {
    pub vis: Visibility,
    pub name: String,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    pub name: String,
    pub kind: VariantKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VariantKind {
    Unit,
    Tuple(Vec<Type>),
    Struct(Vec<FieldDef>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraitDef {
    pub vis: Visibility,
    pub name: String,
    pub methods: Vec<TraitMethod>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraitMethod {
    pub name: String,
    pub params: Vec<Param>,
    pub return_ty: Type,
    pub effects: Vec<Effect>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImplBlock {
    pub trait_name: Option<String>,
    pub self_ty: Type,
    pub methods: Vec<FnDef>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeAlias {
    pub vis: Visibility,
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExternBlock {
    pub vow: Option<VowBlock>,
    pub fns: Vec<ExternFn>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExternFn {
    pub name: String,
    pub params: Vec<Param>,
    pub return_ty: Type,
    pub effects: Vec<Effect>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Named {
        name: String,
        span: Span,
    },
    Generic {
        name: String,
        args: Vec<Type>,
        span: Span,
    },
    Refinement {
        binding: String,
        base: Box<Type>,
        predicate: Box<Expr>,
        span: Span,
    },
    Reference {
        inner: Box<Type>,
        span: Span,
    },
    Slice {
        inner: Box<Type>,
        span: Span,
    },
    Tuple {
        elems: Vec<Type>,
        span: Span,
    },
    Unit {
        span: Span,
    },
    Never {
        span: Span,
    },
}

impl Type {
    pub fn span(&self) -> Span {
        match self {
            Type::Named { span, .. } => *span,
            Type::Generic { span, .. } => *span,
            Type::Refinement { span, .. } => *span,
            Type::Reference { span, .. } => *span,
            Type::Slice { span, .. } => *span,
            Type::Tuple { span, .. } => *span,
            Type::Unit { span } => *span,
            Type::Never { span } => *span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Lit(Lit),
    Ident(String),
    BinaryOp {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    UnaryOp {
        op: UnOp,
        operand: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    MethodCall {
        receiver: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    FieldAccess {
        base: Box<Expr>,
        field: String,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    If {
        condition: Box<Expr>,
        then_branch: Box<Block>,
        else_branch: Option<Box<Expr>>,
    },
    While {
        condition: Box<Expr>,
        vow: Option<VowBlock>,
        body: Box<Block>,
    },
    Loop {
        vow: Option<VowBlock>,
        body: Box<Block>,
    },
    Break {
        value: Option<Box<Expr>>,
    },
    Return {
        value: Option<Box<Expr>>,
    },
    Block(Box<Block>),
    Borrow {
        expr: Box<Expr>,
    },
    Question {
        expr: Box<Expr>,
    },
    Assign {
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Tuple(Vec<Expr>),
    Result,
    StructLiteral {
        name: String,
        fields: Vec<(String, Expr)>,
    },
    EnumConstruct {
        path: Vec<String>,
        fields: Vec<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    Int(i128),
    Float(f64),
    Bool(bool),
    String(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    AddChecked,
    SubChecked,
    MulChecked,
    DivChecked,
    RemChecked,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitXor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub trailing_expr: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        pattern: Pat,
        ty: Option<Type>,
        init: Box<Expr>,
        span: Span,
    },
    Expr {
        expr: Expr,
        has_semicolon: bool,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pat,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pat {
    pub kind: PatKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatKind {
    Wildcard,
    Ident {
        name: String,
        is_mut: bool,
    },
    Lit(Lit),
    Tuple(Vec<Pat>),
    Struct {
        name: String,
        fields: Vec<(String, Pat)>,
    },
    EnumVariant {
        path: Vec<String>,
        inner: Vec<Pat>,
    },
    Or(Vec<Pat>),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_span() -> Span {
        Span::new(0, 1)
    }

    #[test]
    fn item_span_delegation() {
        let fn_def = FnDef {
            vis: Visibility::Public,
            name: "foo".to_string(),
            params: vec![],
            return_ty: Type::Unit { span: dummy_span() },
            effects: vec![],
            vow: None,
            body: Block {
                stmts: vec![],
                trailing_expr: None,
                span: dummy_span(),
            },
            span: Span::new(10, 20),
            is_declaration: false,
        };
        let item = Item::Fn(fn_def);
        assert_eq!(item.span(), Span::new(10, 20));
    }

    #[test]
    fn type_span_delegation() {
        let ty = Type::Named {
            name: "i32".to_string(),
            span: Span::new(5, 3),
        };
        assert_eq!(ty.span(), Span::new(5, 3));
    }

    #[test]
    fn vow_clause_variants() {
        let span = dummy_span();
        let expr = Expr {
            kind: ExprKind::Lit(Lit::Bool(true)),
            span,
        };
        let requires = VowClause::Requires {
            expr: expr.clone(),
            span,
        };
        let ensures = VowClause::Ensures {
            expr: expr.clone(),
            span,
        };
        let invariant = VowClause::Invariant { expr, span };
        let _block = VowBlock {
            clauses: vec![requires, ensures, invariant],
            span,
        };
    }

    #[test]
    fn linear_struct() {
        let s = StructDef {
            vis: Visibility::Public,
            is_linear: true,
            name: "FileHandle".to_string(),
            fields: vec![],
            span: dummy_span(),
        };
        assert!(s.is_linear);
    }

    #[test]
    fn binary_op_variants_all_present() {
        let ops = [
            BinOp::Add,
            BinOp::Sub,
            BinOp::Mul,
            BinOp::Div,
            BinOp::Rem,
            BinOp::AddChecked,
            BinOp::SubChecked,
            BinOp::MulChecked,
            BinOp::Eq,
            BinOp::Ne,
            BinOp::Lt,
            BinOp::Le,
            BinOp::Gt,
            BinOp::Ge,
            BinOp::And,
            BinOp::Or,
        ];
        assert_eq!(ops.len(), 16);
    }
}
