# Plan: issue #1018 — share `build_skeleton_mismatch_prompt` between bench/ and euler/

## 1. Problem restated

`bench/prompts.py:44` and `euler/run.py:199` each define `build_skeleton_mismatch_prompt(message: str) -> str`
with byte-for-byte identical bodies. Both call sites (`bench/runner.py:165`, `euler/run.py:413`) use it
in lockstep with `compare_skeleton` — `fidelity = compare_skeleton(skeleton, code)` immediately followed by
`build_skeleton_mismatch_prompt(fidelity.message)` when `fidelity.matches` is false. `compare_skeleton` and
its `FidelityResult` dataclass already live in `scripts/benchmark_contracts.py` and are imported by both
runners via a `sys.path` insert of `scripts/` — this is the exact precedent PR #990 established to avoid
duplicating skeleton-comparison logic. The deferred issue asks whether the prompt builder should follow
`compare_skeleton` into that shared module, stay duplicated (matching the other three prompt builders in
`euler/run.py`, which are deliberately divergent from their `bench/prompts.py` counterparts), or move
somewhere else.

**Decision: extract `build_skeleton_mismatch_prompt` into `scripts/benchmark_contracts.py`.** The other
three pairs (`build_system_prompt`/`build_initial_user_prompt`/`build_cegis_*prompt` in bench vs.
`build_system_prompt`/`build_initial_prompt`/`build_cegis_prompt` in euler) are genuinely
product-divergent: different skill-doc directories (`docs/spec` vs `docs/skill`), different prompt
wording, different verify-output handling (bench curates structured counterexamples; euler dumps raw
JSON). Those are independent by design and the "second copy matches house style" argument correctly
applies to them. `build_skeleton_mismatch_prompt` is different in kind: it is not a product-facing
prompt choice, it is the mechanical repair instruction for a `FidelityResult.message` string produced
by exactly one function, `compare_skeleton`, which both runners already import from the same shared
module. The two are always called as a pair at both call sites. Keeping the repair-prompt text
duplicated next to each runner means a future change to how `compare_skeleton` phrases `message` (e.g.
adding a new violation category) can drift the remediation instructions out of sync with the message
format silently — exactly the "silently drift" risk the original reviewer flagged. Co-locating it with
`compare_skeleton`/`FidelityResult` in `scripts/benchmark_contracts.py` removes that drift risk at zero
new-module cost (no new shared-prompts package; both runners already have the import path wired).

## 2. Files to touch

No Rust crate or `compiler/` changes — this is tooling-only (`bench/`, `euler/`, `scripts/`), not a
language change, so no `docs/spec/*.md` update is required.

- `scripts/benchmark_contracts.py` — add `build_skeleton_mismatch_prompt(message: str) -> str`,
  placed directly after `compare_skeleton` (and before the private parsing helpers), with the same
  body currently duplicated in `bench/prompts.py` and `euler/run.py`.
- `scripts/test_benchmark_contracts.py` — add test coverage for the new function (see TDD slices).
- `bench/prompts.py` — delete the local `build_skeleton_mismatch_prompt` definition (lines ~44-51).
- `bench/runner.py` — import `build_skeleton_mismatch_prompt` from `benchmark_contracts` instead of
  from `prompts`; the `SCRIPTS_DIR` sys.path insert and the `benchmark_contracts` import already exist
  at lines 12-16, so this is a one-line move within the existing import block (lines 16-23), not a new
  import mechanism.
- `euler/run.py` — delete the local `build_skeleton_mismatch_prompt` definition (lines ~199-206); the
  `SCRIPTS_DIR` sys.path insert and `from benchmark_contracts import compare_skeleton` already exist at
  lines 22-26, so extend that import to `from benchmark_contracts import build_skeleton_mismatch_prompt, compare_skeleton`.
- `bench/test_prompts.py` — no change expected (grep confirms it does not currently test
  `build_skeleton_mismatch_prompt`); double-check during implementation that removing the function from
  `bench/prompts.py` doesn't break an import there.
- `euler/test_run_contracts.py` — no change expected (same check as above for euler's test file).

## 3. TDD slices

1. **Red:** add a test to `scripts/test_benchmark_contracts.py`, e.g.
   `SkeletonMismatchPromptTests.test_matches_the_text_both_runners_currently_duplicate`, that calls
   `build_skeleton_mismatch_prompt("module name changed from Example to Foo")` and asserts the result
   is **exactly equal** (not a substring match) to the current literal output of
   `bench/prompts.py`'s/`euler/run.py`'s identical implementation for that input. Exact equality (rather
   than substring checks) is what actually proves the move is byte-for-byte and not a silent rewording —
   the specific drift risk this issue exists to close. This fails today because `benchmark_contracts`
   does not export the function.
2. **Green:** add `build_skeleton_mismatch_prompt` to `scripts/benchmark_contracts.py` (copy the exact
   body from `bench/prompts.py`/`euler/run.py` — they are already identical, so this is a pure move, not
   a rewrite). Run `python3 scripts/test_benchmark_contracts.py` to confirm the new test (and all
   existing `compare_skeleton` tests) pass.
3. **Refactor — bench side:** remove the duplicate from `bench/prompts.py`; change the import in
   `bench/runner.py` to pull `build_skeleton_mismatch_prompt` from `benchmark_contracts` instead of
   `prompts`. Run `uv run --project bench --locked python -m unittest discover -s bench -p 'test_*.py'`
   — this must stay green with no test edits, since no bench test currently asserts on
   `bench.prompts.build_skeleton_mismatch_prompt` directly (only on the runner's behavior, which is
   unchanged).
4. **Refactor — euler side:** remove the duplicate from `euler/run.py`; extend the existing
   `benchmark_contracts` import to include `build_skeleton_mismatch_prompt`. Run
   `PYTHONPATH=euler python3 -m unittest discover -s euler -p 'test_*.py'` — must stay green with no
   test edits for the same reason.
5. **Full gate:** run the exact CI sequence from `.github/workflows/ci.yml`'s "Benchmark skeleton
   fidelity unit tests" step, in order, as separate commands (not `&&`-chained):
   ```
   python3 scripts/test_benchmark_contracts.py
   uv run --project bench --locked python -m unittest discover -s bench -p 'test_*.py'
   PYTHONPATH=euler python3 -m unittest discover -s euler -p 'test_*.py'
   ```
6. **Lint/format gate:** CI's commit-msg/lint gates run Python formatting through `pre-commit`'s pinned
   `ruff-pre-commit` hook at `rev: v0.15.14` (`.pre-commit-config.yaml:24`). Verify the three touched
   files with that exact pin, not a newer local `ruff` (see the `vow-python-lint-gate-is-a-pinned-ruff`
   precedent — a newer local ruff reports rules CI never runs):
   ```
   uvx ruff@0.15.14 check scripts/benchmark_contracts.py scripts/test_benchmark_contracts.py bench/prompts.py bench/runner.py euler/run.py
   uvx ruff@0.15.14 format --check scripts/benchmark_contracts.py scripts/test_benchmark_contracts.py bench/prompts.py bench/runner.py euler/run.py
   ```
   Or simply run `pre-commit run ruff-check ruff-format --files <those paths>` if hooks are installed
   (per this repo's `pre-commit install --install-hooks` convention). Either way, run it as its own
   command, not chained into slice 5's test commands.

Each slice is independently committable (new shared function → bench call-site switch → euler call-site
switch), so the implementation stage can split this into up to 3 small commits/PRs if it prefers, but
given the total diff is well under 20 lines across 4 files, landing it as one PR is also reasonable —
leave that call to the implementation stage.

## 4. Verification surface

None. This change touches only Python benchmark-harness tooling (`bench/`, `euler/`, `scripts/`) — no
`.vow` source, no contracts, no codegen, no C model, no ESBMC properties. No `tests/run/*.vow` or
`examples/*.vow` fixtures are affected. `vow verify`/ESBMC are not invoked by this change and need no
new coverage.

## 5. Risk areas

- **None of the binary-fixed-point / codegen risk areas apply** (no `compiler/` or `vow-clif-shim`
  touch, no `BTreeMap`/`HashMap` ordering, no stack-slot layout).
- **No `parse → print → parse` idempotency risk** (no `vow-syntax` touch).
- **`cargo clippy --all -- -D warnings`** is unaffected (pure Python change).
- **Real risk: import ordering / circularity.** `euler/run.py` already does
  `from benchmark_contracts import compare_skeleton  # noqa: E402` after the `sys.path.insert` — the
  `# noqa: E402` marker is there because the import follows non-import statements (the `sys.path`
  setup). Extending that import to a second name needs the same `# noqa: E402` marker preserved. Note
  that `from benchmark_contracts import build_skeleton_mismatch_prompt, compare_skeleton  # noqa: E402`
  is long enough that `ruff format` will likely wrap it into a parenthesized multi-line import — if so,
  the `# noqa: E402` must move to the `from benchmark_contracts import (` line, not stay dangling on a
  continuation line. Run the ruff gate in slice 6 and let the formatter settle this rather than
  hand-wrapping it; don't fight the formatter's choice.
- **Real risk: `bench/prompts.py` import surface.** Confirm nothing else in `bench/` imports
  `build_skeleton_mismatch_prompt` from `prompts` by name (only `bench/runner.py` does per the grep
  above) before deleting it — a stale import elsewhere would be a silent `ImportError` at runtime, not
  caught by type-checking (this repo's Python tooling is unittest-based, no mypy gate found).
- **Low risk: `FidelityResult.message` format coupling.** The new function's only real input is the
  `message` string `compare_skeleton` produces; no contract on that format currently exists beyond
  "a human/LLM-readable string embedded verbatim in the repair prompt." The new test added in slice 1
  should assert the message is embedded verbatim (not reformatted) so a future `compare_skeleton` wording
  change surfaces as a visible diff in the shared module, not a silent two-copy drift — this is the
  actual bug class the issue is trying to close.
- **Low risk: stale module docstring.** `scripts/benchmark_contracts.py`'s module docstring currently
  describes only "structural fidelity checks shared by the benchmark runners." Add one sentence noting
  it also owns the repair prompt emitted when a fidelity check fails, so the docstring stays accurate
  about the module's post-change scope.

## 6. Design-decision documentation (required)

The issue explicitly frames this as "a design-convention call for a human to settle, not a clear-cut
bug" — this plan is that settlement, made by the planning agent in lieu of a human since the run is
autonomous. The implementation stage must make the decision traceable in both places a future reader
would look:

- **PR body** must restate the decision and rationale from Section 1 above (extract because the prompt
  is mechanically coupled to `compare_skeleton`'s output and always called as its pair, not because it's
  "shared logic" in general), and explicitly name the two rejected alternatives (keep duplicated to match
  the other three prompt-builder pairs; invent a new shared-prompts module) and why each was rejected.
- **PR title** must be a lower-case Conventional Commits subject, e.g.
  `refactor(bench): share skeleton mismatch prompt with euler runner` (type `refactor` — this is a
  behavior-preserving move, not a `fix` or `feat`).
- After the PR is open, post a `gh issue comment 1018` summarizing the decision and linking the PR, per
  the operating contract's "document best-effort decisions" rule — the issue's own text says the two
  evaluators disagreed, so closing it silently via squash-merge without a comment would lose the
  rationale trail the issue was filed to capture.

## 7. Out of scope

- **Do not touch `build_system_prompt`, `build_initial_user_prompt`/`build_initial_prompt`, or
  `build_cegis_user_prompt`/`build_cegis_prompt`.** These are deliberately divergent per the issue's own
  triage notes (different skill-doc paths, different prompt text, different verify-output handling) —
  merging them would be a scope-creeping redesign the issue explicitly did not ask for and the second
  evaluator explicitly argued against.
- **Do not create a new `scripts/prompts.py` or similar shared-prompts module.** `scripts/benchmark_contracts.py`
  already exists, is already imported by both runners for the paired `compare_skeleton` call, and already
  owns the `FidelityResult` shape this prompt builder consumes — introducing a second shared module for
  one function would be the kind of premature-abstraction surface area this repo's development discipline
  (`CLAUDE.md`: "deep modules", "surgical changes") argues against.
- **No formatting/cleanup pass over `bench/prompts.py` or `euler/run.py` beyond the single function
  move.** Both files have unrelated content (other prompt builders, the full CEGIS loop, `curate_verify_output`
  in bench) that stays untouched.
- **No change to `scripts/test_benchmark_contracts.py`'s existing `compare_skeleton` test suite** beyond
  adding the one new test class for the moved function.
- **Follow-up (not this issue):** if a future issue wants to also de-duplicate wording between
  `build_cegis_user_prompt` (bench) and `build_cegis_prompt` (euler), that is a separate, larger design
  decision (it would require reconciling bench's structured counterexample curation with euler's raw-JSON
  dump) and should get its own issue, not be bundled here.
