# Plan — issue #1136 follow-up (PR #1149 review feedback)

## 1. Problem restated

Issue #1136's landable scope (compare `diagnostics[].error_code` in `compare_error`; compare
`diagnostics[].error_code`, `.blame`, and `counterexamples[].values` in `compare_json`) is
already implemented on this branch in `d56b048f..20661d62` and is up as PR #1149. Two reviewers
(one human-triggered `claude` review, one `chatgpt-codex-connector` P2) independently flagged the
same residual hole in the code this PR touches: the hard-`VerifyFailed` branch of
`scripts/parity.py:compare_json` calls `_compare_counterexamples`, which `zip`s the two lists, but
— unlike the soft-fail branch above it and the non-`VerifyFailed` branch below it — never compares
`len(rust_counterexamples)` against `len(self_counterexamples)`. If Rust reports counterexamples
for `f` and `g` while the self-hosted compiler reports only `f`, the shared prefix agrees, the
comparator returns success, and `scripts/full_test.sh` silently loses the dropped contract failure
and its CEGIS payload — exactly the class of "the diverging field was never in the compared set"
bug this issue exists to close. This follow-up closes that asymmetry, replies to both review
threads, and pushes the same branch (no second PR).

## 2. Files to touch

| Path | Change |
|---|---|
| `scripts/parity.py` | Move the counterexample-count comparison into `_compare_counterexamples` so every caller gets it; drop the now-redundant `elif len(...) != len(...)` branch in `compare_json`. |
| `scripts/test_parity.py` | New tests: hard-failure count mismatch is an error; non-`VerifyFailed` count-mismatch message is unchanged (characterization, currently untested); a `known-cex-divergence` fixture with mismatched counts is a hard FAIL, not a SKIP. |
| `docs/equivalence/README.md` | One sentence in the Tier-1 suppressions section stating counterexample counts are compared on every `compare_json` path and that the `known-cex-divergence` directive is scoped to `values` only, so it cannot mask a count divergence. |

No `crates/`, no `compiler/`, no `docs/spec/` changes. This is harness-only Python: no Vow syntax,
semantics, builtin, operator, effect, or CLI flag changes, so the "both compilers in the same
session" rule and the spec-update rule do not apply. `scripts/cli_compat_test.sh` carries an
inline copy of the *old* comparator with the same shape — deliberately out of scope (§6).

## 3. TDD slices

Each slice is red → green → commit. Run `python3 scripts/test_parity.py` after each.

**Slice 1 — lock the existing non-`VerifyFailed` count message (characterization, red-free).**
`scripts/test_parity.py::CompareJsonCounterexampleValuesTest` (or a new
`CompareJsonCounterexampleCountTest` alongside it). Two non-`VerifyFailed` documents with 2 vs 1
counterexamples must produce exactly `["counterexamples count: 2 vs 1"]`. This message has no test
today; slice 2 relocates the code that emits it, so pin the wording first.

**Slice 2 — hard-failure count mismatch must fail (the review finding).**
Red: two `VerifyFailed` documents, Rust with counterexamples for `f` and `g`, self-hosted with only
`f`, all shared-prefix fields and `values` identical. Assert
`["counterexamples count: 2 vs 1"]`; today it returns `[]`.
Green: in `scripts/parity.py`, seed `_compare_counterexamples` with
`errors = _mismatch("counterexamples count", len(rust_counterexamples), len(self_counterexamples))`
instead of `errors = []`, and delete the `elif len(rust_counterexamples) != len(self_counterexamples)`
branch from `compare_json` (lines 129–133) so the `else` falls straight through to the comparator.
`_mismatch` reproduces the current string byte-for-byte, so slice 1 stays green. Both branches then
report the count *and* the per-index divergences on the shared prefix, which is strictly more
information than the old early-out.
Guards that must stay green without edits:
- `test_hard_verify_failures_require_counterexamples` — both lists empty, `0 == 0`, no new error;
  the two "has no counterexamples for VerifyFailed" messages are unaffected.
- The soft-`VerifyFailed` branch does not call `_compare_counterexamples`; its
  "soft VerifyFailed has N counterexamples" messages are untouched.

**Slice 3 — the suppression cannot swallow a count divergence.**
`scripts/test_parity.py::ParityCliCharacterizationTest`. A fixture carrying
`// TEST: known-cex-divergence 1139 "..."` whose two documents differ in counterexample *count*
must exit 1 with `FAIL: ...counterexamples count...`, not exit 0 with `SKIP:`. This is the
regression guard for the fix: `_known_cex_verdict`'s `covered` predicate matches only
`counterexample[i].values:` errors, so a count error breaks the `all(covered(...))` test and the
directive correctly stops applying. Expected green on the slice-2 code — it is a lock, not a
driver, and it is the "deliberately-divergent fixture proves the comparison fails when it should"
acceptance criterion applied to this new field.

**Slice 4 — docs.** `docs/equivalence/README.md`, one sentence as described in §2. No code.

**Slice 5 — gates and review reply.** Run, as separate commands, never `&&`-chained:
1. `python3 scripts/test_parity.py`
2. `python3 scripts/test_equivalence.py` (shares the ledger/observable vocabulary)
3. `ruff check scripts/parity.py scripts/test_parity.py` and `ruff format --check` on the same
   (the pre-commit hook runs both; CI would too)
4. `scripts/bootstrap.sh --skip-cargo` — must reach the SHA-256 fixed point
5. `scripts/full_test.sh` — must be green
Then commit, push the same branch, and reply to both threads with `gh api
repos/vow-lang/vow/pulls/comments/3893462192/replies -f body=...` and
`.../3893476789/replies -f body=...` (the `gh` CLI, per the run contract — not the GitHub MCP
connector, which is also failing to connect in this environment). Do not open a second PR.

Suggested commit: `fix(parity): compare counterexample counts on every comparator path`.

## 4. Verification surface

None. No contracts, no codegen, no C model, no IR, no ESBMC verification conditions. No fixture
under `tests/run/`, `tests/verify-fail/`, or `examples/` needs to grow: the divergence this slice
detects is between two compilers on *existing* fixtures, and slices 1–3 exercise it at the unit and
CLI level with synthetic documents, which is both faster and able to construct the mismatched-count
shape that no current fixture produces.

Empirical baseline for "will the suite go red" (measured this run, both binaries at branch HEAD, on
all 21 `tests/verify-fail/*.vow` and all 35 `tests/verify/*.vow`): **every fixture agrees on status
and on counterexample count — every hard failure is `VerifyFailed/1` on both compilers, every
passing fixture is `Verified/0`.** Zero divergences. The new check should therefore be a no-op on
today's corpus; see §5 for what to do if a path this probe did not cover disagrees.

## 5. Risk areas

- **Binary fixed point / `parse → print → parse` / clippy.** Untouched. No `compiler/` codegen
  ordering, no `BTreeMap` vs `HashMap`, no `vow-clif-shim` stack slots, no Rust code at all, so
  `cargo clippy --all -- -D warnings` cannot regress. `scripts/bootstrap.sh --skip-cargo` is still
  in the gate list as an acceptance criterion, not because this change can plausibly break it.
- **`full_test.sh` goes red on a path the probe missed.** `compare_json` is also reached from
  `tests/multi/`, `tests/run/` build+verify, and the benchmark corpus. Those runs verify one
  function at a time and ESBMC stops at the first failing property, so the `VerifyFailed/1` shape
  should hold there too — but if a genuine count divergence surfaces, it is a **finding, not a
  harness bug** (the issue is explicit about this). Response, in order: file it with `gh issue
  create`, comment on PR #1149 with the fixture and both counts, and only then, if the fix is not
  small and self-contained, scope a suppression for that one fixture that *names the new issue* —
  never widen `known-cex-divergence`'s `covered` predicate to match count errors generally, since
  that re-opens precisely the hole this slice closes.
- **Message-shape coupling.** `_known_cex_verdict` and `_ledger_verdict` match error strings by
  prefix. The new count error deliberately does not match either predicate. Slice 3 pins that; do
  not "fix" a resulting FAIL by adding the count error to a `covered` predicate.
- **Extra errors where there was previously one.** On the non-`VerifyFailed` path, a count mismatch
  now also reports per-index divergences on the shared prefix instead of early-returning. This is a
  strictly louder message on an already-failing case; slice 1's characterization test is what keeps
  the *count* line itself byte-identical for anything grepping the log.

## 6. Out of scope

- `scripts/cli_compat_test.sh`'s inline comparator (`scripts/cli_compat_test.sh:36-73`), which is a
  stale copy of the pre-extraction `compare_json`. Collapsing it onto `scripts/parity.py` is a real
  cleanup and a real de-duplication, but it is a refactor of a second harness and does not belong in
  a review-follow-up commit. Worth a follow-up issue; note it in the PR reply.
- Items 3 and 4 of the issue (`span.offset`/`span.length`, `counterexamples[].violation`). Ordered
  behind #1135 and #1113 respectively — that ordering is load-bearing, unchanged by this run.
- Dropping the blanket "no diagnostics compared under `VerifyFailed`" suppression. Blocked on #1138
  and already documented in `docs/equivalence/README.md`.
- `diagnostics[].message` / `.hints` / `.span` comparison, and `counterexamples[].source`. Named in
  the issue table but not in its "landable now" set; each needs its own triage budget.
- Any reformat of `scripts/parity.py` or `scripts/test_parity.py` beyond the lines the slices touch.
