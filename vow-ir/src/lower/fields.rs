//! Complete declaration-backed struct reads, writes, and construction.
//! The caller supplies source; field facts and their IR application stay here.
use super::*;

#[derive(Clone, Copy)]
enum FieldUse {
    Read(InstId),
    Write(InstId),
    Initializer,
}

struct DeclaredField {
    ty: Ty,
    data: InstData,
    ast_type: Option<AstType>,
    aggregate: Option<String>,
    vec_element: Option<String>,
}

// Resolve every fact used by a field access together. Diagnostics retain the
// different read/write/initializer fallback rules of the original lowerer.
fn declared_field(
    ctx: &mut LowerCtx,
    struct_name: &str,
    field: &str,
    usage: FieldUse,
    span: Span,
) -> Option<DeclaredField> {
    if struct_name.is_empty() {
        let access = match usage {
            FieldUse::Read(id) => Some(("FieldGet", id)),
            FieldUse::Write(id) => Some(("FieldSet", id)),
            FieldUse::Initializer => None,
        };
        if let Some((operation, id)) = access {
            ctx.warn(
                format!("{operation} on untagged instruction %{}, field '{}' -- ICE: returning sentinel index", id.0, field),
                span,
            );
        }
    }
    let Some(names) = ctx.struct_field_map.get(struct_name) else {
        if !struct_name.is_empty() && !matches!(usage, FieldUse::Initializer) {
            ctx.warn(
                format!(
                    "struct '{}' not registered -- field lookup ICE: returning sentinel index",
                    struct_name
                ),
                span,
            );
        }
        return None;
    };
    let Some(index) = names.iter().position(|name| name == field) else {
        let message = if matches!(usage, FieldUse::Initializer) {
            (!names.is_empty()).then(|| format!("StructLiteral field '{}' not found in struct '{}' -- ICE: returning sentinel index", field, struct_name))
        } else {
            (!struct_name.is_empty()).then(|| {
                format!(
                    "field '{}' not found in struct '{}' -- ICE: returning sentinel index",
                    field, struct_name
                )
            })
        };
        if let Some(message) = message {
            ctx.warn(message, span);
        }
        return None;
    };
    let type_name = ctx
        .struct_field_type_names
        .get(struct_name)
        .and_then(|names| names.get(index))
        .map(String::as_str)
        .unwrap_or_default();
    let ty = scalar_ty_for_field_type_name(type_name);
    let aggregate = (matches!(usage, FieldUse::Read(_))
        && !type_name.is_empty()
        && !is_scalar_field_type_name(type_name))
    .then(|| type_name.to_string());
    let ast_type = if matches!(usage, FieldUse::Write(_)) {
        None
    } else {
        ctx.struct_field_ast_types
            .get(struct_name)
            .and_then(|types| types.get(index))
            .cloned()
    };
    let vec_element = if matches!(usage, FieldUse::Read(_)) {
        ctx.struct_field_vec_elems
            .get(struct_name)
            .and_then(|names| names.get(index))
            .filter(|name| !name.is_empty())
            .cloned()
    } else {
        None
    };
    Some(DeclaredField {
        ty,
        data: struct_field_data(ctx, struct_name, index),
        ast_type,
        aggregate,
        vec_element,
    })
}

fn lower_read(ctx: &mut LowerCtx, base: &Expr, field: &str, span: Span) -> InstId {
    let ptr = lower_expr(ctx, base);
    let struct_name = ctx.inst_struct_type.get(&ptr).cloned().unwrap_or_default();
    let Some(declaration) = declared_field(ctx, &struct_name, field, FieldUse::Read(ptr), span)
    else {
        return ctx.emit(
            Opcode::ConstI64,
            Ty::I64,
            vec![],
            InstData::ConstI64(0),
            span,
        );
    };
    let result = ctx.emit(
        Opcode::FieldGet,
        declaration.ty,
        vec![ptr],
        declaration.data,
        span,
    );
    if let Some(ast_type) = declaration.ast_type {
        ctx.inst_declared_ast_types.insert(result, ast_type);
    }
    if let Some(aggregate) = declaration.aggregate {
        ctx.inst_struct_type.insert(result, aggregate);
    }
    if let Some(element) = declaration.vec_element {
        ctx.inst_vec_elem_types.insert(result, vec![element]);
    }
    result
}

fn lower_assignment(ctx: &mut LowerCtx, lhs: &Expr, rhs: &Expr, span: Span) -> InstId {
    let ExprKind::FieldAccess { base, field } = &lhs.kind else {
        unreachable!("struct assignment requires a field target")
    };
    if let Some(expected) = known_assignment_ast_type(ctx, lhs) {
        record_wide_expected_ast_context(ctx, rhs, &expected);
    }
    if let Some(field_ty) = known_field_assignment_ty(ctx, lhs) {
        record_wide_control_flow_context(ctx, rhs, field_ty);
    }
    // Preserve Rust's RHS-before-receiver order and RHS expression result.
    let mut value = lower_expr(ctx, rhs);
    let ptr = lower_expr(ctx, base);
    let struct_name = ctx.inst_struct_type.get(&ptr).cloned().unwrap_or_default();
    if let Some(declaration) = declared_field(ctx, &struct_name, field, FieldUse::Write(ptr), span)
    {
        if matches!(declaration.ty, Ty::I128 | Ty::U128) {
            value = lower_narrow_literal(ctx, rhs, value, declaration.ty);
        }
        ctx.emit_linear_consume_if_needed(value, span);
        ctx.emit(
            Opcode::FieldSet,
            Ty::Unit,
            vec![ptr, value],
            declaration.data,
            span,
        );
    }
    value
}

fn lower_literal(ctx: &mut LowerCtx, name: &str, fields: &[(String, Expr)], span: Span) -> InstId {
    let field_names = if let Some(names) = ctx.struct_field_map.get(name) {
        names.clone()
    } else {
        ctx.warn(
            format!(
                "struct '{}' not registered -- field lookup ICE: returning sentinel index",
                name
            ),
            span,
        );
        vec![]
    };
    let n_fields = field_names.len().max(fields.len());
    let n_slots = (struct_field_slot(ctx, name, field_names.len()) as usize).max(n_fields);
    let ty = if ctx.linear_owner_names.contains(name) {
        Ty::LinearPtr
    } else {
        Ty::Ptr
    };
    let ptr = ctx.emit(
        Opcode::RegionAlloc,
        ty,
        vec![],
        InstData::AllocSize {
            size: (n_slots as u32 + 1) * 8,
            align: 8,
        },
        span,
    );
    ctx.inst_struct_type.insert(ptr, name.into());
    for (field, expr) in fields {
        let declaration = declared_field(ctx, name, field, FieldUse::Initializer, span);
        if let Some(declaration) = &declaration {
            if let Some(expected) = &declaration.ast_type {
                record_wide_expected_ast_context(ctx, expr, expected);
            }
            if matches!(declaration.ty, Ty::I128 | Ty::U128) {
                record_wide_control_flow_context(ctx, expr, declaration.ty);
            }
        }
        let mut value = lower_consumed_expr(ctx, expr);
        if let Some(declaration) = declaration {
            if matches!(declaration.ty, Ty::I128 | Ty::U128) {
                value = lower_narrow_literal(ctx, expr, value, declaration.ty);
            }
            ctx.emit(
                Opcode::FieldSet,
                Ty::Unit,
                vec![ptr, value],
                declaration.data,
                span,
            );
        }
    }
    ptr
}

/// Lower one field read, field assignment, or struct literal completely.
/// All other expression forms remain the caller's responsibility.
pub(super) fn lower_struct_expr(ctx: &mut LowerCtx, expr: &Expr) -> InstId {
    match &expr.kind {
        ExprKind::FieldAccess { base, field } => lower_read(ctx, base, field, expr.span),
        ExprKind::Assign { lhs, rhs } => lower_assignment(ctx, lhs, rhs, expr.span),
        ExprKind::StructLiteral { name, fields } => lower_literal(ctx, name, fields, expr.span),
        _ => unreachable!("struct lowering requires a field access, field assignment, or literal"),
    }
}
