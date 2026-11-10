//! Enum payload construction and binding share declaration-backed layout here.
//! Callers keep builtin dispatch, match control flow and scope lifetime.
use super::*;

struct PayloadLayout {
    tys: Vec<Ty>,
    slots: Vec<u32>,
}

impl PayloadLayout {
    fn new(tys: Vec<Ty>) -> Self {
        let mut slots = Vec::with_capacity(tys.len() + 1);
        slots.push(1);
        for ty in &tys {
            slots.push(slots.last().copied().unwrap() + payload_slot_width(*ty));
        }
        Self { tys, slots }
    }

    fn slot(&self, index: usize) -> u32 {
        // Error recovery may bind more fields than the declaration has. Keep
        // the old sum-of-declared-prefix fallback for those extra bindings.
        self.slots[index.min(self.tys.len())]
    }

    fn field_data(&self, index: usize, ty: Ty) -> InstData {
        payload_field_data(ty, self.slot(index))
    }

    fn wide_ty(&self, index: usize) -> Option<Ty> {
        self.tys
            .get(index)
            .copied()
            .filter(|ty| matches!(ty, Ty::I128 | Ty::U128))
    }

    fn allocation_size(&self) -> u32 {
        (self.slot(self.tys.len()) + 1) * 8
    }
}

struct Variant<'a> {
    enum_name: &'a str,
    name: &'a str,
    tag: i64,
    declared: PayloadLayout,
}

fn resolve_variant<'a>(ctx: &LowerCtx, path: &'a [String]) -> Variant<'a> {
    let enum_name = path.first().map(String::as_str).unwrap_or("");
    let name = path.get(1).map(String::as_str).unwrap_or("");
    let tag = ctx
        .enum_variant_map
        .get(enum_name)
        .and_then(|variants| variants.iter().position(|variant| variant == name))
        .unwrap_or(0);
    let tys = ctx
        .enum_variant_payload_tys
        .get(enum_name)
        .and_then(|variants| variants.get(tag))
        .cloned()
        .unwrap_or_default();
    Variant {
        enum_name,
        name,
        tag: tag as i64,
        declared: PayloadLayout::new(tys),
    }
}

pub(super) fn lower_constructor(ctx: &mut LowerCtx, expr: &Expr) -> InstId {
    let ExprKind::EnumConstruct { path, fields } = &expr.kind else {
        unreachable!("enum constructor")
    };
    let span = expr.span;
    let variant = resolve_variant(ctx, path);
    let payload_ast_types = ctx
        .enum_variant_payload_ast_types
        .get(variant.enum_name)
        .and_then(|variants| variants.get(variant.tag as usize))
        .cloned()
        .unwrap_or_default();
    // Evaluate and transfer payloads before allocating the wrapper so built-in
    // Option/Result inherit linear ownership from type-erased payloads.
    let payload_values: Vec<InstId> = fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            if let Some(expected) = payload_ast_types.get(index) {
                record_wide_expected_ast_context(ctx, field, expected);
            }
            if let Some(ty) = variant.declared.wide_ty(index) {
                record_wide_control_flow_context(ctx, field, ty);
            }
            let original = lower_consumed_expr(ctx, field);
            match variant.declared.wide_ty(index) {
                Some(ty) => lower_narrow_literal(ctx, field, original, ty),
                None => original,
            }
        })
        .collect();
    let contextual_wide_payload_ty = payload_values
        .first()
        .map(|value| ctx.inst_ty(*value))
        .filter(|ty| matches!(ty, Ty::I128 | Ty::U128));
    let owns_linear = ctx.linear_owner_names.contains(variant.enum_name)
        || payload_values
            .iter()
            .any(|value| ctx.inst_ty(*value) == Ty::LinearPtr);
    // Declarations decide storage width even if a lowered value disagrees.
    // Codegen then rejects that mismatch rather than using another slot map.
    let layout = PayloadLayout::new(
        payload_values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                variant
                    .declared
                    .tys
                    .get(index)
                    .copied()
                    .unwrap_or_else(|| ctx.inst_ty(*value))
            })
            .collect(),
    );
    let ptr_id = ctx.emit(
        Opcode::RegionAlloc,
        if owns_linear { Ty::LinearPtr } else { Ty::Ptr },
        vec![],
        InstData::AllocSize {
            size: layout.allocation_size(),
            align: 8,
        },
        span,
    );
    ctx.inst_struct_type
        .insert(ptr_id, variant.enum_name.to_string());
    if variant.enum_name == "Option" && variant.name == "Some" {
        if let Some(payload_ty) = contextual_wide_payload_ty {
            ctx.inst_option_elem_ty.insert(ptr_id, payload_ty);
        }
    } else if variant.enum_name == "Result"
        && matches!(variant.name, "Ok" | "Err")
        && let Some(payload_ty) = contextual_wide_payload_ty
    {
        let variant_count = ctx
            .enum_variant_map
            .get(variant.enum_name)
            .map_or(2, Vec::len);
        let mut variant_tys = vec![None; variant_count];
        if let Some(slot) = variant_tys.get_mut(variant.tag as usize) {
            *slot = Some(payload_ty);
        }
        ctx.inst_variant_payload_tys.insert(ptr_id, variant_tys);
    }
    let tag_val = ctx.emit(
        Opcode::ConstI64,
        Ty::I64,
        vec![],
        InstData::ConstI64(variant.tag),
        span,
    );
    ctx.emit(
        Opcode::FieldSet,
        Ty::Unit,
        vec![ptr_id, tag_val],
        InstData::FieldIndex(0),
        span,
    );
    for (index, val_id) in payload_values.into_iter().enumerate() {
        ctx.emit(
            Opcode::FieldSet,
            Ty::Unit,
            vec![ptr_id, val_id],
            layout.field_data(index, layout.tys[index]),
            span,
        );
    }
    ptr_id
}

pub(super) fn bind_variant(ctx: &mut LowerCtx, ptr_id: InstId, pattern: &Pat, span: Span) {
    let PatKind::EnumVariant { path, inner } = &pattern.kind else {
        unreachable!("enum variant pattern")
    };
    let variant = resolve_variant(ctx, path);
    // Builtin Option/Result have no declaration. Keep their contextual scalar
    // type, while checker aggregate metadata retains field/index knowledge.
    let payload_ty = variant_payload_ty(ctx, ptr_id, variant.tag)
        .or_else(|| ctx.inst_option_elem_ty.get(&ptr_id).copied())
        .unwrap_or(Ty::I64);
    for (index, inner_pat) in inner.iter().enumerate() {
        if let PatKind::Ident { name, .. } = &inner_pat.kind {
            let aggregate = ctx
                .pattern_aggregates
                .get(&(inner_pat as *const _ as usize))
                .cloned();
            // Declarations preserve wide widths even for parameter payloads;
            // narrow declarations keep the existing checked/contextual path.
            let declared_wide = variant.declared.wide_ty(index);
            let checked_scalar = checked_payload_ty(ctx, inner_pat as *const _ as usize);
            let field_ty = if aggregate.as_ref().is_some_and(|info| info.is_linear) {
                Ty::LinearPtr
            } else if aggregate.is_some() {
                Ty::Ptr
            } else if let Some(wide) = declared_wide {
                wide
            } else if let Some(scalar) = checked_scalar {
                scalar
            } else if index == 0 {
                payload_ty
            } else {
                Ty::I64
            };
            let field_val = ctx.emit(
                Opcode::FieldGet,
                field_ty,
                vec![ptr_id],
                variant.declared.field_data(index, field_ty),
                span,
            );
            if let Some(info) = aggregate {
                apply_pattern_aggregate_metadata(ctx, field_val, info);
            }
            ctx.define(name.clone(), field_val);
        }
    }
}
