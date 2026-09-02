# Plan: Clarify superseded current-baseline handling

## 1. Problem restated

The `current-baseline` retention rule in `reports/README.md` currently tells contributors to "delete or replace" a superseded snapshot, leaving it unclear whether they should preserve the old dated path and overwrite its contents or add the newer dated snapshot and remove the old file. Because every committed report snapshot must use a `YYYY-MM-DD-<topic>.md` filename, the policy should explicitly require deleting the previously committed dated file in the same PR rather than overwriting it in place, while preserving the existing reviewer-controlled reclassification exception. The same ambiguity is echoed verbatim in `scripts/complexity_calibrate.py`'s generated report boilerplate ("Replace this snapshot in the same PR..."), so the generator text should be brought into line with the clarified README wording in the same PR to avoid leaving two inconsistent statements of the same policy.

## 2. Files to touch

| Path | Planned change |
|---|---|
| `reports/README.md` (the `current-baseline` bullet, currently lines 18–21) | Replace only the ambiguous "delete or replace" sentence with an explicit rule: when adding a newer snapshot for the same stream, delete the previous dated file in the same PR rather than overwriting it in place, unless a reviewer explicitly reclassifies the older snapshot. |
| `scripts/complexity_calibrate.py` (retention boilerplate emitted around lines 199–208, the `if args.retention_class == "current-baseline":` block) | Reword the generated `_Retention: ...` sentence from "Replace this snapshot in the same PR..." to match the clarified wording (delete the previous dated file rather than overwriting it in place), so newly generated reports state the same policy as the README. |

No file under `crates/` or `compiler/` is involved: this is repository report-retention guidance and its generator boilerplate, not compiler behavior. No `docs/spec/*.md` update is required because the change does not affect Vow syntax, semantics, types, builtins, operators, effects, contracts, diagnostics, or CLI flags.

**Deliberately not touched:** `tests/test_complexity_calibrate.py` — its `test_reports_output_with_date_includes_retention_metadata` assertion only checks for the retention-class prefix (`` _Retention: `current-baseline` for the `complexity-calibration` stream. ``), not the "Replace"/"delete" wording that follows, so it stays green without modification (verified by `rg -n "Replace|delete" tests/test_complexity_calibrate.py`, which returns nothing). **Not touched:** the already-committed `reports/2026-06-18-complexity-calibration.md` — its retention line reflects the generator template at the time it was generated; rewriting historical committed-snapshot prose to match a later template revision is unnecessary churn on a file that exists as a frozen evidence snapshot, not as living documentation. If a contributor wants that snapshot's wording updated too, that is a follow-up, not part of this issue.

## 3. TDD slices

1. **Clarify the supersession rule in `reports/README.md`.**
   - **Test file/location:** `reports/README.md`, the dated-filename requirement at lines 7–11 and the `current-baseline` rule at lines 18–21. This prose-only policy has no executable test surface, so do not add a brittle exact-string unit test.
   - **Behavior under test (red):** A focused read/search of the existing bullet finds `delete or replace` and cannot determine whether overwriting the prior dated file is allowed.
   - **Production change (green):** In `reports/README.md`, replace only that sentence with wording equivalent to: "When adding a newer snapshot for the same stream, delete the previous dated file in the same PR rather than overwriting it in place, unless a reviewer explicitly reclassifies it."
   - **Refactor/check:** Read the revised bullet together with the global dated-filename rule and the complexity-calibration example (lines 48–50, which already says "Keep only the latest ... snapshot" and needs no change). Confirm it still enforces at most one `current-baseline` per stream, keeps the reviewer reclassification escape hatch, and does not imply that a generator performs repository cleanup automatically. Make no surrounding prose or formatting changes.

2. **Align the generator's emitted retention sentence with the clarified README wording.**
   - **Test file/location:** `tests/test_complexity_calibrate.py::ComplexityCalibrateReportsOutputTests.test_reports_output_with_date_includes_retention_metadata` (existing test, run as-is — red/green here means "still passes," since it does not pin the exact wording being changed).
   - **Behavior under test (red):** Before the change, `python3 scripts/complexity_calibrate.py --date ... --retention-class current-baseline` emits "Replace this snapshot in the same PR as the next committed complexity calibration snapshot unless a reviewer reclassifies it as `release-evidence`.", which is the same ambiguous verb the README bullet is being fixed to avoid.
   - **Production change (green):** In `scripts/complexity_calibrate.py`, edit the three `lines.append(...)` calls inside the `if args.retention_class == "current-baseline":` block so the emitted sentence says to delete the previous dated file in the same PR rather than overwriting it in place, matching slice 1's README wording. Keep the trailing `_` closing the italic markdown span and the `release-evidence` reclassification clause intact.
   - **Refactor/check:** Run `python3 -m pytest tests/test_complexity_calibrate.py -k test_reports_output_with_date_includes_retention_metadata` (or `python3 -m unittest tests.test_complexity_calibrate` if pytest is unavailable) and confirm it still passes. Manually run the `--out reports/...` example from `reports/README.md`'s "## Complexity calibration" section against a scratch temp directory (not `reports/`) and read the emitted `_Retention: ...` sentence to confirm it now matches the README bullet's wording.

## 4. Verification surface

- Contracts, codegen, the verifier C model, and ESBMC are untouched; there are no new properties for ESBMC to prove.
- No fixture under `tests/run/` and no `.vow` program under `examples/` needs to change because the policy has no compiler-runtime behavior. The one Python test that touches the generator (`tests/test_complexity_calibrate.py`) needs no edit, only a passing re-run, per slice 2.
- Run `rg -n -C 3 'current-baseline|delete or replace|overwrit' reports/README.md scripts/complexity_calibrate.py` and confirm the ambiguous phrase is gone from both files, the explicit no-overwrite rule is present, and the reclassification exception remains in both.
- Run `git diff --check` to catch whitespace errors in both touched files.
- Run `python3 -m pytest tests/test_complexity_calibrate.py` (full file, not just the one test) to confirm the generator change has no other regression.
- Run `git diff` and confirm the implementation diff is confined to the `current-baseline` bullet in `reports/README.md` and the retention-boilerplate block in `scripts/complexity_calibrate.py`. No Rust/Vow compiler build, self-hosted bootstrap, `--help`/skill regeneration, or Clippy run is warranted since no `crates/`, `compiler/`, or `docs/spec/` file changes.

## 5. Risk areas

- **Policy drift:** The edit could accidentally remove the reviewer reclassification exception or broaden the rule beyond superseding a committed `current-baseline`. Keep both the retention class and the same-stream condition explicit in both files.
- **README/generator inconsistency:** If only `reports/README.md` is edited and `scripts/complexity_calibrate.py`'s boilerplate is left saying "Replace," the two sources of truth for the same policy will disagree again immediately. Slice 2 exists specifically to prevent this — do not skip it or treat it as unrelated cleanup.
- **Test coupling:** Confirm before editing that `tests/test_complexity_calibrate.py` truly does not pin the "Replace"/"delete" wording (already verified by `rg` above); if a future edit to that test file adds such an assertion, this plan's slice 2 must update it in the same commit rather than leaving a red test.
- **Compiler invariants:** Binary fixed-point ordering, `BTreeMap` determinism, stack-slot layout, parse/print idempotency, and the `cargo clippy --all -- -D warnings` gate cannot be affected because no Rust or Vow source changes are planned.

## 6. Out of scope

- Do not delete or rewrite `reports/2026-06-18-complexity-calibration.md`'s retention sentence; no newer baseline snapshot is being introduced by this issue, and updating a frozen, already-committed snapshot's prose to match a later template revision is unnecessary churn (see Section 2).
- Do not change `tests/test_complexity_calibrate.py`, generated report boilerplate beyond the one retention sentence, or generator behavior/flags beyond that wording. The issue explicitly targets the ambiguous policy sentence and its direct echo.
- Do not alter the `release-evidence` or `temporary-review` retention classes, add new retention classes, or revisit report-stream naming and generator documentation.
- Do not edit `docs/spec/`, regenerate embedded help/skills, touch either compiler, modify `build/`, or modify the `symphony/` submodule.
- Do not bundle refactors, Markdown reformatting, wording cleanups elsewhere in either file, or unrelated repository maintenance.
- **Merge mechanics:** the orchestrator squash-merges this PR; the squash commit subject is taken verbatim from the PR title. Do not plan for a merge commit or rebase merge, and do not plan for a human to merge. The implementation stage should open the PR with a conventional-commit-style, lowercase-subject title such as `docs(reports): clarify delete-vs-overwrite policy for current-baseline snapshots`, since a `commitlint`-style hook now runs on this repo (see `4cbdff6d`/`e1ed951f`) and the title becomes the permanent commit subject on `main`.
