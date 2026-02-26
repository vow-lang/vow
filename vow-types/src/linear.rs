#![allow(unused_variables)]
use crate::env::TypeEnv;
use vow_diag::DiagnosticEmitter;
use vow_syntax::ast::FnDef;

/// Check that all linear-typed variables in fn_def are consumed exactly once.
///
/// Rules (for Wave 2 Agent C to implement):
/// - A variable of linear type (StructInfo::is_linear == true) must be "consumed" exactly once.
/// - "Consumed" = passed by value (not as &reference) to a function.
/// - Borrow (&x) does NOT consume.
/// - If never consumed by end of function: emit LinearTypeViolation ("linear value never consumed").
/// - If consumed more than once: emit LinearTypeViolation ("linear value already consumed").
/// - Branching (if/match): every branch must consume the same set of linear variables.
/// - Loops: linear vars initialized before a loop cannot be consumed inside the loop body
///   (would consume multiple times).
pub fn check_linear_usage(
    fn_def: &FnDef,
    env: &TypeEnv,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    todo!()
}
