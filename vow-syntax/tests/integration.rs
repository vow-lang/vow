use vow_syntax::{
    ast::{
        Block, ConstDef, EnumDef, EnumVariant, Expr, ExprKind, ExternBlock, ExternFn, FieldDef,
        FnDef, ImplBlock, Item, MatchArm, Module, Param, Pat, PatKind, Stmt, StructDef, TraitDef,
        TraitMethod, Type, TypeAlias, UseDecl, VariantKind, VowBlock, VowClause,
    },
    parser::parse_module,
    printer::print_module,
    span::Span,
};

fn z() -> Span {
    Span::new(0, 0)
}

fn strip_type(ty: Type) -> Type {
    match ty {
        Type::Named { name, .. } => Type::Named { name, span: z() },
        Type::Generic { name, args, .. } => Type::Generic {
            name,
            args: args.into_iter().map(strip_type).collect(),
            span: z(),
        },
        Type::Refinement {
            binding,
            base,
            predicate,
            ..
        } => Type::Refinement {
            binding,
            base: Box::new(strip_type(*base)),
            predicate: Box::new(strip_expr(*predicate)),
            span: z(),
        },
        Type::Reference { inner, .. } => Type::Reference {
            inner: Box::new(strip_type(*inner)),
            span: z(),
        },
        Type::Slice { inner, .. } => Type::Slice {
            inner: Box::new(strip_type(*inner)),
            span: z(),
        },
        Type::Tuple { elems, .. } => Type::Tuple {
            elems: elems.into_iter().map(strip_type).collect(),
            span: z(),
        },
        Type::Unit { .. } => Type::Unit { span: z() },
        Type::Never { .. } => Type::Never { span: z() },
    }
}

fn strip_expr(expr: Expr) -> Expr {
    let kind = match expr.kind {
        ExprKind::Lit(l) => ExprKind::Lit(l),
        ExprKind::Ident(s) => ExprKind::Ident(s),
        ExprKind::Result => ExprKind::Result,
        ExprKind::BinaryOp { op, lhs, rhs } => ExprKind::BinaryOp {
            op,
            lhs: Box::new(strip_expr(*lhs)),
            rhs: Box::new(strip_expr(*rhs)),
        },
        ExprKind::UnaryOp { op, operand } => ExprKind::UnaryOp {
            op,
            operand: Box::new(strip_expr(*operand)),
        },
        ExprKind::Call { callee, args } => ExprKind::Call {
            callee: Box::new(strip_expr(*callee)),
            args: args.into_iter().map(strip_expr).collect(),
        },
        ExprKind::MethodCall {
            receiver,
            method,
            args,
        } => ExprKind::MethodCall {
            receiver: Box::new(strip_expr(*receiver)),
            method,
            args: args.into_iter().map(strip_expr).collect(),
        },
        ExprKind::FieldAccess { base, field } => ExprKind::FieldAccess {
            base: Box::new(strip_expr(*base)),
            field,
        },
        ExprKind::Index { base, index } => ExprKind::Index {
            base: Box::new(strip_expr(*base)),
            index: Box::new(strip_expr(*index)),
        },
        ExprKind::Match { scrutinee, arms } => ExprKind::Match {
            scrutinee: Box::new(strip_expr(*scrutinee)),
            arms: arms.into_iter().map(strip_match_arm).collect(),
        },
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => ExprKind::If {
            condition: Box::new(strip_expr(*condition)),
            then_branch: Box::new(strip_block(*then_branch)),
            else_branch: else_branch.map(|e| Box::new(strip_expr(*e))),
        },
        ExprKind::While {
            condition,
            vow,
            body,
        } => ExprKind::While {
            condition: Box::new(strip_expr(*condition)),
            vow: vow.map(strip_vow_block),
            body: Box::new(strip_block(*body)),
        },
        ExprKind::ForEach {
            binding,
            iterable,
            vow,
            body,
        } => ExprKind::ForEach {
            binding,
            iterable: Box::new(strip_expr(*iterable)),
            vow: vow.map(strip_vow_block),
            body: Box::new(strip_block(*body)),
        },
        ExprKind::Loop { vow, body } => ExprKind::Loop {
            vow: vow.map(strip_vow_block),
            body: Box::new(strip_block(*body)),
        },
        ExprKind::Break { value } => ExprKind::Break {
            value: value.map(|e| Box::new(strip_expr(*e))),
        },
        ExprKind::Return { value } => ExprKind::Return {
            value: value.map(|e| Box::new(strip_expr(*e))),
        },
        ExprKind::Block(b) => ExprKind::Block(Box::new(strip_block(*b))),
        ExprKind::Borrow { expr } => ExprKind::Borrow {
            expr: Box::new(strip_expr(*expr)),
        },
        ExprKind::Question { expr } => ExprKind::Question {
            expr: Box::new(strip_expr(*expr)),
        },
        ExprKind::Assign { lhs, rhs } => ExprKind::Assign {
            lhs: Box::new(strip_expr(*lhs)),
            rhs: Box::new(strip_expr(*rhs)),
        },
        ExprKind::Tuple(elems) => ExprKind::Tuple(elems.into_iter().map(strip_expr).collect()),
        ExprKind::StructLiteral { name, fields } => ExprKind::StructLiteral {
            name,
            fields: fields
                .into_iter()
                .map(|(n, e)| (n, strip_expr(e)))
                .collect(),
        },
        ExprKind::EnumConstruct { path, fields } => ExprKind::EnumConstruct {
            path,
            fields: fields.into_iter().map(strip_expr).collect(),
        },
        ExprKind::Cast { expr, target_ty } => ExprKind::Cast {
            expr: Box::new(strip_expr(*expr)),
            target_ty: Box::new(strip_type(*target_ty)),
        },
    };
    Expr { kind, span: z() }
}

fn strip_match_arm(arm: MatchArm) -> MatchArm {
    MatchArm {
        pattern: strip_pat(arm.pattern),
        body: strip_expr(arm.body),
        span: z(),
    }
}

fn strip_pat(pat: Pat) -> Pat {
    let kind = match pat.kind {
        PatKind::Wildcard => PatKind::Wildcard,
        PatKind::Ident { name, is_mut } => PatKind::Ident { name, is_mut },
        PatKind::Lit(l) => PatKind::Lit(l),
        PatKind::Tuple(pats) => PatKind::Tuple(pats.into_iter().map(strip_pat).collect()),
        PatKind::Struct { name, fields } => PatKind::Struct {
            name,
            fields: fields.into_iter().map(|(n, p)| (n, strip_pat(p))).collect(),
        },
        PatKind::EnumVariant { path, inner } => PatKind::EnumVariant {
            path,
            inner: inner.into_iter().map(strip_pat).collect(),
        },
        PatKind::Or(pats) => PatKind::Or(pats.into_iter().map(strip_pat).collect()),
    };
    Pat { kind, span: z() }
}

fn strip_stmt(stmt: Stmt) -> Stmt {
    match stmt {
        Stmt::Let {
            pattern, ty, init, ..
        } => Stmt::Let {
            pattern: strip_pat(pattern),
            ty: ty.map(strip_type),
            init: Box::new(strip_expr(*init)),
            span: z(),
        },
        Stmt::Expr {
            expr,
            has_semicolon,
            ..
        } => Stmt::Expr {
            expr: strip_expr(expr),
            has_semicolon,
            span: z(),
        },
    }
}

fn strip_block(block: Block) -> Block {
    Block {
        stmts: block.stmts.into_iter().map(strip_stmt).collect(),
        trailing_expr: block.trailing_expr.map(|e| Box::new(strip_expr(*e))),
        span: z(),
    }
}

fn strip_vow_block(vow: VowBlock) -> VowBlock {
    VowBlock {
        clauses: vow.clauses.into_iter().map(strip_vow_clause).collect(),
        span: z(),
    }
}

fn strip_vow_clause(clause: VowClause) -> VowClause {
    match clause {
        VowClause::Requires { expr, .. } => VowClause::Requires {
            expr: strip_expr(expr),
            span: z(),
        },
        VowClause::Ensures { expr, .. } => VowClause::Ensures {
            expr: strip_expr(expr),
            span: z(),
        },
        VowClause::Invariant { expr, .. } => VowClause::Invariant {
            expr: strip_expr(expr),
            span: z(),
        },
    }
}

fn strip_param(p: Param) -> Param {
    Param {
        name: p.name,
        ty: strip_type(p.ty),
        refinement: p.refinement.map(|e| Box::new(strip_expr(*e))),
        span: z(),
    }
}

fn strip_field_def(f: FieldDef) -> FieldDef {
    FieldDef {
        name: f.name,
        ty: strip_type(f.ty),
        span: z(),
    }
}

fn strip_fn_def(f: FnDef) -> FnDef {
    FnDef {
        vis: f.vis,
        name: f.name,
        params: f.params.into_iter().map(strip_param).collect(),
        return_ty: strip_type(f.return_ty),
        effects: f.effects,
        vow: f.vow.map(strip_vow_block),
        body: strip_block(f.body),
        span: z(),
        is_declaration: f.is_declaration,
    }
}

fn strip_struct_def(s: StructDef) -> StructDef {
    StructDef {
        vis: s.vis,
        is_linear: s.is_linear,
        name: s.name,
        fields: s.fields.into_iter().map(strip_field_def).collect(),
        span: z(),
    }
}

fn strip_enum_variant(v: EnumVariant) -> EnumVariant {
    let kind = match v.kind {
        VariantKind::Unit => VariantKind::Unit,
        VariantKind::Tuple(types) => {
            VariantKind::Tuple(types.into_iter().map(strip_type).collect())
        }
        VariantKind::Struct(fields) => {
            VariantKind::Struct(fields.into_iter().map(strip_field_def).collect())
        }
    };
    EnumVariant {
        name: v.name,
        kind,
        span: z(),
    }
}

fn strip_enum_def(e: EnumDef) -> EnumDef {
    EnumDef {
        vis: e.vis,
        name: e.name,
        variants: e.variants.into_iter().map(strip_enum_variant).collect(),
        span: z(),
    }
}

fn strip_trait_method(m: TraitMethod) -> TraitMethod {
    TraitMethod {
        name: m.name,
        params: m.params.into_iter().map(strip_param).collect(),
        return_ty: strip_type(m.return_ty),
        effects: m.effects,
        span: z(),
    }
}

fn strip_trait_def(t: TraitDef) -> TraitDef {
    TraitDef {
        vis: t.vis,
        name: t.name,
        methods: t.methods.into_iter().map(strip_trait_method).collect(),
        span: z(),
    }
}

fn strip_impl_block(i: ImplBlock) -> ImplBlock {
    ImplBlock {
        trait_name: i.trait_name,
        self_ty: strip_type(i.self_ty),
        methods: i.methods.into_iter().map(strip_fn_def).collect(),
        span: z(),
    }
}

fn strip_type_alias(t: TypeAlias) -> TypeAlias {
    TypeAlias {
        vis: t.vis,
        name: t.name,
        ty: strip_type(t.ty),
        span: z(),
    }
}

fn strip_extern_fn(f: ExternFn) -> ExternFn {
    ExternFn {
        name: f.name,
        params: f.params.into_iter().map(strip_param).collect(),
        return_ty: strip_type(f.return_ty),
        effects: f.effects,
        span: z(),
    }
}

fn strip_extern_block(e: ExternBlock) -> ExternBlock {
    ExternBlock {
        vow: e.vow.map(strip_vow_block),
        fns: e.fns.into_iter().map(strip_extern_fn).collect(),
        span: z(),
    }
}

fn strip_item(item: Item) -> Item {
    match item {
        Item::Fn(f) => Item::Fn(strip_fn_def(f)),
        Item::Struct(s) => Item::Struct(strip_struct_def(s)),
        Item::Enum(e) => Item::Enum(strip_enum_def(e)),
        Item::Trait(t) => Item::Trait(strip_trait_def(t)),
        Item::Impl(i) => Item::Impl(strip_impl_block(i)),
        Item::TypeAlias(t) => Item::TypeAlias(strip_type_alias(t)),
        Item::Extern(e) => Item::Extern(strip_extern_block(e)),
        Item::Const(c) => Item::Const(ConstDef {
            vis: c.vis,
            name: c.name,
            ty: strip_type(c.ty),
            value: strip_expr(c.value),
            span: z(),
        }),
    }
}

fn strip_use_decl(u: UseDecl) -> UseDecl {
    UseDecl {
        path: u.path,
        span: z(),
    }
}

fn strip_module(m: Module) -> Module {
    Module {
        name: m.name,
        uses: m.uses.into_iter().map(strip_use_decl).collect(),
        items: m.items.into_iter().map(strip_item).collect(),
        span: z(),
    }
}

fn roundtrip(src: &str) {
    let (ast1, diags1) = parse_module(src, "<test>");
    assert!(
        diags1.is_empty(),
        "parse errors on first parse: {:?}",
        diags1.iter().map(|e| &e.message).collect::<Vec<_>>()
    );
    let printed1 = print_module(&ast1);
    let (ast2, diags2) = parse_module(&printed1, "<test>");
    assert!(
        diags2.is_empty(),
        "parse errors on second parse: {:?}\nsource was:\n{}",
        diags2.iter().map(|e| &e.message).collect::<Vec<_>>(),
        printed1
    );
    let printed2 = print_module(&ast2);
    assert_eq!(strip_module(ast1), strip_module(ast2), "AST not idempotent");
    assert_eq!(printed1, printed2, "printed form not idempotent");
}

#[test]
fn binary_search_roundtrip() {
    roundtrip(BINARY_SEARCH_SOURCE);
}

const BINARY_SEARCH_SOURCE: &str = "module BinarySearch\n\nfn binary_search(slice: [i32], target: i32) -> i32 [panic] vow {\n    requires: slice.len() >= 0\n    ensures: result >= -1\n} {\n    let mut lo: i32 = 0;\n    let mut hi: i32 = slice.len();\n    while lo < hi vow {\n        invariant: lo >= 0 && hi <= slice.len()\n    } {\n        let mut mid: i32 = lo + (hi - lo) / 2;\n        if slice[mid] == target {\n            return mid;\n        } else {\n            if slice[mid] < target {\n                lo = mid + 1;\n            } else {\n                hi = mid;\n            }\n        }\n    }\n    return -1;\n}\n";

#[test]
fn struct_literal_roundtrip() {
    roundtrip(STRUCT_LITERAL_SOURCE);
}

const STRUCT_LITERAL_SOURCE: &str = "\
module StructTest

struct Point {
    x: i64,
    y: i64,
}

pub fn make_point() -> i32 {
    let p = Point { x: 1, y: 2 };
    0
}
";

#[test]
fn enum_construct_roundtrip() {
    roundtrip(ENUM_CONSTRUCT_SOURCE);
}

const ENUM_CONSTRUCT_SOURCE: &str = "\
module EnumTest

enum Color {
    Red,
    Green,
    Blue,
}

pub fn main() -> i32 {
    let c = Color::Red;
    let s = Option::Some(42);
    let n = Option::None;
    0
}
";

#[test]
fn match_on_enum_roundtrip() {
    roundtrip(MATCH_ON_ENUM_SOURCE);
}

const MATCH_ON_ENUM_SOURCE: &str = "\
module MatchTest

enum Shape {
    Circle,
    Square,
}

pub fn describe(s: Shape) -> i32 {
    match s {
        Shape::Circle => 1,
        Shape::Square => 2,
    }
}
";

#[test]
fn where_clause_roundtrip() {
    roundtrip(WHERE_CLAUSE_SOURCE);
}

const WHERE_CLAUSE_SOURCE: &str = "\
module WhereTest

fn divide(x: i64, y: i64 where y != 0) -> i64 {
    x / y
}
";

#[test]
fn where_clause_with_vow_block_roundtrip() {
    roundtrip(WHERE_CLAUSE_WITH_VOW_SOURCE);
}

const WHERE_CLAUSE_WITH_VOW_SOURCE: &str = "\
module WhereVowTest

fn safe_divide(x: i64, y: i64 where y != 0) -> i64 vow {
    ensures: result * y <= x
} {
    x / y
}
";

#[test]
fn multiple_where_clauses_roundtrip() {
    roundtrip(MULTIPLE_WHERE_SOURCE);
}

const MULTIPLE_WHERE_SOURCE: &str = "\
module MultiWhereTest

fn clamp(x: i64 where x >= 0, max: i64 where max > 0) -> i64 {
    if x > max {
        max
    } else {
        x
    }
}
";
