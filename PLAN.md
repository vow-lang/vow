# Plan: re-arm a linear local on assignment in the AST linear tracker (#657)

## Goal
`let mut h: Handle = open(); consume(h); h = open(); consume(h);` is accepted by the AST
linear pass (`vow-types/src/linear.rs`). Today the second `consume(h)` hits the stale `Consumed`
state and emits a spurious `LinearTypeViolation: linear value 'h' already consumed`, although the
IR region pass (both compilers) already accepts the program.

## Assumptions
- **Re-arm only; no leak diagnostic at the assignment** (deviates from the issue's "Proposed fix"
  second half). The AST tracker cannot tell whether an `Available` local holds a live obligation:
  `let mut v: Option<Token> = Option::None; if c { v = Option::Some(..) }` is registered
  `Available` (annotated linear-owner type) yet holds no obligation. Flagging the overwrite would
  false-positive on every such program and change the code of existing
  `tests/error/linear_*_mutation_phi_leak.vow` fixtures. The "dual problem" (assign over a live
  value leaks it) is, as the issue itself notes, already reported by the region pass, which models
  reassignment as a fresh origin. Alternatives rejected: (a) emit only for non-enum `linear struct`
  locals - duplicates `RegionLinear` under a second code for no soundness gain; (b) emit for all -
  unsound, see above. (best guess)
- **No self-hosted code change.** `compiler/` has no AST-level linear tracker; its only linear
  analysis is the IR dataflow in `compiler/region.vow` (`check_linear_regions_*`), which is the
  pass the issue says already handles reassignment. `linear.rs` is the sole AST pass, so there is
  no Vow twin to edit and the Rust-only edit does not widen drift. Parity is proven by one shared
  `tests/run/` fixture that `scripts/full_test.sh` Section 4 builds with both compilers. If
  `build/vowc` rejects that fixture, that is a `region.vow` bug and must be fixed in the same PR
  (dual-compiler rule). (best guess; verified by reading, not by running)
- Params are not assignable (`ast::Param` has no `mut`; `check.rs:3140` emits
  `ImmutableAssignment`), so the issue's `fn f(h: Handle) { h = ..}` example is only reachable via
  `let mut` locals. Param-span backstop exemption (`linear.rs:55-71`) is therefore unaffected.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `vow-types/src/linear.rs` | AST linear tracker; fix lives in `check_expr` `ExprKind::Assign` arm | 220-223 (arm), 268-319 (`consume_var`), 54-78 (end-of-fn backstop), 464-485 (tests start at ~554, last test 1467) |
| `vow-types/src/check.rs` | Calls `check_linear_usage` after body check; assignment typing/`mut` rule (context only) | 1730, 3140-3166 |
| `compiler/region.vow` | Self-hosted IR linear dataflow (context only, no edit expected) | 327-454 |
| `vow-ir/src/region.rs` | Rust IR linear dataflow (context only) | 181 |
| `tests/run/linear_reassign_after_consume.vow` [new] | Dual-compiler accept fixture | - |
| `docs/spec/grammar.md` | Linear Structs section: add one sentence on reassignment | ~921-925 |
| `vow/src/skill.rs`, `compiler/main.vow` | Generated embedded skill/help text (copies the Linear Structs paragraph, e.g. `skill.rs:2322`, `main.vow` twice); regenerated, never hand-edited | generated |

## Steps (TDD order)

### 1. RED+GREEN: re-arm after consume (core fix)
- **File**: `vow-types/src/linear.rs`, `mod tests` (append after `test_assign_rhs_consumes_linear`, ~line 1485)
- **Test** `test_assign_after_consume_rearms_linear`: body `let mut h: FileHandle = open; consume(h); h = open; consume(h);`.
  Build `Stmt::Let` as in `test_let_stmt_registers_linear_type` (`linear.rs:832`, use `is_mut: true`),
  `Stmt::Expr` for the call, an `ExprKind::Assign { lhs: ident("h"), rhs: ident("open") }` stmt, then
  trailing `call_with("consume","h")`. Expect `emitter.0.is_empty()`. Fails today with "already consumed".
- **Change** (`linear.rs:220-223`): keep the existing LHS (non-consuming) then RHS (consuming)
  visits, then add: if `lhs.kind` is `ExprKind::Ident(name)` and `tracker.vars.contains_key(name)`,
  `tracker.vars.insert(name.clone(), ConsumeState::Available(lhs.span))`. RHS **must** be visited
  first so `h = wrap(h)` consumes the old value before the re-arm.
  Factor as a small `rearm_var(name, span, tracker)` next to `consume_var` only if it reads cleaner;
  do not widen the interface otherwise.

### 2. Pin RHS-before-rearm ordering (self-reference)
- **Test** `test_assign_self_wrap_rearms_once`: `let mut h: FileHandle = open; h = wrap(h); consume(h);`
  (`wrap` modelled as `call_with("wrap","h")` as the Assign RHS). Expect no diagnostics. Second
  case (same file): omit the final `consume` and expect exactly one `"never consumed"` diagnostic from
  the end-of-function backstop, proving the re-armed value is tracked from scratch.
- No production change beyond slice 1; this guards against a later "leak check before RHS" regression.

### 3. Pin the no-leak-diagnostic decision
- **Test** `test_assign_over_available_is_deferred_to_region_check`: `let mut h: FileHandle = open; h = open2; consume(h);`
  expects no diagnostics from `check_linear_usage` (mirrors the existing
  `*_deferred_to_region_check` naming at `linear.rs:744, 1042, 1108`). Comment-free; the name carries the intent.

### 4. Branch merge semantics preserved
- **Test** `test_assign_in_else_less_if_after_consume_stays_consumed`: `consume(h); if c { h = open; } consume(h);`
  expects exactly one `"already consumed"`. The else-less path in `check_if_branches` (`linear.rs:~416-425`)
  never calls `merge_branch_state`; it only upgrades an outer `Available`, so the outer `Consumed` is kept
  (the program is genuinely invalid: `c` may be false). No production change.
- **Test** `test_assign_in_one_arm_with_else_after_consume_is_maybe_consumed`: `consume(h); if c { h = open; } else {}; consume(h);`
  expects exactly one `"may already be consumed"` (`Available`/`Consumed` via `merge_branch_state`, `linear.rs:~354`).
- **Test** `test_assign_in_both_branches_after_consume_rearms`: `consume(h); if c { h = open; } else { h = open; } consume(h);`
  expects no diagnostics. Build `if` per `test_linear_in_both_branches_no_error` (`linear.rs:710`).

### 5. Non-ident LHS must not re-arm
- **Test** `test_field_assign_does_not_rearm`: `consume(h); h.fd = 1; consume(h);` (LHS
  `ExprKind::FieldAccess`) expects exactly one `"already consumed"`.

### 5b. Shadowing guard (name-keyed tracker)
- Problem the fix would introduce: `consume(h); let mut h: i64 = 0; h = 1;` - the assign re-arms the
  stale `Consumed` linear entry for `h`, and the end-of-function backstop then reports "never consumed" on a
  valid program.
- **Test (red first)** `test_shadowing_nonlinear_let_over_consumed_linear_not_rearmed`: `let mut h: FileHandle = open; consume(h);
  let mut h: i64 = 0; h = 1;` expects no diagnostics. Needs a non-linear annotation type (`named_type("i64")` or the
  existing helper used in `test_let_stmt_non_linear_type_not_tracked`, `linear.rs:866`).
- **Change** (`register_pattern_linear`, `linear.rs:~161-176`): for `PatKind::Ident`, when the binding is not
  linear and `tracker.vars.get(name)` is `Some(ConsumeState::Consumed(_))`, remove that entry. Do **not** remove
  `Available`/`MaybeConsumed` entries (a shadowed live obligation is a real leak and stays reported).

### 6. Dual-compiler fixtures
- **`tests/run/linear_reassign_after_consume.vow`** [new]. Header `// TEST: exit 0` (add
  `// TEST: stdout` only if it prints; prefer a pure program returning 0). Module
  `LinearReassignAfterConsume`; `linear struct Handle { fd: i64 }`; helpers `open(fd) -> Handle`,
  `close(h: Handle) -> i64 { let fd: i64 = h.fd; drop(h); fd }`; `main` cases: reassign after
  consume; reassign in both branches of an if/else after consume; `h = wrap(h)` round trip; shadowing
  `let mut h: i64` after consume (slice 5b). Follow `tests/run/linear_region_ok.vow`. No reassign inside loops.
- **`tests/error/linear_reassign_overwrites_live.vow`** [new, required]: `let mut h: Handle = open(1);
  h = open(2); drop(h);` -> `// TEST: error-code RegionLinear`. After the fix the AST pass reports nothing here,
  so this is the only guard on the issue's "dual problem" in both compilers. Before trusting it, the
  implementer must confirm how CI gates `tests/error/` (`error-code` is parsed in `tests/run_tests.sh:240`;
  grep `scripts/full_test.sh` Section 5/6 for the equivalent) and run it with both compilers.
- **`tests/error/linear_reassign_in_loop_zero_iter_double_consume.vow`** [new, required]: `drop(h);
  while i < n { h = open(2); i = i + 1; } drop(h);` (with `h`, `i`, `n` as `let mut`). The re-arm makes the
  AST pass accept this (loop body runs on the outer tracker with no merge, `linear.rs:~175-200`; today it is
  rejected as "already consumed"), so the region pass must still reject the zero-iteration double consume in
  **both** compilers. Expected code: observe empirically (`RegionLinear`). If either region pass accepts it,
  that blocks the fix: handle loops in this PR (e.g. in the `While`/`ForEach`/`Loop` arms, after the body,
  merge pre-loop and post-body trackers with `merge_branch_state` so a zero-iteration path is modelled) and
  record it in the PR body.
- **`tests/error/linear_reassign_after_consume_leak.vow`** [new, optional]: `let mut h: Handle = open(1);
  drop(h); h = open(2);` (re-armed value never consumed). After the fix both the AST backstop
  (`LinearTypeViolation`, at the assignment span) and the region pass (`RegionLinear`) report it, as for the
  plain `let h: Handle = ..; 0` case (`tests/error/linear_region_unconsumed.vow`). Pin the observed code only
  if stable across both compilers; otherwise skip.
- Run all via `scripts/full_test.sh` Sections 4-5 (both compilers); this is the parity gate.

### 7. Spec + regenerate embedded text (mandatory)
- **File** `docs/spec/grammar.md` (Linear Structs, after the paragraph ending "`RegionLinear`." ~line 925):
  add one sentence: assigning a new value to a `mut` linear local after its previous value was
  consumed starts a fresh obligation that must itself be consumed; overwriting a live
  (unconsumed) value discards it and is reported as `RegionLinear`.
- The paragraph is embedded verbatim in `vow/src/skill.rs` and `compiler/main.vow`. Run
  `uv run python scripts/generate_help.py` (rewrites both), then `cargo build --release -p vow`, then
  `scripts/bootstrap.sh --skip-cargo --no-cache`, and `python3 scripts/check_help_coverage.py`.
  Commit the regenerated files in the same PR (a `docs/spec` change without them fails drift gates).

## Testing / verification commands
- `cargo test -p vow-types linear::` (new + existing unit tests)
- `cargo test -p vow --test effect_gating` (linear double-consume gate stays red→build fail)
- `cargo clippy --all --all-targets -- -D warnings`; `cargo fmt --all`
- `scripts/bootstrap.sh --skip-cargo --no-cache`, then
  `build/vowc build --no-verify tests/run/linear_reassign_after_consume.vow -o $TMPDIR/lr && $TMPDIR/lr`
  and the same with `./target/release/vow`; both must exit 0.
- `VOW_CACHE_DIR=$(mktemp -d)` when validating (stale compile cache gotcha).
- `scripts/full_test.sh` Section 4 + `tests/error/linear_*` (all existing error fixtures must keep
  their codes). Background + poll; wall-clock ~40 min.
- Known pre-existing failures (verify on clean main before blaming this change): run-test SKIP-panics
  without a linked runtime, `u64_marker_propagation`, `contracts_tmp_cleanup`,
  `concrete-block-region-parity`.

## Verification surface (ESBMC / C model)
None. No contract, codegen, IR, runtime, or C-emitter change; `c_emitter.{rs,vow}` byte-parity and
`verify*` fixtures are untouched. The new `tests/run/` fixture is a build+run test, not a verify
fixture; contracts in it (if any) must be the true semantic ones.

## Risks
- **Silent diagnostic-code drift**: the backstop (`linear.rs:54-78`) now sees re-armed locals as
  `Available(assign_span)` and may report "never consumed" at the assignment span where previously
  only the region pass reported the leak. Mitigation: only affects programs that were leaking
  anyway; re-run every `tests/error/linear_*.vow` fixture and the `*_mutation_phi_leak` family.
- **Merge interaction**: `Available`(re-armed in one branch) vs `Consumed` now yields `MaybeConsumed`
  rather than staying `Consumed`; covered by slice 4.
- **Loop false positive remains**: `consume_var`'s in-loop `Available` rule still rejects
  `while c { consume(h); h = open(); }` (pre-existing, distinct from this issue).
- **New false negative in loops**: loop bodies mutate the outer tracker with no pre/post merge, so a
  re-arm inside a loop leaves the var `Available` after a possibly zero-iteration loop. Guarded by the
  required zero-iteration fixture in slice 6; escalate to a loop merge in this PR if the region pass misses it.
- **New false positive under shadowing** (name-keyed tracker): narrowed by the slice 5b guard (only `Consumed`
  entries are dropped on a non-linear rebinding). Residual: a linear rebinding via an annotated linear `let`
  already re-registers by name (pre-existing behaviour). Match-arm bindings are hidden at `linear.rs:~440`,
  so assignment to them stays a no-op.
- **Binary fixed point**: `compiler/main.vow` changes (regenerated embedded help/skill text only; no
  codegen/lowering/emitter logic). Re-run `scripts/bootstrap.sh --skip-cargo --no-cache` on the PR's final
  head SHA and record the SHA in the checklist (CLAUDE.md "green locally" rule). Parse/print idempotency and
  verifier C parity are unaffected (no printer, IR, or `c_emitter` change). Clippy gate
  (`--all --all-targets`) applies to the new unit tests; match existing tests (assert `emitter.0.len()`, then
  index `emitter.0[0]`).
- **Tests use identical `dummy_span()`** everywhere; the param-span backstop exemption compares spans,
  so new tests must use `let mut` locals (no params) or distinct spans.

## Out of scope
- Leak diagnostic at overwrite in the AST pass (deferred to region pass; see Assumptions).
- Accepting `consume` then reassign inside a loop body (`consume_var`'s in-loop rule keeps rejecting
  it) - file as follow-up. Loop *merge* handling is in scope only as the contingency in slice 6.
- Hand edits to any `compiler/*.vow` logic (no AST linear pass exists there; `main.vow` changes only via
  `generate_help.py`), refactors of `linear.rs`
  (e.g. splitting the 1.4k-line file), diagnostic wording changes, scoping/shadowing rework of the tracker.
