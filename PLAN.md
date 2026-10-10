# Plan: Track Stage 2 bootstrap memory reduction (#174) — first slice: #180 bootstrap sequencing

## Goal
Stop Stage 2 and Stage 3 of `scripts/bootstrap.sh` from verifying inside `build` (verification and codegen
overlap in one process there), and run one explicit self-hosted `verify` pass on the fixed-point binary
instead. Same verification coverage, no overlap, and no ESBMC work left in the two heaviest rebuilds.

## State of the tracker (verified 2026-10-10, main @ 8ae1a2c5)
| Child | State | Evidence |
|---|---|---|
| #175 verifier concurrency | CLOSED, landed | `--verify-jobs` (`compiler/main.vow:1269 resolve_verify_jobs`, bounded FIFO pool in `build_verify_phase` :1997); `bootstrap.sh` passes `--verify-jobs 1` |
| #178 driver lifetimes | CLOSED, landed | `run_build_cmd` split into phase helpers (commits 34c5f193, a6a5cd45, 3958b16a); `scripts/measure_bootstrap_rss.sh` |
| #176 Cranelift FFI scratch buffers | CLOSED, landed | streaming `__vow_clif_fn_begin/_block/_inst/_end` API (CLAUDE.md "vow-clif-shim architecture") |
| #298 lexer arena exhaustion | CLOSED | — |
| **#180 bootstrap sequencing** | **OPEN, not implemented** | `scripts/bootstrap.sh:148-155` still passes `--verify-jobs 1` (verifying) to Stage 2; Stage 3 verifies unless `--stage3-no-verify` |
| #179 codegen/verify overlap | OPEN | `run_build_cmd` (`compiler/main.vow:2125-2271`): `build_verify_phase` -> `build_codegen_phase` -> `build_drain_verify_phase` |
| #177 embedded help/spec payload | OPEN | `compiler/main.vow` is 1.3 MB; lines 2636-15969 are generated `GENERATE:SKILL_*` blocks of one `r.push_str(String::from("..."))` per source line (~13k calls) |

## Assumptions
- #174 is a tracker. This PR implements only #180 and says `Closes #180` + `Refs #174`; it must NOT say
  `Closes #174` while #177/#179 are open. (best guess)
- The new verification pass runs the self-hosted `vowc verify` (default ESBMC backend, `--verify-jobs 1`,
  forwarding `--no-cache`) on `build/vowc2`, after the SHA-256 match and before promoting `vowc2` -> `vowc`.
  Reason: it checks the exact binary being promoted, a codegen non-determinism is reported first (cheaper),
  and a verification failure leaves the previous `build/vowc` untouched. (best guess)
- Stage 1 (Rust compiler, `--verify-jobs 1`) keeps verifying `compiler/main.vow` with ESBMC. Dropping it is a
  coverage change and is out of scope. (best guess)
- `--stage3-no-verify` stays an accepted, documented no-op (Stage 3 never verifies now). Removing it would break
  external callers and the four workflow call sites for no memory gain. (best guess)
- `--backend native` is not used for the bootstrap verify pass. Its default is ESBMC; the native verifier is
  epic #1398 and has a pinned-seed story (ADR 2026-10-08-1421). (best guess)
- #179 and #177 are follow-ups, not bundled, because #179 needs a Rust-driver twin (CLAUDE.md dual-compiler
  rule, and it trades build latency for memory) and #177 changes `scripts/generate_help.py` emission with
  byte-for-byte help requirements; each needs its own measurement.

## Files to touch
| File | Role | Lines of Interest |
|---|---|---|
| `scripts/bootstrap.sh` | add the separate verify pass; Stage 2/3 always `--no-verify`; usage text | flag derivation 144-159, `run_self_stage` 177-184, Stage 2 212-224, Stage 3 226-238, fixed-point block 240-258 (`mv` at 248), `usage` 22-60 |
| `tests/bootstrap/tests.sh` | red/green tests for the new invocation sequence | whole file; `run_bootstrap` 59-74, count assertions at 91, 128, 145 |
| `tests/bootstrap/fake_compiler.sh` | fake compiler must accept `verify` (no `-o`), simulate verify failure and fixed-point divergence | whole file (22 lines) |
| `scripts/test_bootstrap_workflow.py` | structural guard that CI bootstrap legs still verify | `test_bootstrap_verifies_with_esbmc` 105-117 |
| `.github/workflows/bootstrap.yml` | drop `--stage3-no-verify`, fix step names/comments | 63, 78-79, 106-107, 124-125, 130-131, 147-148 |
| `.github/workflows/equivalence.yml` | drop `--stage3-no-verify` | 55 |
| `docs/verifier-eval.md` | local-run snippet | 137 |
| `scripts/full_test.sh` | NO change; Section 8b already runs `tests/bootstrap/tests.sh` as `bootstrap/smoke` | 1982-1986 |
| `compiler/main.vow` `run_verify` | NO change; already accepts `--verify-jobs`, `--no-cache`, exits 1 on `Skipped` (`outcome != 0`) | 1881-1950 |

## Steps (ordered; each is one commit)

### 1. Extend the fake compiler (test scaffolding, no behaviour change)
- **File**: `tests/bootstrap/fake_compiler.sh`
- **Change**: log `$0 $*` (so tests can assert which binary — `build/vowc` vs `build/vowc2` — ran each
  invocation; update the existing `assert_*` helpers, which only substring-match, accordingly). Then, if the first non-flag argument is `verify` (or `$1 == verify`): exit 0, or
  exit 1 printing `fake compiler: verification failed` when `VOW_BOOTSTRAP_TEST_VERIFY_FAIL=1`. For `build`,
  if `VOW_BOOTSTRAP_TEST_DIVERGE=1` and the `-o` path ends in `vowc3`, append one byte to the copy so
  `sha256(vowc2) != sha256(vowc3)`.
- **Why first**: current script exits "missing -o output" for `verify`, so the new bootstrap would fail for the wrong reason.

### 2. RED: write the new bootstrap tests
- **File**: `tests/bootstrap/tests.sh` (add helper `run_bootstrap_expect_failure`; add/adjust tests)
  1. `test_default_sequence`: default run logs exactly 4 invocations. #1 (`target/release/vow build`)
     contains `--verify-jobs 1` and not `--no-verify`. #2 (`build/vowc build`) and #3 (`build/vowc2 build`)
     contain `--no-verify` and not `--verify-jobs`. #4 is logged as `build/vowc2 verify ...`, contains `--verify-jobs 1` and
     `compiler/main.vow`, and has no `-o`.
  2. `test_no_verify_runs_no_verify_pass`: `--no-verify` -> 3 invocations, all `--no-verify`, none are `verify`
     (existing `test_combined_flags_*` already covers the same count; keep it).
  3. `test_stage3_no_verify_is_a_noop`: `--stage3-no-verify` alone yields the same 4-invocation sequence as default.
  4. `test_no_cache_forwarded_to_verify_pass`: `--no-cache` appears in all 4 invocations; update the existing
     `test_no_cache_flag_is_forwarded_to_all_stages` count 3 -> 4.
  5. `test_verify_failure_fails_bootstrap_without_promotion`: with `VOW_BOOTSTRAP_TEST_VERIFY_FAIL=1`, exit != 0,
     stdout lacks `Bootstrap successful`, and `build/vowc2` still exists in the fake repo.
  6. `test_fixed_point_mismatch_skips_verify_pass`: with `VOW_BOOTSTRAP_TEST_DIVERGE=1`, exit != 0, invocation
     log has 3 lines (no `verify`).
- **Run**: `bash tests/bootstrap/tests.sh` (expected RED on 1, 3, 4, 5).

### 3. GREEN: implement the sequencing in `scripts/bootstrap.sh`
- **File**: `scripts/bootstrap.sh`
- **Change**:
  - Stage 1 flags unchanged (`--verify-jobs 1`, or `--no-verify` under `--no-verify`).
  - Stage 2 and Stage 3 flags: always `--no-verify`, plus `--no-cache` when set. Delete `stage3_build_flags`
    and the `STAGE3_NO_VERIFY` branch from the flag derivation.
  - `--stage3-no-verify`: still parsed. Keep the "`--no-verify` supersedes" warning so
    `tests/bootstrap/tests.sh` `WARNING` assertions stay valid. Mark it "retained for compatibility; Stage 3
    never verifies" in `usage`.
  - New block after the SHA match and before `mv build/vowc2 build/vowc`:
    `printf "${BOLD}Verify (self-hosted):${RESET} build/vowc2 verify compiler/main.vow\n"`, then, unless
    `NO_VERIFY=true`, `run_self_stage "build/vowc2 verify --verify-jobs 1 [--no-cache] compiler/main.vow"`;
    on failure print `FAILED` and `exit 1`. Under `--no-verify` print `skipped (--no-verify)`.
  - Reuse `run_self_stage` (honours `VOW_BOOTSTRAP_VMEM_KB`) and the `NO_CACHE` forwarding.
  - Update `usage` Stages list (Stages 2-3 `--no-verify`; new "Verify: self-hosted `vowc verify`"), and
    the `--no-cache` paragraph: the self-hosted compiler has no verify cache (`compiler/verifier.vow:1675-1685`,
    `verify_start` :1853 — `use_cache` is kept on signatures for symmetry only) and no compile-object cache
    (`build_codegen_phase` -> `clif_emit_module(ir_mod, out, mode, trace)` takes no cache argument and
    `grep -i cache compiler/clif.vow` is empty), so `--no-cache` on Stages 2-3 and the verify pass is
    CLI symmetry only. Make that explicit in `usage`.
- **Reuses**: `run_self_stage` (`bootstrap.sh:177`), `sha256_file`.

### 4. CI + docs wording (separate commit; mechanical)
- `.github/workflows/bootstrap.yml`, `.github/workflows/equivalence.yml`: call `scripts/bootstrap.sh`
  (`--skip-cargo` where already present) with no `--stage3-no-verify`; step name "Bootstrap (Stage 1 and
  self-hosted verify, fixed-point check)"; fix the three header comments.
- `scripts/test_bootstrap_workflow.py::test_bootstrap_verifies_with_esbmc`: assert `scripts/bootstrap.sh` is
  present, neither `--no-verify` nor `--stage3-no-verify` appears on the invocation line, and `install-esbmc`
  is still in the job (this preserves the guard's purpose: a bare `--no-verify` must not creep in).
- `docs/verifier-eval.md:137`: `scripts/bootstrap.sh`.
- **Run**: `python3 scripts/test_bootstrap_workflow.py` (or `python3 -m unittest` per how CI invokes it).

### 5. Measure and record (PR description, not a repo file)
- On a machine with ESBMC: `scripts/bootstrap.sh --skip-cargo --no-cache` before/after, with
  `/usr/bin/time -v` on the Stage 2 and the verify pass. Record peak RSS and wall time for the Stage 2 build
  (now `--no-verify`) and for `vowc2 verify`. Record head SHA in the checklist line (CLAUDE.md "green locally"
  rule). Use `scripts/measure_bootstrap_rss.sh` for the Stage 2 `--no-verify` number.
- If ESBMC or the sandbox cannot run it, say so explicitly in the PR; do not claim a green bootstrap.

## TDD slices (summary)
1. Fake compiler grows `verify` / failure / divergence modes — `tests/bootstrap/fake_compiler.sh`.
2. RED: 6 tests in `tests/bootstrap/tests.sh` (above).
3. GREEN: `scripts/bootstrap.sh` sequencing; Tests 1-6 pass.
4. Guard test + CI/doc wording: `scripts/test_bootstrap_workflow.py` red against old yml -> green after edit
   (edit the test first, run it red, then the yml).
5. Gate: `bash tests/bootstrap/tests.sh`, `python3 scripts/test_bootstrap_workflow.py`, `shellcheck`/`bash -n
   scripts/bootstrap.sh`, then a real `scripts/bootstrap.sh --skip-cargo --no-cache` (run in the foreground with an explicit timeout, or poll a done-marker file; ~5-40 min; no background-and-wait).

## Verification surface
- No Vow source, contract, IR, or C-model change; no new ESBMC properties. The change is scheduling only.
- Coverage preserved, stated precisely: before, `compiler/main.vow` was ESBMC-verified at Stage 1 (Rust),
  Stage 2 (self-hosted), and Stage 3 (CI skipped). After: Stage 1 (Rust) and one self-hosted `verify` pass
  over the same source on the fixed-point binary. The self-hosted verifier still gets exercised on its own source.
- `vowc verify` fails closed: `Skipped`/`VerifyFailed` -> exit 1 (`verify_finish`, `compiler/main.vow:1952`),
  so the new step fails bootstrap exactly as `build` did.
- No `tests/run/`, `examples/` or `docs/spec/*.md` change: nothing in the language or the CLI changes
  (`docs/spec/cli.md` does not document `bootstrap.sh`). No `generate_help.py` regeneration needed.

## Risk areas
- **Fixed point**: Stage 2/3 under `--no-verify` must emit the same bytes as under verify. Verification
  doesn't alter codegen (existing `--stage3-no-verify` already relies on this and CI has run it). The
  self-hosted compiler has no compile-object cache, so `--no-verify` cannot serve stale objects here (unlike
  Rust Stage 1 under `--no-verify`). Residual: Stage 2 output vs Stage 3 output are both `--no-verify` now, so
  the check compares like with like — strictly safer than before.
- **Mutation oracle**: `vowc mutants` Tier 1 runs `scripts/bootstrap.sh --skip-cargo`
  (`compiler/mutants_defaults.vow`). A `contract-weaken` mutant still dies, via Stage 1 (Rust) or the new
  verify pass. Wall time shifts (verification moves from Stage 2 to the end); unchanged in total.
- **Wall time**: the verify pass is serial after Stage 3 rather than partly overlapped with Stage 2 codegen.
  Expect a small increase for the default path, offset by dropping Stage 3's verification where
  `--stage3-no-verify` wasn't used (local default). CI 90-minute budget has headroom; confirm in Step 5.
- **VMEM cap**: `ulimit -v` now also wraps the verify pass and its ESBMC child (as it did Stage 2). A user
  with a tight `VOW_BOOTSTRAP_VMEM_KB` tuned for `build` should see lower, not higher, need.
- **`--no-verify` warning text**: the test asserts the exact stderr line; don't reword it.
- **Dual-compiler rule**: script-only change; no compiler semantics change, so no Rust/self-hosted twin needed.
- **`parse -> print -> parse` idempotency, verifier C parity (`c_emitter.{rs,vow}`), binary-fixed-point codegen
  ordering, `BTreeMap` determinism, `vow-clif-shim` stack slots, clippy**: N/A — no syntax, codegen, IR or
  C-emitter change, no Rust change.
- **Concurrent runs**: #179 and #177 carry `ready-for-agent` and may be planned/implemented in parallel
  workspaces; this slice touches only `scripts/`, `tests/bootstrap/`, workflows, one doc, so it cannot conflict
  with them. #180 itself has no labels or assignee at planning time.
- **clippy / parity / commitlint**: untouched by this slice (no Rust, no `c_emitter`). PR title must be
  lower-case Conventional Commits, the implementer picks the type: `perf`/`fix` bump the
  release version under semantic-release even for a script-only change, `build`/`chore`/`ci` do not. A
  suitable subject is `build(bootstrap): verify the fixed-point binary separately from stages 2 and 3`
  (<= 92 chars, lower-case, no trailing period). Do not claim "verify once": Stage 1 still verifies.
- **Sandbox**: ESBMC may be unavailable; the bash tests use the fake compiler and don't need it.

## Follow-ups (own PRs; each needs its own plan)
1. **#179 build scheduling** — in `run_build_cmd`, run `build_codegen_phase` before `build_verify_phase`
   (or bound the overlap), keeping JSON status, `Unverified`/`Verified` semantics, and the "stop launching
   after first failure" rule. Requires the Rust driver twin (`vow/src/main.rs`), a decision on the latency
   trade (opt-in flag vs default), and before/after RSS numbers via `scripts/measure_bootstrap_rss.sh
   VOW_RSS_INCLUDE_VERIFY=1`. Parity fixtures under `tests/verify*/` must stay byte-identical C.
2. **#177 payload cost** — change `scripts/generate_help.py::inject_vow` / `inject_skill_vow` to emit each
   payload fn as one escaped literal per chunk (or move the payload into its own module), and prove it
   with `generate_help.py --check`, `check_help_coverage.py`, byte-for-byte `skill print` and `--help`
   output, and a Stage 2 RSS delta. First verify the lexer/IR accept very long literals and that
   dead-function elimination does not already drop unreferenced payload fns.
3. Re-evaluate whether Stage 1 (Rust) verification can be delegated to the pinned seed once epic #1398 lands.

## Out of scope
- Any `compiler/*.vow` or Rust crate change; any contract change (contracts are not to be weakened).
- #179 and #177 (above); removing `--stage3-no-verify`; changing Stage 1 verification; `--backend native`.
- Formatting/refactoring of `bootstrap.sh` beyond the stage flag derivation; touching `full_test.sh`.
