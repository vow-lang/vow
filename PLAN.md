# Plan: #1186 — distinguish an unlocalized divergence from a pair-confirmed one

## 1. Problem restated

`pair_review.py`'s `confirm()` judges a candidate program end-to-end (lexer →
parser → checker → lower → c_emitter) and returns `confirmed` the moment the two
compilers disagree on *any* observable, with no mechanism able to say which
compiler stage the disagreement actually lives in (every existing observable is
whole-pipeline CLI behaviour, and IR-text comparison is not established as a
reliable stage-level signal — see the issue body). `review_pair()` nonetheless
attributes every confirmed finding to whichever pair happened to be under
review when the model proposed the input, `_print_summary()` renders that as
bare `[lexer]`, and `write_ledger()`/`_ledger_outcome()` stamp that pair's
ledger row `outcome: "confirmed"` — three places where a fact that is true
("the pipeline disagrees on this program") gets silently relabelled as a
different, false fact ("this pair's stage is where it disagrees"). Docs already
say CONFIRMED is a program-level claim, not a pair-level one (`b2d180a5`); this
issue is the mechanical follow-through — carry the "not yet localized" state as
an explicit field/value instead of leaving it implicit in prose that the tooling
itself doesn't honor.

## 2. Files to touch

This is Python tooling under `scripts/`, not compiler surface area — **no**
`crates/`, **no** `compiler/`, **no** `docs/spec/*.md` changes apply (verified:
`docs/spec/` has zero references to `pair_review`/`ledger.schema`; this script
is a maintenance/audit tool, not part of the `vowc` CLI or language semantics).

- `scripts/pair_review.py` — production changes:
  - `review_pair()` (~pair_review.py:930-1087): stamp `finding["attribution"] =
    "unlocalized"` alongside the existing `finding["verdict"]` /
    `finding["verdict_detail"]` assignment (~line 1084), for every finding in
    both `equivalence` and `soundness` modes (shared code path — the field
    means the same thing in both: "not yet tied to any specific stage").
  - `_ledger_outcome()` (pair_review.py:1110-1116): mechanical writeback must
    never emit `"confirmed"` — that value becomes human-only, applied during
    triage after the "Attribute" step. Rename the confirmed-verdict branch's
    return value to `"unlocalized"`.
  - `_print_summary()` (pair_review.py:1186-1260, specifically the loop at
    1257-1259 `for pair, finding in confirmed:`): render
    `f"[{finding.get('attribution', 'unlocalized')}; proposed during {pair}]"`
    instead of bare `f"[{pair}]"`.
  - No change needed to `confirm()`, `confirm_both_paths()`, `main()`'s
    `confirmed`/`hypotheses`/`refuted` tallies, or the exit-code logic — those
    all operate on the *verdict* (is the disagreement real?), which is an
    orthogonal question to attribution (where does it live?) and is out of
    scope for this issue.
- `docs/equivalence/ledger.schema.json` — schema change (this is the ledger
  schema change the issue calls out as wanting its own PR; it's still the
  minimal correct scope for this issue, not a reason to split further, since
  the code change and the schema it validates against must land atomically or
  the harness writes ledger rows the schema rejects):
  - `properties.pairs.additionalProperties.properties.outcome.enum`
    (ledger.schema.json:35): add `"unlocalized"`.
  - Update the `outcome` field's `description` (ledger.schema.json:36) to
    state that `"unlocalized"` is what the harness stamps mechanically for any
    runner-confirmed divergence, and `"confirmed"` is now a human-only value
    applied once a triager has attributed the divergence to this pair's stage
    specifically (via the runbook's "Attribute" step). Also state explicitly
    that a human-set `"confirmed"` is not sticky across content changes:
    `write_ledger()` overwrites `outcome` unconditionally on every complete
    re-review of that pair (`entry.update({...})`, pair_review.py:1150-1155),
    so once the pair's `content_hash` changes and it gets reviewed again, the
    row reverts to whatever `_ledger_outcome()` computes fresh
    (`unlocalized`/`hypotheses`/`clean`) — `confirmed_issues` survives (it is
    not part of that `update()` dict) but `outcome` does not. This is correct
    behavior (new source content is new evidence, not a continuation of the
    old triage conclusion) but must be documented so a triager doesn't read a
    reverted row as a bug that silently erased their attribution work.
- `docs/equivalence/README.md` — prose follow-through so the doc doesn't
  contradict the new mechanics:
  - The CONFIRMED paragraph (README.md:132-140, landed in `b2d180a5`) currently
    ends "Locating it is a triage step for a human, and `confirmed_issues` is
    where that conclusion lands." Update to also name the new `unlocalized`
    ledger outcome and the `[unlocalized; proposed during <pair>]` summary
    label as the mechanical expression of that same fact, and clarify that a
    human promotes the ledger row's `outcome` from `unlocalized` to
    `confirmed` only after completing attribution.
- `.claude/commands/equivalence-review.md` — runbook step 5 ("Complete the
  ledger metadata", lines 138-141) currently says "Pair-review hashes, dates
  and outcomes are written separately by the pair-review harness... what you
  add there is confirmed issue numbers." This sentence becomes wrong the
  moment the harness stops being able to write `outcome: "confirmed"` on its
  own — update it to say the harness now writes `unlocalized` (never
  `confirmed`) mechanically, and that a human flips the row to `confirmed`
  during step 3's "Attribute" sub-step once they've verified the divergence
  actually belongs to this pair.
- `scripts/test_pair_review.py` — test changes, see TDD slices below.
- `docs/equivalence/ledger.json` — no change needed. Checked during planning:
  all five current `pairs.*.outcome` rows are `"clean"` (`grep -n '"outcome"'
  docs/equivalence/ledger.json`), so there is no existing `"confirmed"` row
  that would need one-time human reclassification. No action item to carry
  into the PR description.

## 3. TDD slices

Each slice is red → green → (refactor if warranted). All in `scripts/`; run
with `python -m pytest scripts/test_pair_review.py -k <name>` (check repo's
actual test invocation — `full_test.sh` or a `pytest`/`uv run pytest`
convention — before running; do not assume without checking).

1. **Finding carries an explicit `attribution` field.**
   - Test: extend `ReviewReportTest.test_findings_carry_their_chunk_index`
     (test_pair_review.py:885) or add a sibling
     `test_findings_carry_an_unlocalized_attribution` asserting
     `result["findings"][0]["attribution"] == "unlocalized"` for a
     `confirm_fn` that returns `("confirmed", "...")` (currently that test uses
     `("refuted", "agreed")` — add a second case, or a new small test, so the
     field is asserted independent of verdict, since attribution belongs to
     the finding regardless of the confirm/refute/inconclusive outcome).
   - Production: in `review_pair()`, at the same point `finding["verdict"]` /
     `finding["verdict_detail"]` are set (pair_review.py:1084-1085), add
     `finding["attribution"] = "unlocalized"`.

2. **`_ledger_outcome` returns `unlocalized`, not `confirmed`, for a
   runner-confirmed divergence.**
   - Test: update `LedgerWritebackTest.test_outcome_reflects_strongest_verdict`
     (test_pair_review.py:1020-1030) — change the first case's expected value
     from `([{"verdict": "confirmed"}], "confirmed")` to
     `([{"verdict": "confirmed"}], "unlocalized")`. This is a deliberate
     behavior change to an existing assertion, not a regression — call this
     out explicitly in the commit/PR body so a reviewer doesn't read the diff
     as accidentally weakening a test.
   - Production: in `_ledger_outcome()` (pair_review.py:1110-1116), change
     `if "confirmed" in verdicts: return "confirmed"` to
     `return "unlocalized"`.
   - Also update `test_writeback_stamps_hash_date_and_clean_outcome` only if it
     exercises a confirmed finding (it currently uses `self.result()` with no
     findings, which is `"clean"` — unaffected, verify but likely no change
     needed).

3. **`write_ledger` never writes `outcome: "confirmed"` from the mechanical
   path; `unlocalized` passes schema validation.**
   - Test: add
     `test_writeback_never_stamps_confirmed_directly` to
     `LedgerWritebackTest` — write a result with
     `findings=[{"verdict": "confirmed"}]`, then assert
     `written["pairs"]["lexer"]["outcome"] == "unlocalized"` (not
     `"confirmed"`) and that `_validate_pair_entry`/schema-key-set validation
     (mirroring `test_written_entry_matches_schema_key_set`, line 1081) still
     passes — this exercises the schema-enum addition end-to-end, since
     `_validate_pair_entry` currently only checks key sets, not enum values,
     so this test is the only place that would catch a schema/code mismatch
     (e.g. if the schema enum edit were forgotten, a stricter validator would
     reject `unlocalized`; today's validator wouldn't, which is itself worth
     noting — see Risk areas).
   - Production: none beyond slice 2 (the schema file edit is the production
     change here — add `"unlocalized"` to the enum in
     `ledger.schema.json:35`).

4. **`_print_summary` renders `[unlocalized; proposed during lexer]`, not
   `[lexer]`.**
   - Test: add a test (new, since none currently exercises `_print_summary`'s
     confirmed-findings rendering — confirmed via research: no `[lexer]` or
     `_print_summary` string assertions exist in the test file today). Note:
     `run_dry` cannot exercise this — dry-run never calls the model, so
     `findings` is always `[]` and the `confirmed` tuple list `main()` builds
     is always empty, meaning `_print_summary`'s confirmed-loop body never
     runs under it. Two viable approaches: (a) call `_print_summary` directly
     with a hand-built `report`/`confirmed` argument pair under
     `contextlib.redirect_stdout` (or whatever capture helper the file already
     uses elsewhere for non-dry-run output — check before inventing a new
     one), or (b) drive `main()` with `fake_llm` and a `confirm_fn` stub
     returning `("confirmed", "...")`, matching the pattern
     `test_findings_carry_their_chunk_index` already uses to get real findings
     out of `review_pair`. Assert the captured stdout contains
     `"[unlocalized; proposed during lexer]"` and does **not** contain the bare
     `"[lexer] "` form used previously for a confirmed finding line.
   - Production: in `_print_summary()` (pair_review.py:1257-1259), change
     `print(f"    [{pair}] {finding.get('claim', '?')}")` to
     `print(f"    [{finding.get('attribution', 'unlocalized')}; proposed ` \
     `f"during {pair}] {finding.get('claim', '?')}")`.

5. **Schema doc round-trip: a `ledger.json` pairs-row with
   `outcome: "unlocalized"` is accepted by whatever validates the corpus half
   today.** `test_equivalence.py`'s `assert_valid_ledger_document` (lines
   63-99) only validates the **corpus** half, not `pairs`, so there is no
   existing enforcement point for the `pairs.*.outcome` enum today besides
   `_validate_pair_entry`'s key-set check. Given that gap, this slice is:
   confirm (as a test assertion, not just manual inspection) that
   `ledger.schema.json` itself parses as valid JSON and that its `outcome.enum`
   list contains exactly `["clean", "hypotheses", "unlocalized", "confirmed"]`
   post-edit — a small, direct assertion in `test_pair_review.py` (e.g.
   `test_ledger_schema_declares_unlocalized_outcome`) reading the schema file
   and checking the enum list, since nothing else in the suite pins this
   enum's contents today (confirmed via research: `_pair_schema()`/
   `_validate_pair_entry()` only ever inspect `required`/`properties` keys,
   never `enum` values).
   - Production: none beyond the schema file edit itself; this slice exists to
     give the doc/schema edit a regression test, since currently nothing would
     fail if a future edit silently dropped an enum value.

Order: 1 → 2 → 3 → 4 → 5. Slices 1 and 4 are independent of 2/3 and could be
done in either order, but doing 1 first means slice 4's test has a real
`attribution` value to assert on rather than relying on `.get(..., "unlocalized")`
fallback default masking a missing assignment.

## 4. Verification surface

No contracts, codegen, or C model are touched — this is pure Python tooling
around the equivalence-review harness, not the Vow compiler or its verification
pipeline. ESBMC is not involved. No `tests/run/` or `examples/` fixtures need
to grow; the known repro (`tests/run/euclid_gcd_swap_loop.vow`) is cited in the
issue only as *motivation* for why attribution needs to be explicit, not as
something this PR needs to re-diagnose, re-fix, or add a new fixture for — the
lowerer divergence itself is a separate, already-known issue thread, and
conflating "fix the tooling's labeling" with "fix the underlying divergence"
would bundle two unrelated changes into one PR (see Out of scope).

## 5. Risk areas

- **Binary fixed point / `vow-clif-shim` / codegen ordering / `BTreeMap` vs
  `HashMap` / stack-slot layout / `parse → print → parse` idempotency**: none
  apply. This change touches zero Rust crates and zero `compiler/*.vow`
  modules — it is entirely contained to `scripts/pair_review.py`,
  `scripts/test_pair_review.py`, and two doc files. No bootstrap, no
  self-hosted rebuild, no cargo clippy surface changes (Python, not Rust).
- **Backward-compat break in an existing test assertion**: slice 2 deliberately
  flips `test_outcome_reflects_strongest_verdict`'s expected value for the
  `confirmed`-verdict case from `"confirmed"` to `"unlocalized"`. This is the
  intended behavior change, but it's the one place in the diff that could look
  like an accidental test weakening on casual review — call it out explicitly
  in the PR description, since this is the exact kind of change the split
  #1172 review thread (`PRRT_kwDORYe2xs6eBtNx`) was already sensitive to, so an
  automated reviewer doesn't re-flag it as "test relaxed without justification."
- **Schema enum vs. validator gap**: `_validate_pair_entry()` only checks
  required/allowed *key* sets, never enum *values* — so a typo in the new
  `"unlocalized"` enum string (schema says `"unlocalized"`, code writes
  `"unlocalised"`, say) would not be caught by any existing runtime check, only
  by the new schema-content test in slice 5 and by a human reading
  `ledger.json` after a real run. Keep the string identical byte-for-byte
  between `_ledger_outcome()`'s return value and the schema enum entry; slice 5
  is the guardrail, but it only catches schema drift, not a harness/schema
  string mismatch introduced after these tests are written and never touched
  again — low risk given the narrow scope, but worth a second look at review
  time.
- **`confirmed_issues` and `outcome` semantics diverge further**: after this
  change, a pair can have `outcome: "unlocalized"` *and* a non-empty
  `confirmed_issues` array (if a human filed an issue for the divergence but
  hasn't yet completed attribution, or attributed it to a *different* pair's
  stage and filed the issue there instead). This is not a new inconsistency —
  today's `outcome`/`confirmed_issues` pairing already has no enforced
  relationship — but flag it in the README update so a future reader doesn't
  assume `confirmed_issues` non-empty implies `outcome: "confirmed"`.
- **Report `schema_version`**: `pair_review.py`'s `results.json` report carries
  a literal `"schema_version": 2` (pair_review.py:1436) with no consumer or
  test asserting on it (verified: no `grep` hits for report-level
  `schema_version` assertions, unlike the ledger's own `schema_version: 1`
  which `test_equivalence.py:72` does assert on). Adding the `attribution`
  field to findings is an additive, backward-compatible change to that report
  shape; **do not bump `schema_version`** for this — there's no reader that
  branches on it, and bumping it without a documented reason would be scope
  creep against "many small changes beat one large change."

## 6. Out of scope

- **Re-diagnosing or fixing the `tests/run/euclid_gcd_swap_loop.vow` lowerer
  divergence itself.** That's the motivating example, not this issue's
  deliverable. This PR fixes how the tooling *labels* an unlocalized
  divergence; it does not localize that specific one.
- **Implementing either of the two remedies the original #1172 comment
  proposed** ("suppress already-tracked corpus divergences" / "causally tie
  the divergence to the reviewed pair") — the issue body explains both are
  inert or unavailable with current observables. Do not attempt a
  stage-attribution heuristic (e.g. IR-diffing, partial-pipeline execution) as
  part of this change; that would be new verification-adjacent machinery far
  outside a "chore" and contradicts the issue's own conclusion that no such
  signal exists today.
- **One-time reclassification of existing `ledger.json` rows** that may
  already carry a mechanically-stamped `outcome: "confirmed"` from before this
  change. Check whether any exist (informational, during implementation), but
  leave any such row alone — reclassifying a past harness decision is a human
  triage action with its own judgment call per row, not a mechanical
  side-effect of shipping this PR. If any are found, note them in the PR
  description for a human to triage separately, per the "make best-effort
  decisions and document them" operating contract.
- **Validating the whole `ledger.json` against `ledger.schema.json`
  end-to-end** (including the `allOf` conditional rules) as a new CI gate or
  test-suite feature. The issue doesn't ask for this, `schema_check.py`
  already exists for a different schema family, and building a general
  validator is a much larger, separable improvement to the ledger's schema
  enforcement story — not bundled into this attribution fix.
- **Renaming or restructuring `_ledger_outcome`'s function signature**, the
  `finding` dict's other keys (`claim`, `area`, `program`, etc.), or any
  unrelated cleanup in `pair_review.py` noticed along the way. Surgical diff
  only: one new field, one renamed return value, one summary line format, one
  schema enum entry, plus the doc prose that describes them.
