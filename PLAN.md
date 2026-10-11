# Plan: close #1591 — pin `Neg/Not(Question(Cast))` printing in both compilers

## Goal
Issue #1591 (printer emits `-0 as i8?`, reparses as `(-0) as i8?`) is **already fixed on HEAD (3daab7d4)**:
the rule landed inside #1615 (77c65533, originally 60e72a7a). The remaining work is a test-only slice that pins
the shape in the places that are still unpinned, then closes the issue. No production code change expected.

## Findings (verified on HEAD)
- Rust: `vow-syntax/src/printer.rs:552` `question_chain_ends_in_cast`; used in the `UnaryOp` arm at `:616-618`
  (`Neg/Not` over a `?` chain ending in a `Cast` -> `-(a as i8?)`).
- Rust unit test exists: `printer.rs:1323` `test_unary_of_question_of_cast_is_parenthesised` (single `?` only).
- Rust regression seed exists: `vow-syntax/tests/proptest_roundtrip.proptest-regressions:19` (shrunk input is exactly
  `Neg(Question(Cast(Lit 0, i8)))`; the issue's `cc b262a39e...` is the RNG seed of the same case, redundant).
- Self-hosted: `compiler/contract_text.vow:165` `question_chain_ends_in_cast` (loop over `EXPR_QUESTION`), used in the
  `EXPR_UNOP` arm at `:258-262`. Same shape and same semantics as the Rust helper.
- Gap: the self-hosted rule has **no test at all**, and nested `??` (`Question(Question(Cast))`) has no Rust test.
  `tests/fixtures/contracts/*` cannot carry it: `vow contracts` type-checks and `?` on a cast result is ill-typed
  (same reason `-(!c)` lives in `compiler/tests/test_lower_contract_text.vow:104`, precedent #1567 / 88e43822).

## Assumptions
- Treat the issue as "fixed, needs pinning" rather than "blocked/already resolved, no commit": a test-only PR is the
  smallest change that makes the claim durable and satisfies the dual-compiler rule. (best guess)
- No `docs/spec/` change: syntax and semantics are unchanged; `grammar.md:537` already states unary binds tighter than `as`.
- `-0 as i8?` is a valid *source* spelling; only printed text changes behaviour, so no ADR.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `vow-syntax/src/printer.rs` | add nested-`?` unit test next to existing one | `:552-558`, `:616-618`, `:1323-1342` |
| `compiler/tests/test_lower_contract_text.vow` | add self-hosted pin `check_neg_question_cast_parenthesized`, wire into `main` | `:104-136` |
| `compiler/contract_text.vow` | mirrored rule (read-only unless step 1 finds a gap) | `:162-171`, `:255-262` |
| `vow-syntax/tests/proptest_roundtrip.proptest-regressions` | already holds the seed; do not touch | `:19` |

## Steps (TDD slices)

### 0. Re-confirm green at HEAD before touching anything
- Run `cargo test -p vow-syntax --test proptest_roundtrip expr_in_fn_roundtrip` (foreground, `-j` capped, e.g.
  `CARGO_BUILD_JOBS=4`), and once more with `PROPTEST_CASES=20000` to hunt a residual shape.
- If it passes: proceed (the expected outcome). If it fails with a *different* shrunk case, that is the real bug of
  this issue: fix it in both `printer.rs` and `contract_text.vow` (mirror), pin the shrunk seed, add a unit test.

### 1. Slice A — Rust nested `?` pin (red-green is trivially green; pins behaviour)
- **File**: `vow-syntax/src/printer.rs` tests module, beside `test_unary_of_question_of_cast_is_parenthesised`.
- **Test** `test_unary_of_nested_question_of_cast_is_parenthesised`: `Neg(Question(Question(Cast(a,u64))))` ->
  `-(a as u64??)`; `Not(...)` -> `!(a as u64??)`; negative controls: `Neg(Question(Ident a))` -> `-a?`,
  `Neg(Cast(Question(a), u64))` -> `-(a? as u64)`, `Cast(Neg(Question(a)))` keeps `(-a?) as u64`.
- **Reuses**: helpers `unary_expr`, `question_expr`, `ident_expr`, `named_ty` already used at `:1323-1445`.
- To prove the pin has teeth, locally mutate `question_chain_ends_in_cast` recursion arm to `false`, see red, revert.

### 2. Slice B — self-hosted pin (the actual missing coverage)
- **File**: `compiler/tests/test_lower_contract_text.vow`.
- Add `check_neg_question_cast_parenthesized() -> i64` modelled on `check_neg_of_not_parenthesized` (`:104-121`),
  comment it as ill-typed-so-fixtures-cannot-carry-it. Source clauses (params `x: i64`), expected descriptions:
  - `requires: -(x as u64?)` -> `requires -(x as u64?)`
  - `requires: -((x as u64)?)` -> `requires -(x as u64?)`
  - `requires: !(((x as u64)?)?)` -> `requires !(x as u64??)`
  - `requires: -x as u64?` -> `requires (-x) as u64?`   (the old/#1587 binding: cast of neg, then `?`)
  - `requires: -(x?)` -> `requires -x?`                 (negative control: no parens)
  - `requires: -(x? as u64)` -> `requires -(x? as u64)`
- Wire it into `main` as `r5`, error code base `900` (existing bases are 800 for r4; keep `code + 1 + i` scheme).
- Run: `build/vowc test compiler/tests/test_lower_contract_text.vow` (build/vowc must exist; otherwise
  `scripts/bootstrap.sh --skip-cargo`, ~5 min, background-free: run foreground with explicit timeout).
- Mutation check: temporarily drop the `question_chain_ends_in_cast(...)` term in `contract_text.vow:260`, confirm
  clauses 1-3 go red, revert.
- If the lowered Rust text for any clause differs from the expected text (both compilers must agree), the
  mismatch is drift: fix `contract_text.vow` to mirror the Rust printer (Rust is the reference for `parse->print->parse`).

### 3. Slice C — PR hygiene
- `git rm PLAN.md` (stage handoff artefact), commit tests as `test(syntax): pin ? chain over cast under unary in both printers`
  (lower-case, <= 92 chars), PR body `Closes #1591` plus a note that the production fix shipped in #1615 (77c65533).
- Comment on the issue explaining the issue was already fixed on `main` and what this PR adds.

## Verification surface
- No contract, codegen, IR or C-model change. ESBMC properties: none. No `tests/run/` or `examples/` fixtures.
- `contract_text.vow` is only touched if step 0/2 exposes drift; `compiler/` change would ride the bootstrap fixed point,
  so run `scripts/bootstrap.sh --skip-cargo --no-cache` and record the head SHA in the PR checklist (CLAUDE.md rule).
- Gates for the test-only change: `cargo test -p vow-syntax`, `cargo clippy --all --all-targets -- -D warnings`,
  `cargo fmt --all --check`, `build/vowc test compiler/tests/test_lower_contract_text.vow`.

## Risks
- Hidden different failure at HEAD (proptest is randomized): mitigated by step 0 with a large `PROPTEST_CASES`.
- `a as u64??`: `??` lexes as two `Question` tokens (`vow-syntax/src/lexer.rs:245`), so it reparses; the self-hosted
  lexer is expected to match — if slice B shows otherwise, drop the `??` clause and file a follow-up rather than widen scope.
- Self-hosted test runner caveat: `-x as u64?` clause relies on the self-hosted parser applying #1587 binding; if
  `build/vowc` is older than 7d42c57e the expected text differs — rebuild before trusting red/green.
- Environmental flakes (sandbox e2e SKIP-panics, bootstrap contention) are unrelated; verify against clean origin/main.
- Test-only change to `compiler/tests/` is outside the fixed-point/codegen surface; no `HashMap`/layout concerns.

## Out of scope
- Any change to the printer's precedence model or parenthesisation of other shapes (e.g. postfix chains over
  `Question(Cast)` such as `(a as T?).f`), refactors of `print_postfix_base`, proptest generator changes, or
  fixture-table additions (`tests/fixtures/contracts/`) — those cannot type-check this shape.
- Re-adding the proptest seed `cc b262a39e...` (shrunk case already pinned at `:19`).
- Spec/CLI/skill/help regeneration (no language change).
