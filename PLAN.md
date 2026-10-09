# Plan: Pin `?` operand parenthesization with regression tests (issue #594)

## Goal
Issue #594 (`(a + b)?` printed as `a + b?`) is already fixed in production code in both compilers;
the remaining work is the missing regression coverage that would have caught it and will keep it fixed.
No production change is planned unless a new test exposes a real gap.

## Findings (verified on HEAD 70e1d0eb)
- Rust: `vow-syntax/src/printer.rs:682` — `ExprKind::Question { expr } => format!("{}?", print_postfix_base(expr, level))`.
  `print_postfix_base` (`printer.rs:511-526`) parenthesizes block-like, `BinaryOp`, `UnaryOp`, `Assign`, `Break`, `Return`.
  Landed in 1a3a8ed3 (#1488).
- Self-hosted: `compiler/contract_text.vow:316-321` (`EXPR_QUESTION` → `print_postfix_base_text`, `contract_text.vow:166-174`)
  mirrors it with the same tag set. Same commit.
- Parser: `vow-syntax/src/parser/expr.rs:83-130` — `?` is a postfix op in `parse_expr_inner`, binds tighter than infix ops.
  `Cast` as `?` operand needs no parens: `x as i64?` reparses as `Question(Cast(x))` (postfix loop continues after `as`), so omitting `Cast` from `print_postfix_base` is correct.
- Coverage gap: no printer unit test prints a `Question` (only `test_cast_parens_binary_operand`, `printer.rs:1241`, covers Cast);
  `vow-syntax/tests/proptest_arb.rs` has no generator producing `ExprKind::Question`
  (`proptest_roundtrip.rs:129` only strips it); `tests/fixtures/contracts/contract_text_postfix*.vow` contain no `?` (grep -F '?' empty);
  `compiler/tests/test_lower_contract_text.vow` has no `?` check.
- `?` appears in `tests/run/*.vow` only as plain `call()?` / `({ ... })?`; no print/reparse check.

## Assumptions
- The issue is a stale audit finding (written 2026-06-10, fixed by #1488); the implementation PR is tests-only and should say "closes #594" with a note that the fix pre-exists (best guess).
- `?` inside a `vow` clause may be rejected by the checker, so the parse-parity fixture route (Slice 3) is attempted first; if `build/vowc contracts` / `vow contracts` rejects it, fall back to a Vow unit test (best guess).

## Key Files
| File | Role | Lines |
|------|------|-------|
| `vow-syntax/src/printer.rs` | add unit tests in `mod tests` next to `test_cast_parens_binary_operand` | 511-526, 682, 1241 |
| `vow-syntax/tests/proptest_arb.rs` | add `arb_question_expr`, wire into `arb_expr_inner` / `arb_expr_or_if` | 78-111, 199 |
| `vow-syntax/tests/proptest_roundtrip.proptest-regressions` | persisted seeds, only if proptest finds a failure | - |
| `tests/fixtures/contracts/contract_text_postfix.vow` / `_canonical.vow` / `contract_text_postfix.expected` | cross-compiler parse-parity rows for `?` | whole files |
| `compiler/tests/test_lower_contract_text.vow` | fallback Vow-side check if fixtures cannot carry `?` | 57-110 |
| `scripts/full_test.sh` | runs the fixtures (no edit expected) | 2038-2075 |

## Steps (TDD slices; each is a separate small commit)

### 1. Rust printer unit tests (red-green guard)
- **File**: `vow-syntax/src/printer.rs` `mod tests`, after `test_cast_parens_binary_operand` (~line 1251).
- **Change**: add `Question`-wrapping helper-based tests using existing `ident_expr`, `binop_expr`, `named_ty`, `s()`:
  `Question(a + b)` → `"(a + b)?"`; `Question(-a)` (UnaryOp) → `"(-a)?"`; `Question(Assign)` → parenthesized;
  `Question(Call f(x))` → `"f(x)?"`; `Question(Ident)` → `"a?"`; `Question(Question(a))` → `"a??"`;
  `Question(Cast(a, u64))` → `"a as u64?"`; `BinaryOp(a, +, Question(b))` → `"a + b?"` (distinguishes the two ASTs from the issue).
- **Also**: one round-trip test: parse `(a + b)?`, `print_expr`, reparse, assert top-level kind is still `Question` (use `parse_expr` helper in `vow-syntax`; locate the existing one via `grep -n "fn parse_no_errors" vow-syntax/src/parser/expr.rs`; if not importable from printer tests, put this test in `vow-syntax/tests/integration.rs`).
- Mutation check by hand: temporarily replace `print_postfix_base` with `print_expr_at` in the Question arm, confirm the new tests fail, revert.

### 2. Proptest generator for `Question`
- **File**: `vow-syntax/tests/proptest_arb.rs`.
- **Change**: add `arb_question_expr(depth)` = `arb_expr_inner(depth).prop_map(|e| Expr{ kind: Question{expr: Box::new(e)}, span: z() })`; add weight `1 =>` in `arb_expr_inner` and `arb_expr_or_if`. Because operands include `BinaryOp`/`UnaryOp`, the existing `proptest_roundtrip.rs` property now fails if parens are dropped.
- **Caveat**: `if`-containing operands are only generated at top level (see comment at line 85); `arb_expr_or_if` wrapping `Question` around `If` is an unparenthesized-block-like case — keep `Question` out of `arb_expr_or_if` if it produces unparseable shapes the printer already parenthesizes; verify by running the proptest suite.
- Run: `cargo test -p vow-syntax --test proptest_roundtrip`.

### 3. Cross-compiler parse-parity rows
- **Files**: `tests/fixtures/contracts/contract_text_postfix.vow`, `contract_text_postfix_canonical.vow`, `contract_text_postfix.expected`.
- **Change**: add clauses (new fn `questions`) spelled with redundant parens, e.g. `((a + b))?`, `(-x)?`, `(x?)?`, `(f(x))?`, `(x as u64)?`; canonical twin holds the already-canonical text; append escaped expected lines in declaration order. Both compilers must print the same text (checked by `scripts/full_test.sh` ~2042 and `vow-ir/tests/contract_text_forms.rs`).
- **Gate to check first**: run `./target/release/vow contracts <fixture>` (needs `cargo build --release -p vow -j2`) and `build/vowc contracts <fixture>` (needs `scripts/bootstrap.sh --skip-cargo`); if the checker rejects `?` in a `vow` clause, drop this slice and implement Step 4 instead.
- Run: `cargo test -p vow-ir --test contract_text_forms`.

### 4. (Fallback only) Vow unit test
- **File**: `compiler/tests/test_lower_contract_text.vow` — add `check_question_parens()` using `expect_descriptions` (lines 34-55) with a source whose contract clauses contain `(a + b)?`; register it in `main` (line 102) with a new error code range.
- Run: `build/vowc test compiler/tests/test_lower_contract_text.vow`.

### 5. Close-out
- Implementation stage: `git rm PLAN.md`, PR title `test(syntax): pin parenthesization of the ? operand (#594)` (lower-case subject, ≤92 chars), body says the fix pre-exists via #1488 and this adds regression coverage; `gh issue comment 594` noting the same.
- No `docs/spec/*.md` change: grammar and canonical form are unchanged. Check `docs/spec/grammar.md` Canonical Form mentions nothing contradicting `(a + b)?`.

## Testing / gates (separate commands, never `&&`-chained; background/poll long ones)
- `cargo fmt --all -- --check`
- `cargo clippy --all --all-targets -- -D warnings` (test lints gate; avoid `noUncheckedIndexedAccess`-style indexing in tests)
- `cargo test -p vow-syntax`
- `cargo test -p vow-ir --test contract_text_forms` (if Step 3 lands)
- `scripts/bootstrap.sh --skip-cargo --no-cache` only if a `.vow` file under `compiler/` changes (Step 4); record the head SHA.
- Build with `-j2`; use `VOW_CACHE_DIR=$(mktemp -d)` if validating codegen.

## Verification surface
No contracts, codegen, IR, or C-model change; ESBMC and `c_emitter.{rs,vow}` untouched; no `tests/run/` or `examples/` growth needed.

## Risks
- Proptest flakiness: new `Question` weights may surface other latent print/reparse mismatches (e.g. Question around `If`/`Match`). If so, constrain the generator to the operand kinds in scope, and file a follow-up issue rather than widening this PR.
- Parity fixture edits must keep Rust and self-hosted output byte-identical; a mismatch means a real drift bug in one `print_postfix_base*` — in that case fix both compilers in the same PR.
- `codecov/patch`: tests-only diff adds no uncovered production lines; avoid re-indenting existing lines.
- Binary fixed point unaffected unless `compiler/*.vow` production code changes (not planned).

## Out of scope
- Any printer/parser refactor (e.g. unifying `print_postfix_base` with the Cast operand list), formatting changes, other postfix operators' coverage beyond `?`, and unrelated audit findings from `docs/audit-20260610/`.
