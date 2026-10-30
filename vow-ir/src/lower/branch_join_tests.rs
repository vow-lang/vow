#[cfg(test)]
mod tests {
    use super::super::branch_join::{branch_join_capture, branch_join_finish, branch_join_new};
    use super::super::*;

    fn context() -> LowerCtx {
        let mut ctx = LowerCtx::new(
            "join".into(),
            vec![],
            vec![],
            Ty::I64,
            vec![],
            "join.vow".into(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashSet::new(),
            Rc::new(HashMap::new()),
            HashMap::new(),
            Rc::new(HashMap::new()),
            HashMap::new(),
            StringExprSet::new(),
            Rc::new(PatternAggregateMap::new()),
            Rc::new(PayloadScalarMap::new()),
        );
        ctx.push_scope();
        let initial = ctx.emit(
            Opcode::ConstI64,
            Ty::I64,
            vec![],
            InstData::ConstI64(71),
            Span::new(0, 1),
        );
        ctx.define("x".into(), initial);
        ctx
    }

    fn expression(text: &str) -> Expr {
        let source = format!("module Join\nfn f() -> i64 {{ {text} }}");
        let (module, diags) = vow_syntax::parser::parse_module(&source, "join.vow");
        assert!(diags.is_empty(), "{diags:?}");
        let Item::Fn(function) = &module.items[0] else {
            panic!("function")
        };
        *function
            .body
            .trailing_expr
            .clone()
            .expect("join expression")
    }

    // Drive the seam directly: dispatch and pattern matching are caller concerns.
    fn join_expression(ctx: &mut LowerCtx, expr: &Expr) -> InstId {
        match &expr.kind {
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let mutations = collect_if_mutations(ctx, then_branch, else_branch.as_deref());
                lower_expr(ctx, condition);
                let then_block = ctx.new_block();
                let else_block = ctx.new_block();
                let merge = ctx.new_block();
                let mut join = branch_join_new(ctx, expr, merge, mutations);
                ctx.switch_to_block(then_block);
                let result = lower_block(ctx, then_branch);
                branch_join_capture(ctx, &mut join, 0, result);
                ctx.switch_to_block(else_block);
                let result = if let Some(body) = else_branch {
                    lower_value_expr(ctx, body)
                } else {
                    ctx.emit(
                        Opcode::ConstUnit,
                        Ty::Unit,
                        vec![],
                        InstData::None,
                        expr.span,
                    )
                };
                branch_join_capture(ctx, &mut join, 1, result);
                branch_join_finish(ctx, join)
            }
            ExprKind::Match { arms, .. } => {
                let mut seen = HashSet::new();
                let mut names = Vec::new();
                for arm in arms {
                    collect_assigned_in_expr(&arm.body, &mut seen, &mut names);
                }
                let mutations = ctx.in_scope_vars(names);
                let merge = ctx.new_block();
                let mut join = branch_join_new(ctx, expr, merge, mutations);
                for (index, arm) in arms.iter().enumerate() {
                    let block = ctx.new_block();
                    ctx.switch_to_block(block);
                    let result = lower_value_expr(ctx, &arm.body);
                    branch_join_capture(ctx, &mut join, index, result);
                }
                branch_join_finish(ctx, join)
            }
            _ => panic!("expected a branching expression"),
        }
    }

    fn assert_exit_wiring(ctx: &LowerCtx, result: InstId, mutation: InstId, exits: usize) {
        let mut found = 0;
        for block in &ctx.func.blocks {
            let ups: Vec<_> = block
                .insts
                .iter()
                .filter(|i| i.opcode == Opcode::Upsilon)
                .collect();
            if ups.is_empty() {
                continue;
            }
            found += 1;
            assert_eq!(ups.len(), 2);
            assert_eq!(ups[0].data, InstData::PhiTarget(mutation));
            assert_eq!(ups[1].data, InstData::PhiTarget(result));
            assert_eq!(block.insts.last().unwrap().opcode, Opcode::Jump);
        }
        assert_eq!(found, exits);
    }

    #[test]
    fn if_join_restores_sibling_bindings_and_wires_mutations_before_result() {
        let mut ctx = context();
        let expr = expression("if true { x = 2; 11 } else { x = x + 3; 12 }");
        let result = join_expression(&mut ctx, &expr);
        let mutation = ctx.lookup("x").unwrap();
        assert_exit_wiring(&ctx, result, mutation, 2);
        let add = ctx
            .func
            .blocks
            .iter()
            .flat_map(|b| &b.insts)
            .find(|i| i.opcode == Opcode::WrappingAdd)
            .unwrap();
        assert_eq!(add.args[0], InstId(0), "else must read the pre-branch x");
    }

    #[test]
    fn match_join_restores_bindings_and_skips_terminated_arms() {
        let mut ctx = context();
        let expr = expression(
            "match Option::Some(0) { Option::Some(v) => { x = 2; return 11; }, _ => { x = x + 3; 12 }, }",
        );
        let result = join_expression(&mut ctx, &expr);
        assert_exit_wiring(&ctx, result, ctx.lookup("x").unwrap(), 1);
        let add = ctx
            .func
            .blocks
            .iter()
            .flat_map(|b| &b.insts)
            .find(|i| i.opcode == Opcode::WrappingAdd)
            .unwrap();
        assert_eq!(add.args[0], InstId(0));
    }

    #[test]
    fn all_terminated_if_keeps_mutation_phi_but_match_has_none() {
        for (source, phis) in [
            (
                "if true { x = 2; return 11; } else { x = 3; return 12; }",
                1,
            ),
            (
                "match Option::Some(0) { Option::Some(v) => { x = 2; return 11; }, _ => { x = 3; return 12; }, }",
                0,
            ),
        ] {
            let mut ctx = context();
            let expr = expression(source);
            let result = join_expression(&mut ctx, &expr);
            let merge = &ctx.func.blocks[ctx.current_block.0 as usize];
            assert_eq!(
                merge
                    .insts
                    .iter()
                    .filter(|i| i.opcode == Opcode::Phi)
                    .count(),
                phis
            );
            assert_eq!(merge.insts.last().unwrap().id, result);
            assert_eq!(merge.insts.last().unwrap().opcode, Opcode::Unreachable);
        }
    }

    #[test]
    fn branch_join_preserves_narrow_marker_result_width() {
        for source in [
            "if true { -1 } else { 3 as i8 }",
            "match Option::Some(0) { Option::Some(v) => -1, _ => 3 as i8, }",
        ] {
            let mut ctx = context();
            let expr = expression(source);
            let result = join_expression(&mut ctx, &expr);
            assert_eq!(ctx.inst_ty(result), Ty::I8);
            for inst in ctx.func.blocks.iter().flat_map(|b| &b.insts) {
                if inst.data == InstData::PhiTarget(result) {
                    assert_eq!(ctx.inst_ty(inst.args[0]), Ty::I8);
                }
            }
        }
    }

    #[test]
    fn branch_join_preserves_aggregate_result_metadata() {
        for source in [
            "if true { x = 2; \"then\" } else { x = 3; \"else\" }",
            "match Option::Some(0) { Option::Some(v) => { x = 2; \"then\" }, _ => { x = 3; \"else\" }, }",
        ] {
            let mut ctx = context();
            let expr = expression(source);
            let result = join_expression(&mut ctx, &expr);
            assert_eq!(
                ctx.inst_struct_type.get(&result).map(String::as_str),
                Some("String")
            );
            assert_exit_wiring(&ctx, result, ctx.lookup("x").unwrap(), 2);
        }
    }

    #[test]
    fn fallback_match_exit_uses_saved_mutations_and_restores_scope() {
        let mut ctx = context();
        let expr = expression("match Option::Some(0) { 1 => 11, _ => 12, }");
        let initial = ctx.lookup("x").unwrap();
        let merge = ctx.new_block();
        let mut join = branch_join_new(&ctx, &expr, merge, vec![("x".into(), initial)]);
        for index in 0..2 {
            let block = ctx.new_block();
            ctx.switch_to_block(block);
            let value = ctx.emit(
                Opcode::ConstI64,
                Ty::I64,
                vec![],
                InstData::ConstI64(100),
                expr.span,
            );
            ctx.assign("x", value);
            branch_join_capture(&mut ctx, &mut join, index, value);
            assert_eq!(ctx.lookup("x"), Some(initial));
        }
        let result = branch_join_finish(&mut ctx, join);
        let mutation = ctx.lookup("x").unwrap();
        assert_exit_wiring(&ctx, result, mutation, 2);
        let fallback = &ctx.func.blocks[2];
        let first_up = fallback
            .insts
            .iter()
            .find(|i| i.opcode == Opcode::Upsilon)
            .unwrap();
        assert_eq!(
            first_up.args,
            [initial],
            "fallback must use saved input rather than current binding"
        );
    }

    #[test]
    fn mutation_metadata_ignores_a_terminated_sibling() {
        for source in [
            "if true { x = 2; 11 } else { x = 3; 12 }",
            "match Option::Some(0) { Option::Some(v) => { x = 2; 11 }, _ => { x = 3; 12 }, }",
        ] {
            let mut ctx = context();
            let expr = expression(source);
            let initial = ctx.lookup("x").unwrap();
            let merge = ctx.new_block();
            let mut join = branch_join_new(&ctx, &expr, merge, vec![("x".into(), initial)]);
            for index in 0..2 {
                let block = ctx.new_block();
                ctx.switch_to_block(block);
                let value = ctx.emit(
                    Opcode::ConstUnit,
                    Ty::Ptr,
                    vec![],
                    InstData::None,
                    expr.span,
                );
                ctx.inst_struct_type
                    .insert(value, if index == 0 { "Vec" } else { "String" }.into());
                ctx.assign("x", value);
                if index == 0 {
                    ctx.emit(
                        Opcode::Return,
                        Ty::Unit,
                        vec![value],
                        InstData::None,
                        expr.span,
                    );
                }
                branch_join_capture(&mut ctx, &mut join, index, value);
            }
            branch_join_finish(&mut ctx, join);
            let mutation = ctx.lookup("x").unwrap();
            assert_eq!(
                ctx.inst_struct_type.get(&mutation).map(String::as_str),
                Some("String")
            );
        }
    }
}
