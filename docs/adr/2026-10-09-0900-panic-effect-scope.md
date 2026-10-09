# `[panic]` is scoped to `.unwrap()`

**Status:** accepted (2026-10-09)

Resolves #622.

## Context

The effect checker requires `[panic]` only for `.unwrap()`. Out-of-bounds
indexing, the checked operators (`+! -! *! /! %!`) and the `/`, `%` zero-divisor
traps can also abort, yet type-check in functions with no effects. Issue #622
asked whether `[panic]` should cover them.

## Decision

`[panic]` stays scoped to `.unwrap()`. Indexing, checked arithmetic and `/ %`
traps do not require it. The spec (`docs/spec/grammar.md` → Effect Types) states
the scope and both compilers pin it with tests.

## Why

- **Verification.** `vow-verify/src/c_emitter.rs` and `compiler/c_emitter.vow`
  treat any function with a non-empty effect set as non-modelable. Requiring
  `[panic]` on every indexing or `+!` function would silently turn verified
  functions into `VerificationSkipped`, and `docs/spec/contracts.md` recommends
  `+!` precisely so the verifier models the abort. These aborts are verification
  obligations of pure functions, not unmodelled behaviour.
- **Blast radius.** About 2400 index expressions and 157 checked-operator uses
  in `compiler/`, `tests/`, `examples/` and `benchmarks/` sit in functions with
  no `[panic]`; all would need effects, pushing them out of the verifier model.
- **Precedent.** The spec ties `[panic]` to `.unwrap()` only.

## Rejected alternative

Register `Index`, checked `BinaryOp` and `/ %` as panic sites in
`collect_calls_in_expr`. Rejected for the verification and blast-radius reasons
above.

## Follow-up

`.unwrap()` detection is a method-name match (`unwrap`, and the `__unwrap__`
marker in the self-hosted checker). `unwrap` has no call-form spelling, so it is
not bypassable today (`tests/error/unwrap_call_form_rejected.vow`). If a second
panic-producing builtin appears, resolve panic builtins by canonical identity.
