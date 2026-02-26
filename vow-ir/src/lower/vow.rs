use vow_diag::Blame;
use vow_syntax::ast::{ExprKind, VowBlock, VowClause};

use crate::types::{InstData, InstId, Opcode, Ty};

use super::LowerCtx;

fn clause_description(clause: &VowClause) -> String {
    match clause {
        VowClause::Requires { expr, .. } => {
            format!("requires {}", vow_syntax::printer::print_expr(expr))
        }
        VowClause::Ensures { expr, .. } => {
            format!("ensures {}", vow_syntax::printer::print_expr(expr))
        }
        VowClause::Invariant { expr, .. } => {
            format!("invariant {}", vow_syntax::printer::print_expr(expr))
        }
    }
}

fn lower_predicate(ctx: &mut LowerCtx, clause: &VowClause, result_id: Option<InstId>) -> InstId {
    let expr = match clause {
        VowClause::Requires { expr, .. } => expr,
        VowClause::Ensures { expr, .. } => expr,
        VowClause::Invariant { expr, .. } => expr,
    };
    let span = expr.span;
    if matches!(expr.kind, ExprKind::Result) {
        if let Some(id) = result_id {
            return id;
        }
        return ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span);
    }
    super::lower_expr_pub(ctx, expr)
}

pub fn lower_requires(ctx: &mut LowerCtx, vow_block: &VowBlock) {
    for clause in &vow_block.clauses {
        if let VowClause::Requires { span, .. } = clause {
            let desc = clause_description(clause);
            let vow_id = ctx.alloc_vow(desc, Blame::Caller);
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

pub fn lower_ensures(ctx: &mut LowerCtx, vow_block: &VowBlock, result_id: InstId) {
    // Bind "result" in scope so it's accessible in ensures predicates.
    ctx.push_scope();
    ctx.define("result".to_string(), result_id);
    for clause in &vow_block.clauses {
        if let VowClause::Ensures { span, .. } = clause {
            let desc = clause_description(clause);
            let vow_id = ctx.alloc_vow(desc, Blame::Callee);
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
            let vow_id = ctx.alloc_vow(desc, Blame::Callee);
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
        let (func, _) = lower_function(&fn_def, &std::collections::HashMap::new());

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
        let (func, _) = lower_function(&fn_def, &std::collections::HashMap::new());

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
        let (func, _) = lower_function(&fn_def, &std::collections::HashMap::new());

        assert_eq!(func.vows.len(), 1);
        assert_eq!(func.vows[0].blame, Blame::Caller);

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
        let (func, _) = lower_function(&fn_def, &std::collections::HashMap::new());

        assert_eq!(func.vows.len(), 1);
        assert_eq!(func.vows[0].blame, Blame::Callee);

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();
        let ens = all_insts
            .iter()
            .find(|i| i.opcode == Opcode::VowEnsures)
            .expect("expected VowEnsures");
        assert_eq!(ens.data, InstData::VowId(func.vows[0].id));
    }
}
