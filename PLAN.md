# Plan: accept loop-local create-and-consume in Rust linear checker (#650)

## Goal
`vow-types/src/linear.rs::consume_var` must flag "consumed inside a loop" only for linear values declared at a shallower loop depth than the consumption site. Loop-local `let h: Handle = open(); close(h);` must be accepted, matching the self-hosted compiler.

## Assumptions
- Size: Small (1 source file + fixtures + 1 spec sentence). No subagents, no adversarial pass beyond the advisor-free self-review below.
- Self-hosted parity: **no `compiler/` code change.** The self-hosted compiler has no AST-level `in_loop` check (`grep in_loop compiler/*.vow` → 0 hits). It already accepts the program through the IR region pass (`compiler/region.vow` `linear_*` liveness + `linear_emit_consumed_error`, `EC_LINEAR_TYPE_VIOLATION`). The Rust fix aligns Rust to it; CLAUDE.md's dual-compiler rule is satisfied because the self-hosted side is already correct. The PR must prove this with a shared `tests/run/` fixture executed by both compilers (full_test.sh Section 4 runs both). If the fixture unexpectedly fails on the self-hosted compiler, stop and re-scope (file a follow-up, do not land Rust alone).
- Representation: store the declaration depth in a separate `HashMap<String, u32> decl_depth` on `LinearTracker` rather than inside `ConsumeState::Available`. Reason: `ConsumeState` is pattern-matched/merged in ~15 places (`merge_branch_state`, `check_if_branches`, `check_match_arms`, backstop loop at linear.rs:55-90, test at :1076); adding a field to `Available` forces edits at every site for no semantic gain, since depth is immutable per binding. Alternative (issue's suggestion: field in `Available`) rejected as wider churn. `LinearTracker` is `Clone`, so branch trackers inherit the map and merges need no change.
- Shadowing: `vars` is keyed by name with no scope pop (pre-existing). A loop-local `let h` that shadows an outer `h` overwrites both `vars["h"]` and `decl_depth["h"]`; after the loop the outer binding's state is already lost today. Not widened by this change; documented as out of scope.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `vow-types/src/linear.rs` | tracker struct, registration, loop arms, `consume_var`, unit tests | struct 18-30, params 41-47, `register_pattern_linear` 169-181, While/ForEach/Loop arms 232-255, `consume_var` 301-355, tests 770, 974, 1444 |
| `tests/run/linear_loop_local_consume.vow` [new] | positive fixture, run by both compilers | — |
| `tests/error/linear_loop_outer_consume.vow` [new] | negative guard: outer value consumed in loop still rejected with LinearTypeViolation | — |
| `docs/spec/errors.md` | LinearTypeViolation wording (line ~104 and ~290: "consuming it inside a loop that may execute more than once") | 104, 290 |
| `compiler/main.vow`, `vow/src/skill.rs`, `skills/vow/reference/errors.md` | generated copies of the errors.md text | only if spec wording changes → regenerate with `uv run python scripts/generate_help.py` |

## Steps (TDD slices)

### 1. RED — Rust unit test: loop-local create+consume accepted
- **File**: `vow-types/src/linear.rs` `#[cfg(test)]` (next to `test_linear_inside_loop_error` :770; reuse `make_env_with_linear_struct`, `block_with_expr`, `call_with`, `make_fn_def`, `TestEmitter`).
- **Test** `test_loop_local_linear_create_and_consume_ok`: fn body = `loop { let h: FileHandle = <init>; consume(h); }` (Stmt::Let with `ty` = `named_type("FileHandle")`, init any non-linear-ident expr e.g. call `open()`; then Stmt::Expr `consume(h)`). Assert `emitter.0.is_empty()`. Fails today with the "loop" error.
- Variants (same slice, each its own `#[test]`): same body inside `While` and `ForEach`.

### 2. GREEN — depth tracking
- **File**: `vow-types/src/linear.rs`.
- Replace `in_loop: bool` with `loop_depth: u32` (init 0); add `decl_depth: HashMap<String, u32>`.
- Param registration (:45): `decl_depth.insert(name, 0)`. `register_pattern_linear` (:177): `decl_depth.insert(name, tracker.loop_depth)`.
- While/ForEach/Loop arms: `let saved = tracker.loop_depth; tracker.loop_depth = saved + 1; …; tracker.loop_depth = saved;` (three near-identical arms: extract a tiny `check_loop_body(body, tracker, …)` helper to avoid triple duplication — allowed, it removes repetition inside the touched code).
- `consume_var` Available arm: emit the loop error iff `tracker.loop_depth > tracker.decl_depth.get(name).copied().unwrap_or(0)`.
- Keep message/hint unchanged ("cannot be consumed inside a loop (would be consumed multiple times)") — still accurate for outer values.

### 3. Guard tests (should already pass; confirm they stay green)
- Existing `test_linear_inside_loop_error`, `test_while_loop_linear_in_body_error`, `test_break_with_value_consumes_linear` (param declared depth 0, consumed depth 1 → error). Fix the stale comment in the last one only if the edit touches it (otherwise leave).
- New `test_nested_loop_inner_decl_consumed_in_outer_loop_error`-style case: `loop { loop { let h: FileHandle = open(); } consume(h); }` is not expressible due to scoping — skip. Instead add `test_outer_loop_decl_consumed_in_inner_loop_error`: `loop { let h = open(); loop { consume(h); } }` → exactly 1 error (decl depth 1 < consume depth 2).
- `test_loop_local_double_consume_still_error`: `loop { let h = open(); consume(h); consume(h); }` → "already consumed" (1 error), proves only the loop check was relaxed.

### 4. End-to-end fixtures (both compilers)
- `tests/run/linear_loop_local_consume.vow` [new]: `// TEST: stdout "..."`; `linear struct Handle { fd: i64 }`, `fn open(fd: i64) -> Handle`, `fn close(h: Handle) -> i64 { let fd: i64 = h.fd; drop(h); fd }`; `main` runs `while i < 3 { let h: Handle = open(i); total = total + close(h); i = i + 1; }`, prints total. Also a `for`-less `loop { … break }` variant inside the same file. Use only constructs already used by `tests/run/linear_match_binding_shadow.vow` (`drop`, struct literal) to avoid unrelated gaps.
- `tests/error/linear_loop_outer_consume.vow` [new]: header `// TEST: error-code LinearTypeViolation` + `// TEST: error-count 1` (format per `tests/error/drop_twice.vow`); outer `let h` consumed in `while` body. Run against both compilers in full_test.sh error section — **verify the self-hosted compiler produces the same code/count**. Self-hosted may report "already consumed" via region dataflow or accept it (gap noted by audit docs/audit-20260610 line 111). If the self-hosted compiler does not reject it, do NOT add this error fixture to the shared dir; keep the Rust unit test only and note the pre-existing divergence in the PR body (separate follow-up issue; out of scope).
- Check whether tests/error fixtures are Rust-only or dual by reading `scripts/full_test.sh` error section before adding.

### 5. Spec/doc
- `docs/spec/errors.md:~104,~290`: the wording "consuming it inside a loop that may execute more than once" stays correct but should say "a linear value declared outside the loop". If edited: run `uv run python scripts/generate_help.py` and commit regenerated `compiler/main.vow`, `vow/src/skill.rs`, `skills/vow/reference/errors.md`; run `python3 scripts/check_help_coverage.py`. If wording is judged already adequate, skip (no spec change is strictly required: this fixes a false positive without altering the language contract). Recommendation: make the one-phrase clarification — it states the depth rule agents must predict.

## Testing / verification commands
- `cargo test -p vow-types linear` (fast loop for slices 1-3)
- `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all`
- `cargo build --release -p vow`; `./target/release/vow --no-verify tests/run/linear_loop_local_consume.vow -o $TMPDIR/ll && $TMPDIR/ll`
- `scripts/bootstrap.sh --skip-cargo` (background, poll; ~5 min), then `build/vowc build --no-verify tests/run/linear_loop_local_consume.vow -o $TMPDIR/ll2 && $TMPDIR/ll2`; stdout must match.
- Targeted slice of `scripts/full_test.sh` (Sections 4 and error section) for the two new fixtures; full run is ~40 min — run in background with polling, cap `-j`/cargo jobs (`CARGO_BUILD_JOBS=4`), scratch under `$TMPDIR`.
- Use `VOW_CACHE_DIR=$(mktemp -d)` when validating (stale compile cache).

## Verification surface (ESBMC)
No contracts/codegen/C-model/IR change → nothing for ESBMC to prove; `c_emitter.{rs,vow}` parity untouched. Fixture functions carry no `vow` blocks. Bootstrap fixed point unaffected (no `compiler/` edit unless step 5 regenerates help text in `compiler/main.vow`, which does change the binary but deterministically).

## Risks
- Self-hosted compiler might reject/mishandle the new run fixture (e.g. region pass on `while` with loop-local linear origin). Mitigation: step 4 runs it on both before opening the PR; if it fails, that is a real parity bug to fix in `compiler/region.vow` in the same PR or re-scope — not to be papered over.
- Soundness: loop-local binding consumed once per iteration is sound; a loop-local value that is *not* consumed is still caught by the end-of-fn backstop (linear.rs:55-90) and the region pass. `continue`/`break` paths not tracked by this checker today; unchanged.
- Depth map staleness after inner loop ends: a stale inner `decl_depth` entry for a name that no longer exists in scope cannot cause a false negative because the tracker's `vars` entry is equally stale (same lifetime).
- Generated-text drift (step 5) fails `check_help_coverage.py`/CI if sources are not regenerated together.
- Known pre-existing flakes (see memory): run-test SKIP-panics in sandbox, `concrete-block-region-parity`, `u64_marker_propagation`/`contracts_tmp_cleanup`; verify against clean `origin/main` before blaming this change.

## Out of scope
- Adding an AST-level linear checker to the self-hosted compiler or closing the audit's self-hosted double-consume gap (docs/audit-20260610 line 111).
- Fixing shadowing/scope-pop in the tracker; changing messages; `ConsumeState` refactor; unrelated clippy/format cleanups.
- PR title suggestion: `fix(types): accept loop-local linear create-and-consume in linear.rs` (lower-case, <92 chars). Commit scope `types`.
