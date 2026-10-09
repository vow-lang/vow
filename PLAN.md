# Plan: thread nesting level through the canonical printer (#591)

## Goal
Make `vow-syntax/src/printer.rs` render block-bodied expressions (`if`/`else`, `while`, `for`, `loop`, block, `match`, loop `vow {}` clauses) at their real nesting depth, and keep the self-hosted contract-text mirror (`compiler/contract_text.vow`) byte-identical. Single first slice that closes the issue; no other printer behaviour changes.

## Assumptions
- `print_expr` is NOT source-only: `vow-ir/src/lower/vow.rs:26,230` use it for contract `description` (-> `VowViolation.description`, counterexample `violation`, `vow contracts` JSON), and `compiler/contract_text.vow` mirrors it "quirks included (a nested block restarts at indent level 0)" (header comment lines 4-11; `print_block_text` ~l.527; `print_loop_vow_text` ~l.480; `print_control_expr_text` ~l.346). Decision: fix the one printer and fix the mirror in the same PR (dual-compiler rule), rather than keeping a deliberately wrong column-0 mode. Contract descriptions containing nested blocks change text (multi-line only; single-line predicates are unaffected). Alternative considered: leave `print_expr` at fixed level 0 and add a level-aware sibling — rejected: two behaviours in one printer, keeps the documented quirk, and still leaves drift to explain.
- Public API stays `pub fn print_expr(expr: &Expr) -> String` = `print_expr_at(expr, 0)`; level-0 output of a top-level block/if/match is unchanged (body at 1, closer at 0), only constructs nested deeper than one block change.
- No self-hosted *source* printer exists (only `contract_text.vow`), so "update self-hosted printer" == update `contract_text.vow`.
- Parameter `where` predicates / `print_type` refinement predicates / `print_params` have no level; they stay at level 0 (a block inside a `where` is not realistic). Out of scope.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `vow-syntax/src/printer.rs` | add `level` to expression printing | `print_block_body` 211-220, `print_block` 222-228, `print_stmt` 230-255, `print_vow_block` 151-170, `print_const` ~353, `print_postfix_base` 489, `print_expr_with_parens` 506, `print_expr` 525-749 (Match 569, If 577, While 602, ForEach 634, Loop 667, Block 704), `print_match_arm` 782 |
| `compiler/contract_text.vow` | mirror: thread level through `print_expr_text`, `print_control_expr_text`, `print_block_text`, `print_stmt_text`, `print_loop_vow_text`, `print_expr_list_text`, `print_postfix_base_text`, `print_binop_operand_text`; fix header comment | 4-11, 70, 154, 167, 177, 346-455, 480-500, 502-545 |
| `tests/fixtures/contracts/contract_text_blocks.vow` / `.expected` | shared golden for both compilers + `vow-ir/tests/contract_text_forms.rs`; regenerate expected lines affected (e.g. line 3 `let c: i64 = if ...` nested in block; `{ { x > 0 } }`; match-arm blocks; loops) and update header comment ("restart at column 0") | whole files |
| `tests/fixtures/contracts/contract_text_{forms,atoms,postfix}.expected` | re-check; only multi-line nested entries change | grep `\\n` |
| `vow-syntax/tests/integration.rs` | add golden + roundtrip tests for nested indentation | `roundtrip` 397 |
| `vow-syntax/tests/proptest_arb.rs`, `proptest_roundtrip.rs` | add indentation oracle property | `arb_if_expr` 176, `arb_block_inner` 242 |
| `docs/spec/cli.md` | line ~520: replace "a nested block restarts at column 0 exactly as the canonical printer renders it" with nested-depth indentation wording | 510-521 |
| `compiler/main.vow` (~5834), `vow/src/skill.rs` | generated help/skill text embedding that sentence — regenerate, do not hand-edit | via `scripts/generate_help.py` |
| `docs/spec/index.md`/`grammar.md` | grep for "column 0"/indentation claims; update only if they contradict | — |

## Steps (TDD slices, red -> green, each its own commit)

### 1. Golden test for the reported repro (red)
- `vow-syntax/tests/integration.rs`: `nested_control_flow_indents_by_depth` — parse the issue's `while { if/else }` fn and the `if a>0 {1} else if a<0 {2} else {3}` fn, assert exact `print_module` text (inner `if` at 8 spaces, bodies at 12, closers aligned with their opener). Fails today.
- Also a `match` whose arm body is a block, a `loop vow { invariant: ... }` / a single-clause `while ... vow { invariant: ... }` (empty vow blocks may not parse) / `for ... vow {}` nested one level deep (clauses at level+1, closing `}` at the loop's own column — `} {` form), and a block-in-block statement.

### 2. Thread `level` through printer.rs (green)
- Rename body of `print_expr` to `fn print_expr_at(expr: &Expr, level: usize) -> String`; `pub fn print_expr(e) { print_expr_at(e, 0) }`.
- Add `level` to `print_postfix_base`, `print_expr_with_parens`, and pass it to every recursive child call (call/method args, index, unary, cast, assign, tuple, struct literal, enum construct, break/return values, `?`) so a block nested anywhere inside an expression inherits the right depth.
- `print_block_body`/`print_stmt` call `print_expr_at(expr, level)` (stmt level == the line's indent level).
- If: `print_block(then, level)`; else-if recurses `print_expr_at(else, level)`; else-block `print_block(b, level)`; condition at `level`.
- Match: `print_match_arm(arm, level + 1)`; arm takes the level and prints `print_expr_at(&arm.body, level)` where `level` is the arm's own indent; closing `}` is `indent(level)` (currently `out.push('}')` with no indent — must become `indent(level)`).
- While/ForEach/Loop: replace the three duplicated inline `vow { ... }` loops with `push_vow_block_inline(&mut out, v, level)` (printer.rs:175) followed by `out.push(' ')` — byte-identical to today at level 0 (` vow {\n`, clauses at `level+1`, closer `}` at `indent(level)`) — and `print_block(body, level)`; no-vow case keeps `out.push(' ')`. No new helper.
- `print_vow_block` (shared by fn-level and loop clauses): clause exprs via `print_expr_at(expr, level + 1)`; `print_const`: `print_expr_at(&c.value, level)`.
- Reuses: `indent()` printer.rs:95, `print_block` (already level-aware).

### 3. Contract-description goldens (red then green)
- Run `cargo test -p vow-ir --test contract_text_forms` -> expected-file mismatches for nested-block entries. Update `tests/fixtures/contracts/contract_text_blocks.expected` (and any of forms/atoms/postfix that change) by hand/diff review: only clauses with a block nested inside a block/arm/loop change; single-line and depth-1 ones must stay byte-identical (check with `git diff --stat` that unchanged lines are untouched). Fix the "restart at column 0" header comment in `contract_text_blocks.vow`.
- Add 3-4 deeper clauses to `contract_text_blocks.vow` (if-in-while-in-block, match arm containing block containing if, loop vow clause containing nested `if` predicate) so the parity table covers the new levels.

### 4. Mirror in `compiler/contract_text.vow` (green on parity)
- Add `level: i64` parameter to the functions listed above; indentation helper `indent_text(level) -> String` (4 spaces × level; build with a loop — Vow has no repeat builtin). Mirror step 2 exactly: arms at `level+1`, match closer at `indent(level)`, loop clauses at `level+1` / closer `} ` at `indent(level)`, block body via `print_stmt_text(a, sid, level+1)`. External callers `compiler/lower.vow:5626,5656,5685,5829` pass `0` (or keep a 2-arg wrapper `print_expr_text(a, eid) = print_expr_text_at(a, eid, 0)` to leave them untouched — preferred, mirrors Rust `print_expr`/`print_expr_at`).
- Update the header comment (drop "quirks included ... restarts at level 0") and the `print_block_text` doc comment.
- Run `build/vowc test compiler/tests/test_lower_contract_text.vow` and `scripts/full_test.sh` section `contract-text/parity` (both compilers vs the same `.expected`).

### 5. Property test (guards against regression the issue notes went undetected)
- `vow-syntax/tests/proptest_roundtrip.rs`: new property over `arb_module()`: every printed line's leading spaces == 4 × (brace depth at line start, minus 1 if the line starts with `}`), after stripping string-literal contents (including escaped quotes) from each line before counting braces, so braces inside literals cannot skew depth. Make sure `arb_block_inner` depth/`arb_expr_or_if` generate if-in-if and if-in-stmt; if generator never nests (check), bump depth minimally. Keep existing parse->print->parse idempotency property untouched and green.

### 6. Docs + generated text
- `docs/spec/cli.md` ~520: state that nested blocks, `if`/`match` arms and loop clauses indent by nesting depth (4 spaces/level, closer aligned with its opener), consistent in both compilers.
- `uv run python scripts/generate_help.py` (updates `compiler/main.vow` + `vow/src/skill.rs`), then `python3 scripts/check_help_coverage.py`.

## Testing / verification commands (run separately, background long ones, never the 2-min default timeout)
- `cargo fmt --all`, then `cargo clippy --all --all-targets -- -D warnings`, `cargo test -p vow-syntax`, `cargo test -p vow-ir`, `cargo test --all`.
- `cargo build --release -p vow` then `scripts/bootstrap.sh --skip-cargo --no-cache` (~5 min; binary fixed point, `contract_text.vow` is compiled into `build/vowc`).
- `build/vowc test compiler/` (or `--filter contract_text`/`lower_contract_text`).
- `scripts/full_test.sh` (~40 min): `contract-text/parity`, Section 2c verifier C parity (descriptions are embedded in emitted C/diagnostics; both emitters read the same description strings, so they must stay equal — mirror must be exact), `ops/catalogue-drift`, `check_help_coverage`.
- After the last push: record the head SHA with the bootstrap result in the PR checklist (per CLAUDE.md).
- Pre-existing failures to verify against clean main before blaming this change (see memory): `u64_marker_propagation`, `contracts_tmp_cleanup`, `concrete-block-region-parity`, sandbox SKIP-panics.

## External `print_*` consumers (surveyed, no truncation)
- `vow-ir/src/lower/vow.rs:26,230`: contract descriptions (in scope, step 3/4). `vow/src/main.rs:769` `print_declarations` (`vow decl` stubs: signatures + vow clauses only, no bodies; unaffected except multi-line nested predicates, which now indent correctly; no self-hosted twin — Rust-only subcommand today). `vow-syntax/src/parser/{expr.rs:1509,mod.rs:797}`, `vow-syntax/tests/*`, `vow-types/tests/proptest_typecheck.rs`: round-trip idempotency tests, unaffected. `vow/src/replay.rs` uses `print_type` only. `vow-ir::printer` is the IR printer, unrelated. No cache/hash keys depend on the syntax printer.

## Verification surface (ESBMC)
No contract/IR/codegen semantics change. Only text of `vow.description` strings for multi-line nested-block predicates changes. No new ESBMC properties. Need to confirm no `tests/verify*/` or `tests/debug/` fixture asserts a multi-line nested description (grep `\\n}` / `description` in `tests/` and `vow/tests`); update any hit in both compilers. No new `tests/run/` fixtures required; the contract-text parity fixtures grow instead.

## Risks
- Rust/Vow drift: contract descriptions must be byte-identical (Section 2c C parity, contract-text/parity). Mitigation: land both halves in one PR; shared `.expected` files are the oracle.
- Silent behaviour change for consumers of `description` (agents parse JSON): only nested multi-line cases; document in cli.md.
- Easy-to-miss spots: match closing brace (currently unindented), `else if` chain level, `print_postfix_base`/`print_expr_with_parens` paths carrying the level, loop clause closer `}` column, trailing-expression path in `print_block_body`. Step 1 tests cover each.
- Re-indenting the `print_expr` body shows as a large diff; keep the helper extraction in the same commit as the level change so reviewers see one logical change. Rust is covered by cargo tests (codecov/patch ok); `.vow` is uninstrumented.
- Fixed-point risk is low (no codegen ordering/BTreeMap/shim change), but `contract_text.vow` is part of the self-hosted binary, so bootstrap must be re-run.
- `cargo clippy -D warnings`: new `level` params must all be used; avoid needless `format!`.

## Out of scope
- Level for parameter `where` predicates and `print_type` refinement predicates.
- Any other printer formatting change (line wrapping, struct literal layout, comments), verifier changes, adding a self-hosted source formatter, or changing `print_function`/IR printers.
- Broad proptest generator rework beyond what the indentation property needs.
