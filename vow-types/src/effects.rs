use vow_diag::{Blame, Diagnostic, DiagnosticEmitter, ErrorCode, Severity, SourceLocation};
use vow_syntax::ast::{Block, Effect, Expr, ExprKind, FnDef, Stmt, VowBlock, VowClause};

use crate::env::TypeEnv;

fn effect_covered(declared: &[Effect], needed: &Effect) -> bool {
    if declared.contains(needed) {
        return true;
    }
    if (needed == &Effect::Read || needed == &Effect::Write) && declared.contains(&Effect::IO) {
        return true;
    }
    false
}

fn collect_calls_in_expr<'a>(expr: &'a Expr, calls: &mut Vec<(&'a Expr, &'a str)>) {
    match &expr.kind {
        ExprKind::Call { callee, args } => {
            if let ExprKind::Ident(name) = &callee.kind {
                calls.push((callee, name.as_str()));
            }
            for arg in args {
                collect_calls_in_expr(arg, calls);
            }
        }
        ExprKind::BinaryOp { lhs, rhs, .. } => {
            collect_calls_in_expr(lhs, calls);
            collect_calls_in_expr(rhs, calls);
        }
        ExprKind::UnaryOp { operand, .. } => collect_calls_in_expr(operand, calls),
        ExprKind::MethodCall { receiver, args, .. } => {
            collect_calls_in_expr(receiver, calls);
            for arg in args {
                collect_calls_in_expr(arg, calls);
            }
        }
        ExprKind::Block(block) => collect_calls_in_block(block, calls),
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_calls_in_expr(condition, calls);
            collect_calls_in_block(then_branch, calls);
            if let Some(e) = else_branch {
                collect_calls_in_expr(e, calls);
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            collect_calls_in_expr(scrutinee, calls);
            for arm in arms {
                collect_calls_in_expr(&arm.body, calls);
            }
        }
        ExprKind::While {
            condition, body, ..
        } => {
            collect_calls_in_expr(condition, calls);
            collect_calls_in_block(body, calls);
        }
        ExprKind::Loop { body, .. } => collect_calls_in_block(body, calls),
        ExprKind::Return { value } => {
            if let Some(v) = value {
                collect_calls_in_expr(v, calls);
            }
        }
        ExprKind::Borrow { expr } | ExprKind::Question { expr } => {
            collect_calls_in_expr(expr, calls);
        }
        ExprKind::FieldAccess { base, .. } => {
            collect_calls_in_expr(base, calls);
        }
        ExprKind::Index { base, index } => {
            collect_calls_in_expr(base, calls);
            collect_calls_in_expr(index, calls);
        }
        ExprKind::Assign { lhs, rhs } => {
            collect_calls_in_expr(lhs, calls);
            collect_calls_in_expr(rhs, calls);
        }
        ExprKind::Tuple(exprs) => {
            for e in exprs {
                collect_calls_in_expr(e, calls);
            }
        }
        ExprKind::Break { value } => {
            if let Some(v) = value {
                collect_calls_in_expr(v, calls);
            }
        }
        ExprKind::Lit(_) | ExprKind::Ident(_) | ExprKind::Result => {}
        ExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                collect_calls_in_expr(e, calls);
            }
        }
        ExprKind::EnumConstruct { fields, .. } => {
            for e in fields {
                collect_calls_in_expr(e, calls);
            }
        }
    }
}

fn collect_calls_in_block<'a>(block: &'a Block, calls: &mut Vec<(&'a Expr, &'a str)>) {
    for stmt in &block.stmts {
        match stmt {
            Stmt::Let { init, .. } => collect_calls_in_expr(init, calls),
            Stmt::Expr { expr, .. } => collect_calls_in_expr(expr, calls),
        }
    }
    if let Some(e) = &block.trailing_expr {
        collect_calls_in_expr(e, calls);
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
    collect_calls_in_block(&fn_def.body, &mut calls);

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

    if let Some(vow_block) = &fn_def.vow {
        check_vow_purity(vow_block, env, file, emitter);
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

        let mut calls = Vec::new();
        collect_calls_in_expr(expr, &mut calls);

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
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use vow_diag::Diagnostic;
    use vow_syntax::ast::{
        BinOp, Block, Effect, Expr, ExprKind, FnDef, Stmt, Type, Visibility, VowBlock, VowClause,
    };
    use vow_syntax::span::Span;

    use crate::env::{FnSig, TypeEnv};
    use crate::types::Ty;

    struct TestEmitter(Vec<Diagnostic>);

    impl DiagnosticEmitter for TestEmitter {
        fn emit(&mut self, d: &Diagnostic) {
            self.0.push(d.clone());
        }
        fn finish(&mut self) {}
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
}
