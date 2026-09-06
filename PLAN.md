# Plan: issue #420 — help coverage accepts missing types if the token appears elsewhere

## 1. Problem restated

`scripts/check_help_coverage.py` is the staleness gate that cross-references
`docs/spec/grammar.md` against the `--help` JSON emitted by both compilers, and it is
supposed to catch drift such as a type being dropped from the generated help. Instead
of checking each grammar category against its own field in the JSON (`language.types`,
`language.effects`, `language.builtins`), the primitive/parameterized-type check and the
effect check both search a `flatten_json(lang)` string built from the *entire*
`language` object. A token that is missing from its canonical field but still happens
to appear anywhere else in the language object (e.g. `String` mentioned inside a
builtin signature or a cast note) makes the check pass anyway, silently defeating the
gate. `scripts/full_test.sh` invokes this script for both the Rust and self-hosted
`--help` outputs (`help/coverage-rust`, `help/coverage-self`), so this is a real hole in
CI's drift detection, not a cosmetic issue.

## 2. Files to touch

This is a Python CI-tooling fix, not a language-semantics change — it does not touch
`crates/` or `compiler/`, and per `docs/spec/*.md` policy, no spec file changes are
required because no syntax/semantics/builtin/effect/CLI behavior is being added or
changed; only how a *test script* validates already-generated JSON.

- `scripts/check_help_coverage.py` — narrow each check to its exact field:
  - primitive types + parameterized types → exact membership against
    `lang.get("types", [])` (not a flattened whole-object string)
  - effects → exact membership against `lang.get("effects", [])`
  - builtins → exact key membership against `lang.get("builtins", {})` (currently this
    one is already scoped to the `builtins` subtree, but it still substring-searches a
    flattened blob of `{name: "sig effects"}` pairs rather than checking dict keys
    exactly — tighten it for the same reason, and for consistency)
  - remove the now-dead `flatten_json`/`flat` helpers and variables if nothing else in
    the file needs whole-object flattening after the above changes
- `scripts/test_check_help_coverage.py` (new) — regression tests using the existing
  `scripts/test_*.py` + `unittest` convention (see `scripts/test_ci_docs_only.py` for
  the pattern: a plain script, no package, imported sibling modules resolve because
  the script's own directory is on `sys.path[0]`).
- `.github/workflows/ci.yml` — add one `run: python3 scripts/test_check_help_coverage.py`
  step alongside the sibling `scripts/test_*.py` steps (own step, not `&&`-chained,
  per repo convention already used for every other `test_*.py`).

No changes to `docs/spec/*.md`, `crates/`, or `compiler/`.

## 3. TDD slices

All slices live in `scripts/test_check_help_coverage.py`, built around real fixture
data: call `generate_help.build_help_json(grammar_text, cli_text, contracts_text)`
against the actual `docs/spec/{grammar,cli,contracts}.md`, then mutate the resulting
dict per-test before re-serializing to JSON and invoking
`scripts/check_help_coverage.py` as a subprocess (matching exactly how
`scripts/full_test.sh` invokes it: `<grammar.md> <help-json-string>` as argv, exit code
+ stdout as the observable contract). Using the real generator output as the base
means the fixtures track the grammar automatically and slices don't hand-roll a fake
JSON shape that could drift from the real one.

1. **Scaffold + baseline (no assertion changes to production code yet).**
   Add `scripts/test_check_help_coverage.py` with a helper
   `_build_help_data() -> dict` that reads the three real spec files and calls
   `generate_help.build_help_json`, plus a `_run_checker(grammar_path, help_data) -> subprocess.CompletedProcess`
   helper that dumps `help_data` to JSON and runs
   `[sys.executable, "scripts/check_help_coverage.py", grammar_path, json_str]`.
   Add one test, `test_unmutated_help_passes`, asserting `_run_checker` on the
   unmutated real data returns exit code 0. This test already passes on current
   `main` — it is not the red test, it is the safety net that proves the upcoming
   fix doesn't make the checker overly strict on legitimate data.

2. **RED — missing type hidden elsewhere still passes today.**
   Add `test_missing_type_hidden_in_builtins_is_caught`: take `_build_help_data()`,
   remove `"String"` from `data["language"]["types"]` (leave everything else,
   including any builtin signatures or casts text that mention `String`,
   untouched — those mentions are exactly the "elsewhere" the issue describes).
   Assert the subprocess exits nonzero AND stdout contains `type:String`.
   Run it now: it fails (exit code 0) against the current implementation, because
   the primitive/parameterized-type check searches `flatten_json(lang)`, which still
   contains `String` from the builtins section. This reproduces the issue's own
   repro steps exactly.

3. **GREEN — narrow the type check to `lang['types']`.**
   In `scripts/check_help_coverage.py`, change the primitive-types and
   parameterized-types sections to build `types_list = lang.get("types", [])` instead
   of `types_list = flat`, and check exact list membership (`t not in types_list`)
   instead of substring search. For parameterized types, drop the
   `base = t.split("<")[0]` stripping. This was verified empirically during planning,
   not just inferred: `generate_help.extract_table_col` (backtick-stripping via
   `extract_table`'s `re.sub(r"`([^`]*)`", r"\1", c)`) and the checker's own
   `extract_table_column`/`extract_backtick_value` were both run against the real
   `docs/spec/grammar.md` and produce byte-identical lists —
   `['i8', 'i16', ..., '()', '!']` for primitives and
   `['Vec<T>', 'Option<T>', 'Result<T, E>', 'String', 'HashMap<K, V>', 'BTreeMap<K, V>']`
   for parameterized types, including generic-parameter spelling and spacing. So
   `lang["types"]` (built by `generate_help.py` from the same table) and the
   checker's own `t` values agree exactly; base-stripping was only ever needed to
   make the old substring search of a flattened blob behave, and comparing full
   strings against an exact list is strictly tighter and still correct — no fallback
   to base-name comparison is needed. Re-run slice 2's test: it now passes. Re-run
   slice 1's test: still passes (confirms no false positive introduced on real
   grammar data).

4. **RED then GREEN — same class of bug for effects.**
   Add `test_missing_effect_hidden_in_builtin_signature_is_caught`: remove `"io"`
   from `data["language"]["effects"]`. Because many builtin signatures carry an
   `[io]` effect tag in their value string (e.g. `print_str`, `eprintln_str`), `"io"`
   still appears elsewhere in the language object, so this reproduces the same bug
   for the effects check. Assert nonzero exit + `effect:io` in stdout. Confirm it
   is RED against current `main` (effects check also uses `flat`). Fix: change the
   effects loop to check `e not in lang.get("effects", [])`. Re-run: GREEN.

5. **RED then GREEN — tighten the builtins check to exact key membership.**
   Add `test_missing_builtin_hidden_in_another_signature_is_caught`: pick a real
   builtin name from the generated `language.builtins` dict (e.g. `print_str`),
   delete that key, and inject its name as a substring into a different builtin's
   signature value (e.g. append `" print_str"` to another entry's value string) to
   simulate "the token appears elsewhere" for builtins specifically. Assert nonzero
   exit + `builtin:print_str` in stdout. This is RED against current `main` because
   the current builtins check does `b not in flatten_json(lang.get("builtins", {}))`
   — a substring search over concatenated `{name: "sig effects"}` text, not an exact
   key check, so an injected substring anywhere in that blob (not just as a real key)
   still passes. Fix: change to `b not in lang.get("builtins", {})` (exact dict-key
   membership). Re-run: GREEN. Re-run slice 1: still GREEN.

6. **Cleanup pass.** Remove `flatten_json` and the top-level `flat = flatten_json(lang)`
   / `builtins_flat = ...` variables from `scripts/check_help_coverage.py` — confirmed
   during planning via `grep -rn flatten_json scripts/` that the only references are
   the definition and its two call sites inside this same file (the mention of
   `check_help_coverage` in `scripts/ci_docs_only.py` is a prose comment, not an
   import), and the structural-keys check already does `key not in lang` directly and
   never needed flattening, so deleting it is safe. Re-run the full new test file plus
   a manual end-to-end confirmation against real binaries — this is the expensive
   step flagged in "Risk areas": it requires `cargo build --release -p vow` (Rust) and
   `scripts/bootstrap.sh --skip-cargo` (self-hosted) since `build/vowc` is gitignored
   and won't exist in a fresh worktree, then
   `uv run python scripts/check_help_coverage.py docs/spec/grammar.md "$(./target/release/vow --help)"`
   and the same against `build/vowc --help`, matching exactly what
   `scripts/full_test.sh`'s `help/coverage-rust`/`help/coverage-self` sections do.
   Budget wall-clock/memory for this per the run's build-parallelism constraints
   (cap `-j`/`CMAKE_BUILD_PARALLEL_LEVEL` rather than defaulting to full core count).

7. **Wire into CI.** Add a `run: python3 scripts/test_check_help_coverage.py` step to
   `.github/workflows/ci.yml` next to the other `scripts/test_*.py` steps, as its own
   step (never `&&`-chained, per repo convention and the run's own quality-gate rule).
   Plain `python3` (not `uv run`) is correct here and was verified during planning:
   `scripts/generate_help.py`'s only imports are `hashlib`, `json`, `re`, `sys`,
   `pathlib` (all stdlib), and its script body is guarded by
   `if __name__ == "__main__":` at line 1326, so `import generate_help` from the new
   test file is side-effect-free and needs no virtualenv/uv-managed dependency —
   consistent with every sibling `scripts/test_*.py` step already running under plain
   `python3` in `ci.yml`.

## 4. Verification surface

No contracts, IR, codegen, or C model are touched — this is pure Python test tooling
around a documentation/help staleness gate, so there is nothing for ESBMC to prove and
no `tests/run/` or `examples/*.vow` fixtures are needed. The "tests" here are ordinary
Python `unittest` cases in `scripts/test_check_help_coverage.py`. The closest thing to
an integration check is re-running `scripts/full_test.sh`'s `help/coverage-rust` and
`help/coverage-self` sections locally after the fix (they call the same script against
real `--help` JSON from both binaries) to make sure the tightened checks don't newly
fail on legitimate, non-mutated output — that would indicate the fix is too strict
rather than just precise.

## 5. Risk areas

- **False positives from over-tightening.** The main risk is the fix becoming *too*
  strict — e.g. if the self-hosted compiler's embedded `--help` JSON is ever produced
  by a path that doesn't share `generate_help.py`'s exact table-extraction formatting,
  an exact-membership check could fail where the old substring check happened to be
  forgiving. The empirical check in slice 3 (both extractors run against the real
  grammar, byte-identical output) shows this is not a live risk *today*, but it is a
  risk if `check_help_coverage.py`'s own table parser and `generate_help.py`'s ever
  diverge in the future (they are two independently maintained copies of similar
  logic — see "out of scope" below). Mitigation: slice 1's baseline test plus manually
  running the checker against both real `--help` outputs (Rust and self-hosted) before
  considering the fix done, per CLAUDE.md's instruction that both compilers' `--help`
  are generated from the same script and kept in sync
  (`uv run python scripts/generate_help.py`). This is the expensive confirmation (it
  needs a bootstrap or at least `cargo build --release -p vow`); budget for it in the
  implementation stage rather than skipping straight to CI. `build/vowc` is
  gitignored and will not exist in a fresh worktree, so this step cannot be assumed
  free.
- **No binary-fixed-point or dominance/stack-slot risk.** This change never touches
  `compiler/`, `vow-clif-shim`, or codegen ordering, so the bootstrap triple test and
  `BTreeMap`-determinism concerns in CLAUDE.md do not apply.
- **No `parse → print → parse` idempotency risk.** No AST/printer code is touched.
- **Clippy gate is irrelevant** (Python-only change); the relevant gate instead is the
  pinned `ruff` from `.pre-commit-config.yaml` (`v0.15.14`) — run
  `uvx ruff@0.15.14 check scripts/check_help_coverage.py scripts/test_check_help_coverage.py`
  and `uvx ruff@0.15.14 format --check` on the same files before considering the change
  done, since a newer local ruff can report rules CI never runs.
- **Test fixture fragility.** Building fixtures from the *real* `docs/spec/grammar.md`
  (rather than a hand-rolled fake) means slice 2/4/5 tests could break if someone
  renames `String`, removes the `io` effect, or renames `print_str` in the grammar
  later. That's an acceptable, low-probability maintenance cost given it also means
  the tests exercise real generator output instead of a synthetic shape that could
  silently diverge from `generate_help.py`'s actual structure; picking commonly-stable
  tokens (`String`, `io`, `print_str`) minimizes churn risk.

## 6. Out of scope

- Refactoring `check_help_coverage.py`'s `main()` into smaller testable functions
  (e.g. extracting a `find_missing(grammar, help_data) -> list[str]` helper) beyond
  what's needed to remove now-dead flattening code. Subprocess-based tests already
  give full black-box coverage of the observable contract (exit code + message) that
  CI actually depends on; a larger internal refactor is not necessary to close this
  issue and would bundle unrelated cleanup into a bug fix.
- Reworking `extract_table_col` / `extract_table_rows` / `flatten_json` more broadly,
  or deduplicating them against `scripts/generate_help.py`'s near-identical
  `extract_table_col`/`extract_table` functions. That duplication predates this issue
  and is a separate, larger deep-modules cleanup, not part of this fix.
- Adding coverage-check strictness for grammar sections not named in the issue or its
  recommendation (e.g. auditing `structural keys`, `operators`, `vow_clauses` for the
  same class of bug). The structural-keys check already does exact `key not in lang`
  and isn't affected; operators/vow_clauses aren't part of this finding's evidence or
  recommendation. If a follow-up audit finds the same flattening bug elsewhere, file
  it as a separate issue rather than growing this PR.
- Any change to `docs/spec/*.md`, `crates/`, or `compiler/` — none of grammar,
  semantics, builtins, effects, operators, or CLI flags are changing; only how a CI
  test script validates already-generated JSON against the spec.
