#[cfg(test)]
mod tests {
    use super::super::*;

    fn context() -> LowerCtx {
        LowerCtx::new(
            "syntax".into(),
            vec![],
            vec![],
            Ty::I64,
            vec![],
            "syntax.vow".into(),
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
        )
    }

    fn expression(text: &str) -> Expr {
        let source = format!("module Syntax\nfn f() {{ let result = {text}; }}");
        let (module, diagnostics) = vow_syntax::parser::parse_module(&source, "syntax.vow");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let Item::Fn(function) = &module.items[0] else {
            panic!("function")
        };
        let Stmt::Let { init, .. } = &function.body.stmts[0] else {
            panic!("binding")
        };
        *init.clone()
    }

    fn lower_loop(ctx: &mut LowerCtx, expr: &Expr) -> InstId {
        match &expr.kind {
            ExprKind::While {
                condition,
                body,
                vow,
            } => loop_control::lower_while_expr(ctx, condition, body, vow.as_ref(), expr.span),
            ExprKind::Loop { body, vow } => {
                loop_control::lower_loop_expr(ctx, body, vow.as_ref(), expr.span)
            }
            ExprKind::ForEach {
                binding,
                iterable,
                body,
                vow,
            } => loop_control::lower_foreach_expr(
                ctx,
                expr,
                binding,
                iterable,
                body,
                vow.as_ref(),
                expr.span,
            ),
            _ => panic!("loop expression"),
        }
    }

    fn define(ctx: &mut LowerCtx, name: &str, text: &str) -> InstId {
        let value = lower_expr(ctx, &expression(text));
        ctx.define(name.into(), value);
        value
    }

    fn instructions(ctx: &LowerCtx) -> impl Iterator<Item = &Inst> {
        ctx.func.blocks.iter().flat_map(|block| &block.insts)
    }

    fn assert_patched(ctx: &LowerCtx) {
        for up in instructions(ctx).filter(|i| i.opcode == Opcode::Upsilon) {
            let InstData::PhiTarget(target) = up.data else {
                panic!("target")
            };
            let phi = instructions(ctx)
                .find(|i| i.id == target)
                .expect("patched Phi");
            assert_eq!(phi.opcode, Opcode::Phi);
        }
    }

    #[test]
    fn while_exit_uses_the_final_condition_block_and_mutation() {
        let mut ctx = context();
        define(&mut ctx, "x", "4");
        define(&mut ctx, "flag", "true");
        let expr = expression("while flag && { x = x + 1; x < 7 } { x = x + 2; }");
        lower_loop(&mut ctx, &expr);

        let exit = ctx.current_block;
        let exit_phi = ctx.lookup("x").unwrap();
        let header_phi = ctx.func.blocks[1].insts[0].id;
        let condition_block = ctx
            .func
            .blocks
            .iter()
            .find(|block| {
                block.insts.last().is_some_and(|inst| {
                    matches!(inst.data,
                InstData::BranchTargets { else_block, .. } if else_block == exit)
                })
            })
            .expect("natural exit");
        assert_ne!(
            condition_block.id,
            BlockId(1),
            "short-circuit condition creates blocks"
        );
        let natural = condition_block
            .insts
            .iter()
            .find(|inst| {
                inst.opcode == Opcode::Upsilon && inst.data == InstData::PhiTarget(exit_phi)
            })
            .expect("natural exit input");
        assert_ne!(
            natural.args,
            [header_phi],
            "retain the final condition assignment"
        );
        assert_eq!(natural.origin, expr.span);
        assert_patched(&ctx);
    }

    #[test]
    fn foreach_index_precedes_typed_mutations_and_keeps_metadata() {
        let mut ctx = context();
        let initial = define(&mut ctx, "x", "8u64");
        define(&mut ctx, "text", "\"before\"");
        define(&mut ctx, "xs", "Vec::new()");
        let expr = expression("for item in xs { x = x + 1u64; text = \"after\"; }");
        lower_loop(&mut ctx, &expr);

        let inputs: Vec<_> = ctx.func.blocks[0]
            .insts
            .iter()
            .filter(|i| i.opcode == Opcode::Upsilon)
            .collect();
        assert_eq!(inputs.len(), 3);
        assert_eq!(inputs[0].ty, Ty::U64);
        assert_eq!(inputs[1].args, [initial]);
        let phis: Vec<_> = ctx.func.blocks[1]
            .insts
            .iter()
            .filter(|i| i.opcode == Opcode::Phi)
            .collect();
        assert_eq!(
            phis.iter().map(|i| i.ty).collect::<Vec<_>>(),
            [Ty::U64, Ty::U64, Ty::Ptr]
        );
        for (up, phi) in inputs.iter().zip(&phis) {
            assert_eq!(up.data, InstData::PhiTarget(phi.id));
            assert_eq!(phi.origin, expr.span);
        }
        assert_eq!(
            ctx.inst_struct_type.get(&phis[2].id).map(String::as_str),
            Some("String")
        );
        let text_exit = ctx.lookup("text").unwrap();
        assert_eq!(
            ctx.inst_struct_type.get(&text_exit).map(String::as_str),
            Some("String")
        );
        assert_eq!(ctx.lookup("item"), None);
        assert_patched(&ctx);
    }

    #[test]
    fn value_loop_patches_carried_and_break_result_phis() {
        let mut ctx = context();
        define(&mut ctx, "text", "\"before\"");
        let expr = expression("loop { text = \"after\"; break text; }");
        let result = lower_loop(&mut ctx, &expr);
        assert_eq!(ctx.func.blocks.len(), 3);
        assert_eq!(
            ctx.inst_struct_type.get(&result).map(String::as_str),
            Some("String")
        );
        assert_eq!(
            ctx.inst_struct_type
                .get(&ctx.lookup("text").unwrap())
                .map(String::as_str),
            Some("String")
        );
        assert!(!instructions(&ctx).any(|i| i.opcode == Opcode::Branch));
        assert_patched(&ctx);

        let mut empty = context();
        lower_loop(&mut empty, &expression("while false { break; }"));
        assert!(!instructions(&empty).any(|i| i.opcode == Opcode::Phi));
    }
}
