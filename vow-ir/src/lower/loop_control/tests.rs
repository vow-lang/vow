use super::loop_control::{LoopEdges, lower_foreach_body, lower_loop_body, lower_while_body};
use super::*;

fn span() -> Span {
    Span::new(0, 1)
}

fn context() -> LowerCtx {
    LowerCtx::new(
        "control".into(),
        vec![],
        vec![],
        Ty::I64,
        vec![],
        "control.vow".into(),
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

fn body(text: &str) -> Block {
    let source = format!("module Control\nfn body() -> i64 {{ {text} }}");
    let (module, diagnostics) = vow_syntax::parser::parse_module(&source, "control.vow");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let Item::Fn(function) = &module.items[0] else {
        panic!("expected a function")
    };
    function.body.clone()
}

fn edges(ctx: &mut LowerCtx, carried_name: Option<&str>) -> LoopEdges {
    let header = ctx.new_block();
    let exit = ctx.new_block();
    let mut carried_phis = Vec::new();
    let mut exit_phis = Vec::new();
    if let Some(name) = carried_name {
        ctx.switch_to_block(header);
        let phi = ctx.emit(Opcode::Phi, Ty::I64, vec![], InstData::None, span());
        ctx.define(name.into(), phi);
        carried_phis.push((name.into(), phi));
        ctx.switch_to_block(exit);
        let exit_phi = ctx.emit(Opcode::Phi, Ty::I64, vec![], InstData::None, span());
        exit_phis.push((name.into(), exit_phi));
    }
    let entry = ctx.new_block();
    ctx.switch_to_block(entry);
    LoopEdges {
        header,
        exit,
        carried_phis,
        exit_phis,
    }
}

fn insts(ctx: &LowerCtx) -> impl Iterator<Item = &Inst> {
    ctx.func.blocks.iter().flat_map(|block| &block.insts)
}

#[test]
fn nested_body_restores_outer_continue_and_lexical_lookup() {
    let mut ctx = context();
    let edges = edges(&mut ctx, Some("x"));
    let outer_header = edges.header;
    let carried_phi = edges.carried_phis[0].1;
    let exit_phi = edges.exit_phis[0].1;
    let scope_depth = ctx.scope.len();
    let parsed = body("while false { continue; } { let x: i64 = 99; continue; }");

    lower_while_body(&mut ctx, edges, &parsed, span());

    assert_eq!(ctx.scope.len(), scope_depth);
    assert_eq!(ctx.lookup("x"), Some(exit_phi));
    let outer_continue = ctx
        .func
        .blocks
        .iter()
        .find(|block| {
            block.insts.last().is_some_and(|inst| {
                inst.opcode == Opcode::Jump && inst.data == InstData::JumpTarget(outer_header)
            })
        })
        .expect("outer continue must survive nested-loop exit");
    let update = &outer_continue.insts[outer_continue.insts.len() - 2];
    assert_eq!(update.opcode, Opcode::Upsilon);
    assert_eq!(update.data, InstData::PhiTarget(carried_phi));
    assert_eq!(update.args, [carried_phi], "skip the inner x shadow");
}

#[test]
fn foreach_continue_writes_carried_values_before_unsigned_index() {
    let mut ctx = context();
    let edges = edges(&mut ctx, Some("x"));
    let header = edges.header;
    let carried_phi = edges.carried_phis[0].1;
    let block = ctx.current_block;
    let index = ctx.emit(Opcode::Phi, Ty::U64, vec![], InstData::None, span());
    let element = ctx.emit(
        Opcode::ConstI64,
        Ty::I64,
        vec![],
        InstData::ConstI64(8),
        span(),
    );
    let parsed = body("continue;");

    lower_foreach_body(&mut ctx, edges, "x", element, index, &parsed, span());

    let instructions = &ctx.func.blocks[block.0 as usize].insts;
    let tail = &instructions[instructions.len() - 5..];
    assert_eq!(
        tail.iter().map(|i| i.opcode).collect::<Vec<_>>(),
        [
            Opcode::Upsilon,
            Opcode::ConstU64,
            Opcode::WrappingAdd,
            Opcode::Upsilon,
            Opcode::Jump,
        ]
    );
    assert_eq!(tail[0].args, [carried_phi]);
    assert_eq!(tail[0].data, InstData::PhiTarget(carried_phi));
    assert_eq!(tail[1].data, InstData::ConstU64(1));
    assert_eq!(tail[2].args, [index, tail[1].id]);
    assert_eq!(tail[2].data, InstData::Integer(IntegerType::U64));
    assert_eq!(tail[3].data, InstData::PhiTarget(index));
    assert!(tail[1..4].iter().all(|inst| inst.ty == Ty::U64));
    assert_eq!(tail[4].data, InstData::JumpTarget(header));
}

#[test]
fn foreach_fallthrough_writes_unsigned_index_before_carried_values() {
    let mut ctx = context();
    let edges = edges(&mut ctx, Some("x"));
    let block = ctx.current_block;
    let carried_phi = edges.carried_phis[0].1;
    let exit_phi = edges.exit_phis[0].1;
    let scope_depth = ctx.scope.len();
    let index = ctx.emit(Opcode::Phi, Ty::U64, vec![], InstData::None, span());
    let element = ctx.emit(
        Opcode::ConstI64,
        Ty::I64,
        vec![],
        InstData::ConstI64(8),
        span(),
    );
    let parsed = body("x = 11;");

    lower_foreach_body(&mut ctx, edges, "item", element, index, &parsed, span());

    assert_eq!(ctx.scope.len(), scope_depth);
    assert_eq!(ctx.lookup("item"), None);
    assert_eq!(ctx.lookup("x"), Some(exit_phi));
    let instructions = &ctx.func.blocks[block.0 as usize].insts;
    let tail = &instructions[instructions.len() - 5..];
    assert_eq!(
        tail.iter().map(|i| i.opcode).collect::<Vec<_>>(),
        [
            Opcode::ConstU64,
            Opcode::WrappingAdd,
            Opcode::Upsilon,
            Opcode::Upsilon,
            Opcode::Jump,
        ]
    );
    assert_eq!(tail[2].data, InstData::PhiTarget(index));
    assert_eq!(tail[3].data, InstData::PhiTarget(carried_phi));
}

#[test]
fn value_loop_collects_nearest_breaks_and_preserves_result_metadata() {
    let mut ctx = context();
    let edges = edges(&mut ctx, None);
    let parsed = body("let inner: String = loop { break \"inner\"; }; break \"outer\";");

    let result = lower_loop_body(&mut ctx, edges, &parsed, span());

    assert_eq!(
        ctx.inst_struct_type.get(&result).map(String::as_str),
        Some("String")
    );
    let inputs: Vec<_> = insts(&ctx)
        .filter(|inst| inst.opcode == Opcode::Upsilon && inst.data == InstData::PhiTarget(result))
        .collect();
    assert_eq!(
        inputs.len(),
        1,
        "inner breaks must not reach the outer result"
    );
    let descriptor = insts(&ctx)
        .find(|inst| inst.id == inputs[0].args[0])
        .unwrap();
    let literal = insts(&ctx)
        .find(|inst| inst.id == descriptor.args[0])
        .unwrap();
    let InstData::ConstStr(pool_index) = literal.data else {
        panic!("expected literal")
    };
    assert_eq!(ctx.string_pool[pool_index as usize], "outer");
    assert_eq!(
        insts(&ctx)
            .filter(|inst| inst.opcode == Opcode::Phi && inst.ty == Ty::Ptr)
            .count(),
        2
    );
}

#[test]
fn terminated_body_has_no_synthetic_backedge() {
    let mut ctx = context();
    let edges = edges(&mut ctx, Some("x"));
    let header = edges.header;
    let block = ctx.current_block;
    let parsed = body("return 7;");

    lower_while_body(&mut ctx, edges, &parsed, span());

    let instructions = &ctx.func.blocks[block.0 as usize].insts;
    assert_eq!(instructions.last().unwrap().opcode, Opcode::Return);
    assert!(!insts(&ctx).any(|inst| inst.data == InstData::JumpTarget(header)));
}
