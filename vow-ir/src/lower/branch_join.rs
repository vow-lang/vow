//! Sibling-branch exit capture and SSA joins. Dispatch and pattern binding stay
//! with the caller; scope restoration, inputs, and result coercion stay here.
use super::*;

struct BranchExit<'a> {
    block: BlockId,
    upsilon: Option<InstId>,
    value: InstId,
    ty: Ty,
    mutation_values: Vec<InstId>,
    mutation_upsilons: Vec<InstId>,
    marker: bool,
    body: Option<&'a Expr>,
}

pub(super) struct BranchJoin<'a> {
    source: &'a Expr,
    merge: BlockId,
    mutations: Vec<(String, InstId)>,
    scope: Vec<HashMap<String, InstId>>,
    exits: Vec<BranchExit<'a>>,
}

pub(super) fn branch_join_new<'a>(
    ctx: &LowerCtx,
    source: &'a Expr,
    merge: BlockId,
    mutations: Vec<(String, InstId)>,
) -> BranchJoin<'a> {
    BranchJoin {
        source,
        merge,
        mutations,
        scope: ctx.snapshot_scope(),
        exits: Vec::new(),
    }
}

pub(super) fn branch_join_capture(
    ctx: &mut LowerCtx,
    join: &mut BranchJoin<'_>,
    index: usize,
    value: InstId,
) {
    let (marker, body, fallback) = match &join.source.kind {
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            if index == 0 {
                (
                    block_result_is_coercible_int_marker(then_branch),
                    integer_marker_from_block(then_branch),
                    false,
                )
            } else {
                let body = else_branch.as_deref();
                (body.is_some_and(expr_is_coercible_int_marker), body, false)
            }
        }
        ExprKind::Match { arms, .. } => {
            let arm = &arms[index];
            let fallback = !matches!(
                arm.pattern.kind,
                PatKind::EnumVariant { .. } | PatKind::Wildcard | PatKind::Ident { .. }
            );
            (
                !fallback && expr_is_coercible_int_marker(&arm.body),
                (!fallback).then_some(&arm.body),
                fallback,
            )
        }
        _ => unreachable!("branch join requires an if or match expression"),
    };
    let terminated = ctx.is_terminated();
    // If keeps dead-arm types for its historical mutation type fold. Match
    // collects only arms reaching the join, including its unit fallback.
    if !terminated || matches!(join.source.kind, ExprKind::If { .. }) {
        let mutation_values: Vec<_> = join
            .mutations
            .iter()
            .map(|(name, original)| {
                if fallback {
                    *original
                } else {
                    ctx.lookup(name).unwrap_or(*original)
                }
            })
            .collect();
        let mutation_upsilons = if terminated {
            Vec::new()
        } else {
            mutation_values
                .iter()
                .map(|&value| emit_pending_upsilon(ctx, value, join.source.span))
                .collect()
        };
        let upsilon = if terminated {
            None
        } else {
            let up = emit_pending_upsilon(ctx, value, join.source.span);
            ctx.emit(
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(join.merge),
                join.source.span,
            );
            Some(up)
        };
        join.exits.push(BranchExit {
            block: ctx.current_block,
            upsilon,
            value,
            ty: ctx.inst_ty(value),
            mutation_values,
            mutation_upsilons,
            marker,
            body,
        });
    }
    ctx.restore_scope(join.scope.clone());
}

pub(super) fn branch_join_finish(ctx: &mut LowerCtx, mut join: BranchJoin<'_>) -> InstId {
    let span = join.source.span;
    ctx.restore_scope(join.scope);
    ctx.switch_to_block(join.merge);
    let is_if = matches!(join.source.kind, ExprKind::If { .. });
    if !is_if && join.exits.is_empty() {
        return ctx.emit(Opcode::Unreachable, Ty::Unit, vec![], InstData::None, span);
    }
    for (index, (name, _)) in join.mutations.iter().enumerate() {
        let first_ty = ctx.inst_ty(join.exits[0].mutation_values[index]);
        let ty = join.exits.iter().skip(1).fold(first_ty, |ty, exit| {
            merge_phi_ty(ty, ctx.inst_ty(exit.mutation_values[index]))
        });
        let phi = ctx.emit(Opcode::Phi, ty, vec![], InstData::None, span);
        for exit in &join.exits {
            if let Some(&up) = exit.mutation_upsilons.get(index) {
                backpatch_upsilon(ctx, exit.block, up, phi);
            }
        }
        let sources: Vec<_> = join
            .exits
            .iter()
            .filter(|exit| exit.upsilon.is_some())
            .map(|exit| exit.mutation_values[index])
            .collect();
        merge_compatible_aggregate_metadata(ctx, &sources, phi);
        ctx.assign(name, phi);
    }
    // If observes result widths after mutation inputs have been linked; match
    // preserves the widths captured when each arm was lowered.
    if is_if {
        for exit in &mut join.exits {
            exit.ty = ctx.inst_ty(exit.value);
        }
    }
    let live: Vec<_> = join
        .exits
        .iter()
        .filter(|exit| exit.upsilon.is_some())
        .collect();
    let ty = if is_if {
        match live.as_slice() {
            [] => return ctx.emit(Opcode::Unreachable, Ty::Unit, vec![], InstData::None, span),
            [exit] => exit.ty,
            [then, otherwise] => merge_if_result_ty(then.ty, then.marker, otherwise.ty),
            _ => unreachable!("an if has two branches"),
        }
    } else {
        choose_match_result_ty(&join.exits)
    };
    let sources: Vec<_> = live.iter().map(|exit| exit.value).collect();
    // A single live if branch already supplies the exact result type. With
    // sibling inputs, literal markers must match the chosen register width.
    if narrow_int_width(ty).is_some() && (!is_if || live.len() == 2) {
        for exit in &mut join.exits {
            if exit.marker
                && exit.ty != ty
                && let (Some(up), Some(body)) = (exit.upsilon, exit.body)
                && let Some(replacement) =
                    renarrow_if_branch_result(ctx, exit.block, up, body, ty, span)
            {
                exit.upsilon = Some(replacement);
            }
        }
        ctx.switch_to_block(join.merge);
    }
    let phi = ctx.emit(Opcode::Phi, ty, vec![], InstData::None, span);
    if !is_if {
        merge_compatible_aggregate_metadata(ctx, &sources, phi);
    }
    for exit in &join.exits {
        if let Some(up) = exit.upsilon {
            backpatch_upsilon(ctx, exit.block, up, phi);
        }
    }
    if is_if {
        merge_compatible_aggregate_metadata(ctx, &sources, phi);
    }
    phi
}

fn choose_match_result_ty(exits: &[BranchExit<'_>]) -> Ty {
    if exits.iter().any(|exit| exit.ty == Ty::LinearPtr) {
        return Ty::LinearPtr;
    }
    let Some(first) = exits.first() else {
        return Ty::I64;
    };
    let mut ty = first.ty;
    let mut marker = first.marker;
    for exit in &exits[1..] {
        if marker && ir_ty_is_integer(exit.ty) {
            ty = exit.ty;
            marker = exit.marker && exit.ty == Ty::I64;
        } else if !(exit.marker && ir_ty_is_integer(ty)) {
            marker = false;
        }
    }
    ty
}

fn merge_if_result_ty(then: Ty, marker: bool, otherwise: Ty) -> Ty {
    if then == Ty::LinearPtr || otherwise == Ty::LinearPtr {
        Ty::LinearPtr
    } else if marker && ir_ty_is_integer(otherwise) {
        otherwise
    } else {
        then
    }
}
