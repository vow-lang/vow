#![allow(unused_variables)]
use crate::env::TypeEnv;
use vow_diag::DiagnosticEmitter;
use vow_syntax::ast::{FnDef, VowBlock};

/// Check that all effects used in the function body are declared in fn_def.effects.
///
/// Algorithm (for Wave 2 Agent B to implement):
/// 1. Walk all call expressions in the function body.
/// 2. For each call, look up the callee's FnSig in env.
/// 3. If any callee effect is absent from fn_def.effects, emit EffectViolation.
/// 4. Check closures recursively.
/// 5. IO effect subsumes Read and Write (declaring IO covers both).
pub fn check_fn_effects(
    fn_def: &FnDef,
    env: &TypeEnv,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    todo!()
}

/// Check that all expressions in a vow block (requires/ensures/invariant)
/// call no effectful functions. Vow predicates must be pure.
pub fn check_vow_purity(
    vow: &VowBlock,
    env: &TypeEnv,
    file: &str,
    emitter: &mut dyn DiagnosticEmitter,
) {
    todo!()
}
