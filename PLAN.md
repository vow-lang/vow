# Plan: close out clawpatch tracker #440 (Slice 1 = #415, then four follow-up slices)

## Goal
#440 is a tracking issue; 6 of its findings are still open (#420 is already CLOSED/released, #429 was folded into #427). Fixing them in one PR violates "surgical changes", so this plan orders them as independent slices, each with its own PR, and designates **Slice 1 (#415)** as the work for this run. The PR for each slice says `Closes #<child>` + `Refs #440` — never `Closes #440`; #440 is closed by hand once its checklist is fully ticked.

## Assumptions
- Triage decisions of 2026-10-03 (comments on each child) are authoritative and are not re-litigated: #415 keep schema closed (timeout = failed; missing path = `TestsFailed`, `total:0 failed:1`, one synthetic failing entry, both compilers); #419 add `command_details.skill` + `mutants` (`status:"self-hosted-only"`) and a coverage check; #421 approve as filed; #427 tighten positive proptests + fix vacuous message property (also covers #429); #428 fix runtime, not spec; #436 omit non-finite floats and unknown-tag bindings from `values`, document in `cli.md`.
- Re-audit on current `main` (d64e3aa4) found two stale premises, recorded so the implementer does not chase them:
  - **#421 is already fixed in code.** `decoded_hex` (`vow-runtime/src/lib.rs:3669-3695`) now uses `hex_str.get(i..i+2)`, which returns `None` on a non-boundary instead of panicking. What remains is the missing regression test (existing tests at `lib.rs:7053-7056` cover odd length, `"zz"`, null only).
  - **#436 string escaping is fixed** (#1069, `violation.rs:json_string`); only non-finite floats + unknown-tag `0x…` remain (`violation.rs:56,66`; doc comment at `violation.rs:43-50` explicitly defers to #436).
- **Deviation from triage text (#415): `total:1`, not `total:0`.** The triage comment asks for `total:0, failed:1` plus a synthetic failing entry; that breaks the invariant every other result holds (`total == tests.len()`, `passed+failed+skipped == total`). The synthetic entry is therefore counted: `total:1, failed:1`. Flagged on #415 so a reviewer can override.
- Slice 1 = #415 because it is the only open finding that produces a wrong pass/fail verdict (a hung test reports `TestsPassed`, exit 0). The rest are contract/diagnostic/test-quality gaps.
- #428 spec conflict (not covered by the triage comment): `docs/spec/grammar.md:1577` says the process stays running after `-2`, but `grammar.md:1579` (`process_start_capped`: "a `process_wait_timeout` expiry SIGKILL the whole group") and `:1581` ("instead of the process being killed automatically") describe kill-on-timeout. Decision: `:1577` is canonical, per triage; edit `:1579`/`:1581` to match (kill the group via `process_kill`). This is a spec edit the triage comment did not anticipate — record it in the PR body.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `vow/src/test_runner.rs` | Rust `vow test`: failed-count + missing-path early return | `build_test_result` 512-536; `run_test_command` 148-161; test at ~1005-1030 |
| `compiler/main.vow` | Self-hosted `vowc test`: missing-path early return; per-test tally | 2353-2358; `emit_test_entry_json` 2292; tally 2578-2590, 2608 |
| `docs/spec/cli.md` | `vow test` status table / per-test statuses | 175-191 |
| `vow-runtime/src/violation.rs` | Pure VowViolation renderer (#436) | `format_value` 51-69, `render_violation` 85-133 |
| `vow-runtime/src/lib.rs` | `decoded_hex` (#421); `__vow_process_wait_timeout` (#428) | 3669-3695; 4719-4795; `POLL_READERS` 41-47; `__vow_process_wait` 4640; `__vow_process_poll_wait` 4810; `__vow_process_kill` 4883 |
| `vow-types/tests/proptest_typecheck.rs` | Weak positive proptests (#427/#429) | 232-309 |
| `scripts/generate_help.py` | `commands` / `command_details` catalog (#419) | 357-366, `command_details` block following |
| `scripts/check_help_coverage.py` | Coverage checker; only checks `language` today | `main` 65-150 |
| `scripts/test_check_help_coverage.py`, `scripts/test_generate_help.py` | Python tests for the above | — |
| `skills/vow/schemas/vow-violation.schema.json`, `docs/spec/cli.md` | `values` doc (#436) | schema 37-42; cli.md 651-660 |
| `docs/spec/grammar.md` | process_* semantics (#428) | 1577-1583 |
| generated: `vow/src/skill.rs`, `compiler/main.vow` embedded help/skill payload | regenerate via `generate_help.py` after any spec/schema edit | — |

## Steps — Slice 1 (this run): #415 `vow test` timeout and missing-path

### 1. Red: Rust unit tests for the tally
- **File**: `vow/src/test_runner.rs` (tests module; extend `build_test_result_fails_closed_on_each_failure_status` ~line 1005 to include `"timeout"`)
- **Change**: add `"timeout"` to the failure-status list; assert `status == "TestsFailed"`, `failed == 1`.
- **Reuses**: `test_entry()` (line 969), `density()` helper.

### 2. Green: count `timeout` as failed
- **File**: `vow/src/test_runner.rs:518-524`
- **Change**: add `"timeout"` to the `matches!` set; update the doc comment at 505-511 ("any of `failed`, `compile_error`, `verify_failed`, `contract_skipped`, `timeout`").

### 3. Red→green: missing path yields `TestsFailed` (Rust)
- **File**: `vow/src/test_runner.rs:148-161`
- **Change**: replace the `CompileFailed` early return with a `TestResult { status:"TestsFailed", total:1, passed:0, failed:1, skipped:0, tests:[entry] }`. Entry: `file` = the path as given, `name` = file stem or path, `status:"failed"`, `exit_code:None`, empty stdout, `stderr:"test path '<p>' does not exist"`, `duration_ms:0`. Build it with `unexecuted_entry` (line 329) so the shape stays single-sourced. Keep the `eprintln!` and `exit(1)`.
- **Shape**: `total:1` (see Assumptions). Extract a pure `missing_path_result(path: &Path) -> TestResult` (built on `unexecuted_entry`) so it is unit-testable in-process; `run_test_command` only prints it, writes the `eprintln!`, and exits 1.
- **Tests**: (a) in-process unit test of `missing_path_result` (status, `total==tests.len()==1`, `failed==1`, entry `status=="failed"`, stderr names the path) — this is what carries the codecov/patch gate (blocking 95%). (b) subprocess test in `vow/tests/test_command_status.rs` [new], pattern `vow/tests/cli_dispatch.rs:11` (`CARGO_BIN_EXE_vow`): missing path → exit 1, stdout parses, `TestsFailed`, `failed==1`; it exits before compiling, so it needs no runtime or ESBMC. (c) separate hang test (`test_hang.vow`, infinite loop, `--timeout 1`, **`--no-verify`**) → `TestsFailed`, per-test `timeout`, `failed==1`, exit 1; needs a linked runtime, so if the sandbox SKIP-panics, mark only this one `#[ignore]` with the reason — (a)+(b)+step 1 still cover the patch.
- **Exit code**: `run_test_command` exits 1 iff `test_result.failed > 0` (`test_runner.rs:180-182`), and `run_selected`/`run_all` aggregate only through `build_test_result` (no second tally), so step 2 alone fixes the exit code and top-level status.

### 4. Self-hosted twin: missing path
- **File**: `compiler/main.vow:2353-2358`
- **Change**: print `{"status":"TestsFailed","total":1,"passed":0,"failed":1,"skipped":0,"tests":[{...}],"contract_density":{...}}` with the same entry fields as step 3; reuse `emit_test_entry_json` (2292) rather than a hand-written literal, and `return 1`. Per-test `timeout` is already counted failed (`main.vow:2582-2590`) — add no change there, but add the fixture below to pin it.
- **Parity**: `scripts/parity.py compare_test` (line 749) hard-requires `TestsPassed` + exit 0, so it cannot gate this path. Do not claim byte-identical JSON: `exit_code` is `null` in one compiler and omitted in the other (`cli.md` already documents this) and `duration_ms` is in `TEST_NONDETERMINISTIC_FIELDS`. The gate is the dedicated `test/missing-path` check in step 6, which compares only `status`, `total`, `passed`, `failed`, `tests[0].status`, `tests[0].file` and the exit code across both compilers.

### 5. Spec
- **File**: `docs/spec/cli.md:175-191`
- **Change**: state that `timeout` counts toward `failed` (like `contract_skipped`), and document that a nonexistent test path yields `TestsFailed` with one `failed` entry (stderr names the path), exit 1. Then `uv run python scripts/generate_help.py`.

### 6. Gate script coverage
- **File**: `scripts/full_test.sh` near 2355-2430 (the `test/…` checks)
- **Change**: add `test/missing-path` running a nonexistent path through `$RUST test` and `run_self test`; assert exit 1, `status=="TestsFailed"`, `total==1`, `failed==1`, `tests[0].status=="failed"` for both. This is a hand-written field check (see Step 4 Parity), not `run_parity test`. Add a hanging fixture under `tests/fixtures/` [new] only if it can be run with `--no-verify` and a 1 s timeout; otherwise rely on Step 1 for Rust and the existing self-hosted tally (`main.vow:2582-2590`).

## Follow-up slices (separate PRs, each independently mergeable, in this order)

### Slice 2 — #421 regression test only
- `vow-runtime/src/lib.rs` test near 7053: `"€a"` (bytes `E2 82 AC 61`, even length, valid UTF-8) and an invalid-UTF-8 case → empty Vec, no panic. No production change. Close #421 with a comment that the fix landed earlier.

### Slice 3 — #436 non-finite floats / unknown tag
- `vow-runtime/src/violation.rs`: make `format_value` return `Option<String>` (or add `json_safe_value`); in `render_violation` skip bindings for which the JSON form is `None` (non-finite `f32`/`f64`, unknown tag) **in the JSON `values` object only**; keep them in the human line (`NaN`, `inf`, `0x…`). When every binding is skipped, omit `values` entirely (matches the existing empty-bindings rule). Update the doc comment at 43-50 and the tests at 137-165.
- Tests: unit tests for `NaN`, `inf`, `-inf`, f32 variants, unknown tag, mixed finite+non-finite; plus round-trip via `serde_json::from_str` of `rendered.json`.
- Spec: `docs/spec/cli.md:651-660`, `skills/vow/schemas/vow-violation.schema.json:37-42` (state omission), `docs/spec/errors.md#vowviolation` if it repeats the values text. Regenerate help/skill embeds. No dual-compiler change: `vow-runtime` is shared by both compilers. Confirm no in-repo consumer assumes every binding is present (grep found none in `vow/src/replay.rs`/`compiler/`).

### Slice 4 — #427 (+#429) proptests
- `vow-types/tests/proptest_typecheck.rs`: add `assert_no_diagnostics(src, &diags)` helper (prints source + diagnostics) and use it in `typecheck_never_panics`, `struct_program_typechecks`, `if_program_typechecks`, `loop_program_typechecks`; in `welltyped_roundtrip_preserves_typing` assert both original and reprinted are empty. Replace `type_errors_have_messages` input with a new `arb_ill_typed_program()` (returns `bool` literal from an `i64` fn; references an unbound identifier) and assert `!diags.is_empty()` and each `message` non-empty.
- Risk: tightening may expose generator bugs (e.g., `arb_loop_program` / `arb_struct_program` using constructs the checker rejects, or `requires: a > 0` with unused `tmp`). Run with `PROPTEST_CASES=2000` once; if a generator is wrong, fix the generator, not the assertion. Test-only change.

### Slice 5 — #419 help command inventory
- `scripts/generate_help.py`: add `command_details.skill` (status `implemented`, usage `vow skill <print|install>`, scope flags from `docs/spec/cli.md` skill section) and `command_details.mutants` (`status:"self-hosted-only"`, usage from `cli.md:211-236`); add `mutants` to `commands`.
- `scripts/check_help_coverage.py`: add a check that every key of `commands` has a `command_details` entry, and every `command_details` key that is a CLI subcommand is in `commands` (explicit exemption set for the non-command entries `build_result`, `contracts_result`, `diagnostic`, `runtime_*`, `literals`, `operators`, … — better: have `generate_help.py` group them so the exemption is structural). Also cross-check `### \`vow <cmd>\`` headings in `cli.md` against `commands`.
- Tests: `scripts/test_check_help_coverage.py`, `scripts/test_generate_help.py`. Regenerate embeds; both compilers' `--help` is generated from this one script, so no hand edits.
- Risk: the Rust compiler does not implement `mutants`; the `self-hosted-only` status must be accurate in both embeds, or `scripts/check_help_coverage.py` parity checks in `full_test.sh` fail.

### Slice 6 — #428 `process_wait_timeout` must leave the child running
- `vow-runtime/src/lib.rs:4719-4795`: on timeout (`Err(-2)`) do **not** kill. Reuse the poll-reader mechanism: register the drain threads in `POLL_READERS` (as `__vow_process_poll_wait` does at 4821-4830) and re-insert `ProcessState::Running(child)`; return `-2`. The `-1` error path still kills + joins.
- **Required companion change**: `__vow_process_wait` (4640-4676) calls `wait_for_output` → `child.wait_with_output()`, which reads `child.stdout`/`stderr` that were `take()`n, so after a `-2` it would return empty output and leak the `POLL_READERS` threads. Make `__vow_process_wait` (and `wait_timeout`'s success path, which must also `remove` any pre-existing `POLL_READERS` entry before spawning new drains) join the registered readers. This also fixes the same latent hole after `poll_wait`.
- **Piped IDs**: the current `-2` path calls `piped::release(handle)`; the fixed path must NOT, or a piped child loses `process_read_line`/`process_write_stdin` after a timeout, contradicting `grammar.md:1583` (the ID stays piped until `process_wait`/`process_kill`/completion). Release only on completion and kill.
- Callers already `process_kill` on `-2` (`compiler/verifier.vow:612`, `compiler/vc_solver.vow:259`, `compiler/main.vow:2563`, `compiler/mutants_oracle.vow:89`, `main.vow:931`); `verifier.vow:562` ignores the result but the child (`uname`) exits on its own and the later `process_kill` reaps it. No Vow-source change expected; verify with grep before merging.
- Spec: `docs/spec/grammar.md:1579` and `:1581` — see Assumptions. Regenerate embeds.
- Tests: runtime subprocess test per the issue (`sh -c 'sleep 0.2; touch marker'`, `wait_timeout(h,10)` → `-2`, sleep, assert marker exists, then `process_kill`); a second test: `-2` then `process_wait` returns the real exit code and the full stdout; a third: `-2` then `process_kill` leaves no `POLL_READERS` entry. Capped-group variant: after `-2` the group is still alive; `process_kill` kills the group.
- Risk: highest of the set — changes a runtime primitive every long-running caller uses; the seed `vowc` pinned in `scripts/seed.toml` links the older runtime, so stage-0 vs seed behaviour differs until the seed is bumped. Land last.

## Testing / verification (per slice)
- Rust: `cargo test -p <touched crate>`, `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all`.
- Anything touching `compiler/` or any spec/schema: `uv run python scripts/generate_help.py`, `cargo build --release -p vow`, `scripts/bootstrap.sh --skip-cargo --no-cache`, `python3 scripts/check_help_coverage.py`, `python3 scripts/generate_operations.py --check`; then `scripts/full_test.sh` (≈40 min — run in the foreground with an explicit bound, not backgrounded). Record the final head SHA in the PR checklist.
- Pre-existing failures to verify against clean `origin/main` before blaming the slice: `u64_marker_propagation`, `contracts_tmp_cleanup`, `concrete-block-region-parity`, ~8 vow run tests that SKIP-panic without a linked runtime.

## Verification surface
No contracts, codegen, IR, or C model change in any slice; ESBMC proves nothing new, and `c_emitter.{rs,vow}` parity is untouched. No `tests/run/`/`examples/` growth is needed except the optional hanging-test fixture in Slice 1 step 6.

## Risks
- **Binary fixed point**: Slice 1 edits `compiler/main.vow` (a printed literal and one helper call). Deterministic, but re-run bootstrap `--no-cache`; do not reorder functions.
- **Two-compiler drift**: Slice 1 and Slice 5 must change both compilers in the same PR (CLAUDE.md "Vow Compiler"). Slices 2, 3, 4, 6 touch only the shared `vow-runtime` crate or Rust tests — no Vow twin exists or is needed.
- **`parse → print → parse`**: no syntax change; not at risk.
- **Embedded help/skill payload**: spec edits require regeneration of `vow/src/skill.rs` and `compiler/main.vow`; a hand edit or a forgotten regen fails `check_help_coverage.py` / drift gates.
- **Slice 1 `total` field**: see step 3 decision.
- **Slice 6 behavioural break** for any external Vow program that relied on the actual (kill-on-timeout) behaviour — a documented-contract fix, but call it out in the PR and `CHANGELOG`-relevant `fix(runtime):` title.
- **Commit/PR titles**: lower-case Conventional Commits, ≤92 chars, e.g. `fix(test): count timeout as failed and report missing path as TestsFailed`.

## Out of scope
- Closing #440 itself, or re-filing/modifying #434 (moved to vow-lang/chess#7), #420, #438, #439 (already closed).
- Any change to the closed `vow test` status schema (no new statuses, no `Error` status).
- Refactoring `test_runner.rs`, `violation.rs`, or `generate_help.py` beyond what each slice needs; formatting-only edits.
- Fixing `process_poll_wait`/`process_wait` interactions beyond the reader-join required by Slice 6.
- Tightening verification contracts or ESBMC bounds.
