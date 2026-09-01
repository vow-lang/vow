# Plan: #1155 — unify multi-function verification stopping/aggregation policy

## 1. Problem restated

`vow verify`/`vow build` verify every vowed function in a module and report failures as a
`counterexamples[]` JSON array. The Rust driver (`vow/src/verification.rs::run_pool`) already
stops launching new ESBMC jobs after the first halt-class result, drains whatever was already
in flight, and then deterministically collapses the outcome to the **lowest module-declaration-index**
failure — so `counterexamples[]` is always length 0 or 1. The self-hosted driver
(`compiler/main.vow`) uses the *same* "stop launching after first failure, drain what's in flight"
scheduling, but pushes **every** drained failure into its `ces` accumulator and emits all of them.
Because "what's in flight" depends on `--verify-jobs` and how many functions happen to be
launched before the first failure is detected, the self-hosted count is not a deliberate
"accumulate everything" policy — it is scheduling nondeterminism that happens to produce more
than one entry whenever enough concurrency is available (which is why `gc` reports 5 and `math`
reports 3 today, and why `--verify-jobs 1` already makes both compilers agree). The fix is to
make the self-hosted compiler stop accumulating after the first hard failure, mirroring Rust's
existing, already-tested, deterministic single-counterexample policy, and to make that policy
explicit and parity-tested so it can't regress silently.

## 2. Files to touch

**`compiler/` (self-hosted, production change):**
- `compiler/main.vow` — `verify_collect_and_report` (lines ~1299–1447), specifically the two
  `ces.push(build_ce_from_result(...))` call sites at line ~1340 (arith-rerun path) and ~1375
  (main path). Gate both with `if ces.len() == 0 { ces.push(...); }` so only the first
  drained hard failure is ever recorded. This is the single chokepoint: `verify_collect_and_report`
  is called exclusively from `verify_collect_traced` (line 1027), which is in turn the only
  drain point used by `run_verify_loop_traced` (the `verify` path, including the legacy
  `--verify` flag path at ~2347) and by `build_verify_phase`/`build_drain_verify_phase` (the
  `build` path, ~1546–1620). Fixing it here covers `verify`, `build`, and legacy `--verify`
  without touching any of the three separate `ces_final = ces` assignment sites (~1509–1512,
  ~1781–1784, ~2355–2358) — no other file needs to change to get correct behavior on all three
  entry points.
- No change to `record_soft_fail` (already first-wins) or `vf_prefers_counterexample`
  (CE-vs-soft-fail precedence is orthogonal to this issue and already correct).

**`vow/` (Rust, no production change expected):**
- `vow/src/verification.rs` — no change. `run_pool`'s serial path (403–431) and parallel path
  (433–507) already implement "stop launching after first halt, drain in-flight claimed work,
  report the lowest-index halt" and are pinned by existing tests
  (`run_pool_serial_halts_without_evaluating_later`, `run_pool_reports_lowest_index_halt`).
  `VerifyOutcome::Failed` structurally never carries more than one `StructuredCounterexample`
  (`verify_one_function`, ~179–195) — there is nothing to accumulate on this side. If review
  turns up a path where this assumption doesn't hold, treat that as a separate bug, not part of
  this slice.
- Add a Rust-side regression test only if slice 3 below finds a gap; expected not to be needed
  since the existing unit tests already cover this exact behavior.

**Test fixtures (new):**
- `tests/verify-fail/verify_jobs_multi_hard_failure.vow` — new fixture, modeled on the existing
  `tests/verify-fail/verify_jobs_ce_before_soft.vow` pattern, but with **two or more independent,
  always-reachable hard failures** (not one hard failure + one soft/timeout function), so that
  under `--verify-jobs >= 2` multiple functions can genuinely be in flight and fail concurrently.

**Fixture suppressions to remove (forced by the fix, not optional):**
- `stdlib/gc/main.vow:2` — delete the `// TEST: known-cex-count-divergence 1155 "Rust stops after
  first failed function"` line.
- `stdlib/math/main.vow:1` — delete the matching line.
  (`scripts/parity.py::_known_cex_verdict` turns a stale directive into a hard `FAIL` once counts
  agree, so leaving these in place after the fix is not a safe no-op — they must come out in the
  same PR, and `full_test.sh` will catch it if forgotten.)

**`scripts/full_test.sh`:**
- Add a new harness block (modeled on the existing `verify_jobs_counterexample_suppresses_later_soft_meta`
  block, `scripts/full_test.sh:705–739`) that drives the new fixture through **both** `$RUST` and
  `$SELF` at `--verify-jobs 3` (or higher) across `verify`/`build`/legacy modes, calls
  `compare_json` (not a self-only assertion — this is the parity check the issue asks for), and
  additionally asserts `len(counterexamples) == 1` and `counterexamples[0].function` equals the
  lowest-declared-order failing function name on **both** sides. This is what turns "Rust and
  self-hosted implement the same policy" from a prose claim into an executable check.
- No change needed to `scripts/parity.py` itself — `_compare_counterexamples`'s count comparison
  (192–219) already runs unconditionally in the `both_verify_failed` branch (263–272); it is the
  fixture-local directives that were carving out an exception, and slice 2 removes those.

**`docs/spec/cli.md`:**
- Near the `VerifyFailed` row (line 282) and the `counterexamples` row in the Fields Reference
  table (line 367), add one explicit sentence: at most one entry is reported per `verify`/`build`
  run — the lowest module-declaration-order function with a hard failure (definitive
  counterexample, timeout, unknown, tool error) — and later-declared failures are only surfaced by
  fixing the reported one and re-running. Today the doc is silent on cardinality/ordering; this
  closes that gap on both compilers at once since the policy is now shared.
- Check `docs/equivalence/README.md:196–227`, which currently documents the
  `known-cex-count-divergence` directive as live policy; update it to state the directive is no
  longer needed (or drop the paragraph) once the two fixture suppressions are removed, so the doc
  doesn't describe a mechanism that no longer has any active users.
- Leave `docs/spec/schemas/*.json` untouched — no schema test currently asserts array cardinality,
  and adding `maxItems: 1` would be a new consumer-visible tightening beyond what this issue asks
  for.

**Explicitly not touched:** `bench/runner.py` — confirmed via `grep -n counterexamples
bench/runner.py` (line 220) that it already only ever reads `ces[0]`; the CEGIS loop's behavior
is unaffected by capping the array at length 1 (it never used a second element).

## 3. TDD slices

1. **Red: parity fixture shows the divergence explicitly.**
   Write `tests/verify-fail/verify_jobs_multi_hard_failure.vow` with (at minimum) two functions
   with unconditionally violated `ensures` clauses (same shape as `early_bad` in
   `verify_jobs_ce_before_soft.vow`, e.g. `ensures: result >= 0` called with a literal negative
   argument), declared in a fixed order so "lowest declaration index" is unambiguous. Add the new
   `full_test.sh` harness block calling `compare_json` at `--verify-jobs 3`+ for `verify`/`build`/
   legacy modes, asserting `len(counterexamples) == 1` and `function` equals the first-declared
   failing function's name on both `$RUST` and `$SELF` output. Run `scripts/full_test.sh` (or just
   this section) against current `main` behavior first to confirm it fails on the self-hosted side
   with more than one counterexample (and/or a non-lowest-index function) — this is the red step;
   no production code changes yet.

2. **Green: gate the self-hosted accumulator.**
   In `compiler/main.vow::verify_collect_and_report`, wrap both `ces.push(build_ce_from_result(...))`
   call sites (~1340, ~1375) with `if ces.len() == 0 { ... }`. Rebuild the self-hosted compiler
   (`scripts/bootstrap.sh --skip-cargo`, since this is a `compiler/`-only change and stage 0
   Rust is untouched) and re-run the new harness block plus the pre-existing
   `verify_jobs_counterexample_suppresses_later_soft_meta` test to confirm both now pass, and that
   the `gc`/`math` fixtures (still carrying their suppression directives at this point) now
   produce matching counts without needing the directive.

3. **Remove the now-stale suppressions.**
   Delete the `// TEST: known-cex-count-divergence 1155 ...` lines from `stdlib/gc/main.vow` and
   `stdlib/math/main.vow`. Run `scripts/full_test.sh` Section 6 (Multi-Module) end to end; confirm
   `compare_json` passes cleanly for `gc/verify` and `math/verify` with no directive present. If
   `_known_cex_verdict` reports a stale-directive FAIL before the lines are deleted, that is
   expected and confirms the fix is effective — proceed to deletion.

4. **Docs.**
   Update `docs/spec/cli.md` (VerifyFailed row + Fields Reference `counterexamples` row) and
   `docs/equivalence/README.md` (196–227) per the "Files to touch" section above. Re-run
   `scripts/check_help_coverage.py` if it covers this section (spot-check; this is a prose-only
   change to `cli.md`, not a flag/schema change, so `--help` regeneration is not expected to be
   required — confirm by grepping `scripts/generate_help.py`'s source list for `cli.md` sections
   touched before deciding whether to skip the regen step).

5. **Full regression pass.**
   Run `cargo test --all`, `cargo clippy --all -- -D warnings`, and the complete
   `scripts/full_test.sh` (all sections, not just 4c/6) to confirm no other fixture regresses —
   in particular, re-check `tests/verify-fail/*.vow` broadly (Section 4c uses default
   `--verify-jobs`, so any other existing fixture that happens to have multiple hard failures
   under default concurrency should also collapse to one CE now; none are currently known to have
   more than one hard-failing function, but this pass is the safety net).

## 4. Verification surface

This change touches driver/scheduling logic only — no IR, codegen, or C-model changes, so there is
no new ESBMC property to prove and no change to what ESBMC itself checks. The "verification
surface" here is entirely about the **shape of the diagnostic payload**, not soundness: the
self-hosted compiler already ran ESBMC against every function it was going to run it against
before this change; the fix only changes which already-produced failures get serialized into
`counterexamples[]`. No existing counterexample becomes unreported in a way that hides a real
defect — `all_proven` (which drives exit code and `status: VerifyFailed`) is set on every hard
failure regardless of the `ces.len() == 0` gate, so a program with two failing functions still
fails verification; only the JSON detail for the second failure is now suppressed until the first
is fixed and the program is re-verified. New fixture: `tests/verify-fail/verify_jobs_multi_hard_failure.vow`
(slice 1) — two unconditionally-failing functions, no `main`-reachability subtlety needed since
the point is exercising the scheduler, not a soundness edge case.

## 5. Risk areas

- **Bootstrap fixed point:** the change is confined to `compiler/main.vow`, a driver-level
  function with no interaction with `ir.vow`, `clif.vow`, or codegen ordering. It does not touch
  `BTreeMap`/`HashMap` choices, stack-slot layout, or anything in `vow-clif-shim`. Risk to the
  binary fixed point is low, but the standard `scripts/concat_vow.sh` triple-bootstrap check
  (Stage 0 → A → B → C, `sha256sum` compare) should still be run once before merging, since any
  `compiler/` change is in scope for that gate per repo convention.
- **`parse → print → parse` idempotency:** unaffected — no AST/printer changes.
- **Clippy gate:** no Rust production code changes are planned, so `cargo clippy --all -- -D
  warnings` should be a no-op check, not a source of new work. If slice 3's regression pass adds a
  Rust-side test, keep it inside `#[cfg(test)] mod tests` in `verification.rs` so the
  test-target clippy exclusion (already documented as project convention) applies.
- **Ordering assumption:** the fix relies on "FIFO drain order == launch order == module
  declaration order" holding on both the `verify` and `build` self-hosted paths. This was
  confirmed by reading `run_verify_loop_traced` (1038–1097) and `build_verify_phase`/
  `build_drain_verify_phase` (1546–1620): `in_flight_h`/`in_flight_fi`/`in_flight_tmp` are pushed
  in `fi` order and drained via `vec_i64_remove_front`/`vec_string_remove_front` (front-of-queue,
  i.e. oldest-first) with a blocking `verify_collect`/`verify_collect_traced` call on that specific
  handle — so drain order is deterministic and equals launch order regardless of actual OS-level
  ESBMC subprocess completion timing. Slice 1's fixture is the executable check that this holds in
  practice; if it doesn't, that's a more fundamental scheduler bug worth its own issue, not a
  reason to abandon this slice.
- **`--verify-jobs 1` today already "works":** because the divergence is job-count-dependent, any
  manual smoke test using the default single-threaded assumption would falsely appear to confirm
  parity. The new fixture must explicitly pass `--verify-jobs >= 2` (ideally >= number of hard
  failures in the fixture) to actually exercise the bug and the fix.

## 6. Out of scope

- **"Accumulate all failures" as the chosen policy (issue's Option A).** The issue explicitly
  leaves the policy choice open ("this issue does not assume 'all failures' is automatically
  preferable"). Implementing full accumulation would require changing `VerifyOutcome::Failed` from
  a single-function to a multi-function shape in Rust, rewriting `run_pool`'s return type and its
  two pinned tests, removing the "stop launching after first failure" guard on both compilers
  (a genuine behavior change with wall-clock/solver-cost implications), and deciding a new
  CE-vs-soft-fail precedence rule for the multi-failure case on the Rust side (self-hosted's
  `vf_prefers_counterexample` has no Rust analogue today). That is a substantially larger, riskier
  change than the bug this issue reports, and per repo convention ("many small changes beat one
  large change") it belongs in a separate follow-up issue/PR if the team decides the richer CEGIS
  payload is worth the added complexity — not bundled into this fix.
- **Removing or restructuring `record_soft_fail` / `vf_prefers_counterexample`.** Both are already
  correct for this issue's scope (first-wins semantics, CE-vs-soft-fail precedence); no change
  needed.
- **Schema tightening (`maxItems: 1` in `docs/spec/schemas/counterexample.schema.json` or
  `build-result.schema.json`).** Left alone per "Files to touch" — no existing schema-validation
  test requires it, and it's a separate, consumer-visible commitment beyond fixing the count
  mismatch.
- **Reformatting or refactoring `verify_collect_and_report`, `run_verify_loop_traced`, or
  `build_verify_phase`/`build_drain_verify_phase` beyond the two-line gate.** These functions are
  long and carry detailed comments explaining non-obvious behavior (arith-rerun, FIFO drain); this
  PR adds a minimal, additive guard and does not otherwise touch their structure.
- **`bench/runner.py` changes.** Confirmed unaffected (only reads `ces[0]`); no change bundled.
