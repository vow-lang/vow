//! Active-loop lifetime, control transfers, and loop-body completion.
//!
//! Syntax lowering supplies the header/exit edges it has already constructed.
//! This module keeps their state together while recursively lowering the body,
//! resolves transfers at the loop's lexical depth, and completes its result.

use super::{
    LowerCtx, backpatch_upsilon, lower_block, lower_expr, merge_compatible_aggregate_metadata,
};
use crate::types::{BlockId, InstData, InstId, IntegerType, Opcode, Ty};
use vow_syntax::ast::{Block, Expr};
use vow_syntax::span::Span;

pub(super) struct LoopEdges {
    pub header: BlockId,
    pub exit: BlockId,
    pub carried_phis: Vec<(String, InstId)>,
    pub exit_phis: Vec<(String, InstId)>,
}

/// A break result's source block, pending Upsilon, type, and value.
type BreakUpsilon = (BlockId, InstId, Ty, InstId);

pub(super) struct LoopFrame {
    edges: LoopEdges,
    scope_depth: usize,
    index_phi: Option<InstId>,
    break_upsilons: Option<Vec<BreakUpsilon>>,
}

pub(super) fn lower_while_body(
    ctx: &mut LowerCtx,
    edges: LoopEdges,
    body: &Block,
    span: Span,
) -> InstId {
    lower_body(ctx, edges, body, span, None, false)
}

pub(super) fn lower_loop_body(
    ctx: &mut LowerCtx,
    edges: LoopEdges,
    body: &Block,
    span: Span,
) -> InstId {
    lower_body(ctx, edges, body, span, None, true)
}

pub(super) fn lower_foreach_body(
    ctx: &mut LowerCtx,
    edges: LoopEdges,
    binding: &str,
    element: InstId,
    index_phi: InstId,
    body: &Block,
    span: Span,
) -> InstId {
    lower_body(
        ctx,
        edges,
        body,
        span,
        Some((binding, element, index_phi)),
        false,
    )
}

fn lower_body(
    ctx: &mut LowerCtx,
    edges: LoopEdges,
    body: &Block,
    span: Span,
    foreach: Option<(&str, InstId, InstId)>,
    collect_breaks: bool,
) -> InstId {
    // Capture before the for-element scope: a shadowing element binding must
    // never replace an outer loop-carried variable on break or continue.
    let scope_depth = ctx.scope.len();
    if let Some((binding, element, _)) = foreach {
        ctx.push_scope();
        ctx.define(binding.to_string(), element);
    }
    ctx.loop_frames.push(LoopFrame {
        edges,
        scope_depth,
        index_phi: foreach.map(|(_, _, index)| index),
        break_upsilons: collect_breaks.then(Vec::new),
    });
    lower_block(ctx, body);
    let frame = ctx.loop_frames.pop().expect("active loop body");
    if foreach.is_some() {
        ctx.pop_scope();
    }

    if !ctx.is_terminated() {
        // Fallthrough advances the for index before carried-variable writes.
        // Explicit continue below retains its existing opposite order.
        if let Some(index) = frame.index_phi {
            advance_index(ctx, index, span);
        }
        emit_carried_values(ctx, &frame.edges.carried_phis, None, span);
        ctx.emit(
            Opcode::Jump,
            Ty::Unit,
            vec![],
            InstData::JumpTarget(frame.edges.header),
            span,
        );
    }

    for (name, exit_phi) in &frame.edges.exit_phis {
        ctx.assign(name, *exit_phi);
    }
    ctx.switch_to_block(frame.edges.exit);
    finish_result(ctx, frame.break_upsilons.unwrap_or_default(), span)
}

fn finish_result(ctx: &mut LowerCtx, breaks: Vec<BreakUpsilon>, span: Span) -> InstId {
    let Some(first) = breaks.first() else {
        return ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span);
    };
    let result = ctx.emit(Opcode::Phi, first.2, vec![], InstData::None, span);
    for (block, up_id, _, _) in &breaks {
        backpatch_upsilon(ctx, *block, *up_id, result);
    }
    let values: Vec<InstId> = breaks.iter().map(|entry| entry.3).collect();
    merge_compatible_aggregate_metadata(ctx, &values, result);
    result
}

pub(super) fn lower_loop_break(ctx: &mut LowerCtx, value: Option<&Expr>, span: Span) -> InstId {
    let exit_block = ctx
        .loop_frames
        .last()
        .expect("break outside of loop")
        .edges
        .exit;
    if let Some(value) = value {
        let value_id = lower_expr(ctx, value);
        let collects = ctx
            .loop_frames
            .last()
            .is_some_and(|frame| frame.break_upsilons.is_some());
        if collects {
            let ty = ctx.inst_ty(value_id);
            let upsilon = ctx.emit(
                Opcode::Upsilon,
                ty,
                vec![value_id],
                InstData::PhiTarget(InstId(u32::MAX)),
                span,
            );
            let block = ctx.current_block;
            ctx.loop_frames
                .last_mut()
                .and_then(|frame| frame.break_upsilons.as_mut())
                .expect("value loop")
                .push((block, upsilon, ty, value_id));
        }
    }
    let frame = ctx.loop_frames.last().expect("break outside of loop");
    let exit_phis = frame.edges.exit_phis.clone();
    let depth = frame.scope_depth;
    emit_carried_values(ctx, &exit_phis, Some(depth), span);
    ctx.emit(
        Opcode::Jump,
        Ty::Unit,
        vec![],
        InstData::JumpTarget(exit_block),
        span,
    )
}

pub(super) fn lower_loop_continue(ctx: &mut LowerCtx, span: Span) -> InstId {
    let frame = ctx.loop_frames.last().expect("continue outside of loop");
    let header = frame.edges.header;
    let carried = frame.edges.carried_phis.clone();
    let index = frame.index_phi;
    let depth = frame.scope_depth;
    emit_carried_values(ctx, &carried, Some(depth), span);
    if let Some(index) = index {
        advance_index(ctx, index, span);
    }
    ctx.emit(
        Opcode::Jump,
        Ty::Unit,
        vec![],
        InstData::JumpTarget(header),
        span,
    )
}

fn emit_carried_values(
    ctx: &mut LowerCtx,
    phis: &[(String, InstId)],
    depth: Option<usize>,
    span: Span,
) {
    for (name, phi) in phis {
        let value = if let Some(depth) = depth {
            ctx.lookup_at_depth(name, depth)
        } else {
            ctx.lookup(name)
        };
        if let Some(value) = value {
            ctx.emit(
                Opcode::Upsilon,
                ctx.inst_ty(value),
                vec![value],
                InstData::PhiTarget(*phi),
                span,
            );
        }
    }
}

fn advance_index(ctx: &mut LowerCtx, index: InstId, span: Span) {
    let one = ctx.emit(
        Opcode::ConstU64,
        Ty::U64,
        vec![],
        InstData::ConstU64(1),
        span,
    );
    let next = ctx.emit(
        Opcode::WrappingAdd,
        Ty::U64,
        vec![index, one],
        InstData::Integer(IntegerType::U64),
        span,
    );
    ctx.emit(
        Opcode::Upsilon,
        Ty::U64,
        vec![next],
        InstData::PhiTarget(index),
        span,
    );
}
