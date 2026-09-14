# Plan: issue #1252 — exercise Linux/ARM64 ESBMC pin in bootstrap.yml before release

## 1. Problem restated

`#1213` pinned a checksum-verified ESBMC archive for `Linux/ARM64` in
`.github/actions/install-esbmc` and flipped `release.yml`'s `linux/aarch64` matrix leg to
`verify: true`, but `bootstrap.yml` — which runs on every push to `main` and nightly — only
exercises that pin's siblings (`Linux/X64` via the `bootstrap` job, `macOS/ARM64` via
`bootstrap-macos`). Because `release.yml`'s `publish` job requires every `build-package` matrix
leg to succeed, a broken Linux/ARM64 ESBMC install would otherwise surface for the first time
during the weekly release window and block the release across *all* platforms. The fix is to add
a third `bootstrap.yml` job that installs ESBMC on an ARM64 Linux runner and runs
`scripts/bootstrap.sh`, mirroring `bootstrap-macos`, plus test coverage in
`scripts/test_bootstrap_workflow.py` that keeps the three jobs honest the same way the existing
tests keep `bootstrap`/`bootstrap-macos` honest.

## 2. Files to touch

This is a CI-only change — no Rust crate, no self-hosted `compiler/` module, no `docs/spec/*.md`
update. Nothing here touches language syntax, semantics, builtins, or CLI flags, so the "update
both compilers" and "update the spec" obligations in `CLAUDE.md` do not apply.

- `.github/workflows/bootstrap.yml` — add a new job, `bootstrap-linux-arm64`.
- `scripts/test_bootstrap_workflow.py` — extend `BootstrapWorkflowTest` to cover the new job.

No other file changes are needed. `release.yml` and `.github/actions/install-esbmc/action.yml`
already carry the Linux/ARM64 pin (landed in #1213); this issue only wires `bootstrap.yml` to
exercise it earlier.

### New job shape (`bootstrap-linux-arm64`)

Mirrors `bootstrap-macos` exactly in scope (checkout → toolchain → cache → install ESBMC → run
`scripts/bootstrap.sh --stage3-no-verify`), *not* the full `bootstrap` job — the issue explicitly
says "mirroring the existing `bootstrap-macos` job", and `bootstrap-macos` deliberately excludes
the Linux-only "Compiler test suite" / "Verifier-evaluation suite" steps (those already run once,
on `bootstrap`; duplicating them on a second Linux arch would just re-run the same parity/verify-eval
comparison against a differently-shaped runner for no new signal).

```yaml
  bootstrap-linux-arm64:
    needs: changes
    if: needs.changes.outputs.code == 'true'
    # Runs the same verified three-stage bootstrap as Linux/x86_64 and macOS: Stages 1-2 verify
    # with the arm64 Linux ESBMC artifact, while Stage 3 skips ESBMC to roughly halve wall time.
    # Verification does not affect codegen, so the SHA-256 fixed-point check remains meaningful.
    # Runner label matches release.yml's linux/aarch64 leg (ubuntu-24.04-arm) so both jobs depend
    # on the same GitHub-hosted ARM64 image resolving the install-esbmc pin identically.
    runs-on: ubuntu-24.04-arm
    timeout-minutes: 90
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - uses: dtolnay/rust-toolchain@6bed0761d98439e5a578e2877258200ad565ba87 # stable
      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
      - name: Install ESBMC
        uses: ./.github/actions/install-esbmc
      - name: Bootstrap (Stages 1-2 verify, Stage 3 fixed-point check)
        run: scripts/bootstrap.sh --stage3-no-verify
```

Placed after `bootstrap-macos` in the file (append, don't interleave — keeps the diff a pure
addition).

**Naming judgement call:** the job id is `bootstrap-linux-arm64`, not `bootstrap-arm64`. The
existing pair (`bootstrap` = implicit linux/x86_64, `bootstrap-macos` = implicit macos/aarch64)
suffixes by OS only because each OS in this file has exactly one arch today. Linux now has two, so
the OS-only suffix `-arm64` would be ambiguous next to `bootstrap-macos` (is `-macos` an OS or is
`-arm64` an arch?). Spelling out `-linux-arm64` disambiguates without renaming the two existing
job ids — renaming `bootstrap` or `bootstrap-macos` is out of scope (see §6): those ids may be
referenced by branch-protection required-status-check configuration outside version control, and
this repo's discipline (per `CLAUDE.md`, "surgical changes") is to keep this PR a pure addition.

## 3. TDD slices

Both slices land in one PR (this is a single small addition, not a multi-PR feature), but each is
independently red→green so the diff stays reviewable and bisectable.

1. **Red: extend the workflow-shape assertions to expect a third platform.**
   - File: `scripts/test_bootstrap_workflow.py`, `BootstrapWorkflowTest`.
   - Leave all three method names as-is (`test_covers_both_platforms`,
     `test_runs_the_bootstrap_script_on_both_platforms`, `test_bootstrap_verifies_with_esbmc`) —
     the issue cites them verbatim as the tests to extend, and a reviewer grepping for those exact
     names should find them. Renaming only one of the two "...both_platforms" names would leave
     the pair inconsistent (one stale, one not); renaming both is a naming refactor the issue
     didn't ask for. Not touched in this slice.
   - In `test_covers_both_platforms`, add:
     `self.assertIn("runs-on: ubuntu-24.04-arm", self.jobs["bootstrap-linux-arm64"])`.
   - Extend the iteration tuple in `test_runs_the_bootstrap_script_on_both_platforms` from
     `("bootstrap", "bootstrap-macos")` to `("bootstrap", "bootstrap-macos",
     "bootstrap-linux-arm64")`.
   - Extend the same tuple in `test_bootstrap_verifies_with_esbmc`.
   - Run `python3 scripts/test_bootstrap_workflow.py` (or `pytest scripts/test_bootstrap_workflow.py -q`)
     and confirm the three touched tests fail with `KeyError: 'bootstrap-linux-arm64'` (from
     `self.jobs["bootstrap-linux-arm64"]` — `job_blocks()` never produced that key against the
     current `bootstrap.yml`). This confirms the test actually exercises the new job rather than
     vacuously passing.
   - Production code: none yet.

2. **Green: add the job to `bootstrap.yml`.**
   - File: `.github/workflows/bootstrap.yml`.
   - Add the `bootstrap-linux-arm64` job exactly as drafted in §2, appended after
     `bootstrap-macos`.
   - One-word fixes to job-count prose in the two files this slice already touches (not a separate
     cleanup pass — these sentences state the exact fact this slice changes):
     - `bootstrap.yml`'s top-of-file comment: "Together the **two** jobs are ~900s of runner
       time" → "three jobs" (and drop or rephrase "the Linux one alone *was* the critical path" if
       it no longer reads correctly with three legs; check the surrounding sentence when editing).
     - `test_bootstrap_workflow.py`'s module docstring: "it still covers **both** platforms, and
       **both** legs still verify with ESBMC" → adjust to three.
   - Re-run `python3 scripts/test_bootstrap_workflow.py`; all `BootstrapWorkflowTest` cases
     (including the untouched `test_nightly_does_not_collide_with_the_other_scheduled_workflows`
     and the module-level `ParserTest`) must pass. Also run the full file
     (`python3 -m pytest scripts/test_bootstrap_workflow.py -v`) to check no other test class in
     the same file regressed — `CiWorkflowTest`, `ReleaseWorkflowTest`, `FullTestWorkflowTest`,
     etc. share the file but read different workflow files, so they should be unaffected; running
     them is a cheap confirmation, not a fix target.
   - No refactor step: the job is a straight copy of `bootstrap-macos`'s shape with `runs-on` and
     the comment block adjusted, so there is nothing further to simplify.

No other test file changes. `scripts/test_install_esbmc_action.py` tests the composite action
itself (already covers the `Linux/ARM64` case-arm since #1213); it does not need to know which
*workflow* jobs consume the action, so it is out of scope here.

## 4. Verification surface

Not applicable in the ESBMC-contract sense — this change adds no Vow source, contract, IR, or
codegen. It is CI configuration whose entire purpose *is* to run the existing ESBMC verification
pipeline (`scripts/bootstrap.sh`'s Stage 1-2 `--stage3-no-verify` path) on a platform that already
has a checksum-pinned solver archive. No `tests/run/` or `examples/` fixtures need to grow; the
"tests" here are the Python assertions on workflow YAML shape described in §3.

## 5. Risk areas

- **Runner availability/label drift.** `ubuntu-24.04-arm` is a GitHub-hosted label already used by
  `release.yml`'s `linux/aarch64` leg (`runner: ubuntu-24.04-arm`, `verify: true`, landed in
  #1213), so the *label itself* resolving to a real runner is not a new unknown. What is genuinely
  unproven is the ESBMC pin plus `scripts/bootstrap.sh` actually succeeding on that runner: per the
  issue, no `release.yml` run has reached that leg yet (releases are weekly and gated on
  semantic-release deciding one is due), so this job's first run on `main` is plausibly the first
  real exercise of the Linux/ARM64 pin anywhere in CI — and could legitimately fail on a bad
  checksum/archive-layout/solver issue specific to that arch. That failure mode is exactly what
  this issue exists to surface earlier and cheaper than at release time, so it is expected
  behavior for slice 2's first `main` run to be red, not a sign the plan is wrong.
  Optional pre-merge check: `bootstrap.yml` has a `workflow_dispatch` trigger, and
  `ci_docs_only.py` reports `code=true` when there's no commit range — so
  `gh workflow run bootstrap.yml --ref <branch>` can exercise the new job before merge, at the
  cost of a ~90-minute job run. Left to the implementation stage's judgment; not required to close
  the issue.
- **CI runner-minute cost / queue pressure.** A third `bootstrap.yml` job runs on every push to
  `main` and nightly. It runs in parallel with `bootstrap` and `bootstrap-macos` (all three
  `needs: changes` only), so wall-clock impact is bounded by the slowest leg, not additive; the
  additive cost is ARM64 runner-minutes themselves. This is a cost tradeoff already accepted by
  the issue's own reasoning (catch it on every push/nightly, not just weekly at release time), so
  it is not treated as a blocking risk, only noted.
  - **Not a required-status-check risk in practice.** `bootstrap.yml` triggers on `push`/`schedule`/
    `workflow_dispatch`, not `pull_request` — it cannot be a PR-blocking required check today, so
    adding a job here cannot newly block merges. No branch-protection config change is implied or
    needed.
- **Test parser regressions.** `job_blocks()`/`JOB_KEY` in `test_bootstrap_workflow.py` parse on a
  strict "exactly two leading spaces" convention. `ParserTest.test_the_workflows_actually_parse`
  already guards this generically; slice 1 adds a job-specific guard
  (`self.jobs["bootstrap-linux-arm64"]` must resolve) that would fail loudly, not silently, if the
  new job's indentation broke the regex.
- **No risk to:** the Rust/self-hosted binary fixed point (`compiler/` untouched), `vow-clif-shim`
  stack-slot layout, `BTreeMap`/`HashMap` ordering, `parse → print → parse` idempotency, or
  `cargo clippy --all -- -D warnings` (no Rust source touched). `commitlint`/PR-title rules apply
  only to the PR title, handled at PR-creation time by the implementation stage, not by this plan.

## 6. Out of scope

- Renaming the existing `bootstrap` or `bootstrap-macos` job ids for axis symmetry with the new
  `bootstrap-linux-arm64` name — deliberately not bundled; renaming existing job ids is a
  behavior-invisible-but-external-surface change (possible branch-protection references) that
  doesn't belong in a "add one job" PR.
  Note: no repository branch-protection config lives in this checked-out worktree to inspect, so
  this plan cannot confirm whether either existing job id is referenced as a required check
  outside version control; that is exactly why renaming is deferred rather than attempted blind.
- Adding the Linux-only "Compiler test suite (equivalence tier 1)" and "Verifier-evaluation suite"
  steps to the new ARM64 job — `bootstrap-macos` already establishes the precedent of skipping
  these on the non-primary-Linux legs; duplicating them a third time adds runner cost without new
  signal (same parity/verify-eval comparison, different host arch, no arch-specific behavior under
  test in those particular steps).
  - Follow-up worth filing separately if this precedent is ever revisited: whether the equivalence
    tier-1 comparison should run per-arch. Not this issue.
- Any change to `release.yml` or `.github/actions/install-esbmc/action.yml` — both already carry
  the Linux/ARM64 pin from #1213; this issue only adds an earlier exerciser.
- Bumping the ESBMC version or any checksum in `install-esbmc/action.yml`.
- Reformatting or refactoring unrelated parts of `bootstrap.yml` or
  `scripts/test_bootstrap_workflow.py` — e.g. the existing `bootstrap`/`bootstrap-macos` per-job
  comment prose beyond the job-count corrections already folded into slice 2 (§3), or unrelated
  test classes in the same file (`CiWorkflowTest`, `ReleaseWorkflowTest`, etc.).
- `docs/spec/*.md` updates — not applicable; no syntax/semantics/CLI surface changed.
