# Plan: make the canonical printer emit `||` for refinement types (#620)

## Goal
Make `parse -> print -> parse` idempotent for `Type::Refinement` by changing the printer to emit the `||` separator the Rust parser already requires, and cover the form in the proptest round-trip.

## Assumptions
- Chosen fix: printer emits `||` (parser unchanged). Rejected: parser accepting `|`. Reasons: (a) parser, checker error text (`vow-types/src/env.rs:857`), tests (`types.rs:466`), comments and the audit all use `||`; (b) the self-hosted parser has no `{` type branch at all (`compiler/parser.vow` `parse_type`, `compiler/contract_text.vow:19` documents "parse error in both compilers"), so changing the Rust grammar to `|` would create NEW drift against `compiler/` and add a grammar axis; keeping `||` leaves the two parsers' behaviour unchanged; (c) the type is fail-closed at type-check (`env.rs:855`), so no semantic surface changes. (best guess; issue permits either)
- Dual-compiler rule: no `compiler/*.vow` change is needed — the self-hosted printer (`compiler/contract_text.vow` `print_type_text`) intentionally has no refinement arm, and the self-hosted parser rejects the form. Nothing there prints `|`. Record this in the PR body.
- No `docs/spec/*.md` change: `grammar.md` does not document the refinement-type form (only `where` clauses); the syntax accepted by the parser is unchanged. No `--help`/skill regeneration.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `vow-syntax/src/printer.rs` | printer emits `|` → change to `||`; unit test hard-codes old output | 431-441, 1433-1447 (`test_type_refinement`) |
| `vow-syntax/src/parser/types.rs` | parser `expect(PipePipe)` (unchanged); existing test `type_refinement_with_pipepipe` | 107-124, 466-485 |
| `vow-syntax/tests/proptest_arb.rs` | `arb_type_inner` never builds `Type::Refinement` | 39-66; `arb_binop_expr` 150, `arb_expr_leaf` 112 |
| `vow-syntax/tests/proptest_roundtrip.rs` | `strip_type` already handles Refinement; round-trip properties | 13-35, 326-416 |
| `vow-syntax/tests/integration.rs` | has its own `strip_type` (Refinement arm at 24) | 24-35 |

## Steps (TDD slices, in order)

### 1. RED: fix the printer unit test
- **File**: `vow-syntax/src/printer.rs` `test_type_refinement` (~1447)
- **Change**: assert `"{ x: i64 || x > 0 }"`. Run `cargo test -p vow-syntax test_type_refinement` → fails.

### 2. GREEN: printer emits `||`
- **File**: `vow-syntax/src/printer.rs:435-440`
- **Change**: format string `"{{ {}: {} || {} }}"`. Test from step 1 passes.

### 3. RED→GREEN: explicit round-trip unit test
- **File**: `vow-syntax/src/parser/types.rs` tests module (next to `type_refinement_with_pipepipe`, ~466) or `vow-syntax/tests/integration.rs`
- **Change**: parse `{ x: i64 || x > 0 }` → `print_type` → parse again; assert no diagnostics and equal ASTs (spans stripped). Add a case whose predicate contains `||` (`{ x: i64 || x > 0 || x < 5 }`) and one with a nested generic base (`Vec<i64>`) to pin that the separator scan is unambiguous. Use the module-level `parse -> print -> parse` helper style already in `integration.rs` (its `strip_type` handles Refinement).
- Sanity: temporarily revert step 2 locally to confirm this test fails (red) before keeping green.

### 4. Proptest coverage of `Type::Refinement`
- **File**: `vow-syntax/tests/proptest_arb.rs` `arb_type_inner` (39-66)
- **Change**: add a `1 =>` arm: `(arb_ident(), arb_type_leaf(), arb_refinement_predicate())` → `Type::Refinement { binding, base: Box::new(base), predicate: Box::new(pred), span: z() }`. Predicate strategy: a small binop over `arb_expr_leaf()` (reuse `arb_binop_expr`/`arb_expr_leaf`, lines 112/150); avoid block-like expressions (`if`, blocks, struct literals) to keep predicates trivially re-parseable inside `{ ... }` and avoid unrelated pre-existing printer/parser gaps. Include `||`-containing predicates (BinOp::Or is in `arb_binop`).
- **Reuses**: `strip_type`'s Refinement arm in `proptest_roundtrip.rs:21`.
- **Check**: `arb_type()` feeds `let` annotations (229), params (317), fn return types (329), struct fields (364); verify the round-trip passes in each position, notably a refinement return type followed by the body `{` and a refinement in `let x: {..} = e;`. If a position fails for a reason unrelated to the separator (e.g. pre-existing ambiguity), restrict the new arm to param/return/field positions via a separate `arb_type_with_refinement()` and file a follow-up issue rather than widening this fix.
- Run `cargo test -p vow-syntax --test proptest_roundtrip` (with a few extra cases via `PROPTEST_CASES=2000`).

### 5. Quality gate (each as a separate command)
- `cargo fmt --all -- --check`, `cargo clippy --all --all-targets -- -D warnings`, `cargo test -p vow-syntax`, `cargo test --all` (note memory: ~8 vow-crate run tests may fail in sandbox for environmental reasons; compare against clean origin/main).
- `build/vowc` is unaffected (no compiler/ change); no bootstrap re-run required. Still run `rg '\{ *[a-z_]+: [A-Za-z0-9<>]+ \| ' --glob '!target' .` to confirm no other place emits/expects the single-pipe form (tests/, docs/, examples/, scripts/).

### 6. PR
- Title: `fix(syntax): print refinement types with || so parse-print-parse round-trips` (<92 chars, lower-case subject). Body: option chosen and why, no `compiler/` change and why (self-hosted parser/printer have no refinement type), closes #620. `git rm PLAN.md` first.

## Testing
- Changed: `printer::tests::test_type_refinement`. New: round-trip unit test (step 3), proptest refinement arm (step 4). Existing `type_refinement_with_pipepipe` and `resolve_refinement_predicate_fails_closed` (`vow-types/src/env.rs:1283`) must stay green.

## Verification surface
No contracts, IR, codegen or C model touched; no ESBMC properties; no `tests/run/` or `examples/` fixtures need to grow (refinement types are rejected by the type checker, so they cannot appear in a runnable fixture). Verifier C parity unaffected.

## Risks
- Proptest generator exposing unrelated printer/parser gaps for refinement predicates (e.g. precedence/paren printing, `let` position): mitigate with the restricted predicate strategy and position fallback in step 4; do not fix unrelated bugs in this PR.
- Binary fixed point / `BTreeMap` ordering / clif shim: untouched (Rust `vow-syntax` only, no `compiler/` edits).
- Stale compile cache: irrelevant (no codegen change).
- clippy `-D warnings` incl. test targets is enforced in CI; keep the new strategy free of unused imports/needless clones.
- Two parsers diverge on the form (Rust accepts, self-hosted rejects): pre-existing, tracked by the audit; not widened by this change.

## Out of scope
- Making the parser also accept `|`; any semantic support for refinement types (predicate forwarding to obligations); adding `{`-type parsing to `compiler/parser.vow`; `where`-clause changes; spec/help regeneration; closing the Rust/self-hosted parser gap.
