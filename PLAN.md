# Plan: #1346 — false `Verified` when ESBMC prints `VERIFICATION SUCCESSFUL` after a solver failure

## 1. Problem restated

Both verifiers' output classifiers test for the literal substring
`VERIFICATION SUCCESSFUL` before checking for any error/out-of-memory marker
in the same ESBMC run. ESBMC 8.4+ (esbmc/esbmc#4484) no longer aborts when its
SMT solver throws mid-run; `do_bmc_strategy` treats an unknown base case as
"not violated" and a later forward-condition pass can still print
`VERIFICATION SUCCESSFUL` even though the base case never actually completed.
Because both `classify_esbmc_output` (`vow-verify/src/esbmc.rs:903`) and
`parse_verify_status` (`compiler/verifier.vow:519`) check `SUCCESSFUL` first
and only classify `ERROR: Out of memory` / other solver-error text afterward,
a run that hit a solver exception and then happened to print `SUCCESSFUL` is
reported `proven` — a false positive on a genuinely false contract. The fix
is to make both classifiers (and the two Rust-only probes that duplicate the
same check, `run_esbmc_reach`'s vacuity probe and `parse_unknown_reason`)
treat any `ERROR:`-flavored ESBMC output as disqualifying a `SUCCESSFUL`
verdict, while continuing to trust a genuine `VERIFICATION FAILED` (a real
counterexample) regardless of an earlier solver hiccup elsewhere in the run.

## 2. Files to touch

Rust (`vow-verify`):
- `vow-verify/src/esbmc.rs`
  - `classify_esbmc_output` (~903) — reorder/extend classification.
  - new private helper `esbmc_tool_error_reason(combined: &str) -> Option<String>`
    next to `is_memory_limit_output`/`memory_limit_reason` (~1055).
  - `parse_unknown_reason` (~1070) — early-return the captured `ERROR:` line
    instead of falling through to the generic "ESBMC returned VERIFICATION
    UNKNOWN" message.
  - `run_esbmc_reach` (~1007) — its inline SUCCESSFUL/FAILED check duplicates
    `classify_esbmc_output`'s old bug; fix independently since it does not
    call through that function.
  - `mod tests` (~1126+) — new unit tests (see TDD slices).
- `vow-verify/tests/fixtures/fake-esbmc/` — new fixture scripts:
  - `solver-error-oom-then-successful.sh` (the issue's literal repro shape:
    `ERROR: Out of memory` + `ERROR: SMT solver failed` + later `VERIFICATION
    SUCCESSFUL`).
  - `solver-error-then-successful.sh` (generic `ERROR: SMT solver failed`
    with no "out of memory" wording, + `VERIFICATION SUCCESSFUL` — exercises
    the new detector independent of the existing OOM path).
  - `solver-error-then-failed.sh` (an `ERROR:` line followed by a genuine
    `VERIFICATION FAILED` — locks in that a real counterexample still wins).
  - `reach-solver-error-then-successful.sh` (same shape for the vacuity probe).

Self-hosted (`compiler/`) — single choke point, so one slice covers build,
verify, `--multi-property`, and the vacuity/weakness probes together:
- `compiler/verifier.vow`
  - `parse_verify_status` (519) — same reorder as `classify_esbmc_output`.
  - new function `parse_solver_error_reason(combined: String) -> String`
    (returns `""` when no `ERROR:` line is found) next to
    `output_has_memory_limit`/`memory_limit_reason` (547).
  - `parse_unknown_reason` (564) — same early-return fix as the Rust mirror.
  - `verify_function_vacuity` (961) and `verify_function_multi_property` (904)
    need **no** direct changes — both already route through
    `parse_verify_status`, so the fix is inherited automatically. Verify this
    by re-reading both after the `parse_verify_status` edit, not by assuming it.
- `compiler/tests/test_verifier.vow` — new pure-function unit tests calling
  `parse_verify_status` / `parse_solver_error_reason` / `parse_unknown_reason`
  directly with string literals (mirrors the existing
  `check_memory_limit_classifier_ignores_echoed_memlimit_option` style at
  line 47 — no subprocess fixture needed on this side).

Docs:
- `docs/verifier-discipline.md` — add a short note (in "The core rule" or a
  new subsection near "Status taxonomy", ~117) stating that a terminal
  `VERIFICATION SUCCESSFUL`/`FAILED` line is only trustworthy when no
  `ERROR:`/out-of-memory marker preceded it in the same run, citing
  esbmc/esbmc#4484 as the reason this is not hypothetical. This is the
  document that already codifies the retry/soundness ladder; this bug is a
  soundness hole in the same family and belongs there.
- `docs/spec/cli.md` — no new `status` enum value is introduced (still folds
  into existing `unknown`), so no schema change. Optionally add one clause to
  the `unknown` row (~470) noting ESBMC-internal solver errors are included.
  Treat as optional polish, not required — this is not a syntax/CLI-flag/type
  change, so CLAUDE.md's mandatory-spec-update rule does not apply here.

No C-emitter, IR, or codegen files are touched — this is a pure output-text
classification fix in both verifier drivers.

## 3. TDD slices

Each slice is red→green→(refactor only if needed); do them in order, Rust
first (more test infrastructure already exists there), self-hosted second.

1. **Rust: pure detector, no I/O.**
   Red: in `vow-verify/src/esbmc.rs` tests, add
   `esbmc_tool_error_reason_detects_error_line` asserting
   `esbmc_tool_error_reason("...\nERROR: SMT solver failed\n...")` returns
   `Some("ERROR: SMT solver failed")`, and
   `esbmc_tool_error_reason_ignores_clean_output` asserting `None` for output
   with no `ERROR:`-prefixed line (use a real success transcript excerpt so
   the test doesn't just assert on an empty string).
   Green: implement `esbmc_tool_error_reason` (line-trim, `starts_with("ERROR:")`,
   return the first match) next to `is_memory_limit_output`.

2. **Rust: `classify_esbmc_output` stops trusting a tainted `SUCCESSFUL`.**
   Red: add `classify_esbmc_output_rejects_successful_after_solver_error`
   feeding the literal issue transcript (base case `ERROR: Out of memory` /
   `ERROR: SMT solver failed`, then `VERIFICATION SUCCESSFUL`) directly into
   `classify_esbmc_output`, asserting `matches!(result,
   VerificationResult::Unknown { .. })` and NOT `Proven`. Add a second test,
   `classify_esbmc_output_rejects_successful_after_generic_solver_error`, for
   the non-OOM `ERROR: SMT solver failed` + `SUCCESSFUL` shape (proves the new
   detector fires independent of the pre-existing OOM path). Add a third,
   `classify_esbmc_output_still_trusts_failed_after_solver_error`, asserting a
   genuine `VERIFICATION FAILED` after an `ERROR:` line still yields `Failed`
   (locks in the existing `failed_output_takes_priority_over_memory_limit_text`
   invariant for the *new* detector, not just the old OOM one).
   Green: reorder `classify_esbmc_output` to: `FAILED` → `Failed`; then
   `is_memory_limit_output` → `Unknown(memory_limit_reason())`; then
   `esbmc_tool_error_reason` → `Unknown(reason)`; then `SUCCESSFUL` →
   `Proven`; then the existing `UNKNOWN`/timeout/error branches unchanged.

3. **Rust: integration test through the real subprocess path.**
   Red: add `tests/fixtures/fake-esbmc/solver-error-oom-then-successful.sh`,
   `solver-error-then-successful.sh`, `solver-error-then-failed.sh` (executable,
   `chmod +x`, matching the existing fixture style at
   `vow-verify/tests/fixtures/fake-esbmc/memory-limit.sh`). Add
   `#[cfg(unix)]` tests `run_esbmc_never_reports_proven_after_solver_error`
   and `run_esbmc_still_reports_failed_after_solver_error` calling
   `run_esbmc_with_max_k_step` against them (mirrors
   `run_esbmc_reports_memlimit_output_as_unknown` at line 2239).
   Green: should already pass from slice 2 — this slice is proving the fix
   through the actual `run_esbmc_capture` → `classify_esbmc_output` pipeline,
   not adding new production code. If it fails, the gap is in argument/output
   plumbing, not classification.

4. **Rust: `parse_unknown_reason` reports the real cause.**
   Red: add `parse_unknown_reason_surfaces_solver_error_line` — feed the
   `solver-error-then-successful.sh` transcript text and assert the returned
   reason contains `"ERROR: SMT solver failed"`, not the generic fallback
   string.
   Green: add an early-return in `parse_unknown_reason`, after the existing
   `is_memory_limit_output` check, calling `esbmc_tool_error_reason` and
   returning it directly if present.

5. **Rust: vacuity probe (`run_esbmc_reach`) gets the same guard.**
   Red: add `reach-solver-error-then-successful.sh` fixture and a test
   `run_esbmc_reach_never_claims_vacuous_after_solver_error` asserting
   `ReachVerdict::Inconclusive`, not `Vacuous`.
   Green: rewrite `run_esbmc_reach`'s inline match to check `FAILED` → `Live`,
   then `is_memory_limit_output(&combined) ||
   esbmc_tool_error_reason(&combined).is_some()` → `Inconclusive`, then
   `SUCCESSFUL` → `Vacuous`, else `Inconclusive`.

6. **Self-hosted: mirror slices 1+2+4 in one pass (single choke point).**
   Red: in `compiler/tests/test_verifier.vow`, add
   `check_parse_solver_error_reason_detects_error_line`,
   `check_parse_verify_status_rejects_successful_after_solver_error` (both the
   OOM-flavored and generic-`ERROR:` transcript, as string literals — no
   subprocess needed), `check_parse_verify_status_still_trusts_failed`, and
   `check_parse_unknown_reason_surfaces_solver_error_line`, following the
   existing `return <code>` convention in that file (pick unused small
   integers, e.g. 45–49).
   Green: implement `parse_solver_error_reason` in `compiler/verifier.vow`
   next to `output_has_memory_limit`; reorder `parse_verify_status` the same
   way as `classify_esbmc_output`; add the same early-return to
   `parse_unknown_reason`.
   Confirm (read, don't just assume) that `verify_function_multi_property`
   (904) and `verify_function_vacuity` (961) now correctly demote a tainted
   run without any further edit, since both call `parse_verify_status`.

7. **Docs.**
   Update `docs/verifier-discipline.md` per Section 2 above. Run
   `uv run python scripts/generate_help.py` only if `docs/spec/cli.md` is
   actually touched (it likely won't need to be); otherwise skip — this
   avoids an unrelated regen diff.

Run after every slice: `cargo test -p vow-verify` (fast) and, once both
compilers are edited, `cargo test --all`, `cargo clippy --all -- -D warnings`,
then the self-hosted test entry point for `compiler/tests/test_verifier.vow`
(`scripts/bootstrap.sh --skip-cargo` to rebuild `build/vowc`, then whatever
`vowc test compiler/` invocation the harness uses — check
`scripts/full_test.sh` for the exact self-hosted test-running incantation
before assuming a flag name).

## 4. Verification surface

This change touches only the classification of ESBMC's own stdout/stderr
text — no C model, IR, or contract semantics change, so there is no new
ESBMC property to prove and no change to `parse → print → parse`
idempotency. Concretely:

- No new test fixtures are needed under `tests/run/`, `tests/verify/`, or
  `examples/`. The issue's own suggested-fix text explicitly recommends
  testing this "without a real solver, with a fake `esbmc` on PATH" — the
  fake-esbmc fixtures in slice 3/5 are the correct verification surface, not
  a new real-ESBMC-dependent fixture.
- **Do not** add the issue's literal `no_factors.vow` reproducer to
  `tests/verify/` or `tests/verify-fail/`. Neither harness fits: `tests/verify/`
  asserts `status == "Verified"` and `tests/verify-fail/` asserts
  `status == "VerifyFailed"` (a genuine counterexample) — but the correct
  post-fix outcome for this contract is `unknown` or `timeout` (ESBMC's
  forward condition can't disprove it within a practical unwind bound either;
  the issue's own table shows 8.3 times out on it, it doesn't find a CE). A
  fixture with that expected outcome would also only exercise the real bug on
  ESBMC 8.5 specifically (a solver-internal memory bug); it is not a stable,
  version-independent regression test. Rely on the fake-esbmc fixtures for a
  deterministic guarantee, and record the 8.5 CI runs to watch (Risk area 1)
  as the real-world confirmation channel instead.
- The local sandboxed ESBMC is 8.3.0 (`esbmc --version`), which already fails
  closed on this bug (it aborts rather than printing a laundered SUCCESSFUL),
  so the implementation stage cannot reproduce the exact 8.5 solver behavior
  locally against a real binary — another reason the fake-esbmc fixtures,
  not a real-ESBMC fixture, are the testable surface here.

## 5. Risk areas

1. **CI fixtures that currently "pass" via this bug may change outcome.**
   The issue's own audit found one flagged real run
   (`benchmarks/hard/H02_geometry_area` `rect_area`, self-hosted) and three
   `full_test.sh` fixtures that hit the tainted-SUCCESSFUL path today without
   currently causing a *wrong* verdict (`modulo_safe`, `stdlib/geometry`,
   `stdlib/math` — all have true contracts). After this fix, an
   OOM-flavored solver error is classified as `Unknown{memory limit exceeded}`,
   which is the *exact* trigger string `retry_plan` (`vow-verify/src/
   solver_strategy.rs:263`) and its self-hosted mirror
   (`compiler/main.vow:1459`) already use to retry under `--z3 --ir` — so
   this reordering fix should transparently re-route these fixtures through
   the existing, already-tested BV→IR fallback rather than break them. This
   is expected to still converge to `proven`/`proven-ir`, just via a
   different, honest path, possibly a few seconds slower. It cannot be
   confirmed locally (local ESBMC is 8.3, which never enters this path) —
   flag explicitly for the implementation stage to watch CI on this PR, and
   do not treat a CI regression here as unrelated flakiness.
2. **Non-OOM `ERROR:` lines do not get the BV→IR retry.** A generic
   `esbmc_tool_error_reason` match (no "out of memory" wording) produces a
   *new*, distinct reason string, which `retry_plan`'s exact-string match on
   `memory_limit_reason()` will **not** recognize — so these runs report a
   plain `Unknown` and fail closed without a retry attempt. This is
   deliberate: the issue's own suggested fix explicitly defers "should a
   solver-exception BV attempt retry with --z3 --ir" as a "longer term"
   decision (item 3), and inventing a broader auto-retry trigger in the same
   PR as the soundness fix would be exactly the kind of bundling CLAUDE.md
   warns against. Do not fold this case into `memory_limit_reason()` just to
   get a free retry — that conflates two distinct causes and would make the
   `Unknown` reason text describe the wrong problem to a human/agent debugging
   it.
3. **`vow contracts --verify`'s per-clause path has an un-audited residual
   gap.** `resolve_clause_status` (`vow/src/contracts.rs:79`) deliberately
   trusts an individual `--multi-property` per-claim `PASSED` verdict *even
   when the function's overall status is not proven* (see the test
   `resolve_clause_status_prefers_per_claim_verdict_over_overall`) — by
   design, so one clause's real counterexample doesn't taint a sibling
   clause's independently-proven claim. This fix does not audit whether
   ESBMC's per-property `PASSED [...] vow:N` lines under `--multi-property`
   can themselves be printed after the same kind of solver exception
   (`do_bmc_strategy`'s base-case-vs-forward-condition confusion is described
   for whole-function incremental-BMC/k-induction, not per-property splitting
   — it is not established whether `--multi-property` mode shares the exact
   code path). Do not silently assume this is also fixed. This PR closes the
   literal reproducer (`vow verify` / `vow build --verify` / self-hosted
   `verify`, both compilers) via the single overall-status choke point; the
   per-clause path is explicitly out of scope (see below) and should be
   tracked as a fast-follow issue once someone can test against a real 8.5
   binary with `--multi-property`.
4. **Binary fixed point / clippy / idempotency.** Low risk: no IR, codegen,
   or C-emitter change, no new struct/enum layout, no `BTreeMap`/`HashMap`
   ordering touched, no new parser/printer surface. `cargo clippy --all -- -D
   warnings` risk is limited to the new helper functions' style (keep them
   small and match existing `is_memory_limit_output` conventions). The
   self-hosted binary fixed point (Stage A/B/C `sha256sum` match) is at risk
   only if `parse_verify_status`/`parse_solver_error_reason` introduce
   nondeterminism (they don't — pure string scans, no maps/iteration order
   dependent on hashing).

## 6. Out of scope

- Changing `retry_plan`/`bv_config_for` to add a new retry trigger for
  non-OOM solver errors (issue's suggested-fix item 3; explicitly deferred
  there as a separate, longer-term decision).
- Auditing or changing `resolve_clause_status`'s per-claim trust model
  (Risk area 3) — needs real ESBMC 8.5 `--multi-property` output evidence
  this plan cannot produce.
- Landing or referencing the upstream ESBMC fix (issue's suggested-fix
  item 2) — that is an esbmc/esbmc-repo change and a separate CI pin bump,
  tracked independently of this Vow-side classification fix.
- Any change to the `docs/spec/cli.md` `status` enum values — no new value
  is introduced.
- Bundling this with unrelated cleanup in `vow-verify/src/esbmc.rs` or
  `compiler/verifier.vow` (both files are large and have plenty of
  refactor-shaped temptations nearby; touch only the functions listed in
  Section 2).
