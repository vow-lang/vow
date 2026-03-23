mod proptest_arb;

use proptest::prelude::*;
use vow_syntax::ast::*;
use vow_syntax::parser::parse_module;
use vow_syntax::printer::print_module;
use vow_syntax::span::Span;

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

fn strip_item(item: Item) -> Item {
    match item {
        Item::Fn(f) => Item::Fn(strip_fn_def(f)),
        Item::Struct(s) => Item::Struct(strip_struct_def(s)),
        Item::Enum(e) => Item::Enum(strip_enum_def(e)),
        Item::Trait(t) => Item::Trait(t),
        Item::Impl(i) => Item::Impl(i),
        Item::TypeAlias(t) => Item::TypeAlias(t),
        Item::Extern(e) => Item::Extern(e),
        Item::Const(c) => Item::Const(c),
    }
}

fn strip_module(m: Module) -> Module {
    Module {
        name: m.name,
        uses: m.uses,
        items: m.items.into_iter().map(strip_item).collect(),
        span: z(),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn print_parse_roundtrip(module in proptest_arb::arb_module()) {
        let printed1 = print_module(&module);

        // The printed source must parse without errors
        let (parsed, diags) = parse_module(&printed1, "<proptest>");
        prop_assert!(
            diags.is_empty(),
            "Parse errors on generated source:\n{}\nErrors: {:?}",
            printed1,
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );

        // Print again — must be identical (idempotency)
        let printed2 = print_module(&parsed);
        prop_assert_eq!(
            &printed1,
            &printed2,
            "Printed form not idempotent.\nFirst:\n{}\nSecond:\n{}",
            printed1,
            printed2
        );

        // ASTs must be structurally equal (ignoring spans)
        let stripped1 = strip_module(module);
        let stripped2 = strip_module(parsed);
        prop_assert_eq!(
            stripped1,
            stripped2,
            "AST not equal after roundtrip.\nSource:\n{}",
            printed1
        );
    }

    #[test]
    fn fn_def_roundtrip(f in proptest_arb::arb_fn_def()) {
        let module = Module {
            name: "Test".to_string(),
            uses: vec![],
            items: vec![Item::Fn(f)],
            span: z(),
        };
        let printed = print_module(&module);
        let (_, diags) = parse_module(&printed, "<proptest>");
        prop_assert!(
            diags.is_empty(),
            "Parse errors:\n{}\nErrors: {:?}",
            printed,
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn struct_def_roundtrip(s in proptest_arb::arb_struct_def()) {
        let module = Module {
            name: "Test".to_string(),
            uses: vec![],
            items: vec![Item::Struct(s)],
            span: z(),
        };
        let printed = print_module(&module);
        let (_, diags) = parse_module(&printed, "<proptest>");
        prop_assert!(
            diags.is_empty(),
            "Parse errors:\n{}\nErrors: {:?}",
            printed,
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn enum_def_roundtrip(e in proptest_arb::arb_enum_def()) {
        let module = Module {
            name: "Test".to_string(),
            uses: vec![],
            items: vec![Item::Enum(e)],
            span: z(),
        };
        let printed = print_module(&module);
        let (_, diags) = parse_module(&printed, "<proptest>");
        prop_assert!(
            diags.is_empty(),
            "Parse errors:\n{}\nErrors: {:?}",
            printed,
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn lexer_never_panics(input in ".*") {
        let lexer = vow_syntax::lexer::Lexer::new(&input);
        let _ = lexer.tokenize();
    }

    #[test]
    fn parser_never_panics(input in "module [A-Z][a-z]*\n(fn [a-z]+ *\\( *\\) *-> *i64 *\\{ *[0-9]+ *\\}\n?)*") {
        let (_, _) = parse_module(&input, "<fuzz>");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn expr_in_fn_roundtrip(expr in proptest_arb::arb_expr()) {
        let module = Module {
            name: "Test".to_string(),
            uses: vec![],
            items: vec![Item::Fn(FnDef {
                vis: Visibility::Private,
                name: "test_fn".to_string(),
                params: vec![],
                return_ty: Type::Unit { span: z() },
                effects: vec![],
                vow: None,
                body: Block {
                    stmts: vec![],
                    trailing_expr: Some(Box::new(expr)),
                    span: z(),
                },
                span: z(),
                is_declaration: false,
            })],
            span: z(),
        };

        let printed1 = print_module(&module);
        let (parsed, diags) = parse_module(&printed1, "<proptest>");

        prop_assert!(
            diags.is_empty(),
            "Parse errors on expr roundtrip:\n{}\nErrors: {:?}",
            printed1,
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );

        let printed2 = print_module(&parsed);
        prop_assert_eq!(
            &printed1, &printed2,
            "Expression print not idempotent.\nSource:\n{}",
            printed1
        );
    }
}
