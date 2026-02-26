pub mod vow;

use std::collections::HashMap;

use vow_diag::Blame;
use vow_syntax::ast::{
    BinOp, Block, Effect, ExprKind, FnDef, Item, Lit, Module as AstModule, PatKind, Stmt,
    Type as AstType, UnOp,
};
use vow_syntax::span::Span;

use crate::types::{
    BasicBlock, BlockId, FuncId, Function, Inst, InstData, InstId, Module, Opcode, Ty, VowEntry,
    VowId,
};

fn lower_ty(ast_ty: &AstType) -> Ty {
    match ast_ty {
        AstType::Named { name, .. } => match name.as_str() {
            "i32" => Ty::I32,
            "i64" => Ty::I64,
            "f32" => Ty::F32,
            "f64" => Ty::F64,
            "bool" => Ty::Bool,
            _ => Ty::Ptr,
        },
        AstType::Unit { .. } => Ty::Unit,
        AstType::Never { .. } => Ty::Unit,
        _ => Ty::Ptr,
    }
}

pub struct LowerCtx {
    pub(super) func: Function,
    pub(super) current_block: BlockId,
    next_inst_id: u32,
    scope: Vec<HashMap<String, InstId>>,
}

impl LowerCtx {
    pub fn new(name: String, params: Vec<Ty>, return_ty: Ty, effects: Vec<Effect>) -> Self {
        let entry = BasicBlock {
            id: BlockId(0),
            insts: vec![],
        };
        let func = Function {
            id: FuncId(0),
            name,
            params,
            return_ty,
            effects,
            vows: vec![],
            blocks: vec![entry],
        };
        LowerCtx {
            func,
            current_block: BlockId(0),
            next_inst_id: 0,
            scope: vec![HashMap::new()],
        }
    }

    pub(super) fn push_scope(&mut self) {
        self.scope.push(HashMap::new());
    }

    pub(super) fn pop_scope(&mut self) {
        self.scope.pop();
    }

    pub(super) fn define(&mut self, name: String, id: InstId) {
        if let Some(top) = self.scope.last_mut() {
            top.insert(name, id);
        }
    }

    pub(super) fn lookup(&self, name: &str) -> Option<InstId> {
        for frame in self.scope.iter().rev() {
            if let Some(&id) = frame.get(name) {
                return Some(id);
            }
        }
        None
    }

    pub(super) fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.func.blocks.len() as u32);
        self.func.blocks.push(BasicBlock { id, insts: vec![] });
        id
    }

    pub(super) fn switch_to_block(&mut self, block: BlockId) {
        self.current_block = block;
    }

    pub(super) fn alloc_vow(&mut self, description: String, blame: Blame) -> VowId {
        let id = VowId(self.func.vows.len() as u32);
        self.func.vows.push(VowEntry {
            id,
            description,
            blame,
        });
        id
    }

    pub(super) fn emit(
        &mut self,
        opcode: Opcode,
        ty: Ty,
        args: Vec<InstId>,
        data: InstData,
        origin: Span,
    ) -> InstId {
        let id = InstId(self.next_inst_id);
        self.next_inst_id += 1;
        let inst = Inst {
            id,
            opcode,
            ty,
            args,
            data,
            origin,
        };
        let block_idx = self.current_block.0 as usize;
        self.func.blocks[block_idx].insts.push(inst);
        id
    }

    pub fn finish(self) -> Function {
        self.func
    }
}

pub(super) fn lower_expr_pub(ctx: &mut LowerCtx, expr: &vow_syntax::ast::Expr) -> InstId {
    lower_expr(ctx, expr)
}

fn lower_expr(ctx: &mut LowerCtx, expr: &vow_syntax::ast::Expr) -> InstId {
    let span = expr.span;
    match &expr.kind {
        ExprKind::Lit(lit) => match lit {
            Lit::Int(v) => ctx.emit(
                Opcode::ConstI64,
                Ty::I64,
                vec![],
                InstData::ConstI64(*v as i64),
                span,
            ),
            Lit::Float(v) => ctx.emit(
                Opcode::ConstF64,
                Ty::F64,
                vec![],
                InstData::ConstF64(*v),
                span,
            ),
            Lit::Bool(v) => ctx.emit(
                Opcode::ConstBool,
                Ty::Bool,
                vec![],
                InstData::ConstBool(*v),
                span,
            ),
            Lit::String(_) => ctx.emit(Opcode::ConstUnit, Ty::Ptr, vec![], InstData::None, span),
        },
        ExprKind::Ident(name) => ctx
            .lookup(name)
            .unwrap_or_else(|| panic!("undefined variable: {name}")),
        ExprKind::BinaryOp { op, lhs, rhs } => {
            let lhs_id = lower_expr(ctx, lhs);
            let rhs_id = lower_expr(ctx, rhs);
            let (opcode, ty) = binop_opcode(*op);
            ctx.emit(opcode, ty, vec![lhs_id, rhs_id], InstData::None, span)
        }
        ExprKind::UnaryOp { op, operand } => {
            let val = lower_expr(ctx, operand);
            match op {
                UnOp::Not => ctx.emit(Opcode::Not, Ty::Bool, vec![val], InstData::None, span),
                UnOp::Neg => ctx.emit(
                    Opcode::WrappingSubI64,
                    Ty::I64,
                    vec![val],
                    InstData::None,
                    span,
                ),
            }
        }
        ExprKind::Call { callee: _, args } => {
            let arg_ids: Vec<InstId> = args.iter().map(|a| lower_expr(ctx, a)).collect();
            ctx.emit(
                Opcode::Call,
                Ty::Unit,
                arg_ids,
                InstData::CallTarget(FuncId(0)),
                span,
            )
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let cond_id = lower_expr(ctx, condition);
            let then_block = ctx.new_block();
            let else_block = ctx.new_block();
            let merge_block = ctx.new_block();

            ctx.emit(
                Opcode::Branch,
                Ty::Unit,
                vec![cond_id],
                InstData::BranchTargets {
                    then_block,
                    else_block,
                },
                span,
            );

            ctx.switch_to_block(then_block);
            let then_val = lower_block(ctx, then_branch);
            let then_upsilon_id = ctx.emit(
                Opcode::Upsilon,
                Ty::Unit,
                vec![then_val],
                InstData::PhiTarget(InstId(u32::MAX)),
                span,
            );
            ctx.emit(
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(merge_block),
                span,
            );

            ctx.switch_to_block(else_block);
            let else_val = if let Some(else_expr) = else_branch {
                lower_expr(ctx, else_expr)
            } else {
                ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
            };
            let else_upsilon_id = ctx.emit(
                Opcode::Upsilon,
                Ty::Unit,
                vec![else_val],
                InstData::PhiTarget(InstId(u32::MAX)),
                span,
            );
            ctx.emit(
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(merge_block),
                span,
            );

            ctx.switch_to_block(merge_block);
            let phi_id = ctx.emit(Opcode::Phi, Ty::I64, vec![], InstData::None, span);

            backpatch_upsilon(ctx, then_block, then_upsilon_id, phi_id);
            backpatch_upsilon(ctx, else_block, else_upsilon_id, phi_id);

            phi_id
        }
        ExprKind::Block(block) => {
            ctx.push_scope();
            let result = lower_block_inner(ctx, block);
            ctx.pop_scope();
            result
        }
        ExprKind::Return { value } => {
            if let Some(val_expr) = value {
                let val = lower_expr(ctx, val_expr);
                ctx.emit(Opcode::Return, Ty::Unit, vec![val], InstData::None, span)
            } else {
                let unit = ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span);
                ctx.emit(Opcode::Return, Ty::Unit, vec![unit], InstData::None, span)
            }
        }
        _ => ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span),
    }
}

fn binop_opcode(op: BinOp) -> (Opcode, Ty) {
    match op {
        BinOp::Add => (Opcode::WrappingAddI64, Ty::I64),
        BinOp::Sub => (Opcode::WrappingSubI64, Ty::I64),
        BinOp::Mul => (Opcode::WrappingMulI64, Ty::I64),
        BinOp::Div => (Opcode::WrappingDivI64, Ty::I64),
        BinOp::Rem => (Opcode::WrappingRemI64, Ty::I64),
        BinOp::AddChecked => (Opcode::CheckedAddI64, Ty::I64),
        BinOp::SubChecked => (Opcode::CheckedSubI64, Ty::I64),
        BinOp::MulChecked => (Opcode::CheckedMulI64, Ty::I64),
        BinOp::DivChecked => (Opcode::CheckedDivI64, Ty::I64),
        BinOp::RemChecked => (Opcode::CheckedRemI64, Ty::I64),
        BinOp::Eq => (Opcode::EqI64, Ty::Bool),
        BinOp::Ne => (Opcode::NeI64, Ty::Bool),
        BinOp::Lt => (Opcode::LtI64, Ty::Bool),
        BinOp::Le => (Opcode::LeI64, Ty::Bool),
        BinOp::Gt => (Opcode::GtI64, Ty::Bool),
        BinOp::Ge => (Opcode::GeI64, Ty::Bool),
        BinOp::And => (Opcode::And, Ty::Bool),
        BinOp::Or => (Opcode::Or, Ty::Bool),
    }
}

fn backpatch_upsilon(ctx: &mut LowerCtx, block_id: BlockId, upsilon_id: InstId, phi_id: InstId) {
    let block_idx = block_id.0 as usize;
    for inst in ctx.func.blocks[block_idx].insts.iter_mut() {
        if inst.id == upsilon_id {
            inst.data = InstData::PhiTarget(phi_id);
            break;
        }
    }
}

fn lower_stmt(ctx: &mut LowerCtx, stmt: &Stmt) {
    match stmt {
        Stmt::Let { pattern, init, .. } => {
            let val = lower_expr(ctx, init);
            if let PatKind::Ident { name, .. } = &pattern.kind {
                ctx.define(name.clone(), val);
            }
        }
        Stmt::Expr { expr, .. } => {
            lower_expr(ctx, expr);
        }
    }
}

fn lower_block(ctx: &mut LowerCtx, block: &Block) -> InstId {
    ctx.push_scope();
    let result = lower_block_inner(ctx, block);
    ctx.pop_scope();
    result
}

fn lower_block_inner(ctx: &mut LowerCtx, block: &Block) -> InstId {
    for stmt in &block.stmts {
        lower_stmt(ctx, stmt);
    }
    if let Some(expr) = &block.trailing_expr {
        lower_expr(ctx, expr)
    } else {
        ctx.emit(
            Opcode::ConstUnit,
            Ty::Unit,
            vec![],
            InstData::None,
            block.span,
        )
    }
}

pub fn lower_function(fn_def: &FnDef) -> Function {
    let params: Vec<Ty> = fn_def.params.iter().map(|p| lower_ty(&p.ty)).collect();
    let return_ty = lower_ty(&fn_def.return_ty);
    let effects = fn_def.effects.clone();

    let mut ctx = LowerCtx::new(fn_def.name.clone(), params.clone(), return_ty, effects);

    for (idx, param) in fn_def.params.iter().enumerate() {
        let ty = params[idx];
        let arg_id = ctx.emit(
            Opcode::GetArg,
            ty,
            vec![],
            InstData::ArgIndex(idx as u32),
            fn_def.span,
        );
        ctx.define(param.name.clone(), arg_id);
    }

    if let Some(vow_block) = &fn_def.vow {
        vow::lower_requires(&mut ctx, vow_block);
    }

    ctx.push_scope();
    let trailing = lower_block_inner(&mut ctx, &fn_def.body);
    ctx.pop_scope();

    let has_return = {
        let block_idx = ctx.current_block.0 as usize;
        ctx.func.blocks[block_idx]
            .insts
            .last()
            .is_some_and(|i| i.opcode.is_terminal())
    };

    if !has_return {
        let span = fn_def.body.span;
        if let Some(vow_block) = &fn_def.vow {
            vow::lower_ensures(&mut ctx, vow_block, trailing);
        }
        ctx.emit(
            Opcode::Return,
            Ty::Unit,
            vec![trailing],
            InstData::None,
            span,
        );
    }

    ctx.finish()
}

pub fn lower_module(module: &AstModule) -> Module {
    let functions = module
        .items
        .iter()
        .filter_map(|item| {
            if let Item::Fn(fn_def) = item {
                Some(lower_function(fn_def))
            } else {
                None
            }
        })
        .enumerate()
        .map(|(idx, mut func)| {
            func.id = FuncId(idx as u32);
            func
        })
        .collect();

    Module {
        name: module.name.clone(),
        functions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vow_syntax::ast::{
        Block, Effect, Expr, ExprKind, FnDef, Lit, Pat, PatKind, Stmt, Type, Visibility,
    };
    use vow_syntax::span::Span;

    fn sp() -> Span {
        Span::new(0, 1)
    }

    fn unit_ty() -> Type {
        Type::Unit { span: sp() }
    }

    fn i64_ty() -> Type {
        Type::Named {
            name: "i64".to_string(),
            span: sp(),
        }
    }

    fn int_expr(v: i128) -> Expr {
        Expr {
            kind: ExprKind::Lit(Lit::Int(v)),
            span: sp(),
        }
    }

    fn bool_expr(v: bool) -> Expr {
        Expr {
            kind: ExprKind::Lit(Lit::Bool(v)),
            span: sp(),
        }
    }

    fn ident_expr(name: &str) -> Expr {
        Expr {
            kind: ExprKind::Ident(name.to_string()),
            span: sp(),
        }
    }

    fn empty_block() -> Block {
        Block {
            stmts: vec![],
            trailing_expr: None,
            span: sp(),
        }
    }

    fn make_fn(
        name: &str,
        params: Vec<vow_syntax::ast::Param>,
        return_ty: Type,
        body: Block,
        effects: Vec<Effect>,
    ) -> FnDef {
        FnDef {
            vis: Visibility::Public,
            name: name.to_string(),
            generics: vec![],
            params,
            return_ty,
            effects,
            vow: None,
            body,
            span: sp(),
        }
    }

    fn make_param(name: &str, ty: Type) -> vow_syntax::ast::Param {
        vow_syntax::ast::Param {
            name: name.to_string(),
            ty,
            refinement: None,
            span: sp(),
        }
    }

    #[test]
    fn lower_const_i64() {
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(int_expr(42))),
            span: sp(),
        };
        let fn_def = make_fn("const_fn", vec![], i64_ty(), body, vec![]);
        let func = lower_function(&fn_def);

        assert_eq!(func.name, "const_fn");
        assert_eq!(func.return_ty, Ty::I64);

        let entry = &func.blocks[0];
        let const_inst = entry.insts.iter().find(|i| i.opcode == Opcode::ConstI64);
        assert!(const_inst.is_some());
        assert_eq!(const_inst.unwrap().data, InstData::ConstI64(42));

        let ret = entry.insts.iter().find(|i| i.opcode == Opcode::Return);
        assert!(ret.is_some());
    }

    #[test]
    fn lower_addition() {
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(Expr {
                kind: ExprKind::BinaryOp {
                    op: BinOp::Add,
                    lhs: Box::new(ident_expr("a")),
                    rhs: Box::new(ident_expr("b")),
                },
                span: sp(),
            })),
            span: sp(),
        };
        let fn_def = make_fn(
            "add",
            vec![make_param("a", i64_ty()), make_param("b", i64_ty())],
            i64_ty(),
            body,
            vec![],
        );
        let func = lower_function(&fn_def);

        let entry = &func.blocks[0];
        let get_args: Vec<_> = entry
            .insts
            .iter()
            .filter(|i| i.opcode == Opcode::GetArg)
            .collect();
        assert_eq!(get_args.len(), 2);

        let add = entry
            .insts
            .iter()
            .find(|i| i.opcode == Opcode::WrappingAddI64);
        assert!(add.is_some());
        assert_eq!(add.unwrap().args.len(), 2);
    }

    #[test]
    fn lower_let_binding() {
        let let_stmt = Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: "x".to_string(),
                    is_mut: false,
                },
                span: sp(),
            },
            ty: None,
            init: Box::new(int_expr(42)),
            span: sp(),
        };
        let body = Block {
            stmts: vec![let_stmt],
            trailing_expr: Some(Box::new(ident_expr("x"))),
            span: sp(),
        };
        let fn_def = make_fn("let_fn", vec![], i64_ty(), body, vec![]);
        let func = lower_function(&fn_def);

        let entry = &func.blocks[0];
        let const_inst = entry.insts.iter().find(|i| i.opcode == Opcode::ConstI64);
        assert!(const_inst.is_some(), "expected ConstI64 for let binding");
        assert_eq!(const_inst.unwrap().data, InstData::ConstI64(42));

        let ret = entry.insts.iter().find(|i| i.opcode == Opcode::Return);
        assert!(ret.is_some());
        let const_id = const_inst.unwrap().id;
        assert_eq!(ret.unwrap().args, vec![const_id]);
    }

    #[test]
    fn lower_if_else() {
        let if_expr = Expr {
            kind: ExprKind::If {
                condition: Box::new(bool_expr(true)),
                then_branch: Box::new(Block {
                    stmts: vec![],
                    trailing_expr: Some(Box::new(int_expr(1))),
                    span: sp(),
                }),
                else_branch: Some(Box::new(int_expr(2))),
            },
            span: sp(),
        };
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(if_expr)),
            span: sp(),
        };
        let fn_def = make_fn("if_fn", vec![], i64_ty(), body, vec![]);
        let func = lower_function(&fn_def);

        assert!(
            func.blocks.len() >= 4,
            "expected entry + then + else + merge"
        );

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();

        let branch = all_insts.iter().find(|i| i.opcode == Opcode::Branch);
        assert!(branch.is_some(), "expected Branch instruction");

        let phi = all_insts.iter().find(|i| i.opcode == Opcode::Phi);
        assert!(phi.is_some(), "expected Phi instruction");

        let upsilons: Vec<_> = all_insts
            .iter()
            .filter(|i| i.opcode == Opcode::Upsilon)
            .collect();
        assert_eq!(upsilons.len(), 2, "expected 2 Upsilon instructions");

        let phi_id = phi.unwrap().id;
        for up in &upsilons {
            assert_eq!(
                up.data,
                InstData::PhiTarget(phi_id),
                "Upsilon should target the Phi"
            );
        }
    }

    #[test]
    fn lower_empty_function() {
        let fn_def = make_fn("empty_fn", vec![], unit_ty(), empty_block(), vec![]);
        let func = lower_function(&fn_def);

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();
        let ret = all_insts.iter().find(|i| i.opcode == Opcode::Return);
        assert!(ret.is_some(), "expected Return instruction");
        assert_eq!(func.return_ty, Ty::Unit);
    }
}
