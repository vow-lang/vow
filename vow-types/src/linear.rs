use std::collections::{HashMap, HashSet};

use vow_diag::{Blame, Diagnostic, DiagnosticEmitter, ErrorCode, Severity, SourceLocation};
use vow_syntax::ast::{Block, Expr, ExprKind, FnDef, MatchArm, Pat, PatKind, Stmt};
use vow_syntax::span::Span;

use crate::env::TypeEnv;
use crate::env::VariantKind;
use crate::types::Ty;

#[derive(Debug, Clone, PartialEq)]
enum ConsumeState {
    /// Not yet consumed; the `u32` is the loop depth the binding was declared at.
    Available(Span, u32),
    Consumed(Span),
    MaybeConsumed(Span),
}

#[derive(Debug)]
struct LinearTracker {
    vars: HashMap<String, ConsumeState>,
    loop_depth: u32,
    scopes: Vec<Vec<ShadowedBinding>>,
}

/// A linear `let` registered in a block, with whatever entry it overwrote, so
/// the block can hand the name back to the enclosing scope on exit.
type ShadowedBinding = (String, Option<ConsumeState>);

impl LinearTracker {
    fn new() -> Self {
        Self {
            vars: HashMap::new(),
            loop_depth: 0,
            scopes: Vec::new(),
        }
    }

    /// Branch copy: a branch only closes scopes it opens itself, so the
    /// enclosing frames are not carried along.
    fn fork(&self) -> Self {
        Self {
            vars: self.vars.clone(),
            loop_depth: self.loop_depth,
            scopes: Vec::new(),
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
                .insert(param.name.clone(), ConsumeState::Available(param.span, 0));
        }
    }

    check_block(&fn_def.body, &mut tracker, env, file, emitter);

    // Backstop for linear let-bindings whose IR producer lowers as i64 (FieldGet/Index of Vec<Linear>); flags both Available (never consumed) and MaybeConsumed (consumed on some paths) since the region pass can't see those origins. Params skip via span (not name) so a shadowing `let h = v[0]` is still flagged.
    let param_spans: std::collections::HashSet<Span> = fn_def
        .params
        .iter()
        .filter(|p| is_linear_ast_type(&p.ty, env))
        .map(|p| p.span)
        .collect();
    for (name, state) in &tracker.vars {
        let (def_span, message, hint) = match state {
            ConsumeState::Available(s, _) => (
                *s,
                format!("linear value `{name}` is never consumed"),
                format!("consume `{name}` by passing it to a function or using drop()"),
            ),
            ConsumeState::MaybeConsumed(s) => (
                *s,
                format!("linear value `{name}` may not be consumed on every path"),
                format!(
                    "ensure `{name}` is consumed on every control-flow path before the function returns"
                ),
            ),
            ConsumeState::Consumed(_) => continue,
        };
        if param_spans.contains(&def_span) {
            continue;
        }
        emit_violation(file, emitter, message, def_span, Blame::Callee, vec![hint]);
    }
}

fn is_linear_ast_type(ast_ty: &vow_syntax::ast::Type, env: &TypeEnv) -> bool {
    env.resolve(ast_ty)
        .is_ok_and(|ty| is_linear_owner_ty(&ty, env))
}

/// True when a value owns a linear obligation directly or through an enum-like
/// wrapper. References borrow rather than own, and collections do not become
/// linear containers merely because their element type is linear.
pub(crate) fn is_linear_owner_ty(ty: &Ty, env: &TypeEnv) -> bool {
    fn visit(ty: &Ty, env: &TypeEnv, visiting: &mut HashSet<String>) -> bool {
        match ty {
            Ty::Struct(name) => env.lookup_struct(name).is_some_and(|info| info.is_linear),
            Ty::Enum(name) => {
                let key = format!("enum:{name}");
                if !visiting.insert(key.clone()) {
                    return false;
                }
                let result = env.lookup_enum(name).is_some_and(|info| {
                    info.variants.iter().any(|variant| match &variant.kind {
                        VariantKind::Tuple(types) => {
                            types.iter().any(|payload| visit(payload, env, visiting))
                        }
                        VariantKind::Struct(fields) => fields
                            .iter()
                            .any(|(_, payload)| visit(payload, env, visiting)),
                        VariantKind::Unit => false,
                    })
                });
                visiting.remove(&key);
                result
            }
            // Option and Result are synthesized by TypeEnv::resolve rather than
            // registered in enum_defs, while user enums are found through the
            // environment. Keep both paths explicit here.
            Ty::Applied(base, args) if matches!(base.as_ref(), Ty::Enum(name) if name == "Option" || name == "Result" || env.lookup_enum(name).is_some()) => {
                visit(base, env, visiting)
                    || args.iter().any(|payload| visit(payload, env, visiting))
            }
            Ty::Reference(_) => false,
            _ => false,
        }
    }

    visit(ty, env, &mut HashSet::new())
}

fn check_block(
    block: &Block,
    tracker: &mut LinearTracker,
    env: &TypeEnv,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    tracker.scopes.push(Vec::new());
    for stmt in &block.stmts {
        check_stmt(stmt, tracker, env, file, emitter);
    }
    if let Some(expr) = &block.trailing_expr {
        check_expr(expr, tracker, env, file, emitter, true);
    }
    close_scope(tracker);
}

/// Names are tracked flat, so a consumed block-local would otherwise keep
/// shadowing (or being mistaken for) a same-named binding outside the block.
/// Only consumed bindings are handed back: an unconsumed one stays so the
/// end-of-function backstop still reports it. A non-linear `let` that evicted a
/// consumed entry is restored the same way.
fn close_scope(tracker: &mut LinearTracker) {
    let frame = tracker.scopes.pop().expect("check_block pushed a scope");
    for (name, shadowed) in frame.into_iter().rev() {
        match tracker.vars.get(&name) {
            None => {
                if let Some(state) = shadowed {
                    tracker.vars.insert(name, state);
                }
            }
            Some(ConsumeState::Consumed(_)) => {
                match shadowed {
                    Some(state) => tracker.vars.insert(name, state),
                    None => tracker.vars.remove(&name),
                };
            }
            Some(_) => {}
        }
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
    if let PatKind::Ident { name, .. } = &pat.kind
        && ty_ann.is_some_and(|t| is_linear_ast_type(t, env))
    {
        let shadowed = tracker.vars.insert(
            name.clone(),
            ConsumeState::Available(span, tracker.loop_depth),
        );
        if let Some(frame) = tracker.scopes.last_mut() {
            frame.push((name.clone(), shadowed));
        }
        return;
    }
    // Names are tracked flat: evict a consumed entry so a later assignment cannot re-arm it.
    if tracker.vars.is_empty() {
        return;
    }
    let mut names = vec![];
    collect_pattern_binding_names(pat, &mut names);
    for name in names {
        if matches!(tracker.vars.get(&name), Some(ConsumeState::Consumed(_)))
            && let Some(state) = tracker.vars.remove(&name)
            && let Some(frame) = tracker.scopes.last_mut()
        {
            frame.push((name, Some(state)));
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
            // Non-linear scrutinees are not registered in the tracker, so this
            // only consumes enum-like wrappers that own a linear payload.
            check_expr(scrutinee, tracker, env, file, emitter, true);
            check_match_arms(arms, tracker, env, file, emitter);
        }
        ExprKind::While {
            condition, body, ..
        } => {
            check_expr(condition, tracker, env, file, emitter, false);
            check_loop_body(body, true, tracker, env, file, emitter);
        }
        ExprKind::ForEach { iterable, body, .. } => {
            check_expr(iterable, tracker, env, file, emitter, false);
            check_loop_body(body, true, tracker, env, file, emitter);
        }
        ExprKind::Loop { body, .. } => {
            check_loop_body(body, false, tracker, env, file, emitter);
        }
        ExprKind::Block(block) => check_block(block, tracker, env, file, emitter),
        ExprKind::Assign { lhs, rhs } => {
            check_expr(lhs, tracker, env, file, emitter, false);
            check_expr(rhs, tracker, env, file, emitter, true);
            // The RHS is visited first so `h = wrap(h)` consumes the old value before re-arming.
            if let ExprKind::Ident(name) = &lhs.kind
                && let Some(state) = tracker.vars.get_mut(name)
            {
                let depth = match *state {
                    ConsumeState::Available(_, depth) => depth,
                    _ => tracker.loop_depth,
                };
                *state = ConsumeState::Available(lhs.span, depth);
            }
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

fn check_loop_body(
    body: &Block,
    may_skip: bool,
    tracker: &mut LinearTracker,
    env: &TypeEnv,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    let consumed_before: Vec<(String, Span)> = tracker
        .vars
        .iter()
        .filter(|_| may_skip)
        .filter_map(|(name, state)| {
            state_may_be_consumed(Some(state)).map(|span| (name.clone(), span))
        })
        .collect();
    tracker.loop_depth += 1;
    check_block(body, tracker, env, file, emitter);
    tracker.loop_depth -= 1;
    // A body that may run zero times cannot re-arm a value consumed before the loop.
    for (name, span) in consumed_before {
        if let Some(state @ ConsumeState::Available(..)) = tracker.vars.get_mut(&name) {
            *state = ConsumeState::MaybeConsumed(span);
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
        None => return,
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
            return;
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
        Some(ConsumeState::Available(_, decl_depth)) => {
            if tracker.loop_depth > *decl_depth {
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
        }
    }
    tracker
        .vars
        .insert(name.to_string(), ConsumeState::Consumed(span));
}

fn merge_branch_state(
    left: Option<&ConsumeState>,
    right: Option<&ConsumeState>,
) -> Option<ConsumeState> {
    match (left, right) {
        (Some(ConsumeState::Consumed(span)), Some(ConsumeState::Consumed(_))) => {
            Some(ConsumeState::Consumed(*span))
        }
        (Some(ConsumeState::Available(span, l)), Some(ConsumeState::Available(_, r))) => {
            Some(ConsumeState::Available(*span, *l.min(r)))
        }
        (Some(ConsumeState::MaybeConsumed(span)), _)
        | (_, Some(ConsumeState::MaybeConsumed(span))) => Some(ConsumeState::MaybeConsumed(*span)),
        (Some(ConsumeState::Consumed(span)), Some(ConsumeState::Available(..)))
        | (Some(ConsumeState::Available(..)), Some(ConsumeState::Consumed(span))) => {
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
    let mut then_tracker = tracker.fork();
    let mut else_tracker = tracker.fork();

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
                && matches!(tracker.vars.get(name), Some(ConsumeState::Available(..)))
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
            let mut arm_tracker = tracker.fork();
            let mut bound_names = vec![];
            collect_pattern_binding_names(&arm.pattern, &mut bound_names);
            bound_names.sort();
            bound_names.dedup();
            let hidden: Vec<(String, Option<ConsumeState>)> = bound_names
                .into_iter()
                .map(|name| {
                    let state = arm_tracker.vars.remove(&name);
                    (name, state)
                })
                .collect();
            check_expr(&arm.body, &mut arm_tracker, env, file, emitter, true);
            for (name, state) in hidden {
                if let Some(state) = state {
                    arm_tracker.vars.insert(name, state);
                } else {
                    arm_tracker.vars.remove(&name);
                }
            }
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

fn collect_pattern_binding_names(pat: &Pat, names: &mut Vec<String>) {
    match &pat.kind {
        PatKind::Ident { name, .. } => names.push(name.clone()),
        PatKind::Tuple(patterns) | PatKind::Or(patterns) => {
            for pattern in patterns {
                collect_pattern_binding_names(pattern, names);
            }
        }
        PatKind::Struct { fields, .. } => {
            for (_, pattern) in fields {
                collect_pattern_binding_names(pattern, names);
            }
        }
        PatKind::EnumVariant { inner, .. } => {
            for pattern in inner {
                collect_pattern_binding_names(pattern, names);
            }
        }
        PatKind::Wildcard | PatKind::Lit(_) => {}
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

    use crate::env::{EnumInfo, StructInfo, TypeEnv, VariantInfo};

    struct TestEmitter(Vec<Diagnostic>);

    impl DiagnosticEmitter for TestEmitter {
        fn try_emit(&mut self, d: &Diagnostic) -> std::io::Result<()> {
            self.0.push(d.clone());
            Ok(())
        }

        fn try_finish(&mut self) -> std::io::Result<()> {
            Ok(())
        }
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

    #[test]
    fn enum_like_wrappers_inherit_owned_linearity_only() {
        let mut env = make_env_with_linear_struct("Token");
        env.define_enum(
            "Payload",
            EnumInfo {
                variants: vec![VariantInfo {
                    name: "Token".to_string(),
                    kind: VariantKind::Tuple(vec![Ty::Struct("Token".to_string())]),
                }],
            },
        );
        let payload = Ty::Enum("Payload".to_string());
        let nested = Ty::Applied(
            Box::new(Ty::Enum("Option".to_string())),
            vec![payload.clone()],
        );
        let borrowed = Ty::Reference(Box::new(Ty::Struct("Token".to_string())));
        let vector = Ty::Applied(
            Box::new(Ty::Struct("Vec".to_string())),
            vec![Ty::Struct("Token".to_string())],
        );

        assert!(is_linear_owner_ty(&payload, &env));
        assert!(is_linear_owner_ty(&nested, &env));
        assert!(!is_linear_owner_ty(&borrowed, &env));
        assert!(!is_linear_owner_ty(&vector, &env));
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

    fn block_with_stmts(stmts: Vec<Stmt>) -> Block {
        Block {
            stmts,
            trailing_expr: None,
            span: dummy_span(),
        }
    }

    fn block_expr(stmts: Vec<Stmt>) -> Expr {
        Expr {
            kind: ExprKind::Block(Box::new(block_with_stmts(stmts))),
            span: dummy_span(),
        }
    }

    fn true_expr() -> Expr {
        Expr {
            kind: ExprKind::Lit(Lit::Bool(true)),
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

    fn let_stmt(name: &str, ty: &str) -> Stmt {
        Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: name.to_string(),
                    is_mut: false,
                },
                span: dummy_span(),
            },
            ty: Some(named_type(ty)),
            init: Box::new(ident_expr("open")),
            span: dummy_span(),
        }
    }

    fn let_linear(name: &str) -> Stmt {
        let_stmt(name, "FileHandle")
    }

    fn expr_stmt(expr: Expr) -> Stmt {
        Stmt::Expr {
            expr,
            has_semicolon: true,
            span: dummy_span(),
        }
    }

    fn loop_expr(kind: &str, body: Block) -> Expr {
        let body = Box::new(body);
        let cond = || {
            Box::new(Expr {
                kind: ExprKind::Lit(Lit::Bool(true)),
                span: dummy_span(),
            })
        };
        let kind = match kind {
            "loop" => ExprKind::Loop { vow: None, body },
            "while" => ExprKind::While {
                condition: cond(),
                vow: None,
                body,
            },
            "for" => ExprKind::ForEach {
                binding: "i".to_string(),
                iterable: cond(),
                vow: None,
                body,
            },
            other => panic!("unknown loop kind {other}"),
        };
        Expr {
            kind,
            span: dummy_span(),
        }
    }

    fn stmts_block(stmts: Vec<Stmt>) -> Block {
        Block {
            stmts,
            trailing_expr: None,
            span: dummy_span(),
        }
    }

    fn check_errors(body: Block) -> Vec<Diagnostic> {
        let env = make_env_with_linear_struct("FileHandle");
        let fn_def = make_fn_def(vec![], body);
        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);
        emitter.0
    }

    #[test]
    fn test_loop_local_linear_create_and_consume_ok() {
        for kind in ["loop", "while", "for"] {
            let inner = stmts_block(vec![let_linear("h"), expr_stmt(call_with("consume", "h"))]);
            let errors = check_errors(stmts_block(vec![expr_stmt(loop_expr(kind, inner))]));
            assert!(errors.is_empty(), "{kind}: {errors:?}");
        }
    }

    #[test]
    fn test_loop_local_double_consume_still_error() {
        let inner = stmts_block(vec![
            let_linear("h"),
            expr_stmt(call_with("consume", "h")),
            expr_stmt(call_with("consume", "h")),
        ]);
        let errors = check_errors(stmts_block(vec![expr_stmt(loop_expr("loop", inner))]));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].message.contains("already consumed"));
    }

    #[test]
    fn test_outer_loop_decl_consumed_in_inner_loop_error() {
        let innermost = stmts_block(vec![expr_stmt(call_with("consume", "h"))]);
        let inner = stmts_block(vec![
            let_linear("h"),
            expr_stmt(loop_expr("loop", innermost)),
        ]);
        let errors = check_errors(stmts_block(vec![expr_stmt(loop_expr("loop", inner))]));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].message.contains("loop"));
    }

    #[test]
    fn test_loop_local_shadowing_outer_linear_keeps_outer_available() {
        let inner = stmts_block(vec![let_linear("h"), expr_stmt(call_with("consume", "h"))]);
        let errors = check_errors(stmts_block(vec![
            let_linear("h"),
            expr_stmt(loop_expr("loop", inner)),
            expr_stmt(call_with("consume", "h")),
        ]));
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn test_loop_local_name_reused_by_non_linear_binding_after_loop() {
        let inner = stmts_block(vec![let_linear("h"), expr_stmt(call_with("consume", "h"))]);
        let errors = check_errors(stmts_block(vec![
            expr_stmt(loop_expr("loop", inner)),
            let_stmt("h", "i64"),
            expr_stmt(call_with("use_it", "h")),
        ]));
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn test_unconsumed_loop_local_still_reported() {
        let inner = stmts_block(vec![let_linear("h")]);
        let errors = check_errors(stmts_block(vec![expr_stmt(loop_expr("loop", inner))]));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].message.contains("never consumed"));
    }

    #[test]
    fn test_outer_decl_consumed_in_loop_error() {
        let inner = stmts_block(vec![expr_stmt(call_with("consume", "h"))]);
        let errors = check_errors(stmts_block(vec![
            let_linear("h"),
            expr_stmt(loop_expr("while", inner)),
        ]));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].message.contains("loop"));
    }

    #[test]
    fn test_reference_parameter_passed_on_does_not_consume() {
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param(
            "h",
            Type::Reference {
                inner: Box::new(named_type("FileHandle")),
                span: dummy_span(),
            },
        )];
        let body = Block {
            stmts: vec![Stmt::Expr {
                expr: call_with("inspect", "h"),
                has_semicolon: true,
                span: dummy_span(),
            }],
            trailing_expr: Some(Box::new(call_with("inspect", "h"))),
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

        // `return;` doesn't consume `h`, but the param case is left to the
        // region pass — the AST backstop only flags non-param let-bindings.
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

    #[test]
    fn test_match_binding_shadows_consumed_scrutinee() {
        let env = make_env_with_linear_struct("Token");
        let mut tracker = LinearTracker::new();
        tracker
            .vars
            .insert("value".to_string(), ConsumeState::Consumed(dummy_span()));
        let arms = vec![MatchArm {
            pattern: Pat {
                kind: PatKind::EnumVariant {
                    path: vec!["Option".to_string(), "Some".to_string()],
                    inner: vec![Pat {
                        kind: PatKind::Ident {
                            name: "value".to_string(),
                            is_mut: false,
                        },
                        span: dummy_span(),
                    }],
                },
                span: dummy_span(),
            },
            body: call_with("consume", "value"),
            span: dummy_span(),
        }];

        let mut emitter = TestEmitter(vec![]);
        check_match_arms(&arms, &mut tracker, &env, "test.vow", &mut emitter);

        assert!(emitter.0.is_empty(), "Got: {:?}", emitter.0);
        assert!(matches!(
            tracker.vars.get("value"),
            Some(ConsumeState::Consumed(_))
        ));
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
    fn test_linear_never_consumed_param_skipped() {
        // The backstop intentionally skips params — region pass catches them.
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let body = empty_block();
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert!(emitter.0.is_empty(), "Got: {:?}", emitter.0);
    }

    #[test]
    fn test_shadowed_param_let_still_flagged() {
        // `fn(h: Handle) { let h: Handle = ...; 0 }` — the let binding shadows
        // the param; the backstop must still report the shadowing let by span,
        // not skip the entry just because its name matches a param name.
        let env = make_env_with_linear_struct("FileHandle");
        let params = vec![make_param("h", named_type("FileHandle"))];
        let let_stmt = Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: "h".to_string(),
                    is_mut: false,
                },
                span: Span::new(100, 1),
            },
            ty: Some(named_type("FileHandle")),
            init: Box::new(ident_expr("open")),
            span: Span::new(100, 1),
        };
        let body = Block {
            stmts: vec![let_stmt],
            trailing_expr: None,
            span: dummy_span(),
        };
        let fn_def = make_fn_def(params, body);

        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);

        assert_eq!(emitter.0.len(), 1, "Got: {:?}", emitter.0);
        assert!(emitter.0[0].message.contains("never consumed"));
        // The diagnostic must point at the let span, not the param span.
        assert_eq!(emitter.0[0].primary.byte_offset, 100);
    }

    #[test]
    fn test_let_stmt_linear_never_consumed_error() {
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

        assert_eq!(emitter.0.len(), 1);
        assert!(emitter.0[0].message.contains("never consumed"));
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
        // Note: h is registered at loop depth 0, outside the loop body
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

    fn let_local(name: &str, ty: Type, init: &str) -> Stmt {
        Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: name.to_string(),
                    is_mut: true,
                },
                span: dummy_span(),
            },
            ty: Some(ty),
            init: Box::new(ident_expr(init)),
            span: dummy_span(),
        }
    }

    fn assign_expr(lhs: Expr, rhs: Expr) -> Expr {
        Expr {
            kind: ExprKind::Assign {
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            },
            span: dummy_span(),
        }
    }

    fn assign_stmt(name: &str, rhs: &str) -> Stmt {
        expr_stmt(assign_expr(ident_expr(name), ident_expr(rhs)))
    }

    fn if_expr(then_stmts: Vec<Stmt>, else_stmts: Option<Vec<Stmt>>) -> Expr {
        Expr {
            kind: ExprKind::If {
                condition: Box::new(true_expr()),
                then_branch: Box::new(block_with_stmts(then_stmts)),
                else_branch: else_stmts.map(|stmts| Box::new(block_expr(stmts))),
            },
            span: dummy_span(),
        }
    }

    fn run_linear(stmts: Vec<Stmt>) -> Vec<Diagnostic> {
        let env = make_env_with_linear_struct("FileHandle");
        let fn_def = make_fn_def(vec![], block_with_stmts(stmts));
        let mut emitter = TestEmitter(vec![]);
        check_linear_usage(&fn_def, &env, "test.vow", &mut emitter);
        emitter.0
    }

    fn consume_stmt() -> Stmt {
        expr_stmt(call_with("consume", "h"))
    }

    #[test]
    fn test_assign_after_consume_rearms_linear() {
        let diags = run_linear(vec![
            let_local("h", named_type("FileHandle"), "open"),
            consume_stmt(),
            assign_stmt("h", "open"),
            consume_stmt(),
        ]);

        assert!(diags.is_empty(), "Got: {diags:?}");
    }

    #[test]
    fn test_assign_self_wrap_rearms_once() {
        let wrap_assign = expr_stmt(assign_expr(ident_expr("h"), call_with("wrap", "h")));
        let ok = run_linear(vec![
            let_local("h", named_type("FileHandle"), "open"),
            wrap_assign.clone(),
            consume_stmt(),
        ]);
        assert!(ok.is_empty(), "Got: {ok:?}");

        let leaked = run_linear(vec![
            let_local("h", named_type("FileHandle"), "open"),
            wrap_assign,
        ]);
        assert_eq!(leaked.len(), 1, "Got: {leaked:?}");
        assert!(leaked[0].message.contains("never consumed"));
    }

    #[test]
    fn test_assign_over_available_is_deferred_to_region_check() {
        let diags = run_linear(vec![
            let_local("h", named_type("FileHandle"), "open"),
            assign_stmt("h", "open2"),
            consume_stmt(),
        ]);

        assert!(diags.is_empty(), "Got: {diags:?}");
    }

    #[test]
    fn test_assign_in_else_less_if_after_consume_stays_consumed() {
        let diags = run_linear(vec![
            let_local("h", named_type("FileHandle"), "open"),
            consume_stmt(),
            expr_stmt(if_expr(vec![assign_stmt("h", "open")], None)),
            consume_stmt(),
        ]);

        assert_eq!(diags.len(), 1, "Got: {diags:?}");
        assert!(diags[0].message.contains("already consumed"));
    }

    #[test]
    fn test_assign_in_one_arm_with_else_after_consume_is_maybe_consumed() {
        let diags = run_linear(vec![
            let_local("h", named_type("FileHandle"), "open"),
            consume_stmt(),
            expr_stmt(if_expr(vec![assign_stmt("h", "open")], Some(vec![]))),
            consume_stmt(),
        ]);

        assert_eq!(diags.len(), 1, "Got: {diags:?}");
        assert!(diags[0].message.contains("may already be consumed"));
    }

    #[test]
    fn test_assign_in_both_branches_after_consume_rearms() {
        let diags = run_linear(vec![
            let_local("h", named_type("FileHandle"), "open"),
            consume_stmt(),
            expr_stmt(if_expr(
                vec![assign_stmt("h", "open")],
                Some(vec![assign_stmt("h", "open")]),
            )),
            consume_stmt(),
        ]);

        assert!(diags.is_empty(), "Got: {diags:?}");
    }

    #[test]
    fn test_field_assign_does_not_rearm() {
        let field_assign = expr_stmt(assign_expr(
            Expr {
                kind: ExprKind::FieldAccess {
                    base: Box::new(ident_expr("h")),
                    field: "fd".to_string(),
                },
                span: dummy_span(),
            },
            Expr {
                kind: ExprKind::Lit(Lit::Int(1)),
                span: dummy_span(),
            },
        ));
        let diags = run_linear(vec![
            let_local("h", named_type("FileHandle"), "open"),
            consume_stmt(),
            field_assign,
            consume_stmt(),
        ]);

        assert_eq!(diags.len(), 1, "Got: {diags:?}");
        assert!(diags[0].message.contains("already consumed"));
    }

    #[test]
    fn test_shadowing_nonlinear_let_over_consumed_linear_not_rearmed() {
        let diags = run_linear(vec![
            let_local("h", named_type("FileHandle"), "open"),
            consume_stmt(),
            let_local("h", named_type("i64"), "zero"),
            assign_stmt("h", "one"),
        ]);

        assert!(diags.is_empty(), "Got: {diags:?}");
    }

    fn while_expr(stmts: Vec<Stmt>) -> Expr {
        Expr {
            kind: ExprKind::While {
                condition: Box::new(true_expr()),
                vow: None,
                body: Box::new(block_with_stmts(stmts)),
            },
            span: dummy_span(),
        }
    }

    #[test]
    fn test_tuple_let_shadow_over_consumed_linear_not_rearmed() {
        let tuple_let = Stmt::Let {
            pattern: Pat {
                kind: PatKind::Tuple(vec![
                    Pat {
                        kind: PatKind::Ident {
                            name: "h".to_string(),
                            is_mut: true,
                        },
                        span: dummy_span(),
                    },
                    Pat {
                        kind: PatKind::Ident {
                            name: "y".to_string(),
                            is_mut: false,
                        },
                        span: dummy_span(),
                    },
                ]),
                span: dummy_span(),
            },
            ty: None,
            init: Box::new(ident_expr("zero")),
            span: dummy_span(),
        };
        let diags = run_linear(vec![
            let_local("h", named_type("FileHandle"), "open"),
            consume_stmt(),
            tuple_let,
            assign_stmt("h", "one"),
        ]);

        assert!(diags.is_empty(), "Got: {diags:?}");
    }

    #[test]
    fn test_assign_in_while_after_consume_is_maybe_consumed_after_loop() {
        let diags = run_linear(vec![
            let_local("h", named_type("FileHandle"), "open"),
            consume_stmt(),
            expr_stmt(while_expr(vec![assign_stmt("h", "open")])),
            consume_stmt(),
        ]);

        assert_eq!(diags.len(), 1, "Got: {diags:?}");
        assert!(diags[0].message.contains("may already be consumed"));
    }

    #[test]
    fn test_inner_block_shadow_does_not_forget_outer_consumed_linear() {
        let inner = expr_stmt(block_expr(vec![let_local("h", named_type("i64"), "zero")]));
        let diags = run_linear(vec![
            let_local("h", named_type("FileHandle"), "open"),
            consume_stmt(),
            inner,
            consume_stmt(),
        ]);

        assert_eq!(diags.len(), 1, "Got: {diags:?}");
        assert!(diags[0].message.contains("already consumed"));
    }

    #[test]
    fn test_reassign_then_consume_inside_loop_after_consume_ok() {
        let diags = run_linear(vec![
            let_local("h", named_type("FileHandle"), "open"),
            consume_stmt(),
            expr_stmt(while_expr(vec![assign_stmt("h", "open"), consume_stmt()])),
        ]);

        assert!(diags.is_empty(), "Got: {diags:?}");
    }

    #[test]
    fn test_consume_in_loop_then_reassign_still_reports_loop_consume() {
        let diags = run_linear(vec![
            let_local("h", named_type("FileHandle"), "open"),
            expr_stmt(while_expr(vec![consume_stmt(), assign_stmt("h", "open")])),
            consume_stmt(),
        ]);

        assert_eq!(diags.len(), 1, "Got: {diags:?}");
        assert!(diags[0].message.contains("inside a loop"));
    }

    #[test]
    fn test_inner_linear_then_non_linear_shadow_keeps_outer_available() {
        let diags = run_linear(vec![
            let_local("h", named_type("FileHandle"), "open"),
            expr_stmt(block_expr(vec![
                let_local("h", named_type("FileHandle"), "open"),
                consume_stmt(),
                let_local("h", named_type("i64"), "zero"),
            ])),
            consume_stmt(),
        ]);

        assert!(diags.is_empty(), "Got: {diags:?}");
    }
}
