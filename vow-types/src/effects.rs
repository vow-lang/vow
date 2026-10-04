use std::collections::BTreeMap;

use vow_diag::{Blame, Diagnostic, DiagnosticEmitter, ErrorCode, Severity, SourceLocation};
use vow_syntax::ast::{Block, Effect, Expr, ExprKind, FnDef, Param, Stmt, VowBlock, VowClause};

use crate::env::TypeEnv;

/// Builtin methods that never write through their receiver or arguments.
/// Allowlist, not a denylist: a builtin method not on this list is
/// write-suspect until proven otherwise, so a newly added mutating method
/// ships guarded by default instead of silently slipping through. Exhaustive
/// against `check.rs`'s `builtin_method_names` (`Str`, `Vec`, `HashMap`,
/// `BTreeMap`, `Option`/`Result` — the only receivers with builtin methods;
/// Vow has no user-defined methods).
const READ_ONLY_METHODS: &[&str] = &[
    "len",
    "get",
    "eq",
    "contains",
    "contains_key",
    "byte_at",
    "substring",
    "parse_i64",
    "parse_u64",
    "unwrap",
];

/// Collects every sub-expression of `expr` that, when evaluated, performs a
/// write reachable from a parameter: a `FieldAccess`/`Index` assignment
/// target, a builtin method call not on `READ_ONLY_METHODS`, or a call to a
/// function whose own may-write bit (from `env`, see `compute_may_write_table`)
/// is set or unknown. Shares `collect_calls_in_expr`'s traversal shape so a
/// new `ExprKind` variant can't silently escape one walker but not the other.
///
/// A `Call` whose callee cannot be resolved at all in `env` fails closed (is
/// treated as a write) — this only happens for a callee name with no
/// registered `FnSig`, since every builtin free function and every
/// non-declaration module function is registered before this runs.
fn collect_may_write_sites<'a>(expr: &'a Expr, env: &TypeEnv, sites: &mut Vec<&'a Expr>) {
    match &expr.kind {
        ExprKind::Assign { lhs, rhs } => {
            if matches!(
                lhs.kind,
                ExprKind::FieldAccess { .. } | ExprKind::Index { .. }
            ) {
                sites.push(expr);
            }
            collect_may_write_sites(lhs, env, sites);
            collect_may_write_sites(rhs, env, sites);
        }
        ExprKind::MethodCall {
            receiver,
            method,
            args,
        } => {
            if !READ_ONLY_METHODS.contains(&method.as_str()) {
                sites.push(expr);
            }
            collect_may_write_sites(receiver, env, sites);
            for arg in args {
                collect_may_write_sites(arg, env, sites);
            }
        }
        ExprKind::Call { callee, args } => {
            let may_write = match &callee.kind {
                ExprKind::Ident(name) => match env.may_write(name) {
                    Some(bit) => bit,
                    None => env.lookup_fn(name).is_none(),
                },
                _ => true,
            };
            if may_write {
                sites.push(expr);
            }
            collect_may_write_sites(callee, env, sites);
            for arg in args {
                collect_may_write_sites(arg, env, sites);
            }
        }
        ExprKind::BinaryOp { lhs, rhs, .. } => {
            collect_may_write_sites(lhs, env, sites);
            collect_may_write_sites(rhs, env, sites);
        }
        ExprKind::UnaryOp { operand, .. } => collect_may_write_sites(operand, env, sites),
        ExprKind::Block(block) => collect_may_write_sites_in_block(block, env, sites),
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_may_write_sites(condition, env, sites);
            collect_may_write_sites_in_block(then_branch, env, sites);
            if let Some(e) = else_branch {
                collect_may_write_sites(e, env, sites);
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            collect_may_write_sites(scrutinee, env, sites);
            for arm in arms {
                collect_may_write_sites(&arm.body, env, sites);
            }
        }
        ExprKind::While {
            condition, body, ..
        } => {
            collect_may_write_sites(condition, env, sites);
            collect_may_write_sites_in_block(body, env, sites);
        }
        ExprKind::ForEach { iterable, body, .. } => {
            collect_may_write_sites(iterable, env, sites);
            collect_may_write_sites_in_block(body, env, sites);
        }
        ExprKind::Loop { body, .. } => collect_may_write_sites_in_block(body, env, sites),
        ExprKind::Return { value } => {
            if let Some(v) = value {
                collect_may_write_sites(v, env, sites);
            }
        }
        ExprKind::Borrow { expr } | ExprKind::Question { expr } => {
            collect_may_write_sites(expr, env, sites);
        }
        ExprKind::FieldAccess { base, .. } => {
            collect_may_write_sites(base, env, sites);
        }
        ExprKind::Index { base, index } => {
            collect_may_write_sites(base, env, sites);
            collect_may_write_sites(index, env, sites);
        }
        ExprKind::Tuple(exprs) => {
            for e in exprs {
                collect_may_write_sites(e, env, sites);
            }
        }
        ExprKind::Break { value } => {
            if let Some(v) = value {
                collect_may_write_sites(v, env, sites);
            }
        }
        ExprKind::Continue => {}
        ExprKind::Lit(_) | ExprKind::Ident(_) | ExprKind::Result => {}
        ExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                collect_may_write_sites(e, env, sites);
            }
        }
        ExprKind::EnumConstruct { fields, .. } => {
            for e in fields {
                collect_may_write_sites(e, env, sites);
            }
        }
        ExprKind::Cast { expr, .. } => collect_may_write_sites(expr, env, sites),
    }
}

fn collect_may_write_sites_in_block<'a>(
    block: &'a Block,
    env: &TypeEnv,
    sites: &mut Vec<&'a Expr>,
) {
    for stmt in &block.stmts {
        match stmt {
            Stmt::Let { init, .. } => collect_may_write_sites(init, env, sites),
            Stmt::Expr { expr, .. } => collect_may_write_sites(expr, env, sites),
        }
    }
    if let Some(e) = &block.trailing_expr {
        collect_may_write_sites(e, env, sites);
    }
}

/// Computes, for every function in `fn_defs` (declarations included — the
/// caller must pass every `Item::Fn`, not just the ones with a real body),
/// whether calling it can perform a write reachable from its own parameters —
/// a monotone fixed point over the call graph (`false` → `true` only), so
/// (mutual) recursion is handled by construction: a cycle with one writing
/// member taints the whole cycle, and the loop terminates because the
/// lattice is 2-valued over a finite function set. A declaration
/// (`fn f(..) -> T;`, no body — the `.vow.d` stub form `use` can load instead
/// of a module's real source, see `tests/multi/decl_stub_preference/`) seeds
/// straight to `true`: its body is a parsed-but-empty placeholder, so
/// analyzing it would find no writes and fail *open* on a function this
/// translation unit has no definition for. Iterates `fn_defs` in
/// caller-supplied order (the module's own item order), not a `HashMap`, so
/// the fixed point is deterministic across compiler invocations. Mutates
/// `env`'s table bit-by-bit as each write is discovered (mirroring the
/// self-hosted `e.may_write_fns[fidx] = 1` in-place update) rather than
/// cloning and reinstalling the whole table every round — sound because a
/// bit only ever flips `false` → `true`, so a lookup seeing this round's
/// partial progress for an earlier-processed function in the same round is
/// still a safe under-approximation, not a correctness hazard.
pub fn compute_may_write_table(fn_defs: &[&FnDef], env: &mut TypeEnv) {
    let initial: BTreeMap<String, bool> = fn_defs
        .iter()
        .map(|f| (f.name.clone(), f.is_declaration))
        .collect();
    env.install_may_write_table(initial);
    loop {
        let mut changed = false;
        for f in fn_defs {
            if f.is_declaration || env.may_write(&f.name) == Some(true) {
                continue;
            }
            let mut sites = Vec::new();
            collect_may_write_sites_in_block(&f.body, env, &mut sites);
            if !sites.is_empty() {
                env.set_may_write(&f.name, true);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// Collects the loop-invariant `vow` blocks of every `while`/`for`/`loop` in
/// `block`, however deeply nested. `check_fn_effects` only ever purity-checks
/// `fn_def.vow` (the function-level contract) directly — nothing walks into a
/// loop's own `vow` field, so `check_vow_purity` is silently inert on
/// `while c vow { invariant: .. } { .. }` today, for both the pre-existing
/// declared-effect check and the new may-write check. This is that missing
/// traversal, kept separate from `collect_may_write_sites` because it
/// produces a different item (vow blocks to re-check, not write sites).
fn collect_loop_vows<'a>(block: &'a Block, out: &mut Vec<&'a VowBlock>) {
    for stmt in &block.stmts {
        match stmt {
            Stmt::Let { init, .. } => collect_loop_vows_in_expr(init, out),
            Stmt::Expr { expr, .. } => collect_loop_vows_in_expr(expr, out),
        }
    }
    if let Some(e) = &block.trailing_expr {
        collect_loop_vows_in_expr(e, out);
    }
}

fn collect_loop_vows_in_expr<'a>(expr: &'a Expr, out: &mut Vec<&'a VowBlock>) {
    match &expr.kind {
        ExprKind::While {
            condition,
            body,
            vow,
        } => {
            if let Some(v) = vow {
                out.push(v);
            }
            collect_loop_vows_in_expr(condition, out);
            collect_loop_vows(body, out);
        }
        ExprKind::ForEach {
            iterable,
            body,
            vow,
            ..
        } => {
            if let Some(v) = vow {
                out.push(v);
            }
            collect_loop_vows_in_expr(iterable, out);
            collect_loop_vows(body, out);
        }
        ExprKind::Loop { body, vow } => {
            if let Some(v) = vow {
                out.push(v);
            }
            collect_loop_vows(body, out);
        }
        ExprKind::BinaryOp { lhs, rhs, .. } => {
            collect_loop_vows_in_expr(lhs, out);
            collect_loop_vows_in_expr(rhs, out);
        }
        ExprKind::UnaryOp { operand, .. } => collect_loop_vows_in_expr(operand, out),
        ExprKind::Call { callee, args } => {
            collect_loop_vows_in_expr(callee, out);
            for arg in args {
                collect_loop_vows_in_expr(arg, out);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            collect_loop_vows_in_expr(receiver, out);
            for arg in args {
                collect_loop_vows_in_expr(arg, out);
            }
        }
        ExprKind::Block(block) => collect_loop_vows(block, out),
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_loop_vows_in_expr(condition, out);
            collect_loop_vows(then_branch, out);
            if let Some(e) = else_branch {
                collect_loop_vows_in_expr(e, out);
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            collect_loop_vows_in_expr(scrutinee, out);
            for arm in arms {
                collect_loop_vows_in_expr(&arm.body, out);
            }
        }
        ExprKind::Return { value } => {
            if let Some(v) = value {
                collect_loop_vows_in_expr(v, out);
            }
        }
        ExprKind::Borrow { expr } | ExprKind::Question { expr } => {
            collect_loop_vows_in_expr(expr, out);
        }
        ExprKind::FieldAccess { base, .. } => collect_loop_vows_in_expr(base, out),
        ExprKind::Index { base, index } => {
            collect_loop_vows_in_expr(base, out);
            collect_loop_vows_in_expr(index, out);
        }
        ExprKind::Assign { lhs, rhs } => {
            collect_loop_vows_in_expr(lhs, out);
            collect_loop_vows_in_expr(rhs, out);
        }
        ExprKind::Tuple(exprs) => {
            for e in exprs {
                collect_loop_vows_in_expr(e, out);
            }
        }
        ExprKind::Break { value } => {
            if let Some(v) = value {
                collect_loop_vows_in_expr(v, out);
            }
        }
        ExprKind::Continue => {}
        ExprKind::Lit(_) | ExprKind::Ident(_) | ExprKind::Result => {}
        ExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                collect_loop_vows_in_expr(e, out);
            }
        }
        ExprKind::EnumConstruct { fields, .. } => {
            for e in fields {
                collect_loop_vows_in_expr(e, out);
            }
        }
        ExprKind::Cast { expr, .. } => collect_loop_vows_in_expr(expr, out),
    }
}

fn effect_covered(declared: &[Effect], needed: &Effect) -> bool {
    if declared.contains(needed) {
        return true;
    }
    if (needed == &Effect::Read || needed == &Effect::Write) && declared.contains(&Effect::IO) {
        return true;
    }
    false
}

fn collect_calls_in_expr<'a>(
    expr: &'a Expr,
    calls: &mut Vec<(&'a Expr, &'a str)>,
    panic_exprs: &mut Vec<&'a Expr>,
) {
    match &expr.kind {
        ExprKind::Call { callee, args } => {
            if let ExprKind::Ident(name) = &callee.kind {
                calls.push((callee, name.as_str()));
            }
            for arg in args {
                collect_calls_in_expr(arg, calls, panic_exprs);
            }
        }
        ExprKind::BinaryOp { lhs, rhs, .. } => {
            collect_calls_in_expr(lhs, calls, panic_exprs);
            collect_calls_in_expr(rhs, calls, panic_exprs);
        }
        ExprKind::UnaryOp { operand, .. } => collect_calls_in_expr(operand, calls, panic_exprs),
        ExprKind::MethodCall {
            receiver,
            method,
            args,
        } => {
            collect_calls_in_expr(receiver, calls, panic_exprs);
            for arg in args {
                collect_calls_in_expr(arg, calls, panic_exprs);
            }
            if method == "unwrap" {
                panic_exprs.push(expr);
            }
        }
        ExprKind::Block(block) => collect_calls_in_block(block, calls, panic_exprs),
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_calls_in_expr(condition, calls, panic_exprs);
            collect_calls_in_block(then_branch, calls, panic_exprs);
            if let Some(e) = else_branch {
                collect_calls_in_expr(e, calls, panic_exprs);
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            collect_calls_in_expr(scrutinee, calls, panic_exprs);
            for arm in arms {
                collect_calls_in_expr(&arm.body, calls, panic_exprs);
            }
        }
        ExprKind::While {
            condition, body, ..
        } => {
            collect_calls_in_expr(condition, calls, panic_exprs);
            collect_calls_in_block(body, calls, panic_exprs);
        }
        ExprKind::ForEach { iterable, body, .. } => {
            collect_calls_in_expr(iterable, calls, panic_exprs);
            collect_calls_in_block(body, calls, panic_exprs);
        }
        ExprKind::Loop { body, .. } => collect_calls_in_block(body, calls, panic_exprs),
        ExprKind::Return { value } => {
            if let Some(v) = value {
                collect_calls_in_expr(v, calls, panic_exprs);
            }
        }
        ExprKind::Borrow { expr } | ExprKind::Question { expr } => {
            collect_calls_in_expr(expr, calls, panic_exprs);
        }
        ExprKind::FieldAccess { base, .. } => {
            collect_calls_in_expr(base, calls, panic_exprs);
        }
        ExprKind::Index { base, index } => {
            collect_calls_in_expr(base, calls, panic_exprs);
            collect_calls_in_expr(index, calls, panic_exprs);
        }
        ExprKind::Assign { lhs, rhs } => {
            collect_calls_in_expr(lhs, calls, panic_exprs);
            collect_calls_in_expr(rhs, calls, panic_exprs);
        }
        ExprKind::Tuple(exprs) => {
            for e in exprs {
                collect_calls_in_expr(e, calls, panic_exprs);
            }
        }
        ExprKind::Break { value } => {
            if let Some(v) = value {
                collect_calls_in_expr(v, calls, panic_exprs);
            }
        }
        ExprKind::Continue => {}
        ExprKind::Lit(_) | ExprKind::Ident(_) | ExprKind::Result => {}
        ExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                collect_calls_in_expr(e, calls, panic_exprs);
            }
        }
        ExprKind::EnumConstruct { fields, .. } => {
            for e in fields {
                collect_calls_in_expr(e, calls, panic_exprs);
            }
        }
        ExprKind::Cast { expr, .. } => collect_calls_in_expr(expr, calls, panic_exprs),
    }
}

fn collect_calls_in_block<'a>(
    block: &'a Block,
    calls: &mut Vec<(&'a Expr, &'a str)>,
    panic_exprs: &mut Vec<&'a Expr>,
) {
    for stmt in &block.stmts {
        match stmt {
            Stmt::Let { init, .. } => collect_calls_in_expr(init, calls, panic_exprs),
            Stmt::Expr { expr, .. } => collect_calls_in_expr(expr, calls, panic_exprs),
        }
    }
    if let Some(e) = &block.trailing_expr {
        collect_calls_in_expr(e, calls, panic_exprs);
    }
}

fn effect_name(e: &Effect) -> &'static str {
    match e {
        Effect::Read => "Read",
        Effect::Write => "Write",
        Effect::IO => "IO",
        Effect::Panic => "Panic",
        Effect::Unsafe => "Unsafe",
    }
}

fn effects_display(effects: &[Effect]) -> String {
    let names: Vec<&str> = effects.iter().map(effect_name).collect();
    format!("[{}]", names.join(", "))
}

pub fn check_fn_effects(
    fn_def: &FnDef,
    env: &TypeEnv,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    let mut calls = Vec::new();
    let mut panic_exprs = Vec::new();
    collect_calls_in_block(&fn_def.body, &mut calls, &mut panic_exprs);

    for (callee_expr, callee_name) in &calls {
        if let Some(sig) = env.lookup_fn(callee_name) {
            for effect in &sig.effects {
                if !effect_covered(&fn_def.effects, effect) {
                    let msg = format!(
                        "function `{}` is declared with effects {} but calls `{}` which requires effect `{}`",
                        fn_def.name,
                        effects_display(&fn_def.effects),
                        callee_name,
                        effect_name(effect),
                    );
                    let hint = format!(
                        "add '{}' to `{}`'s effect list",
                        effect_name(effect),
                        fn_def.name,
                    );
                    emitter.emit(&Diagnostic {
                        severity: Severity::Error,
                        code: ErrorCode::EffectViolation,
                        message: msg,
                        primary: SourceLocation {
                            file: file.to_string(),
                            byte_offset: callee_expr.span.start,
                            byte_len: callee_expr.span.len,
                        },
                        secondary: vec![],
                        blame: Blame::None,
                        hints: vec![hint],
                    });
                }
            }
        }
    }

    if !panic_exprs.is_empty() && !effect_covered(&fn_def.effects, &Effect::Panic) {
        for panic_expr in &panic_exprs {
            let msg = format!(
                "function `{}` is declared with effects {} but calls `.unwrap()` which requires effect `Panic`",
                fn_def.name,
                effects_display(&fn_def.effects),
            );
            emitter.emit(&Diagnostic {
                severity: Severity::Error,
                code: ErrorCode::EffectViolation,
                message: msg,
                primary: SourceLocation {
                    file: file.to_string(),
                    byte_offset: panic_expr.span.start,
                    byte_len: panic_expr.span.len,
                },
                secondary: vec![],
                blame: Blame::None,
                hints: vec![format!(
                    "add 'Panic' to `{}`'s effect list, or use `?` to propagate the error instead",
                    fn_def.name,
                )],
            });
        }
    }

    if let Some(vow_block) = &fn_def.vow {
        check_vow_purity(vow_block, env, file, emitter);
    }

    let mut loop_vows = Vec::new();
    collect_loop_vows(&fn_def.body, &mut loop_vows);
    for vow_block in loop_vows {
        check_vow_purity(vow_block, env, file, emitter);
    }

    check_param_refinement_purity(&fn_def.params, env, file, emitter);
}

/// Checks a single predicate expression (a `requires`/`ensures`/`invariant`
/// clause, or a parameter `where` refinement) for purity: no effectful calls,
/// no writes through a shared argument. Shared by `check_vow_purity` (one call
/// per clause) and `check_param_refinement_purity` (one call per refinement).
fn check_expr_purity(expr: &Expr, env: &TypeEnv, file: &str, emitter: &mut dyn DiagnosticEmitter) {
    let mut calls = Vec::new();
    let mut panic_exprs = Vec::new();
    collect_calls_in_expr(expr, &mut calls, &mut panic_exprs);

    for (callee_expr, callee_name) in &calls {
        if let Some(sig) = env.lookup_fn(callee_name)
            && !sig.effects.is_empty()
        {
            emitter.emit(&Diagnostic {
                severity: Severity::Error,
                code: ErrorCode::EffectViolation,
                message: format!(
                    "vow predicate must be pure but calls effectful function `{}`",
                    callee_name,
                ),
                primary: SourceLocation {
                    file: file.to_string(),
                    byte_offset: callee_expr.span.start,
                    byte_len: callee_expr.span.len,
                },
                secondary: vec![],
                blame: Blame::Callee,
                hints: vec![
                    "vow predicates must be pure — move effectful code outside the vow block"
                        .to_string(),
                ],
            });
        }
    }

    let mut write_sites = Vec::new();
    collect_may_write_sites(expr, env, &mut write_sites);
    for site in write_sites {
        emitter.emit(&Diagnostic {
            severity: Severity::Error,
            code: ErrorCode::EffectViolation,
            message: "vow predicate must be pure but this expression may write through a shared argument".to_string(),
            primary: SourceLocation {
                file: file.to_string(),
                byte_offset: site.span.start,
                byte_len: site.span.len,
            },
            secondary: vec![],
            blame: Blame::Callee,
            hints: vec![
                "vow predicates must not write to a struct field, a Vec/map element, or call a mutating builtin method — move the write outside the vow block"
                    .to_string(),
            ],
        });
    }
}

pub fn check_vow_purity(
    vow_block: &VowBlock,
    env: &TypeEnv,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    for clause in &vow_block.clauses {
        let expr = match clause {
            VowClause::Requires { expr, .. } => expr,
            VowClause::Ensures { expr, .. } => expr,
            VowClause::Invariant { expr, .. } => expr,
        };
        check_expr_purity(expr, env, file, emitter);
    }
}

/// Parameter `where` refinements must be pure, like `requires`/`ensures`:
/// they are lowered unconditionally into an `__ESBMC_assume`, so an effectful
/// refinement would let a side effect run as a verifier-trusted assumption.
pub fn check_param_refinement_purity(
    params: &[Param],
    env: &TypeEnv,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    for param in params {
        if let Some(refinement) = &param.refinement {
            check_expr_purity(refinement, env, file, emitter);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use vow_diag::Diagnostic;
    use vow_syntax::ast::{
        BinOp, Block, Effect, Expr, ExprKind, FnDef, Lit, Param, Stmt, Type, UnOp, Visibility,
        VowBlock, VowClause,
    };
    use vow_syntax::span::Span;

    use crate::env::{FnSig, TypeEnv};
    use crate::types::Ty;

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
        Span::new(0, 0)
    }

    fn call_expr(name: &str) -> Expr {
        Expr {
            kind: ExprKind::Call {
                callee: Box::new(Expr {
                    kind: ExprKind::Ident(name.to_string()),
                    span: dummy_span(),
                }),
                args: vec![],
            },
            span: dummy_span(),
        }
    }

    fn simple_body(call_name: &str) -> Block {
        Block {
            stmts: vec![Stmt::Expr {
                expr: call_expr(call_name),
                has_semicolon: true,
                span: dummy_span(),
            }],
            trailing_expr: None,
            span: dummy_span(),
        }
    }

    fn make_fn(name: &str, effects: Vec<Effect>, body: Block) -> FnDef {
        FnDef {
            vis: Visibility::Private,
            name: name.to_string(),
            params: vec![],
            return_ty: Type::Unit { span: dummy_span() },
            effects,
            vow: None,
            body,
            span: dummy_span(),
            is_declaration: false,
        }
    }

    fn env_with_read_file() -> TypeEnv {
        let mut env = TypeEnv::new();
        env.define_fn(
            "read_file",
            FnSig {
                params: vec![],
                return_ty: Ty::Unit,
                effects: BTreeSet::from([Effect::Read]),
            },
        );
        env
    }

    #[test]
    fn pure_caller_calling_effectful_fn_emits_violation() {
        let env = env_with_read_file();
        let caller = make_fn("caller", vec![], simple_body("read_file"));
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert_eq!(emitter.0.len(), 1);
        assert_eq!(emitter.0[0].code, ErrorCode::EffectViolation);
        assert!(emitter.0[0].message.contains("read_file"));
    }

    #[test]
    fn effectful_caller_calling_effectful_fn_no_error() {
        let env = env_with_read_file();
        let caller = make_fn("caller", vec![Effect::Read], simple_body("read_file"));
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert!(emitter.0.is_empty());
    }

    #[test]
    fn io_subsumes_read() {
        let env = env_with_read_file();
        let caller = make_fn("caller", vec![Effect::IO], simple_body("read_file"));
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert!(emitter.0.is_empty());
    }

    #[test]
    fn io_subsumes_write() {
        let mut env = TypeEnv::new();
        env.define_fn(
            "write_file",
            FnSig {
                params: vec![],
                return_ty: Ty::Unit,
                effects: BTreeSet::from([Effect::Write]),
            },
        );
        let caller = make_fn("caller", vec![Effect::IO], simple_body("write_file"));
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert!(emitter.0.is_empty());
    }

    #[test]
    fn caller_missing_one_of_multiple_effects() {
        let mut env = TypeEnv::new();
        env.define_fn(
            "risky_read",
            FnSig {
                params: vec![],
                return_ty: Ty::Unit,
                effects: BTreeSet::from([Effect::Read, Effect::Panic]),
            },
        );
        let caller = make_fn("caller", vec![Effect::Read], simple_body("risky_read"));
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert_eq!(emitter.0.len(), 1);
        assert_eq!(emitter.0[0].code, ErrorCode::EffectViolation);
        assert!(emitter.0[0].message.contains("Panic"));
    }

    #[test]
    fn vow_purity_impure_predicate_emits_violation() {
        let env = env_with_read_file();
        let vow = VowBlock {
            clauses: vec![VowClause::Requires {
                expr: call_expr("read_file"),
                span: dummy_span(),
            }],
            span: dummy_span(),
        };
        let mut emitter = TestEmitter(vec![]);
        check_vow_purity(&vow, &env, "test.vow", &mut emitter);
        assert_eq!(emitter.0.len(), 1);
        assert_eq!(emitter.0[0].code, ErrorCode::EffectViolation);
        assert_eq!(emitter.0[0].blame, Blame::Callee);
    }

    #[test]
    fn param_refinement_impure_predicate_emits_violation() {
        let env = env_with_read_file();
        let params = vec![Param {
            name: "a".to_string(),
            ty: Type::Named {
                name: "i64".to_string(),
                span: dummy_span(),
            },
            refinement: Some(Box::new(call_expr("read_file"))),
            span: dummy_span(),
        }];
        let mut emitter = TestEmitter(vec![]);
        check_param_refinement_purity(&params, &env, "test.vow", &mut emitter);
        assert_eq!(emitter.0.len(), 1);
        assert_eq!(emitter.0[0].code, ErrorCode::EffectViolation);
        assert_eq!(emitter.0[0].blame, Blame::Callee);
    }

    fn binary_op_with_call(lhs: Expr, rhs: Expr) -> Expr {
        Expr {
            kind: ExprKind::BinaryOp {
                op: BinOp::Add,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            },
            span: dummy_span(),
        }
    }

    fn if_expr(cond: Expr, then: Block) -> Expr {
        Expr {
            kind: ExprKind::If {
                condition: Box::new(cond),
                then_branch: Box::new(then),
                else_branch: None,
            },
            span: dummy_span(),
        }
    }

    fn while_expr(cond: Expr, body: Block) -> Expr {
        Expr {
            kind: ExprKind::While {
                condition: Box::new(cond),
                body: Box::new(body),
                vow: None,
            },
            span: dummy_span(),
        }
    }

    fn method_call_expr(receiver: Expr, method: &str) -> Expr {
        Expr {
            kind: ExprKind::MethodCall {
                receiver: Box::new(receiver),
                method: method.to_string(),
                args: vec![],
            },
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

    fn block_with_call(call_name: &str) -> Block {
        Block {
            stmts: vec![Stmt::Expr {
                expr: call_expr(call_name),
                has_semicolon: true,
                span: dummy_span(),
            }],
            trailing_expr: None,
            span: dummy_span(),
        }
    }

    #[test]
    fn call_inside_binary_op_detected() {
        let env = env_with_read_file();
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(binary_op_with_call(
                call_expr("read_file"),
                call_expr("read_file"),
            ))),
            span: dummy_span(),
        };
        let caller = make_fn("caller", vec![], body);
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert!(
            !emitter.0.is_empty(),
            "should detect read_file inside binop"
        );
    }

    #[test]
    fn call_inside_if_condition_detected() {
        let env = env_with_read_file();
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(if_expr(call_expr("read_file"), empty_block()))),
            span: dummy_span(),
        };
        let caller = make_fn("caller", vec![], body);
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert!(!emitter.0.is_empty(), "should detect call in if condition");
    }

    #[test]
    fn call_inside_if_branch_detected() {
        let env = env_with_read_file();
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(if_expr(
                Expr {
                    kind: ExprKind::Ident("x".into()),
                    span: dummy_span(),
                },
                block_with_call("read_file"),
            ))),
            span: dummy_span(),
        };
        let caller = make_fn("caller", vec![], body);
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert!(!emitter.0.is_empty(), "should detect call in if branch");
    }

    #[test]
    fn call_inside_while_detected() {
        let env = env_with_read_file();
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(while_expr(call_expr("read_file"), empty_block()))),
            span: dummy_span(),
        };
        let caller = make_fn("caller", vec![], body);
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert!(
            !emitter.0.is_empty(),
            "should detect call in while condition"
        );
    }

    #[test]
    fn call_inside_method_receiver_detected() {
        let env = env_with_read_file();
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(method_call_expr(call_expr("read_file"), "len"))),
            span: dummy_span(),
        };
        let caller = make_fn("caller", vec![], body);
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert!(
            !emitter.0.is_empty(),
            "should detect call inside method receiver"
        );
    }

    #[test]
    fn call_inside_nested_block_detected() {
        let env = env_with_read_file();
        let inner_block = Block {
            stmts: vec![Stmt::Expr {
                expr: call_expr("read_file"),
                has_semicolon: true,
                span: dummy_span(),
            }],
            trailing_expr: None,
            span: dummy_span(),
        };
        let outer_body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(Expr {
                kind: ExprKind::Block(Box::new(inner_block)),
                span: dummy_span(),
            })),
            span: dummy_span(),
        };
        let caller = make_fn("caller", vec![], outer_body);
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert!(
            !emitter.0.is_empty(),
            "should detect call inside nested block"
        );
    }

    #[test]
    fn vow_purity_pure_predicate_no_error() {
        let mut env = TypeEnv::new();
        env.define_fn(
            "is_valid",
            FnSig {
                params: vec![],
                return_ty: Ty::Unit,
                effects: BTreeSet::new(),
            },
        );
        let vow = VowBlock {
            clauses: vec![VowClause::Requires {
                expr: call_expr("is_valid"),
                span: dummy_span(),
            }],
            span: dummy_span(),
        };
        let mut emitter = TestEmitter(vec![]);
        check_vow_purity(&vow, &env, "test.vow", &mut emitter);
        assert!(emitter.0.is_empty());
    }

    #[test]
    fn effect_violation_includes_add_effect_hint() {
        let env = env_with_read_file();
        let caller = make_fn("caller", vec![], simple_body("read_file"));
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert_eq!(emitter.0.len(), 1);
        assert!(
            emitter.0[0].hints.iter().any(|h| h.contains("Read")),
            "expected hint mentioning the missing effect"
        );
    }

    #[test]
    fn vow_purity_violation_includes_hint() {
        let env = env_with_read_file();
        let vow = VowBlock {
            clauses: vec![VowClause::Requires {
                expr: call_expr("read_file"),
                span: dummy_span(),
            }],
            span: dummy_span(),
        };
        let mut emitter = TestEmitter(vec![]);
        check_vow_purity(&vow, &env, "test.vow", &mut emitter);
        assert_eq!(emitter.0.len(), 1);
        assert!(
            emitter.0[0].hints.iter().any(|h| h.contains("pure")),
            "expected hint about purity"
        );
    }

    fn unwrap_method_call() -> Expr {
        Expr {
            kind: ExprKind::MethodCall {
                receiver: Box::new(Expr {
                    kind: ExprKind::Ident("opt".to_string()),
                    span: dummy_span(),
                }),
                method: "unwrap".to_string(),
                args: vec![],
            },
            span: dummy_span(),
        }
    }

    fn body_with_unwrap() -> Block {
        Block {
            stmts: vec![Stmt::Expr {
                expr: unwrap_method_call(),
                has_semicolon: true,
                span: dummy_span(),
            }],
            trailing_expr: None,
            span: dummy_span(),
        }
    }

    #[test]
    fn unwrap_without_panic_effect_emits_violation() {
        let env = TypeEnv::new();
        let caller = make_fn("caller", vec![], body_with_unwrap());
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert_eq!(emitter.0.len(), 1);
        assert_eq!(emitter.0[0].code, ErrorCode::EffectViolation);
        assert!(emitter.0[0].message.contains(".unwrap()"));
        assert!(emitter.0[0].message.contains("Panic"));
    }

    #[test]
    fn unwrap_with_panic_effect_no_error() {
        let env = TypeEnv::new();
        let caller = make_fn("caller", vec![Effect::Panic], body_with_unwrap());
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert!(emitter.0.is_empty());
    }

    #[test]
    fn unwrap_violation_includes_hint() {
        let env = TypeEnv::new();
        let caller = make_fn("caller", vec![], body_with_unwrap());
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert_eq!(emitter.0.len(), 1);
        assert!(
            emitter.0[0]
                .hints
                .iter()
                .any(|h| h.contains("Panic") || h.contains("?")),
            "expected hint about adding Panic or using ?"
        );
    }

    // --- issue #1032: contract clauses writing shared state through a helper ---

    fn field_access_expr(base: Expr, field: &str) -> Expr {
        Expr {
            kind: ExprKind::FieldAccess {
                base: Box::new(base),
                field: field.to_string(),
            },
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

    fn ident_expr(name: &str) -> Expr {
        Expr {
            kind: ExprKind::Ident(name.to_string()),
            span: dummy_span(),
        }
    }

    fn lit_bool(b: bool) -> Expr {
        Expr {
            kind: ExprKind::Lit(Lit::Bool(b)),
            span: dummy_span(),
        }
    }

    fn lit_int(v: u128) -> Expr {
        Expr {
            kind: ExprKind::Lit(Lit::Int(v)),
            span: dummy_span(),
        }
    }

    fn call_expr_with_args(name: &str, args: Vec<Expr>) -> Expr {
        Expr {
            kind: ExprKind::Call {
                callee: Box::new(ident_expr(name)),
                args,
            },
            span: dummy_span(),
        }
    }

    fn method_call_with_args_expr(receiver: Expr, method: &str, args: Vec<Expr>) -> Expr {
        Expr {
            kind: ExprKind::MethodCall {
                receiver: Box::new(receiver),
                method: method.to_string(),
                args,
            },
            span: dummy_span(),
        }
    }

    fn block_ending_in(stmts: Vec<Expr>, trailing: Expr) -> Block {
        Block {
            stmts: stmts
                .into_iter()
                .map(|expr| Stmt::Expr {
                    expr,
                    has_semicolon: true,
                    span: dummy_span(),
                })
                .collect(),
            trailing_expr: Some(Box::new(trailing)),
            span: dummy_span(),
        }
    }

    fn while_expr_with_vow(condition: Expr, vow: VowBlock, body: Block) -> Expr {
        Expr {
            kind: ExprKind::While {
                condition: Box::new(condition),
                vow: Some(vow),
                body: Box::new(body),
            },
            span: dummy_span(),
        }
    }

    fn loop_expr_with_vow(vow: VowBlock, body: Block) -> Expr {
        Expr {
            kind: ExprKind::Loop {
                vow: Some(vow),
                body: Box::new(body),
            },
            span: dummy_span(),
        }
    }

    fn one_clause_vow(clause: VowClause) -> VowBlock {
        VowBlock {
            clauses: vec![clause],
            span: dummy_span(),
        }
    }

    /// `fn mark(p) -> bool { p.x = 1; true }` — the issue's own reproducer: a
    /// helper that writes through a struct-field assignment.
    fn field_write_helper(name: &str) -> FnDef {
        make_fn(
            name,
            vec![],
            block_ending_in(
                vec![assign_expr(
                    field_access_expr(ident_expr("p"), "x"),
                    lit_int(1),
                )],
                lit_bool(true),
            ),
        )
    }

    /// `fn mark(v) -> bool { v.push(1); true }` — the `Vec` sibling.
    fn vec_push_helper(name: &str) -> FnDef {
        make_fn(
            name,
            vec![],
            block_ending_in(
                vec![method_call_with_args_expr(
                    ident_expr("v"),
                    "push",
                    vec![lit_int(1)],
                )],
                lit_bool(true),
            ),
        )
    }

    /// `fn mark(p) -> bool { inner(p); true }` — writes only transitively,
    /// through `inner`.
    fn calls_other_fn_helper(name: &str, callee: &str) -> FnDef {
        make_fn(
            name,
            vec![],
            block_ending_in(
                vec![call_expr_with_args(callee, vec![ident_expr("p")])],
                lit_bool(true),
            ),
        )
    }

    #[test]
    fn direct_write_in_clause_block_detected() {
        let env = TypeEnv::new();
        let clause_expr = Expr {
            kind: ExprKind::Block(Box::new(block_ending_in(
                vec![assign_expr(
                    field_access_expr(ident_expr("p"), "x"),
                    lit_int(1),
                )],
                lit_bool(true),
            ))),
            span: dummy_span(),
        };
        let vow = one_clause_vow(VowClause::Ensures {
            expr: clause_expr,
            span: dummy_span(),
        });
        let mut emitter = TestEmitter(vec![]);
        check_vow_purity(&vow, &env, "test.vow", &mut emitter);
        assert_eq!(emitter.0.len(), 1);
        assert_eq!(emitter.0[0].code, ErrorCode::EffectViolation);
        assert_eq!(emitter.0[0].blame, Blame::Callee);
    }

    #[test]
    fn direct_mutating_method_call_in_clause_detected() {
        let env = TypeEnv::new();
        let vow = one_clause_vow(VowClause::Ensures {
            expr: method_call_with_args_expr(ident_expr("v"), "push", vec![lit_int(1)]),
            span: dummy_span(),
        });
        let mut emitter = TestEmitter(vec![]);
        check_vow_purity(&vow, &env, "test.vow", &mut emitter);
        assert_eq!(emitter.0.len(), 1);
        assert_eq!(emitter.0[0].code, ErrorCode::EffectViolation);
    }

    #[test]
    fn struct_field_write_through_helper_rejected_from_ensures() {
        let mark = field_write_helper("mark");
        let fn_defs = vec![&mark];
        let mut env = TypeEnv::new();
        compute_may_write_table(&fn_defs, &mut env);
        assert_eq!(env.may_write("mark"), Some(true));

        let vow = one_clause_vow(VowClause::Ensures {
            expr: call_expr_with_args("mark", vec![ident_expr("result")]),
            span: dummy_span(),
        });
        let mut emitter = TestEmitter(vec![]);
        check_vow_purity(&vow, &env, "test.vow", &mut emitter);
        assert_eq!(emitter.0.len(), 1);
        assert_eq!(emitter.0[0].code, ErrorCode::EffectViolation);
        assert_eq!(emitter.0[0].blame, Blame::Callee);
    }

    #[test]
    fn vec_push_through_helper_rejected_from_ensures() {
        let mark = vec_push_helper("mark");
        let fn_defs = vec![&mark];
        let mut env = TypeEnv::new();
        compute_may_write_table(&fn_defs, &mut env);
        assert_eq!(env.may_write("mark"), Some(true));

        let vow = one_clause_vow(VowClause::Ensures {
            expr: call_expr_with_args("mark", vec![ident_expr("result")]),
            span: dummy_span(),
        });
        let mut emitter = TestEmitter(vec![]);
        check_vow_purity(&vow, &env, "test.vow", &mut emitter);
        assert_eq!(emitter.0.len(), 1);
    }

    #[test]
    fn transitive_write_through_two_hop_helper_rejected() {
        let inner = field_write_helper("inner");
        let mark = calls_other_fn_helper("mark", "inner");
        let fn_defs = vec![&inner, &mark];
        let mut env = TypeEnv::new();
        compute_may_write_table(&fn_defs, &mut env);
        assert_eq!(env.may_write("inner"), Some(true));
        assert_eq!(
            env.may_write("mark"),
            Some(true),
            "mark's own body has no direct write, only a call to a writing helper"
        );

        let vow = one_clause_vow(VowClause::Ensures {
            expr: call_expr_with_args("mark", vec![ident_expr("result")]),
            span: dummy_span(),
        });
        let mut emitter = TestEmitter(vec![]);
        check_vow_purity(&vow, &env, "test.vow", &mut emitter);
        assert_eq!(emitter.0.len(), 1);
    }

    #[test]
    fn mutual_recursion_with_one_writing_member_taints_the_cycle() {
        // `a` calls `b`, `b` calls `a` and also writes directly — both must
        // end up `may_write == true` without the fixed point looping forever.
        let a = calls_other_fn_helper("a", "b");
        let b = make_fn(
            "b",
            vec![],
            block_ending_in(
                vec![
                    assign_expr(field_access_expr(ident_expr("p"), "x"), lit_int(1)),
                    call_expr_with_args("a", vec![ident_expr("p")]),
                ],
                lit_bool(true),
            ),
        );
        let fn_defs = vec![&a, &b];
        let mut env = TypeEnv::new();
        compute_may_write_table(&fn_defs, &mut env);
        assert_eq!(env.may_write("a"), Some(true));
        assert_eq!(env.may_write("b"), Some(true));
    }

    #[test]
    fn read_only_methods_in_clause_do_not_trigger_write_violation() {
        let env = TypeEnv::new();
        let vow = one_clause_vow(VowClause::Ensures {
            expr: method_call_with_args_expr(ident_expr("result"), "len", vec![]),
            span: dummy_span(),
        });
        let mut emitter = TestEmitter(vec![]);
        check_vow_purity(&vow, &env, "test.vow", &mut emitter);
        assert!(emitter.0.is_empty());

        let vow_contains_key = one_clause_vow(VowClause::Ensures {
            expr: method_call_with_args_expr(
                ident_expr("result"),
                "contains_key",
                vec![ident_expr("k")],
            ),
            span: dummy_span(),
        });
        let mut emitter2 = TestEmitter(vec![]);
        check_vow_purity(&vow_contains_key, &env, "test.vow", &mut emitter2);
        assert!(emitter2.0.is_empty());
    }

    #[test]
    fn pure_builtin_free_fn_in_clause_does_not_trigger_write_violation() {
        let env = TypeEnv::new();
        let vow = one_clause_vow(VowClause::Ensures {
            expr: call_expr_with_args("string_trim", vec![ident_expr("s")]),
            span: dummy_span(),
        });
        let mut emitter = TestEmitter(vec![]);
        check_vow_purity(&vow, &env, "test.vow", &mut emitter);
        assert!(emitter.0.is_empty());
    }

    #[test]
    fn unresolvable_callee_in_clause_fails_closed() {
        let env = TypeEnv::new();
        let vow = one_clause_vow(VowClause::Requires {
            expr: call_expr_with_args("mystery_fn", vec![]),
            span: dummy_span(),
        });
        let mut emitter = TestEmitter(vec![]);
        check_vow_purity(&vow, &env, "test.vow", &mut emitter);
        assert_eq!(emitter.0.len(), 1);
    }

    #[test]
    fn call_through_non_ident_callee_fails_closed() {
        let env = TypeEnv::new();
        let weird_callee = Expr {
            kind: ExprKind::Block(Box::new(empty_block())),
            span: dummy_span(),
        };
        let call = Expr {
            kind: ExprKind::Call {
                callee: Box::new(weird_callee),
                args: vec![],
            },
            span: dummy_span(),
        };
        let vow = one_clause_vow(VowClause::Requires {
            expr: call,
            span: dummy_span(),
        });
        let mut emitter = TestEmitter(vec![]);
        check_vow_purity(&vow, &env, "test.vow", &mut emitter);
        assert_eq!(emitter.0.len(), 1);
    }

    #[test]
    fn while_loop_invariant_write_violation_detected() {
        let mark = field_write_helper("mark");
        let fn_defs = vec![&mark];
        let mut env = TypeEnv::new();
        compute_may_write_table(&fn_defs, &mut env);

        let loop_vow = one_clause_vow(VowClause::Invariant {
            expr: call_expr_with_args("mark", vec![ident_expr("p")]),
            span: dummy_span(),
        });
        let while_node = while_expr_with_vow(ident_expr("c"), loop_vow, empty_block());
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(while_node)),
            span: dummy_span(),
        };
        let caller = make_fn("caller", vec![], body);
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert!(
            !emitter.0.is_empty(),
            "write through a loop invariant must be rejected"
        );
        assert_eq!(emitter.0[0].code, ErrorCode::EffectViolation);
    }

    #[test]
    fn while_loop_invariant_declared_effect_violation_detected() {
        // Confirms the pre-existing declared-effect purity check — not just
        // the new write check — was blind to loop invariants before this fix.
        let env = env_with_read_file();
        let loop_vow = one_clause_vow(VowClause::Invariant {
            expr: call_expr("read_file"),
            span: dummy_span(),
        });
        let while_node = while_expr_with_vow(ident_expr("c"), loop_vow, empty_block());
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(while_node)),
            span: dummy_span(),
        };
        let caller = make_fn("caller", vec![], body);
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert_eq!(emitter.0.len(), 1);
        assert_eq!(emitter.0[0].code, ErrorCode::EffectViolation);
    }

    #[test]
    fn nested_loop_invariant_write_violation_detected() {
        let mark = field_write_helper("mark");
        let fn_defs = vec![&mark];
        let mut env = TypeEnv::new();
        compute_may_write_table(&fn_defs, &mut env);

        let inner_loop_vow = one_clause_vow(VowClause::Invariant {
            expr: call_expr_with_args("mark", vec![ident_expr("p")]),
            span: dummy_span(),
        });
        let inner_loop = loop_expr_with_vow(inner_loop_vow, empty_block());
        let outer_body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(inner_loop)),
            span: dummy_span(),
        };
        let outer_while = while_expr_with_vow(
            ident_expr("c"),
            one_clause_vow(VowClause::Invariant {
                expr: lit_bool(true),
                span: dummy_span(),
            }),
            outer_body,
        );
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(outer_while)),
            span: dummy_span(),
        };
        let caller = make_fn("caller", vec![], body);
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert!(
            emitter
                .0
                .iter()
                .any(|d| d.code == ErrorCode::EffectViolation),
            "a loop nested inside another loop must still be reached"
        );
    }

    // --- codecov: rare ExprKind arms (Index, UnaryOp, Borrow, Question,
    // Cast, Tuple, EnumConstruct, StructLiteral, Break, ForEach, Loop) must
    // still be searched through, not just the common ones already exercised
    // above. ---

    fn wrap_index(inner: Expr) -> Expr {
        Expr {
            kind: ExprKind::Index {
                base: Box::new(inner),
                index: Box::new(lit_int(0)),
            },
            span: dummy_span(),
        }
    }

    fn wrap_unary(inner: Expr) -> Expr {
        Expr {
            kind: ExprKind::UnaryOp {
                op: UnOp::Not,
                operand: Box::new(inner),
            },
            span: dummy_span(),
        }
    }

    fn wrap_borrow(inner: Expr) -> Expr {
        Expr {
            kind: ExprKind::Borrow {
                expr: Box::new(inner),
            },
            span: dummy_span(),
        }
    }

    fn wrap_question(inner: Expr) -> Expr {
        Expr {
            kind: ExprKind::Question {
                expr: Box::new(inner),
            },
            span: dummy_span(),
        }
    }

    fn wrap_cast(inner: Expr) -> Expr {
        Expr {
            kind: ExprKind::Cast {
                expr: Box::new(inner),
                target_ty: Box::new(Type::Unit { span: dummy_span() }),
            },
            span: dummy_span(),
        }
    }

    fn wrap_tuple(inner: Expr) -> Expr {
        Expr {
            kind: ExprKind::Tuple(vec![inner]),
            span: dummy_span(),
        }
    }

    fn wrap_enum_construct(inner: Expr) -> Expr {
        Expr {
            kind: ExprKind::EnumConstruct {
                path: vec!["Option".to_string(), "Some".to_string()],
                fields: vec![inner],
            },
            span: dummy_span(),
        }
    }

    fn wrap_struct_literal(inner: Expr) -> Expr {
        Expr {
            kind: ExprKind::StructLiteral {
                name: "Foo".to_string(),
                fields: vec![("f".to_string(), inner)],
            },
            span: dummy_span(),
        }
    }

    fn wrap_break(inner: Expr) -> Expr {
        Expr {
            kind: ExprKind::Break {
                value: Some(Box::new(inner)),
            },
            span: dummy_span(),
        }
    }

    fn wrap_foreach(inner: Expr) -> Expr {
        Expr {
            kind: ExprKind::ForEach {
                binding: "x".to_string(),
                iterable: Box::new(ident_expr("xs")),
                vow: None,
                body: Box::new(Block {
                    stmts: vec![],
                    trailing_expr: Some(Box::new(inner)),
                    span: dummy_span(),
                }),
            },
            span: dummy_span(),
        }
    }

    fn wrap_loop(inner: Expr) -> Expr {
        Expr {
            kind: ExprKind::Loop {
                vow: None,
                body: Box::new(Block {
                    stmts: vec![],
                    trailing_expr: Some(Box::new(inner)),
                    span: dummy_span(),
                }),
            },
            span: dummy_span(),
        }
    }

    /// Wraps `inner` through every rare `ExprKind` arm, one layer each, so a
    /// single test exercises all of them at once.
    fn wrap_all_rare_kinds(inner: Expr) -> Expr {
        let e = wrap_index(inner);
        let e = wrap_unary(e);
        let e = wrap_borrow(e);
        let e = wrap_question(e);
        let e = wrap_cast(e);
        let e = wrap_tuple(e);
        let e = wrap_enum_construct(e);
        let e = wrap_struct_literal(e);
        let e = wrap_break(e);
        let e = wrap_foreach(e);
        wrap_loop(e)
    }

    #[test]
    fn write_through_rare_expr_kinds_still_detected_in_clause() {
        let env = TypeEnv::new();
        let leaf = assign_expr(field_access_expr(ident_expr("p"), "x"), lit_int(1));
        let vow = one_clause_vow(VowClause::Ensures {
            expr: wrap_all_rare_kinds(leaf),
            span: dummy_span(),
        });
        let mut emitter = TestEmitter(vec![]);
        check_vow_purity(&vow, &env, "test.vow", &mut emitter);
        assert!(
            !emitter.0.is_empty(),
            "a write nested inside Index/UnaryOp/Borrow/Question/Cast/Tuple/\
             EnumConstruct/StructLiteral/Break/ForEach/Loop must still be found"
        );
        assert_eq!(emitter.0[0].code, ErrorCode::EffectViolation);
    }

    #[test]
    fn loop_invariant_nested_in_rare_expr_kinds_still_reached() {
        let mark = field_write_helper("mark");
        let fn_defs = vec![&mark];
        let mut env = TypeEnv::new();
        compute_may_write_table(&fn_defs, &mut env);

        let loop_vow = one_clause_vow(VowClause::Invariant {
            expr: call_expr_with_args("mark", vec![ident_expr("p")]),
            span: dummy_span(),
        });
        let while_node = while_expr_with_vow(ident_expr("c"), loop_vow, empty_block());
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(wrap_all_rare_kinds(while_node))),
            span: dummy_span(),
        };
        let caller = make_fn("caller", vec![], body);
        let mut emitter = TestEmitter(vec![]);
        check_fn_effects(&caller, &env, "test.vow", &mut emitter);
        assert!(
            emitter
                .0
                .iter()
                .any(|d| d.code == ErrorCode::EffectViolation),
            "a while loop nested inside Index/UnaryOp/Borrow/Question/Cast/Tuple/\
             EnumConstruct/StructLiteral/Break/ForEach/Loop must still be reached"
        );
    }
}
