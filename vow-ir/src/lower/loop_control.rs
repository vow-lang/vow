//! Complete loop lowering, from syntax and SSA construction through completion.
//!
//! Callers supply syntax. Header/exit Phis, metadata, natural edges and active
//! frames stay here, alongside lexical transfers and expression results.

use super::{
    LowerCtx, backpatch_upsilon, checked_float_vec_elem, checked_wide_vec_elem,
    collect_loop_assigned_vars, emit_wide_vec_elem_get, float_from_vec_slot, foreach_elem_is_bool,
    lower_block, lower_expr, merge_compatible_aggregate_metadata, narrow_vec_bool_elem,
    project_vec_index_metadata, record_collection_foreach_element, vow,
};
use crate::types::{BlockId, InstData, InstId, IntegerType, Opcode, Ty};
use vow_syntax::ast::{Block, Expr, VowBlock};
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

pub(super) fn lower_while_expr(
    ctx: &mut LowerCtx,
    condition: &Expr,
    body: &Block,
    invariant: Option<&VowBlock>,
    span: Span,
) -> InstId {
    let variables = ctx.in_scope_vars(collect_loop_assigned_vars(Some(condition), body));
    let header = ctx.new_block();
    let body_block = ctx.new_block();
    let exit = ctx.new_block();
    let (carried_phis, _) = begin_header(ctx, &variables, header, None, span);
    if let Some(invariant) = invariant {
        vow::lower_invariant(ctx, invariant);
    }
    let condition = lower_expr(ctx, condition);
    let exit_phis = make_exit_phis(ctx, &variables, exit, span);
    // Read after the condition: its final false evaluation may assign a variable.
    for (name, phi) in &exit_phis {
        let value = ctx
            .lookup(name)
            .expect("loop variable is rebound to its header Phi");
        ctx.emit(
            Opcode::Upsilon,
            ctx.inst_ty(value),
            vec![value],
            InstData::PhiTarget(*phi),
            span,
        );
    }
    branch_body(ctx, condition, body_block, exit, span);
    lower_while_body(
        ctx,
        LoopEdges {
            header,
            exit,
            carried_phis,
            exit_phis,
        },
        body,
        span,
    )
}

pub(super) fn lower_loop_expr(
    ctx: &mut LowerCtx,
    body: &Block,
    invariant: Option<&VowBlock>,
    span: Span,
) -> InstId {
    let variables = ctx.in_scope_vars(collect_loop_assigned_vars(None, body));
    let header = ctx.new_block();
    let exit = ctx.new_block();
    let (carried_phis, _) = begin_header(ctx, &variables, header, None, span);
    if let Some(invariant) = invariant {
        vow::lower_invariant(ctx, invariant);
    }
    let exit_phis = make_exit_phis(ctx, &variables, exit, span);
    ctx.switch_to_block(header);
    lower_loop_body(
        ctx,
        LoopEdges {
            header,
            exit,
            carried_phis,
            exit_phis,
        },
        body,
        span,
    )
}

pub(super) fn lower_foreach_expr(
    ctx: &mut LowerCtx,
    expr: &Expr,
    binding: &str,
    iterable: &Expr,
    body: &Block,
    invariant: Option<&VowBlock>,
    span: Span,
) -> InstId {
    let iterable = lower_expr(ctx, iterable);
    ctx.inst_struct_type.insert(iterable, "Vec".to_string());
    let length = ctx.emit(
        Opcode::Call,
        Ty::U64,
        vec![iterable],
        InstData::CallExtern("__vow_vec_len".to_string()),
        span,
    );
    let initial_index = ctx.emit(
        Opcode::ConstU64,
        Ty::U64,
        vec![],
        InstData::ConstU64(0),
        span,
    );
    let variables = ctx.in_scope_vars(collect_loop_assigned_vars(None, body));
    let header = ctx.new_block();
    let body_block = ctx.new_block();
    let exit = ctx.new_block();
    let (carried_phis, index) = begin_header(ctx, &variables, header, Some(initial_index), span);
    let index = index.expect("foreach index Phi");
    if let Some(invariant) = invariant {
        vow::lower_invariant(ctx, invariant);
    }
    let condition = ctx.emit(
        Opcode::Lt,
        Ty::Bool,
        vec![index, length],
        InstData::Integer(IntegerType::U64),
        span,
    );
    let exit_phis = make_exit_phis(ctx, &variables, exit, span);
    ctx.switch_to_block(header);
    // Foreach's natural edge supplies the header values, in mutation order.
    for ((_, phi), (_, exit_phi)) in carried_phis.iter().zip(&exit_phis) {
        ctx.emit(
            Opcode::Upsilon,
            ctx.inst_ty(*phi),
            vec![*phi],
            InstData::PhiTarget(*exit_phi),
            span,
        );
    }
    branch_body(ctx, condition, body_block, exit, span);
    let element = if let Some(wide_ty) = checked_wide_vec_elem(ctx, expr) {
        emit_wide_vec_elem_get(ctx, iterable, index, wide_ty, span)
    } else {
        let raw = ctx.emit(
            Opcode::Call,
            Ty::I64,
            vec![iterable, index],
            InstData::CallExtern("__vow_vec_get_val".to_string()),
            span,
        );
        if let Some(float_ty) = checked_float_vec_elem(ctx, expr) {
            float_from_vec_slot(ctx, raw, float_ty, span)
        } else if foreach_elem_is_bool(ctx, iterable) {
            narrow_vec_bool_elem(ctx, raw, span)
        } else {
            raw
        }
    };
    project_vec_index_metadata(ctx, iterable, element);
    record_collection_foreach_element(ctx, iterable, element);
    lower_foreach_body(
        ctx,
        LoopEdges {
            header,
            exit,
            carried_phis,
            exit_phis,
        },
        binding,
        element,
        index,
        body,
        span,
    )
}

// Initial inputs and Phis have identical order. An anonymous foreach index is
// first on both sides; named inputs are patched by position, without name scans.
fn begin_header(
    ctx: &mut LowerCtx,
    variables: &[(String, InstId)],
    header: BlockId,
    index: Option<InstId>,
    span: Span,
) -> (Vec<(String, InstId)>, Option<InstId>) {
    let pre_header = ctx.current_block;
    let index_input = index.map(|value| pending_input(ctx, value, span));
    let inputs: Vec<_> = variables
        .iter()
        .map(|(_, value)| pending_input(ctx, *value, span))
        .collect();
    ctx.emit(
        Opcode::Jump,
        Ty::Unit,
        vec![],
        InstData::JumpTarget(header),
        span,
    );
    ctx.switch_to_block(header);
    let index_phi = index_input.map(|input| {
        let phi = ctx.emit(Opcode::Phi, Ty::U64, vec![], InstData::None, span);
        backpatch_upsilon(ctx, pre_header, input, phi);
        phi
    });
    let phis = make_phis(ctx, variables, span);
    for (input, (_, phi)) in inputs.iter().zip(&phis) {
        backpatch_upsilon(ctx, pre_header, *input, *phi);
    }
    for (name, phi) in &phis {
        ctx.assign(name, *phi);
    }
    (phis, index_phi)
}

fn pending_input(ctx: &mut LowerCtx, value: InstId, span: Span) -> InstId {
    ctx.emit(
        Opcode::Upsilon,
        ctx.inst_ty(value),
        vec![value],
        InstData::PhiTarget(InstId(u32::MAX)),
        span,
    )
}

fn make_phis(
    ctx: &mut LowerCtx,
    variables: &[(String, InstId)],
    span: Span,
) -> Vec<(String, InstId)> {
    variables
        .iter()
        .map(|(name, value)| {
            let phi = ctx.emit(
                Opcode::Phi,
                ctx.inst_ty(*value),
                vec![],
                InstData::None,
                span,
            );
            merge_compatible_aggregate_metadata(ctx, &[*value], phi);
            (name.clone(), phi)
        })
        .collect()
}

fn make_exit_phis(
    ctx: &mut LowerCtx,
    variables: &[(String, InstId)],
    exit: BlockId,
    span: Span,
) -> Vec<(String, InstId)> {
    let current = ctx.current_block;
    ctx.switch_to_block(exit);
    let phis = make_phis(ctx, variables, span);
    ctx.switch_to_block(current);
    phis
}

fn branch_body(ctx: &mut LowerCtx, condition: InstId, body: BlockId, exit: BlockId, span: Span) {
    ctx.emit(
        Opcode::Branch,
        Ty::Unit,
        vec![condition],
        InstData::BranchTargets {
            then_block: body,
            else_block: exit,
        },
        span,
    );
    ctx.switch_to_block(body);
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
