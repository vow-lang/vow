# Plan: issue #1094 — CI does not run scripts/full_test.sh (the ~590-assertion differential suite)

## 1. Problem restated

`scripts/full_test.sh` is a ~1680-line, 13-section differential harness that builds the Rust
compiler and a one-pass self-hosted compiler, then compares their JSON output byte-for-byte
(`compare_json`/`compare_error`/`compare_runtime`/`run_parity`) across `examples/`, `tests/`,
`stdlib/`-adjacent fixtures, the `vow test compiler/` contract, `vow complexity`, and the
Section 9 bootstrap triple test — roughly 590 assertions total (589 passed / 0 failed locally on
2026-08-25). No workflow under `.github/workflows/` invokes it (`grep -rn "full_test"
.github/workflows/` is empty), so this entire tier of Rust-vs-self-hosted parity coverage —
already described as a stated, named gap in `docs/equivalence/README.md`'s Tier-1 section ("This
tier is intended to block every PR. It does not yet do so… This is a stated coverage gap, not an
advisory green check.") — runs only when a developer remembers to invoke it by hand. #902's
`c_emitter.vow` `str2` typo reached `main` specifically because `vowc test compiler/` (a subset
already inside `full_test.sh` Section 10b) was red while the bootstrap fixed point stayed green.
The fix is wiring the existing script into CI, not writing new test logic.

Two research findings materially shape the plan and are not restated in the issue:

- **~30 of the ~40 minutes is coverage this repo already pays for elsewhere.** Section 10b
  (`vow test compiler/`, both compilers, `parity.py`) is already a *blocking* step in
  `bootstrap.yml`'s `bootstrap` job (push-to-`main` + nightly 03:53 UTC), and
  `docs/equivalence/README.md` records it alone measured 17.8 minutes in one run. Section 11 is
  literally `scripts/verify_arena.sh` — the same ~800s ESBMC proof `arena-verify.yml` already runs
  nightly (06:29 UTC) and per-PR when `vow-runtime/` changes. Wiring `full_test.sh` in unmodified
  means the new job **duplicates** both of these on every run it fires. This is priced explicitly
  below (§5), not left for a reviewer to discover, and the fix (env-gated section skipping) is
  deliberately deferred to a follow-up (§6) rather than bundled here — `full_test.sh` already has
  an env-var-driven partial-run precedent (`VOW_FULL_TEST_BOOTSTRAP_ONLY` /
  `VOW_FULL_TEST_RUST` / `VOW_FULL_TEST_CONCAT`, lines 325-334) that a follow-up can extend without
  inventing a flag parser.
- **`docs/equivalence/README.md` already names why this can't be a *required* per-PR check**: #1171
  tracks the self-hosted-suite performance gap ("no JSON before a 45-minute bound in one fresh
  concatenated run"). The issue's own three-option framing (per-PR / nightly / split) is missing a
  fourth shape this repo has already adopted for exactly this trade-off: `bootstrap.yml`'s
  `push: branches: [main]` + nightly `schedule` + `workflow_dispatch`, gated behind the same
  `ci_docs_only.py` docs-only classifier `ci.yml`/`bootstrap.yml`/`arena-verify.yml` already share.
  That answers the issue's stated objection to pure-nightly ("a regression can sit on `main` for up
  to a day") at zero pull-request latency cost, and it is an in-repo precedent with its own written
  rationale (`bootstrap.yml:1-18`) rather than a new pattern to justify from scratch. This plan uses
  that shape.

## 2. Files to touch

- **`.github/workflows/full-test.yml`** (new) — the workflow. Mirrors `bootstrap.yml`'s trigger
  shape (`push: branches: [main]`, nightly `schedule`, `workflow_dispatch`), the shared `changes`
  job gated on `ci_docs_only.py`'s `code` output, `permissions: contents: read`, cargo/ESBMC/uv
  toolchain setup steps copied from `ci.yml`'s `build-and-test` job (this script needs
  `cargo build --all --release`, ESBMC on `PATH`, and `uv` for the `uv run python -c` calls inside
  Section 10b — confirmed `uv run python -c "print(1)"` resolves fine from repo root with no
  `pyproject.toml` present), a single `run: scripts/full_test.sh` step, and a summary-floor check
  (see below) that fails the job if the suite silently skipped almost everything.
- **`scripts/test_bootstrap_workflow.py`** — extend with a new `FullTestWorkflowTest` class
  following the file's existing stdlib-only regex style (`job_blocks`, `crons`, `header`). This is
  the TDD surface for a YAML change (see §3). No changes needed to the shared `crons()` /
  `job_blocks()` helpers — they already operate on any workflow text handed to them, and
  `test_nightly_does_not_collide_with_the_other_scheduled_workflows` already globs
  `WORKFLOWS.glob("*.yml")`, so it automatically covers the new file's cron once added — no edit
  needed there.
- **`.github/workflows/ci.yml`** — no change needed. `scripts/test_bootstrap_workflow.py` is
  already invoked as a `ci.yml` step (`python3 scripts/test_bootstrap_workflow.py`, line 78), so
  extending that file's test classes is picked up automatically.
- **`docs/equivalence/README.md`** — update the Tier-1 paragraph. The current text ("This tier is
  intended to block every PR. It does not yet do so… stated coverage gap") becomes stale the moment
  this lands; rewrite it to say Tier 1 now runs automatically (push-to-`main` + nightly) but is
  still not a *blocking, required* per-PR check pending #1171, matching what actually landed. Also
  update the Tier-1 table row's "Cadence" cell (`minutes` / "local full suite; nightly compiler
  tests") to reflect the new automated cadence.
- **`docs/spec/*.md`** — **no change required.** This issue adds no syntax, semantics, type,
  builtin, operator, effect, or CLI flag — it wires an existing test script into CI. Stating this
  explicitly per the source-of-truth instructions, not leaving the section blank.
- **`crates/` / `compiler/`** — **no change required**, for the same reason. This is CI
  infrastructure, not a compiler change; nothing here touches either compiler's source.

## 3. TDD slices

This is a YAML + CI-glue change, not a language feature, so "TDD" here means: write the structural
assertion first (red, because the workflow file doesn't exist yet), then add the workflow to make
it pass — mirroring exactly how `test_bootstrap_workflow.py`'s existing `BootstrapWorkflowTest`
class was presumably built alongside `bootstrap.yml`.

1. **Red: workflow existence + trigger shape.**
   File: `scripts/test_bootstrap_workflow.py`, new `FullTestWorkflowTest` class (add a
   `FULL_TEST_WORKFLOW = WORKFLOWS / "full-test.yml"` constant next to the existing three).
   Add `test_runs_on_push_to_main_and_nightly` (asserts `push: branches: [main]` via `header()`
   plus a daily cron via `crons()`, same pattern as `BootstrapWorkflowTest.test_runs_on_every_push_to_main`
   / `test_runs_nightly_as_a_backstop`) and `test_workflow_keeps_read_only_repository_permissions`
   (copy of the existing `EquivalenceWorkflowTest` version, retargeted). Both fail immediately
   (`FileNotFoundError`/empty read) since `full-test.yml` does not exist.
   Production code: create `.github/workflows/full-test.yml` with the header block only (name,
   `on:`, `permissions:`) to turn these two green.

2. **Red→green: docs-only gate wiring.**
   Test: `test_gated_on_the_docs_only_classifier` — assert the workflow has a `changes` job whose
   `code` output feeds a job-level `if: needs.changes.outputs.code == 'true'` on the real job,
   mirroring `ci.yml`/`bootstrap.yml`'s existing `changes` job byte-for-byte (same
   `ci_docs_only.py` invocation, same `fetch-depth: 0`). Add that job to the workflow to pass it.

3. **Red→green: the suite actually runs, with the toolchain it needs.**
   Test: `test_runs_full_test_sh` — assert `"scripts/full_test.sh"` appears in the job body, and
   assert the job installs ESBMC (`"install-esbmc"` present, matching
   `test_linux_bootstrap_verifies_with_esbmc`'s style) and installs `uv`
   (`"astral-sh/setup-uv"` present) — both are load-bearing: Section 11 skips (not fails) without
   `esbmc` on `PATH`, but Section 10b's `uv run python -c` calls hard-`FAIL` (not skip) without
   `uv`, which would misreport a missing-toolchain problem as a genuine parity regression.
   Add the `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`, `install-esbmc`, and
   `astral-sh/setup-uv` steps plus the `run: scripts/full_test.sh` step to pass it.

4. **Red→green: the summary-floor assertion.**
   `full_test.sh`'s `print_summary` (lines 301-322) exits nonzero only when `FAIL > 0` — a run
   that skips almost everything (missing `esbmc`, missing `uv`, a broken `cargo build`) still
   exits 0 and reports green, exactly the failure mode `equivalence.yml`'s `--min-compared 20` and
   `verify_eval.py`'s SOUNDNESS banner already guard against elsewhere in this repo. Test:
   `test_enforces_a_minimum_passed_count` — assert the job body greps the captured `full_test.sh`
   output for the "`N passed`" summary line (the number appears contiguously before the literal
   text `" passed"` in `print_summary`'s `echo -e` — the ANSI reset code sits *after* "passed", not
   between the digits and the word, so no ANSI-stripping is needed) and fails the step if `N` is
   below a floor. Add the actual step:
   ```bash
   scripts/full_test.sh 2>&1 | tee /tmp/full_test.log
   passed=$(grep -oP '\d+(?= passed)' /tmp/full_test.log | tail -1)
   test -n "$passed" && [ "$passed" -ge 500 ]
   ```
   500 is chosen with margin below the observed 589 (2026-08-25) to tolerate legitimate future
   `skip` growth (e.g. new `// TEST: known-divergence` fixtures) without false-failing, while still
   catching a near-total-skip run. Note `full_test.sh`'s own `set -euo pipefail` plus the `tee`
   means the exit code of the pipeline is `full_test.sh`'s own (last non-`tee` command in a
   `pipefail` shell) — this must be preserved so a genuine `FAIL` still fails the step even when
   the floor check would otherwise pass; write the step as two sequential commands (run, then
   check), not compressed into one `&&`-chain per this repo's own quality-gate convention of
   separate steps.

5. **Green: docs update, then a real run.**
   Update `docs/equivalence/README.md`'s Tier-1 paragraph and table row (§2). No new test guards
   prose; this is a documentation-accuracy fix, checked by re-reading the paragraph against what
   actually landed. Then trigger the workflow for real: `workflow_dispatch` cannot run from a
   non-default branch before this PR merges (GitHub Actions requires the workflow file to already
   exist on the default branch), so the first automated execution is the push-to-`main` trigger
   after the PR is squash-merged — mirroring how `bootstrap.yml` itself must have first run. Before
   opening the PR, the implementation stage should run `scripts/full_test.sh` once locally (or in
   whatever sandboxed CI-like environment it has) to confirm the suite is green on the actual
   changeset, and say so in the PR body together with the fact that first in-CI evidence lands
   post-merge — this satisfies the issue's "confirm on a CI runner before making it blocking" to
   the extent that's achievable pre-merge, and is honest about the rest.

## 4. Verification surface

**None required.** This change touches no contracts, no codegen, no C model, and no `vow verify`
path — it is CI wiring around an existing shell script. No new `tests/run/` or `examples/` fixture
is needed: the ~590 assertions already exist inside `full_test.sh` and its constituent Python
tools (`parity.py`, `verify_eval.py`, `verify_arena.sh`). The "known pre-existing failures" the
issue flags (`arena/esbmc` OOM under the 2 GB cap, `math/verify` abs/libc collision) are both
already-documented, already-classified environment properties rather than open bugs: `math`'s
`abs`/libc block is recorded in `stdlib/README.md`'s module table today ("Blocked (env: `abs`/libc)"),
and the arena OOM risk is the same 2 GB cap `arena-verify.yml` already runs under successfully on
GitHub-hosted runners per-PR today (its own comment records "~845s on a GitHub runner at ESBMC
8.1"). Neither requires new verification work; both are pre-existing, tracked conditions that this
plan's new job will either reproduce identically to `arena-verify.yml` (arena) or continue to skip
via the existing skip/known-divergence machinery (math), which is exactly what §3 slice 5's
pre-merge local run is meant to confirm before this becomes a standing CI signal.

## 5. Risk areas

A YAML-and-shell CI change cannot touch the binary fixed point, `vow-clif-shim` stack-slot layout,
`BTreeMap`/`HashMap` codegen ordering, or `parse → print → parse` idempotency — none of those are
at risk here. The actual risks are operational:

- **Duplicated ESBMC cost (priced, not a defect).** Every push-to-`main` and nightly firing of
  `full-test.yml` re-runs `arena-verify.yml`'s ~800s proof (Section 11) and re-runs
  `bootstrap.yml`'s already-blocking `vow test compiler/` parity check (Section 10b), on top of
  this job's own ~40-minute cost. Total nightly ESBMC/CI spend goes up by roughly one more
  `arena-verify.yml`-sized run per day. Accepted for this slice; a follow-up (§6) can gate those
  two sections out via an env var extending the script's existing `VOW_FULL_TEST_*` precedent.
- **Silent-skip theater.** Addressed directly in §3 slice 4 — without the summary-floor check, a
  job missing `esbmc` or `uv` reports green having measured a fraction of the suite, which is
  worse than not running it at all because it looks like a passing signal.
- **Cron collision.** `test_nightly_does_not_collide_with_the_other_scheduled_workflows` already
  auto-discovers every `.github/workflows/*.yml` cron, so picking a colliding time fails an
  existing, already-`ci.yml`-blocking test rather than silently contending for runners. Existing
  crons: 02:17 (`cargo-mutants`), 03:53 (`bootstrap`), 04:41 (`equivalence`), 06:29
  (`arena-verify`); this plan picks something clear of all four (e.g. 05:15 UTC) and lets the test
  enforce it stays that way.
- **Local-vs-runner failure delta.** The issue itself notes the historically-flagged failures did
  not reproduce locally on 2026-08-25 and "may be environment-specific." The push-to-`main` +
  nightly (not per-PR-blocking, no branch-protection required-check) placement means a first red
  run on the GitHub runner is informational, not merge-blocking — consistent with how
  `arena-verify.yml`/`equivalence.yml` already treat their own first-class-citizen-but-not-gating
  status on `main` (this repo currently has no required status checks per `ci.yml`'s own header
  comment).
- **`uv run python -c` resolution.** Confirmed working from repo root with no `pyproject.toml`
  present (`uv run python -c "print(1)"` succeeds), unlike `ci.yml`'s existing `uv run --project
  bench --locked` invocations which target a specific project. `astral-sh/setup-uv` must still be
  a step in the new job for `uv` to be on `PATH` at all.

## 6. Out of scope

- **Env-gated section skipping** (dropping Section 10b/11 from the new job to eliminate the
  duplication in §5) — a real follow-up, but a second, independent, reviewable change to
  `full_test.sh` itself; bundling it here would mix a CI-wiring PR with a test-script refactor.
- **Fixing #1171** (the self-hosted-suite performance gap that blocks *required, per-PR* placement)
  — tracked separately; this plan does not attempt to speed up `vow test compiler/` or the
  self-hosted build.
- **Making `full-test.yml` a required/blocking status check** on `main` — no required checks exist
  today (`ci.yml`'s own comment), and adding one is a repository-settings change outside this PR's
  and this agent's authority.
- **Fixing the `math`/`abs`-libc or any other pre-existing stdlib verification gap** — both are
  pre-existing, already-documented conditions (§4), not regressions introduced or required to be
  fixed by this issue.
- **Any reformatting or refactor of `scripts/full_test.sh`'s existing 13 sections** — the issue is
  explicit that the script's content is already correct and complete; this plan changes zero lines
  inside it.
