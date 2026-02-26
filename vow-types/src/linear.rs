use std::collections::HashMap;

use vow_diag::{Blame, Diagnostic, DiagnosticEmitter, ErrorCode, Severity, SourceLocation};
use vow_syntax::ast::{Block, Expr, ExprKind, FnDef, MatchArm, Pat, PatKind, Stmt};
use vow_syntax::span::Span;

use crate::env::TypeEnv;

#[derive(Debug, Clone, PartialEq)]
enum ConsumeState {
    Available(Span),
    Consumed(Span),
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

    for (name, state) in &tracker.vars {
        if let ConsumeState::Available(def_span) = state {
            emit_violation(
                file,
                emitter,
                format!("linear value `{name}` is never consumed"),
                *def_span,
                Blame::Callee,
            );
        }
    }
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
        ExprKind::Lit(_) | ExprKind::Result => {}
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
                );
            }
            tracker
                .vars
                .insert(name.to_string(), ConsumeState::Consumed(span));
        }
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
            match (then_state, else_state) {
                (Some(ConsumeState::Consumed(span)), Some(ConsumeState::Consumed(_))) => {
                    tracker
                        .vars
                        .insert(name.clone(), ConsumeState::Consumed(*span));
                }
                (Some(ConsumeState::Available(_)), Some(ConsumeState::Available(_))) => {}
                (Some(ConsumeState::Consumed(span)), Some(ConsumeState::Available(_))) => {
                    emit_violation(
                        file,
                        emitter,
                        format!(
                            "linear value `{name}` is consumed in one branch but not the other"
                        ),
                        *span,
                        Blame::None,
                    );
                }
                (Some(ConsumeState::Available(_)), Some(ConsumeState::Consumed(span))) => {
                    emit_violation(
                        file,
                        emitter,
                        format!(
                            "linear value `{name}` is consumed in one branch but not the other"
                        ),
                        *span,
                        Blame::None,
                    );
                }
                _ => {}
            }
        }
    } else {
        let names: Vec<String> = then_tracker.vars.keys().cloned().collect();
        for name in &names {
            let then_state = then_tracker.vars.get(name);
            if matches!(then_state, Some(ConsumeState::Consumed(_)))
                && matches!(tracker.vars.get(name), Some(ConsumeState::Available(_)))
            {
                emit_violation(
                    file,
                    emitter,
                    format!(
                        "linear value `{name}` is consumed in `if` branch but there is no `else` branch"
                    ),
                    then_branch.span,
                    Blame::None,
                );
                tracker
                    .vars
                    .insert(name.clone(), ConsumeState::Consumed(then_branch.span));
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
        let all_consumed = arm_trackers
            .iter()
            .all(|t| matches!(t.vars.get(name), Some(ConsumeState::Consumed(_))));
        let any_consumed = arm_trackers
            .iter()
            .any(|t| matches!(t.vars.get(name), Some(ConsumeState::Consumed(_))));

        if all_consumed {
            if let Some(first_arm) = arm_trackers.first()
                && let Some(ConsumeState::Consumed(span)) = first_arm.vars.get(name)
            {
                tracker
                    .vars
                    .insert(name.clone(), ConsumeState::Consumed(*span));
            }
        } else if any_consumed {
            let consumed_span = arm_trackers
                .iter()
                .find_map(|t| {
                    if let Some(ConsumeState::Consumed(s)) = t.vars.get(name) {
                        Some(*s)
                    } else {
                        None
                    }
                })
                .unwrap();
            emit_violation(
                file,
                emitter,
                format!("linear value `{name}` is consumed in one branch but not the other"),
                consumed_span,
                Blame::None,
            );
        }
    }
}

fn emit_violation(
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
    message: String,
    span: Span,
    blame: Blame,
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
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use vow_diag::Diagnostic;
    use vow_syntax::ast::{Block, Expr, ExprKind, FnDef, Lit, Param, Stmt, Type, Visibility};
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
                generics: vec![],
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
            generics: vec![],
            params,
            return_ty: Type::Unit { span: dummy_span() },
            effects: vec![],
            vow: None,
            body,
            span: dummy_span(),
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
    fn test_linear_never_consumed_error() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let body = empty_block();
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert_eq!(emitter.0.len(), 1);
        assert!(emitter.0[0].message.contains("never consumed"));
        assert_eq!(emitter.0[0].code, ErrorCode::LinearTypeViolation);
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
    fn test_linear_in_only_one_branch_error() {
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

        assert_eq!(
            emitter.0.len(),
            1,
            "Expected 1 error but got: {:?}",
            emitter.0
        );
        assert!(
            emitter.0[0].message.contains("no `else` branch")
                || emitter.0[0].message.contains("one branch")
        );
        assert_eq!(emitter.0[0].code, ErrorCode::LinearTypeViolation);
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
}
