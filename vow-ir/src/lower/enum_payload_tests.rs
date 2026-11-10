#[cfg(test)]
mod tests {
    use super::super::*;

    fn named(name: &str) -> AstType {
        AstType::Named {
            name: name.into(),
            span: Span::new(0, 0),
        }
    }

    fn context() -> LowerCtx {
        let mut ctx = LowerCtx::new(
            "enum_payloads".into(),
            vec![],
            vec![],
            Ty::I64,
            vec![],
            "enum_payloads.vow".into(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([("Packet".into(), vec!["Empty".into(), "Data".into()])]),
            HashSet::new(),
            Rc::new(HashMap::new()),
            HashMap::new(),
            Rc::new(HashMap::new()),
            HashMap::new(),
            StringExprSet::new(),
            Rc::new(PatternAggregateMap::new()),
            Rc::new(PayloadScalarMap::new()),
        );
        ctx.enum_variant_payload_tys.insert(
            "Packet".into(),
            vec![vec![], vec![Ty::I64, Ty::I128, Ty::U128, Ty::U8]],
        );
        ctx.enum_variant_payload_ast_types = Rc::new(HashMap::from([(
            "Packet".into(),
            vec![
                vec![],
                vec![named("i64"), named("i128"), named("u128"), named("u8")],
            ],
        )]));
        ctx
    }

    fn expression(text: &str) -> Expr {
        let source = format!("module EnumPayloads\nfn f() {{ {text} }}");
        let (module, diagnostics) = vow_syntax::parser::parse_module(&source, "enum_payloads.vow");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let Item::Fn(function) = &module.items[0] else {
            panic!("function")
        };
        *function.body.trailing_expr.clone().expect("expression")
    }

    fn construct(ctx: &mut LowerCtx, expr: &Expr) -> InstId {
        enum_payload::lower_constructor(ctx, expr)
    }

    fn instructions(ctx: &LowerCtx) -> Vec<&Inst> {
        ctx.func
            .blocks
            .iter()
            .flat_map(|block| &block.insts)
            .collect()
    }

    fn parameter(ctx: &mut LowerCtx) -> InstId {
        let wrapper = ctx.emit(
            Opcode::GetArg,
            Ty::Ptr,
            vec![],
            InstData::ArgIndex(0),
            Span::new(0, 0),
        );
        ctx.define("packet".into(), wrapper);
        wrapper
    }

    fn payload_reads(ctx: &LowerCtx) -> Vec<&Inst> {
        instructions(ctx)
            .into_iter()
            .filter(|inst| inst.opcode == Opcode::FieldGet && inst.data != InstData::FieldIndex(0))
            .collect()
    }

    #[test]
    fn enum_payload_constructor_preserves_mixed_wide_layout_and_source_origins() {
        let mut ctx = context();
        let expr = expression("Packet::Data(7, { if true { 9 } else { 11 } }, 13, 17u8)");
        let wrapper = construct(&mut ctx, &expr);
        let insts = instructions(&ctx);
        let alloc = insts.iter().find(|inst| inst.id == wrapper).unwrap();
        assert_eq!(alloc.data, InstData::AllocSize { size: 64, align: 8 });
        assert_eq!(alloc.origin, expr.span);
        let stores: Vec<_> = insts
            .iter()
            .filter(|inst| inst.opcode == Opcode::FieldSet)
            .collect();
        assert_eq!(
            stores
                .iter()
                .map(|inst| inst.data.clone())
                .collect::<Vec<_>>(),
            vec![
                InstData::FieldIndex(0),
                InstData::FieldIndex(1),
                InstData::WideSlot(2),
                InstData::WideSlot(4),
                InstData::FieldIndex(6),
            ]
        );
        assert_eq!(ctx.inst_ty(stores[2].args[1]), Ty::I128);
        assert_eq!(ctx.inst_ty(stores[3].args[1]), Ty::U128);
        assert!(stores.iter().all(|inst| inst.origin == expr.span));
        assert_eq!(
            ctx.inst_struct_type.get(&wrapper).map(String::as_str),
            Some("Packet")
        );
        let tag = insts
            .iter()
            .find(|inst| inst.id == stores[0].args[1])
            .unwrap();
        assert_eq!(tag.data, InstData::ConstI64(1));
    }

    #[test]
    fn enum_payload_constructor_keeps_declaration_layout_when_value_types_disagree() {
        let mut ctx = context();
        let narrow = ctx.emit(
            Opcode::GetArg,
            Ty::I64,
            vec![],
            InstData::ArgIndex(0),
            Span::new(0, 0),
        );
        ctx.define("narrow".into(), narrow);
        construct(
            &mut ctx,
            &expression("Packet::Data(1i128, narrow, 3u128, 4u8)"),
        );
        let stores: Vec<_> = instructions(&ctx)
            .into_iter()
            .filter(|inst| inst.opcode == Opcode::FieldSet)
            .collect();
        assert_eq!(stores[1].data, InstData::FieldIndex(1));
        assert_eq!(ctx.inst_ty(stores[1].args[1]), Ty::I128);
        assert_eq!(stores[2].data, InstData::WideSlot(2));
        assert_eq!(ctx.inst_ty(stores[2].args[1]), Ty::I128);
        assert_eq!(stores[3].data, InstData::WideSlot(4));
    }

    #[test]
    fn enum_payload_constructor_transfers_linear_payloads_before_allocating_the_wrapper() {
        let mut ctx = context();
        let token = ctx.emit(
            Opcode::GetArg,
            Ty::LinearPtr,
            vec![],
            InstData::ArgIndex(0),
            Span::new(0, 0),
        );
        ctx.define("token".into(), token);
        let expr = expression("Option::Some(token)");
        let wrapper = construct(&mut ctx, &expr);
        let insts = instructions(&ctx);
        let consume = insts
            .iter()
            .position(|inst| inst.opcode == Opcode::LinearConsume)
            .unwrap();
        let alloc = insts.iter().position(|inst| inst.id == wrapper).unwrap();
        assert!(consume < alloc);
        assert_eq!(insts[consume].args, vec![token]);
        assert_eq!(ctx.inst_ty(wrapper), Ty::LinearPtr);
        assert_eq!(
            insts[alloc].data,
            InstData::AllocSize { size: 24, align: 8 }
        );
        assert_eq!(
            insts
                .iter()
                .filter(|inst| inst.opcode == Opcode::LinearConsume)
                .count(),
            1
        );
    }

    #[test]
    fn enum_payload_builtins_publish_wide_context_and_use_value_layout() {
        for (source, name, expected_tag, ty) in [
            ("Option::Some(23i128)", "Option", 1, Ty::I128),
            ("Result::Ok(29u128)", "Result", 0, Ty::U128),
            ("Result::Err(31i128)", "Result", 1, Ty::I128),
        ] {
            let mut ctx = context();
            let wrapper = construct(&mut ctx, &expression(source));
            let insts = instructions(&ctx);
            let alloc = insts.iter().find(|inst| inst.id == wrapper).unwrap();
            assert_eq!(alloc.data, InstData::AllocSize { size: 32, align: 8 });
            let store = insts
                .iter()
                .rev()
                .find(|inst| inst.opcode == Opcode::FieldSet)
                .unwrap();
            assert_eq!(store.data, InstData::WideSlot(1));
            assert_eq!(ctx.inst_ty(store.args[1]), ty);
            assert_eq!(
                ctx.inst_struct_type.get(&wrapper).map(String::as_str),
                Some(name)
            );
            if name == "Option" {
                assert_eq!(ctx.inst_option_elem_ty.get(&wrapper), Some(&ty));
            } else {
                assert_eq!(
                    ctx.inst_variant_payload_tys[&wrapper][expected_tag],
                    Some(ty)
                );
            }
        }
    }

    #[test]
    fn enum_payload_pattern_accounts_for_wildcards_and_declared_wide_slots() {
        let mut ctx = context();
        parameter(&mut ctx);
        let expr = expression("match packet { Packet::Data(_, wide, other, byte) => 0 }");
        lower_expr(&mut ctx, &expr);
        let reads = payload_reads(&ctx);
        assert_eq!(
            reads
                .iter()
                .map(|inst| (inst.ty, inst.data.clone()))
                .collect::<Vec<_>>(),
            vec![
                (Ty::I128, InstData::WideSlot(2)),
                (Ty::U128, InstData::WideSlot(4)),
                (Ty::I64, InstData::FieldIndex(6)),
            ]
        );
        assert!(reads.iter().all(|inst| inst.origin == expr.span));
    }

    #[test]
    fn enum_payload_pattern_preserves_aggregate_and_checked_scalar_precedence() {
        let mut ctx = context();
        let wrapper = parameter(&mut ctx);
        ctx.inst_variant_payload_tys
            .insert(wrapper, vec![None, Some(Ty::F64)]);
        ctx.inst_option_elem_ty.insert(wrapper, Ty::U64);
        let expr = expression("match packet { Packet::Data(first, wide, aggregate, byte) => 0 }");
        let ExprKind::Match { arms, .. } = &expr.kind else {
            panic!("match")
        };
        let PatKind::EnumVariant { inner, .. } = &arms[0].pattern.kind else {
            panic!("variant")
        };
        let key = |index: usize| &inner[index] as *const Pat as usize;
        ctx.payload_scalars = Rc::new(HashMap::from([
            (key(0), PatternScalarType::U16),
            (key(1), PatternScalarType::I8),
            (key(2), PatternScalarType::U32),
            (key(3), PatternScalarType::U8),
        ]));
        ctx.pattern_aggregates = Rc::new(HashMap::from([(
            key(2),
            PatternAggregateInfo {
                type_name: "Vec".into(),
                vec_elem_types: vec!["Option".into()],
                vec_option_elem_types: vec![Some(PatternScalarType::I128)],
                vec_variant_payload_types: vec![vec![None, Some(PatternScalarType::U128)]],
                option_elem_type: Some(PatternScalarType::I32),
                variant_payload_types: vec![Some(PatternScalarType::U16)],
                is_linear: true,
            },
        )]));
        lower_expr(&mut ctx, &expr);
        let reads = payload_reads(&ctx);
        assert_eq!(
            reads
                .iter()
                .map(|inst| (inst.ty, inst.data.clone()))
                .collect::<Vec<_>>(),
            vec![
                (Ty::U16, InstData::FieldIndex(1)),
                (Ty::I128, InstData::WideSlot(2)),
                (Ty::LinearPtr, InstData::FieldIndex(4)),
                (Ty::U8, InstData::FieldIndex(6)),
            ]
        );
        let aggregate = reads[2].id;
        assert_eq!(
            ctx.inst_struct_type.get(&aggregate).map(String::as_str),
            Some("Vec")
        );
        assert_eq!(
            ctx.inst_vec_elem_types[&aggregate],
            vec!["Option".to_string()]
        );
        assert_eq!(
            ctx.inst_vec_option_elem_tys[&aggregate],
            vec![Some(Ty::I128)]
        );
        assert_eq!(
            ctx.inst_vec_variant_payload_tys[&aggregate],
            vec![vec![None, Some(Ty::U128)]]
        );
        assert_eq!(ctx.inst_option_elem_ty[&aggregate], Ty::I32);
        assert_eq!(
            ctx.inst_variant_payload_tys[&aggregate],
            vec![Some(Ty::U16)]
        );
    }

    #[test]
    fn enum_payload_missing_declarations_keep_context_and_extra_pattern_fallbacks() {
        let mut ctx = context();
        let wrapper = parameter(&mut ctx);
        ctx.inst_variant_payload_tys
            .insert(wrapper, vec![Some(Ty::F32)]);
        ctx.inst_option_elem_ty.insert(wrapper, Ty::U64);
        let expr = expression("match packet { Unknown::Missing(first, second, third) => 0 }");
        lower_expr(&mut ctx, &expr);
        let reads = payload_reads(&ctx);
        assert_eq!(
            reads
                .iter()
                .map(|inst| (inst.ty, inst.data.clone()))
                .collect::<Vec<_>>(),
            vec![
                (Ty::F32, InstData::FieldIndex(1)),
                (Ty::I64, InstData::FieldIndex(1)),
                (Ty::I64, InstData::FieldIndex(1)),
            ]
        );
        let mut ctx = context();
        let wrapper = construct(&mut ctx, &expression("Unknown::Missing(7i128, 9u8)"));
        let insts = instructions(&ctx);
        let alloc = insts.iter().find(|inst| inst.id == wrapper).unwrap();
        assert_eq!(alloc.data, InstData::AllocSize { size: 40, align: 8 });
        let stores: Vec<_> = insts
            .iter()
            .filter(|inst| inst.opcode == Opcode::FieldSet)
            .collect();
        let tag = insts
            .iter()
            .find(|inst| inst.id == stores[0].args[1])
            .unwrap();
        assert_eq!(tag.data, InstData::ConstI64(0));
        assert_eq!(stores[1].data, InstData::WideSlot(1));
        assert_eq!(stores[2].data, InstData::FieldIndex(3));
    }

    #[test]
    fn enum_payload_pattern_uses_option_context_when_variant_metadata_is_absent() {
        let mut ctx = context();
        let wrapper = parameter(&mut ctx);
        ctx.inst_option_elem_ty.insert(wrapper, Ty::U8);
        lower_expr(
            &mut ctx,
            &expression("match packet { Option::Some(value) => 0 }"),
        );
        let reads = payload_reads(&ctx);
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].ty, Ty::U8);
        assert_eq!(reads[0].data, InstData::FieldIndex(1));
    }
}
