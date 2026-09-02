# Plan: #1175 — Run promoted equivalence fixtures in blocking pull-request CI

## 1. Problem restated

`scripts/full_test.sh` Section 4 (`tests/run/*.vow`, Rust-vs-self-hosted build/run/output
parity) and Section 7 (`tests/error/*.vow`, Rust-vs-self-hosted diagnostic parity) are the
"promoted fixtures" tier: every confirmed Rust/self-hosted divergence gets delta-debugged to a
minimal `.vow` reproducer and committed here so it regression-guards forever
(`docs/equivalence/README.md`). Today that tier only runs as part of the *complete*
`full_test.sh` sweep, which `full-test.yml` fires on push-to-`main` and nightly — never on a
pull request — because the full sweep is ~4384s and #1171 (self-hosted suite performance) makes
that latency unacceptable per-PR. But Section 4 + Section 7 alone measured 145s + 24s on the same
contended host. Section 7 is `build --no-verify` throughout; Section 4 is too, except for the 3
`tests/run/*.vow` fixtures carrying `// TEST: verify-only`, which call `vow verify` and therefore
do need ESBMC on `PATH`. That's still cheap — ESBMC install is a checksum-verified cache restore
(seconds), and the 145s measurement already includes those 3 verify calls — it just means the new
job isn't ESBMC-free, only ESBMC-*light* compared to sections 4b/4c/4d/9-13, which are the parts
actually excluded. This issue carves that narrow, cheap slice out into its own blocking
pull-request job, without copying Section 4/7's shell logic into new YAML (which would drift
from `scripts/parity.py`'s suppression/comparison rules the next time either changes).

## 2. Files to touch

- `scripts/full_test.sh` — extract Section 0's compiler-build lines and the Section 4 / Section 7
  loop bodies into named functions; add a `VOW_FULL_TEST_PROMOTED_ONLY=1` early-exit gate
  (mirrors the existing `VOW_FULL_TEST_BOOTSTRAP_ONLY` gate at line ~325). No behavioral change
  to the default (unset) path — same functions, called from the same place in the same order.
- `.github/workflows/promoted-fixtures.yml` — **new** workflow. `pull_request: branches: [main]`
  only (deliberately *not* `push: [main]` — see §5 for why). Docs-only `changes` gate copied from
  `ci.yml`'s pattern. Single job, explicit `timeout-minutes`, **includes** the `install-esbmc`
  composite action (3 `verify-only` fixtures in `tests/run/` need ESBMC on `PATH` — see §1; the
  install itself is a checksum-verified cache restore, seconds not minutes). No `astral-sh/setup-uv`
  step — Section 4/7 only shell out to `python3 scripts/parity.py`, and `scripts/schema_check.py`
  (its one dependency) is stdlib-only, confirmed by reading it. Runs
  `VOW_FULL_TEST_PROMOTED_ONLY=1 scripts/full_test.sh`, with the same `PASS` floor pattern
  `full-test.yml` uses (`tee` + `grep -oP` + numeric floor), scaled down to the Section 0+4+7
  assertion count.
- `scripts/test_bootstrap_workflow.py` — extend (not a new file: its docstring already scopes it
  to "which workflow carries which equivalence tier"). Add a `PromotedFixturesWorkflowTest`
  class mirroring `FullTestWorkflowTest`, plus a small script-shape test class asserting the new
  bash gate exists and that Section 4/7 are each defined once and called exactly twice (gate path
  + normal sequential path) — the mechanical guard against someone re-inlining a copy later.
  Add `PROMOTED_FIXTURES_WORKFLOW = WORKFLOWS / "promoted-fixtures.yml"` next to the other
  workflow constants.
- `.github/workflows/ci.yml` — no code change expected (extending an already-wired test file),
  but re-verify the `test_bootstrap_workflow.py` step still covers it after the edit.
- `docs/equivalence/README.md` — update the Tier 1 table row and the "Both jobs fail... but
  neither is on the pull-request path" paragraph (lines ~35-67) to state that the promoted-fixture
  half of Tier 1 (`tests/run/`, `tests/error/`) now blocks pull requests via
  `promoted-fixtures.yml`, while the `vow test compiler/` comparison in `bootstrap.yml` remains
  nightly/push-only pending #1171. This is a documentation-only edit; no `docs/spec/*.md` changes
  are needed since no language surface, CLI flag, or builtin changes.
- `docs/spec/cli.md` — checked, not touched: its "Tier 1/Tier 2" table is the *mutation-testing*
  oracle tiers (`vowc mutants`), an unrelated naming collision with the equivalence tiers. Leave
  it alone.

## 3. TDD slices

1. **Extract Section 0's compiler-build lines into `setup_compilers()`.**
   Red: none needed (pure refactor, no behavior change) — the safety net is
   `scripts/test_bootstrap_workflow.py::test_passed_count_grep_matches_full_test_sh_summary_format`-style
   structural assertions plus running `full_test.sh` locally before/after and diffing the
   PASS/FAIL/SKIP totals (must be identical). Production: move the two lines
   (`cargo build --all --release`, `$RUST --no-verify compiler/main.vow -o "$TMPDIR/vowc_self"`)
   into a function defined in the "Helpers" block near the top; call it from Section 0's existing
   position.

2. **Extract the Section 4 loop body into `run_promoted_run_tests()`.**
   Test: new assertion in `scripts/test_bootstrap_workflow.py` (new class, e.g.
   `FullTestPromotedGateTest`) that `scripts/full_test.sh` contains a function definition
   `run_promoted_run_tests()` and that the literal string `run_promoted_run_tests` appears exactly
   three times in the file: one `run_promoted_run_tests() {` definition, plus two call sites (the
   normal sequential flow, and the new gate). Write this test first — it fails against the current
   file (function doesn't exist yet, so the count is zero).
   Production: cut Section 4's `for vow_file in tests/run/*.vow; do ... done` body into the new
   function; replace the inline loop with a call. No logic changes inside the loop.

3. **Extract the Section 7 loop body into `run_promoted_error_tests()`.**
   Same test-first pattern as slice 2 (exactly three occurrences: one definition, two calls), for
   `run_promoted_error_tests`, including the four
   synthetic `$TMPDIR/*.vow` fixtures Section 7 heredocs in before the `tests/error/*.vow` glob
   (parse_error, type_error, missing_module, const_type_mismatch) — those stay inside the
   function since they're inputs to the same comparison, not setup shared with other sections.

4. **Add the `VOW_FULL_TEST_PROMOTED_ONLY` gate.**
   Test: extend the same new test class to assert the gate block exists, is positioned after
   `setup_compilers` and before Section 0b (so it never touches ESBMC-bearing sections), and
   calls `print_summary` + `exit` (mirrors the `VOW_FULL_TEST_BOOTSTRAP_ONLY` block's shape via a
   regex/substring check, not by executing it — full execution needs both compilers built and is
   exercised for real by the workflow in slice 6). Production: add the gate block right after
   Section 0's `setup_compilers` call:
   ```bash
   section_begin "Section 0: Setup"
   setup_compilers
   if [ "${VOW_FULL_TEST_PROMOTED_ONLY:-0}" = "1" ]; then
       section_begin "Section 4: Run Tests"
       run_promoted_run_tests
       echo ""
       section_begin "Section 7: Error Handling"
       run_promoted_error_tests
       echo ""
       summary_status=0
       print_summary || summary_status=$?
       exit "$summary_status"
   fi
   ```
   Manually run `VOW_FULL_TEST_PROMOTED_ONLY=1 scripts/full_test.sh` locally once implemented, to
   confirm it exits after Section 7 and to record the actual PASS count for slice 6's floor.

5. **Workflow-shape tests for the new job.**
   Test: `PromotedFixturesWorkflowTest` in `scripts/test_bootstrap_workflow.py`, modeled on
   `FullTestWorkflowTest`:
   - `pull_request: branches: [main]` present; `push:` to `main` is **absent** (regression guard
     against the double-run described in §5).
   - `permissions: contents: read` only.
   - docs-only `changes` gate wired the same way as `ci.yml`/`full-test.yml`
     (`fetch-depth: 0`, `python3 scripts/ci_docs_only.py`, `needs: changes`,
     `if: needs.changes.outputs.code == 'true'`).
   - `timeout-minutes:` present on the job.
   - `VOW_FULL_TEST_PROMOTED_ONLY=1` and `scripts/full_test.sh` both present in the run step.
   - `install-esbmc` is **present** (the 3 `verify-only` fixtures need it — see §1/§2; this
     mirrors `FullTestWorkflowTest.test_runs_full_test_sh_with_required_toolchain`, which asserts
     the same for `full-test.yml`).
   - a PASS-floor check following `full-test.yml`'s `tee`/`grep -oP`/numeric-floor shape.
   Write these against a not-yet-created `promoted-fixtures.yml` first (red), then add the file.

6. **Add the workflow file.**
   Production: `.github/workflows/promoted-fixtures.yml`, closely mirroring `full-test.yml`'s
   `changes` + single-job shape (checkout, `dtolnay/rust-toolchain`, `Swatinem/rust-cache`,
   `install-esbmc`, then `VOW_FULL_TEST_PROMOTED_ONLY=1 scripts/full_test.sh`). No
   `astral-sh/setup-uv` step (see §2). Set the PASS floor from slice 4's locally-recorded number,
   with headroom below it (mirror `full-test.yml`'s ~500-vs-~590/976 ratio). Set `timeout-minutes`
   generously above the observed local wall time: a *cold* `Swatinem/rust-cache` means
   `cargo build --all --release` alone can run 8-12 minutes on a shared runner, before the
   145s+24s of Section 4/7 even start, so start the ceiling at 30 minutes rather than a tight 20 —
   a timeout failure on a per-PR check is worse than a slow-but-passing one — and tighten once
   real runs show a stable warm-cache number.

7. **Docs update.**
   Production: `docs/equivalence/README.md` Tier 1 row and prose (§2 above). No test — this is
   prose describing CI reality that slice 6's actual workflow run makes true; nothing to assert
   beyond human review that the words match the YAML.

8. **Full local regression pass.**
   Not a new slice's test, but the exit gate for the whole change: run
   `scripts/full_test.sh` (full, unset `VOW_FULL_TEST_PROMOTED_ONLY`) locally start-to-finish and
   confirm PASS/FAIL/SKIP totals match a pre-change baseline run exactly. The refactor in slices
   1-3 must be behavior-preserving for the *existing* nightly/push-to-main job, not just for the
   new gate.

## 4. Verification surface

No contracts, codegen, or C-model changes — this is CI/tooling only. No ESBMC properties to
prove. No new `tests/run/` or `examples/` fixtures needed; the change makes *existing* fixtures
run earlier in the pipeline, it doesn't add new ones. `scripts/parity.py` (the comparator both
the old and new code paths call) is untouched, which is the point: zero drift because it's the
same function calls, same file, just reachable through a new short-circuit.

## 5. Risk areas

- **Double-running on push to `main`.** `full-test.yml` already runs the *complete* suite
  (including Section 4/7) on every push to `main`. If `promoted-fixtures.yml` also triggered on
  `push: [main]`, Section 4/7 would run twice per merge for no benefit. Trigger on `pull_request`
  only — slice 5's test asserts `push:` is absent as a permanent regression guard.
- **Refactor correctness (bash, no type system to catch mistakes).** Moving Section 4/7 bodies
  into functions must not change which variables are global vs. shadowed — the loop bodies rely
  on script-global `TMPDIR`/`RUST`/`SELF`/`PASS`/`FAIL`/`SKIP` and don't currently use `local`,
  so wrapping them in `function name() { ... }` preserves that (bash functions don't create a new
  variable scope unless `local` is used inside). Slice 8's full local run is the actual proof;
  the unit tests in slice 2/3/5 only guard the *shape*, not the runtime semantics.
- **Gate placement relative to Section 0b.** The gate must sit after `setup_compilers` (both
  compilers need to exist) but before Section 0b (concrete block-region parity) and everything
  after — Section 0b is out of scope for this issue (not `tests/run/`/`tests/error/`) and, more
  importantly, sections after Section 4 include ESBMC-touching work (4b/4c/4d) that would defeat
  the whole point of a cheap job if accidentally reachable.
- **PASS-floor staleness.** `full-test.yml`'s existing floor (500) is a magic number tied to a
  point-in-time fixture count; the new job needs its own floor tied to Section 4+7's fixture
  count only. Getting this number from a real local run (slice 4/6) rather than guessing avoids
  either a floor so loose it never catches a real regression, or so tight it flakes on legitimate
  environment-specific skips.
- **Not a binary-fixed-point or `parse → print → parse` risk.** This change touches no compiler
  code, no `compiler/*.vow`, no `vow-clif-shim`, no codegen ordering. The self-hosted compiler
  built by `setup_compilers()` for the new job is the same `--no-verify` build Section 0 already
  produces today; nothing here changes what gets compiled or how.
- **`cargo clippy --all -- -D warnings`** — unaffected; no Rust source changes.

## 6. Out of scope

- Moving the `vow test compiler/` two-compiler comparison (added by #1082, currently in
  `bootstrap.yml`, nightly/push-only) onto the pull-request path. That's explicitly blocked on
  #1171 per the issue body and stays where it is.
- Any change to `scripts/parity.py`'s comparison or suppression logic. This issue is about
  *where* the existing comparator runs, not what it compares.
- Speeding up `cargo build --all --release` or the self-hosted `--no-verify` build themselves
  (e.g., splitting the release build to only what's needed, caching the self-hosted binary across
  jobs). Worth a follow-up if the new job's wall time turns out to be dominated by setup rather
  than Section 4/7 themselves, but not needed to close this issue.
- Refactoring any *other* `full_test.sh` section into a function. Only Section 0's build lines,
  Section 4, and Section 7 move — sections 4b/4c/4d/etc. are untouched, even though the same
  function-extraction pattern would apply cleanly to them too. Bundling that in would be exactly
  the kind of unrelated cleanup CLAUDE.md's "surgical changes" principle warns against.
- Adding a directory-glob override mechanism (e.g. `VOW_FULL_TEST_RUN_DIR`) to make Section 4/7
  independently testable against synthetic fixture sets in isolation from the real
  `tests/run/`/`tests/error/` corpus. Considered and rejected: the workflow run against the real
  corpus on this PR itself is the functional proof; a synthetic-fixture harness would be new
  surface area to maintain for a benefit the shape tests in slice 2/3/5 and the real CI run
  already cover.
- Updating branch-protection "required checks" — the repo has no ruleset-as-code for that today
  (`ci.yml`'s own comment notes "`main` has no required checks today"), so there's no in-repo file
  to change; this is a GitHub Settings action left to a human/operator if the new job should
  become formally required later.
