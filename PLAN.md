# Plan: issue #1182 — record a one-sided soundness gate failure alongside a confirmed finding

## 1. Problem restated

`confirm_soundness_pair` in `scripts/pair_review.py` (soundness mode) runs the
verifier-vs-debug-runtime gate independently against both C emitters and reduces the
two outcomes to a single `(verdict, detail)` pair by testing `"confirmed" in verdicts`
before `"error" in verdicts`. When one emitter's gate genuinely confirms a false proof
and the other emitter's gate merely failed to run (`skipped` → `"error"`), the function
returns `"confirmed"` and the one-sided gate failure is silently dropped — it never
reaches `result["errors"]`, so `reviewed_completely()` reports the pair as fully
reviewed even though one emitter's half of the claim was never judged. Flipping the
priority to check `"error"` first was rejected in #1172's review because `review_pair`
rewrites an `"error"` verdict to `"inconclusive"` before storing it — that would delete
a real positive from `report["confirmed"]`, the `CONFIRMED:` summary, and `main`'s exit
code. The fix the issue asks for is already proven out elsewhere in the same file:
`confirm_both_paths` (added in #1172, `scripts/pair_review.py:755`) solves the identical
problem for equivalence mode by returning an optional third tuple element — a
human-readable `unjudged` string — that `review_pair` already knows how to fold into
`result["errors"]` without changing the stored `verdict`. `confirm_soundness_pair` just
needs to adopt the same three-element contract; no other function needs to change.

## 2. Files to touch

- `scripts/pair_review.py` — widen `confirm_soundness_pair` (currently
  `scripts/pair_review.py:804-821`) to return a 3-tuple
  `(verdict, observed, unjudged_or_None)` instead of a 2-tuple, mirroring
  `confirm_both_paths` (`scripts/pair_review.py:755-781`). No other function in this
  file changes:
  - `review_pair` (`scripts/pair_review.py:958-1091`) already unpacks `confirm_fn`'s
    result with `verdict, detail, *rest = outcome` (line 1073) and already treats a
    non-`"error"` verdict's third element as an unjudged-claim message appended to
    `result["errors"]` (lines 1074-1081). This is the "contract widened to carry
    optional partial-failure metadata" the issue asks for — it was built generically
    in #1172 and only `confirm_soundness_pair` has not adopted it yet.
  - `confirm_soundness` (singular, `scripts/pair_review.py:784-801`) is unchanged — it
    already returns a plain 2-tuple per side, which is exactly what
    `confirm_soundness_pair` consumes.
  - `Mode`/`MODES` (`scripts/pair_review.py:824-846`) are unchanged — `confirm_fn` was
    always typed loosely enough (`object`) to return either tuple shape; `review_pair`
    is the only caller and already branches on tuple length.
- `scripts/test_pair_review.py` — update
  `SoundnessModeTest.test_soundness_pair_gate_checks_both_compilers`
  (`scripts/test_pair_review.py:1499-1521`) for the new 3-tuple return, and add new
  cases for the one-sided-failure-beside-a-confirmed-finding behavior (see TDD slices
  below). Pattern to mirror: `CandidateDirectiveTest.test_a_verify_path_confirmation_is_not_lost_to_the_build_path`
  and `test_a_failed_verify_gate_is_reported_beside_a_build_finding`
  (`scripts/test_pair_review.py:656-685`), which test the exact same shape against
  `confirm_both_paths`.
- `docs/equivalence/README.md` — **no change planned.** The README does not document
  `confirm_fn`'s internal tuple contract, the `errors`/`reviewed_completely` wiring, or
  exit codes at this level of detail even for `confirm_both_paths`, which shipped the
  identical mechanism in #1172 without a doc update. Adding documentation for this one
  function while the equivalence-mode sibling remains undocumented would be scope
  creep relative to the issue (see Out of scope).
- `docs/spec/*.md` — **not applicable.** This change is confined to
  `scripts/pair_review.py`, a Python CI/tooling script outside the Vow language and
  compiler. It touches no syntax, semantics, builtins, operators, effects, or CLI flag
  of the Vow language itself, so none of the "any change to X must update
  docs/spec/*.md" triggers apply.
- `compiler/*.vow` (self-hosted compiler) — **not applicable**, for the same reason:
  this is not a change to Vow language semantics or either compiler pipeline, so the
  "modify both compilers in the same session" rule does not apply to a Python
  dev-tooling script.

## 3. TDD slices

All slices run with `python3 scripts/test_pair_review.py` (this is exactly the command
CI runs — `.github/workflows/ci.yml:81`). Separately, both touched files are also
subject to the repo's pinned `ruff` pre-commit gate
(`.pre-commit-config.yaml:23-28`, `astral-sh/ruff-pre-commit` rev `v0.15.14`, hooks
`ruff-check --fix` and `ruff-format`, no path exclusion for `scripts/`) — run
`uvx ruff@0.15.14 check scripts/pair_review.py scripts/test_pair_review.py` and
`uvx ruff@0.15.14 format --check scripts/pair_review.py scripts/test_pair_review.py`
before committing, per this project's pinned-ruff convention (a newer local `ruff`
reports rules the pinned one does not).

1. **Red:** add
   `test_a_one_sided_soundness_gate_failure_is_recorded_beside_a_confirmed_finding` to
   `SoundnessModeTest`, written against the target 3-tuple contract:
   `side_effect=[("confirmed", "rust false proof"), ("error", "verifier timed out")]`
   (rust confirms, self-hosted's gate failed to run). Assert
   `verdict == "confirmed"`, `"rust: confirmed"` and `"self-hosted: error"` both appear
   in `detail`, and `unjudged` is not `None` and contains both
   `"self-hosted gate did not run"` and `"verifier timed out"`. Also update
   `test_soundness_pair_gate_checks_both_compilers` (`scripts/test_pair_review.py:1499`)
   in the same step: change `verdict, detail = pair_review.confirm_soundness_pair(...)`
   to `verdict, detail, unjudged = pair_review.confirm_soundness_pair(...)` and add
   `self.assertIsNone(unjudged)` (both sides there are `"refuted"`/`"confirmed"` — no
   `"error"` side). Run the suite: both fail — the new test on a 2-vs-3-value unpack
   mismatch, the edited existing test with "not enough values to unpack" — since
   `confirm_soundness_pair` still returns a 2-tuple.
   **Green:** widen `confirm_soundness_pair` (`scripts/pair_review.py:804-821`) to
   compute, before branching on verdicts:
   ```python
   unjudged = "; ".join(
       f"{side} gate did not run: {why}"
       for side, (verdict, why) in (("rust", rust_result), ("self-hosted", self_result))
       if verdict == "error"
   )
   ```
   and change every `return "<verdict>", observed` to
   `return "<verdict>", observed, unjudged or None`, matching
   `confirm_both_paths`'s uniform-3-tuple style exactly. Re-run: both tests green.

2. **Red:** add the symmetric case, swapping which side errors:
   `side_effect=[("error", "verifier timed out"), ("confirmed", "self-hosted false proof")]`.
   Assert `verdict == "confirmed"` and `unjudged` names `"rust"`, not `"self-hosted"` —
   this catches an implementation that hardcodes which side it reports on instead of
   iterating both. **Green:** satisfied by slice 1's implementation (the comprehension
   iterates both `("rust", rust_result)` and `("self-hosted", self_result)`).

3. **Red:** add a regression case for the pre-existing all-error behavior:
   `side_effect=[("error", "rust verifier crashed"), ("error", "self-hosted timed out")]`.
   Assert `verdict == "error"` (unchanged from today — `review_pair` already turns this
   into the claim's `detail` via `unjudged = detail if verdict == "error" else ...`, so
   this slice only needs to confirm the verdict itself, not inspect the third element).
   **Green:** already satisfied; this slice exists to prove slice 1 did not change the
   both-sides-failed case, since that case is the one the issue's "why deferred"
   section explicitly says must not be reprioritized.

4. **Red, mandatory — pins the issue's "Wanted" sentence end to end:** add a
   `main()`-level (or `review_pair`-level, whichever is less fixture-heavy) test
   asserting that a one-sided soundness failure beside a confirmed finding produces
   *both* halves of the issue's ask simultaneously: the finding still counts as
   confirmed (`report["confirmed"] == 1` and the finding appears in the `CONFIRMED:`
   summary data), **and** the run's exit status reflects the unjudged claim. Call
   `review_pair` with `confirm_fn` set to a fake soundness-pair gate (no network/model
   call needed — pass `confirm_fn=` directly as the existing
   `test_a_raising_gate_costs_one_claim_not_the_run` pattern does,
   `scripts/test_pair_review.py:687-720`), with `mode="soundness"`, a fake `llm_module`
   returning one finding, and the fake gate returning
   `("confirmed", "obs", "self-hosted gate did not run: timeout")`. Assert
   `result["findings"][0]["verdict"] == "confirmed"`,
   `"claim not judged" in result["errors"][0]["error"]`, and
   `reviewed_completely(result) is False`. This slice is largely redundant with the
   already-generic `review_pair` unjudged-handling tests
   (`scripts/test_pair_review.py:935-957`) for the `result["errors"]` wiring itself,
   but it is the only slice that also pins the exit-code consequence flagged in Risk
   areas below — keep it, don't treat it as optional, since it is the direct test of
   what the issue asks for rather than of `confirm_soundness_pair` in isolation.

## 4. Verification surface

Not applicable. This change touches no contracts, codegen, or the C model — it is a
pure-Python change to a CI/dev-tooling script (`scripts/pair_review.py`) that shells
out to already-built compiler binaries; it does not modify ESBMC-verified code,
`vow-verify`, `vow-codegen`, or any `.vow` source. No new `tests/run/*.vow` or
`examples/*.vow` fixtures are needed, and no ESBMC properties are affected.

## 5. Risk areas

- **None of the fixed-point/codegen risk categories apply.** This change is isolated
  to `scripts/pair_review.py` and its test file. It does not touch `compiler/`
  codegen ordering, `BTreeMap`/`HashMap` choices, `vow-clif-shim` stack-slot layout,
  the `parse → print → parse` idempotency property, or any `.vow` source the
  `cargo clippy --all -- -D warnings` gate lints.
- **Expected, not accidental: the run's exit code changes for this case.**
  `main()` checks `any(r["errors"] for r in results)` (line 1471) *before* `return 1 if
  confirmed else 0` (line 1478), so once the one-sided failure lands in
  `result["errors"]`, a run containing this pair now exits **2** ("not a verdict")
  instead of **1** ("confirmed findings"). The finding itself is not lost — it is
  still in `report["confirmed"]`, the `CONFIRMED:` summary lines, and
  `result["findings"]` with `verdict == "confirmed"` — only the overall run status
  changes, and only because the run is honestly reporting that it did not fully judge
  everything it touched. This exactly matches the pre-existing `confirm_both_paths`
  behavior (a confirmed build-path finding beside a verify-path error already exits 2
  today), and `main`'s own comment at line 1466 documents checking errors "2 before 1"
  as deliberate. This is not a regression to guard against — it is the intended
  consequence of "record the one-sided gate failure" — but the implementation stage
  should call it out explicitly in the PR body, since the issue's own "why deferred"
  section is specifically worried about exit-code effects (just the opposite one: an
  `error`-prioritized fix losing a `1`-exit entirely). Slice 4 pins this behavior with
  an explicit `reviewed_completely(result) is False` assertion.
- **Real risk: silently breaking the existing `confirm_both_paths` precedent's
  invariant.** `review_pair`'s unpacking (`scripts/pair_review.py:1073-1074`) treats
  the third tuple element as `None` as "nothing unjudged" (it checks `if unjudged:`,
  so an empty string is already handled safely today, but there is no reason to rely
  on that fallback). Mitigation: return `unjudged or None` in every branch, mirroring
  `confirm_both_paths`'s exact pattern rather than inventing a new idiom.
- **Real risk: changing the all-error case's return arity breaks nothing today, but
  only because `review_pair` ignores the third element when `verdict == "error"`.**
  If a future change to `review_pair` (outside this issue's scope) ever started
  reading the third element in the error branch, the "both sides errored" case's
  `unjudged` value (computed from both sides' error reasons) would need to make
  sense as a *claim-not-judged* message too. Slice 3's regression test pins today's
  behavior (verdict stays `"error"`) so that a future change to `review_pair` has a
  test to fail against if it accidentally changes this case's semantics.
- **Test-ordering risk inside `test_pair_review.py` is low** — the new/edited tests
  live in the existing `SoundnessModeTest` class using the same `mock.patch.object(
  pair_review, "confirm_soundness", side_effect=[...])` pattern already used at line
  1499-1521, so no new test infrastructure or fixtures are introduced.

## 6. Out of scope

- **Documenting the `confirm_fn` 3-tuple contract in `docs/equivalence/README.md`** —
  the equivalence-mode sibling (`confirm_both_paths`) shipped the same mechanism in
  #1172 undocumented; adding docs for only the soundness side now would be
  inconsistent and is unrelated to closing this issue.
- **Changing `review_pair`, `reviewed_completely`, `_ledger_outcome`, or `main`'s exit
  code logic.** The issue's own "consequences are narrower than they look" paragraph
  establishes that soundness mode never stamps the ledger
  (`MODES["soundness"].uses_ledger is False`), so nothing in that machinery needs to
  change for this fix to be correct — only `confirm_soundness_pair`'s return contract
  does.
- **Formalizing a shared helper for the "`unjudged` from per-side error reasons"
  pattern** that both `confirm_both_paths` and `confirm_soundness_pair` would then
  share. The two functions' per-side tuples differ enough (`("build", (result, why))`
  vs. `("rust", rust_result)`) that extracting a helper now would be a speculative
  abstraction for two call sites with different shapes — three similar lines beat a
  premature abstraction, per this repo's development discipline. Worth revisiting only
  if a third mode-specific pair-confirmation function appears.
- **Any change to `confirm`, `confirm_soundness` (singular), `Mode`/`MODES`, or the
  `PAIRS`/`VERIFY_PATH_PAIRS` tables.** None of these are implicated by the issue.
- **Re-running a real soundness pair-review sweep against live compiler binaries** to
  manually reproduce the original one-sided-failure report from #1172's review
  thread. The unit tests mock `confirm_soundness` directly (as the existing test suite
  already does throughout `SoundnessModeTest`), so no `target/release/vow` /
  `build/vowc` build or real ESBMC run is needed to verify this fix.
