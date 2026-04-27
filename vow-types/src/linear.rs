use std::collections::HashMap;

use vow_diag::{Blame, Diagnostic, DiagnosticEmitter, ErrorCode, Severity, SourceLocation};
use vow_syntax::ast::{Block, Expr, ExprKind, FnDef, MatchArm, Pat, PatKind, Stmt};
use vow_syntax::span::Span;

use crate::env::TypeEnv;

#[derive(Debug, Clone, PartialEq)]
enum ConsumeState {
    Available(Span),
    Consumed(Span),
    MaybeConsumed(Span),
}

#[derive(Debug, Clone)]
struct LinearTracker {
    vars: HashMap<String, ConsumeState>,
    in_loop: bool,
}

impl LinearTracker {
    fn new() -> Self {
        Self {
            vars: HashMap::new(),
            in_loop: false,
        }
    }
}

pub fn check_linear_usage(
    fn_def: &FnDef,
    env: &TypeEnv,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    let mut tracker = LinearTracker::new();

    for param in &fn_def.params {
        if is_linear_ast_type(&param.ty, env) {
            tracker
                .vars
                .insert(param.name.clone(), ConsumeState::Available(param.span));
        }
    }

    check_block(&fn_def.body, &mut tracker, env, file, emitter);

    // Region-linear checking runs after region inference and reports obligations
    // that remain live at a region close. This pass only rejects uses that are
    // immediately invalid before region placement is known.
}

fn is_linear_ast_type(ast_ty: &vow_syntax::ast::Type, env: &TypeEnv) -> bool {
    match ast_ty {
        vow_syntax::ast::Type::Named { name, .. } => env
            .lookup_struct(name)
            .map(|info| info.is_linear)
            .unwrap_or(false),
        _ => false,
    }
}

fn check_block(
    block: &Block,
    tracker: &mut LinearTracker,
    env: &TypeEnv,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    for stmt in &block.stmts {
        check_stmt(stmt, tracker, env, file, emitter);
    }
    if let Some(expr) = &block.trailing_expr {
        check_expr(expr, tracker, env, file, emitter, true);
    }
}

fn check_stmt(
    stmt: &Stmt,
    tracker: &mut LinearTracker,
    env: &TypeEnv,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    match stmt {
        Stmt::Let {
            pattern,
            ty,
            init,
            span,
        } => {
            check_expr(init, tracker, env, file, emitter, true);
            register_pattern_linear(pattern, ty.as_ref(), env, tracker, *span);
        }
        Stmt::Expr { expr, .. } => {
            check_expr(expr, tracker, env, file, emitter, true);
        }
    }
}

fn register_pattern_linear(
    pat: &Pat,
    ty_ann: Option<&vow_syntax::ast::Type>,
    env: &TypeEnv,
    tracker: &mut LinearTracker,
    span: Span,
) {
    if let PatKind::Ident { name, .. } = &pat.kind {
        let is_linear = ty_ann.map(|t| is_linear_ast_type(t, env)).unwrap_or(false);
        if is_linear {
            tracker
                .vars
                .insert(name.clone(), ConsumeState::Available(span));
        }
    }
}

fn check_expr(
    expr: &Expr,
    tracker: &mut LinearTracker,
    env: &TypeEnv,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
    consume: bool,
) {
    match &expr.kind {
        ExprKind::Ident(name) => {
            if consume {
                consume_var(name, expr.span, tracker, file, emitter);
            }
        }
        ExprKind::Borrow { expr: inner } => {
            check_expr(inner, tracker, env, file, emitter, false);
        }
        ExprKind::Call { callee, args } => {
            check_expr(callee, tracker, env, file, emitter, false);
            for arg in args {
                check_expr(arg, tracker, env, file, emitter, true);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            check_expr(receiver, tracker, env, file, emitter, false);
            for arg in args {
                check_expr(arg, tracker, env, file, emitter, true);
            }
        }
        ExprKind::Return { value } => {
            if let Some(v) = value {
                check_expr(v, tracker, env, file, emitter, true);
            }
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            check_expr(condition, tracker, env, file, emitter, false);
            check_if_branches(
                then_branch,
                else_branch.as_deref(),
                tracker,
                env,
                file,
                emitter,
            );
        }
        ExprKind::Match { scrutinee, arms } => {
            check_expr(scrutinee, tracker, env, file, emitter, false);
            check_match_arms(arms, tracker, env, file, emitter);
        }
        ExprKind::While {
            condition, body, ..
        } => {
            check_expr(condition, tracker, env, file, emitter, false);
            let was_in_loop = tracker.in_loop;
            tracker.in_loop = true;
            check_block(body, tracker, env, file, emitter);
            tracker.in_loop = was_in_loop;
        }
        ExprKind::ForEach { iterable, body, .. } => {
            check_expr(iterable, tracker, env, file, emitter, false);
            let was_in_loop = tracker.in_loop;
            tracker.in_loop = true;
            check_block(body, tracker, env, file, emitter);
            tracker.in_loop = was_in_loop;
        }
        ExprKind::Loop { body, .. } => {
            let was_in_loop = tracker.in_loop;
            tracker.in_loop = true;
            check_block(body, tracker, env, file, emitter);
            tracker.in_loop = was_in_loop;
        }
        ExprKind::Block(block) => check_block(block, tracker, env, file, emitter),
        ExprKind::Assign { lhs, rhs } => {
            check_expr(lhs, tracker, env, file, emitter, false);
            check_expr(rhs, tracker, env, file, emitter, true);
        }
        ExprKind::BinaryOp { lhs, rhs, .. } => {
            check_expr(lhs, tracker, env, file, emitter, false);
            check_expr(rhs, tracker, env, file, emitter, false);
        }
        ExprKind::UnaryOp { operand, .. } => {
            check_expr(operand, tracker, env, file, emitter, false);
        }
        ExprKind::FieldAccess { base, .. } => {
            check_expr(base, tracker, env, file, emitter, false);
        }
        ExprKind::Index { base, index } => {
            check_expr(base, tracker, env, file, emitter, false);
            check_expr(index, tracker, env, file, emitter, false);
        }
        ExprKind::Question { expr: inner } => {
            check_expr(inner, tracker, env, file, emitter, true);
        }
        ExprKind::Tuple(exprs) => {
            for e in exprs {
                check_expr(e, tracker, env, file, emitter, true);
            }
        }
        ExprKind::Break { value } => {
            if let Some(v) = value {
                check_expr(v, tracker, env, file, emitter, true);
            }
        }
        ExprKind::Continue | ExprKind::Lit(_) | ExprKind::Result => {}
        ExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                check_expr(e, tracker, env, file, emitter, true);
            }
        }
        ExprKind::EnumConstruct { fields, .. } => {
            for e in fields {
                check_expr(e, tracker, env, file, emitter, true);
            }
        }
        ExprKind::Cast { expr: inner, .. } => {
            check_expr(inner, tracker, env, file, emitter, consume);
        }
    }
}

fn consume_var(
    name: &str,
    span: Span,
    tracker: &mut LinearTracker,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    match tracker.vars.get(name) {
        None => {}
        Some(ConsumeState::Consumed(_)) => {
            emit_violation(
                file,
                emitter,
                format!("linear value `{name}` already consumed"),
                span,
                Blame::None,
                vec![format!(
                    "`{name}` was already consumed; clone it or restructure to use it only once"
                )],
            );
        }
        Some(ConsumeState::MaybeConsumed(_)) => {
            emit_violation(
                file,
                emitter,
                format!("linear value `{name}` may already be consumed"),
                span,
                Blame::None,
                vec![format!(
                    "`{name}` is consumed on some control-flow paths; restructure before using it again"
                )],
            );
        }
        Some(ConsumeState::Available(_)) => {
            if tracker.in_loop {
                emit_violation(
                    file,
                    emitter,
                    format!(
                        "linear value `{name}` cannot be consumed inside a loop (would be consumed multiple times)"
                    ),
                    span,
                    Blame::None,
                    vec![format!("move the consumption of `{name}` outside the loop")],
                );
            }
            tracker
                .vars
                .insert(name.to_string(), ConsumeState::Consumed(span));
        }
    }
}

fn merge_branch_state(
    left: Option<&ConsumeState>,
    right: Option<&ConsumeState>,
) -> Option<ConsumeState> {
    match (left, right) {
        (Some(ConsumeState::Consumed(span)), Some(ConsumeState::Consumed(_))) => {
            Some(ConsumeState::Consumed(*span))
        }
        (Some(ConsumeState::Available(span)), Some(ConsumeState::Available(_))) => {
            Some(ConsumeState::Available(*span))
        }
        (Some(ConsumeState::MaybeConsumed(span)), _)
        | (_, Some(ConsumeState::MaybeConsumed(span))) => Some(ConsumeState::MaybeConsumed(*span)),
        (Some(ConsumeState::Consumed(span)), Some(ConsumeState::Available(_)))
        | (Some(ConsumeState::Available(_)), Some(ConsumeState::Consumed(span))) => {
            Some(ConsumeState::MaybeConsumed(*span))
        }
        (Some(state), None) | (None, Some(state)) => Some(state.clone()),
        (None, None) => None,
    }
}

fn state_may_be_consumed(state: Option<&ConsumeState>) -> Option<Span> {
    match state {
        Some(ConsumeState::Consumed(span)) | Some(ConsumeState::MaybeConsumed(span)) => Some(*span),
        _ => None,
    }
}

fn check_if_branches(
    then_branch: &Block,
    else_branch: Option<&Expr>,
    tracker: &mut LinearTracker,
    env: &TypeEnv,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    let mut then_tracker = tracker.clone();
    let mut else_tracker = tracker.clone();

    check_block(then_branch, &mut then_tracker, env, file, emitter);
    if let Some(else_expr) = else_branch {
        check_expr(else_expr, &mut else_tracker, env, file, emitter, true);
    }

    if else_branch.is_some() {
        let names: Vec<String> = then_tracker.vars.keys().cloned().collect();
        for name in &names {
            let then_state = then_tracker.vars.get(name);
            let else_state = else_tracker.vars.get(name);
            if let Some(merged) = merge_branch_state(then_state, else_state) {
                tracker.vars.insert(name.clone(), merged);
            }
        }
    } else {
        let names: Vec<String> = then_tracker.vars.keys().cloned().collect();
        for name in &names {
            let then_state = then_tracker.vars.get(name);
            if let Some(span) = state_may_be_consumed(then_state)
                && matches!(tracker.vars.get(name), Some(ConsumeState::Available(_)))
            {
                tracker
                    .vars
                    .insert(name.clone(), ConsumeState::MaybeConsumed(span));
            }
        }
    }
}

fn check_match_arms(
    arms: &[MatchArm],
    tracker: &mut LinearTracker,
    env: &TypeEnv,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    if arms.is_empty() {
        return;
    }

    let arm_trackers: Vec<LinearTracker> = arms
        .iter()
        .map(|arm| {
            let mut arm_tracker = tracker.clone();
            check_expr(&arm.body, &mut arm_tracker, env, file, emitter, true);
            arm_tracker
        })
        .collect();

    let names: Vec<String> = tracker.vars.keys().cloned().collect();
    for name in &names {
        let mut merged = arm_trackers.first().and_then(|t| t.vars.get(name)).cloned();
        for arm_tracker in arm_trackers.iter().skip(1) {
            merged = merge_branch_state(merged.as_ref(), arm_tracker.vars.get(name));
        }
        if let Some(state) = merged {
            tracker.vars.insert(name.clone(), state);
        } else if let Some(consumed_span) = arm_trackers
            .iter()
            .find_map(|t| state_may_be_consumed(t.vars.get(name)))
        {
            tracker
                .vars
                .insert(name.clone(), ConsumeState::MaybeConsumed(consumed_span));
        }
    }
}

fn emit_violation(
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
    message: String,
    span: Span,
    blame: Blame,
    hints: Vec<String>,
) {
    emitter.emit(&Diagnostic {
        severity: Severity::Error,
        code: ErrorCode::LinearTypeViolation,
        message,
        primary: SourceLocation {
            file: file.to_string(),
            byte_offset: span.start,
            byte_len: span.len,
        },
        secondary: vec![],
        blame,
        hints,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use vow_diag::Diagnostic;
    use vow_syntax::ast::{
        BinOp, Block, Expr, ExprKind, FnDef, Lit, MatchArm, Param, Pat, PatKind, Stmt, Type, UnOp,
        Visibility,
    };
    use vow_syntax::span::Span;

    use crate::env::{StructInfo, TypeEnv};

    struct TestEmitter(Vec<Diagnostic>);

    impl DiagnosticEmitter for TestEmitter {
        fn emit(&mut self, d: &Diagnostic) {
            self.0.push(d.clone());
        }
        fn finish(&mut self) {}
    }

    fn dummy_span() -> Span {
        Span::new(0, 1)
    }

    fn ident_expr(name: &str) -> Expr {
        Expr {
            kind: ExprKind::Ident(name.to_string()),
            span: dummy_span(),
        }
    }

    fn call_with(fn_name: &str, arg_name: &str) -> Expr {
        Expr {
            kind: ExprKind::Call {
                callee: Box::new(ident_expr(fn_name)),
                args: vec![ident_expr(arg_name)],
            },
            span: dummy_span(),
        }
    }

    fn borrow_expr(name: &str) -> Expr {
        Expr {
            kind: ExprKind::Borrow {
                expr: Box::new(ident_expr(name)),
            },
            span: dummy_span(),
        }
    }

    fn make_env_with_linear_struct(name: &str) -> TypeEnv {
        let mut env = TypeEnv::new();
        env.define_struct(
            name,
            StructInfo {
                fields: vec![],
                is_linear: true,
            },
        );
        env
    }

    fn named_type(name: &str) -> Type {
        Type::Named {
            name: name.to_string(),
            span: dummy_span(),
        }
    }

    fn make_fn_def(params: Vec<Param>, body: Block) -> FnDef {
        FnDef {
            vis: Visibility::Private,
            name: "test_fn".to_string(),
            params,
            return_ty: Type::Unit { span: dummy_span() },
            effects: vec![],
            vow: None,
            body,
            span: dummy_span(),
            is_declaration: false,
        }
    }

    fn make_param(name: &str, ty: Type) -> Param {
        Param {
            name: name.to_string(),
            ty,
            refinement: None,
            span: dummy_span(),
        }
    }

    fn empty_block() -> Block {
        Block {
            stmts: vec![],
            trailing_expr: None,
            span: dummy_span(),
        }
    }

    fn block_with_expr(expr: Expr) -> Block {
        Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(expr)),
            span: dummy_span(),
        }
    }

    #[test]
    fn test_linear_consumed_once_no_error() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let body = block_with_expr(call_with("close_handle", "h"));
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(
            emitter.0.is_empty(),
            "Expected no errors but got: {:?}",
            emitter.0
        );
    }

    #[test]
    fn test_linear_never_consumed_deferred_to_region_check() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let body = empty_block();
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(emitter.0.is_empty(), "Got: {:?}", emitter.0);
    }

    #[test]
    fn test_linear_consumed_twice_error() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let body = Block {
            stmts: vec![
                Stmt::Expr {
                    expr: call_with("consume", "h"),
                    has_semicolon: true,
                    span: dummy_span(),
                },
                Stmt::Expr {
                    expr: call_with("consume", "h"),
                    has_semicolon: true,
                    span: dummy_span(),
                },
            ],
            trailing_expr: None,
            span: dummy_span(),
        };
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert_eq!(emitter.0.len(), 1);
        assert!(emitter.0[0].message.contains("already consumed"));
        assert_eq!(emitter.0[0].code, ErrorCode::LinearTypeViolation);
    }

    #[test]
    fn test_linear_in_both_branches_no_error() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];

        let then_block = block_with_expr(call_with("consume", "h"));
        let else_expr = Expr {
            kind: ExprKind::Block(Box::new(block_with_expr(call_with("consume", "h")))),
            span: dummy_span(),
        };
        let if_expr = Expr {
            kind: ExprKind::If {
                condition: Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Bool(true)),
                    span: dummy_span(),
                }),
                then_branch: Box::new(then_block),
                else_branch: Some(Box::new(else_expr)),
            },
            span: dummy_span(),
        };
        let body = block_with_expr(if_expr);
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(
            emitter.0.is_empty(),
            "Expected no errors but got: {:?}",
            emitter.0
        );
    }

    #[test]
    fn test_linear_in_only_one_branch_deferred_to_region_check() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];

        let then_block = block_with_expr(call_with("consume", "h"));
        let if_expr = Expr {
            kind: ExprKind::If {
                condition: Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Bool(true)),
                    span: dummy_span(),
                }),
                then_branch: Box::new(then_block),
                else_branch: None,
            },
            span: dummy_span(),
        };
        let body = block_with_expr(if_expr);
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(emitter.0.is_empty(), "Got: {:?}", emitter.0);
    }

    #[test]
    fn test_linear_inside_loop_error() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];

        let loop_body = block_with_expr(call_with("consume", "h"));
        let loop_expr = Expr {
            kind: ExprKind::Loop {
                vow: None,
                body: Box::new(loop_body),
            },
            span: dummy_span(),
        };
        let body = block_with_expr(loop_expr);
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert_eq!(
            emitter.0.len(),
            1,
            "Expected 1 error but got: {:?}",
            emitter.0
        );
        assert!(emitter.0[0].message.contains("loop"));
        assert_eq!(emitter.0[0].code, ErrorCode::LinearTypeViolation);
    }

    #[test]
    fn test_borrow_does_not_consume() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];

        let borrow = borrow_expr("h");
        let call_borrow = Expr {
            kind: ExprKind::Call {
                callee: Box::new(ident_expr("inspect")),
                args: vec![borrow],
            },
            span: dummy_span(),
        };
        let body = Block {
            stmts: vec![Stmt::Expr {
                expr: call_borrow,
                has_semicolon: true,
                span: dummy_span(),
            }],
            trailing_expr: Some(Box::new(call_with("close", "h"))),
            span: dummy_span(),
        };
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(
            emitter.0.is_empty(),
            "Expected no errors but got: {:?}",
            emitter.0
        );
    }

    // --- Stmt::Let registration ---

    #[test]
    fn test_let_stmt_registers_linear_type() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![];
        // let h: FileHandle = open(); then consume h
        let let_stmt = Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: "h".to_string(),
                    is_mut: false,
                },
                span: dummy_span(),
            },
            ty: Some(named_type("FileHandle")),
            init: Box::new(ident_expr("open")),
            span: dummy_span(),
        };
        let body = Block {
            stmts: vec![let_stmt],
            trailing_expr: Some(Box::new(call_with("close", "h"))),
            span: dummy_span(),
        };
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(
            emitter.0.is_empty(),
            "Expected no errors but got: {:?}",
            emitter.0
        );
    }

    #[test]
    fn test_let_stmt_linear_never_consumed_deferred_to_region_check() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![];
        let let_stmt = Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: "h".to_string(),
                    is_mut: false,
                },
                span: dummy_span(),
            },
            ty: Some(named_type("FileHandle")),
            init: Box::new(ident_expr("open")),
            span: dummy_span(),
        };
        let body = Block {
            stmts: vec![let_stmt],
            trailing_expr: None,
            span: dummy_span(),
        };
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(emitter.0.is_empty(), "Got: {:?}", emitter.0);
    }

    #[test]
    fn test_let_stmt_non_linear_type_not_tracked() {
        let env = TypeEnv::new();
        let params = vec![];
        let let_stmt = Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: "x".to_string(),
                    is_mut: false,
                },
                span: dummy_span(),
            },
            ty: None,
            init: Box::new(Expr {
                kind: ExprKind::Lit(Lit::Int(42)),
                span: dummy_span(),
            }),
            span: dummy_span(),
        };
        let body = Block {
            stmts: vec![let_stmt],
            trailing_expr: None,
            span: dummy_span(),
        };
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(emitter.0.is_empty());
    }

    // --- MethodCall ---

    #[test]
    fn test_method_call_arg_consumes_linear() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let method_call = Expr {
            kind: ExprKind::MethodCall {
                receiver: Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Int(0)),
                    span: dummy_span(),
                }),
                method: "write".to_string(),
                args: vec![ident_expr("h")],
            },
            span: dummy_span(),
        };
        let body = block_with_expr(method_call);
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(
            emitter.0.is_empty(),
            "Expected no errors but got: {:?}",
            emitter.0
        );
    }

    // --- Return ---

    #[test]
    fn test_return_consumes_linear() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let return_expr = Expr {
            kind: ExprKind::Return {
                value: Some(Box::new(ident_expr("h"))),
            },
            span: dummy_span(),
        };
        let body = block_with_expr(return_expr);
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(
            emitter.0.is_empty(),
            "Expected no errors but got: {:?}",
            emitter.0
        );
    }

    #[test]
    fn test_return_no_value_does_not_consume() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let return_expr = Expr {
            kind: ExprKind::Return { value: None },
            span: dummy_span(),
        };
        let body = block_with_expr(return_expr);
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(emitter.0.is_empty(), "Got: {:?}", emitter.0);
    }

    // --- While ---

    #[test]
    fn test_while_loop_linear_in_body_error() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let while_expr = Expr {
            kind: ExprKind::While {
                condition: Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Bool(true)),
                    span: dummy_span(),
                }),
                body: Box::new(block_with_expr(call_with("consume", "h"))),
                vow: None,
            },
            span: dummy_span(),
        };
        let body = block_with_expr(while_expr);
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert_eq!(emitter.0.len(), 1);
        assert!(emitter.0[0].message.contains("loop"));
    }

    // --- Match ---

    fn make_wildcard_arm(body: Expr) -> MatchArm {
        MatchArm {
            pattern: Pat {
                kind: PatKind::Wildcard,
                span: dummy_span(),
            },
            body,
            span: dummy_span(),
        }
    }

    #[test]
    fn test_match_all_arms_consume_linear_no_error() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let match_expr = Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Bool(true)),
                    span: dummy_span(),
                }),
                arms: vec![
                    make_wildcard_arm(call_with("consume", "h")),
                    make_wildcard_arm(call_with("close", "h")),
                ],
            },
            span: dummy_span(),
        };
        let body = block_with_expr(match_expr);
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(
            emitter.0.is_empty(),
            "Expected no errors but got: {:?}",
            emitter.0
        );
    }

    #[test]
    fn test_match_only_some_arms_consume_linear_deferred_to_region_check() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let match_expr = Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Bool(true)),
                    span: dummy_span(),
                }),
                arms: vec![
                    make_wildcard_arm(call_with("consume", "h")),
                    make_wildcard_arm(Expr {
                        kind: ExprKind::Lit(Lit::Int(0)),
                        span: dummy_span(),
                    }),
                ],
            },
            span: dummy_span(),
        };
        let body = block_with_expr(match_expr);
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(emitter.0.is_empty(), "Got: {:?}", emitter.0);
    }

    // --- if-else asymmetric consumption ---

    #[test]
    fn test_if_else_only_then_consumes_deferred_to_region_check() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];

        let then_block = block_with_expr(call_with("consume", "h"));
        let else_expr = Expr {
            kind: ExprKind::Block(Box::new(Block {
                stmts: vec![],
                trailing_expr: Some(Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Int(0)),
                    span: dummy_span(),
                })),
                span: dummy_span(),
            })),
            span: dummy_span(),
        };
        let if_expr = Expr {
            kind: ExprKind::If {
                condition: Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Bool(true)),
                    span: dummy_span(),
                }),
                then_branch: Box::new(then_block),
                else_branch: Some(Box::new(else_expr)),
            },
            span: dummy_span(),
        };
        let body = block_with_expr(if_expr);
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(emitter.0.is_empty(), "Got: {:?}", emitter.0);
    }

    #[test]
    fn test_partial_branch_then_later_consume_error() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];

        let then_block = block_with_expr(call_with("consume", "h"));
        let if_expr = Expr {
            kind: ExprKind::If {
                condition: Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Bool(true)),
                    span: dummy_span(),
                }),
                then_branch: Box::new(then_block),
                else_branch: None,
            },
            span: dummy_span(),
        };
        let body = Block {
            stmts: vec![Stmt::Expr {
                expr: if_expr,
                has_semicolon: true,
                span: dummy_span(),
            }],
            trailing_expr: Some(Box::new(call_with("close", "h"))),
            span: dummy_span(),
        };
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert_eq!(emitter.0.len(), 1, "Got: {:?}", emitter.0);
        assert!(emitter.0[0].message.contains("may already be consumed"));
        assert_eq!(emitter.0[0].code, ErrorCode::LinearTypeViolation);
    }

    // --- BinaryOp, UnaryOp, FieldAccess, Index, Question, Tuple, Break ---

    #[test]
    fn test_binary_op_does_not_consume() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let binop = Expr {
            kind: ExprKind::BinaryOp {
                op: BinOp::Add,
                lhs: Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Int(1)),
                    span: dummy_span(),
                }),
                rhs: Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Int(2)),
                    span: dummy_span(),
                }),
            },
            span: dummy_span(),
        };
        let body = Block {
            stmts: vec![Stmt::Expr {
                expr: binop,
                has_semicolon: true,
                span: dummy_span(),
            }],
            trailing_expr: Some(Box::new(call_with("close", "h"))),
            span: dummy_span(),
        };
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(emitter.0.is_empty(), "Got: {:?}", emitter.0);
    }

    #[test]
    fn test_unary_op_does_not_consume() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let unop = Expr {
            kind: ExprKind::UnaryOp {
                op: UnOp::Neg,
                operand: Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Int(1)),
                    span: dummy_span(),
                }),
            },
            span: dummy_span(),
        };
        let body = Block {
            stmts: vec![Stmt::Expr {
                expr: unop,
                has_semicolon: true,
                span: dummy_span(),
            }],
            trailing_expr: Some(Box::new(call_with("close", "h"))),
            span: dummy_span(),
        };
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(emitter.0.is_empty(), "Got: {:?}", emitter.0);
    }

    #[test]
    fn test_field_access_does_not_consume() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let field = Expr {
            kind: ExprKind::FieldAccess {
                base: Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Int(0)),
                    span: dummy_span(),
                }),
                field: "len".to_string(),
            },
            span: dummy_span(),
        };
        let body = Block {
            stmts: vec![Stmt::Expr {
                expr: field,
                has_semicolon: true,
                span: dummy_span(),
            }],
            trailing_expr: Some(Box::new(call_with("close", "h"))),
            span: dummy_span(),
        };
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(emitter.0.is_empty(), "Got: {:?}", emitter.0);
    }

    #[test]
    fn test_index_does_not_consume() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let index = Expr {
            kind: ExprKind::Index {
                base: Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Int(0)),
                    span: dummy_span(),
                }),
                index: Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Int(1)),
                    span: dummy_span(),
                }),
            },
            span: dummy_span(),
        };
        let body = Block {
            stmts: vec![Stmt::Expr {
                expr: index,
                has_semicolon: true,
                span: dummy_span(),
            }],
            trailing_expr: Some(Box::new(call_with("close", "h"))),
            span: dummy_span(),
        };
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(emitter.0.is_empty(), "Got: {:?}", emitter.0);
    }

    #[test]
    fn test_question_consumes_inner() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let question = Expr {
            kind: ExprKind::Question {
                expr: Box::new(ident_expr("h")),
            },
            span: dummy_span(),
        };
        let body = block_with_expr(question);
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(emitter.0.is_empty(), "Got: {:?}", emitter.0);
    }

    #[test]
    fn test_tuple_consumes_elements() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let tuple = Expr {
            kind: ExprKind::Tuple(vec![
                ident_expr("h"),
                Expr {
                    kind: ExprKind::Lit(Lit::Int(0)),
                    span: dummy_span(),
                },
            ]),
            span: dummy_span(),
        };
        let body = block_with_expr(tuple);
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(emitter.0.is_empty(), "Got: {:?}", emitter.0);
    }

    #[test]
    fn test_break_with_value_consumes_linear() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let break_expr = Expr {
            kind: ExprKind::Break {
                value: Some(Box::new(ident_expr("h"))),
            },
            span: dummy_span(),
        };
        let loop_body = block_with_expr(break_expr);
        let loop_expr = Expr {
            kind: ExprKind::Loop {
                vow: None,
                body: Box::new(loop_body),
            },
            span: dummy_span(),
        };
        // Note: h is not in loop tracker since loop sets in_loop=true AFTER registering h
        // but break with value from *outside* the loop is different; this tests the Break arm
        // We use h as a loop-external param consumed via break inside loop — should error (loop)
        let body = block_with_expr(loop_expr);
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        // h is consumed inside a loop → error
        assert_eq!(emitter.0.len(), 1);
        assert!(emitter.0[0].message.contains("loop"));
    }

    #[test]
    fn test_assign_rhs_consumes_linear() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let assign = Expr {
            kind: ExprKind::Assign {
                lhs: Box::new(ident_expr("x")),
                rhs: Box::new(ident_expr("h")),
            },
            span: dummy_span(),
        };
        let body = block_with_expr(assign);
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(emitter.0.is_empty(), "Got: {:?}", emitter.0);
    }
}
