# Plan: baseline (unmutated-tree) oracle check for `vowc mutants run` (#617)

## Goal
Run the Tier-1 and Tier-1.5 oracles once on the unmutated worktree before the per-mutant loop in `run_mutants_run`; if either does not exit 0, abort with `baseline oracle failed; mutation results would be meaningless`, so a globally broken oracle (e.g. missing `./target/release/vow` in the fresh worktree) can no longer score every mutant `caught`.

## Scope / classification
Medium. Self-hosted only: `vowc mutants` has no Rust twin (`docs/spec/cli.md:202`), so the dual-compiler rule does not apply. No language, grammar, contract, C-model or verifier change.

## Assumptions
- Baseline covers Tier 1 + Tier 1.5, not Tier 2 (best guess). The issue says "ideally Tier-2", but the full suite is ~30-46 min and would be paid once per shard before any result. Tier 1.5 (~10 min, `docs/mutants.md`) shares the same failure mode (`classify_oracle_rc` maps any nonzero rc to `caught`) and already runs the same `full_test.sh` prefix that Tier 2 runs, so a Tier-2-only breakage is far less likely. Tier-2 baseline is a listed follow-up.
- Baseline reuses the per-tier timeouts (`--tier1-timeout-secs`, `--tier15-timeout-secs`); no new timeout flags.
- Add an opt-out flag `--skip-baseline` (best guess): needed by the existing smoke tests that deliberately use failing oracles (`tests/mutants/tests.sh:154`, `:871`) and by hermetic callers; default is ON (safe). Alternative (no flag, rewrite those tests) rejected: removes a legitimate escape hatch for oracles that are only meaningful on mutated trees.
- Abort exit code is 1, same as the existing failure paths (`mutants_main.vow:520-523,642`). Abort happens right after `worktree_create` and before `enumerate_root`/`write_mutants_json`, so no misleading `mutants.json`/`outcomes.json` is left behind.
- A baseline rc of -1 (spawn error) and -2 (timeout) also abort, with distinct reasons.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `compiler/mutants_main.vow` | `run_mutants_run`: add flag parse, call baseline helper, cleanup on failure; new `run_baseline` helper | 487-659 (flags 496-506; worktree 519-529; loop 559+); `cd_with_log` 291-313; `run_shell` use 601 |
| `compiler/mutants_oracle.vow` | Pure helper `baseline_failure_reason(rc: i64) -> String` next to `classify_oracle_rc`-style helpers; `run_shell` already here | 33-75 |
| `compiler/tests/test_mutants_baseline.vow` [new] | Unit test of `baseline_failure_reason` (pattern: `compiler/tests/test_mutants_main_tier15_default.vow`) | - |
| `tests/mutants/tests.sh` | Smoke tests: add `--skip-baseline` to the two failing-oracle tests; add new baseline tests | 96-100 (`do_run`), 154, 871, tail list ~895 |
| `docs/mutants.md` | Flag table row, new "Baseline check" section, adjust Tier-1 note and Caveats (worktree `target/`) | 11-30, 32-37 |
| `docs/spec/cli.md` | `vowc mutants run` usage block + flag table row | 207-227 |
| `compiler/main.vow` | Embedded help/skill copies of the cli.md section (generated) | ~5421-5440, ~11345-11365 |
| `vow/src/skill.rs`, `skills/vow/reference/cli.md` | Generated copies of the same section | via script |

## Steps

### 1. Add the pure verdict helper (TDD slice 1, red→green)
- **Test first** `compiler/tests/test_mutants_baseline.vow` [new]: `use mutants_oracle`; assert `baseline_failure_reason(0) == ""`, and that rc `-1`, `-2`, `1`, `124` each give a non-empty reason, with `-1` containing "spawn", `-2` containing "timed out", and `1` containing "exit code 1". Run: `build/vowc test compiler/tests/test_mutants_baseline.vow`.
- **Code** `compiler/mutants_oracle.vow`: add `fn baseline_failure_reason(rc: i64) -> String` (pure, no effects); `""` means baseline passed. Compose with `i64_to_string` as `tier_json_str` does.
- Keeps the policy testable without a worktree (seam), keeps `run_mutants_run` from growing a branchy block.

### 2. Add the effectful baseline runner
- **File** `compiler/mutants_main.vow`: new `fn run_baseline(workdir, tier1_cmd, tier15_cmd, t1_to_ms, t15_to_ms, log_p) -> i64 [io, read, write]` placed near `cd_with_log`. Returns 0 on success; on failure prints to stderr (`eprintln_str`) `vow-mutants run: baseline oracle failed; mutation results would be meaningless`, then `  tier <1|1.5> baseline: <reason>`, then `  see <log_p>` and returns 1.
- Runs Tier 1 via `cd_with_log(workdir, tier1_cmd, log_p, "baseline-tier1", false)` + `run_shell(.., t1_to_ms)`; if `baseline_failure_reason(rc) == ""` then Tier 1.5 via `cd_with_log(.., "baseline-tier15", true)` + `run_shell(.., t15_to_ms)`. Reuses `cd_with_log` (`mutants_main.vow:291`), `run_shell` (`mutants_oracle.vow:61`).
- Log goes to `<output_dir>/logs/baseline.log` (`log_path_for` yields numeric ids only, so build the path with `join_path(join_path(output_dir,"logs"),"baseline.log")`; per-mutant `logs/<id>.log` files are untouched, so id-alignment with `outcomes.json` is preserved).

### 3. Wire into `run_mutants_run` (slice 2, driven by smoke tests)
- **File** `compiler/mutants_main.vow` after the `worktree_create` success check (~line 529): `let skip_baseline: bool = has_flag(argv, String::from("--skip-baseline"));` and `if !skip_baseline { if run_baseline(...) != 0 { worktree_remove(workdir); fs_remove_dir(lock_path); return 1; } }` — same cleanup pair as the restore-failure path (`mutants_main.vow:640-643`), so the lock is released and no `/tmp/vow-mutants-*` worktree leaks.
- Check how unknown flags are handled (`get_flag_arg`/`has_flag`; `grep -n "has_flag" compiler/mutants_main.vow`) and whether a flag allow-list/usage text in `mutants_main.vow` needs `--skip-baseline` added.
- The `t1_to_ms`/`t15_to_ms` values already exist (lines ~503-505); move nothing.

### 4. Smoke tests in `tests/mutants/tests.sh` (written BEFORE step 3 → red, then green)
- Existing tests that use a deliberately failing oracle would now abort: add `--skip-baseline` to `:154` (`--tier1-cmd 'false'`) and `:871` (`--tier15-cmd 'false'`). `:166` (`--tier2-cmd 'false'`) and the `true`-oracle tests need no change and now implicitly cover the passing-baseline path.
- New `t29_baseline_failure_aborts_run`: `--tier1-cmd 'false'` (no skip) → assert rc ≠ 0, stderr contains `baseline oracle failed`, `outcomes.json`/`mutants.json` absent, `.lock` absent (released), worktree count unchanged (same before/after pattern as T10 at `tests/mutants/tests.sh:112-130`), `logs/baseline.log` exists. `do_run` discards stderr (`>/dev/null 2>&1`), so call `run_vowm` directly for stderr capture.
- New `t30_baseline_tier15_failure_aborts_run`: `--tier1-cmd 'true' --tier15-cmd 'false'` → same assertions, stderr mentions tier 1.5.
- New `t31_skip_baseline_restores_old_behaviour`: `--tier1-cmd 'false' --skip-baseline` → rc 0, ≥1 `"status":"caught","tier":1,`.
- New `t32_relative_target_regression` (the issue's actual scenario): `--tier1-cmd 'test -x ./does-not-exist'` → aborts. Register all in the main list at the bottom (note the list currently repeats t9/t10; leave as is).
- Check `t14_per_mutant_diff_and_log_captured` (probe commands at :358): baseline now runs `echo TIER1 PROBE` into `baseline.log`, not per-mutant logs; verify any `ls logs | wc -l` style assertion there tolerates the extra file.

### 5. Docs and generated copies
- `docs/mutants.md`: add `--skip-baseline` to the usage block and flag table; new section "Baseline check" (what runs, tiers 1 and 1.5 only, Tier 2 not baselined, where the log is, exit 1, how to opt out); amend the Worktree caveat (empty `target/` now yields an immediate, loud baseline failure rather than silent all-caught) and drop/adjust the "Build-vs-test failures both classify as caught" limitation wording to mention baseline only guards global breakage.
- `docs/spec/cli.md` (`vowc mutants run` block, ~207-227): same flag + one-line note. Mention the abort exit code 1.
- Regenerate embedded copies: `uv run python scripts/generate_help.py` (updates `compiler/main.vow`, `vow/src/skill.rs`, `skills/vow/reference/cli.md`); then `python3 scripts/check_help_coverage.py`. Do not hand-edit generated regions.
- `docs/spec/schemas/mutants-result.schema.json`: no change (no new output fields).

### 6. Rebuild and verify (separate commands, never `&&`-chained; long runs in foreground with explicit timeout, bootstrap ~5 min)
1. `scripts/bootstrap.sh --skip-cargo --no-cache` (self-hosted compiler changed → must reach fixed point; record head SHA per CLAUDE.md).
2. `build/vowc test compiler/tests/test_mutants_baseline.vow`, then `build/vowc test compiler/` filter `mutants`.
3. `VOWC_BIN=build/vowc bash tests/mutants/tests.sh`.
4. `python3 scripts/check_help_coverage.py`, `python3 scripts/generate_operations.py --check`.
5. Manual repro of the issue: from a checkout with no `target/`, `build/vowc mutants run --root tests/fixtures/mutants` must now exit 1 with the baseline diagnostic instead of `caught: N, missed: 0`.
Full `scripts/full_test.sh` (Section 12 runs `tests/mutants/tests.sh`) is ~40 min; run it in the foreground only if turn budget allows, else rely on step 3.

## Verification surface
No contracts, codegen, C model or ESBMC properties change. New helper is pure and carries no `vow` block. No new `tests/run/` or `examples/` fixtures; coverage is the compiler unit test + `tests/mutants/tests.sh`.

## Risks
- **Bootstrap fixed point**: only ordinary Vow constructs; use annotated `let` for struct fields and `u64` indices per recent u64-clean changes (#1486). Re-run triple test via bootstrap.
- **Existing smoke tests silently regress**: any test with a nonzero Tier-1/1.5 command must pass `--skip-baseline` (found: 154, 871). Re-grep `tests/mutants/tests.sh` and `.github/`/`scripts/` for other `mutants run` callers before finishing (`grep -rn "mutants run" scripts .github docs`).
- **Nested invocation**: `full_test.sh` Section 12 may run inside a mutants Tier-2 oracle; the new worktree-count assertions must use before/after deltas like T10.
- **Cost**: default runs now pay Tier 1 (~5 min) + Tier 1.5 (~10 min) once per shard. Documented; `--skip-baseline` exists. Unavoidable for the guarantee requested.
- **Leaks on abort**: worktree and `.lock` must both be released on every new return path (tested in t29/t30).
- **Generated-doc drift**: forgetting `generate_help.py` fails `check_help_coverage.py` in `full_test.sh`.
- **Commit hygiene**: conventional commit, lower-case subject, e.g. `fix(mutants): run a baseline oracle on the unmutated tree before mutating`; PR title ≤ ~92 chars; implementation stage `git rm PLAN.md` before opening the PR.

## Out of scope
- Tier-2 baseline (follow-up issue; could be opt-in `--baseline-tier2`).
- Distinguishing build failures from test failures per mutant (existing documented limitation).
- Auto-symlinking `target/` into the worktree or changing the default Tier-1 command.
- Refactoring `run_mutants_run` beyond the extracted helper, formatting, or splitting `mutants_main.vow`.
- Rust compiler changes (mutants is self-hosted only).
