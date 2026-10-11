# Plan: report every non-modelable skip regardless of halts (issue #1561)

## Goal
Make the set of `VerificationSkipped` warnings a pure function of the program: a non-modelable vowed
function is reported whether or not another function fails, in both compilers, so cold cache, warm
cache, `--no-cache` and any `--verify-jobs` agree. Fix the CI flake `heap/verify` (full_test.sh Section 6).

## Diagnosis (the issue's suspected cause is wrong)
The cache does not drop anything: `non_modelable_reason` runs *before* any cache lookup
(`vow/src/verification.rs:100`). The real mechanism is the halt gate:

- `run_pool` stops claiming indices once any function halts (`verification.rs:496`, serial path
  `:465` returns at the first halt). Skipped (non-modelable) functions are evaluated *inside* the pool
  (`verify_one_function`, `:100-105`), so a skip whose index is claimed after the halt is silently lost.
- Cold: the failing function waits on ESBMC for seconds, so workers have time to claim the cheap
  later skip indices. Warm: `prove_cached` (`:305`) returns the cached FAILED counterexample in
  microseconds, `stop` is set first, later skip indices are never claimed. Result: 5 skips cold, 2-5 warm.
- Observed (see Assumptions for provenance): old binary on `stdlib/heap/main.vow` cold -> 5 skips, then
  5, then 2 with the cache holding 2 `.vr` FAILED entries. `--no-cache` is stable only because every
  halt takes real solver time.
- Self-hosted has the same class of bug without a cache: skips are emitted only `if all_proven == 1`
  (`compiler/main.vow:1233`, `:2110`), and `all_proven` flips when the FIFO drain collects a failure,
  so the skip set depends on `--verify-jobs` (default `num_cpus/2`). No on-disk verify cache exists there
  (`compiler/verifier.vow:1680-1690`), so nothing to fix cache-side.
- Item 3 (VerifyFailed, 0 counterexamples): on the old binary the halt is `Error` for `min_heap_new`,
  ESBMC `PARSING ERROR` (`__vow_heap[v3 + 0] = v2;` assigning `__vow_vec_t` to `int64_t`), i.e.
  `verify_status: "error"`, `counterexamples: []`. That is a C-emitter model bug, not a cache effect.
  It must be re-confirmed on a HEAD build; the fix (if still present) needs byte-identical
  `c_emitter.rs`/`c_emitter.vow` changes and is a follow-up issue, not bundled here.

## Assumptions
- Item 3 evidence comes from the pre-built `vow` binary in the `1409-feat-verify-op-model-table-and-fail-closed-skipped-gate`
  workspace (HEAD `20d60561b4361826f68a0567d3197a305d2776c3`), not from this branch's HEAD; no build was run in
  planning. Slice 0 re-confirms on HEAD. (best guess: the halt reason is unchanged on HEAD; the issue's
  "VerifyFailed, 0 counterexamples" matches `Error`/`Unknown`/`Timeout`, none of which carry counterexamples.)
- Semantics chosen: emit **all** skips (option A) rather than only skips indexed before the first halt
  (option B). A skip is a static fact about the program, not proof progress; A is trivially mirrored in the
  self-hosted loop and matches the issue's "Expected" and today's stable no-cache output (best guess).
- Emission order: skip warnings are produced in a pre-pass, in function order, before verification warnings.
  `scripts/parity.py` canonicalises non-error diagnostic order (`_canonical_diagnostic_order`), so this does
  not affect Rust/self parity.
- `vowc test` / Rust `test --verify` loops are out of scope: Rust `test --verify` goes through
  `test_runner::run_test_command` (`vow/src/main.rs:~935`), not `run_verification_sync`; the self-hosted test
  loop (`compiler/main.vow:~2604`) folds skips into `contract_skipped` status where a failure already takes
  precedence (#386), so no skip diagnostic is lost to a halt that changes the verdict. Leave untouched.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `vow/src/verification.rs` | `verify_one_function` skip check (100-105), `run_verification_sync` (369-423), `run_pool` (437-551), pool unit tests (610+) | 100, 369, 437 |
| `vow/src/verify_outcome.rs` | `SkippedFunction`, `to_output_with_warnings` (312) - already emits findings on halt, no change expected | 52, 312 |
| `vow/src/cache.rs` | `VerifyCache` - read-only reference; no change | 280-335 |
| `compiler/main.vow` | `skip_if_non_modelable` (1163), `run_verify_loop_traced` (1205-1273), `build_verify_phase` (2084-2135) | 1233, 2110 |
| `compiler/verifier.vow` | `is_verify_target` (1262), `report_unchecked_calls` (1293); cache note (1680) | 1262 |
| `compiler/c_emitter.vow:626` / `vow-verify` `non_modelable_reason` | pure, cheap classifier reused for the pre-pass | 626 |
| `scripts/full_test.sh` | Section 6 `heap/verify` (1968-1991), native cold/warm pattern (1577-1605), Section 4c verify-fail | 1577, 1968 |
| `tests/verify-fail/` | home for the new e2e fixture (Section 4c checks a `// TEST: counterexample-violation` directive; read the section first) | |
| `docs/spec/errors.md` | `VerificationSkipped` section (1379) | 1379 |
| `docs/spec/cli.md` | `Skipped` status row (349), `skipped` per-function row (607) | 349, 607 |

## Steps (TDD slices, in order)

### 0. Confirm on HEAD (no code)
- `cargo build --release -p vow -j4` (memory-budget cap), then `scripts/bootstrap.sh --skip-cargo` for `build/vowc`.
- Run the issue repro with an isolated `VOW_CACHE_DIR` and with `--verify-jobs 1`/`4`/`--no-cache`; record the
  halting function and `verify_status`/`verify_message` for `stdlib/heap/main.vow`. Note whether `min_heap_new`
  still hits the ESBMC parse error; if it does, file the follow-up (see Out of scope).
- Pick the e2e fixture's failing function so it fails fast and deterministically.

### 1. RED e2e fixture, deterministic (`--verify-jobs 1`)
- **File**: `tests/verify-fail/skip_after_failure.vow` [new] (place per Section 4c's conventions; copy a
  non-modelable construct from `tests/verify-skip/vec_param_push_skipped.vow` or `vec_of_string_skipped.vow`).
- Layout, in source order: (1) a cheap, modelable function with a wrong `ensures` (fails with a
  counterexample), then (2) at least two non-modelable vowed functions. Use wrapping `+`, no checked
  arithmetic, so no `ArithOverflowReachable` warning can flake the comparison (dynamic warnings stay
  halt-dependent, see Out of scope).
- Assert: `vow verify --no-cache --verify-jobs 1` reports `VerifyFailed` **and** a `VerificationSkipped`
  warning for every non-modelable function. Today both compilers fail it deterministically: Rust serial path
  returns at the halt (`verification.rs:465`); self-hosted gates at `main.vow:1233`.
- Add as a Rust integration test `vow/tests/verify_skip_after_halt.rs` [new] (pattern: `vow/tests/verifier_panic.rs`,
  `vow/tests/contracts.rs`) running the fixture at `--verify-jobs 1`, skipping when ESBMC is absent.

### 2. GREEN Rust: pre-pass for skips
- **File**: `vow/src/verification.rs`.
- Extract a pure helper, e.g. `fn split_non_modelable<'a>(vowed: &[&'a Function], module, const_fns) -> (Vec<(usize, SkippedFunction)>, Vec<&'a Function>)`
  (reuses `non_modelable_reason`, `:100`). `run_verification_sync` builds skip warnings for **all** vowed
  functions up front (function order), runs `run_pool` only over the modelable remainder, and returns
  `skips ++ pool_warnings ++ unchecked`. Outcome: halt wins; otherwise `SkippedNonModelable` if any skip
  exists, else the pool outcome (preserve the lift currently done inside `run_pool` at `:468`/`:531`).
- Remove the non-modelable branch from `verify_one_function` (`:99-105`) or keep it as a debug-assert; the
  pool never sees an unmodelable function.
- Keep `run_pool`'s signature unchanged (its ~10 unit tests inject closures; do not churn them).
- Unit tests (same file, `#[cfg(test)]`): the partition returns every skip in index order and the modelable
  rest; a skip *after* a halt-class index is still returned (pure, no solver); `run_verification_sync`
  with a stub-free module keeps `effectful vow reported as skipped` test (`:1194`) green.

### 3. GREEN self-hosted: mirror in both verify loops
- **File**: `compiler/main.vow`. Add a pre-pass helper (e.g. `report_non_modelable_skips(fns, ir_mod,
  const_fn_indices, dctx) -> i64` returning `any_skipped`) that, for every `is_verify_target` function with a
  non-empty `non_modelable_reason`, emits via `skip_if_non_modelable` (`:1163`). Call it once before the
  launch loop in `run_verify_loop_traced` (`:1205`) and in the `run_build` caller of `build_verify_phase`
  (`:2084`); in both loops the in-loop branch becomes "launch ESBMC for modelable targets only" (non-modelable
  targets are `continue`d; record a `Vec<i64>` flag or re-call `non_modelable_reason`, whichever is smaller).
  Keep `any_skipped` return values identical so exit codes/status mapping (`:1266`, `:2086`) are unchanged.
- Update the three comments that say a skip is only reported while `all_proven == 1`.
- No change to `compiler/verifier.vow` caching (none exists).

### 4. Cold/warm/`--no-cache` guard in `scripts/full_test.sh`
- New block after Section 4d (or in Section 6 setup): for a bounded set of fixtures (the new
  `skip_after_failure.vow`, `stdlib/heap/main.vow`, one all-modelable `tests/verify-fail/*.vow` with
  counterexample so the cache is actually stored):
  `base=$($RUST verify --no-cache --verify-jobs 4 f)`; then with an isolated
  `VOW_CACHE_DIR=$(mktemp -d "$TMPDIR/rust-cache.XXXXXX")` run cold and warm and require JSON+exit equal to
  `base` (copy the shape of `full_test.sh:1577-1605`).
- Non-vacuity: assert at least one `*.vr` file exists under the dir after the cold run (otherwise the
  warm run proves nothing).
- Rust only: self-hosted has no verify cache (`compiler/verifier.vow:1680`). Also compare self-hosted
  `--verify-jobs 1` vs `4` JSON for the skip fixture (jobs-independence).
- Make Section 6 `heap/verify` hermetic: run `$RUST verify` with an isolated `VOW_CACHE_DIR` so a CI/dev
  shared `~/.cache/vow` can no longer influence it.

### 5. Spec + help
- `docs/spec/errors.md` (VerificationSkipped, ~1379): add that a skip is reported for every non-modelable
  vowed function even when another function fails, independent of cache state and `--verify-jobs`.
- `docs/spec/cli.md:349`/`:607`: same one-line clarification. No CLI flag/syntax change, so
  `generate_help.py` should be a no-op; run `scripts/check_help_coverage.py` to confirm.

## Verification surface
- No contract, codegen, IR, or C-model change. ESBMC proves nothing new; `vow-verify/src/c_emitter.rs` and
  `compiler/c_emitter.vow` are untouched, so verifier-C parity (Section 2c) is unaffected by construction
  (functions that reach the emitter are unchanged; skipped ones never did).
- Fixtures: new `tests/verify-fail/skip_after_failure.vow` (exercised by Sections 2c/4c and the new
  cold/warm block). `tests/run/` and `examples/` unchanged.
- Gates to run: `cargo test -p vow`, `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all`,
  `scripts/bootstrap.sh --skip-cargo --no-cache` (record head SHA), `build/vowc test compiler/` if a
  compiler test is added, targeted `scripts/full_test.sh` sections (2c, 4c, 4d, 6, new block).

## Risks
- Existing snapshot/fixture expectations that pinned "skips dropped after a failure" or the interleaved
  order: grep `tests/verify-fail*/` and `vow/tests/` for `VerificationSkipped` alongside a counterexample;
  Section 4c/4d full-json parity canonicalises order but not membership.
- Binary fixed point: the self-hosted edit changes `compiler/main.vow` only (no `BTreeMap`/`HashMap`, codegen,
  or stack-slot layout change); bootstrap fixed point must still be byte-identical - re-run
  `scripts/bootstrap.sh --skip-cargo --no-cache`.
- `run_pool` outcome lift: moving skips out of the pool must preserve `SkippedNonModelable` when *only*
  skips exist and must not lift it for arith warnings (#585); covered by existing pool tests plus a new one.
- Native backend (`compiler/vc_native.vow`) has its own skip path; not touched. Confirm `--backend native`
  fixtures in Section 4e/verify-native still pass.
- Scheduling-dependent fixtures: keep the new fixture's non-skip warnings empty or the cold/warm check flakes.
- Codecov patch gate: the Rust change is covered by the new unit tests + integration test; keep re-indented
  existing lines to a minimum.

## Out of scope (file as follow-ups via `gh issue create`)
1. Other warnings produced inside the pool are still halt-/scheduling-dependent on a halted parallel run:
   `ArithOverflowReachable` findings and demoted `UncheckedCalls` notes (capacity notes are already dropped
   on halt for this reason, `verify_outcome.rs:318`). Same class, separate decision (drop vs always compute).
2. The C-emitter bug behind `VerifyFailed` with zero counterexamples on `stdlib/heap/main.vow` (ESBMC parse
   error for `min_heap_new`: `__vow_heap[v3 + 0] = v2` with a `__vow_vec_t` value) - if still present on HEAD;
   needs a byte-identical `c_emitter.rs` + `c_emitter.vow` fix.
3. Making `heap` verify clean (status `Verified`/documented `Skipped`) is not part of this PR.
4. `test --verify` loops, `contracts --verify`, native backend skip handling.
5. No refactor of `run_pool`'s signature, no formatting/unrelated cleanups.
