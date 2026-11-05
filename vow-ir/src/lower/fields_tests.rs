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
            "fields".into(),
            vec![],
            vec![],
            Ty::I64,
            vec![],
            "fields.vow".into(),
            HashMap::new(),
            HashMap::from([(
                "S".into(),
                vec!["first".into(), "wide".into(), "last".into(), "items".into()],
            )]),
            HashMap::new(),
            HashSet::new(),
            Rc::new(HashMap::new()),
            HashMap::from([(
                "S".into(),
                vec!["i64".into(), "i128".into(), "u8".into(), "Vec".into()],
            )]),
            Rc::new(HashMap::from([(
                "S".into(),
                vec![
                    named("i64"),
                    named("i128"),
                    named("u8"),
                    AstType::Generic {
                        name: "Vec".into(),
                        args: vec![named("Box")],
                        span: Span::new(0, 0),
                    },
                ],
            )])),
            HashMap::from([(
                "S".into(),
                vec!["".into(), "".into(), "".into(), "Box".into()],
            )]),
            StringExprSet::new(),
            Rc::new(PatternAggregateMap::new()),
            Rc::new(PayloadScalarMap::new()),
        );
        let ptr = ctx.emit(
            Opcode::ConstI64,
            Ty::Ptr,
            vec![],
            InstData::ConstI64(0),
            Span::new(0, 1),
        );
        ctx.inst_struct_type.insert(ptr, "S".into());
        ctx.define("s".into(), ptr);
        let x = ctx.emit(
            Opcode::ConstI64,
            Ty::I64,
            vec![],
            InstData::ConstI64(0),
            Span::new(0, 1),
        );
        ctx.define("x".into(), x);
        ctx
    }

    fn expression(text: &str) -> Expr {
        let source = format!("module Fields\nfn f() {{ {text} }}");
        let (parsed, diagnostics) = vow_syntax::parser::parse_module(&source, "fields.vow");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let Item::Fn(function) = &parsed.items[0] else {
            panic!("function")
        };
        *function
            .body
            .trailing_expr
            .clone()
            .expect("field expression")
    }

    fn field_expression(ctx: &mut LowerCtx, expr: &Expr) -> InstId {
        super::super::fields::lower_struct_expr(ctx, expr)
    }

    fn insts(ctx: &LowerCtx) -> Vec<&Inst> {
        ctx.func.blocks.iter().flat_map(|b| &b.insts).collect()
    }

    #[test]
    fn struct_fields_construct_in_source_order_with_declared_wide_layout() {
        let mut ctx = context();
        let expr = expression("S { last: 7, wide: { 9 }, first: 11, items: Vec::new() }");
        let ptr = field_expression(&mut ctx, &expr);
        assert_eq!(
            ctx.inst_struct_type.get(&ptr).map(String::as_str),
            Some("S")
        );
        let instructions = insts(&ctx);
        let alloc = instructions.iter().find(|i| i.id == ptr).unwrap();
        assert_eq!(alloc.data, InstData::AllocSize { size: 48, align: 8 });
        assert_eq!(alloc.origin, expr.span);
        let stores: Vec<_> = instructions
            .iter()
            .filter(|i| i.opcode == Opcode::FieldSet)
            .collect();
        assert_eq!(
            stores.iter().map(|i| i.data.clone()).collect::<Vec<_>>(),
            vec![
                InstData::FieldIndex(3),
                InstData::WideSlot(1),
                InstData::FieldIndex(0),
                InstData::FieldIndex(4),
            ]
        );
        assert_eq!(ctx.inst_ty(stores[1].args[1]), Ty::I128);
        assert!(stores.iter().all(|i| i.origin == expr.span));
        assert!(ctx.warnings.is_empty());
    }

    #[test]
    fn struct_fields_read_preserves_declared_type_tags_and_element_metadata() {
        let mut ctx = context();
        let expr = expression("s.items");
        let result = field_expression(&mut ctx, &expr);
        let instruction = insts(&ctx).into_iter().find(|i| i.id == result).unwrap();
        assert_eq!(instruction.opcode, Opcode::FieldGet);
        assert_eq!(instruction.data, InstData::FieldIndex(4));
        assert_eq!(instruction.origin, expr.span);
        assert_eq!(
            ctx.inst_struct_type.get(&result).map(String::as_str),
            Some("Vec")
        );
        assert_eq!(
            ctx.inst_vec_elem_types.get(&result),
            Some(&vec!["Box".to_string()])
        );
        let AstType::Generic { name, args, .. } = &ctx.inst_declared_ast_types[&result] else {
            panic!("declared Vec type")
        };
        assert_eq!(name, "Vec");
        assert_eq!(args.len(), 1);
        let wide_expr = expression("s.wide");
        let wide = field_expression(&mut ctx, &wide_expr);
        assert_eq!(ctx.inst_ty(wide), Ty::I128);
        assert_eq!(insts(&ctx).last().unwrap().data, InstData::WideSlot(1));
    }

    #[test]
    fn struct_fields_assignment_keeps_rhs_before_receiver_and_returns_rhs() {
        let mut ctx = context();
        let expr = expression("({ x = 2; s }).wide = { x = 1; 3i128 }");
        let result = field_expression(&mut ctx, &expr);
        let instructions = insts(&ctx);
        let constants: Vec<_> = instructions
            .iter()
            .filter_map(|i| match i.data {
                InstData::ConstI64(n) => Some(n),
                _ => None,
            })
            .collect();
        assert_eq!(constants, vec![0, 0, 1, 2]);
        let last = instructions.last().unwrap();
        assert_eq!(last.opcode, Opcode::FieldSet);
        assert_eq!(last.data, InstData::WideSlot(1));
        assert_eq!(last.args[1], result);
        assert_eq!(ctx.inst_ty(result), Ty::I128);
        assert_eq!(last.origin, expr.span);
        assert_eq!(
            ctx.lookup("x"),
            Some(
                instructions
                    .iter()
                    .find(|i| i.data == InstData::ConstI64(2))
                    .unwrap()
                    .id
            )
        );
    }

    #[test]
    fn struct_fields_assignment_prepares_wide_context_and_consumes_linear_values() {
        let mut ctx = context();
        let expr = expression("s.wide = { if true { 3 } else { 4 } }");
        let result = field_expression(&mut ctx, &expr);
        assert_eq!(ctx.inst_ty(result), Ty::I128);
        let linear = ctx.emit(
            Opcode::ConstI64,
            Ty::LinearPtr,
            vec![],
            InstData::ConstI64(0),
            Span::new(0, 1),
        );
        ctx.define("token".into(), linear);
        let expr = expression("s.items = token");
        let result = field_expression(&mut ctx, &expr);
        assert_eq!(result, linear);
        let instructions = insts(&ctx);
        let consume = instructions[instructions.len() - 2];
        assert_eq!(consume.opcode, Opcode::LinearConsume);
        assert_eq!(consume.args, vec![linear]);
        assert_eq!(consume.origin, expr.span);
        assert_eq!(instructions.last().unwrap().opcode, Opcode::FieldSet);
    }

    #[test]
    fn struct_fields_invalid_access_preserves_evaluation_and_warning_fallbacks() {
        let mut ctx = context();
        let expr = expression("s.missing = { x = 9; 17 }");
        let result = field_expression(&mut ctx, &expr);
        assert_eq!(
            insts(&ctx).iter().find(|i| i.id == result).unwrap().data,
            InstData::ConstI64(17)
        );
        assert!(insts(&ctx).iter().all(|i| i.opcode != Opcode::FieldSet));
        assert_eq!(ctx.warnings.len(), 1);
        assert_eq!(
            ctx.warnings[0].message,
            "field 'missing' not found in struct 'S' -- ICE: returning sentinel index"
        );
        assert_eq!(ctx.warnings[0].primary.byte_offset, expr.span.start);
        let expr = expression("s.missing");
        let result = field_expression(&mut ctx, &expr);
        assert_eq!(
            insts(&ctx).iter().find(|i| i.id == result).unwrap().data,
            InstData::ConstI64(0)
        );
        assert_eq!(ctx.warnings.len(), 2);
    }

    #[test]
    fn struct_fields_read_keeps_independent_defaults_for_partial_declarations() {
        let mut ctx = context();
        ctx.struct_field_type_names
            .get_mut("S")
            .unwrap()
            .truncate(2);
        let expr = expression("s.items");
        let result = field_expression(&mut ctx, &expr);
        assert_eq!(ctx.inst_ty(result), Ty::I64);
        assert!(!ctx.inst_struct_type.contains_key(&result));
        assert!(ctx.inst_declared_ast_types.contains_key(&result));
        assert_eq!(ctx.inst_vec_elem_types[&result], vec!["Box".to_string()]);
        assert_eq!(insts(&ctx).last().unwrap().data, InstData::FieldIndex(4));
        ctx.struct_field_ast_types = Rc::new(HashMap::new());
        ctx.struct_field_vec_elems.clear();
        let result = field_expression(&mut ctx, &expr);
        assert!(!ctx.inst_declared_ast_types.contains_key(&result));
        assert!(!ctx.inst_vec_elem_types.contains_key(&result));
        assert!(ctx.warnings.is_empty());
    }

    #[test]
    fn struct_fields_untagged_and_unknown_receivers_keep_distinct_warnings() {
        let mut ctx = context();
        let ptr = ctx.lookup("s").unwrap();
        ctx.inst_struct_type.remove(&ptr);
        let expr = expression("s.first");
        let result = field_expression(&mut ctx, &expr);
        assert_eq!(insts(&ctx).last().unwrap().id, result);
        assert_eq!(ctx.warnings.len(), 1);
        assert_eq!(
            ctx.warnings[0].message,
            format!(
                "FieldGet on untagged instruction %{}, field 'first' -- ICE: returning sentinel index",
                ptr.0
            )
        );
        ctx.inst_struct_type.insert(ptr, "Ghost".into());
        let result = field_expression(&mut ctx, &expr);
        assert_eq!(insts(&ctx).last().unwrap().id, result);
        assert_eq!(ctx.warnings.len(), 2);
        assert_eq!(
            ctx.warnings[1].message,
            "struct 'Ghost' not registered -- field lookup ICE: returning sentinel index"
        );
    }

    #[test]
    fn struct_fields_constructor_owns_linear_allocation_and_consumes_even_missing_fields() {
        for field in ["items", "missing"] {
            let mut ctx = context();
            ctx.linear_owner_names.insert("S".into());
            let token = ctx.emit(
                Opcode::ConstI64,
                Ty::LinearPtr,
                vec![],
                InstData::ConstI64(0),
                Span::new(0, 1),
            );
            ctx.define("token".into(), token);
            let expr = expression(&format!("S {{ {field}: token }}"));
            let result = field_expression(&mut ctx, &expr);
            assert_eq!(ctx.inst_ty(result), Ty::LinearPtr);
            let instructions = insts(&ctx);
            let consumes: Vec<_> = instructions
                .iter()
                .filter(|i| i.opcode == Opcode::LinearConsume)
                .collect();
            assert_eq!(consumes.len(), 1);
            assert_eq!(consumes[0].args, vec![token]);
            let ExprKind::StructLiteral { fields, .. } = &expr.kind else {
                panic!("literal")
            };
            assert_eq!(consumes[0].origin, fields[0].1.span);
            assert_eq!(
                instructions
                    .iter()
                    .filter(|i| i.opcode == Opcode::FieldSet)
                    .count(),
                usize::from(field == "items")
            );
            assert_eq!(ctx.warnings.len(), usize::from(field == "missing"));
        }
    }

    #[test]
    fn struct_fields_empty_and_absent_declarations_keep_initializer_fallbacks() {
        for registered in [true, false] {
            let mut ctx = context();
            if registered {
                ctx.struct_field_map.insert("Empty".into(), vec![]);
            }
            let expr = expression("Empty { unexpected: { x = 9; 17 } }");
            let result = field_expression(&mut ctx, &expr);
            let instructions = insts(&ctx);
            assert_eq!(
                instructions.iter().find(|i| i.id == result).unwrap().data,
                InstData::AllocSize { size: 16, align: 8 }
            );
            assert!(instructions.iter().all(|i| i.opcode != Opcode::FieldSet));
            assert!(
                instructions
                    .iter()
                    .any(|i| i.id == ctx.lookup("x").unwrap() && i.data == InstData::ConstI64(9))
            );
            assert_eq!(ctx.warnings.len(), usize::from(!registered));
            if !registered {
                assert_eq!(
                    ctx.warnings[0].message,
                    "struct 'Empty' not registered -- field lookup ICE: returning sentinel index"
                );
            }
        }
    }
}
