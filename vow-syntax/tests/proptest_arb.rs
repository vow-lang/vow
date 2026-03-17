use proptest::prelude::*;
use vow_syntax::ast::*;
use vow_syntax::span::Span;

fn z() -> Span {
    Span::new(0, 0)
}

pub fn arb_ident() -> impl Strategy<Value = String> {
    prop::sample::select(&[
        "x", "y", "z", "a", "b", "c", "foo", "bar", "baz", "val", "tmp", "acc", "idx", "ptr",
        "lhs", "rhs", "cnt", "buf", "res", "ans", "item", "node", "data", "elem",
    ])
    .prop_map(|s| s.to_string())
}

pub fn arb_module_name() -> impl Strategy<Value = String> {
    prop::sample::select(&[
        "Main", "Test", "Foo", "Bar", "Module1", "MyMod", "Alpha", "Beta",
    ])
    .prop_map(|s| s.to_string())
}

pub fn arb_type_name() -> impl Strategy<Value = String> {
    prop::sample::select(&[
        "i8", "i16", "i32", "i64", "i128", "u8", "u16", "u32", "u64", "u128", "f32", "f64", "bool",
        "str",
    ])
    .prop_map(|s| s.to_string())
}

pub fn arb_user_type_name() -> impl Strategy<Value = String> {
    prop::sample::select(&[
        "Point", "Color", "Node", "Pair", "Entry", "State", "Config", "Result2",
    ])
    .prop_map(|s| s.to_string())
}

pub fn arb_type() -> impl Strategy<Value = Type> {
    arb_type_inner(3)
}

fn arb_type_inner(depth: u32) -> impl Strategy<Value = Type> {
    if depth == 0 {
        return arb_type_leaf().boxed();
    }
    prop_oneof![
        8 => arb_type_leaf(),
        // Only reference leaf types — nested `&&T` is lexed as `AmpAmp` not two `Amp` tokens
        1 => arb_type_leaf().prop_map(|inner| Type::Reference {
            inner: Box::new(inner),
            span: z(),
        }),
        1 => prop::collection::vec(arb_type_leaf(), 2..=3).prop_map(|elems| Type::Tuple {
            elems,
            span: z(),
        }),
    ]
    .boxed()
}

fn arb_type_leaf() -> impl Strategy<Value = Type> {
    prop_oneof![
        8 => arb_type_name().prop_map(|name| Type::Named { name, span: z() }),
        1 => Just(Type::Unit { span: z() }),
    ]
}

pub fn arb_lit() -> impl Strategy<Value = Lit> {
    prop_oneof![
        (0i128..1000).prop_map(Lit::Int),
        prop::bool::ANY.prop_map(Lit::Bool),
        prop::sample::select(&["hello", "world", "test", ""])
            .prop_map(|s| Lit::String(s.to_string())),
    ]
}

pub fn arb_expr() -> impl Strategy<Value = Expr> {
    arb_expr_inner(3)
}

fn arb_expr_inner(depth: u32) -> impl Strategy<Value = Expr> {
    if depth == 0 {
        return arb_expr_leaf().boxed();
    }
    // Note: if-expressions are only generated at the top level (trailing_expr),
    // not inside binary/unary ops, because `if x {} + y` is not parseable.
    prop_oneof![
        5 => arb_expr_leaf(),
        2 => arb_binop_expr(depth - 1),
        1 => arb_unop_expr(depth - 1),
        1 => arb_call_expr(depth - 1),
    ]
    .boxed()
}

fn arb_expr_or_if(depth: u32) -> impl Strategy<Value = Expr> {
    if depth == 0 {
        return arb_expr_leaf().boxed();
    }
    prop_oneof![
        5 => arb_expr_leaf(),
        2 => arb_binop_expr(depth - 1),
        1 => arb_unop_expr(depth - 1),
        1 => arb_if_expr(depth - 1),
        1 => arb_call_expr(depth - 1),
    ]
    .boxed()
}

fn arb_expr_leaf() -> impl Strategy<Value = Expr> {
    prop_oneof![
        3 => arb_lit().prop_map(|l| Expr { kind: ExprKind::Lit(l), span: z() }),
        2 => arb_ident().prop_map(|id| Expr { kind: ExprKind::Ident(id), span: z() }),
        // ExprKind::Result prints as `result` but only round-trips inside ensures clauses.
        // Outside vow blocks, the parser treats `result` as Ident("result"), not ExprKind::Result.
        // So we don't generate it here.
    ]
}

fn arb_binop() -> impl Strategy<Value = BinOp> {
    prop::sample::select(&[
        BinOp::Add,
        BinOp::Sub,
        BinOp::Mul,
        BinOp::Div,
        BinOp::Rem,
        BinOp::AddChecked,
        BinOp::SubChecked,
        BinOp::MulChecked,
        BinOp::DivChecked,
        BinOp::RemChecked,
        BinOp::Eq,
        BinOp::Ne,
        BinOp::Lt,
        BinOp::Le,
        BinOp::Gt,
        BinOp::Ge,
        BinOp::And,
        BinOp::Or,
        BinOp::BitXor,
    ])
}

fn arb_binop_expr(depth: u32) -> impl Strategy<Value = Expr> {
    (arb_binop(), arb_expr_inner(depth), arb_expr_inner(depth)).prop_map(|(op, lhs, rhs)| Expr {
        kind: ExprKind::BinaryOp {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
        span: z(),
    })
}

fn arb_unop_expr(_depth: u32) -> impl Strategy<Value = Expr> {
    // Only apply unary ops to leaf expressions to avoid ambiguities like
    // `-!0` (parsed as MinusChecked token) or `!!x` (parsed as BangBang).
    (
        prop::sample::select(&[UnOp::Neg, UnOp::Not]),
        arb_expr_leaf(),
    )
        .prop_map(|(op, operand)| Expr {
            kind: ExprKind::UnaryOp {
                op,
                operand: Box::new(operand),
            },
            span: z(),
        })
}

fn arb_if_expr(depth: u32) -> impl Strategy<Value = Expr> {
    // Condition must be a simple expression (not if/while).
    // Both branches must have non-empty blocks.
    (
        arb_expr_leaf(),
        arb_block_inner(depth),
        prop::option::of(arb_block_inner(depth)),
    )
        .prop_map(|(cond, then_b, else_b)| Expr {
            kind: ExprKind::If {
                condition: Box::new(cond),
                then_branch: Box::new(then_b),
                else_branch: else_b.map(|b| {
                    Box::new(Expr {
                        kind: ExprKind::Block(Box::new(b)),
                        span: z(),
                    })
                }),
            },
            span: z(),
        })
}

fn arb_call_expr(depth: u32) -> impl Strategy<Value = Expr> {
    (
        arb_ident(),
        prop::collection::vec(arb_expr_inner(depth), 0..=2),
    )
        .prop_map(|(name, args)| Expr {
            kind: ExprKind::Call {
                callee: Box::new(Expr {
                    kind: ExprKind::Ident(name),
                    span: z(),
                }),
                args,
            },
            span: z(),
        })
}

fn arb_stmt(depth: u32) -> impl Strategy<Value = Stmt> {
    prop_oneof![
        // let binding
        (arb_ident(), arb_type(), arb_expr_inner(depth)).prop_map(|(name, ty, init)| {
            Stmt::Let {
                pattern: Pat {
                    kind: PatKind::Ident {
                        name,
                        is_mut: false,
                    },
                    span: z(),
                },
                ty: Some(ty),
                init: Box::new(init),
                span: z(),
            }
        }),
        // expression statement
        arb_expr_inner(depth).prop_map(|expr| Stmt::Expr {
            expr,
            has_semicolon: true,
            span: z(),
        }),
    ]
}

fn arb_block_inner(depth: u32) -> impl Strategy<Value = Block> {
    // Always include a trailing expression to avoid empty blocks like `{}`
    // which the parser may reject in some positions (e.g., if-then branch).
    // Use arb_expr_or_if since trailing position can accept if-expressions.
    (
        prop::collection::vec(arb_stmt(depth), 0..=2),
        arb_expr_or_if(depth),
    )
        .prop_map(|(stmts, trailing)| Block {
            stmts,
            trailing_expr: Some(Box::new(trailing)),
            span: z(),
        })
}

pub fn arb_block() -> impl Strategy<Value = Block> {
    arb_block_inner(2)
}

fn arb_fn_vow_clause() -> impl Strategy<Value = VowClause> {
    let pred = arb_expr_inner(2);
    (prop::sample::select(&["requires", "ensures"]), pred).prop_map(|(kind, expr)| match kind {
        "requires" => VowClause::Requires { expr, span: z() },
        _ => VowClause::Ensures { expr, span: z() },
    })
}

#[allow(dead_code)]
pub fn arb_vow_clause() -> impl Strategy<Value = VowClause> {
    let pred = arb_expr_inner(2);
    (
        prop::sample::select(&["requires", "ensures", "invariant"]),
        pred,
    )
        .prop_map(|(kind, expr)| match kind {
            "requires" => VowClause::Requires { expr, span: z() },
            "ensures" => VowClause::Ensures { expr, span: z() },
            _ => VowClause::Invariant { expr, span: z() },
        })
}

pub fn arb_vow_block() -> impl Strategy<Value = VowBlock> {
    prop::collection::vec(arb_fn_vow_clause(), 1..=3)
        .prop_map(|clauses| VowBlock { clauses, span: z() })
}

pub fn arb_effects() -> impl Strategy<Value = Vec<Effect>> {
    prop::collection::vec(
        prop::sample::select(&[
            Effect::IO,
            Effect::Panic,
            Effect::Read,
            Effect::Write,
            Effect::Unsafe,
        ]),
        0..=2,
    )
    .prop_map(|mut effects| {
        effects.sort();
        effects.dedup();
        effects
    })
}

pub fn arb_param() -> impl Strategy<Value = Param> {
    (arb_ident(), arb_type()).prop_map(|(name, ty)| Param {
        name,
        ty,
        refinement: None,
        span: z(),
    })
}

pub fn arb_fn_def() -> impl Strategy<Value = FnDef> {
    (
        arb_ident(),
        prop::collection::vec(arb_param(), 0..=3),
        arb_type(),
        arb_effects(),
        prop::option::of(arb_vow_block()),
        arb_block(),
    )
        .prop_map(|(name, params, ret_ty, effects, vow, body)| FnDef {
            vis: Visibility::Private,
            name,
            params,
            return_ty: ret_ty,
            effects,
            vow,
            body,
            span: z(),
            is_declaration: false,
        })
}

pub fn arb_field_def() -> impl Strategy<Value = FieldDef> {
    (arb_ident(), arb_type()).prop_map(|(name, ty)| FieldDef {
        name,
        ty,
        span: z(),
    })
}

pub fn arb_struct_def() -> impl Strategy<Value = StructDef> {
    (
        arb_user_type_name(),
        prop::bool::ANY,
        prop::collection::vec(arb_field_def(), 1..=4),
    )
        .prop_map(|(name, is_linear, fields)| StructDef {
            vis: Visibility::Private,
            name,
            is_linear,
            fields,
            span: z(),
        })
}

pub fn arb_enum_def() -> impl Strategy<Value = EnumDef> {
    (
        arb_user_type_name(),
        prop::collection::vec(
            arb_ident().prop_map(|name| {
                let capped = {
                    let mut c = name.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    }
                };
                EnumVariant {
                    name: capped,
                    kind: VariantKind::Unit,
                    span: z(),
                }
            }),
            1..=4,
        ),
    )
        .prop_map(|(name, variants)| EnumDef {
            vis: Visibility::Private,
            name,
            variants,
            span: z(),
        })
}

pub fn arb_item() -> impl Strategy<Value = Item> {
    prop_oneof![
        6 => arb_fn_def().prop_map(Item::Fn),
        2 => arb_struct_def().prop_map(Item::Struct),
        2 => arb_enum_def().prop_map(Item::Enum),
    ]
}

pub fn arb_module() -> impl Strategy<Value = Module> {
    (arb_module_name(), prop::collection::vec(arb_item(), 1..=4)).prop_map(|(name, items)| Module {
        name,
        uses: vec![],
        items,
        span: z(),
    })
}
