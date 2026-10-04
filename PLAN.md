# Plan: issue #589 — `where`-clause refinement is never type-checked

## 1. Problem restated

A parameter's `where` clause (e.g. `b: i64 where a >= 0`) is parsed into
`Param.refinement` (Rust: `vow-syntax/src/ast.rs:70`; self-hosted: param list
slot `i*4+2`, `compiler/parser.vow:487-494`) but `check_fn` in both compilers
never visits it — `vow-types/src/check.rs:1517-1608` and
`compiler/checker.vow:1026-1106` define parameter types and type-check
`requires`/`ensures`, but skip `param.refinement` entirely. Lowering, however,
unconditionally turns every `param.refinement` into a `VowClause::Requires`
(`vow-ir/src/lower/vow.rs:221-245`, called from `vow-ir/src/lower/mod.rs:5193`
before the body is lowered), which the C emitter renders as
`__ESBMC_assume(pred)` (`vow-verify/src/c_emitter.rs:1012-1015`). A `where`
clause that is not `bool`-typed, or that references a sibling parameter
instead of only its own (forbidden by `docs/spec/grammar.md:112` and
`docs/spec/contracts.md:313`), is silently accepted and becomes an unchecked,
possibly-wrong `assume` the verifier treats as ground truth — a soundness
hole, not merely a missing diagnostic.

Confirmed by direct inspection (not just the issue's own probe):
- Self-hosted's generic identifier resolution (`EXPR_IDENT` in
  `compiler/checker.vow:2515-2528`) has **no "undefined variable" diagnostic
  at all** — `env_lookup_var` (`compiler/env.vow:614-624`) returns `CTY_UNIT()`
  silently when a name isn't bound, and that is the only call site in the
  checker. Comparison operators (`==`, `!=`, `<`, `<=`, `>`, `>=`) then
  unconditionally `return CTY_BOOL()` regardless of operand validity
  (`compiler/checker.vow:2610`). So **merely type-checking the refinement as
  `bool` is not enough**: `b: i64 where a >= 0` (sibling reference, correctly
  bool-shaped) would pass silently in the self-hosted compiler even with a
  bool-only check. Only the ill-typed case (`where a + 1`, issue's probe) is
  caught by a bare bool check, and only by accident.
- Rust's generic identifier resolution (`ExprKind::Ident` in
  `vow-types/src/check.rs:2058-2081`) *does* already emit
  `ErrorCode::TypeMismatch` ("undefined variable `{name}`") when a name isn't
  in scope — but only if the sibling parameter truly isn't visible in the
  scope chain at the point of the check. `TypeEnv::lookup` walks **all**
  scopes on the stack (`vow-types/src/env.rs:553`), so scope nesting order
  alone does not reliably hide siblings either.

So the fix needs an **explicit "references only its own parameter" check**
in both compilers, independent of incidental scope-stack shape — see §3.

## 2. Files to touch

Rust (bootstrap) compiler:
- `vow-types/src/check.rs` — `check_fn` (currently lines 1517-1608): type-check
  each `param.refinement`, require `bool`/`Never`, reject references to
  anything but the owning parameter (consts excepted).
- `vow-types/src/effects.rs` — `check_fn_effects` (line 492) /
  `check_vow_purity` (line 573): extract the per-expression purity body
  (lines 586-635) into a reusable `check_expr_purity`, add
  `check_param_refinement_purity` over `fn_def.params`, call it from
  `check_fn_effects`.

Self-hosted compiler (`compiler/`):
- `compiler/env.vow` — add `refinement_own_name: String` field to `CheckEnv`
  (next to `contract_depth: i64` at line 101; initialize to `String::from("")`
  near line 271, alongside `contract_depth: 0`).
- `compiler/checker.vow` — `check_fn` (lines 1026-1106): read the refinement
  slot (`list_get(a, params_lid, i * 4 + 2)`), type-check it, require `bool`,
  call `check_clause_purity` on it (already exists, `compiler/checker.vow:893`).
  `EXPR_IDENT()` handling (lines 2515-2528): when `e.refinement_own_name` is
  non-empty and the name is neither the owner nor a known const, emit
  `EC_TYPE_MISMATCH()` "undefined variable `{name}`" instead of silently
  returning `CTY_UNIT()`.

Docs:
- `docs/spec/errors.md` — `ContractTypeMismatch` entry (line 534-549):
  "Meaning" currently says "A `requires`, `ensures`, or `invariant` clause...";
  extend to include "or a parameter's `where` refinement".
- After editing `errors.md`, regenerate the embedded copies:
  `uv run python scripts/generate_help.py`, then `cargo build --release -p vow`
  and `scripts/bootstrap.sh --skip-cargo` (the embedded skill/help text in
  `compiler/main.vow` is checked for drift by `scripts/check_help_coverage.py`
  in `full_test.sh`).

Tests:
- `tests/error/param_refinement_non_bool.vow` (new) — `where` clause with
  non-`bool` type.
- `tests/error/param_refinement_sibling_reference.vow` (new) — `where` clause
  that is correctly `bool`-typed but references a sibling parameter (the case
  a bool-only check would miss in self-hosted).
- Rust unit tests in `vow-types/src/check.rs` (near the existing
  `extern_signature_refinement_predicate_fails_closed` test at line 4183) and
  in `vow-types/src/effects.rs` (near its existing `#[cfg(test)] mod tests` at
  line 639).

No changes needed in `vow-ir/src/lower/vow.rs`, `vow-ir/src/lower/mod.rs`,
`compiler/lower.vow`, or `vow-verify/src/c_emitter.rs`: lowering only runs
after `has_errors()` is false (gated in `vow/src/frontend.rs:220`; self-hosted
has the equivalent gate in `compiler/main.vow`), so once `check_fn` rejects a
malformed `where` clause, it never reaches lowering. Verified no other
consumer of `Param.refinement` needs updating: `vow-syntax/src/printer.rs:137`
(printer, no type checking), `vow/src/complexity.rs:1676` /
`compiler/complexity.vow:946` (complexity classification, walks the predicate
but doesn't validate it), and the `AstType::Refinement` comment in
`vow-types/src/env.rs:772-784` (a different, already-rejected feature: inline
`{ x: T || pred }` refinement *types*, not parameter `where` clauses).

## 3. TDD slices

Each slice is red (failing test) → green (minimal production code) →
refactor-if-needed. Rust and self-hosted slices are listed in alternating
pairs per behavior, but each is independently committable.

1. **Rust: reject non-`bool` `where` clause.**
   Red: new test in `vow-types/src/check.rs` (e.g.
   `param_refinement_rejects_non_bool_type`) building a `FnDef` with
   `Param { refinement: Some(Box::new(<i64 expr>)), .. }`, asserting
   `checker.has_errors()` and that the emitted diagnostic has
   `ErrorCode::ContractTypeMismatch`.
   Green: in `check_fn`'s parameter loop (`vow-types/src/check.rs:1531-1537`),
   after `self.env.define(&param.name, ty.clone())`, if
   `param.refinement.is_some()`, call `self.check_contract_expr(refinement)`
   and emit `ContractTypeMismatch` (message: `` `where` clause has type `{ty}`
   but must be `bool` ``) when the result isn't `Ty::Bool`/`Ty::Never`.

2. **Rust: reject sibling/undefined references in a `where` clause.**
   Red: new test with two params, `b`'s refinement referencing `a` (e.g.
   `a >= 0` with `a` the *other* param) — bool-shaped, so slice 1's check
   alone must not catch it; assert `has_errors()` and
   `ErrorCode::TypeMismatch` with an "undefined variable" message.
   Green: add a `refinement_scope: Option<String>` field to `Checker`
   (alongside `contract_depth` at `vow-types/src/check.rs:1000`). In
   `check_fn`, wrap each refinement check: set
   `self.refinement_scope = Some(param.name.clone())` before
   `check_contract_expr`, restore the previous value after. In
   `ExprKind::Ident` handling (`vow-types/src/check.rs:2058`), when
   `self.refinement_scope` is `Some(owner)` and `name != owner` and
   `!self.const_types.contains_key(name)`, emit `TypeMismatch`
   ("undefined variable `{name}`") and return `Ty::Unit` *before* falling
   into the normal const/env lookup (self-references and const references
   fall through unchanged so their real type is still resolved for later
   binop checks). This check is in the existing recursive `check_expr`
   traversal, so it already skips call-callee identifiers (the `Call` arm at
   `vow-types/src/check.rs:2286-2301` reads `callee.kind` directly without
   routing it through `check_expr`), so calling a pure predicate function
   from a `where` clause is unaffected.

3. **Rust: `where` clauses must be pure, like `requires`/`ensures`.**
   Red: new test in `vow-types/src/effects.rs` with a `where` clause calling
   an effectful function (or writing through a shared argument); assert the
   emitted diagnostics include `ErrorCode::EffectViolation`.
   Green: extract the loop body of `check_vow_purity`
   (`vow-types/src/effects.rs:579-636`, the part from `let mut calls = ...`
   through the write-sites loop) into `fn check_expr_purity(expr: &Expr, env:
   &TypeEnv, file: &str, emitter: &mut dyn DiagnosticEmitter)`; have
   `check_vow_purity` call it per clause (no behavior change). Add `fn
   check_param_refinement_purity(params: &[Param], env: &TypeEnv, file: &str,
   emitter: &mut dyn DiagnosticEmitter)` that calls `check_expr_purity` for
   each `param.refinement`. Call it from `check_fn_effects`
   (`vow-types/src/effects.rs:492`) next to the existing
   `check_vow_purity(vow_block, ...)` call at line 562.

4. **Self-hosted: reject non-`bool` `where` clause.**
   Red: add a probe to the self-hosted test corpus mirroring slice 1 (or
   cover it directly via the `tests/error/param_refinement_non_bool.vow`
   fixture from slice 8 run through `build/vowc build --no-verify`, checking
   the JSON diagnostic's `error_code`). If the project's self-hosted unit-test
   convention (`compiler/test_*.vow`, excluded from mutation scanning per
   `CLAUDE.md`) supports a lighter-weight in-process probe, prefer that for a
   faster red/green loop; otherwise this slice's "test" is the fixture.
   Green: in `check_fn` (`compiler/checker.vow:1047-1062`), read
   `refinement_eid: i64 = list_get(a, params_lid, i * 4 + 2)` alongside the
   existing `ast_ptid` read at `i * 4 + 1`. After `env_define_var`, when
   `refinement_eid != -1`, bump `e.contract_depth`, call
   `check_expr(e, m, refinement_eid)`, restore `e.contract_depth`, and emit
   `EC_CONTRACT_TYPE_MISMATCH()` ("`where` clause must be `bool`") when the
   result isn't `CTY_BOOL()`/opaque — mirroring `check_vow_clause`
   (`compiler/checker.vow:934-967`).

5. **Self-hosted: reject sibling/undefined references.**
   Red: the `tests/error/param_refinement_sibling_reference.vow` fixture
   (slice 8) run through the self-hosted binary must also emit an error
   (today it would pass silently — see the `CTY_BOOL()`-regardless-of-operand
   finding in §1).
   Green: add `refinement_own_name: String` to `CheckEnv`
   (`compiler/env.vow:101`, initialized to `String::from("")` near line 271).
   In slice 4's refinement-checking block, before calling `check_expr`, save
   and set `e.refinement_own_name = pname` (the owning parameter's name),
   restore the saved value after. In `EXPR_IDENT()` handling
   (`compiler/checker.vow:2515-2528`), when `e.refinement_own_name.len() > 0`
   and `name` is neither equal to it nor a known const name, emit
   `EC_TYPE_MISMATCH()` ("undefined variable `{name}`") and return
   `CTY_UNIT()` before the existing const-scan/`env_lookup_var` path. Extract
   the existing const-name scan (already duplicated once in that function)
   into a small `fn env_is_const_name(e: CheckEnv, name: String) -> bool`
   helper shared by both the new check and the existing lookup, rather than
   writing it a third time.

6. **Self-hosted: `where` clauses must be pure.**
   Red: mirror slice 3's effectful-call fixture; self-hosted's existing
   `check_clause_purity` (`compiler/checker.vow:893-932`) already emits
   `EC_EFFECT_VIOLATION()` for this shape, so this should go green almost
   immediately once wired up.
   Green: in slice 4's refinement block, call
   `check_clause_purity(e, m, refinement_eid);` right after the bool check —
   the function already exists and needs no changes.

7. **Docs + generated-help sync.**
   Update `docs/spec/errors.md`'s `ContractTypeMismatch` entry (§2). Run
   `uv run python scripts/generate_help.py`, `cargo build --release -p vow`,
   `scripts/bootstrap.sh --skip-cargo`. Confirm
   `python3 scripts/check_help_coverage.py` (or the `full_test.sh` section
   that runs it) is clean.

8. **Cross-compiler fixtures.**
   Add `tests/error/param_refinement_non_bool.vow` and
   `tests/error/param_refinement_sibling_reference.vow`, each with a `//
   TEST: stderr "..."` line documenting the expected message (checked locally
   by `tests/run_tests.sh`; not CI-gating per this repo's `tests/error/*`
   convention, but keeps the fixture self-documenting and matches every
   existing file in that directory). Run the relevant section of
   `scripts/full_test.sh` (`run_promoted_error_tests`, which runs both
   compilers over every `tests/error/*.vow` fixture and asserts via
   `compare_error`/`scripts/parity.py` that both emit the **same
   `error_code` multiset** — this is what forces the two compilers' new
   diagnostics to agree on `ErrorCode::ContractTypeMismatch` /
   `ErrorCode::TypeMismatch` as appropriate, not just "some error").

9. **Full regression sweep.**
   `cargo test --all`, `cargo clippy --all -- -D warnings` (note: CI's gate is
   `--all`, not `--all-targets` — don't fix unrelated test-module lints beyond
   what `--all` surfaces), `scripts/bootstrap.sh`, `scripts/full_test.sh`.
   Confirm the existing valid-usage corpus still passes unchanged:
   `tests/verify/where_clamp.vow`, `tests/verify/where_divide.vow`,
   `examples/where_clamp.vow`, `examples/where_divide.vow`,
   `tests/fixtures/contracts/where_refinement_offsets.vow` (all
   self-reference-only, bool-typed — confirmed by inspection, no change
   expected, but must be re-run since this PR touches the exact check path
   they exercise).

## 4. Verification surface

This change is confined to the type checker; it does not alter the IR, the C
model, or what ESBMC proves for any program that already type-checks. No new
ESBMC-provable property is introduced — the fix's job is to make sure a
malformed `where` clause never reaches the point where it would silently
become an `__ESBMC_assume`. The two new `tests/error/*.vow` fixtures are
compile-time rejections (`build --no-verify` suffices in
`run_promoted_error_tests`); they do not need `verify`/`verify-fail` variants.

No bound, cap, or verification-driven weakening is introduced anywhere —
the fix only adds `bool`-type and self-reference-only checks, both of which
are exact restatements of the existing spec text (`grammar.md:112`,
`contracts.md:313`), not verifier accommodations.

## 5. Risk areas

- **Binary fixed point**: no codegen, IR, or `vow-clif-shim` changes — the
  bootstrap triple test (`scripts/concat_vow.sh` + stage 0/1/2 comparison)
  should be unaffected. Still run it (slice 9) since `compiler/env.vow` and
  `compiler/checker.vow` are concatenated into the self-hosted binary and a
  typo could break the self-hosted compiler's ability to compile itself.
- **`parse → print → parse` idempotency**: untouched — the printer
  (`vow-syntax/src/printer.rs:137`) already prints `param.refinement`
  correctly today; this fix only adds checking, not new syntax or AST shape.
- **`cargo clippy --all -- -D warnings`**: the new `Option<String>` field
  (`refinement_scope`) and the extracted `check_expr_purity` function are
  small, idiomatic additions; watch for `clippy::too_many_arguments` if
  `check_expr_purity`'s signature grows, and for an unused-`mut`-style lint
  on the self-hosted side being mirrored into Rust tests (none expected).
- **False positives on existing valid code.** The explicit self-reference
  check is strict: it rejects consts referenced by a *different* name
  resolution path than expected, or any identifier shape not already in
  `const_types`/self name. Before considering this slice done, grep
  `compiler/**/*.vow`, `benchmarks/**/reference.vow`, and every `*.vow` under
  `docs/spec/`'s embedded examples (if any are extracted and compiled by a
  doc test) for `where` usage beyond what was already inventoried in this
  plan's §1/§2, since a previously-silent sibling-reference bug anywhere in
  that corpus would now surface as a hard compile error and block the
  regression sweep. (This plan's own grep across `compiler/*.vow` and
  `benchmarks/**/reference.vow` for `where` outside comments found none; the
  `tests/`/`examples/` corpus was fully inventoried in §1 and is clean.)
- **Self-hosted `const_names` scan cost.** The new `env_is_const_name` helper
  is an O(n) linear scan like the existing one it's extracted from; fine at
  current const-table sizes, consistent with the rest of `checker.vow`'s
  style, not a new asymptotic concern.
- **Ambient-state leakage.** Both `refinement_scope` (Rust) and
  `refinement_own_name` (self-hosted) must be restored (not just set) around
  each refinement check, including on the self-hosted side where `check_expr`
  has no early-return/panic path that would skip the restore — a plain
  save/restore around a single straight-line call is sufficient; no
  `Drop`-guard or `try`/`finally` equivalent is needed since neither
  compiler's `check_expr` unwinds.

## 6. Out of scope

- **General self-hosted "undefined variable" diagnostic.** The root cause
  that makes self-hosted's `EXPR_IDENT` silent (`env_lookup_var` returning
  `CTY_UNIT()` with no error, the *only* call site, confirmed by grep) is a
  pre-existing gap affecting all of Vow, not just `where` clauses — e.g. any
  truly undefined identifier in a function body may currently type-check as
  unit and only fail downstream if something else cares about the resulting
  type. Fixing that globally is a much larger, separate change (touches
  every expression context, not just contract clauses) with its own
  regression surface across the whole self-hosted test suite. This plan adds
  only the narrow, contract-scoped version needed to close #589. Worth its
  own follow-up issue.
- **Inline refinement types** (`{ x: T || pred }` in type position, rejected
  today per `vow-types/src/env.rs:778-784`). Unrelated to parameter `where`
  clauses; not touched.
- **Extending `where` clauses to reference other parameters** or relaxing the
  spec's self-reference-only rule. Out of scope — this issue closes a
  soundness gap against the *current* spec, it doesn't change the spec.
- **Reformatting or refactoring `check_fn`/`checker.vow`'s `check_fn` beyond
  the minimal additions above.** Both functions are long; this plan
  deliberately does not restructure them further, beyond the one small
  `check_expr_purity`/`env_is_const_name` extraction each, which is extracted
  directly in service of the fix (reused twice within this same PR), not a
  speculative cleanup.
