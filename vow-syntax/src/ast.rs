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
    ForEach {
        binding: String,
        iterable: Box<Expr>,
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
    Continue,
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
    Cast {
        expr: Box<Expr>,
        target_ty: Box<Type>,
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
    Int(u128),
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
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
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

/// Return values carried by breaks that target the loop owning `block`.
/// Nested loop bodies are excluded because an unlabeled break targets the
/// innermost loop. Conditions and iterables are included because they execute
/// before their nested loop becomes the active break target.
pub fn loop_break_values(block: &Block) -> Vec<&Expr> {
    fn push_block_exprs<'a>(block: &'a Block, pending: &mut Vec<&'a Expr>) {
        for stmt in &block.stmts {
            pending.push(match stmt {
                Stmt::Let { init, .. } => init,
                Stmt::Expr { expr, .. } => expr,
            });
        }
        if let Some(expr) = &block.trailing_expr {
            pending.push(expr);
        }
    }

    let mut pending = Vec::new();
    let mut values = Vec::new();
    push_block_exprs(block, &mut pending);
    while let Some(expr) = pending.pop() {
        match &expr.kind {
            ExprKind::Break { value: Some(value) } => {
                values.push(value.as_ref());
                pending.push(value);
            }
            ExprKind::Loop { .. }
            | ExprKind::Break { value: None }
            | ExprKind::Continue
            | ExprKind::Lit(_)
            | ExprKind::Ident(_)
            | ExprKind::Result => {}
            ExprKind::While { condition, .. } => pending.push(condition),
            ExprKind::ForEach { iterable, .. } => pending.push(iterable),
            ExprKind::BinaryOp { lhs, rhs, .. }
            | ExprKind::Index {
                base: lhs,
                index: rhs,
            }
            | ExprKind::Assign { lhs, rhs } => pending.extend([lhs.as_ref(), rhs.as_ref()]),
            ExprKind::UnaryOp { operand, .. }
            | ExprKind::Borrow { expr: operand }
            | ExprKind::Question { expr: operand }
            | ExprKind::Cast { expr: operand, .. }
            | ExprKind::FieldAccess { base: operand, .. } => pending.push(operand),
            ExprKind::Call { callee, args } => {
                pending.push(callee);
                pending.extend(args);
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                pending.push(receiver);
                pending.extend(args);
            }
            ExprKind::Match { scrutinee, arms } => {
                pending.push(scrutinee);
                pending.extend(arms.iter().map(|arm| &arm.body));
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                pending.push(condition);
                push_block_exprs(then_branch, &mut pending);
                if let Some(else_expr) = else_branch {
                    pending.push(else_expr);
                }
            }
            ExprKind::Return { value: Some(value) } => pending.push(value),
            ExprKind::Return { value: None } => {}
            ExprKind::Block(block) => push_block_exprs(block, &mut pending),
            ExprKind::Tuple(items) | ExprKind::EnumConstruct { fields: items, .. } => {
                pending.extend(items);
            }
            ExprKind::StructLiteral { fields, .. } => {
                pending.extend(fields.iter().map(|(_, value)| value));
            }
        }
    }
    values
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

    fn expr(kind: ExprKind) -> Expr {
        Expr {
            kind,
            span: dummy_span(),
        }
    }

    fn break_value(value: u128) -> Expr {
        expr(ExprKind::Break {
            value: Some(Box::new(expr(ExprKind::Lit(Lit::Int(value))))),
        })
    }

    fn block_with(expressions: Vec<Expr>) -> Block {
        Block {
            stmts: expressions
                .into_iter()
                .map(|expr| Stmt::Expr {
                    expr,
                    has_semicolon: true,
                    span: dummy_span(),
                })
                .collect(),
            trailing_expr: None,
            span: dummy_span(),
        }
    }

    #[test]
    fn loop_break_values_walks_expressions_but_stops_at_nested_loop_bodies() {
        let unit_ty = || Type::Unit { span: dummy_span() };
        let wildcard = || Pat {
            kind: PatKind::Wildcard,
            span: dummy_span(),
        };
        let body = block_with(vec![
            expr(ExprKind::BinaryOp {
                op: BinOp::Add,
                lhs: Box::new(break_value(1)),
                rhs: Box::new(break_value(2)),
            }),
            expr(ExprKind::UnaryOp {
                op: UnOp::Neg,
                operand: Box::new(break_value(3)),
            }),
            expr(ExprKind::Call {
                callee: Box::new(break_value(4)),
                args: vec![break_value(5)],
            }),
            expr(ExprKind::MethodCall {
                receiver: Box::new(break_value(6)),
                method: "m".into(),
                args: vec![break_value(7)],
            }),
            expr(ExprKind::FieldAccess {
                base: Box::new(break_value(8)),
                field: "f".into(),
            }),
            expr(ExprKind::Index {
                base: Box::new(break_value(9)),
                index: Box::new(break_value(10)),
            }),
            expr(ExprKind::Match {
                scrutinee: Box::new(break_value(11)),
                arms: vec![MatchArm {
                    pattern: wildcard(),
                    body: break_value(12),
                    span: dummy_span(),
                }],
            }),
            expr(ExprKind::If {
                condition: Box::new(break_value(13)),
                then_branch: Box::new(block_with(vec![break_value(14)])),
                else_branch: Some(Box::new(break_value(15))),
            }),
            expr(ExprKind::While {
                condition: Box::new(break_value(16)),
                vow: None,
                body: Box::new(block_with(vec![break_value(900)])),
            }),
            expr(ExprKind::ForEach {
                binding: "item".into(),
                iterable: Box::new(break_value(17)),
                vow: None,
                body: Box::new(block_with(vec![break_value(901)])),
            }),
            expr(ExprKind::Loop {
                vow: None,
                body: Box::new(block_with(vec![break_value(902)])),
            }),
            expr(ExprKind::Return {
                value: Some(Box::new(break_value(18))),
            }),
            expr(ExprKind::Block(Box::new(block_with(vec![break_value(19)])))),
            expr(ExprKind::Borrow {
                expr: Box::new(break_value(20)),
            }),
            expr(ExprKind::Question {
                expr: Box::new(break_value(21)),
            }),
            expr(ExprKind::Cast {
                expr: Box::new(break_value(22)),
                target_ty: Box::new(unit_ty()),
            }),
            expr(ExprKind::Assign {
                lhs: Box::new(break_value(23)),
                rhs: Box::new(break_value(24)),
            }),
            expr(ExprKind::Tuple(vec![break_value(25)])),
            expr(ExprKind::StructLiteral {
                name: "S".into(),
                fields: vec![("f".into(), break_value(26))],
            }),
            expr(ExprKind::EnumConstruct {
                path: vec!["E".into(), "V".into()],
                fields: vec![break_value(27)],
            }),
            break_value(28),
            expr(ExprKind::Break { value: None }),
            expr(ExprKind::Continue),
            expr(ExprKind::Return { value: None }),
            expr(ExprKind::Lit(Lit::Bool(true))),
            expr(ExprKind::Ident("x".into())),
            expr(ExprKind::Result),
        ]);

        let mut magnitudes: Vec<u128> = loop_break_values(&body)
            .into_iter()
            .filter_map(|value| match &value.kind {
                ExprKind::Lit(Lit::Int(magnitude)) => Some(*magnitude),
                _ => None,
            })
            .collect();
        magnitudes.sort_unstable();
        assert_eq!(magnitudes, (1..=28).collect::<Vec<_>>());
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
            BinOp::BitAnd,
            BinOp::BitOr,
            BinOp::BitXor,
            BinOp::Shl,
            BinOp::Shr,
        ];
        assert_eq!(ops.len(), 21);
    }
}
