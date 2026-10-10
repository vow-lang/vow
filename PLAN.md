# Plan: stage codegen before verification in `vow build` / `vowc build` (#179)

## Goal
Make full codegen+link finish (and its Cranelift memory be released) before any ESBMC process is started, in both
compilers, so peak process-tree RSS is `max(codegen, verify)` instead of `codegen + jobs x ESBMC`. Build status,
JSON shape, exit codes and the default-verify behaviour do not change.

## Decision (staged vs bounded overlap)
Staged: codegen -> link -> verify. Bounded overlap was evaluated and rejected: any overlap still stacks the
in-process Cranelift working set (the #174 24 GB Stage-2 case) on top of up to `--verify-jobs` ESBMC children, and a
cap on overlap adds a tuning knob and a scheduler for no determinism benefit. Cost accepted: wall-clock becomes
`codegen + verify` instead of `max(codegen, verify)`; codegen is a small fraction of verification time on
contract-bearing programs (measured in Slice 6, recorded in the ADR).
Two side effects are strict improvements: (a) on codegen/link failure verification is never started (Rust used to
run the whole verify and discard it; self-hosted used to return with ESBMC children still in flight);
(b) no verifier work races with the object/exe being written.

## Assumptions
- Staged over bounded overlap (above): best guess, justified by the issue's "codegen first, verify second" option.
- `Skipped`/non-modelable warnings added by the self-hosted verify launcher no longer appear in the
  `CompileFailed` JSON when codegen fails (they were emitted before codegen). This matches the Rust driver, which
  already discards verifier warnings on `codegen_error_to_output`. Build `status` is unchanged. Documented in the ADR.
- Upfront "ESBMC not found" check stays before codegen in both compilers (fail fast, no codegen work wasted).
- `--perfetto`: the `verification` span / proof spans now start after the `codegen`+`link` spans end. Span names
  and tracks unchanged.
- No spec/CLI flag is added; only the prose that claims "run in parallel" changes.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `vow/src/main.rs` | Rust driver `run_pipeline_from_frontend`: spawns verify thread before codegen, joins at 4 error sites + 2 success sites | 424-645 (spawn 456-503, cache-hit join 546-573, codegen 576-612, link/join 621-644) |
| `vow/src/verify_outcome.rs` | `to_output_with_warnings`, `panicked_output` (fail-closed #413) — reused unchanged | |
| `vow/tests/verifier_panic.rs` | existing fail-closed regression; must stay green; pattern for new Rust test | all |
| `compiler/main.vow` | `run_build_cmd` (2127-2272): `build_verify_phase` (1999-2047) runs before `build_codegen_phase` (2113); `build_drain_verify_phase` (2057) after. Stale comments at 1985-1998, 2049-2056, 2215 | 2213-2237 |
| `compiler/main.vow` embedded skill text | "Codegen (Cranelift) and verification run in parallel" (generated; lines ~6239, ~12307) | |
| `vow/src/skill.rs`, `skills/vow/reference/contracts.md` | generated copies of the same sentence | via `scripts/generate_help.py` |
| `docs/spec/contracts.md` | "Verification Pipeline" prose + diagram (line 25-31) | 25-31 |
| `README.md` | "parallel codegen+verify" (line 155) | 155 |
| `docs/spec/cli.md` | `--perfetto` row (line 32) mentions "compiler->ESBMC handoff"; add ordering sentence only if needed | 32 |
| `tests/build-staging/tests.sh` [new] | shared shell harness, runs against either compiler (`VOWC_BIN`, `VOWC_KIND`) with a fake `esbmc` | |
| `vow/tests/build_stage_order.rs` [new] | cargo-test twin for the Rust driver (codecov/patch gate needs instrumented coverage) | |
| `scripts/full_test.sh` | wire new harness next to `verifier/esbmc-path-cache` (~line 850) and `cli-flags` (1526-1545) | |
| `scripts/measure_build_tree_rss.py` [new] | samples summed RSS of the build process + descendants from /proc; prints peak and phase | |
| `docs/adr/2026-10-10-HHMM-staged-codegen-then-verify.md` [new] | ADR: decision, alternatives, measured before/after, wall-clock trade-off | follow `docs/adr/README.md` naming (UTC) |
| `scripts/measure_bootstrap_rss.sh` | header comment says verify RSS is "tracked in #175 / #179"; update comment only | 4-6 |

## Steps (TDD order; each slice = red, green, refactor)

### Slice 1 - RED: ordering test for the Rust driver
- **File**: `vow/tests/build_stage_order.rs` [new] (model on `vow/tests/verifier_panic.rs`).
- **Behaviour**: put a fake `esbmc` first on `PATH`. It receives ESBMC's args, not `build`'s `-o`, so the test exports
  `VOW_TEST_EXE=<tmp>/out` and `VOW_TEST_LOG=<tmp>/log`; the script appends `exe=present|absent` per `[ -e
  "$VOW_TEST_EXE" ]` and prints `VERIFICATION SUCCESSFUL`. Build a one-contract program
  (`fn f(x: i64) -> i64 vow { ensures: result == x } { x }` + `main`) with `--no-cache -o <tmp>/out`.
  Assert: every log line is `exe=present`, `status == "Verified"`, exit 0, executable exists.
- **Fails today** because the verify thread is spawned (main.rs:470) before `CraneliftBackend::new()` and the exe is
  only linked at main.rs:622. Note: red is timing-based but reliable (linker subprocess runs after ESBMC launch).
- Second test in the same file: codegen failure never starts verification. Fixture
  `tests/error/float_remainder_unsupported.vow` plus a contract-bearing function; fake esbmc appends to `$LOG`;
  assert `status == "CompileFailed"`, error_code `CodegenUnsupported` in diagnostics, and `$LOG` absent/empty.
  (Fails today: verify thread runs to completion first.)

### Slice 2 - GREEN: Rust driver stages verification after link
- **File**: `vow/src/main.rs` (`run_pipeline_from_frontend`, 424-645).
- **Change**:
  - Keep the early `find_esbmc` check (451) and compute `call_site_index`, `verify_cache`, limits as today.
  - Replace the up-front `thread::spawn` (470) with a local closure `run_verify` (same body incl. the
    `VOW_TEST_VERIFIER_PANIC` hook and `verification` span) that is *not* started yet.
  - Delete the five early `verify_handle.join()` calls (550, 582, 590, 596, 625): on codegen/link/io error return
    `codegen_error_to_output` directly.
  - After the object-cache-hit link (546-573) and after the normal link (621-638) run
    `thread::spawn(run_verify).join()` and map `Err(_)` to `verify_outcome::panicked_output(..)` exactly as now.
    Keep a dedicated thread (not `catch_unwind` on main) to preserve the #413 JoinError path and thread stack size.
  - Fold the two duplicated tail blocks (cache-hit and normal path) into one local `finish_with_verify` helper so
    the join + `to_output_with_warnings` is written once.
  - **Free codegen memory before verifying** (required, otherwise staging saves nothing in-process):
    `backend` and `compiled` (main.rs:578-579) live to end of function today. Scope codegen, `write_to_file` and the
    cache `store` in an inner block (or `drop(compiled); drop(backend);`) that ends before `link_obj` / the verify
    thread. Also drop `ir_module` borrows that are not needed by verify.
  - Update the stale comment block at 424-428 (`Arc` no longer "shared with the verify thread concurrently").
- **Reuses**: `verification::run_verification_sync` (`vow/src/verification.rs`), `verify_outcome::{to_output_with_warnings,
  panicked_output}`, `codegen_error_to_output`.
- **Refactor**: drop now-unneeded `Arc` clone if `ir_module` can be borrowed by the closure (scoped thread);
  keep `Arc` otherwise - do not widen scope.

### Slice 3 - RED then GREEN: self-hosted driver
- **Test**: `tests/build-staging/tests.sh` [new]: same two scenarios as slice 1 (same `VOW_TEST_EXE`/`VOW_TEST_LOG` env contract), parameterised by `VOWC_BIN` /
  `VOWC_KIND` (`rust` uses `target/release/vow`, `self` uses `build/vowc`); additionally for `self` assert that with
  `--no-verify` the fake esbmc is never invoked and status is `Unverified`, and that a default `build` still
  verifies. Runs under `$TMPDIR`/`mktemp -d` with the kill-signal traps used by `tests/esbmc-path-cache/tests.sh`.
- **File**: `compiler/main.vow` `run_build_cmd` (2213-2237).
- **Change**: move the `if do_verify { build_verify_phase(...) }` block and `verify_origin = trace_now(tctx)` to after
  the `codegen_status != 0` / `!do_verify` returns; immediately followed by the existing `build_drain_verify_phase`
  call. `build_verify_phase` / `build_drain_verify_phase` keep their signatures (no churn); rewrite their header
  comments (1985-1998, 2049-2056) and the "bounded pool" comment (2215) to say verification runs after codegen,
  so the pool-bounded in-flight set is the only concurrency left. The `trace_span("verification", ...)` now measures
  post-codegen time only.
- **Success-path memory**: `__vow_clif_finish` (`vow-clif-shim/src/lib.rs:2872`) takes `Box::from_raw` of the
  `ModuleContext` and drops it on return, so the shim context is already reclaimed before `__vow_clif_link`; no
  shim change is needed. `__vow_clif_destroy` (1019) only covers the error path. Slice 6 confirms with data.
- **Parity**: Rust and self-hosted must land in the same PR (CLAUDE.md dual-compiler rule). The native verifier
  (`vowc verify --backend native`, `run_verify_native`) is not on this path and is untouched.

### Slice 4 - wire into CI harness
- **File**: `scripts/full_test.sh`: add `build-staging/rust` and `build-staging/self-hosted` pass/fail entries
  (gate on `command -v esbmc` is NOT needed - the fake esbmc is on PATH; but `find_esbmc` for the Rust driver and
  `esbmc_path()` for the self-hosted one both resolve via PATH, so the fake is found).

### Slice 5 - docs and generated text
- `docs/spec/contracts.md` 25-31: replace "run in parallel" with the staged pipeline
  (`IR Lower -> Cranelift -> link -> C Emit -> ESBMC`), one sentence on why (peak memory) and that codegen failure
  skips verification. Update `README.md:155` ("parallel codegen+verify" -> "staged codegen then bounded-parallel verify").
- `uv run python scripts/generate_help.py` to regenerate `compiler/main.vow` embedded skill, `vow/src/skill.rs`,
  `skills/vow/reference/contracts.md`; then `python3 scripts/check_help_coverage.py`.
- `docs/adr/2026-10-10-HHMM-staged-codegen-then-verify.md` [new] with measured numbers (Slice 6).
- Leave historical docs (`docs/roadmap.md`, `docs/level5-test-trace.md`, `docs/live-programming-research.md`) untouched.

### Slice 6 - measurement (acceptance criterion)
- **File**: `scripts/measure_build_tree_rss.py` [new]: spawn the command, poll `/proc` every 20 ms, sum
  `VmRSS` of the pid and all descendants (walk `/proc/<pid>/task/*/children` or scan PPid), report
  `peak_tree_kb`, `peak_self_kb`, time of peak, and wall time as JSON. Linux only; `-I`-safe stdlib only.
  (`/usr/bin/time -v` and `ru_maxrss` report the largest single process, not the concurrent sum, so cannot show
  this change - say so in the script docstring.)
- Procedure (recorded in the ADR and PR body): build the pre-change binaries from `origin/main`
  (`git worktree`/saved `build/vowc.main`), then for `{old,new} x {rust, self}` run `build --verify-jobs 2` on a
  mid-size contract-heavy program (e.g. a `benchmarks/*/reference.vow` or the largest `examples/` program with
  several `vow` blocks), 1-3 samples, median `peak_tree_kb` + wall. Memory budget: #174 reports ~24 GB for a
  self-hosted `compiler/main.vow` build; run `compiler/main.vow` at most once per compiler, only if it fits the host
  share (check `free -g`), `--verify-jobs 1`, otherwise state it was skipped. Use `VOW_CACHE_DIR=$(mktemp -d)` so
  the compile cache cannot serve stale objects. Honest reporting: if the allocator does not return Cranelift pages
  to the OS (glibc keeps freed arenas) the parent's RSS stays high while ESBMC runs and the gain is smaller than
  the sum - read `peak_tree_kb` with that in mind and report the measured number either way.
- Do **not** add a hard RSS bound to `bench/memory` or `full_test.sh` (machine-dependent, ESBMC-version
  dependent); the ordering test is the regression guard.

## Verification surface
- No contracts, C model, or IR change: ESBMC input is byte-identical (same IR, same emitter, same launch order
  within the verify phase). Run `python3 scripts/parity.py c RUST_BIN SELF_BIN tests/verify*/...` (full_test
  Section 2c) as a no-diff check only.
- Fixtures: no new `tests/run/` programs. New fixtures are inline in the shell/Rust tests; reuse
  `tests/error/float_remainder_unsupported.vow` for the codegen-failure case (add a contract-bearing fn inline if
  it has none, otherwise the verifier would skip it).
- Existing gates that must stay green: `vow/tests/verifier_panic.rs` (fail-closed #413), `tests/verify-fail*`,
  `tests/error/*`, `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all`.

## Risks
- **Fixed point (`scripts/bootstrap.sh`)**: only driver scheduling changes; IR, codegen order, and `clif`/shim
  untouched, so Stage 2/3 binaries stay byte-identical. Bootstrap Stage 1 verification now runs after Stage 1
  codegen: wall-clock for the verified stage rises by codegen time (~minutes); the bootstrap wall-clock budget
  note (~5 min) should be re-measured and reported. Per CLAUDE.md, re-run `scripts/bootstrap.sh --skip-cargo
  --no-cache` on the final head SHA before ticking the checklist and record the SHA.
- **Untouched invariants**: parse->print->parse idempotency, `BTreeMap` slot map / stack-slot layout in
  `vow-clif-shim`, `c_emitter.{rs,vow}` byte parity, and clippy are unaffected (driver scheduling only); still run
  Section 2c parity and clippy as no-diff checks.
- **Perfetto test**: `vow/tests/perfetto.rs` has no assertions on codegen/verification span overlap or handoff
  ordering (grep'd), so span reordering is safe; re-grep before landing.
- **Fail-closed semantics**: a verifier panic must still yield `VerifyFailed`/`panicked` with the exe removed
  (`verify_outcome::panicked_output`); guarded by keeping the verify thread + `JoinError` mapping. Do not turn
  the join into `unwrap`.
- **Object cache**: cache hit path used to join an already-`NotRun` thread; the hit path is only enabled when
  `no_verify` (`compile_cache_enabled`, main.rs:356), so staging must still call the same tail helper there.
- **Diagnostics delta on codegen failure**: self-hosted `CompileFailed` JSON loses verifier skip warnings (see
  Assumptions); acceptable, matches Rust, but call out in PR body. No test fixture should depend on them -
  `grep -rn "VerificationSkipped" tests/error` before landing.
- **stderr ordering**: self-hosted "Starting verification of ..." lines now follow codegen output; any
  `TEST: stderr` directive on a build fixture that expects interleaving would break - grep `tests/` for
  `Starting verification`.
- **Wall-clock regression**: unavoidable for staged scheduling; quantify in ADR. Mitigation is the already-bounded
  `--verify-jobs` pool; no new knob.
- **Residual in-process RSS**: staging cannot return pages the allocator keeps. A follow-up (codegen in a
  subprocess, or `malloc_trim` after `__vow_clif_finish`) is listed below, not bundled.
- **codecov/patch (95%)**: Rust changes in `main.rs` must be exercised by `cargo test` (hence the Rust twin test,
  and the tail-helper covers both cache-hit and normal path via existing `compile_cache_enabled` tests/e2e).
- **Memory budget for the implementer**: cap `cargo build -j2`; run bootstrap and full_test in the foreground with
  explicit timeouts (full_test ~40 min; do not background-and-wait).

## Out of scope
- Releasing the frontend/IR earlier (#178), Cranelift FFI copying (#176), embedded help payload (#177),
  bootstrap Stage 2/3 verify skipping (#180), verifier job limit (#175, already merged).
- Running codegen in a child process or calling `malloc_trim` to return pages (follow-up issue to file if the
  measurement shows residual parent RSS dominates).
- `--backend native` verify scheduling, `vowc test --verify` pool, any change to verification results,
  contracts, C emitter, or CLI flags.
- Formatting or unrelated cleanup in `main.rs` / `main.vow`.
