# Plan: issue #1180 — make the ledger read-modify-write transactional

## 1. Problem restated

`write_ledger` in `scripts/pair_review.py` (line 1143) already re-reads
`docs/equivalence/ledger.json` from disk right before merging, which closes
the minutes-long window between the start of a model-driven review and its
write (covered by the existing `test_concurrent_triage_edit_is_not_clobbered`).
But the merge-then-`os.replace()` sequence after that re-read is still not a
compare-and-swap: an edit landing in the microseconds between the re-read and
the rename — concretely, a human hand-editing the git-tracked
`ledger.json` in a text editor during this monthly, single-operator command —
is silently discarded, because the atomic rename only guarantees the file is
never seen half-written, not that no update is lost. The fix is to detect that
narrowed race at write time and retry the merge against the fresh content,
rather than to add a lock that the actual conflicting writer (a text editor)
would never honor.

## 2. Files to touch

This is a Python tooling change with no language-semantics, contract, or
codegen surface. Nothing in the Rust crates, `compiler/`, or `docs/spec/*.md`
is touched, so the "change both compilers" rule does not apply.

- `scripts/pair_review.py` — `write_ledger` (line 1143) gains a
  compare-and-retry loop; the existing merge body (lines ~1150–1174) is
  extracted into a small helper so the retry loop can call it per attempt
  without duplicating the merge logic.
- `scripts/test_pair_review.py` — `LedgerWritebackTest` (line 1007) gains the
  new race-injection tests described below; `test_concurrent_triage_edit_is_not_clobbered`
  (line 1087) must keep passing unmodified, since it already covers the
  pre-existing re-read fix from #1172.
- `docs/equivalence/README.md:209` — the sentence "Pair rows are written
  atomically by `pair_review.py --update-ledger --date <YYYY-MM-DD>`..." is
  imprecise once this lands: "atomically" today refers only to the rename.
  Reword to say the writeback detects a concurrent edit made after its
  internal re-read and retries the merge against the fresh content, and name
  the residual (unclosed) window honestly: the gap between the final
  pre-replace comparison and the rename itself.

No `ledger.schema.json` change — the on-disk shape of a pair entry is
unaffected; only the merge's robustness changes.

## 3. TDD slices

All slices are in `scripts/test_pair_review.py::LedgerWritebackTest` /
`scripts/pair_review.py::write_ledger`. Each slice is a vertical red-green
step against the real temp-file ledger fixture already set up in
`LedgerWritebackTest.setUp`.

1. **Baseline/no-op guard.** Confirm (no new test needed — covered by
   existing `test_writeback_stamps_hash_date_and_clean_outcome` and
   `test_partially_reviewed_pair_is_not_stamped`) that the refactor that pulls
   the merge body into a helper changes no observable behavior before any
   retry logic is added. Run the existing suite after the pure
   extract-helper refactor, before writing new tests, to pin this down as a
   separate, reviewable commit boundary from the retry logic itself.

2. **Red: conflict-then-success is not yet detected.**
   Add `test_concurrent_edit_during_writeback_is_retried_not_lost` to
   `LedgerWritebackTest`. Monkeypatch `_validate_pair_entry` (called once per
   merge, right after the fresh read, per the advisor's seam suggestion —
   it is already on the hot path and has no production reason to be
   observable, so patching it to inject a write is non-invasive) with a
   `side_effect` that, on its **first** call only, writes a distinct
   concurrent edit to `self.ledger_path` (e.g. sets
   `live["pairs"]["parser"]["confirmed_issues"] = [4242]`, matching the
   existing fixture's edit) before returning normally, and on the second call
   is a no-op. Call `pair_review.write_ledger(stale_ledger_dict,
   [self.result()], "2026-09-01", self.ledger_path)`. Assert:
   - the final file has `pairs.lexer.last_reviewed == "2026-09-01"` (the
     write succeeded),
   - the final file has `pairs.parser.confirmed_issues == [4242]` (the
     concurrent edit survived — this is the regression this issue is about),
   - `_validate_pair_entry` was called more than once (the merge actually
     retried, not merely happened to match).
   This test fails against current `write_ledger`, which has no retry loop
   and clobbers the injected edit exactly like
   `test_concurrent_triage_edit_is_not_clobbered` demonstrates for the
   pre-call window.

3. **Green: implement the compare-and-retry loop.** In `write_ledger`:
   - Read the ledger file **once per attempt** as raw bytes via a helper
     (`_read_ledger_bytes(path) -> bytes | None`, returning `None` on
     `FileNotFoundError`) and hash/compare those exact bytes — not a second
     `read_text()` call — so there is no gap between "read for parsing" and
     "read for comparison" within one attempt.
   - Parse that same byte buffer for the merge (`json.loads(data)` if present,
     else the `ledger` parameter, matching current fallback behavior).
   - Run the existing merge body against the parsed dict, producing the
     merged ledger and the `updated` list. If `updated` is empty, return
     immediately — no write, no comparison needed (gate behavior is
     unchanged from today).
   - Serialize and write the temp file exactly as today (`tempfile.mkstemp`,
     `json.dumps(..., indent=2) + "\n"`).
   - **Before** `os.replace`, re-read the path's current raw bytes and
     compare to the bytes captured at the top of this attempt. If they
     match, `os.replace` and return `updated`. If they don't match, discard
     the temp file and loop to the next attempt, re-reading and re-merging
     against the now-current content.
   - Bound attempts with a module-level constant (e.g.
     `_LEDGER_WRITE_MAX_ATTEMPTS = 5`); no new CLI flag. Exhausting attempts
     raises `OSError` (reusing an existing type `main()` already catches at
     line ~1436, rather than introducing a new exception type that would
     escape that handler and discard `results.json` after real model spend).
   Re-run slice 2's test: green.

4. **Red: exhaustion is reported, not silently accepted or hung.**
   Add `test_concurrent_edit_every_attempt_exhausts_retries_cleanly`.
   Monkeypatch the same seam so it writes a **different** payload to
   `self.ledger_path` on every call (e.g. increment a counter into
   `confirmed_issues`), so no attempt's pre-replace comparison can ever match
   — this is the detail called out explicitly: if the injected edit were
   identical each time, the second attempt would spuriously match and the
   test would pass for the wrong reason. Assert:
   - `write_ledger` raises `OSError` (the exact type `main()`'s
     `except (OSError, ValueError)` already catches),
   - the ledger file on disk holds the last injected edit, not a half-written
     merge,
   - no stray `.ledger.json.*.tmp` file is left in the ledger's directory
     (glob `path.parent.glob(f".{path.name}.*.tmp")` and assert it's empty —
     the `finally: temp_path.unlink(missing_ok=True)` must fire on every
     discarded attempt, not just the last).
   Fails against a naive implementation that leaks the temp file on a
   mismatch-and-retry path, or that raises an uncaught type.

5. **Green: finally-unlink on every attempt, bounded raise.** Ensure the
   per-attempt temp-file cleanup (`try/finally: temp_path.unlink(missing_ok=True)`)
   runs on the mismatch-and-retry path, not only on success/exception from the
   write itself, and that the final `OSError` is raised only after the
   attempt budget is exhausted. Re-run slice 4's test: green.

6. **Integration: `main()`'s existing catch still fires end to end.**
   No new test needed if slice 3 reuses `OSError` — confirm
   `test_a_ledger_failure_still_writes_the_run_results` (line 1170, which
   already mocks `write_ledger` to raise `ValueError`) still passes untouched,
   and manually trace that a real `OSError` from exhausted retries would hit
   the same `except (OSError, ValueError)` branch at line ~1436 and preserve
   `results.json` exactly as that test checks. This is verification, not a
   new test, since the exception tuple already covers the type chosen in
   slice 3.

7. **Regression gate.** Run the *entire* existing `LedgerWritebackTest` class
   unmodified (`test_writeback_stamps_hash_date_and_clean_outcome`,
   `test_outcome_reflects_strongest_verdict`,
   `test_partially_reviewed_pair_is_not_stamped`,
   `test_unmatched_coverage_alone_never_blocks_a_stamp`,
   `test_concurrent_triage_edit_is_not_clobbered`,
   `test_corpus_and_untouched_pairs_survive`,
   `test_written_entry_matches_schema_key_set`,
   `test_writeback_never_stamps_confirmed_directly`,
   `test_writeback_is_off_by_default`,
   `test_a_ledger_failure_still_writes_the_run_results`) to confirm none of
   them needed a behavior change — only the two new tests from slices 2 and 4
   are additions.

## 4. Verification surface

Not applicable. This change touches no `.vow` contracts, no IR/codegen, no C
model, and no ESBMC-verified surface — it is a Python script's file-write
discipline. No `tests/run/*.vow` or `examples/*.vow` fixtures need to grow.
The only "verification" here is the Python unit-test suite in slices 2–7.

## 5. Risk areas

- **Not a risk to the binary fixed point.** No `compiler/`, `vow-clif-shim`,
  or Cranelift-backend code is touched; `BTreeMap`/`HashMap` ordering and
  stack-slot layout are unaffected.
- **Not a risk to `parse → print → parse` idempotency.** No `vow-syntax`
  grammar or printer code is touched.
- **`cargo clippy --all -- -D warnings`** is unaffected — this PR touches no
  Rust source. (Confirmed no Rust crate is in the file list above.)
- **Python lint gate.** `.pre-commit-config.yaml` pins `ruff-pre-commit` at
  `v0.15.14` (`ruff-check --fix` + `ruff-format`); run via
  `pre-commit run ruff-check --files scripts/pair_review.py scripts/test_pair_review.py`
  and `pre-commit run ruff-format --files <same>` (or `uvx ruff@0.15.14 check`
  / `uvx ruff@0.15.14 format --check` directly) before committing — a newer
  local ruff can report rules CI/pre-commit never enforces.
- **codecov/patch gate does not apply.** `codecov.yml` lists `scripts` under
  `ignore:` and measures only Rust crates via `cargo llvm-cov`, so the 95%
  patch gate is not a concern for this file — no lcov entries are generated
  for `scripts/pair_review.py` regardless of coverage.
- **CI wiring.** `.github/workflows/ci.yml:81` already runs
  `python3 scripts/test_pair_review.py` as "Adversarial pair-review unit
  tests" — no new CI step is needed, the new tests ride the existing one.
- **Real risk: a retry loop that doesn't actually bound, or that swallows
  the eventual failure into a silent no-op.** Mitigated by slice 4's
  exhaustion test, which requires the raised exception to propagate with a
  type `main()` already handles, and requires the temp-file cleanup to leave
  no stray file.
- **Real risk: reusing `_validate_pair_entry` as the test seam makes a test
  brittle against unrelated changes to that function.** Acceptable per the
  advisor's note — it already runs once per merge attempt with no production
  reason to be otherwise observable, and the alternative (adding a
  production-only hook solely for tests) would be a worse shape of change for
  a chore this narrow.
- **Honesty of the guarantee.** A compare-then-replace on POSIX still leaves
  a sliver between the final comparison read and `os.replace` itself — this
  PR narrows the window from "the whole merge" to "one stat+read", it does
  not claim to eliminate it. The `write_ledger` docstring and the
  `docs/equivalence/README.md` update must say exactly that, not claim full
  transactionality.

## 6. Out of scope

- **No advisory `flock`.** Rejected per the advisor's analysis: the only
  real conflicting writer named in the issue is a human editing
  `ledger.json` by hand in a text editor, which does not take `flock`, so a
  lock would close nothing the issue is actually about. It would also require
  a sidecar lock file (since `os.replace` swaps to a new inode, a lock held
  on the old path stops excluding anyone after the first write) — net new
  surface for a second automated writer (`scripts/equivalence.py`) that does
  not exist today and is explicitly out of scope per the issue body.
- **No CLI flag for retry count or backoff.** The attempt bound is a fixed
  internal constant; this is a correctness fix, not a new tunable surface.
- **No change to `ledger.schema.json` or the shape of a pair entry.**
  Unrelated to the lost-update bug.
- **No change to `scripts/equivalence.py`.** It only reads the ledger today
  (`pair_review.py` invokes it with `--no-ledger`); untouched, per the issue
  body's own framing of why this was deferred rather than folded into #1172.
- **No broader refactor of `write_ledger`'s surrounding code** (e.g.
  `_ledger_outcome`, `_validate_pair_entry`, `reviewed_completely`) beyond
  extracting the merge body into a helper the retry loop can call repeatedly.
  Any unrelated cleanup there is deliberately deferred to keep this PR
  surgical and bisectable, per repo convention.
