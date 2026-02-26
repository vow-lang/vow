#![allow(unused_variables)]
use crate::{env::TypeEnv, types::Ty};
use vow_diag::DiagnosticEmitter;
use vow_syntax::ast::MatchArm;
use vow_syntax::span::Span;

/// Check that the match arms exhaustively cover all cases of scrutinee_ty.
///
/// Rules (for Wave 2 Agent A to implement):
/// - If any arm is a wildcard `_`, trivially exhaustive.
/// - Bool: must cover true and false (or wildcard).
/// - Enum: must cover every variant name (or wildcard).
/// - Option<T> (Enum("Option")): must cover Some(_) and None.
/// - Result<T,E> (Enum("Result")): must cover Ok(_) and Err(_).
/// - Tuples/nested: recursive check.
/// - Missing patterns emitted as NonExhaustiveMatch diagnostic on `span`.
pub fn check_exhaustive(
    scrutinee_ty: &Ty,
    arms: &[MatchArm],
    env: &TypeEnv,
    span: Span,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    todo!()
}
