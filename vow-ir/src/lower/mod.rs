pub mod vow;

use std::collections::{HashMap, HashSet};

pub type StringExprSet = HashSet<usize>;

use vow_diag::Blame;
use vow_syntax::ast::{
    BinOp, Block, Effect, Expr, ExprKind, FnDef, Item, Lit, Module as AstModule, PatKind, Stmt,
    Type as AstType, UnOp, VariantKind, VowBlock,
};
use vow_syntax::span::Span;

use crate::types::{
    BasicBlock, BlockId, EnumLayout, FieldLayout, FuncId, Function, Inst, InstData, InstId, Module,
    Opcode, StructLayout, Ty, VariantLayout, VowEntry, VowId,
};

fn builtin_alloc_tag(sym: &str) -> &'static str {
    match sym {
        "__vow_fs_read"
        | "__vow_string_substr"
        | "__vow_string_trim"
        | "__vow_string_to_upper"
        | "__vow_string_to_lower"
        | "__vow_string_replace"
        | "__vow_string_join"
        | "__vow_string_from_i64"
        | "__vow_stdin_read"
        | "__vow_stdin_read_line"
        | "__vow_process_get_stdout"
        | "__vow_process_get_stderr"
        | "__vow_process_stdout_for"
        | "__vow_process_stderr_for"
        | "__vow_hex_encode" => "String",
        "__vow_fs_listdir" | "__vow_string_split" | "__vow_vec_sort" | "__vow_hex_decode"
        | "__vow_args" => "Vec",
        _ => "",
    }
}

fn vow_debug_builtin_to_runtime(name: &str) -> Option<(&'static str, Ty)> {
    match name {
        "debug_str" => Some(("__vow_debug_str", Ty::Unit)),
        "debug_i64" => Some(("__vow_debug_i64", Ty::Unit)),
        "debug_u64" => Some(("__vow_debug_u64", Ty::Unit)),
        _ => None,
    }
}

fn vow_builtin_to_runtime(name: &str) -> Option<(&'static str, Ty)> {
    match name {
        "print_str" => Some(("__vow_string_print", Ty::Unit)),
        "print_i64" => Some(("__vow_print_i64", Ty::Unit)),
        "print_u64" => Some(("__vow_print_u64", Ty::Unit)),
        "eprintln_str" => Some(("__vow_eprintln_str", Ty::Unit)),
        "fs_read" => Some(("__vow_fs_read", Ty::Ptr)),
        "fs_write" => Some(("__vow_fs_write", Ty::I64)),
        "fs_exists" => Some(("__vow_fs_exists", Ty::I64)),
        "fs_mkdir" => Some(("__vow_fs_mkdir", Ty::I64)),
        "fs_listdir" => Some(("__vow_fs_listdir", Ty::Ptr)),
        "fs_remove" => Some(("__vow_fs_remove", Ty::I64)),
        "fs_remove_dir" => Some(("__vow_fs_remove_dir", Ty::I64)),
        "fs_is_dir" => Some(("__vow_fs_is_dir", Ty::I64)),
        "fs_rename" => Some(("__vow_fs_rename", Ty::I64)),
        "string_substr" => Some(("__vow_string_substr", Ty::Ptr)),
        "string_split" => Some(("__vow_string_split", Ty::Ptr)),
        "string_starts_with" => Some(("__vow_string_starts_with", Ty::I64)),
        "string_ends_with" => Some(("__vow_string_ends_with", Ty::I64)),
        "string_trim" => Some(("__vow_string_trim", Ty::Ptr)),
        "string_to_upper" => Some(("__vow_string_to_upper", Ty::Ptr)),
        "string_to_lower" => Some(("__vow_string_to_lower", Ty::Ptr)),
        "string_replace" => Some(("__vow_string_replace", Ty::Ptr)),
        "string_join" => Some(("__vow_string_join", Ty::Ptr)),
        "parse_i64" => Some(("__vow_parse_i64", Ty::I64)),
        "i64_to_string" => Some(("__vow_string_from_i64", Ty::Ptr)),
        "vec_sort" => Some(("__vow_vec_sort", Ty::Ptr)),
        "time_unix" => Some(("__vow_time_unix", Ty::I64)),
        "time_unix_ms" => Some(("__vow_time_unix_ms", Ty::I64)),
        "hex_encode" => Some(("__vow_hex_encode", Ty::Ptr)),
        "hex_decode" => Some(("__vow_hex_decode", Ty::Ptr)),
        "args" => Some(("__vow_args", Ty::Ptr)),
        "stdin_read" => Some(("__vow_stdin_read", Ty::Ptr)),
        "stdin_read_line" => Some(("__vow_stdin_read_line", Ty::Ptr)),
        "stdin_ready" => Some(("__vow_stdin_ready", Ty::Bool)),
        "process_exit" => Some(("__vow_process_exit", Ty::Unit)),
        "process_run" => Some(("__vow_process_run", Ty::I64)),
        "process_get_stdout" => Some(("__vow_process_get_stdout", Ty::Ptr)),
        "process_get_stderr" => Some(("__vow_process_get_stderr", Ty::Ptr)),
        "process_start" => Some(("__vow_process_start", Ty::I64)),
        "process_wait" => Some(("__vow_process_wait", Ty::I64)),
        "process_wait_timeout" => Some(("__vow_process_wait_timeout", Ty::I64)),
        "process_kill" => Some(("__vow_process_kill", Ty::I64)),
        "process_stdout_for" => Some(("__vow_process_stdout_for", Ty::Ptr)),
        "process_stderr_for" => Some(("__vow_process_stderr_for", Ty::Ptr)),
        "__vow_clif_create" => Some(("__vow_clif_create", Ty::I64)),
        "__vow_clif_add_string" => Some(("__vow_clif_add_string", Ty::Unit)),
        "__vow_clif_declare_extern" => Some(("__vow_clif_declare_extern", Ty::Unit)),
        "__vow_clif_declare_function" => Some(("__vow_clif_declare_function", Ty::Unit)),
        "__vow_clif_compile_function" => Some(("__vow_clif_compile_function", Ty::I64)),
        "__vow_clif_finish" => Some(("__vow_clif_finish", Ty::I64)),
        "__vow_clif_link" => Some(("__vow_clif_link", Ty::I64)),
        "__vow_clif_destroy" => Some(("__vow_clif_destroy", Ty::Unit)),
        _ => None,
    }
}

pub(crate) fn lower_ty(ast_ty: &AstType) -> Ty {
    match ast_ty {
        AstType::Named { name, .. } => match name.as_str() {
            "i32" => Ty::I32,
            "i64" => Ty::I64,
            "u64" => Ty::U64,
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
    string_pool_index: HashMap<String, u32>,
    func_index: HashMap<String, (FuncId, Ty)>,
    // struct name → field names in declaration order
    pub(super) struct_field_map: HashMap<String, Vec<String>>,
    // enum name → variant names in declaration order (index = tag)
    pub(super) enum_variant_map: HashMap<String, Vec<String>>,
    // InstId of a struct/enum allocation → type name
    pub(super) inst_struct_type: HashMap<InstId, String>,
    inst_ty_cache: HashMap<InstId, Ty>,
    // source file path for vow entries
    file: String,
    // struct name → field type names (from AST declarations) for FieldGet auto-tagging
    pub(super) struct_field_type_names: HashMap<String, Vec<String>>,
    // expr addresses whose resolved type is String (from checker)
    string_exprs: StringExprSet,
    // const name → (compile-time value, declared type)
    const_map: HashMap<String, (i64, Ty)>,
    // loop exit block stack for break
    loop_exit_blocks: Vec<BlockId>,
    // loop header block stack for continue
    loop_header_blocks: Vec<BlockId>,
    // Per-loop Phi IDs for back-edge Upsilons on continue
    loop_continue_phis: Vec<Vec<(String, InstId)>>,
    // For for-each: the index Phi to increment on continue (None for while/loop)
    loop_continue_idx_phi: Vec<Option<InstId>>,
    // Scope depth at loop header (before body scope push) for correct continue resolution.
    // continue must resolve loop-carried vars from this depth, not the current scope, to
    // avoid picking up shadowed bindings in inner blocks.
    loop_continue_scope_depth: Vec<usize>,
    // Per-loop break-value Upsilon collector.  `Some(vec)` for `loop` (collects
    // (source_block, upsilon_id, value_ty)), `None` for `while`.
    loop_break_upsilons: Vec<Option<Vec<(BlockId, InstId, Ty)>>>,
    // InstId of a Vec allocation → element type name (for struct-in-Vec field access)
    inst_vec_elem_type: HashMap<InstId, String>,
    // struct name → per-field Vec element type name (for FieldGet → Vec propagation)
    struct_field_vec_elems: HashMap<String, Vec<String>>,
    warnings: Vec<vow_diag::Diagnostic>,
    // Scope-based deallocation: stack of alloc scopes, each tracking (InstId, tag).
    // Push on entering a branch/loop/match arm, pop (with frees) on exit.
    alloc_scopes: Vec<Vec<(InstId, String)>>,
    escaped_allocs: HashSet<InstId>,
    // Alloc scope depth at each loop body entry (for break/continue frees).
    loop_alloc_scope_depth: Vec<usize>,
}

impl LowerCtx {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: String,
        params: Vec<Ty>,
        param_names: Vec<String>,
        return_ty: Ty,
        effects: Vec<Effect>,
        file: String,
        func_index: HashMap<String, (FuncId, Ty)>,
        struct_field_map: HashMap<String, Vec<String>>,
        enum_variant_map: HashMap<String, Vec<String>>,
        struct_field_type_names: HashMap<String, Vec<String>>,
        struct_field_vec_elems: HashMap<String, Vec<String>>,
        string_exprs: StringExprSet,
    ) -> Self {
        let entry = BasicBlock {
            id: BlockId(0),
            insts: vec![],
        };
        let func = Function {
            id: FuncId(0),
            name,
            params,
            param_names,
            return_ty,
            effects,
            vows: vec![],
            blocks: vec![entry],
            local_names: HashMap::new(),
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
            string_pool_index: HashMap::new(),
            func_index,
            struct_field_map,
            enum_variant_map,
            inst_struct_type: HashMap::new(),
            inst_ty_cache: HashMap::new(),
            file,
            struct_field_type_names,
            string_exprs,
            const_map: HashMap::new(),
            loop_exit_blocks: Vec::new(),
            loop_header_blocks: Vec::new(),
            loop_continue_phis: Vec::new(),
            loop_continue_idx_phi: Vec::new(),
            loop_continue_scope_depth: Vec::new(),
            loop_break_upsilons: Vec::new(),
            inst_vec_elem_type: HashMap::new(),
            struct_field_vec_elems,
            warnings: Vec::new(),
            alloc_scopes: vec![Vec::new()],
            escaped_allocs: HashSet::new(),
            loop_alloc_scope_depth: Vec::new(),
        }
    }

    pub(super) fn track_heap_alloc(&mut self, id: InstId, tag: &str) {
        self.alloc_scopes
            .last_mut()
            .expect("alloc_scopes must have at least one scope")
            .push((id, tag.to_string()));
    }

    pub(super) fn mark_escaped(&mut self, id: InstId) {
        self.escaped_allocs.insert(id);
    }

    pub(super) fn push_alloc_scope(&mut self) {
        self.alloc_scopes.push(Vec::new());
    }

    /// Pop the current alloc scope and emit frees for non-escaped allocations.
    /// `live_out` contains InstIds that flow out of this scope (e.g. via Upsilon)
    /// and must not be freed.
    pub(super) fn pop_alloc_scope_frees(&mut self, live_out: &[InstId], span: Span) {
        let scope = self.alloc_scopes.pop().expect("alloc_scopes underflow");
        for (id, tag) in &scope {
            if self.escaped_allocs.contains(id) || live_out.contains(id) {
                continue;
            }
            if let Some(size_str) = tag.strip_prefix("region:") {
                let size: u32 = size_str.parse().unwrap_or(0);
                self.emit(
                    Opcode::RegionFree,
                    Ty::Unit,
                    vec![*id],
                    InstData::AllocSize { size, align: 8 },
                    span,
                );
                continue;
            }
            let sym = match tag.as_str() {
                "String" => "__vow_string_free",
                "Vec" => "__vow_vec_free_val",
                "HashMap" => "__vow_map_free",
                _ => continue,
            };
            self.emit(
                Opcode::Call,
                Ty::Unit,
                vec![*id],
                InstData::CallExtern(sym.to_string()),
                span,
            );
        }
    }

    /// Emit frees for allocations from the innermost scope down to (and including)
    /// the scope at `target_depth`. Does NOT pop any scopes.
    /// Used by break/continue to free loop body temporaries before jumping.
    pub(super) fn emit_alloc_frees_to_depth(
        &mut self,
        target_depth: usize,
        live_out: &[InstId],
        span: Span,
    ) {
        let allocs: Vec<(InstId, String)> = self.alloc_scopes[target_depth..]
            .iter()
            .flat_map(|scope| scope.iter().cloned())
            .collect();
        for (id, tag) in allocs {
            if self.escaped_allocs.contains(&id) || live_out.contains(&id) {
                continue;
            }
            if let Some(size_str) = tag.strip_prefix("region:") {
                let size: u32 = size_str.parse().unwrap_or(0);
                self.emit(
                    Opcode::RegionFree,
                    Ty::Unit,
                    vec![id],
                    InstData::AllocSize { size, align: 8 },
                    span,
                );
                continue;
            }
            let sym = match tag.as_str() {
                "String" => "__vow_string_free",
                "Vec" => "__vow_vec_free_val",
                "HashMap" => "__vow_map_free",
                _ => continue,
            };
            self.emit(
                Opcode::Call,
                Ty::Unit,
                vec![id],
                InstData::CallExtern(sym.to_string()),
                span,
            );
        }
    }

    fn collect_return_sources(&self, return_val: InstId) -> HashSet<InstId> {
        let mut sources = HashSet::new();
        sources.insert(return_val);
        let mut worklist = vec![return_val];
        while let Some(val) = worklist.pop() {
            for block in &self.func.blocks {
                for inst in &block.insts {
                    if inst.opcode == Opcode::Upsilon
                        && let InstData::PhiTarget(target) = inst.data
                        && target == val
                        && let Some(&src) = inst.args.first()
                        && sources.insert(src)
                    {
                        worklist.push(src);
                    }
                }
            }
        }
        sources
    }

    pub(super) fn emit_return_frees(&mut self, return_val: InstId, span: Span) {
        let return_sources = self.collect_return_sources(return_val);
        let all_allocs: Vec<(InstId, String)> = self
            .alloc_scopes
            .iter()
            .flat_map(|scope| scope.iter().cloned())
            .collect();
        for (id, tag) in all_allocs {
            if self.escaped_allocs.contains(&id) || return_sources.contains(&id) {
                continue;
            }
            if let Some(size_str) = tag.strip_prefix("region:") {
                let size: u32 = size_str.parse().unwrap_or(0);
                self.emit(
                    Opcode::RegionFree,
                    Ty::Unit,
                    vec![id],
                    InstData::AllocSize { size, align: 8 },
                    span,
                );
                continue;
            }
            let sym = match tag.as_str() {
                "String" => "__vow_string_free",
                "Vec" => "__vow_vec_free_val",
                "HashMap" => "__vow_map_free",
                _ => continue,
            };
            self.emit(
                Opcode::Call,
                Ty::Unit,
                vec![id],
                InstData::CallExtern(sym.to_string()),
                span,
            );
        }
    }

    pub(super) fn intern_str(&mut self, s: &str) -> u32 {
        if let Some(&idx) = self.string_pool_index.get(s) {
            return idx;
        }
        let idx = self.string_pool.len() as u32;
        self.string_pool_index.insert(s.to_string(), idx);
        self.string_pool.push(s.to_string());
        idx
    }

    pub(super) fn push_scope(&mut self) {
        self.scope.push(HashMap::new());
    }

    pub(super) fn pop_scope(&mut self) {
        self.scope.pop();
    }

    pub(super) fn emit_string_free(&mut self, id: InstId, span: Span) {
        self.escaped_allocs.insert(id);
        self.emit(
            Opcode::Call,
            Ty::Unit,
            vec![id],
            InstData::CallExtern("__vow_string_free".to_string()),
            span,
        );
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

    pub(super) fn inst_ty(&self, id: InstId) -> Ty {
        self.inst_ty_cache.get(&id).copied().unwrap_or(Ty::Unit)
    }

    pub(super) fn lookup(&self, name: &str) -> Option<InstId> {
        for frame in self.scope.iter().rev() {
            if let Some(&id) = frame.get(name) {
                return Some(id);
            }
        }
        None
    }

    /// Look up a variable considering only scope frames up to (exclusive) `depth`.
    /// Used by `continue` to resolve loop-carried vars from the loop header scope,
    /// skipping any inner-scope shadows introduced in the loop body.
    pub(super) fn lookup_at_depth(&self, name: &str, depth: usize) -> Option<InstId> {
        for frame in self.scope[..depth].iter().rev() {
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
        offset: u32,
    ) -> VowId {
        let id = VowId(self.func.vows.len() as u32);
        self.func.vows.push(VowEntry {
            id,
            description,
            blame,
            bindings,
            file: self.file.clone(),
            offset,
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
        self.inst_ty_cache.insert(id, ty);
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

    fn warn(&mut self, message: String, span: Span) {
        self.warnings.push(vow_diag::Diagnostic {
            severity: vow_diag::Severity::Warning,
            code: vow_diag::ErrorCode::LoweringWarning,
            message,
            primary: vow_diag::SourceLocation {
                file: self.file.clone(),
                byte_offset: span.start,
                byte_len: span.len,
            },
            secondary: vec![],
            blame: vow_diag::Blame::None,
            hints: vec![],
        });
    }

    pub fn finish(self) -> (Function, Vec<String>, Vec<vow_diag::Diagnostic>) {
        (self.func, self.string_pool, self.warnings)
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
        ExprKind::Loop { body, .. } => {
            for s in &body.stmts {
                collect_assigned_in_stmt(s, seen, out);
            }
            if let Some(e) = &body.trailing_expr {
                collect_assigned_in_expr(e, seen, out);
            }
        }
        ExprKind::ForEach { body, .. } => {
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
        ExprKind::Match { arms, .. } => {
            for arm in arms {
                collect_assigned_in_expr(&arm.body, seen, out);
            }
        }
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
                ctx.track_heap_alloc(vow_str, "String");
                vow_str
            }
        },
        ExprKind::Ident(name) => {
            if let Some(&(val, ref ty)) = ctx.const_map.get(name.as_str()) {
                let (opcode, data) = if *ty == Ty::Bool {
                    (Opcode::ConstBool, InstData::ConstBool(val != 0))
                } else {
                    (Opcode::ConstI64, InstData::ConstI64(val))
                };
                return ctx.emit(opcode, *ty, vec![], data, span);
            }
            ctx.lookup(name)
                .unwrap_or_else(|| panic!("undefined variable: {name}"))
        }
        ExprKind::BinaryOp { op, lhs, rhs } => {
            // Short-circuit evaluation for && and ||
            if *op == BinOp::And || *op == BinOp::Or {
                let lhs_id = lower_expr(ctx, lhs);
                let rhs_block = ctx.new_block();
                let short_block = ctx.new_block();
                let merge_block = ctx.new_block();

                // For &&: if LHS false → short-circuit (false); else → evaluate RHS
                // For ||: if LHS true → short-circuit (true); else → evaluate RHS
                let (then_target, else_target) = if *op == BinOp::And {
                    (rhs_block, short_block)
                } else {
                    (short_block, rhs_block)
                };
                ctx.emit(
                    Opcode::Branch,
                    Ty::Unit,
                    vec![lhs_id],
                    InstData::BranchTargets {
                        then_block: then_target,
                        else_block: else_target,
                    },
                    span,
                );

                // RHS block: evaluate RHS, feed into Phi
                ctx.switch_to_block(rhs_block);
                ctx.push_alloc_scope();
                let rhs_id = lower_expr(ctx, rhs);
                ctx.alloc_scopes.pop();
                let rhs_upsilon = ctx.emit(
                    Opcode::Upsilon,
                    Ty::Unit,
                    vec![rhs_id],
                    InstData::PhiTarget(InstId(u32::MAX)),
                    span,
                );
                let rhs_upsilon_block = ctx.current_block;
                ctx.emit(
                    Opcode::Jump,
                    Ty::Unit,
                    vec![],
                    InstData::JumpTarget(merge_block),
                    span,
                );

                // Short-circuit block: produce constant false (&&) or true (||)
                ctx.switch_to_block(short_block);
                let short_val = ctx.emit(
                    Opcode::ConstBool,
                    Ty::Bool,
                    vec![],
                    InstData::ConstBool(*op == BinOp::Or),
                    span,
                );
                let short_upsilon = ctx.emit(
                    Opcode::Upsilon,
                    Ty::Unit,
                    vec![short_val],
                    InstData::PhiTarget(InstId(u32::MAX)),
                    span,
                );
                let short_upsilon_block = ctx.current_block;
                ctx.emit(
                    Opcode::Jump,
                    Ty::Unit,
                    vec![],
                    InstData::JumpTarget(merge_block),
                    span,
                );

                // Merge block: Phi collects the result
                ctx.switch_to_block(merge_block);
                let phi = ctx.emit(Opcode::Phi, Ty::Bool, vec![], InstData::None, span);
                backpatch_upsilon(ctx, rhs_upsilon_block, rhs_upsilon, phi);
                backpatch_upsilon(ctx, short_upsilon_block, short_upsilon, phi);

                return phi;
            }

            let lhs_id = lower_expr(ctx, lhs);
            let rhs_id = lower_expr(ctx, rhs);
            let lhs_is_str = ctx
                .string_exprs
                .contains(&(lhs.as_ref() as *const Expr as usize));
            let rhs_is_str = ctx
                .string_exprs
                .contains(&(rhs.as_ref() as *const Expr as usize));
            if (lhs_is_str || rhs_is_str) && (*op == BinOp::Eq || *op == BinOp::Ne) {
                let eq_result = ctx.emit(
                    Opcode::Call,
                    Ty::Bool,
                    vec![lhs_id, rhs_id],
                    InstData::CallExtern("__vow_string_eq".to_string()),
                    span,
                );
                if matches!(&lhs.kind, ExprKind::Lit(Lit::String(_))) {
                    ctx.emit_string_free(lhs_id, span);
                }
                if matches!(&rhs.kind, ExprKind::Lit(Lit::String(_))) {
                    ctx.emit_string_free(rhs_id, span);
                }
                if *op == BinOp::Ne {
                    ctx.emit(Opcode::Not, Ty::Bool, vec![eq_result], InstData::None, span)
                } else {
                    eq_result
                }
            } else {
                let operand_ty = ctx.inst_ty(lhs_id);
                let (opcode, ty) = binop_opcode(*op, &operand_ty);
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
                for &aid in &arg_ids {
                    ctx.mark_escaped(aid);
                }
                ctx.emit(
                    Opcode::Call,
                    ret_ty,
                    arg_ids,
                    InstData::CallTarget(fid),
                    span,
                )
            } else if let Some((sym, ret_ty)) = vow_debug_builtin_to_runtime(&callee_name) {
                ctx.emit(
                    Opcode::DebugCall,
                    ret_ty,
                    arg_ids,
                    InstData::CallExtern(sym.to_string()),
                    span,
                )
            } else if let Some((sym, ret_ty)) = vow_builtin_to_runtime(&callee_name) {
                for &aid in &arg_ids {
                    ctx.mark_escaped(aid);
                }
                let result = ctx.emit(
                    Opcode::Call,
                    ret_ty,
                    arg_ids,
                    InstData::CallExtern(sym.to_string()),
                    span,
                );
                if ret_ty == Ty::Ptr {
                    let tag = builtin_alloc_tag(sym);
                    if !tag.is_empty() {
                        ctx.track_heap_alloc(result, tag);
                    }
                }
                result
            } else {
                for &aid in &arg_ids {
                    ctx.mark_escaped(aid);
                }
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
            ctx.push_alloc_scope();
            let then_val = lower_block(ctx, then_branch);
            let then_terminated = ctx.is_terminated();
            let then_upsilon_block = ctx.current_block;
            // Capture mutation values from then-branch (or pre-if value if not modified).
            let then_mut_vals: Vec<InstId> = mutations
                .iter()
                .map(|(name, pre_id)| ctx.lookup(name).unwrap_or(*pre_id))
                .collect();
            // Free branch temporaries (keep result and transitive sources alive).
            if !then_terminated {
                let mut live_out: Vec<InstId> = then_mut_vals.clone();
                live_out.push(then_val);
                for src in ctx.collect_return_sources(then_val) {
                    live_out.push(src);
                }
                for mv in &then_mut_vals {
                    for src in ctx.collect_return_sources(*mv) {
                        live_out.push(src);
                    }
                }
                ctx.pop_alloc_scope_frees(&live_out, span);
            } else {
                ctx.alloc_scopes.pop();
            }
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
            ctx.push_alloc_scope();
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
            // Free branch temporaries (keep result and transitive sources alive).
            if !else_terminated {
                let mut live_out: Vec<InstId> = else_mut_vals.clone();
                live_out.push(else_val);
                for src in ctx.collect_return_sources(else_val) {
                    live_out.push(src);
                }
                for mv in &else_mut_vals {
                    for src in ctx.collect_return_sources(*mv) {
                        live_out.push(src);
                    }
                }
                ctx.pop_alloc_scope_frees(&live_out, span);
            } else {
                ctx.alloc_scopes.pop();
            }
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
                ctx.emit_return_frees(val, span);
                ctx.emit(Opcode::Return, Ty::Unit, vec![val], InstData::None, span)
            } else {
                let unit = ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span);
                if let Some(vow_block) = ctx.vow_block.clone() {
                    vow::lower_ensures(ctx, &vow_block, unit);
                }
                ctx.emit_return_frees(unit, span);
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
                    let struct_name = ctx
                        .inst_struct_type
                        .get(&ptr_id)
                        .cloned()
                        .unwrap_or_default();
                    if struct_name.is_empty() {
                        ctx.warn(
                            format!("FieldSet on untagged instruction %{}, field '{}' -- defaulting to index 0", ptr_id.0, field),
                            span,
                        );
                    }
                    let field_idx = if let Some(names) = ctx.struct_field_map.get(&struct_name) {
                        match names.iter().position(|n| n == field) {
                            Some(idx) => idx,
                            None => {
                                if !struct_name.is_empty() {
                                    ctx.warn(
                                        format!("field '{}' not found in struct '{}' -- defaulting to index 0", field, struct_name),
                                        span,
                                    );
                                }
                                0
                            }
                        }
                    } else {
                        if !struct_name.is_empty() {
                            ctx.warn(
                                format!("struct '{}' not registered -- field lookup defaulting to index 0", struct_name),
                                span,
                            );
                        }
                        0
                    } as u32;
                    ctx.mark_escaped(new_val);
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
                    ctx.mark_escaped(new_val);
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
            ctx.loop_exit_blocks.push(exit_block);
            ctx.loop_header_blocks.push(header_block);
            ctx.loop_continue_phis.push(phi_ids.clone());
            ctx.loop_continue_idx_phi.push(None);
            ctx.loop_continue_scope_depth.push(ctx.scope.len());
            ctx.loop_break_upsilons.push(None);
            ctx.push_alloc_scope();
            ctx.loop_alloc_scope_depth.push(ctx.alloc_scopes.len() - 1);
            lower_block(ctx, body);
            ctx.loop_alloc_scope_depth.pop();
            ctx.loop_break_upsilons.pop();
            ctx.loop_continue_scope_depth.pop();
            ctx.loop_continue_idx_phi.pop();
            ctx.loop_continue_phis.pop();
            ctx.loop_header_blocks.pop();
            ctx.loop_exit_blocks.pop();

            // Free loop-body temporaries before back-edge.
            // Include transitive sources so Phi/Upsilon aliases aren't freed.
            if !ctx.is_terminated() {
                let mut loop_live_out: Vec<InstId> = Vec::new();
                for (name, _) in &phi_ids {
                    if let Some(val) = ctx.lookup(name) {
                        for src in ctx.collect_return_sources(val) {
                            loop_live_out.push(src);
                        }
                    }
                }
                ctx.pop_alloc_scope_frees(&loop_live_out, span);
            } else {
                ctx.alloc_scopes.pop();
            }

            // Emit back-edge Upsilons with the current scope values.
            if !ctx.is_terminated() {
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
            }

            // Restore scope to Phi values so the exit block sees the loop-exit values.
            for (name, phi_id) in &phi_ids {
                ctx.assign(name, *phi_id);
            }

            // Exit block.
            ctx.switch_to_block(exit_block);
            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
        }
        ExprKind::ForEach {
            binding,
            iterable,
            body,
            vow: for_vow,
        } => {
            // Desugar: for <binding> in <iterable> { <body> }
            // into:    let iter = <iterable>; let len = iter.len(); let idx = 0;
            //          while idx < len { let <binding> = iter[idx]; <body>; idx = idx + 1; }

            let iter_id = lower_expr(ctx, iterable);
            ctx.inst_struct_type.insert(iter_id, "Vec".to_string());

            let len_id = ctx.emit(
                Opcode::Call,
                Ty::I64,
                vec![iter_id],
                InstData::CallExtern("__vow_vec_len".to_string()),
                span,
            );
            let idx_init = ctx.emit(
                Opcode::ConstI64,
                Ty::I64,
                vec![],
                InstData::ConstI64(0),
                span,
            );

            let mutated = collect_assigned_vars(body);
            let loop_vars: Vec<(String, InstId)> = mutated
                .into_iter()
                .filter_map(|name| ctx.lookup(&name).map(|id| (name, id)))
                .collect();

            let pre_header_block = ctx.current_block;
            let header_block = ctx.new_block();
            let body_block = ctx.new_block();
            let exit_block = ctx.new_block();

            // Pre-header: Upsilon for index
            let idx_up = ctx.emit(
                Opcode::Upsilon,
                Ty::I64,
                vec![idx_init],
                InstData::PhiTarget(InstId(u32::MAX)),
                span,
            );

            // Pre-header: Upsilons for user mutated vars
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

            // Header: Phi for index
            ctx.switch_to_block(header_block);
            let idx_phi = ctx.emit(Opcode::Phi, Ty::I64, vec![], InstData::None, span);
            backpatch_upsilon(ctx, pre_header_block, idx_up, idx_phi);

            // Header: Phi for user mutated vars
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

            // Update scope: rebind mutated vars to their Phis
            for (name, phi_id) in &phi_ids {
                ctx.assign(name, *phi_id);
            }

            // Lower vow invariant at top of header (before condition)
            if let Some(wv) = for_vow {
                vow::lower_invariant(ctx, wv);
            }

            // Condition: idx < len
            let cond_id = ctx.emit(
                Opcode::LtI64,
                Ty::Bool,
                vec![idx_phi, len_id],
                InstData::None,
                span,
            );
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

            // Body: get element and bind to loop variable
            ctx.switch_to_block(body_block);
            let elem_id = ctx.emit(
                Opcode::Call,
                Ty::I64,
                vec![iter_id, idx_phi],
                InstData::CallExtern("__vow_vec_get_val".to_string()),
                span,
            );
            if let Some(elem_name) = ctx.inst_vec_elem_type.get(&iter_id).cloned() {
                ctx.inst_struct_type.insert(elem_id, elem_name);
            }

            // Save scope depth before pushing the for-each binding scope.
            // Loop-carried phis track outer mutation variables whose bindings
            // live at this depth; the for-each binding is a new scope that must
            // be excluded from continue's lookup to avoid resolving to it.
            let for_scope_depth = ctx.scope.len();

            ctx.push_scope();
            ctx.define(binding.clone(), elem_id);

            ctx.loop_exit_blocks.push(exit_block);
            ctx.loop_header_blocks.push(header_block);
            ctx.loop_continue_phis.push(phi_ids.clone());
            ctx.loop_continue_idx_phi.push(Some(idx_phi));
            ctx.loop_continue_scope_depth.push(for_scope_depth);
            ctx.push_alloc_scope();
            ctx.loop_alloc_scope_depth.push(ctx.alloc_scopes.len() - 1);
            lower_block(ctx, body);
            ctx.loop_alloc_scope_depth.pop();
            ctx.loop_continue_scope_depth.pop();
            ctx.loop_continue_idx_phi.pop();
            ctx.loop_continue_phis.pop();
            ctx.loop_header_blocks.pop();
            ctx.loop_exit_blocks.pop();

            ctx.pop_scope();

            // Free loop-body temporaries before back-edge.
            // Include transitive sources so Phi/Upsilon aliases aren't freed.
            if !ctx.is_terminated() {
                let mut loop_live_out: Vec<InstId> = Vec::new();
                for (name, _) in &phi_ids {
                    if let Some(val) = ctx.lookup(name) {
                        for src in ctx.collect_return_sources(val) {
                            loop_live_out.push(src);
                        }
                    }
                }
                ctx.pop_alloc_scope_frees(&loop_live_out, span);
            } else {
                ctx.alloc_scopes.pop();
            }

            // Increment index and emit back-edge
            if !ctx.is_terminated() {
                let one = ctx.emit(
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(1),
                    span,
                );
                let idx_next = ctx.emit(
                    Opcode::WrappingAddI64,
                    Ty::I64,
                    vec![idx_phi, one],
                    InstData::None,
                    span,
                );
                ctx.emit(
                    Opcode::Upsilon,
                    Ty::I64,
                    vec![idx_next],
                    InstData::PhiTarget(idx_phi),
                    span,
                );
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
            }

            // Restore scope to Phi values for exit
            for (name, phi_id) in &phi_ids {
                ctx.assign(name, *phi_id);
            }

            ctx.switch_to_block(exit_block);
            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
        }
        ExprKind::Loop {
            body,
            vow: loop_vow,
        } => {
            let mutated = collect_assigned_vars(body);
            let loop_vars: Vec<(String, InstId)> = mutated
                .into_iter()
                .filter_map(|name| ctx.lookup(&name).map(|id| (name, id)))
                .collect();

            let pre_header_block = ctx.current_block;
            let header_block = ctx.new_block();
            let exit_block = ctx.new_block();

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
            for (name, phi_id) in &phi_ids {
                ctx.assign(name, *phi_id);
            }

            if let Some(lv) = loop_vow {
                vow::lower_invariant(ctx, lv);
            }

            ctx.loop_exit_blocks.push(exit_block);
            ctx.loop_header_blocks.push(header_block);
            ctx.loop_continue_phis.push(phi_ids.clone());
            ctx.loop_continue_idx_phi.push(None);
            ctx.loop_continue_scope_depth.push(ctx.scope.len());
            ctx.loop_break_upsilons.push(Some(Vec::new()));
            ctx.push_alloc_scope();
            ctx.loop_alloc_scope_depth.push(ctx.alloc_scopes.len() - 1);
            lower_block(ctx, body);
            ctx.loop_alloc_scope_depth.pop();
            let break_ups = ctx.loop_break_upsilons.pop().unwrap();
            ctx.loop_continue_scope_depth.pop();
            ctx.loop_continue_idx_phi.pop();
            ctx.loop_continue_phis.pop();
            ctx.loop_header_blocks.pop();
            ctx.loop_exit_blocks.pop();

            // Free loop-body temporaries before back-edge.
            if !ctx.is_terminated() {
                let loop_live_out: Vec<InstId> = phi_ids
                    .iter()
                    .filter_map(|(name, _)| ctx.lookup(name))
                    .collect();
                ctx.pop_alloc_scope_frees(&loop_live_out, span);
            } else {
                ctx.alloc_scopes.pop();
            }

            // Back-edge Upsilons
            if !ctx.is_terminated() {
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
            }

            for (name, phi_id) in &phi_ids {
                ctx.assign(name, *phi_id);
            }

            ctx.switch_to_block(exit_block);

            // If any break carried a value, emit a Phi to merge them.
            if let Some(ups) = break_ups {
                if ups.is_empty() {
                    ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                } else {
                    let ty = ups[0].2;
                    let phi_id = ctx.emit(Opcode::Phi, ty, vec![], InstData::None, span);
                    for (block, up_id, _) in &ups {
                        backpatch_upsilon(ctx, *block, *up_id, phi_id);
                    }
                    phi_id
                }
            } else {
                ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
            }
        }
        ExprKind::Break { value } => {
            let exit_block = ctx
                .loop_exit_blocks
                .last()
                .copied()
                .expect("break outside of loop");

            let break_val = if let Some(val_expr) = value {
                let val_id = lower_expr(ctx, val_expr);
                // If inside a `loop` (Some), emit Upsilon for the break-value Phi.
                let is_loop = matches!(ctx.loop_break_upsilons.last(), Some(Some(_)));
                if is_loop {
                    let val_ty = ctx.inst_ty(val_id);
                    let up_id = ctx.emit(
                        Opcode::Upsilon,
                        val_ty,
                        vec![val_id],
                        InstData::PhiTarget(InstId(u32::MAX)),
                        span,
                    );
                    let block = ctx.current_block;
                    if let Some(Some(ups)) = ctx.loop_break_upsilons.last_mut() {
                        ups.push((block, up_id, val_ty));
                    }
                }
                Some(val_id)
            } else {
                None
            };

            // Free loop body allocations before jumping to exit.
            // Use collect_return_sources to trace through Phi/Upsilon chains —
            // break_val may alias multiple heap allocations (e.g., from if-else).
            if let Some(&depth) = ctx.loop_alloc_scope_depth.last() {
                let live_out: Vec<InstId> = if let Some(bv) = break_val {
                    ctx.collect_return_sources(bv).into_iter().collect()
                } else {
                    vec![]
                };
                ctx.emit_alloc_frees_to_depth(depth, &live_out, span);
            }

            ctx.emit(
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(exit_block),
                span,
            )
        }
        ExprKind::Continue => {
            let header_block = ctx
                .loop_header_blocks
                .last()
                .copied()
                .expect("continue outside of loop");
            let phis = ctx.loop_continue_phis.last().cloned().unwrap_or_default();
            let idx_phi = ctx.loop_continue_idx_phi.last().copied().flatten();
            let scope_depth = ctx
                .loop_continue_scope_depth
                .last()
                .copied()
                .expect("continue outside of loop");

            // Free loop body allocations before jumping back to header.
            // Include transitive sources so Phi/Upsilon aliases aren't freed.
            if let Some(&depth) = ctx.loop_alloc_scope_depth.last() {
                let mut live_out: Vec<InstId> = Vec::new();
                for (name, _) in &phis {
                    if let Some(val) = ctx.lookup_at_depth(name, scope_depth) {
                        for src in ctx.collect_return_sources(val) {
                            live_out.push(src);
                        }
                    }
                }
                ctx.emit_alloc_frees_to_depth(depth, &live_out, span);
            }

            // Emit back-edge Upsilons for mutation variables.
            // Use lookup_at_depth to resolve from the loop header scope, not the
            // current scope, so that shadowed bindings in inner blocks are skipped.
            for (name, phi_id) in &phis {
                if let Some(cur_val) = ctx.lookup_at_depth(name, scope_depth) {
                    ctx.emit(
                        Opcode::Upsilon,
                        ctx.inst_ty(cur_val),
                        vec![cur_val],
                        InstData::PhiTarget(*phi_id),
                        span,
                    );
                }
            }

            // For for-each: increment index and emit Upsilon for index Phi.
            if let Some(ip) = idx_phi {
                let one = ctx.emit(
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(1),
                    span,
                );
                let idx_next = ctx.emit(
                    Opcode::WrappingAddI64,
                    Ty::I64,
                    vec![ip, one],
                    InstData::None,
                    span,
                );
                ctx.emit(
                    Opcode::Upsilon,
                    Ty::I64,
                    vec![idx_next],
                    InstData::PhiTarget(ip),
                    span,
                );
            }

            ctx.emit(
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(header_block),
                span,
            )
        }
        ExprKind::FieldAccess { base, field } => {
            let ptr_id = lower_expr(ctx, base);
            let struct_name = ctx
                .inst_struct_type
                .get(&ptr_id)
                .cloned()
                .unwrap_or_default();
            if struct_name.is_empty() {
                ctx.warn(
                    format!(
                        "FieldGet on untagged instruction %{}, field '{}' -- defaulting to index 0",
                        ptr_id.0, field
                    ),
                    span,
                );
            }
            let field_idx = if let Some(names) = ctx.struct_field_map.get(&struct_name) {
                match names.iter().position(|n| n == field) {
                    Some(idx) => idx,
                    None => {
                        if !struct_name.is_empty() {
                            ctx.warn(
                                format!(
                                    "field '{}' not found in struct '{}' -- defaulting to index 0",
                                    field, struct_name
                                ),
                                span,
                            );
                        }
                        0
                    }
                }
            } else {
                if !struct_name.is_empty() {
                    ctx.warn(
                        format!(
                            "struct '{}' not registered -- field lookup defaulting to index 0",
                            struct_name
                        ),
                        span,
                    );
                }
                0
            } as u32;
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
                && !matches!(type_name.as_str(), "i32" | "i64" | "f32" | "f64" | "bool")
            {
                ctx.inst_struct_type.insert(result_id, type_name.clone());
            }
            if let Some(vec_elems) = ctx.struct_field_vec_elems.get(&struct_name)
                && let Some(elem_name) = vec_elems.get(field_idx as usize)
                && !elem_name.is_empty()
            {
                ctx.inst_vec_elem_type.insert(result_id, elem_name.clone());
            }
            result_id
        }
        ExprKind::StructLiteral { name, fields } => {
            let field_names = if let Some(names) = ctx.struct_field_map.get(name) {
                names.clone()
            } else {
                ctx.warn(
                    format!(
                        "struct '{}' not registered -- field lookup defaulting to index 0",
                        name
                    ),
                    span,
                );
                vec![]
            };
            let n_fields = field_names.len().max(fields.len());
            let ptr_id = ctx.emit(
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize {
                    size: (n_fields as u32 + 1) * 8,
                    align: 8,
                },
                span,
            );
            ctx.inst_struct_type.insert(ptr_id, name.clone());
            let alloc_size = (n_fields as u32 + 1) * 8;
            ctx.track_heap_alloc(ptr_id, &format!("region:{alloc_size}"));
            for (field_name, field_expr) in fields {
                let idx = match field_names.iter().position(|n| n == field_name) {
                    Some(i) => i,
                    None => {
                        if !field_names.is_empty() {
                            ctx.warn(
                                format!("StructLiteral field '{}' not found in struct '{}' -- defaulting to index 0", field_name, name),
                                span,
                            );
                        }
                        0
                    }
                } as u32;
                let val_id = lower_expr(ctx, field_expr);
                ctx.mark_escaped(val_id);
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
            // String::new() builtin — empty string via __vow_vec_new(1, 1)
            if enum_name == "String" && variant_name == "new" {
                let size_val = ctx.emit(
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(1),
                    span,
                );
                let align_val = ctx.emit(
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(1),
                    span,
                );
                let result = ctx.emit(
                    Opcode::Call,
                    Ty::Ptr,
                    vec![size_val, align_val],
                    InstData::CallExtern("__vow_vec_new".to_string()),
                    span,
                );
                ctx.inst_struct_type.insert(result, "String".to_string());
                ctx.track_heap_alloc(result, "String");
                return result;
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
                ctx.track_heap_alloc(result, "HashMap");
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
                let result = ctx.emit(
                    Opcode::Call,
                    Ty::Ptr,
                    vec![size_val, align_val],
                    InstData::CallExtern("__vow_vec_new".to_string()),
                    span,
                );
                ctx.inst_struct_type.insert(result, "Vec".to_string());
                ctx.track_heap_alloc(result, "Vec");
                return result;
            }
            let tag = ctx
                .enum_variant_map
                .get(enum_name)
                .and_then(|vs| vs.iter().position(|v| v == variant_name))
                .unwrap_or(0) as i64;
            let n_payload = fields.len();
            let size = (2 + n_payload) as u32 * 8;
            let ptr_id = ctx.emit(
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size, align: 8 },
                span,
            );
            ctx.inst_struct_type.insert(ptr_id, enum_name.to_string());
            ctx.track_heap_alloc(ptr_id, &format!("region:{size}"));
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
                ctx.mark_escaped(val_id);
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
            let tag_id = ctx.emit(
                Opcode::FieldGet,
                Ty::I64,
                vec![ptr_id],
                InstData::FieldIndex(0),
                span,
            );

            let merge_block = ctx.new_block();

            // Collect mutations across all arm bodies.
            let mutations: Vec<(String, InstId)> = {
                let mut seen = HashSet::new();
                let mut names = vec![];
                for arm in arms {
                    collect_assigned_in_expr(&arm.body, &mut seen, &mut names);
                }
                names
                    .into_iter()
                    .filter_map(|name| ctx.lookup(&name).map(|id| (name, id)))
                    .collect()
            };

            let scope_snap = ctx.snapshot_scope();

            // Per-arm tracking: (exit_block, result_upsilon, result_ty, mut_vals)
            let mut arm_results: Vec<(BlockId, InstId, Ty, Vec<InstId>)> = Vec::new();

            let mut arm_iter = arms.iter().peekable();
            while let Some(arm) = arm_iter.next() {
                let is_last = arm_iter.peek().is_none();
                match &arm.pattern.kind {
                    PatKind::EnumVariant { path, inner } => {
                        let enum_name = path.first().map(|s| s.as_str()).unwrap_or("");
                        let variant_name = path.get(1).map(|s| s.as_str()).unwrap_or("");
                        let expected_tag = ctx
                            .enum_variant_map
                            .get(enum_name)
                            .and_then(|vs| vs.iter().position(|v| v == variant_name))
                            .unwrap_or(0) as i64;

                        let arm_block = ctx.new_block();
                        let next_check_block = if is_last { arm_block } else { ctx.new_block() };

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
                        ctx.push_alloc_scope();
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
                        let arm_terminated = ctx.is_terminated();
                        ctx.pop_scope();

                        let arm_mut_vals: Vec<InstId> = mutations
                            .iter()
                            .map(|(name, pre_id)| ctx.lookup(name).unwrap_or(*pre_id))
                            .collect();

                        // Free match arm temporaries (keep transitive sources alive).
                        if !arm_terminated {
                            let mut live_out: Vec<InstId> = arm_mut_vals.clone();
                            live_out.push(arm_result);
                            for src in ctx.collect_return_sources(arm_result) {
                                live_out.push(src);
                            }
                            for mv in &arm_mut_vals {
                                for src in ctx.collect_return_sources(*mv) {
                                    live_out.push(src);
                                }
                            }
                            ctx.pop_alloc_scope_frees(&live_out, span);
                        } else {
                            ctx.alloc_scopes.pop();
                        }

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
                        let exit_block = ctx.current_block;
                        arm_results.push((exit_block, up_id, arm_ty, arm_mut_vals));

                        ctx.restore_scope(scope_snap.clone());

                        if !is_last {
                            ctx.switch_to_block(next_check_block);
                        }
                    }
                    PatKind::Wildcard | PatKind::Ident { .. } => {
                        let arm_block = ctx.current_block;
                        if let PatKind::Ident { name, .. } = &arm.pattern.kind {
                            ctx.push_scope();
                            ctx.define(name.clone(), ptr_id);
                        } else {
                            ctx.push_scope();
                        }
                        ctx.push_alloc_scope();
                        let arm_result = lower_expr(ctx, &arm.body);
                        let arm_ty = ctx.inst_ty(arm_result);
                        let arm_terminated = ctx.is_terminated();
                        ctx.pop_scope();

                        let arm_mut_vals: Vec<InstId> = mutations
                            .iter()
                            .map(|(name, pre_id)| ctx.lookup(name).unwrap_or(*pre_id))
                            .collect();

                        // Free match arm temporaries (keep transitive sources alive).
                        if !arm_terminated {
                            let mut live_out: Vec<InstId> = arm_mut_vals.clone();
                            live_out.push(arm_result);
                            for src in ctx.collect_return_sources(arm_result) {
                                live_out.push(src);
                            }
                            for mv in &arm_mut_vals {
                                for src in ctx.collect_return_sources(*mv) {
                                    live_out.push(src);
                                }
                            }
                            ctx.pop_alloc_scope_frees(&live_out, span);
                        } else {
                            ctx.alloc_scopes.pop();
                        }

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
                        arm_results.push((arm_block, up_id, arm_ty, arm_mut_vals));

                        ctx.restore_scope(scope_snap.clone());
                    }
                    _ => {
                        let arm_block = ctx.current_block;
                        let unit =
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span);

                        let arm_mut_vals: Vec<InstId> =
                            mutations.iter().map(|(_, pre_id)| *pre_id).collect();

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
                        arm_results.push((arm_block, up_id, Ty::Unit, arm_mut_vals));
                    }
                }
            }

            ctx.restore_scope(scope_snap);
            ctx.switch_to_block(merge_block);

            // Create Phis for mutated variables.
            for (i, (name, pre_id)) in mutations.iter().enumerate() {
                let changed = arm_results.iter().any(|(_, _, _, mvs)| mvs[i] != *pre_id);
                if !changed {
                    continue;
                }
                let phi_ty = ctx.inst_ty(arm_results[0].3[i]);
                let phi_id = ctx.emit(Opcode::Phi, phi_ty, vec![], InstData::None, span);
                for (exit_block, _, _, arm_mut_vals) in &arm_results {
                    ctx.switch_to_block(*exit_block);
                    ctx.emit(
                        Opcode::Upsilon,
                        phi_ty,
                        vec![arm_mut_vals[i]],
                        InstData::PhiTarget(phi_id),
                        span,
                    );
                }
                ctx.switch_to_block(merge_block);
                ctx.assign(name, phi_id);
            }

            let phi_ty = arm_results
                .first()
                .map(|(_, _, ty, _)| *ty)
                .unwrap_or(Ty::I64);
            let phi_id = ctx.emit(Opcode::Phi, phi_ty, vec![], InstData::None, span);

            for (arm_block, up_id, _, _) in &arm_results {
                backpatch_upsilon(ctx, *arm_block, *up_id, phi_id);
            }

            phi_id
        }
        ExprKind::MethodCall {
            receiver,
            method,
            args,
        } => {
            let recv_id = lower_expr(ctx, receiver);
            let recv_struct = ctx.inst_struct_type.get(&recv_id).cloned().or_else(|| {
                if ctx
                    .string_exprs
                    .contains(&(receiver.as_ref() as *const Expr as usize))
                {
                    Some("String".to_string())
                } else {
                    None
                }
            });
            match (recv_struct.as_deref(), method.as_str()) {
                (Some("String"), "len") => ctx.emit(
                    Opcode::Call,
                    Ty::I64,
                    vec![recv_id],
                    InstData::CallExtern("__vow_string_len".to_string()),
                    span,
                ),
                (Some("String"), "push_str") => {
                    let arg_id = args.first().map(|e| lower_expr(ctx, e)).unwrap_or_else(|| {
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
                    let arg_id = args.first().map(|e| lower_expr(ctx, e)).unwrap_or_else(|| {
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
                (Some("String"), "contains") => {
                    let arg_expr = args.first();
                    let arg_id = arg_expr.map(|e| lower_expr(ctx, e)).unwrap_or_else(|| {
                        ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                    });
                    let result = ctx.emit(
                        Opcode::Call,
                        Ty::Bool,
                        vec![recv_id, arg_id],
                        InstData::CallExtern("__vow_string_contains".to_string()),
                        span,
                    );
                    if arg_expr.is_some_and(|e| matches!(&e.kind, ExprKind::Lit(Lit::String(_)))) {
                        ctx.emit_string_free(arg_id, span);
                    }
                    result
                }
                (Some("String"), "byte_at") => {
                    let idx_id = args.first().map(|e| lower_expr(ctx, e)).unwrap_or_else(|| {
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
                    let byte_id = args.first().map(|e| lower_expr(ctx, e)).unwrap_or_else(|| {
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
                (Some("String"), "clear") => ctx.emit(
                    Opcode::Call,
                    Ty::Unit,
                    vec![recv_id],
                    InstData::CallExtern("__vow_string_clear".to_string()),
                    span,
                ),
                (Some("String"), "substring") => {
                    let start_id = args.first().map(|e| lower_expr(ctx, e)).unwrap_or_else(|| {
                        ctx.emit(
                            Opcode::ConstI64,
                            Ty::I64,
                            vec![],
                            InstData::ConstI64(0),
                            span,
                        )
                    });
                    let end_id = args.get(1).map(|e| lower_expr(ctx, e)).unwrap_or_else(|| {
                        ctx.emit(
                            Opcode::ConstI64,
                            Ty::I64,
                            vec![],
                            InstData::ConstI64(0),
                            span,
                        )
                    });
                    let result = ctx.emit(
                        Opcode::Call,
                        Ty::Ptr,
                        vec![recv_id, start_id, end_id],
                        InstData::CallExtern("__vow_string_substring".to_string()),
                        span,
                    );
                    ctx.inst_struct_type.insert(result, "String".to_string());
                    result
                }
                (Some("String"), "parse_i64") => {
                    let result = ctx.emit(
                        Opcode::Call,
                        Ty::Ptr,
                        vec![recv_id],
                        InstData::CallExtern("__vow_string_parse_i64_opt".to_string()),
                        span,
                    );
                    ctx.inst_struct_type.insert(result, "Option".to_string());
                    result
                }
                (Some("String"), "parse_u64") => {
                    let result = ctx.emit(
                        Opcode::Call,
                        Ty::Ptr,
                        vec![recv_id],
                        InstData::CallExtern("__vow_string_parse_u64_opt".to_string()),
                        span,
                    );
                    ctx.inst_struct_type.insert(result, "Option".to_string());
                    result
                }
                (Some("HashMap"), "len") => ctx.emit(
                    Opcode::Call,
                    Ty::I64,
                    vec![recv_id],
                    InstData::CallExtern("__vow_map_len".to_string()),
                    span,
                ),
                (Some("HashMap"), "insert") => {
                    let k_id = args.first().map(|e| lower_expr(ctx, e)).unwrap_or_else(|| {
                        ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                    });
                    let v_id = args.get(1).map(|e| lower_expr(ctx, e)).unwrap_or_else(|| {
                        ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                    });
                    ctx.mark_escaped(k_id);
                    ctx.mark_escaped(v_id);
                    ctx.emit(
                        Opcode::Call,
                        Ty::Unit,
                        vec![recv_id, k_id, v_id],
                        InstData::CallExtern("__vow_map_insert".to_string()),
                        span,
                    )
                }
                (Some("HashMap"), "get") => {
                    let k_id = args.first().map(|e| lower_expr(ctx, e)).unwrap_or_else(|| {
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
                    let k_id = args.first().map(|e| lower_expr(ctx, e)).unwrap_or_else(|| {
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
                    let k_id = args.first().map(|e| lower_expr(ctx, e)).unwrap_or_else(|| {
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
                    let elem_id = args.first().map(|e| lower_expr(ctx, e)).unwrap_or_else(|| {
                        ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                    });
                    ctx.mark_escaped(elem_id);
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
                (_, "clear") => ctx.emit(
                    Opcode::Call,
                    Ty::Unit,
                    vec![recv_id],
                    InstData::CallExtern("__vow_vec_clear".to_string()),
                    span,
                ),
                (_, "truncate") => {
                    let len_id = args.first().map(|e| lower_expr(ctx, e)).unwrap_or_else(|| {
                        ctx.emit(
                            Opcode::ConstI64,
                            Ty::I64,
                            vec![],
                            InstData::ConstI64(0),
                            span,
                        )
                    });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Unit,
                        vec![recv_id, len_id],
                        InstData::CallExtern("__vow_vec_truncate".to_string()),
                        span,
                    )
                }
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
            let result = ctx.emit(
                Opcode::Call,
                Ty::I64,
                vec![vec_ptr, idx_id],
                InstData::CallExtern("__vow_vec_get_val".to_string()),
                span,
            );
            if let Some(elem_name) = ctx.inst_vec_elem_type.get(&vec_ptr).cloned() {
                ctx.inst_struct_type.insert(result, elem_name);
            }
            result
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
            let none_size: u32 = 16; // discriminant + guard slot
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
            ctx.emit_return_frees(none_ptr, span);
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
        ExprKind::Cast { expr, target_ty } => {
            let val = lower_expr(ctx, expr);
            let src_ty = ctx.inst_ty(val);
            let tgt = lower_ty(target_ty);
            match (src_ty, tgt) {
                (Ty::I64, Ty::U64) | (Ty::I32, Ty::U64) => {
                    // If the source is a literal, emit ConstU64 directly
                    if let ExprKind::Lit(Lit::Int(v)) = &expr.kind {
                        ctx.emit(
                            Opcode::ConstU64,
                            Ty::U64,
                            vec![],
                            InstData::ConstU64(*v as u64),
                            span,
                        )
                    } else {
                        ctx.emit(
                            Opcode::CastI64ToU64,
                            Ty::U64,
                            vec![val],
                            InstData::None,
                            span,
                        )
                    }
                }
                (Ty::U64, Ty::I64) => ctx.emit(
                    Opcode::CastU64ToI64,
                    Ty::I64,
                    vec![val],
                    InstData::None,
                    span,
                ),
                _ => val,
            }
        }
        _ => todo!("IR lowering not implemented for {:?}", expr.kind),
    }
}

fn binop_opcode(op: BinOp, operand_ty: &Ty) -> (Opcode, Ty) {
    let is_u64 = *operand_ty == Ty::U64;
    match op {
        BinOp::Add => {
            if is_u64 {
                (Opcode::WrappingAddU64, Ty::U64)
            } else {
                (Opcode::WrappingAddI64, Ty::I64)
            }
        }
        BinOp::Sub => {
            if is_u64 {
                (Opcode::WrappingSubU64, Ty::U64)
            } else {
                (Opcode::WrappingSubI64, Ty::I64)
            }
        }
        BinOp::Mul => {
            if is_u64 {
                (Opcode::WrappingMulU64, Ty::U64)
            } else {
                (Opcode::WrappingMulI64, Ty::I64)
            }
        }
        BinOp::Div => {
            if is_u64 {
                (Opcode::WrappingDivU64, Ty::U64)
            } else {
                (Opcode::WrappingDivI64, Ty::I64)
            }
        }
        BinOp::Rem => {
            if is_u64 {
                (Opcode::WrappingRemU64, Ty::U64)
            } else {
                (Opcode::WrappingRemI64, Ty::I64)
            }
        }
        BinOp::AddChecked => {
            if is_u64 {
                (Opcode::CheckedAddU64, Ty::U64)
            } else {
                (Opcode::CheckedAddI64, Ty::I64)
            }
        }
        BinOp::SubChecked => {
            if is_u64 {
                (Opcode::CheckedSubU64, Ty::U64)
            } else {
                (Opcode::CheckedSubI64, Ty::I64)
            }
        }
        BinOp::MulChecked => {
            if is_u64 {
                (Opcode::CheckedMulU64, Ty::U64)
            } else {
                (Opcode::CheckedMulI64, Ty::I64)
            }
        }
        BinOp::DivChecked => {
            if is_u64 {
                (Opcode::CheckedDivU64, Ty::U64)
            } else {
                (Opcode::CheckedDivI64, Ty::I64)
            }
        }
        BinOp::RemChecked => {
            if is_u64 {
                (Opcode::CheckedRemU64, Ty::U64)
            } else {
                (Opcode::CheckedRemI64, Ty::I64)
            }
        }
        BinOp::Eq => {
            if is_u64 {
                (Opcode::EqU64, Ty::Bool)
            } else {
                (Opcode::EqI64, Ty::Bool)
            }
        }
        BinOp::Ne => {
            if is_u64 {
                (Opcode::NeU64, Ty::Bool)
            } else {
                (Opcode::NeI64, Ty::Bool)
            }
        }
        BinOp::Lt => {
            if is_u64 {
                (Opcode::LtU64, Ty::Bool)
            } else {
                (Opcode::LtI64, Ty::Bool)
            }
        }
        BinOp::Le => {
            if is_u64 {
                (Opcode::LeU64, Ty::Bool)
            } else {
                (Opcode::LeI64, Ty::Bool)
            }
        }
        BinOp::Gt => {
            if is_u64 {
                (Opcode::GtU64, Ty::Bool)
            } else {
                (Opcode::GtI64, Ty::Bool)
            }
        }
        BinOp::Ge => {
            if is_u64 {
                (Opcode::GeU64, Ty::Bool)
            } else {
                (Opcode::GeI64, Ty::Bool)
            }
        }
        BinOp::And => (Opcode::And, Ty::Bool),
        BinOp::Or => (Opcode::Or, Ty::Bool),
        BinOp::BitAnd => {
            if is_u64 {
                (Opcode::BitAndU64, Ty::U64)
            } else {
                (Opcode::BitAndI64, Ty::I64)
            }
        }
        BinOp::BitOr => {
            if is_u64 {
                (Opcode::BitOrU64, Ty::U64)
            } else {
                (Opcode::BitOrI64, Ty::I64)
            }
        }
        BinOp::BitXor => {
            if is_u64 {
                (Opcode::XorU64, Ty::U64)
            } else {
                (Opcode::XorI64, Ty::I64)
            }
        }
        BinOp::Shl => {
            if is_u64 {
                (Opcode::ShlU64, Ty::U64)
            } else {
                (Opcode::ShlI64, Ty::I64)
            }
        }
        BinOp::Shr => {
            if is_u64 {
                (Opcode::ShrU64, Ty::U64)
            } else {
                (Opcode::ShrI64, Ty::I64)
            }
        }
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
        Stmt::Let {
            pattern, init, ty, ..
        } => {
            let mut val = lower_expr(ctx, init);
            let span = init.span;
            if let Some(AstType::Named {
                name: type_name, ..
            }) = ty
                && type_name == "u64"
                && ctx.inst_ty(val) != Ty::U64
            {
                val = ctx.emit(
                    Opcode::CastI64ToU64,
                    Ty::U64,
                    vec![val],
                    InstData::None,
                    span,
                );
            }
            if let PatKind::Ident { name, .. } = &pattern.kind {
                if let Some(ann) = ty {
                    match ann {
                        AstType::Named {
                            name: type_name, ..
                        } => match type_name.as_str() {
                            "i32" | "i64" | "u64" | "f32" | "f64" | "bool" => {}
                            _ => {
                                ctx.inst_struct_type.insert(val, type_name.clone());
                            }
                        },
                        AstType::Generic {
                            name: type_name,
                            args,
                            ..
                        } => {
                            ctx.inst_struct_type.insert(val, type_name.clone());
                            if type_name == "Vec"
                                && let Some(AstType::Named {
                                    name: elem_name, ..
                                }) = args.first()
                                && !matches!(
                                    elem_name.as_str(),
                                    "i32" | "i64" | "u64" | "f32" | "f64" | "bool"
                                )
                            {
                                ctx.inst_vec_elem_type.insert(val, elem_name.clone());
                            }
                        }
                        _ => {}
                    }
                }
                ctx.func.local_names.insert(val.0, name.clone());
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

#[allow(clippy::too_many_arguments)]
pub fn lower_function(
    fn_def: &FnDef,
    file: &str,
    func_index: &HashMap<String, (FuncId, Ty)>,
    struct_field_map: HashMap<String, Vec<String>>,
    enum_variant_map: HashMap<String, Vec<String>>,
    struct_field_type_names: HashMap<String, Vec<String>>,
    struct_field_vec_elems: HashMap<String, Vec<String>>,
    string_exprs: &StringExprSet,
    const_map: &HashMap<String, (i64, Ty)>,
) -> (Function, Vec<String>, Vec<vow_diag::Diagnostic>) {
    let params: Vec<Ty> = fn_def.params.iter().map(|p| lower_ty(&p.ty)).collect();
    let param_names: Vec<String> = fn_def.params.iter().map(|p| p.name.clone()).collect();
    let return_ty = lower_ty(&fn_def.return_ty);
    let effects = fn_def.effects.clone();

    let mut ctx = LowerCtx::new(
        fn_def.name.clone(),
        params.clone(),
        param_names,
        return_ty,
        effects,
        file.to_string(),
        func_index.clone(),
        struct_field_map,
        enum_variant_map,
        struct_field_type_names,
        struct_field_vec_elems,
        string_exprs.clone(),
    );

    ctx.const_map = const_map.clone();

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
            AstType::Generic { name, args, .. } if name == "Vec" => {
                ctx.inst_struct_type.insert(arg_id, "Vec".to_string());
                if let Some(AstType::Named {
                    name: elem_name, ..
                }) = args.first()
                    && !matches!(
                        elem_name.as_str(),
                        "i32" | "i64" | "u64" | "f32" | "f64" | "bool"
                    )
                {
                    ctx.inst_vec_elem_type.insert(arg_id, elem_name.clone());
                }
            }
            AstType::Named { name, .. } if ctx.struct_field_map.contains_key(name.as_str()) => {
                ctx.inst_struct_type.insert(arg_id, name.clone());
            }
            _ => {}
        }
        ctx.define(param.name.clone(), arg_id);
    }

    vow::lower_param_refinements(&mut ctx, &fn_def.params);

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
        ctx.emit_return_frees(trailing, span);
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

pub fn lower_module(module: &AstModule, file: &str, string_exprs: &StringExprSet) -> Module {
    let fn_items: Vec<&FnDef> = module
        .items
        .iter()
        .filter_map(|item| {
            if let Item::Fn(fn_def) = item
                && !fn_def.is_declaration
            {
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

    // Collect const declarations
    let mut const_map: HashMap<String, (i64, Ty)> = HashMap::new();
    for item in &module.items {
        if let Item::Const(c) = item {
            let val = match &c.value.kind {
                ExprKind::Lit(Lit::Int(v)) => *v as i64,
                ExprKind::Lit(Lit::Bool(b)) => *b as i64,
                ExprKind::UnaryOp {
                    op: UnOp::Neg,
                    operand,
                } => {
                    if let ExprKind::Lit(Lit::Int(v)) = &operand.kind {
                        -(*v as i64)
                    } else {
                        0
                    }
                }
                _ => 0,
            };
            let ty = lower_ty(&c.ty);
            const_map.insert(c.name.clone(), (val, ty));
        }
    }

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
    // struct name → per-field Vec element type name (empty if not Vec<Named>)
    let mut struct_field_vec_elems: HashMap<String, Vec<String>> = HashMap::new();
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
            let vec_elems: Vec<String> = s
                .fields
                .iter()
                .map(|f| match &f.ty {
                    AstType::Generic { name, args, .. } if name == "Vec" => {
                        if let Some(AstType::Named {
                            name: elem_name, ..
                        }) = args.first()
                            && !matches!(
                                elem_name.as_str(),
                                "i32" | "i64" | "u64" | "f32" | "f64" | "bool"
                            )
                        {
                            return elem_name.clone();
                        }
                        String::new()
                    }
                    _ => String::new(),
                })
                .collect();
            struct_field_type_names.insert(s.name.clone(), type_names);
            struct_field_vec_elems.insert(s.name.clone(), vec_elems);
        }
    }

    // Build enum layout info
    let mut enum_variant_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut enum_layouts: Vec<EnumLayout> = Vec::new();
    for item in &module.items {
        if let Item::Enum(e) = item {
            let variant_names: Vec<String> = e.variants.iter().map(|v| v.name.clone()).collect();
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
    let mut all_warnings: Vec<vow_diag::Diagnostic> = Vec::new();
    let functions: Vec<Function> = fn_items
        .iter()
        .enumerate()
        .map(|(idx, fn_def)| {
            let (mut func, pool, func_warnings) = lower_function(
                fn_def,
                file,
                &func_index,
                struct_field_map.clone(),
                enum_variant_map.clone(),
                struct_field_type_names.clone(),
                struct_field_vec_elems.clone(),
                string_exprs,
                &const_map,
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
            all_warnings.extend(func_warnings);
            func
        })
        .collect();

    Module {
        name: module.name.clone(),
        strings: all_strings,
        struct_layouts,
        enum_layouts,
        functions,
        warnings: all_warnings,
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
            is_declaration: false,
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
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

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
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

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
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

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
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

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
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

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
            is_declaration: false,
        };
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

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
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

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

    #[test]
    fn continue_in_while_emits_jump_to_header() {
        // fn f() { let mut i = 0; while i < 10 { i = i + 1; if i == 5 { continue; } } }
        let let_i = Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: "i".to_string(),
                    is_mut: true,
                },
                span: sp(),
            },
            ty: None,
            init: Box::new(int_expr(0)),
            span: sp(),
        };

        // i = i + 1
        let incr = Stmt::Expr {
            expr: Expr {
                kind: ExprKind::Assign {
                    lhs: Box::new(ident_expr("i")),
                    rhs: Box::new(Expr {
                        kind: ExprKind::BinaryOp {
                            op: BinOp::Add,
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

        // if i == 5 { continue; }
        let if_continue = Stmt::Expr {
            expr: Expr {
                kind: ExprKind::If {
                    condition: Box::new(Expr {
                        kind: ExprKind::BinaryOp {
                            op: BinOp::Eq,
                            lhs: Box::new(ident_expr("i")),
                            rhs: Box::new(int_expr(5)),
                        },
                        span: sp(),
                    }),
                    then_branch: Box::new(Block {
                        stmts: vec![Stmt::Expr {
                            expr: Expr {
                                kind: ExprKind::Continue,
                                span: sp(),
                            },
                            has_semicolon: true,
                            span: sp(),
                        }],
                        trailing_expr: None,
                        span: sp(),
                    }),
                    else_branch: None,
                },
                span: sp(),
            },
            has_semicolon: true,
            span: sp(),
        };

        let while_body = Block {
            stmts: vec![incr, if_continue],
            trailing_expr: None,
            span: sp(),
        };

        let while_expr = Expr {
            kind: ExprKind::While {
                condition: Box::new(Expr {
                    kind: ExprKind::BinaryOp {
                        op: BinOp::Lt,
                        lhs: Box::new(ident_expr("i")),
                        rhs: Box::new(int_expr(10)),
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
            trailing_expr: None,
            span: sp(),
        };

        let fn_def = make_fn("f", vec![], unit_ty(), body, vec![]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();

        // continue produces an extra Jump to the header block (3 total: pre-header→header,
        // continue→header, end-of-body→header)
        let jumps: Vec<_> = all_insts
            .iter()
            .filter(|i| i.opcode == Opcode::Jump)
            .collect();
        assert!(
            jumps.len() >= 3,
            "expected at least 3 Jumps (pre-header, continue, back-edge), got {}",
            jumps.len()
        );

        // continue also produces Upsilons for the mutation variable before the jump
        let upsilons: Vec<_> = all_insts
            .iter()
            .filter(|i| i.opcode == Opcode::Upsilon)
            .collect();
        assert!(
            upsilons.len() >= 3,
            "expected at least 3 Upsilons (pre-header, continue, back-edge), got {}",
            upsilons.len()
        );
    }

    #[test]
    fn struct_alloc_includes_guard_slot() {
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(Expr {
                kind: ExprKind::StructLiteral {
                    name: "Point".to_string(),
                    fields: vec![
                        ("x".to_string(), int_expr(1)),
                        ("y".to_string(), int_expr(2)),
                        ("z".to_string(), int_expr(3)),
                    ],
                },
                span: sp(),
            })),
            span: sp(),
        };
        let fn_def = make_fn("make_point", vec![], i64_ty(), body, vec![]);
        let mut sfm = HashMap::new();
        sfm.insert(
            "Point".to_string(),
            vec!["x".to_string(), "y".to_string(), "z".to_string()],
        );
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            sfm,
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );
        let alloc = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .find(|i| i.opcode == Opcode::RegionAlloc)
            .expect("expected RegionAlloc");
        // 3 fields + 1 guard = 4 slots * 8 bytes = 32
        assert_eq!(alloc.data, InstData::AllocSize { size: 32, align: 8 });
    }

    #[test]
    fn enum_alloc_includes_guard_slot() {
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(Expr {
                kind: ExprKind::EnumConstruct {
                    path: vec!["Option".to_string(), "Some".to_string()],
                    fields: vec![int_expr(42)],
                },
                span: sp(),
            })),
            span: sp(),
        };
        let fn_def = make_fn("make_some", vec![], i64_ty(), body, vec![]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );
        let alloc = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .find(|i| i.opcode == Opcode::RegionAlloc)
            .expect("expected RegionAlloc");
        // 1 discriminant + 1 payload + 1 guard = 3 slots * 8 bytes = 24
        assert_eq!(alloc.data, InstData::AllocSize { size: 24, align: 8 });
    }

    // --- Deallocation tests ---

    fn pair_ty() -> Type {
        Type::Named {
            name: "Pair".to_string(),
            span: sp(),
        }
    }

    fn let_stmt(name: &str, ty: Option<Type>, init: Expr) -> Stmt {
        Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: name.to_string(),
                    is_mut: false,
                },
                span: sp(),
            },
            ty,
            init: Box::new(init),
            span: sp(),
        }
    }

    fn pair_literal(a: i128, b: i128) -> Expr {
        Expr {
            kind: ExprKind::StructLiteral {
                name: "Pair".to_string(),
                fields: vec![
                    ("a".to_string(), int_expr(a)),
                    ("b".to_string(), int_expr(b)),
                ],
            },
            span: sp(),
        }
    }

    fn lower_with_structs(fn_def: &FnDef, fields: Vec<&str>) -> Function {
        let mut sfm = HashMap::new();
        sfm.insert(
            "Pair".to_string(),
            fields.into_iter().map(|s| s.to_string()).collect(),
        );
        let (func, _, _) = lower_function(
            fn_def,
            "",
            &HashMap::new(),
            sfm,
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );
        func
    }

    #[test]
    fn region_free_emitted_for_unused_struct() {
        // fn f() -> i64 { let p = Pair{a:1, b:2}; 42 }
        let body = Block {
            stmts: vec![let_stmt("p", Some(pair_ty()), pair_literal(1, 2))],
            trailing_expr: Some(Box::new(int_expr(42))),
            span: sp(),
        };
        let fn_def = make_fn("f", vec![], i64_ty(), body, vec![]);
        let func = lower_with_structs(&fn_def, vec!["a", "b"]);

        let has_region_free = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .any(|i| i.opcode == Opcode::RegionFree);
        assert!(has_region_free, "expected RegionFree for unused struct");
    }

    #[test]
    fn no_region_free_for_returned_struct() {
        // fn f() -> Pair { Pair{a:1, b:2} }
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(pair_literal(1, 2))),
            span: sp(),
        };
        let fn_def = make_fn("f", vec![], pair_ty(), body, vec![]);
        let func = lower_with_structs(&fn_def, vec!["a", "b"]);

        let has_region_free = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .any(|i| i.opcode == Opcode::RegionFree);
        assert!(!has_region_free, "returned struct should not be freed");
    }

    #[test]
    fn no_region_free_for_phi_returned_struct() {
        // fn f(flag: bool) -> Pair {
        //   let p = Pair{a:1, b:2};
        //   if flag { p } else { Pair{a:3, b:4} }
        // }
        let body = Block {
            stmts: vec![let_stmt("p", Some(pair_ty()), pair_literal(1, 2))],
            trailing_expr: Some(Box::new(Expr {
                kind: ExprKind::If {
                    condition: Box::new(ident_expr("flag")),
                    then_branch: Box::new(Block {
                        stmts: vec![],
                        trailing_expr: Some(Box::new(ident_expr("p"))),
                        span: sp(),
                    }),
                    else_branch: Some(Box::new(Expr {
                        kind: ExprKind::Block(Box::new(Block {
                            stmts: vec![],
                            trailing_expr: Some(Box::new(pair_literal(3, 4))),
                            span: sp(),
                        })),
                        span: sp(),
                    })),
                },
                span: sp(),
            })),
            span: sp(),
        };
        let bool_ty = Type::Named {
            name: "bool".to_string(),
            span: sp(),
        };
        let fn_def = make_fn(
            "f",
            vec![make_param("flag", bool_ty)],
            pair_ty(),
            body,
            vec![],
        );
        let func = lower_with_structs(&fn_def, vec!["a", "b"]);

        let region_frees: Vec<_> = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .filter(|i| i.opcode == Opcode::RegionFree)
            .collect();
        assert!(
            region_frees.is_empty(),
            "struct returned through Phi should not be freed, found {} RegionFree(s)",
            region_frees.len()
        );
    }

    #[test]
    fn no_free_for_escaped_struct() {
        // fn f(sink: fn(Pair)->i64) -> i64 { let p = Pair{a:1, b:2}; sink(p) }
        let body = Block {
            stmts: vec![let_stmt("p", Some(pair_ty()), pair_literal(1, 2))],
            trailing_expr: Some(Box::new(Expr {
                kind: ExprKind::Call {
                    callee: Box::new(ident_expr("sink")),
                    args: vec![ident_expr("p")],
                },
                span: sp(),
            })),
            span: sp(),
        };
        let fn_def = make_fn(
            "f",
            vec![make_param("sink", i64_ty())],
            i64_ty(),
            body,
            vec![],
        );
        let func = lower_with_structs(&fn_def, vec!["a", "b"]);

        let has_region_free = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .any(|i| i.opcode == Opcode::RegionFree);
        assert!(!has_region_free, "escaped struct should not be freed");
    }

    #[test]
    fn string_free_emitted_for_unused_string() {
        // fn f() -> i64 { let s = String::from("hello"); 42 }
        let body = Block {
            stmts: vec![let_stmt(
                "s",
                Some(Type::Named {
                    name: "String".to_string(),
                    span: sp(),
                }),
                Expr {
                    kind: ExprKind::EnumConstruct {
                        path: vec!["String".to_string(), "from".to_string()],
                        fields: vec![Expr {
                            kind: ExprKind::Lit(Lit::String("hello".to_string())),
                            span: sp(),
                        }],
                    },
                    span: sp(),
                },
            )],
            trailing_expr: Some(Box::new(int_expr(42))),
            span: sp(),
        };
        let fn_def = make_fn("f", vec![], i64_ty(), body, vec![]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let has_string_free = func.blocks.iter().flat_map(|b| b.insts.iter()).any(|i| {
            i.opcode == Opcode::Call
                && i.data == InstData::CallExtern("__vow_string_free".to_string())
        });
        assert!(
            has_string_free,
            "expected __vow_string_free for unused string"
        );
    }

    #[test]
    fn region_free_has_correct_size() {
        // Verify RegionFree carries the same AllocSize as RegionAlloc
        let body = Block {
            stmts: vec![let_stmt("p", Some(pair_ty()), pair_literal(1, 2))],
            trailing_expr: Some(Box::new(int_expr(0))),
            span: sp(),
        };
        let fn_def = make_fn("f", vec![], i64_ty(), body, vec![]);
        let func = lower_with_structs(&fn_def, vec!["a", "b"]);

        let alloc_data = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .find(|i| i.opcode == Opcode::RegionAlloc)
            .map(|i| i.data.clone())
            .expect("expected RegionAlloc");
        let free_data = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .find(|i| i.opcode == Opcode::RegionFree)
            .map(|i| i.data.clone())
            .expect("expected RegionFree");
        assert_eq!(
            alloc_data, free_data,
            "RegionFree size must match RegionAlloc size"
        );
    }

    #[test]
    fn no_region_free_for_struct_returned_through_phi() {
        // fn f(flag: bool) -> Pair { if flag { Pair{a:1,b:2} } else { Pair{a:3,b:4} } }
        // Both allocations feed the return Phi — neither should be freed
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(Expr {
                kind: ExprKind::If {
                    condition: Box::new(ident_expr("flag")),
                    then_branch: Box::new(Block {
                        stmts: vec![],
                        trailing_expr: Some(Box::new(pair_literal(1, 2))),
                        span: sp(),
                    }),
                    else_branch: Some(Box::new(Expr {
                        kind: ExprKind::Block(Box::new(Block {
                            stmts: vec![],
                            trailing_expr: Some(Box::new(pair_literal(3, 4))),
                            span: sp(),
                        })),
                        span: sp(),
                    })),
                },
                span: sp(),
            })),
            span: sp(),
        };
        let bool_ty = Type::Named {
            name: "bool".to_string(),
            span: sp(),
        };
        let fn_def = make_fn(
            "f",
            vec![make_param("flag", bool_ty)],
            pair_ty(),
            body,
            vec![],
        );
        let func = lower_with_structs(&fn_def, vec!["a", "b"]);

        let region_frees: Vec<_> = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .filter(|i| i.opcode == Opcode::RegionFree)
            .collect();
        assert!(
            region_frees.is_empty(),
            "struct returned directly through Phi should not be freed, found {} RegionFree(s)",
            region_frees.len()
        );
    }

    #[test]
    fn receiver_method_does_not_prevent_free() {
        // fn f() -> i64 { let s = String::from("x"); s.len() }
        // s.len() is read-only — s must still be freed (#71)
        let body = Block {
            stmts: vec![let_stmt(
                "s",
                Some(Type::Named {
                    name: "String".to_string(),
                    span: sp(),
                }),
                Expr {
                    kind: ExprKind::EnumConstruct {
                        path: vec!["String".to_string(), "from".to_string()],
                        fields: vec![Expr {
                            kind: ExprKind::Lit(Lit::String("x".to_string())),
                            span: sp(),
                        }],
                    },
                    span: sp(),
                },
            )],
            trailing_expr: Some(Box::new(Expr {
                kind: ExprKind::MethodCall {
                    receiver: Box::new(ident_expr("s")),
                    method: "len".to_string(),
                    args: vec![],
                },
                span: sp(),
            })),
            span: sp(),
        };
        let fn_def = make_fn("f", vec![], i64_ty(), body, vec![]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let has_string_free = func.blocks.iter().flat_map(|b| b.insts.iter()).any(|i| {
            i.opcode == Opcode::Call
                && i.data == InstData::CallExtern("__vow_string_free".to_string())
        });
        assert!(
            has_string_free,
            "method call on receiver must not prevent deallocation (#71)"
        );
    }

    #[test]
    fn nested_phi_struct_not_freed() {
        // fn f(a: bool, b: bool) -> Pair {
        //   let p = Pair{a:1,b:2};
        //   if a { if b { p } else { Pair{a:3,b:4} } } else { Pair{a:5,b:6} }
        // }
        // p flows through nested Phi chain → must not be freed
        let inner_if = Expr {
            kind: ExprKind::If {
                condition: Box::new(ident_expr("b")),
                then_branch: Box::new(Block {
                    stmts: vec![],
                    trailing_expr: Some(Box::new(ident_expr("p"))),
                    span: sp(),
                }),
                else_branch: Some(Box::new(Expr {
                    kind: ExprKind::Block(Box::new(Block {
                        stmts: vec![],
                        trailing_expr: Some(Box::new(pair_literal(3, 4))),
                        span: sp(),
                    })),
                    span: sp(),
                })),
            },
            span: sp(),
        };
        let body = Block {
            stmts: vec![let_stmt("p", Some(pair_ty()), pair_literal(1, 2))],
            trailing_expr: Some(Box::new(Expr {
                kind: ExprKind::If {
                    condition: Box::new(ident_expr("a")),
                    then_branch: Box::new(Block {
                        stmts: vec![],
                        trailing_expr: Some(Box::new(inner_if)),
                        span: sp(),
                    }),
                    else_branch: Some(Box::new(Expr {
                        kind: ExprKind::Block(Box::new(Block {
                            stmts: vec![],
                            trailing_expr: Some(Box::new(pair_literal(5, 6))),
                            span: sp(),
                        })),
                        span: sp(),
                    })),
                },
                span: sp(),
            })),
            span: sp(),
        };
        let bool_ty = Type::Named {
            name: "bool".to_string(),
            span: sp(),
        };
        let fn_def = make_fn(
            "f",
            vec![make_param("a", bool_ty.clone()), make_param("b", bool_ty)],
            pair_ty(),
            body,
            vec![],
        );
        let func = lower_with_structs(&fn_def, vec!["a", "b"]);

        let region_frees: Vec<_> = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .filter(|i| i.opcode == Opcode::RegionFree)
            .collect();
        assert!(
            region_frees.is_empty(),
            "struct reachable through nested Phi chain should not be freed, found {} RegionFree(s)",
            region_frees.len()
        );
    }

    #[test]
    fn string_freed_after_method_call() {
        // fn f() -> i64 { let s: String = String::from("hello"); s.len() }
        // s.len() is read-only — s must still be freed
        let string_from = Expr {
            kind: ExprKind::EnumConstruct {
                path: vec!["String".to_string(), "from".to_string()],
                fields: vec![Expr {
                    kind: ExprKind::Lit(Lit::String("hello".to_string())),
                    span: sp(),
                }],
            },
            span: sp(),
        };
        let method_call = Expr {
            kind: ExprKind::MethodCall {
                receiver: Box::new(ident_expr("s")),
                method: "len".to_string(),
                args: vec![],
            },
            span: sp(),
        };
        let body = Block {
            stmts: vec![let_stmt(
                "s",
                Some(Type::Named {
                    name: "String".to_string(),
                    span: sp(),
                }),
                string_from,
            )],
            trailing_expr: Some(Box::new(method_call)),
            span: sp(),
        };
        let fn_def = make_fn("f", vec![], i64_ty(), body, vec![]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let has_string_free = func.blocks.iter().flat_map(|b| b.insts.iter()).any(|i| {
            i.opcode == Opcode::Call
                && i.data == InstData::CallExtern("__vow_string_free".to_string())
        });
        assert!(
            has_string_free,
            "String used via .len() must still be freed at function exit"
        );
    }

    #[test]
    fn vec_freed_after_method_call() {
        // fn f() -> i64 { let v: Vec<i64> = Vec::new(); v.len() }
        // v.len() is read-only — v must still be freed
        let vec_new = Expr {
            kind: ExprKind::EnumConstruct {
                path: vec!["Vec".to_string(), "new".to_string()],
                fields: vec![],
            },
            span: sp(),
        };
        let method_call = Expr {
            kind: ExprKind::MethodCall {
                receiver: Box::new(ident_expr("v")),
                method: "len".to_string(),
                args: vec![],
            },
            span: sp(),
        };
        let body = Block {
            stmts: vec![let_stmt(
                "v",
                Some(Type::Named {
                    name: "Vec".to_string(),
                    span: sp(),
                }),
                vec_new,
            )],
            trailing_expr: Some(Box::new(method_call)),
            span: sp(),
        };
        let fn_def = make_fn("f", vec![], i64_ty(), body, vec![]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let has_vec_free = func.blocks.iter().flat_map(|b| b.insts.iter()).any(|i| {
            i.opcode == Opcode::Call
                && i.data == InstData::CallExtern("__vow_vec_free_val".to_string())
        });
        assert!(
            has_vec_free,
            "Vec used via .len() must still be freed at function exit"
        );
    }

    #[test]
    fn hashmap_freed_after_method_call() {
        // fn f() -> i64 { let m: HashMap = HashMap::new(); m.len() }
        // m.len() is read-only — m must still be freed
        let map_new = Expr {
            kind: ExprKind::EnumConstruct {
                path: vec!["HashMap".to_string(), "new".to_string()],
                fields: vec![],
            },
            span: sp(),
        };
        let method_call = Expr {
            kind: ExprKind::MethodCall {
                receiver: Box::new(ident_expr("m")),
                method: "len".to_string(),
                args: vec![],
            },
            span: sp(),
        };
        let body = Block {
            stmts: vec![let_stmt(
                "m",
                Some(Type::Named {
                    name: "HashMap".to_string(),
                    span: sp(),
                }),
                map_new,
            )],
            trailing_expr: Some(Box::new(method_call)),
            span: sp(),
        };
        let fn_def = make_fn("f", vec![], i64_ty(), body, vec![]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let has_map_free = func.blocks.iter().flat_map(|b| b.insts.iter()).any(|i| {
            i.opcode == Opcode::Call && i.data == InstData::CallExtern("__vow_map_free".to_string())
        });
        assert!(
            has_map_free,
            "HashMap used via .len() must still be freed at function exit"
        );
    }
}
