#![allow(unused_variables, dead_code)]
use crate::{env::TypeEnv, types::Ty};
use std::collections::BTreeSet;
use vow_diag::DiagnosticEmitter;
use vow_syntax::ast::{Block, Effect, EnumDef, Expr, FnDef, Item, Module, Stmt, StructDef};

/// Main type checker. Walks the AST and type-checks every node.
///
/// Parallel Wave 2 agent responsibility: implement all todo!() methods.
/// Call sites to effects::check_fn_effects and linear::check_linear_usage
/// should be added inside check_fn after verifying the body's type.
#[allow(dead_code)]
pub struct Checker<'e> {
    pub(crate) env: TypeEnv,
    pub(crate) emitter: &'e mut dyn DiagnosticEmitter,
    /// Effect set declared by the currently-checked function.
    pub(crate) current_fn_effects: BTreeSet<Effect>,
    /// Return type of the currently-checked function.
    pub(crate) current_return_ty: Ty,
    /// Source file path (for diagnostics).
    pub(crate) file: String,
    pub(crate) error_count: usize,
}

impl<'e> Checker<'e> {
    pub fn new(emitter: &'e mut dyn DiagnosticEmitter, file: impl Into<String>) -> Self {
        todo!()
    }

    /// Entry point: register all top-level definitions, then check each item.
    pub fn check_module(&mut self, module: &Module) {
        todo!()
    }

    pub fn has_errors(&self) -> bool {
        todo!()
    }

    fn check_item(&mut self, item: &Item) {
        todo!()
    }

    fn check_fn(&mut self, fn_def: &FnDef) {
        todo!()
    }

    fn check_struct(&mut self, s: &StructDef) {
        todo!()
    }

    fn check_enum(&mut self, e: &EnumDef) {
        todo!()
    }

    fn check_block(&mut self, block: &Block) -> Ty {
        todo!()
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        todo!()
    }

    fn check_expr(&mut self, expr: &Expr) -> Ty {
        todo!()
    }
}
