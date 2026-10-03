# Plan: #1331 — full_test.sh: treat a one-sided empty compiler output as FAIL, not SKIP

## 1. Problem restated

`scripts/full_test.sh` has 13 "build/verify with both compilers" call sites that all share the
same bug: `if [ -z "$rust_json" ] || [ -z "$self_json" ]; then skip "…" "empty output"; continue`.
The `||` makes *either side alone* producing empty stdout collapse into the same SKIP as *both*
sides being empty. But those two cases mean opposite things: both empty is usually a shared,
benign precondition (e.g. a fixture that legitimately produces no JSON under both compilers); one
side empty while the other succeeds means that compiler crashed or otherwise failed to emit a
result — a real divergence. Evidence from #1329: with mutant 3897 applied to
`compiler/checker.vow:2012`, the mutated self-hosted compiler aborted with
`{"error":"IndexOutOfBounds"}` (exit 134, empty stdout) on 19 of 433 `tests/run` + `tests/error` +
`examples` fixtures, while the unmutated compiler produced 0 empty outputs over the same corpus.
Every one of those 19 crashes was reported as SKIP, not FAIL, which is why `vowc mutants` scored
the mutation `missed` instead of `caught`. The fix is to stop conflating "nothing to compare" with
"one compiler failed silently": keep both-empty as SKIP, and make exactly-one-empty a FAIL, at
every one of these call sites, backed by a single tested classification function rather than
13 independent copies of the same bash conditional.

## 2. Files to touch

This is a CI-harness/tooling change, not a Vow language, compiler, or CLI-surface change — it
touches no Rust crate, no `compiler/*.vow` module, and no `docs/spec/*.md` file (those document
syntax/semantics/builtins/CLI flags; `full_test.sh`'s internal SKIP/FAIL bookkeeping is none of
those). Concretely:

- `scripts/parity.py` — add one pure function, `classify_empty_output(rust_empty, self_empty)`,
  plus a small CLI mode (`empty-output RUST_EMPTY SELF_EMPTY`) so `full_test.sh` can call it the
  same way it already shells out to `parity.py` for `compare_json`/`compare_error`/`compare_test`.
- `scripts/test_parity.py` — unit tests for the new function and its CLI mode. This file is
  already a CI-gating step (`.github/workflows/ci.yml:85`, `python3 scripts/test_parity.py`), which
  is exactly why the issue asks for the classification to live here rather than as untested bash.
- `scripts/full_test.sh` — add one small helper (`check_empty_output`, next to `run_parity` /
  `compare_json` / `compare_error` around line 108-137), then replace each of the 13 "OR → skip"
  call sites with a call to it. No change to the 3 sites that are already correct and have
  different semantics (see "Out of scope").
- `docs/equivalence/README.md` — optional, recommended one-paragraph addition. This file already
  describes `parity.py`'s "comparison and suppression rules" as the single source of truth for how
  `full_test.sh` and `promoted-fixtures.yml`/`full-test.yml` agree; it should say in one sentence
  that one-sided empty output is a hard FAIL, both-empty is a SKIP, so the policy doesn't go
  undocumented the moment this PR lands. Not required by CLAUDE.md (not a `docs/spec/*.md` file),
  so treat as a nice-to-have, not a blocker.

## 3. TDD slices

**Slice 0 — Validation census (no code change).** Before touching anything, confirm the
"both-empty vs one-sided-empty" split actually holds on clean `main`. Two things are already known
from source (read, not run, during planning — see below), and everything else needs an actual run.

*Already established by reading the source (no need to re-derive):*
- `compare_json`/`compare_error` already treat ESBMC unavailability as a **non-empty** soft outcome:
  both `vow/src/verify_outcome.rs:355` (Rust) and `compiler/main.vow:1412/1484/1497`
  (self-hosted, `record_soft_fail(..., "tool_not_found", "ESBMC not found", ...)`) emit a structured
  `verify_status: "tool_not_found"` JSON document, not empty stdout. So "ESBMC availability" —
  the example the issue itself raises in point 1 — **cannot** produce a one-sided empty output;
  rule it out as a source of legitimate leniency.
- `run_self`/`run_self_bin` (`scripts/full_test.sh:63-74`, `apply_vmem_limit`) apply an *optional*
  `ulimit -v "$VOW_ULIMIT_KB"` cap to the self-hosted compiler only — `$RUST` is invoked directly,
  uncapped, everywhere. `VOW_ULIMIT_KB` is not set by any `.github/workflows/*.yml` today (grepped;
  only `scripts/verify_arena.sh` / `scripts/test_verify_arena.py` set it, for Section 11's arena
  checks, which don't use `check_empty_output`). So this asymmetry is dormant in GitHub Actions as
  of this plan, but it is a real, pre-existing, one-sided resource constraint baked into the
  harness itself — the one mechanism in this file that *could* legitimately OOM-kill only the
  self-hosted side on a memory-heavy fixture (Section 6/6b's `stdlib`/`tests/multi` builds, Section
  10b's `test compiler/`) and produce a true one-sided empty output that isn't a compiler bug.
  Slice 0's run must note whether `VOW_ULIMIT_KB` is set in whatever environment runs it (`printenv
  VOW_ULIMIT_KB`), since this autonomous run's own operating contract asks for capped build
  parallelism under a shared memory budget — if some wrapper sets it, that's the one scenario this
  plan should not treat uniformly without a closer look.

*Still needs an actual run* — #1329's census ("unmutated compiler → 0 empty outputs" across the
433 `tests/run` + `tests/error` + `examples` fixtures) is suggestive but was run to characterize
mutant 3897's *self-hosted* crash signature, not explicitly framed as a two-sided rust-vs-self
empty-output sweep — do not cite it as proof the Rust side never empties on these fixtures;
re-derive it directly instead.
Run `scripts/full_test.sh` once on current `HEAD` (wall-clock ~40 min per prior sessions —
background it, poll, do not use the 2-minute default timeout) and grep its output for every `SKIP`
line whose reason is `empty output`, cross-referencing the printed `rust=N, self=M` exit codes, for
every one of the 13 sites — this both closes the Rust-side gap #1329 left open and covers the
sections #1329 never touched at all (Section 2 `verify`, 4b/4c/4d `tests/verify*`, Section 6
`stdlib/*/main.vow` both modes, Section 6b `tests/multi/*/main.vow`, Section 10 `build` default
mode, Section 10b `test compiler/`, and the Section 4 `test-verify` verify-only branch). Expected
result given the two bullets above: 0 one-sided cases anywhere. If that holds, proceed with Slices
1-4 exactly as written — **no section gets special-cased leniency merely for being "verify
mode"**; the classification is uniform across all 13 sites, per Vow's "crisp rule, no speculative
exceptions" design principle. If the census instead finds a genuine, reproducible one-sided-empty
case that isn't a bug (e.g. a `VOW_ULIMIT_KB`-induced self-side OOM on a specific heavy fixture),
do not weaken the classification function or add a per-section carve-out — post a `gh issue
comment` on #1331 documenting the fixture, the reason, and add a narrow `// TEST:`-directive-based
suppression for that fixture only (mirroring the existing `// TEST: known-divergence` mechanism
`compare_runtime` already supports), then continue.

**Slice 1 — RED: `scripts/test_parity.py`.** Add a new test class (e.g.
`ClassifyEmptyOutputTest`) exercising `parity.classify_empty_output`:
  - `classify_empty_output(True, True)` → `"SKIP"` (both empty).
  - `classify_empty_output(True, False)` → `"FAIL"` (rust empty, self produced output).
  - `classify_empty_output(False, True)` → `"FAIL"` (self empty, rust produced output).
  (The call sites in `full_test.sh` only ever invoke this when at least one side actually is empty,
  so `classify_empty_output(False, False)` is not a contract the function needs to define; do not
  add a test — or a bash call — for an input that can't occur.)
  Run `python3 scripts/test_parity.py` — fails with `AttributeError: module 'parity' has no
  attribute 'classify_empty_output'`.

**Slice 2 — GREEN: `scripts/parity.py`.** Add:
  ```python
  def classify_empty_output(rust_empty, self_empty):
      """Verdict for a both-compilers call site where at least one side's JSON is empty.

      Both empty: neither compiler produced anything to compare against the other — SKIP.
      Exactly one empty: that compiler produced nothing while its counterpart succeeded,
      which is itself a parity divergence (most often a crash) — FAIL.
      """
      return "SKIP" if rust_empty and self_empty else "FAIL"
  ```
  Add a CLI mode in `main()`: `empty-output RUST_EMPTY SELF_EMPTY` (each arg `"1"`/`""` or
  `"true"`/`"false"` — match the convention already used for exit codes elsewhere in the file,
  i.e. plain `"1"`/`"0"`), printing `classify_empty_output(...)` and returning exit 0 always (the
  bash caller decides skip-vs-fail from the printed word, exactly as `run_parity` already does from
  `SKIP:`-prefixed stdout). `main`'s existing argument guard is
  `if len(args) not in (5, 6) or args[0] not in ("json", "error", "test"): ...; return 2` — a fixed
  arity tied to three mode names. Adding a 3-argument `empty-output` mode means restructuring this
  into a per-mode arity check (e.g. a `{"json": (5, 6), "error": (5, 6), "test": (5, 6),
  "empty-output": (3, 3)}` table) rather than bolting another special case onto one `or` chain.
  Add a test that `empty-output` with the wrong number of arguments still exits 2 with the usage
  message, alongside one CLI-level test in the existing `ParityCliCharacterizationTest` class
  confirming the subprocess prints `SKIP` / `FAIL` for the three cases above. Run
  `python3 scripts/test_parity.py` — green.

**Slice 3 — `scripts/full_test.sh` helper.** Add, next to `compare_json`/`compare_error`
(~line 135-226):
  ```bash
  # Verdict for a both-compilers call site where at least one side's JSON is empty.
  # Returns 0 (caller should skip the comparison) after logging SKIP or FAIL;
  # returns 1 (caller should proceed to compare_json/compare_error) if neither side is empty.
  check_empty_output() {
      local label="$1" rust_json="$2" self_json="$3" rust_exit="$4" self_exit="$5"
      local rust_empty=0 self_empty=0
      [ -z "$rust_json" ] && rust_empty=1
      [ -z "$self_json" ] && self_empty=1
      if [ "$rust_empty" = 0 ] && [ "$self_empty" = 0 ]; then
          return 1
      fi
      local verdict
      verdict=$(python3 scripts/parity.py empty-output "$rust_empty" "$self_empty")
      if [ "$verdict" = "SKIP" ]; then
          skip "$label" "empty output (rust=$rust_exit, self=$self_exit)"
      else
          fail "$label" "one-sided empty output (rust=$rust_exit, self=$self_exit)"
      fi
      return 0
  }
  ```
  No automated harness exists to unit-test `full_test.sh`'s internal bash functions in isolation
  (`tests/full_test_bootstrap/tests.sh` exercises `vowc` CLI behavior, not this script's own
  helpers, and the file executes top-level section loops immediately on load rather than being
  library-style sourceable). Treat Slice 5 below as this slice's regression test instead of
  inventing a bash test harness that doesn't fit the file's structure.

**Slice 4 — Convert the 13 call sites.** Mechanical, one hunk per site, two shapes depending on
whether the original used `skip "…"; continue` (loop-skip) or `skip "…"` with no `continue`
(if/else inline). Current sites and shapes (line numbers are current-`HEAD`; expect drift after
Slice 3 is committed — recheck before editing):

  | # | Current line | Label | Shape |
  |---|---|---|---|
  | 1 | `full_test.sh:244` | `${name}/test-verify` (Section 4, verify-only branch) | loop-skip |
  | 2 | `full_test.sh:264` | `${name}/test-build` (Section 4) | loop-skip |
  | 3 | `full_test.sh:362` | `${fixture}/error` (Section 7) | loop-skip |
  | 4 | `full_test.sh:591` | `${name}/build-no-verify` (Section 1) | loop-skip |
  | 5 | `full_test.sh:676` | `${name}/verify` (Section 2) | loop-skip |
  | 6 | `full_test.sh:818` | `${name}/verify-test` (Section 4b) | loop-skip |
  | 7 | `full_test.sh:841` | `${name}/verify-fail-test` (Section 4c) | loop-skip |
  | 8 | `full_test.sh:1088` | `${name}/verify-skip-test` (Section 4d) | loop-skip |
  | 9 | `full_test.sh:1296` | `${multi}/build-no-verify` (Section 6) | if/else inline |
  | 10 | `full_test.sh:1307` | `${multi}/verify` (Section 6) | if/else inline |
  | 11 | `full_test.sh:1337` | `${name}/build` (Section 6b) | loop-skip |
  | 12 | `full_test.sh:1688` | `${name}/build-verify` (Section 10) | loop-skip |
  | 13 | `full_test.sh:1706` | `test/subcommand` (Section 10b) | if/else inline (not in the issue's list but the exact same bug shape — convert it too, noting the addition in the PR description) |

  Loop-skip shape becomes:
  ```bash
  if check_empty_output "${name}/test-build" "$rust_json" "$self_json" "$rust_exit" "$self_exit"; then
      continue
  fi
  compare_json "${name}/test-build" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"
  ```
  If/else-inline shape becomes:
  ```bash
  if ! check_empty_output "${multi}/build-no-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit"; then
      compare_json "${multi}/build-no-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$main_file"
  fi
  ```
  Site 13 (`test/subcommand`) passes `$rust_test_json`/`$self_test_json`/`$rust_test_exit`/
  `$self_test_exit` instead — same transform, different variable names.

  Do these 13 edits as separate, reviewable hunks in one commit (or a few commits, per "many small
  changes" — but one PR is fine since they're all the same mechanical change over the same helper;
  splitting into 13 PRs would be churn, not rigor).

**Slice 5 — Regression-prove the fix.** Reuse #1329's exact repro: apply mutant 3897
(`compiler/checker.vow:2012`, `<=` → `>` in `bind_arm_pattern`) to a scratch copy, rebuild
`build/vowc` there (`VOW_CACHE_DIR=$(mktemp -d)` per the compile-cache-staleness memory), and run
the now-patched `scripts/full_test.sh` (or at minimum `run_promoted_run_tests` /
`run_promoted_error_tests` plus a direct invocation against
`tests/run/btreemap_enum_value.vow`) against the mutated self-hosted compiler. Confirm the
previously-SKIPped fixtures now report FAIL with `one-sided empty output (rust=0, self=134)`. Do
**not** commit the mutation — this is a manual verification step, not a permanent fixture (a
deliberately-broken `compiler/checker.vow` has no place in the merged tree). Then re-run the full
census from Slice 0 one more time against the real (unmutated) changes, in full, to confirm no
previously-passing fixture newly regresses to FAIL on clean `main`.

## 4. Verification surface

None. This change touches no contracts, no codegen, no C model, and no `.vow` source — it's a
bash/Python CI-harness fix. ESBMC verification is not implicated; no `tests/run/` or `examples/`
fixtures need to grow (Slice 5's mutation is a throwaway, not a committed fixture — the bug it
demonstrates already has its own regression test from #1329's PR #1332,
`compiler/tests/test_checker_pattern_metadata.vow`, which is out of scope here).

## 5. Risk areas

- **PR-blocking path (`promoted-fixtures.yml`).** Sections 4 and 7 gate every pull request via
  `run_promoted_run_tests` + `run_promoted_error_tests`. Of the 13 sites, only #2 (`test-build`,
  Section 4) and #3 (`error`, Section 7) run there; #1329's self-hosted-crash census is suggestive
  evidence for these two specifically (the fixtures and build-mode invocation match), but per
  Slice 0 it does not substitute for Slice 0's own run, since it was never confirmed to be a
  two-sided (rust-and-self) empty-output measurement. Site #1 (`test-verify`, the verify-only
  branch inside Section 4) also runs on every PR whenever a `tests/run/*.vow` fixture carries
  `// TEST: verify-only` — it is **not** covered by #1329's census and needs Slice 0's run like
  everything else. Converting these three sites is the highest-leverage part of Slice 0 to get
  right before merging, since a false positive here blocks every subsequent PR, not just this one.
- **`VOW_ULIMIT_KB`-induced self-only OOM (see Slice 0).** This is the one mechanism already known
  to exist in the harness itself that could legitimately make only the self-hosted side empty on a
  memory-heavy fixture. It is unset in CI today, so it is not expected to fire in
  `promoted-fixtures.yml` or `full-test.yml`, but Slice 0 must still check it is unset in whatever
  environment runs the validation census, since a false "uniform policy is safe" conclusion reached
  under a silently-capped run would not transfer to an uncapped one (or vice versa).
- **Nightly-only sections (2, 4b, 4c, 4d, 6, 6b, 10, 10b).** Lower urgency (don't block merges),
  but a newly-introduced FAIL here would still turn `full-test.yml` red on `main` after merge. This
  is exactly what Slice 0's census is for — do not skip it on the theory that "it's only nightly."
- **No binary-fixed-point or codegen risk.** This PR touches no `compiler/*.vow`, no
  `vow-clif-shim`, no IR lowering — the self-hosted/Rust binary fixed point, `BTreeMap` ordering,
  and stack-slot layout concerns in CLAUDE.md are not implicated.
- **No `parse → print → parse` risk.** No syntax or printer change.
- **`cargo clippy --all -- -D warnings`.** Not implicated — no Rust source changes in this PR.
- **`scripts/test_parity.py` CI gate.** Already enforced by `ci.yml:85`; Slice 1/2's red/green
  cycle runs against the exact command CI uses (`python3 scripts/test_parity.py`), so there's no
  drift risk between local verification and CI.
- **Don't let the helper's message format silently diverge from the old text.** The existing
  `"empty output (rust=$rust_exit, self=$self_exit)"` wording is preserved verbatim for the SKIP
  case so any tooling or dashboards that grep `full_test.sh` output for that exact phrase keep
  matching; only the FAIL case gets new wording (`"one-sided empty output (...)"`), since that
  message is new (there was no such FAIL before).

## 6. Out of scope

- **The 2 "always FAIL on empty" sites** (`build-no-verify/missing-output-parent` at
  `full_test.sh:610`, `build-no-verify/default-output-no-slash` at `full_test.sh:646`). These
  already FAIL even when *both* sides are empty, because that test's actual assertion is "the
  compiler must create the output directory and executable," not "the two compilers must agree on
  JSON" — both-empty there is itself the failure, not a benign non-comparison. Routing them through
  `check_empty_output` would wrongly demote a real both-sided bug to SKIP. Leave them untouched.
- **Section 2b (`verifier-preamble/rust`, `verifier-preamble/self`, `full_test.sh:747` /
  `:753`).** These already check each side independently with `fail` (not an OR-to-skip pattern) —
  there is no bug here to fix.
- **Any suppression/known-divergence mechanism for empty output**, unless Slice 0's census actually
  surfaces a legitimate case. Do not speculatively build a `// TEST: known-empty-divergence`
  directive now; CLAUDE.md's contract-authoring discipline ("don't distort to accommodate
  hypothetical cases") applies here by analogy — build the escape hatch only once a real case
  needs it.
- **`docs/spec/*.md` updates.** Not a language/CLI/semantics change; none of the spec files
  reference `full_test.sh`'s internal SKIP/FAIL bookkeeping.
- **Refactoring `run_parity`, `compare_json`, `compare_error`, or any other existing comparator.**
  This PR adds one new narrow function; it does not touch the existing comparison/suppression
  logic those functions already implement.
- **Mutation-testing integration** (`vowc mutants`, `docs/mutants.md`). The issue is motivated by a
  mutation-testing false-negative, but the fix is entirely in `full_test.sh`/`parity.py`; no
  `vowc mutants` code changes are needed, and re-running a full mutation sweep to confirm mutant
  3897 now scores `caught` is out of scope for this PR (it's already separately pinned by
  `compiler/tests/test_checker_pattern_metadata.vow` per #1329/#1332).
