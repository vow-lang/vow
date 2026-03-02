pub mod vow;

use std::collections::{HashMap, HashSet};

use vow_diag::Blame;
use vow_syntax::ast::{
    BinOp, Block, Effect, Expr, ExprKind, FnDef, Item, Lit, Module as AstModule, PatKind, Stmt,
    Type as AstType, UnOp, VariantKind, VowBlock,
};
use vow_syntax::span::Span;

use crate::types::{
    BasicBlock, BlockId, EnumLayout, FieldLayout, FuncId, Function, Inst, InstData, InstId,
    Module, Opcode, StructLayout, Ty, VariantLayout, VowEntry, VowId,
};

fn vow_builtin_to_runtime(name: &str) -> Option<(&'static str, Ty)> {
    match name {
        "print_str" => Some(("__vow_string_print", Ty::Unit)),
        "print_i64" => Some(("__vow_print_i64", Ty::Unit)),
        "eprintln_str" => Some(("__vow_eprintln_str", Ty::Unit)),
        "fs_read" => Some(("__vow_fs_read", Ty::Ptr)),
        "fs_write" => Some(("__vow_fs_write", Ty::I64)),
        "args" => Some(("__vow_args", Ty::Ptr)),
        "process_exit" => Some(("__vow_process_exit", Ty::Unit)),
        _ => None,
    }
}

pub(crate) fn lower_ty(ast_ty: &AstType) -> Ty {
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
    pub(super) vow_block: Option<VowBlock>,
    pub(super) string_pool: Vec<String>,
    func_index: HashMap<String, (FuncId, Ty)>,
    // struct name → field names in declaration order
    pub(super) struct_field_map: HashMap<String, Vec<String>>,
    // enum name → variant names in declaration order (index = tag)
    pub(super) enum_variant_map: HashMap<String, Vec<String>>,
    // InstId of a struct/enum allocation → type name
    pub(super) inst_struct_type: HashMap<InstId, String>,
    // struct name → field type names (from AST declarations) for FieldGet auto-tagging
    struct_field_type_names: HashMap<String, Vec<String>>,
}

impl LowerCtx {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: String,
        params: Vec<Ty>,
        return_ty: Ty,
        effects: Vec<Effect>,
        func_index: HashMap<String, (FuncId, Ty)>,
        struct_field_map: HashMap<String, Vec<String>>,
        enum_variant_map: HashMap<String, Vec<String>>,
        struct_field_type_names: HashMap<String, Vec<String>>,
    ) -> Self {
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
        let mut enum_variant_map = enum_variant_map;
        enum_variant_map
            .entry("Option".to_string())
            .or_insert_with(|| vec!["None".to_string(), "Some".to_string()]);
        enum_variant_map
            .entry("Result".to_string())
            .or_insert_with(|| vec!["Ok".to_string(), "Err".to_string()]);
        LowerCtx {
            func,
            current_block: BlockId(0),
            next_inst_id: 0,
            scope: vec![HashMap::new()],
            vow_block: None,
            string_pool: Vec::new(),
            func_index,
            struct_field_map,
            enum_variant_map,
            inst_struct_type: HashMap::new(),
            struct_field_type_names,
        }
    }

    pub(super) fn intern_str(&mut self, s: &str) -> u32 {
        if let Some(idx) = self.string_pool.iter().position(|x| x == s) {
            return idx as u32;
        }
        let idx = self.string_pool.len() as u32;
        self.string_pool.push(s.to_string());
        idx
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

    /// Update an existing binding in the outermost scope frame that contains it.
    /// If not found, creates a new binding in the current frame.
    pub(super) fn assign(&mut self, name: &str, id: InstId) {
        for frame in self.scope.iter_mut().rev() {
            if frame.contains_key(name) {
                frame.insert(name.to_string(), id);
                return;
            }
        }
        self.define(name.to_string(), id);
    }

    /// Look up the type of an already-emitted instruction.
    pub(super) fn inst_ty(&self, id: InstId) -> Ty {
        for block in &self.func.blocks {
            for inst in &block.insts {
                if inst.id == id {
                    return inst.ty;
                }
            }
        }
        Ty::Unit
    }

    pub(super) fn lookup(&self, name: &str) -> Option<InstId> {
        for frame in self.scope.iter().rev() {
            if let Some(&id) = frame.get(name) {
                return Some(id);
            }
        }
        None
    }

    /// Snapshot the current scope (all variable bindings) for save/restore.
    pub(super) fn snapshot_scope(&self) -> Vec<HashMap<String, InstId>> {
        self.scope.clone()
    }

    /// Restore scope to a previously saved snapshot.
    pub(super) fn restore_scope(&mut self, snap: Vec<HashMap<String, InstId>>) {
        self.scope = snap;
    }

    pub(super) fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.func.blocks.len() as u32);
        self.func.blocks.push(BasicBlock { id, insts: vec![] });
        id
    }

    pub(super) fn switch_to_block(&mut self, block: BlockId) {
        self.current_block = block;
    }

    pub(super) fn alloc_vow(
        &mut self,
        description: String,
        blame: Blame,
        bindings: Vec<(String, InstId)>,
    ) -> VowId {
        let id = VowId(self.func.vows.len() as u32);
        self.func.vows.push(VowEntry {
            id,
            description,
            blame,
            bindings,
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

    pub(super) fn is_terminated(&self) -> bool {
        let block_idx = self.current_block.0 as usize;
        self.func.blocks[block_idx]
            .insts
            .last()
            .map(|i| {
                matches!(
                    i.opcode,
                    Opcode::Return | Opcode::Jump | Opcode::Branch | Opcode::Unreachable
                )
            })
            .unwrap_or(false)
    }

    pub fn finish(self) -> (Function, Vec<String>) {
        (self.func, self.string_pool)
    }
}

/// Collect names of variables assigned anywhere in a block (recursively).
/// Used to identify loop-carried variables that need Phi nodes.
fn collect_assigned_vars(block: &Block) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = vec![];
    for stmt in &block.stmts {
        collect_assigned_in_stmt(stmt, &mut seen, &mut result);
    }
    if let Some(e) = &block.trailing_expr {
        collect_assigned_in_expr(e, &mut seen, &mut result);
    }
    result
}

fn collect_assigned_in_stmt(stmt: &Stmt, seen: &mut HashSet<String>, out: &mut Vec<String>) {
    if let Stmt::Expr { expr, .. } = stmt {
        collect_assigned_in_expr(expr, seen, out);
    }
}

fn collect_assigned_in_expr(expr: &Expr, seen: &mut HashSet<String>, out: &mut Vec<String>) {
    match &expr.kind {
        ExprKind::Assign { lhs, rhs } => {
            if let ExprKind::Ident(name) = &lhs.kind
                && seen.insert(name.clone())
            {
                out.push(name.clone());
            }
            collect_assigned_in_expr(rhs, seen, out);
        }
        ExprKind::Block(b) => {
            for s in &b.stmts {
                collect_assigned_in_stmt(s, seen, out);
            }
            if let Some(e) = &b.trailing_expr {
                collect_assigned_in_expr(e, seen, out);
            }
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_assigned_in_expr(condition, seen, out);
            for s in &then_branch.stmts {
                collect_assigned_in_stmt(s, seen, out);
            }
            if let Some(e) = &then_branch.trailing_expr {
                collect_assigned_in_expr(e, seen, out);
            }
            if let Some(e) = else_branch {
                collect_assigned_in_expr(e, seen, out);
            }
        }
        ExprKind::While {
            condition, body, ..
        } => {
            collect_assigned_in_expr(condition, seen, out);
            for s in &body.stmts {
                collect_assigned_in_stmt(s, seen, out);
            }
            if let Some(e) = &body.trailing_expr {
                collect_assigned_in_expr(e, seen, out);
            }
        }
        ExprKind::BinaryOp { lhs, rhs, .. } => {
            collect_assigned_in_expr(lhs, seen, out);
            collect_assigned_in_expr(rhs, seen, out);
        }
        ExprKind::UnaryOp { operand, .. } => collect_assigned_in_expr(operand, seen, out),
        ExprKind::Return { value: Some(v), .. } => {
            collect_assigned_in_expr(v, seen, out);
        }
        ExprKind::Return { value: None, .. } => {}
        _ => {}
    }
}

/// Return variables that are assigned in `then_branch` or `else_branch` AND
/// currently exist in scope (so they're live across the branch).
fn collect_if_mutations(
    ctx: &LowerCtx,
    then_branch: &Block,
    else_branch: Option<&Expr>,
) -> Vec<(String, InstId)> {
    let mut seen = HashSet::new();
    let mut names = vec![];
    for s in &then_branch.stmts {
        collect_assigned_in_stmt(s, &mut seen, &mut names);
    }
    if let Some(e) = &then_branch.trailing_expr {
        collect_assigned_in_expr(e, &mut seen, &mut names);
    }
    if let Some(e) = else_branch {
        collect_assigned_in_expr(e, &mut seen, &mut names);
    }
    names
        .into_iter()
        .filter_map(|name| ctx.lookup(&name).map(|id| (name, id)))
        .collect()
}

pub(super) fn lower_expr_pub(ctx: &mut LowerCtx, expr: &vow_syntax::ast::Expr) -> InstId {
    lower_expr(ctx, expr)
}

fn lower_expr(ctx: &mut LowerCtx, expr: &vow_syntax::ast::Expr) -> InstId {
    let span = expr.span;
    match &expr.kind {
        ExprKind::Lit(lit) => match lit {
            Lit::Int(v) => {
                ctx.emit(
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(*v as i64),
                    span,
                )
            }
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
            Lit::String(s) => {
                let idx = ctx.intern_str(s);
                let cstr = ctx.emit(
                    Opcode::ConstStr,
                    Ty::Ptr,
                    vec![],
                    InstData::ConstStr(idx),
                    span,
                );
                let vow_str = ctx.emit(
                    Opcode::Call,
                    Ty::Ptr,
                    vec![cstr],
                    InstData::CallExtern("__vow_string_from_cstr".to_string()),
                    span,
                );
                ctx.inst_struct_type.insert(vow_str, "String".to_string());
                vow_str
            }
        },
        ExprKind::Ident(name) => ctx
            .lookup(name)
            .unwrap_or_else(|| panic!("undefined variable: {name}")),
        ExprKind::BinaryOp { op, lhs, rhs } => {
            let lhs_id = lower_expr(ctx, lhs);
            let rhs_id = lower_expr(ctx, rhs);
            let lhs_is_str = ctx
                .inst_struct_type
                .get(&lhs_id)
                .map(|s| s == "String")
                .unwrap_or(false);
            let rhs_is_str = ctx
                .inst_struct_type
                .get(&rhs_id)
                .map(|s| s == "String")
                .unwrap_or(false);
            if (lhs_is_str || rhs_is_str) && (*op == BinOp::Eq || *op == BinOp::Ne) {
                let eq_result = ctx.emit(
                    Opcode::Call,
                    Ty::Bool,
                    vec![lhs_id, rhs_id],
                    InstData::CallExtern("__vow_string_eq".to_string()),
                    span,
                );
                if *op == BinOp::Ne {
                    ctx.emit(Opcode::Not, Ty::Bool, vec![eq_result], InstData::None, span)
                } else {
                    eq_result
                }
            } else {
                let (opcode, ty) = binop_opcode(*op);
                ctx.emit(opcode, ty, vec![lhs_id, rhs_id], InstData::None, span)
            }
        }
        ExprKind::UnaryOp { op, operand } => {
            let val = lower_expr(ctx, operand);
            match op {
                UnOp::Not => ctx.emit(Opcode::Not, Ty::Bool, vec![val], InstData::None, span),
                UnOp::Neg => {
                    let zero = ctx.emit(
                        Opcode::ConstI64,
                        Ty::I64,
                        vec![],
                        InstData::ConstI64(0),
                        span,
                    );
                    ctx.emit(
                        Opcode::WrappingSubI64,
                        Ty::I64,
                        vec![zero, val],
                        InstData::None,
                        span,
                    )
                }
            }
        }
        ExprKind::Call { callee, args } => {
            let arg_ids: Vec<InstId> = args.iter().map(|a| lower_expr(ctx, a)).collect();
            let callee_name = match &callee.kind {
                ExprKind::Ident(name) => name.clone(),
                _ => todo!("non-ident callee in Call lowering"),
            };
            let call_info = ctx.func_index.get(&callee_name).copied();
            if let Some((fid, ret_ty)) = call_info {
                ctx.emit(
                    Opcode::Call,
                    ret_ty,
                    arg_ids,
                    InstData::CallTarget(fid),
                    span,
                )
            } else if let Some((sym, ret_ty)) = vow_builtin_to_runtime(&callee_name) {
                ctx.emit(
                    Opcode::Call,
                    ret_ty,
                    arg_ids,
                    InstData::CallExtern(sym.to_string()),
                    span,
                )
            } else {
                ctx.emit(
                    Opcode::Call,
                    Ty::Unit,
                    arg_ids,
                    InstData::CallExtern(callee_name),
                    span,
                )
            }
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            // Collect variables that may be mutated in any branch AND exist in outer scope.
            let mutations: Vec<(String, InstId)> =
                collect_if_mutations(ctx, then_branch, else_branch.as_deref());

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

            // Snapshot scope so then-branch mutations don't bleed into else-branch.
            let scope_snap = ctx.snapshot_scope();

            // Lower then-branch.
            ctx.switch_to_block(then_block);
            let then_val = lower_block(ctx, then_branch);
            let then_terminated = ctx.is_terminated();
            let then_upsilon_block = ctx.current_block;
            // Capture mutation values from then-branch (or pre-if value if not modified).
            let then_mut_vals: Vec<InstId> = mutations
                .iter()
                .map(|(name, pre_id)| ctx.lookup(name).unwrap_or(*pre_id))
                .collect();
            let then_upsilon_id = if !then_terminated {
                let u = ctx.emit(
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
                Some(u)
            } else {
                None
            };

            // Restore scope so else-branch starts from the pre-if state.
            ctx.restore_scope(scope_snap.clone());

            // Lower else-branch.
            ctx.switch_to_block(else_block);
            let else_val = if let Some(else_expr) = else_branch {
                lower_expr(ctx, else_expr)
            } else {
                ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
            };
            let else_terminated = ctx.is_terminated();
            let else_upsilon_block = ctx.current_block;
            let else_mut_vals: Vec<InstId> = mutations
                .iter()
                .map(|(name, pre_id)| ctx.lookup(name).unwrap_or(*pre_id))
                .collect();
            let else_upsilon_id = if !else_terminated {
                let u = ctx.emit(
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
                Some(u)
            } else {
                None
            };

            // Restore scope before building merge.
            ctx.restore_scope(scope_snap);

            ctx.switch_to_block(merge_block);

            // Create Phis for each mutated variable, wiring Upsilons from both branches.
            // Upsilons are appended even after the Jump (they are no-ops in codegen but
            // are found by collect_target_block_args which scans all instructions).
            for (i, (name, pre_id)) in mutations.iter().enumerate() {
                let t_val = then_mut_vals[i];
                let e_val = else_mut_vals[i];
                if t_val == *pre_id && e_val == *pre_id {
                    // Variable unchanged by both branches — no phi needed.
                    continue;
                }
                let phi_ty = ctx.inst_ty(t_val);
                let phi_id = ctx.emit(Opcode::Phi, phi_ty, vec![], InstData::None, span);
                if !then_terminated {
                    ctx.switch_to_block(then_upsilon_block);
                    ctx.emit(
                        Opcode::Upsilon,
                        phi_ty,
                        vec![t_val],
                        InstData::PhiTarget(phi_id),
                        span,
                    );
                    ctx.switch_to_block(merge_block);
                }
                if !else_terminated {
                    ctx.switch_to_block(else_upsilon_block);
                    ctx.emit(
                        Opcode::Upsilon,
                        phi_ty,
                        vec![e_val],
                        InstData::PhiTarget(phi_id),
                        span,
                    );
                    ctx.switch_to_block(merge_block);
                }
                ctx.assign(name, phi_id);
            }

            match (then_upsilon_id, else_upsilon_id) {
                (None, None) => {
                    // Both branches terminate — merge block is unreachable.
                    ctx.emit(Opcode::Unreachable, Ty::Unit, vec![], InstData::None, span)
                }
                (Some(t_up), None) => {
                    let phi_ty = ctx.inst_ty(then_val);
                    let phi_id = ctx.emit(Opcode::Phi, phi_ty, vec![], InstData::None, span);
                    backpatch_upsilon(ctx, then_upsilon_block, t_up, phi_id);
                    phi_id
                }
                (None, Some(e_up)) => {
                    let phi_ty = ctx.inst_ty(else_val);
                    let phi_id = ctx.emit(Opcode::Phi, phi_ty, vec![], InstData::None, span);
                    backpatch_upsilon(ctx, else_upsilon_block, e_up, phi_id);
                    phi_id
                }
                (Some(t_up), Some(e_up)) => {
                    let phi_ty = ctx.inst_ty(then_val);
                    let phi_id = ctx.emit(Opcode::Phi, phi_ty, vec![], InstData::None, span);
                    backpatch_upsilon(ctx, then_upsilon_block, t_up, phi_id);
                    backpatch_upsilon(ctx, else_upsilon_block, e_up, phi_id);
                    phi_id
                }
            }
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
                if let Some(vow_block) = ctx.vow_block.clone() {
                    vow::lower_ensures(ctx, &vow_block, val);
                }
                ctx.emit(Opcode::Return, Ty::Unit, vec![val], InstData::None, span)
            } else {
                let unit = ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span);
                if let Some(vow_block) = ctx.vow_block.clone() {
                    vow::lower_ensures(ctx, &vow_block, unit);
                }
                ctx.emit(Opcode::Return, Ty::Unit, vec![unit], InstData::None, span)
            }
        }
        ExprKind::Assign { lhs, rhs } => {
            let new_val = lower_expr(ctx, rhs);
            match &lhs.kind {
                ExprKind::Ident(name) => {
                    ctx.assign(name, new_val);
                }
                ExprKind::FieldAccess { base, field } => {
                    let ptr_id = lower_expr(ctx, base);
                    let struct_name =
                        ctx.inst_struct_type.get(&ptr_id).cloned().unwrap_or_default();
                    if struct_name.is_empty() {
                        eprintln!("warning: FieldSet on untagged instruction %{}, field '{}' — defaulting to index 0", ptr_id.0, field);
                    }
                    let field_idx = ctx
                        .struct_field_map
                        .get(&struct_name)
                        .and_then(|names| names.iter().position(|n| n == field))
                        .unwrap_or(0) as u32;
                    ctx.emit(
                        Opcode::FieldSet,
                        Ty::Unit,
                        vec![ptr_id, new_val],
                        InstData::FieldIndex(field_idx),
                        span,
                    );
                }
                ExprKind::Index { base, index } => {
                    let vec_ptr = lower_expr(ctx, base);
                    let idx_id = lower_expr(ctx, index);
                    ctx.emit(
                        Opcode::Call,
                        Ty::Unit,
                        vec![vec_ptr, idx_id, new_val],
                        InstData::CallExtern("__vow_vec_set_val".to_string()),
                        span,
                    );
                }
                _ => {}
            }
            new_val
        }
        ExprKind::While {
            condition,
            body,
            vow: while_vow,
        } => {
            let mutated = collect_assigned_vars(body);

            // Gather pre-loop (name, current_value) for mutated vars that exist in scope.
            let loop_vars: Vec<(String, InstId)> = mutated
                .into_iter()
                .filter_map(|name| ctx.lookup(&name).map(|id| (name, id)))
                .collect();

            let pre_header_block = ctx.current_block;
            let header_block = ctx.new_block();
            let body_block = ctx.new_block();
            let exit_block = ctx.new_block();

            // Emit placeholder Upsilons for each loop var, then jump to header.
            let mut upsilon_ids: Vec<(String, InstId)> = vec![];
            for (name, pre_val) in &loop_vars {
                let ty = ctx.inst_ty(*pre_val);
                let up_id = ctx.emit(
                    Opcode::Upsilon,
                    ty,
                    vec![*pre_val],
                    InstData::PhiTarget(InstId(u32::MAX)),
                    span,
                );
                upsilon_ids.push((name.clone(), up_id));
            }
            ctx.emit(
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(header_block),
                span,
            );

            // Header: emit Phis, then backpatch the pre-header Upsilons.
            ctx.switch_to_block(header_block);
            let mut phi_ids: Vec<(String, InstId)> = vec![];
            for (name, pre_val) in &loop_vars {
                let ty = ctx.inst_ty(*pre_val);
                let phi_id = ctx.emit(Opcode::Phi, ty, vec![], InstData::None, span);
                phi_ids.push((name.clone(), phi_id));
            }
            for (name, up_id) in &upsilon_ids {
                let phi_id = phi_ids.iter().find(|(n, _)| n == name).unwrap().1;
                backpatch_upsilon(ctx, pre_header_block, *up_id, phi_id);
            }

            // Update scope: rebind each loop var to its Phi.
            for (name, phi_id) in &phi_ids {
                ctx.assign(name, *phi_id);
            }

            // Lower vow invariant at top of header (before condition).
            if let Some(wv) = while_vow {
                vow::lower_invariant(ctx, wv);
            }

            // Lower condition, then branch.
            let cond_id = lower_expr(ctx, condition);
            ctx.emit(
                Opcode::Branch,
                Ty::Unit,
                vec![cond_id],
                InstData::BranchTargets {
                    then_block: body_block,
                    else_block: exit_block,
                },
                span,
            );

            // Body: lower body (push/pop scope handles lets inside body).
            ctx.switch_to_block(body_block);
            lower_block(ctx, body);

            // Emit back-edge Upsilons with the current scope values.
            for (name, phi_id) in &phi_ids {
                if let Some(cur_val) = ctx.lookup(name) {
                    ctx.emit(
                        Opcode::Upsilon,
                        ctx.inst_ty(cur_val),
                        vec![cur_val],
                        InstData::PhiTarget(*phi_id),
                        span,
                    );
                }
            }
            ctx.emit(
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(header_block),
                span,
            );

            // Restore scope to Phi values so the exit block sees the loop-exit values.
            for (name, phi_id) in &phi_ids {
                ctx.assign(name, *phi_id);
            }

            // Exit block.
            ctx.switch_to_block(exit_block);
            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
        }
        ExprKind::FieldAccess { base, field } => {
            let ptr_id = lower_expr(ctx, base);
            let struct_name = ctx.inst_struct_type.get(&ptr_id).cloned().unwrap_or_default();
            if struct_name.is_empty() {
                eprintln!("warning: FieldGet on untagged instruction %{}, field '{}' — defaulting to index 0", ptr_id.0, field);
            }
            let field_idx = ctx
                .struct_field_map
                .get(&struct_name)
                .and_then(|names| names.iter().position(|n| n == field))
                .unwrap_or(0) as u32;
            let result_id = ctx.emit(
                Opcode::FieldGet,
                Ty::I64,
                vec![ptr_id],
                InstData::FieldIndex(field_idx),
                span,
            );
            if let Some(type_names) = ctx.struct_field_type_names.get(&struct_name)
                && let Some(type_name) = type_names.get(field_idx as usize)
                && !type_name.is_empty()
                && !matches!(
                    type_name.as_str(),
                    "i32" | "i64" | "f32" | "f64" | "bool"
                )
            {
                ctx.inst_struct_type.insert(result_id, type_name.clone());
            }
            result_id
        }
        ExprKind::StructLiteral { name, fields } => {
            let field_names = ctx
                .struct_field_map
                .get(name)
                .cloned()
                .unwrap_or_default();
            let n_fields = field_names.len().max(fields.len());
            let ptr_id = ctx.emit(
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize {
                    size: n_fields as u32 * 8,
                    align: 8,
                },
                span,
            );
            ctx.inst_struct_type.insert(ptr_id, name.clone());
            for (field_name, field_expr) in fields {
                let idx = field_names
                    .iter()
                    .position(|n| n == field_name)
                    .unwrap_or_else(|| {
                        eprintln!("warning: StructLiteral field '{}' not found in struct '{}' — defaulting to index 0", field_name, name);
                        0
                    }) as u32;
                let val_id = lower_expr(ctx, field_expr);
                ctx.emit(
                    Opcode::FieldSet,
                    Ty::Unit,
                    vec![ptr_id, val_id],
                    InstData::FieldIndex(idx),
                    span,
                );
            }
            ptr_id
        }
        ExprKind::EnumConstruct { path, fields } => {
            let enum_name = path.first().map(|s| s.as_str()).unwrap_or("");
            let variant_name = path.get(1).map(|s| s.as_str()).unwrap_or("");
            // String::from(lit) builtin: lower_expr for Lit::String already calls
            // __vow_string_from_cstr and returns a tagged VowVec; no second call needed.
            if enum_name == "String" && variant_name == "from" {
                let lit_expr = fields.first().expect("String::from requires an argument");
                let ptr_id = lower_expr(ctx, lit_expr);
                ctx.inst_struct_type.insert(ptr_id, "String".to_string());
                return ptr_id;
            }
            // HashMap::new() builtin
            if enum_name == "HashMap" && variant_name == "new" {
                let result = ctx.emit(
                    Opcode::Call,
                    Ty::Ptr,
                    vec![],
                    InstData::CallExtern("__vow_map_new".to_string()),
                    span,
                );
                ctx.inst_struct_type.insert(result, "HashMap".to_string());
                return result;
            }
            // Vec::new() builtin
            if enum_name == "Vec" && variant_name == "new" {
                let size_val = ctx.emit(
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(8),
                    span,
                );
                let align_val = ctx.emit(
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(8),
                    span,
                );
                return ctx.emit(
                    Opcode::Call,
                    Ty::Ptr,
                    vec![size_val, align_val],
                    InstData::CallExtern("__vow_vec_new".to_string()),
                    span,
                );
            }
            let tag = ctx
                .enum_variant_map
                .get(enum_name)
                .and_then(|vs| vs.iter().position(|v| v == variant_name))
                .unwrap_or(0) as i64;
            let n_payload = fields.len();
            let size = (1 + n_payload) as u32 * 8;
            let ptr_id = ctx.emit(
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size, align: 8 },
                span,
            );
            ctx.inst_struct_type.insert(ptr_id, enum_name.to_string());
            let tag_val = ctx.emit(
                Opcode::ConstI64,
                Ty::I64,
                vec![],
                InstData::ConstI64(tag),
                span,
            );
            ctx.emit(
                Opcode::FieldSet,
                Ty::Unit,
                vec![ptr_id, tag_val],
                InstData::FieldIndex(0),
                span,
            );
            for (i, field_expr) in fields.iter().enumerate() {
                let val_id = lower_expr(ctx, field_expr);
                ctx.emit(
                    Opcode::FieldSet,
                    Ty::Unit,
                    vec![ptr_id, val_id],
                    InstData::FieldIndex(1 + i as u32),
                    span,
                );
            }
            ptr_id
        }
        ExprKind::Match { scrutinee, arms } => {
            let ptr_id = lower_expr(ctx, scrutinee);
            // Load enum discriminant (stored as i64 at field 0)
            let tag_id = ctx.emit(
                Opcode::FieldGet,
                Ty::I64,
                vec![ptr_id],
                InstData::FieldIndex(0),
                span,
            );

            let merge_block = ctx.new_block();
            let mut arm_results: Vec<(BlockId, InstId, Ty)> = Vec::new();

            let mut arm_iter = arms.iter().peekable();
            while let Some(arm) = arm_iter.next() {
                let is_last = arm_iter.peek().is_none();
                match &arm.pattern.kind {
                    PatKind::EnumVariant { path, inner } => {
                        let enum_name =
                            path.first().map(|s| s.as_str()).unwrap_or("");
                        let variant_name =
                            path.get(1).map(|s| s.as_str()).unwrap_or("");
                        let expected_tag = ctx
                            .enum_variant_map
                            .get(enum_name)
                            .and_then(|vs| vs.iter().position(|v| v == variant_name))
                            .unwrap_or(0) as i64;

                        let arm_block = ctx.new_block();
                        let next_check_block = if is_last {
                            arm_block
                        } else {
                            ctx.new_block()
                        };

                        let expected_id = ctx.emit(
                            Opcode::ConstI64,
                            Ty::I64,
                            vec![],
                            InstData::ConstI64(expected_tag),
                            span,
                        );
                        let cmp_id = ctx.emit(
                            Opcode::EqI64,
                            Ty::Bool,
                            vec![tag_id, expected_id],
                            InstData::None,
                            span,
                        );
                        ctx.emit(
                            Opcode::Branch,
                            Ty::Unit,
                            vec![cmp_id],
                            InstData::BranchTargets {
                                then_block: arm_block,
                                else_block: next_check_block,
                            },
                            span,
                        );

                        ctx.switch_to_block(arm_block);
                        ctx.push_scope();
                        // Bind payload fields
                        for (i, inner_pat) in inner.iter().enumerate() {
                            if let PatKind::Ident { name, .. } = &inner_pat.kind {
                                let field_val = ctx.emit(
                                    Opcode::FieldGet,
                                    Ty::I64,
                                    vec![ptr_id],
                                    InstData::FieldIndex(1 + i as u32),
                                    span,
                                );
                                ctx.define(name.clone(), field_val);
                            }
                        }
                        let arm_result = lower_expr(ctx, &arm.body);
                        let arm_ty = ctx.inst_ty(arm_result);
                        ctx.pop_scope();
                        let up_id = ctx.emit(
                            Opcode::Upsilon,
                            Ty::Unit,
                            vec![arm_result],
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
                        arm_results.push((arm_block, up_id, arm_ty));

                        if !is_last {
                            ctx.switch_to_block(next_check_block);
                        }
                    }
                    PatKind::Wildcard | PatKind::Ident { .. } => {
                        // Catch-all arm — just lower the body
                        let arm_block = ctx.current_block;
                        if let PatKind::Ident { name, .. } = &arm.pattern.kind {
                            ctx.push_scope();
                            ctx.define(name.clone(), ptr_id);
                        } else {
                            ctx.push_scope();
                        }
                        let arm_result = lower_expr(ctx, &arm.body);
                        let arm_ty = ctx.inst_ty(arm_result);
                        ctx.pop_scope();
                        let up_id = ctx.emit(
                            Opcode::Upsilon,
                            Ty::Unit,
                            vec![arm_result],
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
                        arm_results.push((arm_block, up_id, arm_ty));
                    }
                    _ => {
                        // Other patterns not yet supported; emit a unit and jump to merge
                        let arm_block = ctx.current_block;
                        let unit = ctx.emit(
                            Opcode::ConstUnit,
                            Ty::Unit,
                            vec![],
                            InstData::None,
                            span,
                        );
                        let up_id = ctx.emit(
                            Opcode::Upsilon,
                            Ty::Unit,
                            vec![unit],
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
                        arm_results.push((arm_block, up_id, Ty::Unit));
                    }
                }
            }

            let phi_ty = arm_results
                .first()
                .map(|(_, _, ty)| *ty)
                .unwrap_or(Ty::I64);
            ctx.switch_to_block(merge_block);
            let phi_id = ctx.emit(Opcode::Phi, phi_ty, vec![], InstData::None, span);

            for (arm_block, up_id, _) in arm_results {
                backpatch_upsilon(ctx, arm_block, up_id, phi_id);
            }

            phi_id
        }
        ExprKind::MethodCall {
            receiver,
            method,
            args,
        } => {
            let recv_id = lower_expr(ctx, receiver);
            let recv_struct = ctx.inst_struct_type.get(&recv_id).cloned();
            match (recv_struct.as_deref(), method.as_str()) {
                (Some("String"), "len") => ctx.emit(
                    Opcode::Call,
                    Ty::I64,
                    vec![recv_id],
                    InstData::CallExtern("__vow_string_len".to_string()),
                    span,
                ),
                (Some("String"), "push_str") => {
                    let arg_id = args
                        .first()
                        .map(|e| lower_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Unit,
                        vec![recv_id, arg_id],
                        InstData::CallExtern("__vow_string_push_str".to_string()),
                        span,
                    )
                }
                (Some("String"), "eq") => {
                    let arg_id = args
                        .first()
                        .map(|e| lower_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Bool,
                        vec![recv_id, arg_id],
                        InstData::CallExtern("__vow_string_eq".to_string()),
                        span,
                    )
                }
                (Some("String"), "byte_at") => {
                    let idx_id = args
                        .first()
                        .map(|e| lower_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::I64,
                        vec![recv_id, idx_id],
                        InstData::CallExtern("__vow_string_byte_at".to_string()),
                        span,
                    )
                }
                (Some("String"), "push_byte") => {
                    let byte_id = args
                        .first()
                        .map(|e| lower_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Unit,
                        vec![recv_id, byte_id],
                        InstData::CallExtern("__vow_string_push_byte".to_string()),
                        span,
                    )
                }
                (Some("HashMap"), "len") => ctx.emit(
                    Opcode::Call,
                    Ty::I64,
                    vec![recv_id],
                    InstData::CallExtern("__vow_map_len".to_string()),
                    span,
                ),
                (Some("HashMap"), "insert") => {
                    let k_id = args
                        .first()
                        .map(|e| lower_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    let v_id = args
                        .get(1)
                        .map(|e| lower_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Unit,
                        vec![recv_id, k_id, v_id],
                        InstData::CallExtern("__vow_map_insert".to_string()),
                        span,
                    )
                }
                (Some("HashMap"), "get") => {
                    let k_id = args
                        .first()
                        .map(|e| lower_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::I64,
                        vec![recv_id, k_id],
                        InstData::CallExtern("__vow_map_get".to_string()),
                        span,
                    )
                }
                (Some("HashMap"), "contains_key") => {
                    let k_id = args
                        .first()
                        .map(|e| lower_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Bool,
                        vec![recv_id, k_id],
                        InstData::CallExtern("__vow_map_contains".to_string()),
                        span,
                    )
                }
                (Some("HashMap"), "remove") => {
                    let k_id = args
                        .first()
                        .map(|e| lower_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Unit,
                        vec![recv_id, k_id],
                        InstData::CallExtern("__vow_map_remove".to_string()),
                        span,
                    )
                }
                (_, "len") => ctx.emit(
                    Opcode::Call,
                    Ty::I64,
                    vec![recv_id],
                    InstData::CallExtern("__vow_vec_len".to_string()),
                    span,
                ),
                (_, "push") => {
                    let elem_id = args
                        .first()
                        .map(|e| lower_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Unit,
                        vec![recv_id, elem_id],
                        InstData::CallExtern("__vow_vec_push_val".to_string()),
                        span,
                    )
                }
                (_, "pop") => ctx.emit(
                    Opcode::Call,
                    Ty::Unit,
                    vec![recv_id],
                    InstData::CallExtern("__vow_vec_pop".to_string()),
                    span,
                ),
                _ => {
                    for a in args {
                        lower_expr(ctx, a);
                    }
                    ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                }
            }
        }
        ExprKind::Index { base, index } => {
            let vec_ptr = lower_expr(ctx, base);
            let idx_id = lower_expr(ctx, index);
            ctx.emit(
                Opcode::Call,
                Ty::I64,
                vec![vec_ptr, idx_id],
                InstData::CallExtern("__vow_vec_get_val".to_string()),
                span,
            )
        }
        // ? operator: unwrap Option::Some or short-circuit with None
        ExprKind::Question { expr: inner } => {
            let ptr_id = lower_expr(ctx, inner);
            // Load discriminant from field 0
            let tag_id = ctx.emit(
                Opcode::FieldGet,
                Ty::I64,
                vec![ptr_id],
                InstData::FieldIndex(0),
                span,
            );
            let zero_id = ctx.emit(
                Opcode::ConstI64,
                Ty::I64,
                vec![],
                InstData::ConstI64(0),
                span,
            );
            // tag == 0 means None (short-circuit) for Option; Ok (continue) for Result
            let is_none = ctx.emit(
                Opcode::EqI64,
                Ty::Bool,
                vec![tag_id, zero_id],
                InstData::None,
                span,
            );
            let early_return_block = ctx.new_block();
            let continue_block = ctx.new_block();
            ctx.emit(
                Opcode::Branch,
                Ty::Unit,
                vec![is_none],
                InstData::BranchTargets {
                    then_block: early_return_block,
                    else_block: continue_block,
                },
                span,
            );

            // Early return: wrap as None and return
            ctx.switch_to_block(early_return_block);
            let none_size: u32 = 8; // just the discriminant slot
            let none_ptr = ctx.emit(
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize {
                    size: none_size,
                    align: 8,
                },
                span,
            );
            let none_tag = ctx.emit(
                Opcode::ConstI64,
                Ty::I64,
                vec![],
                InstData::ConstI64(0),
                span,
            );
            ctx.emit(
                Opcode::FieldSet,
                Ty::Unit,
                vec![none_ptr, none_tag],
                InstData::FieldIndex(0),
                span,
            );
            if let Some(vow_block) = ctx.vow_block.clone() {
                vow::lower_ensures(ctx, &vow_block, none_ptr);
            }
            ctx.emit(
                Opcode::Return,
                Ty::Unit,
                vec![none_ptr],
                InstData::None,
                span,
            );

            // Continue: extract payload from field 1
            ctx.switch_to_block(continue_block);
            ctx.emit(
                Opcode::FieldGet,
                Ty::I64,
                vec![ptr_id],
                InstData::FieldIndex(1),
                span,
            )
        }
        _ => todo!("IR lowering not implemented for {:?}", expr.kind),
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
        Stmt::Let { pattern, init, ty, .. } => {
            let val = lower_expr(ctx, init);
            if let PatKind::Ident { name, .. } = &pattern.kind {
                if let Some(ann) = ty {
                    match ann {
                        AstType::Named { name: type_name, .. } => {
                            match type_name.as_str() {
                                "i32" | "i64" | "f32" | "f64" | "bool" => {}
                                _ => {
                                    ctx.inst_struct_type.insert(val, type_name.clone());
                                }
                            }
                        }
                        AstType::Generic { name: type_name, .. } => {
                            ctx.inst_struct_type.insert(val, type_name.clone());
                        }
                        _ => {}
                    }
                }
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
        if ctx.is_terminated() {
            break;
        }
        lower_stmt(ctx, stmt);
    }
    if ctx.is_terminated() {
        // Block already terminated (e.g. by a return statement); no trailing expr.
        // Return a sentinel — callers that care will check is_terminated().
        InstId(u32::MAX)
    } else if let Some(expr) = &block.trailing_expr {
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

pub fn lower_function(
    fn_def: &FnDef,
    func_index: &HashMap<String, (FuncId, Ty)>,
    struct_field_map: HashMap<String, Vec<String>>,
    enum_variant_map: HashMap<String, Vec<String>>,
    struct_field_type_names: HashMap<String, Vec<String>>,
) -> (Function, Vec<String>) {
    let params: Vec<Ty> = fn_def.params.iter().map(|p| lower_ty(&p.ty)).collect();
    let return_ty = lower_ty(&fn_def.return_ty);
    let effects = fn_def.effects.clone();

    let mut ctx = LowerCtx::new(
        fn_def.name.clone(),
        params.clone(),
        return_ty,
        effects,
        func_index.clone(),
        struct_field_map,
        enum_variant_map,
        struct_field_type_names,
    );

    if let Some(vow) = &fn_def.vow {
        ctx.vow_block = Some(vow.clone());
    }

    for (idx, param) in fn_def.params.iter().enumerate() {
        let ty = params[idx];
        let arg_id = ctx.emit(
            Opcode::GetArg,
            ty,
            vec![],
            InstData::ArgIndex(idx as u32),
            fn_def.span,
        );
        match &param.ty {
            AstType::Named { name, .. } if name == "str" || name == "String" => {
                ctx.inst_struct_type.insert(arg_id, "String".to_string());
            }
            AstType::Generic { name, .. } if name == "HashMap" => {
                ctx.inst_struct_type.insert(arg_id, "HashMap".to_string());
            }
            AstType::Generic { name, .. } if name == "Vec" => {
                ctx.inst_struct_type.insert(arg_id, "Vec".to_string());
            }
            AstType::Named { name, .. } if ctx.struct_field_map.contains_key(name.as_str()) => {
                ctx.inst_struct_type.insert(arg_id, name.clone());
            }
            _ => {}
        }
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
    let fn_items: Vec<&FnDef> = module
        .items
        .iter()
        .filter_map(|item| {
            if let Item::Fn(fn_def) = item {
                Some(fn_def)
            } else {
                None
            }
        })
        .collect();

    let func_index: HashMap<String, (FuncId, Ty)> = fn_items
        .iter()
        .enumerate()
        .map(|(idx, fn_def)| {
            let ret_ty = lower_ty(&fn_def.return_ty);
            (fn_def.name.clone(), (FuncId(idx as u32), ret_ty))
        })
        .collect();

    // Build struct layout info
    let mut struct_field_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut struct_layouts: Vec<StructLayout> = Vec::new();
    for item in &module.items {
        if let Item::Struct(s) = item {
            let field_names: Vec<String> = s.fields.iter().map(|f| f.name.clone()).collect();
            let field_layouts: Vec<FieldLayout> = s
                .fields
                .iter()
                .map(|f| FieldLayout {
                    name: f.name.clone(),
                    ty: lower_ty(&f.ty),
                })
                .collect();
            struct_field_map.insert(s.name.clone(), field_names);
            struct_layouts.push(StructLayout {
                name: s.name.clone(),
                fields: field_layouts,
                is_linear: s.is_linear,
            });
        }
    }

    // Build struct field type names for FieldGet auto-tagging
    let mut struct_field_type_names: HashMap<String, Vec<String>> = HashMap::new();
    for item in &module.items {
        if let Item::Struct(s) = item {
            let type_names: Vec<String> = s
                .fields
                .iter()
                .map(|f| match &f.ty {
                    AstType::Named { name, .. } => name.clone(),
                    AstType::Generic { name, .. } => name.clone(),
                    _ => String::new(),
                })
                .collect();
            struct_field_type_names.insert(s.name.clone(), type_names);
        }
    }

    // Build enum layout info
    let mut enum_variant_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut enum_layouts: Vec<EnumLayout> = Vec::new();
    for item in &module.items {
        if let Item::Enum(e) = item {
            let variant_names: Vec<String> =
                e.variants.iter().map(|v| v.name.clone()).collect();
            let variant_layouts: Vec<VariantLayout> = e
                .variants
                .iter()
                .enumerate()
                .map(|(tag, v)| {
                    let payload: Vec<FieldLayout> = match &v.kind {
                        VariantKind::Unit => vec![],
                        VariantKind::Tuple(tys) => tys
                            .iter()
                            .enumerate()
                            .map(|(i, ty)| FieldLayout {
                                name: i.to_string(),
                                ty: lower_ty(ty),
                            })
                            .collect(),
                        VariantKind::Struct(fields) => fields
                            .iter()
                            .map(|f| FieldLayout {
                                name: f.name.clone(),
                                ty: lower_ty(&f.ty),
                            })
                            .collect(),
                    };
                    VariantLayout {
                        name: v.name.clone(),
                        tag: tag as u64,
                        payload,
                    }
                })
                .collect();
            enum_variant_map.insert(e.name.clone(), variant_names);
            enum_layouts.push(EnumLayout {
                name: e.name.clone(),
                variants: variant_layouts,
            });
        }
    }

    let mut all_strings: Vec<String> = Vec::new();
    let functions: Vec<Function> = fn_items
        .iter()
        .enumerate()
        .map(|(idx, fn_def)| {
            let (mut func, pool) = lower_function(
                fn_def,
                &func_index,
                struct_field_map.clone(),
                enum_variant_map.clone(),
                struct_field_type_names.clone(),
            );
            func.id = FuncId(idx as u32);
            let base = all_strings.len() as u32;
            if base > 0 || !pool.is_empty() {
                for block in &mut func.blocks {
                    for inst in &mut block.insts {
                        if let InstData::ConstStr(ref mut i) = inst.data {
                            *i += base;
                        }
                    }
                }
            }
            all_strings.extend(pool);
            func
        })
        .collect();

    Module {
        name: module.name.clone(),
        strings: all_strings,
        struct_layouts,
        enum_layouts,
        functions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vow_syntax::ast::{
        Block, Effect, Expr, ExprKind, FnDef, Lit, Pat, PatKind, Stmt, Type, Visibility, VowBlock,
        VowClause,
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
        let (func, _) = lower_function(&fn_def, &HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new());

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
        let (func, _) = lower_function(&fn_def, &HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new());

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
        let (func, _) = lower_function(&fn_def, &HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new());

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
        let (func, _) = lower_function(&fn_def, &HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new());

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
        let (func, _) = lower_function(&fn_def, &HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new());

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();
        let ret = all_insts.iter().find(|i| i.opcode == Opcode::Return);
        assert!(ret.is_some(), "expected Return instruction");
        assert_eq!(func.return_ty, Ty::Unit);
    }

    #[test]
    fn ensures_emitted_before_explicit_return() {
        let ensures_clause = VowClause::Ensures {
            expr: bool_expr(true),
            span: sp(),
        };
        let vow_block = VowBlock {
            clauses: vec![ensures_clause],
            span: sp(),
        };
        let return_expr = Expr {
            kind: ExprKind::Return {
                value: Some(Box::new(int_expr(42))),
            },
            span: sp(),
        };
        let body = Block {
            stmts: vec![Stmt::Expr {
                expr: return_expr,
                has_semicolon: true,
                span: sp(),
            }],
            trailing_expr: None,
            span: sp(),
        };
        let fn_def = FnDef {
            vis: Visibility::Public,
            name: "explicit_return_fn".to_string(),
            params: vec![],
            return_ty: i64_ty(),
            effects: vec![],
            vow: Some(vow_block),
            body,
            span: sp(),
        };
        let (func, _) = lower_function(&fn_def, &HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new());

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();
        let ens_pos = all_insts
            .iter()
            .position(|i| i.opcode == Opcode::VowEnsures)
            .expect("expected VowEnsures");
        let ret_pos = all_insts
            .iter()
            .position(|i| i.opcode == Opcode::Return)
            .expect("expected Return");
        assert!(
            ens_pos < ret_pos,
            "VowEnsures must appear before Return for explicit return"
        );
    }

    #[test]
    fn lower_while_loop_emits_phi_upsilon_and_backedge() {
        // fn countdown(n: i64) -> i64 { let mut i = n; while i > 0 { i = i - 1 }; i }
        let i64_ty = i64_ty();
        let param_n = make_param("n", i64_ty.clone());

        // let mut i = n
        let let_i = Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: "i".to_string(),
                    is_mut: true,
                },
                span: sp(),
            },
            ty: None,
            init: Box::new(ident_expr("n")),
            span: sp(),
        };

        // while body: i = i - 1
        let assign_stmt = Stmt::Expr {
            expr: Expr {
                kind: ExprKind::Assign {
                    lhs: Box::new(ident_expr("i")),
                    rhs: Box::new(Expr {
                        kind: ExprKind::BinaryOp {
                            op: BinOp::Sub,
                            lhs: Box::new(ident_expr("i")),
                            rhs: Box::new(int_expr(1)),
                        },
                        span: sp(),
                    }),
                },
                span: sp(),
            },
            has_semicolon: true,
            span: sp(),
        };
        let while_body = Block {
            stmts: vec![assign_stmt],
            trailing_expr: None,
            span: sp(),
        };

        // while i > 0 { ... }
        let while_expr = Expr {
            kind: ExprKind::While {
                condition: Box::new(Expr {
                    kind: ExprKind::BinaryOp {
                        op: BinOp::Gt,
                        lhs: Box::new(ident_expr("i")),
                        rhs: Box::new(int_expr(0)),
                    },
                    span: sp(),
                }),
                vow: None,
                body: Box::new(while_body),
            },
            span: sp(),
        };

        let body = Block {
            stmts: vec![
                let_i,
                Stmt::Expr {
                    expr: while_expr,
                    has_semicolon: true,
                    span: sp(),
                },
            ],
            trailing_expr: Some(Box::new(ident_expr("i"))),
            span: sp(),
        };

        let fn_def = make_fn("countdown", vec![param_n], i64_ty, body, vec![]);
        let (func, _) = lower_function(&fn_def, &HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new());

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();

        // Must have a Phi (for loop var `i`)
        let phi = all_insts.iter().find(|i| i.opcode == Opcode::Phi);
        assert!(phi.is_some(), "expected Phi for loop variable");

        // Must have at least 2 Upsilons: pre-loop initial feed and back-edge feed
        let upsilons: Vec<_> = all_insts
            .iter()
            .filter(|i| i.opcode == Opcode::Upsilon)
            .collect();
        assert!(
            upsilons.len() >= 2,
            "expected at least 2 Upsilons for while loop"
        );

        // Must have a GtI64 for the condition
        assert!(
            all_insts.iter().any(|i| i.opcode == Opcode::GtI64),
            "expected GtI64 for while condition"
        );

        // Must have Branch
        assert!(
            all_insts.iter().any(|i| i.opcode == Opcode::Branch),
            "expected Branch for while loop"
        );

        // Must have at least 2 Jumps (pre-header -> header, body -> header)
        let jumps: Vec<_> = all_insts
            .iter()
            .filter(|i| i.opcode == Opcode::Jump)
            .collect();
        assert!(jumps.len() >= 2, "expected at least 2 Jumps for while loop");

        // Should produce at least 4 blocks: entry, header, body, exit
        assert!(
            func.blocks.len() >= 4,
            "expected entry+header+body+exit blocks"
        );
    }
}
