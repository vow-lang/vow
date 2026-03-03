use vow_diag::Blame;
use vow_syntax::ast::{Expr, ExprKind, Param, VowBlock, VowClause};

use crate::types::{InstData, InstId, Opcode, Ty};

use super::LowerCtx;

fn clause_expr(clause: &VowClause) -> &Expr {
    match clause {
        VowClause::Requires { expr, .. } => expr,
        VowClause::Ensures { expr, .. } => expr,
        VowClause::Invariant { expr, .. } => expr,
    }
}

fn clause_description(clause: &VowClause) -> String {
    let prefix = match clause {
        VowClause::Requires { .. } => "requires",
        VowClause::Ensures { .. } => "ensures",
        VowClause::Invariant { .. } => "invariant",
    };
    format!(
        "{} {}",
        prefix,
        vow_syntax::printer::print_expr(clause_expr(clause))
    )
}

fn lower_predicate(ctx: &mut LowerCtx, clause: &VowClause, result_id: Option<InstId>) -> InstId {
    let expr = clause_expr(clause);
    let span = expr.span;
    if matches!(expr.kind, ExprKind::Result) {
        if let Some(id) = result_id {
            return id;
        }
        return ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span);
    }
    super::lower_expr_pub(ctx, expr)
}

fn collect_free_vars(ctx: &LowerCtx, expr: &Expr) -> Vec<(String, InstId)> {
    let mut out: Vec<(String, InstId)> = Vec::new();
    collect_vars_in_expr(ctx, expr, &mut out);
    out
}

fn collect_vars_in_expr(ctx: &LowerCtx, expr: &Expr, out: &mut Vec<(String, InstId)>) {
    match &expr.kind {
        ExprKind::Ident(name) => {
            if let Some(id) = ctx.lookup(name)
                && !out.iter().any(|(n, _)| n == name)
            {
                out.push((name.clone(), id));
            }
        }
        ExprKind::Result => {
            if let Some(id) = ctx.lookup("result")
                && !out.iter().any(|(n, _)| n == "result")
            {
                out.push(("result".to_string(), id));
            }
        }
        ExprKind::BinaryOp { lhs, rhs, .. } => {
            collect_vars_in_expr(ctx, lhs, out);
            collect_vars_in_expr(ctx, rhs, out);
        }
        ExprKind::UnaryOp { operand, .. } => {
            collect_vars_in_expr(ctx, operand, out);
        }
        ExprKind::Call { callee, args } => {
            collect_vars_in_expr(ctx, callee, out);
            for arg in args {
                collect_vars_in_expr(ctx, arg, out);
            }
        }
        ExprKind::Block(block) => {
            for stmt in &block.stmts {
                if let vow_syntax::ast::Stmt::Expr { expr, .. } = stmt {
                    collect_vars_in_expr(ctx, expr, out);
                }
            }
            if let Some(e) = &block.trailing_expr {
                collect_vars_in_expr(ctx, e, out);
            }
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_vars_in_expr(ctx, condition, out);
            for stmt in &then_branch.stmts {
                if let vow_syntax::ast::Stmt::Expr { expr, .. } = stmt {
                    collect_vars_in_expr(ctx, expr, out);
                }
            }
            if let Some(e) = &then_branch.trailing_expr {
                collect_vars_in_expr(ctx, e, out);
            }
            if let Some(e) = else_branch {
                collect_vars_in_expr(ctx, e, out);
            }
        }
        ExprKind::Lit(_) | ExprKind::Break { .. } | ExprKind::Return { .. } => {}
        _ => {}
    }
}

pub fn lower_requires(ctx: &mut LowerCtx, vow_block: &VowBlock) {
    for clause in &vow_block.clauses {
        if let VowClause::Requires { span, .. } = clause {
            let desc = clause_description(clause);
            let bindings = collect_free_vars(ctx, clause_expr(clause));
            let vow_id = ctx.alloc_vow(desc, Blame::Caller, bindings, span.start);
            let pred_id = lower_predicate(ctx, clause, None);
            ctx.emit(
                Opcode::VowRequires,
                Ty::Unit,
                vec![pred_id],
                InstData::VowId(vow_id),
                *span,
            );
        }
    }
}

pub fn lower_param_refinements(ctx: &mut LowerCtx, params: &[Param]) {
    for param in params {
        if let Some(ref refinement) = param.refinement {
            let clause = VowClause::Requires {
                expr: (**refinement).clone(),
                span: refinement.span,
            };
            let desc = format!(
                "requires {} (where on parameter {})",
                vow_syntax::printer::print_expr(refinement),
                param.name,
            );
            let bindings = collect_free_vars(ctx, refinement);
            let vow_id = ctx.alloc_vow(desc, Blame::Caller, bindings, param.span.start);
            let pred_id = lower_predicate(ctx, &clause, None);
            ctx.emit(
                Opcode::VowRequires,
                Ty::Unit,
                vec![pred_id],
                InstData::VowId(vow_id),
                param.span,
            );
        }
    }
}

pub fn lower_ensures(ctx: &mut LowerCtx, vow_block: &VowBlock, result_id: InstId) {
    ctx.push_scope();
    ctx.define("result".to_string(), result_id);
    for clause in &vow_block.clauses {
        if let VowClause::Ensures { span, .. } = clause {
            let desc = clause_description(clause);
            let bindings = collect_free_vars(ctx, clause_expr(clause));
            let vow_id = ctx.alloc_vow(desc, Blame::Callee, bindings, span.start);
            let pred_id = lower_predicate(ctx, clause, Some(result_id));
            ctx.emit(
                Opcode::VowEnsures,
                Ty::Unit,
                vec![pred_id],
                InstData::VowId(vow_id),
                *span,
            );
        }
    }
    ctx.pop_scope();
}

pub fn lower_invariant(ctx: &mut LowerCtx, vow_block: &VowBlock) {
    for clause in &vow_block.clauses {
        if let VowClause::Invariant { span, .. } = clause {
            let desc = clause_description(clause);
            let bindings = collect_free_vars(ctx, clause_expr(clause));
            let vow_id = ctx.alloc_vow(desc, Blame::Callee, bindings, span.start);
            let pred_id = lower_predicate(ctx, clause, None);
            ctx.emit(
                Opcode::VowInvariant,
                Ty::Unit,
                vec![pred_id],
                InstData::VowId(vow_id),
                *span,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vow_syntax::ast::{
        Block, Effect, Expr, ExprKind, FnDef, Lit, Param, Type, Visibility, VowBlock, VowClause,
    };
    use vow_syntax::span::Span;

    use crate::lower::lower_function;
    use crate::types::{InstData, Opcode};

    fn sp() -> Span {
        Span::new(0, 1)
    }

    fn bool_expr(v: bool) -> Expr {
        Expr {
            kind: ExprKind::Lit(Lit::Bool(v)),
            span: sp(),
        }
    }

    fn empty_block() -> Block {
        Block {
            stmts: vec![],
            trailing_expr: None,
            span: sp(),
        }
    }

    fn make_fn_with_vow(vow: Option<VowBlock>) -> FnDef {
        FnDef {
            vis: Visibility::Public,
            name: "test_fn".to_string(),
            params: vec![Param {
                name: "x".to_string(),
                ty: Type::Named {
                    name: "bool".to_string(),
                    span: sp(),
                },
                refinement: None,
                span: sp(),
            }],
            return_ty: Type::Unit { span: sp() },
            effects: vec![Effect::IO],
            vow,
            body: empty_block(),
            span: sp(),
        }
    }

    #[test]
    fn requires_emits_vow_requires() {
        let clause = VowClause::Requires {
            expr: bool_expr(true),
            span: sp(),
        };
        let vow_block = VowBlock {
            clauses: vec![clause],
            span: sp(),
        };
        let fn_def = make_fn_with_vow(Some(vow_block));
        let (func, _) = lower_function(
            &fn_def,
            "",
            &std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        );

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();

        let get_arg_pos = all_insts
            .iter()
            .position(|i| i.opcode == Opcode::GetArg)
            .expect("expected GetArg");
        let vow_req_pos = all_insts
            .iter()
            .position(|i| i.opcode == Opcode::VowRequires)
            .expect("expected VowRequires");

        assert!(
            vow_req_pos > get_arg_pos,
            "VowRequires must appear after GetArg"
        );

        assert_eq!(func.vows.len(), 1);
        assert_eq!(func.vows[0].blame, Blame::Caller);
        assert_eq!(func.vows[0].bindings, vec![]);
    }

    #[test]
    fn ensures_emits_vow_ensures() {
        let clause = VowClause::Ensures {
            expr: bool_expr(true),
            span: sp(),
        };
        let vow_block = VowBlock {
            clauses: vec![clause],
            span: sp(),
        };
        let fn_def = make_fn_with_vow(Some(vow_block));
        let (func, _) = lower_function(
            &fn_def,
            "",
            &std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        );

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();

        let vow_ens_pos = all_insts
            .iter()
            .position(|i| i.opcode == Opcode::VowEnsures)
            .expect("expected VowEnsures");
        let ret_pos = all_insts
            .iter()
            .position(|i| i.opcode == Opcode::Return)
            .expect("expected Return");

        assert!(
            vow_ens_pos < ret_pos,
            "VowEnsures must appear before Return"
        );
        assert_eq!(func.vows[0].bindings, vec![]);
    }

    #[test]
    fn requires_blame_is_caller() {
        let clause = VowClause::Requires {
            expr: bool_expr(true),
            span: sp(),
        };
        let vow_block = VowBlock {
            clauses: vec![clause],
            span: sp(),
        };
        let fn_def = make_fn_with_vow(Some(vow_block));
        let (func, _) = lower_function(
            &fn_def,
            "",
            &std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        );

        assert_eq!(func.vows.len(), 1);
        assert_eq!(func.vows[0].blame, Blame::Caller);
        assert_eq!(func.vows[0].bindings, vec![]);

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();
        let req = all_insts
            .iter()
            .find(|i| i.opcode == Opcode::VowRequires)
            .expect("expected VowRequires");
        assert_eq!(req.data, InstData::VowId(func.vows[0].id));
    }

    #[test]
    fn ensures_blame_is_callee() {
        let clause = VowClause::Ensures {
            expr: bool_expr(true),
            span: sp(),
        };
        let vow_block = VowBlock {
            clauses: vec![clause],
            span: sp(),
        };
        let fn_def = make_fn_with_vow(Some(vow_block));
        let (func, _) = lower_function(
            &fn_def,
            "",
            &std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        );

        assert_eq!(func.vows.len(), 1);
        assert_eq!(func.vows[0].blame, Blame::Callee);
        assert_eq!(func.vows[0].bindings, vec![]);

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();
        let ens = all_insts
            .iter()
            .find(|i| i.opcode == Opcode::VowEnsures)
            .expect("expected VowEnsures");
        assert_eq!(ens.data, InstData::VowId(func.vows[0].id));
    }

    #[test]
    fn param_refinement_emits_vow_requires() {
        let fn_def = FnDef {
            vis: Visibility::Public,
            name: "divide".to_string(),
            params: vec![
                Param {
                    name: "x".to_string(),
                    ty: Type::Named {
                        name: "i64".to_string(),
                        span: sp(),
                    },
                    refinement: None,
                    span: sp(),
                },
                Param {
                    name: "y".to_string(),
                    ty: Type::Named {
                        name: "i64".to_string(),
                        span: sp(),
                    },
                    refinement: Some(Box::new(Expr {
                        kind: ExprKind::BinaryOp {
                            op: vow_syntax::ast::BinOp::Ne,
                            lhs: Box::new(Expr {
                                kind: ExprKind::Ident("y".to_string()),
                                span: sp(),
                            }),
                            rhs: Box::new(Expr {
                                kind: ExprKind::Lit(Lit::Int(0)),
                                span: sp(),
                            }),
                        },
                        span: sp(),
                    })),
                    span: sp(),
                },
            ],
            return_ty: Type::Named {
                name: "i64".to_string(),
                span: sp(),
            },
            effects: vec![],
            vow: None,
            body: Block {
                stmts: vec![],
                trailing_expr: Some(Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Int(0)),
                    span: sp(),
                })),
                span: sp(),
            },
            span: sp(),
        };
        let (func, _) = lower_function(
            &fn_def,
            "",
            &std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        );

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();
        let vow_req = all_insts
            .iter()
            .find(|i| i.opcode == Opcode::VowRequires)
            .expect("expected VowRequires from where clause");

        assert_eq!(func.vows.len(), 1);
        assert_eq!(func.vows[0].blame, Blame::Caller);
        assert!(
            func.vows[0].description.contains("y != 0"),
            "description should contain the predicate"
        );
        assert!(
            func.vows[0].description.contains("parameter y"),
            "description should mention the parameter name"
        );
        assert_eq!(vow_req.data, InstData::VowId(func.vows[0].id));
    }

    #[test]
    fn param_refinement_merged_with_explicit_requires() {
        let fn_def = FnDef {
            vis: Visibility::Public,
            name: "clamp".to_string(),
            params: vec![
                Param {
                    name: "x".to_string(),
                    ty: Type::Named {
                        name: "i64".to_string(),
                        span: sp(),
                    },
                    refinement: Some(Box::new(Expr {
                        kind: ExprKind::BinaryOp {
                            op: vow_syntax::ast::BinOp::Ge,
                            lhs: Box::new(Expr {
                                kind: ExprKind::Ident("x".to_string()),
                                span: sp(),
                            }),
                            rhs: Box::new(Expr {
                                kind: ExprKind::Lit(Lit::Int(0)),
                                span: sp(),
                            }),
                        },
                        span: sp(),
                    })),
                    span: sp(),
                },
                Param {
                    name: "max".to_string(),
                    ty: Type::Named {
                        name: "i64".to_string(),
                        span: sp(),
                    },
                    refinement: Some(Box::new(Expr {
                        kind: ExprKind::BinaryOp {
                            op: vow_syntax::ast::BinOp::Gt,
                            lhs: Box::new(Expr {
                                kind: ExprKind::Ident("max".to_string()),
                                span: sp(),
                            }),
                            rhs: Box::new(Expr {
                                kind: ExprKind::Lit(Lit::Int(0)),
                                span: sp(),
                            }),
                        },
                        span: sp(),
                    })),
                    span: sp(),
                },
            ],
            return_ty: Type::Named {
                name: "i64".to_string(),
                span: sp(),
            },
            effects: vec![],
            vow: Some(VowBlock {
                clauses: vec![VowClause::Requires {
                    expr: bool_expr(true),
                    span: sp(),
                }],
                span: sp(),
            }),
            body: Block {
                stmts: vec![],
                trailing_expr: Some(Box::new(Expr {
                    kind: ExprKind::Lit(Lit::Int(0)),
                    span: sp(),
                })),
                span: sp(),
            },
            span: sp(),
        };
        let (func, _) = lower_function(
            &fn_def,
            "",
            &std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        );

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();
        let vow_reqs: Vec<_> = all_insts
            .iter()
            .filter(|i| i.opcode == Opcode::VowRequires)
            .collect();

        assert_eq!(
            vow_reqs.len(),
            3,
            "2 param refinements + 1 explicit requires"
        );
        assert_eq!(func.vows.len(), 3);
        assert!(func.vows.iter().all(|v| v.blame == Blame::Caller));
    }
}
