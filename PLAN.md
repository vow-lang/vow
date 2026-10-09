# Plan: Decide and pin the `[panic]` effect scope (issue #622)

## Goal
Close #622 by making the spec the single source of truth for which abort sources require `[panic]`, and pin that decision with tests in both compilers. Recommended decision: `[panic]` stays scoped to `.unwrap()`; indexing, checked arithmetic (`+! -! *! /! %!`) and `/ %` zero-divisor traps do NOT require `[panic]`.

## Assumptions
- Decision (best guess, no operator available): do NOT extend `[panic]` to `Index`/checked-arith/div. Alternative considered and rejected: register those as panic sites in `collect_calls_in_expr` (what the issue's "proposed fix" says). Reasons (all verified in tree):
  1. **Breaks verification (design criterion 1).** `vow-verify/src/c_emitter.rs:1002,1031` and `compiler/c_emitter.vow:522,630` treat ANY function with a non-empty effect set as non-modelable ("has effects; the verifier model is restricted to pure functions"). Requiring `[panic]` on every indexing / `+!` function would strip those functions from verification, silently converting the verified `Verified`/`CheckedArithmeticAbort` surface into `VerificationSkipped`. `docs/spec/contracts.md:177,437` recommends `+!` precisely so the verifier models the abort.
  2. **Blast radius.** ~2400 index expressions and ~157 checked-op uses in `compiler/*.vow`, `tests/`, `examples/`, `benchmarks/` live in functions without `[panic]`; the whole self-hosted compiler and corpus would stop type-checking, and fixing them would force effects onto verified contracted functions (criterion 1 again).
  3. **Established precedent is narrow.** The spec only ties `[panic]` to `.unwrap()` (grammar.md:1125, 1233); the verifier already models checked-arith aborts as pruned executions (errors.md:829-845) and a failed unwrap as `unwrap() called on None` (compiler/verifier.vow:895), so `[panic]` is NOT defined as "unmodelled aborts" — the decision rests on rule 1 alone (any effect ⇒ non-modelable).
- View 2 (call-form `Option::unwrap(x)`): `unwrap` exists only as a builtin *method* (`vow-types/src/check.rs:474` `builtin_method_names`, `compiler/checker.vow:1974`); there is no free-function/call-form spelling, so the string match is not currently bypassable. Plan pins this with an error fixture rather than introducing a resolved-builtin-identity refactor (would touch type-check results/IR in both compilers; follow-up if a second panic builtin is ever added).
- Roadmap §24.3 (`docs/roadmap.md:395-401`) is stale wording ("no builtins are annotated"); updated to the decided scope.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `vow-types/src/effects.rs` | Rust `collect_calls_in_expr` (355-460), `unwrap` panic site (383), diagnostic (536-560), unit tests (~1050+) | add scope doc comment + unit tests |
| `compiler/checker.vow` | self-hosted `collect_calls_in_expr` (4400-4480), `__unwrap__` marker (4435), `check_effects_fn` (4583-4605) | doc comment + test hook |
| `docs/spec/grammar.md` | Effect Types table (1225-1235), `v[i]` (1053), Checked Arithmetic (340-362), `.unwrap()` (1125) | document scope |
| `docs/spec/contracts.md` | verifier modelability / effects (142), `+!` guidance (177,437) | cross-reference |
| `docs/spec/errors.md` | `EffectViolation`, `IndexOutOfBounds`, `ArithmeticOverflow` sections (928-968) | note: not `[panic]`-gated |
| `docs/roadmap.md` | §24.3 stale claim (395-401) | rewrite |
| `docs/adr/2026-10-09-NNNN-panic-effect-scope.md` [new] | ADR recording decision + rejected alternative | new |
| `docs/adr/README.md` | index of ADRs | add row |
| `tests/error/panic_effect_unwrap_pure_fn.vow` [new] | pure fn with `.unwrap()` → EffectViolation | new |
| `tests/error/unwrap_call_form_rejected.vow` [new] | `Option::unwrap(x)` call-form rejected (unknown fn) | new |
| `tests/run/index_checked_arith_no_panic_effect.vow` [new] | pure fn with `v[i]`, `+!`, `/!`, `%!`, `/` compiles & runs | new |
| `tests/verify/checked_arith_abort_modelled.vow` | existing pin: pure `+!` fns are Verified | unchanged |
| `compiler/tests/test_checker_panic_effect.vow` [new] | self-hosted unit test mirroring Rust unit tests | new |

## Steps (TDD slices)

### 1. Pin current behavior with tests (red/green-neutral characterization)
- **Rust**: in `vow-types/src/effects.rs` `mod tests`, add (a) `index_does_not_require_panic`, (b) `checked_binop_does_not_require_panic` (each `BinOp::{Add,Sub,Mul,Div,Rem}Checked`, plus `Div`/`Rem`) building a pure `FnDef` and asserting zero diagnostics via the existing test emitter/`body_with_unwrap` helpers (~line 1050), (c) existing `unwrap_without_panic_effect_emits_violation` / `unwrap_with_panic_effect_no_error` (effects.rs:1078,1090) already cover unwrap — keep.
- **Self-hosted**: `compiler/tests/test_checker_panic_effect.vow` — use the `checked_diags(source) -> DiagCtx` helper pattern from `compiler/tests/test_checker_diag_text.vow:9-25` (parse + `check_module`, then inspect the returned `DiagCtx`); `test_checker_pattern_metadata.vow` only exposes `env_error_count` and is a fallback: pure fn with `.unwrap()` errors; pure fn with `v[i]` / `a +! b` / `a /! b` has no EffectViolation.
- **Fixtures**: add `tests/error/panic_effect_unwrap_pure_fn.vow` (`// TEST: error-code EffectViolation`), `tests/error/unwrap_call_form_rejected.vow` (`Option::unwrap(x)`; implementer must run once to capture the actual error code and set the `TEST:` directive from the real output; if it unexpectedly type-checks, STOP and escalate – that means View 2 is live and a resolved-identity fix is needed), `tests/run/index_checked_arith_no_panic_effect.vow`. No new verify fixture: `tests/verify/checked_arith_abort_modelled.vow` (pure fns with `+!`, Verified) already pins modelability; the implementer confirms it contains no `[panic]` and leaves it unchanged.
- All of these pass on current code; they are regression pins for the decision.

### 2. Document the decision in code (both compilers, same commit)
- `vow-types/src/effects.rs` above `collect_calls_in_expr`: 3-line comment — panic sites = builtin aborts the verifier cannot model (`.unwrap()` only); `Index`/checked-arith/div aborts are verifier-modelled obligations and deliberately NOT sites (ref ADR, c_emitter non-modelable-on-effects rule). Same in `compiler/checker.vow` above `collect_calls_in_expr` (4400). Comments only; no behavior change, so no bootstrap fixed-point or C-parity impact.
- Fix stale cross-reference at `compiler/checker.vow:1060-1061` only if wording becomes wrong (it references issue #601; leave otherwise).

### 3. Spec + ADR
- `docs/spec/grammar.md` Effect Types table row `panic` (line 1233, keep the row one line, e.g. "`.unwrap()` aborts; not indexing/checked ops") plus a paragraph below the table: "`.unwrap()` requires `[panic]`. Out-of-bounds indexing, checked operators (`+! -! *! /! %!`) and `/`,`%` traps do not. Their aborts are verification obligations of pure functions, and declaring an effect would remove a function from the verifier model." Also add one sentence under `v[i]` (1053) and Checked Arithmetic (340).
- `docs/spec/contracts.md` near 142/437: one sentence linking "why `+!`/indexing need no `[panic]`".
- `docs/spec/errors.md` `IndexOutOfBounds` / `ArithmeticOverflow`: "not gated by `[panic]`".
- `docs/roadmap.md` §24.3: replace stale claim with decided scope; list "resolve panic builtins by canonical identity" as the follow-up trigger (second panic-producing builtin).
- New ADR `docs/adr/2026-10-09-NNNN-panic-effect-scope.md` (follow `docs/adr/README.md` format; pick NNNN by current time as existing names do) + README row.
- REQUIRED: the Effect Types table row is embedded verbatim in `vow/src/skill.rs:2627,8543` and `compiler/main.vow:4926,10864`. After editing grammar.md run `uv run python scripts/generate_help.py`, `cargo build --release -p vow`, `scripts/bootstrap.sh --skip-cargo`, then `python3 scripts/check_help_coverage.py`. (Avoid this churn by leaving the table row text unchanged and putting the new text only in the paragraph below it, if that paragraph is not embedded — implementer checks `git diff` of skill.rs/main.vow after generate_help.py.)

### 4. Gates
Run separately, never `&&`-chained: `cargo fmt --all`, `cargo clippy --all --all-targets -- -D warnings`, `cargo test -p vow-types`, `build/vowc test compiler/tests/test_checker_panic_effect.vow`, `scripts/bootstrap.sh --skip-cargo --no-cache` (background + poll; ~5 min), `python3 scripts/generate_operations.py --check`, targeted `scripts/full_test.sh` sections for new fixtures (full ~40 min; background). Record head SHA in PR checklist.

## Testing
- Rust unit tests in `vow-types/src/effects.rs`; self-hosted `compiler/tests/test_checker_panic_effect.vow`; fixtures listed above. No new production behavior, so no coverage-gate exposure (`codecov/patch`): only comment/test lines change in `.rs`.

## Verification surface
- No contract/codegen/C-model change; ESBMC obligations unchanged. Existing `tests/verify/checked_arith_abort_modelled.vow` guards that pure `+!` functions stay modelable. `vow-ir/src/lower/mod.rs:8031` (`vec![Effect::Panic]`) was checked: it is a unit-test fixture, not lowering-side panic inference.

## Risks
- Reviewer expects the issue's literal "proposed fix" (flag Index/checked-arith): PR description and ADR must state the verifier-modelability argument and blast-radius numbers; the issue itself flags "needs a spec decision".
- If Step 1's call-form fixture unexpectedly passes type-check, plan is invalid for View 2 → implementer posts `gh issue comment` and narrows to the resolved-identity fix.
- Doc edits in grammar.md may require regenerated help/skill in both compilers (`generate_help.py`) – byte drift fails `check_help_coverage.py`/fixed point if forgotten.
- `.ultraplan/` and `PLAN.md` must not reach the PR (`git rm PLAN.md` first).
- Commit/PR title lower-case Conventional Commits, e.g. `docs(spec): scope [panic] effect to unwrap and pin with tests`; type `docs`/`test` means no release bump.

## Out of scope
- Making Index / checked-arith / div require `[panic]` or any new effect-system axis.
- Refactoring `__unwrap__` string marker into a resolved builtin identity / IR opcode (follow-up if a second panic builtin appears).
- Changing verifier modelability rules for effectful functions, or `[panic]` handling in `check_clause_purity` (issue #601).
- Roadmap items beyond §24.3 wording.
