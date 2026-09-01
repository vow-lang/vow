# Plan: issue #1090 — counterexample replay diverges when a bounds abort preempts the vow check

## 1. Problem restated

`vow verify --replay-cex` on `tests/verify-fail/off_by_one_bounds.vow` reports `replay: diverged`
even though the verifier's verdict (`VerifyFailed`) is correct and the runtime genuinely fails.
The mismatch is one of *evidence*, not *verdict*: `last_element` indexes `v[n]` one past the Vec
it filled (`0..n-1`), so at runtime `__vow_vec_get_ptr`'s bounds check fires and calls
`std::process::exit(134)` (reserved runtime-abort status, issue #877) with `{"error":"IndexOutOfBounds"}`
on stderr — before the function ever reaches `ensures: result >= 0`. No `VowViolation` JSON is
emitted, so `classify_replay_run` (`vow/src/replay.rs`) and its self-hosted twin
(`replay_one_ce` in `compiler/main.vow`) fall through to the generic `"diverged"` bucket with the
reason "harness exited with status Some(134) but emitted no VowViolation". That bucket is meant
for genuine model/runtime disagreement (verifier predicts a violation the runtime never produces);
conflating a preempting runtime abort with that case would make CEGIS repair attempts target the
`ensures` contract when the real defect is the index expression. The fix (per the issue's cheaper,
preferred direction) is a third replay outcome — `"aborted"` — for exactly this shape: the harness
terminated via a non-`VowViolation` runtime abort before any vow predicate was evaluated. `"diverged"`
keeps its narrower, more actionable meaning: "the model and the runtime disagree about whether the
program fails."

Both compilers already emit a structured, machine-parseable marker for every runtime abort —
`{"error":"<Kind>"}` on stderr (`IndexOutOfBounds`, `ArithmeticOverflow`, `UnwrapOnNone`,
`RegionLiteralMutation`, `RuntimeInvariantViolation`, `UseAfterFree`, `OutOfMemory`, sanitizer traps;
see `docs/spec/errors.md` "Runtime Errors" and `vow-runtime/src/lib.rs`'s `VOW_RUNTIME_ABORT_EXIT`
convention, issue #877). Detecting `"aborted"` is therefore a matter of reusing that existing
diagnostic surface in the replay classifier — no new runtime instrumentation needed.

## 2. Files to touch

**Rust compiler (`vow-*` crates):**
- `vow/src/replay.rs` — `classify_replay_run`: add a `parse_runtime_abort_line` helper (sibling to
  the existing `parse_vow_violation_line`) that recognizes a stderr line shaped
  `{"error":"<Kind>"}` where `<Kind> != "VowViolation"`, and returns `<Kind>`. When no
  `VowViolation` line is found but a runtime-abort line is, return a new `"aborted"`
  `ReplayOutcome` whose reason names the abort kind and the vow_id/blame the counterexample
  predicted but never reached. Existing `"diverged"` paths (clean exit with no violation; a
  `VowViolation` with a mismatched id/blame) are unchanged.
- `vow/src/main.rs` — update the `StructuredCounterexample::replay` doc comment
  (`"confirmed"`, `"diverged"`, `"skipped"` → add `"aborted"`).
- `vow/src/report.rs` — same doc-comment update on the JSON-facing `replay` field; no schema/type
  change since `replay: Option<String>` is already free-form.

**Self-hosted compiler (`compiler/`):**
- `compiler/main.vow` — `replay_one_ce`: add a `replay_abort(reason)` constructor (sibling to
  `replay_skip`/`replay_diverge`) and a substring-based abort-marker scan (mirroring the existing
  `replay_find_violation_line` style: scan stderr lines for `"error":"` that is not
  `"error":"VowViolation"`, extract the `<Kind>` substring). Apply the same precedence as the Rust
  side: check for a matching `VowViolation` first, then an abort marker, then fall back to
  `diverged`.

**Docs (source of truth, per `CLAUDE.md`):**
- `docs/spec/cli.md` — "Counterexample replay" section (~line 367-381): add an `"aborted"` row to
  the replay-value table between `"diverged"` and `"skipped"`, and one sentence describing when it
  fires, cross-referencing the "Runtime Errors" / exit-134 convention already documented there
  (~line 261) and in `docs/spec/errors.md`.
- `docs/spec/schemas/counterexample.schema.json` — this, **not** `cli.md`, is where the machine
  enum actually lives: `"replay": { "enum": ["confirmed", "diverged", "skipped"], ... }`. Add
  `"aborted"`. Confirmed by tracing `scripts/generate_help.py`'s `build_skill_bundle`/
  `build_skill_support_files`, which inline every `docs/spec/schemas/*.json` file verbatim into the
  skill bundle and the `skills/vow/schemas/` mirror — the embedded copy in `vow/src/skill.rs` /
  `compiler/main.vow`'s `GENERATE:SKILL_FULL` block is this file's contents, not something the
  generator computes from `cli.md`'s prose table. `scripts/test_schema_check.py` /
  `scripts/schema_check.py` load `docs/spec/schemas/` and validate real CLI output against it
  (confirmed: `SCHEMA_DIR = REPO_ROOT / "docs/spec/schemas"`), so this file is load-bearing, not
  just documentation — emitting `"replay":"aborted"` without this edit fails schema validation.
- Regenerate derived artifacts after the `cli.md` and schema edits (do **not** hand-edit these):
  `uv run python scripts/generate_help.py`, then `cargo build --release -p vow` and
  `scripts/bootstrap.sh --skip-cargo`. This updates `vow/src/skill.rs`, `compiler/main.vow`'s
  generated block, `skills/vow/reference/cli.md`, and `skills/vow/schemas/counterexample.schema.json`
  (the on-disk mirror `scripts/generate_help.py` already knows how to sync/drift-check).

**Test corpus / harnesses:**
- `tests/verify-fail/off_by_one_bounds.vow` is the issue's motivating fixture but **cannot** carry
  a `// TEST: replay aborted` directive: it defines `fn main`, and the self-hosted `replay_one_ce`
  unconditionally reports `"skipped"` for any entry file that already defines `main` (documented
  self-hosted v1 limitation, `docs/spec/cli.md:379`, unrelated to and unaffected by this fix). A
  directive here would require an exact match across *both* compilers (`verify_eval.py:503`) and
  the self-hosted side would still say `"skipped"`. Leave this fixture's directives untouched;
  slice 4 checks its Rust-side outcome manually instead of via a machine-checked directive.
- New fixture `tests/verify-fail/replay_abort_bounds.vow` — follows the existing no-`main`
  convention used by `replay_confirm_scalar.vow` / `replay_skip_string.vow` so the assertion holds
  identically for both compilers. Pin the input with an equality `requires` (as
  `replay_confirm_scalar.vow` does) so reconstruction is deterministic, and make the body index a
  `Vec` one past its length before the `ensures` can be evaluated — e.g. a single scalar parameter,
  one `Vec::push`, then `v[n]` where `n` is forced to equal the vec's length. Carries
  `// TEST: replay aborted`. This fixture, not `off_by_one_bounds.vow`, is the machine-checked
  acceptance test for the new status.
- `scripts/verify_eval.py` — `VALID_REPLAY = {"confirmed", "diverged", "skipped"}` → add
  `"aborted"`. Also update the directive-parse error message at line ~210
  (`"(expected: replay <confirmed|diverged|skipped>)"`) to include `aborted` — it's a hardcoded
  string, not derived from `VALID_REPLAY`, so it silently drifts if left alone. No other change
  needed: the assertion at line ~503 (`ce.get("replay") != exp.replay_expect`) is already generic
  over the status string.
- `scripts/verifier_runtime.py` — `check_precision`: currently buckets every non-`confirmed`,
  non-`diverged` replay value into `skipped` (line ~183-184). Add an explicit `aborted` bucket so
  the PRECISION verdict distinguishes "the runtime abort preempted the vow check" (informational,
  not a failure — the issue explicitly says this is not a soundness problem) from a true
  unattempted/`skipped` replay. `PRECISION` stays reserved for `diverged`.

No changes are needed to `vow-ir`, `vow-codegen`, `vow-verify`, `vow-runtime`, or any C-model /
ESBMC-facing code — the runtime abort JSON envelopes already exist (issue #877); this is purely a
replay-classification change in both compilers' CEX-replay glue code.

## 3. TDD slices

1. **Red/green: `parse_runtime_abort_line` unit tests (Rust).**
   File: `vow/src/replay.rs` `#[cfg(test)] mod tests`. Add
   `parse_runtime_abort_line_extracts_kind_and_ignores_vow_violation`: assert it returns
   `Some("IndexOutOfBounds")` for `{"error":"IndexOutOfBounds"}`, `None` for a `VowViolation` line,
   `None` for plain text, and `Some("ArithmeticOverflow")` for
   `{"error":"ArithmeticOverflow"}`. Write the helper (a thin `serde_json`-based parse, sibling to
   `parse_vow_violation_line`) to make it pass.

2. **Red/green: `classify_replay_run` gains the `"aborted"` branch (Rust).**
   File: `vow/src/replay.rs`, same test module. Add
   `classify_replay_run_reports_aborted_when_bounds_check_preempts_vow_check`: call
   `classify_replay_run(false, Some(134), "{\"error\":\"IndexOutOfBounds\"}\nindex out of bounds\n", &ce(...))`
   and assert `status == "aborted"` and the reason mentions `IndexOutOfBounds` and the predicted
   `vow_id`/blame. Add a second case confirming the untouched paths still work:
   `classify_replay_run_still_diverges_on_clean_exit_with_no_marker` (existing behavior, regression
   guard). Implement the branch in `classify_replay_run` to make both pass, keeping the existing
   `Some((vid, blame))` match arm untouched.

3. **Red/green: new no-`main` fixture, both compilers (`compiler/main.vow` +
   `tests/verify-fail/replay_abort_bounds.vow`).**
   The self-hosted compiler has no `#[cfg(test)]` harness for this path; its test surface is
   `tests/verify-fail/*.vow` run through `scripts/full_test.sh` / `verify_eval.py`. Slice:
   - Update `VALID_REPLAY` and the directive-parse error message in `scripts/verify_eval.py` first
     (small, independent green step) so an `aborted` directive parses at all.
   - Add `tests/verify-fail/replay_abort_bounds.vow` (no `fn main`, pinned scalar input, out-of-
     bounds `Vec` access before the `ensures`) with `// TEST: replay aborted`. This is the red step:
     today neither compiler emits `"aborted"`, so the directive assertion at
     `verify_eval.py:503` fails for both.
   - Implement `replay_abort` + the abort-marker scan in `replay_one_ce` (`compiler/main.vow`) to
     make the fixture's self-hosted replay outcome become `"aborted"`.
   - Rebuild `build/vowc` (`scripts/bootstrap.sh --skip-cargo`) and run `scripts/full_test.sh` (or
     the narrower `verify_eval.py` invocation it wraps) against the new fixture with the self-hosted
     binary to confirm green.

4. **Green: Rust-side fixture parity check + the issue's own reproduction.**
   Run `cargo build --release -p vow` and re-run `verify_eval.py` against
   `tests/verify-fail/replay_abort_bounds.vow` with the Rust `vow` binary, confirming it also
   reports `"aborted"` — this is the machine-checked acceptance test for the new status on both
   compilers. Separately, as a manual (non-directive) sanity check, run
   `target/release/vow verify --replay-cex --no-cache tests/verify-fail/off_by_one_bounds.vow` and
   confirm its counterexample's `replay` is now `"aborted"` (not `"diverged"`) — this is the exact
   fixture named in the issue, so it's the direct evidence the bug is fixed, even though it can't
   carry a machine-checked directive (see slice-3 rationale on `fn main`). Confirm the self-hosted
   binary still reports `"skipped"` for this same fixture (unaffected pre-existing behavior).

5. **Refactor (no behavior change): `verifier_runtime.py` bucketing.**
   After slices 1-4 are green, update `check_precision`'s classification to add an explicit
   `aborted` list alongside `diverged`/`skipped`/`confirmed`, and treat `aborted` as `ok` (not
   `PRECISION`) in the verdict, with a distinct detail message so a report reader can see how many
   counterexamples were confirmed vs. aborted-before-reaching-the-check. Since this script has no
   existing unit tests for `check_precision` (only directive-regex tests in
   `scripts/test_verifier_runtime.py`), verify manually by running
   `python3 scripts/verifier_runtime.py tests/verify-fail/off_by_one_bounds.vow --verifier target/release/vow`
   and confirming the PRECISION direction now reports `ok`, not `PRECISION`.

Each slice after 1-2 is independently revertible; 1-2 are the load-bearing logic change, 3-4 are
the fixture proving it against both compilers, 5 is bookkeeping in the discovery tool that
motivated the issue.

## 4. Verification surface

This change touches only replay-harness classification glue, not contracts, IR, codegen, or the
C model handed to ESBMC. No new ESBMC properties are introduced and no existing `requires`/`ensures`
clause changes. `off_by_one_bounds.vow`'s own contract stays exactly as written — the bug it
demonstrates (`v[n]` one past a `0..n-1` fill) is a real defect, not a verification artifact, so
`requires`/`ensures` on `last_element` are untouched. `tests/run/` and `examples/` need no new
fixtures: the only new coverage is the `replay` classification path, exercised by unit tests
(slice 1-2) and the new `tests/verify-fail/replay_abort_bounds.vow` fixture (slice 3-4), with
`off_by_one_bounds.vow` itself serving as a manual (non-directive) confirmation.

## 5. Risk areas

- **Binary fixed point.** `compiler/main.vow` changes are ordinary control-flow additions
  (`replay_abort`, an extra string-scan helper) with no new nondeterministic iteration order —
  string/Vec operations here mirror the existing `replay_find_violation_line` pattern, which is
  already fixed-point-safe. No `HashMap` is introduced; the abort-marker scan is a linear byte scan
  like its `VowViolation` sibling. Re-run the bootstrap triple test
  (`scripts/concat_vow.sh` + three-stage compile + `sha256sum`) after the self-hosted edit, per
  `CLAUDE.md`'s Vow Compiler guidance ("run the full test suite after changes to both").
- **`parse → print → parse` idempotency.** Not touched — no AST/grammar change.
- **`cargo clippy --all -- -D warnings`.** The new `parse_runtime_abort_line` helper should follow
  the existing `parse_vow_violation_line` shape closely enough to avoid new lint surface (same
  `serde_json::Value` access pattern already in the file).
- **Generated-file drift.** `vow/src/skill.rs`, `skills/vow/reference/cli.md`,
  `skills/vow/schemas/counterexample.schema.json`, and the generated block in `compiler/main.vow`
  must be regenerated from `docs/spec/cli.md` **and** `docs/spec/schemas/counterexample.schema.json`
  via `scripts/generate_help.py`, not hand-edited — `scripts/check_help_coverage.py` and the
  `skills/vow/` drift check (both run in `full_test.sh`) will catch drift if this is missed.
  Forgetting the schema-file edit specifically is the likely failure mode here, since it is not
  the file `CLAUDE.md`'s generic "update the spec, then regenerate" guidance calls out by name.
- **Detection precision.** The abort-marker scan must not fire on a `VowViolation` line (already
  guarded by checking `VowViolation` match first and excluding it by name in the marker parser) and
  must not misclassify an unrelated non-zero exit with no structured marker (e.g. a genuine segfault
  outside the `{"error":"..."}` convention) as `"aborted"` — that case must remain `"diverged"`,
  since there we have no positive evidence of which mechanism preempted the check.
- **`replay_expect` directive strictness.** `verify_eval.py` does an exact string match
  (`ce.get("replay") != exp.replay_expect`) across *all* counterexamples for a fixture. If
  `replay_abort_bounds.vow` ever produces more than one counterexample (unlikely for a single
  `ensures`, but ESBMC's multi-property mode is in play elsewhere in this codebase), every one must
  replay to `"aborted"` or the directive will need per-counterexample handling — confirm during
  slice 3 that exactly one counterexample is produced for the new fixture before finalizing the
  directive.

## 6. Out of scope

- The issue's second, more expensive suggested direction — making the debug-mode bounds check
  itself emit a `VowViolation`-shaped diagnostic instead of `IndexOutOfBounds` — is explicitly not
  pursued; the issue calls the `"aborted"`-state approach "cheaper" and preferable because it keeps
  `"diverged"` meaning "model and runtime disagree," and conflating bounds/overflow/etc. aborts into
  the `VowViolation` shape would blur a distinction (contract violation vs. runtime-representation
  failure) that's independently useful.
- No change to `vow-runtime`'s abort JSON envelopes, exit-code convention, or any other runtime
  abort kind's message shape (issue #877's territory) — this plan only *reads* that existing
  surface from the replay harness's stderr.
- No new fixtures for every runtime-abort kind (`ArithmeticOverflow`, `UnwrapOnNone`,
  `RegionLiteralMutation`, `RuntimeInvariantViolation`, `UseAfterFree`, `OutOfMemory`) preempting a
  vow check — the detection logic is generic over the `{"error":"<Kind>"}` shape (by design, so it
  doesn't need a per-kind allowlist), and unit tests (slice 1) cover more than one kind string. A
  fixture sweep across every abort kind is a reasonable follow-up but is not needed to close this
  issue and would bloat this PR beyond a surgical fix.
- No changes to `scripts/equivalence.py`, `scripts/parity.py`, `scripts/test_parity.py`, or any
  other differential-testing script that has its own independent `"skipped"`/`"diverged"`
  vocabulary unrelated to `--replay-cex`'s `replay` field — grepped during planning and confirmed
  unrelated (they track compiler-pair equivalence and test-count parity, not CEX replay outcomes).
- No refactor of `classify_replay_run`/`replay_one_ce` beyond adding the new branch — both functions
  already have several branches; this plan resists the temptation to restructure them into a match
  statement or shared enum type as part of a bug fix, per `CLAUDE.md`'s "many small changes beat one
  large change."
