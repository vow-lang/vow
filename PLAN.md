# Plan: abort `vowc mutants run` when the oracle's `cd <workdir>` fails (#654)

## Goal
A `cd <workdir>` failure in the oracle wrapper must never be scored `caught`. The wrapper emits a reserved exit code (125) when `cd` fails; `run` treats it as an infrastructure failure and aborts loudly (exit 1, worktree + lock released), like the restore-failure path.

## Assumptions
- Sentinel is 125 (as proposed in the issue). It is also what coreutils `timeout(1)` returns when `timeout` itself fails, which is likewise infrastructure. An oracle that itself exits 125 is indistinguishable and aborts the run; that is the safe direction (loud, not silently `caught`) and is documented. Best guess.
- `fs_exists(workdir)` probes are NOT added: racy, and miss permission-denied `cd`. The sentinel alone closes the hole. Best guess.
- No Rust twin: `mutants` is self-hosted only (`vow/src/main.rs:1016`, `docs/spec/cli.md:208`). No `docs/spec` change: no syntax/flag/schema change; exit code stays 1 on abort.
- Redirect failure on the log file (`> log` unwritable) also exits nonzero -> `caught`. Same class of bug, different cause; out of scope (follow-up).
- Current line numbers differ from the issue: `cd_with_log` is `compiler/mutants_main.vow:297-315`, `classify_oracle_rc` :255, per-mutant loop rc1 handling ~:632-640, tier-1.5 ~:645-651, tier-2 ~:660-668, restore-abort ~:672-682, `run_baseline` :320-335.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `compiler/mutants_oracle.vow` | Home for new sentinel constant + pure predicate; `baseline_failure_reason` | 38-45 |
| `compiler/mutants_main.vow` | `cd_with_log` wrapper; per-mutant loop abort; `classify_oracle_rc` | 297-315, 255-260, 625-690 |
| `compiler/tests/test_mutants_baseline.vow` | Existing unit test pattern for oracle verdicts | all |
| `compiler/tests/test_mutants_workdir_unreachable.vow` [new] | Unit tests for sentinel + real shell wrapper | - |
| `tests/mutants/tests.sh` | E2E smoke tests (T29-T32 baseline pattern, `assert_baseline_abort` :876) | 876-953 |
| `docs/mutants.md` | Document sentinel + abort behavior (:37 baseline, :156 limitations) | 37-39, 156 |

## Steps (TDD slices, in order)

### Slice 1 — sentinel constant and predicate (red -> green)
- **Test** `compiler/tests/test_mutants_workdir_unreachable.vow` [new, `use mutants_oracle`]: `oracle_workdir_unreachable(125) == true`; false for 0, 1, 2, 124, 126, -1, -2.
- **Code** `compiler/mutants_oracle.vow`: add `fn RC_WORKDIR_UNREACHABLE() -> i64 { 125 }` (same style as `ST_*()`), and `fn oracle_workdir_unreachable(rc: i64) -> bool { rc == RC_WORKDIR_UNREACHABLE() }`. Comment: reserved exit code; a user oracle exiting 125 aborts the run by design.
- Run: `build/vowc test compiler/tests/test_mutants_workdir_unreachable.vow`.

### Slice 2 — baseline message
- **Test** extend `compiler/tests/test_mutants_baseline.vow`: `baseline_failure_reason(125)` == `"workdir unreachable (cd failed before the oracle ran)"` (keep 124 -> `"exit code 124"` assertion).
- **Code** `baseline_failure_reason` in `mutants_oracle.vow`: add the 125 branch before the generic `exit code N`. The baseline already aborts on any nonzero rc, so only the message changes.

### Slice 3 — wrapper emits the sentinel (real-shell test)
- **Test** same new test file, `use mutants_main` (verify the module root `compiler/` resolves; if importing `mutants_main` is impractical, move `cd_with_log` + `shell_quote` into `mutants_oracle.vow` — only then): `run_shell(cd_with_log("<nonexistent dir>", "true", log, "tier1", false), 10000) == 125`; with an existing dir (`.`-style tmp path) and cmd `true` -> 0, cmd `exit 3` -> 3 (the oracle's own status still passes through `;`-sequenced subshell); log file contains `--- tier1 ---` for the success case. Use `$TMPDIR`-based paths via existing `fs_*`/`join_path` helpers.
- **Code** `cd_with_log`: replace `" && (echo --- "` with `" || exit 125; (echo --- "` built from `RC_WORKDIR_UNREACHABLE()` (i64_to_string). Result: `cd W || exit 125; (echo --- L --- && CMD) > LOG 2>&1`; final status is the subshell's. Update the doc comment above `cd_with_log` (:290-296).

### Slice 4 — abort in the per-mutant loop
- **Test** `tests/mutants/tests.sh` new `t33_workdir_unreachable_aborts_run`: `run --skip-baseline --tier1-cmd 'exit 125' --tier2-cmd 'true' --root tests/fixtures/mutants --output-dir $outdir`; assert exit 1, stderr matches `workdir unreachable`, worktree released (reuse count-of-`/tmp/vow-mutants-` worktrees pattern), `.lock` removed, no `outcomes.json`. Add `t34` for the same with `--tier1-cmd true --tier15-cmd 'exit 125' --skip-baseline`. Register both in the main list near :953. (E2E can't induce a real `cd` failure because the restore `fs_write` fails first on a vanished worktree; `exit 125` exercises the identical abort glue, real `cd` covered by Slice 3.)
- **Code** `compiler/mutants_main.vow`, in the loop immediately after each `run_shell` (tier1 ~:632, tier1.5 ~:645, tier2 ~:660): if `oracle_workdir_unreachable(rc)`, restore the mutated file (`fs_write(abs_path, original)`, ignore result), `eprintln_str("vow-mutants: oracle workdir unreachable (cd failed); aborting shard")`, `eprintln_str(workdir)`, `worktree_remove(workdir)`, `fs_remove_dir(lock_path)`, `return 1`. To avoid triplicating, factor a small helper `abort_workdir_unreachable(workdir, lock_path) -> i64 [io, write]` mirroring the existing restore-abort block (:672-682); keep `classify_oracle_rc` untouched (the check runs before classification, so 125 never reaches `ST_CAUGHT`).

### Slice 5 — docs
- `docs/mutants.md`: in "Baseline check" (:37) or a new short "Infrastructure failures" paragraph: wrapper exits 125 if `cd <workdir>` fails; `run` aborts with exit 1 instead of scoring `caught`; 125 is reserved (an oracle exiting 125 aborts the run). Amend the :156 limitation sentence only if it contradicts.

## Testing
- `build/vowc test compiler/tests/test_mutants_workdir_unreachable.vow` and `test_mutants_baseline.vow`; `build/vowc test compiler/` (filter `mutants`).
- `VOWC_BIN=build/vowc bash tests/mutants/tests.sh` (needs rebuilt `build/vowc`: `scripts/bootstrap.sh --skip-cargo`, background + poll, ~5 min).
- Final: `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA (record SHA in PR checklist). Full `scripts/full_test.sh` (Section 12 covers mutants smoke) ~40 min, run in background with polling.

## Verification surface
No contracts, codegen, or C-model changes; new functions are pure/total (`bool`/`i64` returns, no `requires`), so ESBMC has nothing new to prove beyond what `vowc build` verifies by default for the touched functions (the `[io]` helper is effectful and gets no VCs). No `tests/run/` or `examples/` fixtures need to grow.

## Risks
- Fixed point: adding functions/constants to `mutants_oracle.vow`/`mutants_main.vow` changes compiler output deterministically; re-run bootstrap to confirm byte-identical b/c (no `HashMap`, no new layout-sensitive constructs).
- `scripts/concat_vow.sh` file list already includes `mutants_oracle`/`mutants_main`; a new test file is not in it (tests are standalone) — no change needed.
- Importing `mutants_main` from a test: confirm `build/vowc test` resolves its `use` closure; fallback described in Slice 3.
- Vow `||`/`;` quoting: the string is built in Vow source; keep literal `;` inside the string, and keep `shell_quote(workdir)` before ` || exit`.
- `timeout(1)` wraps `sh -c`; exit 125 passes through `run_shell` unchanged (only 124/137 are remapped).
- Rust gates (`cargo clippy`, fmt) unaffected: no Rust changes. Python lint unaffected.
- Commit/PR title lowercase conventional: `fix(mutants): abort when the oracle workdir is unreachable instead of scoring caught`.

## Out of scope
- Log-redirect failure (`> log`) scoring `caught`; build-vs-test `caught` conflation (docs/mutants.md:156).
- Writing partial `outcomes.json` on abort; periodic `fs_exists(workdir)` probes or baseline re-probes.
- Treating the mutation-write-failure `unviable` path differently; any refactor of `run_mutants_run` beyond the helper; Rust-side changes.
