# Plan: canonical PascalCase `blame` on every JSON surface (#649)

## Goal
Emit `"Caller"` / `"Callee"` / `"None"` for `blame` in build/verify diagnostics and counterexamples (Rust + self-hosted), matching `vow contracts` and the runtime `VowViolation` (public schema `["Caller","Callee"]`). One casing, no per-surface normalisation for agents.

## Assumptions
- Canonical = PascalCase (issue's recommended fix; runtime schema is already a public contract): best guess.
- Counterexample "no blame" value becomes `"None"` (was `"none"`). Diagnostics still *omit* `blame` when none (shape unchanged; schema enum stays `["Caller","Callee"]`).
- This is a wire-format change for consumers of `vow build/verify` JSON. Land as `fix(diag): emit PascalCase blame on every JSON surface` (<=100 chars, lower-case subject); describe the wire change in the PR body. Implementer: glance at the release config; if it maps 0.x breaking changes to a major bump, keep plain `fix` anyway (pre-1.0 JSON polish) and note it in the PR.
- Historical docs (`docs/audit*/`, `docs/audits/`, `docs/level5-test-trace.md`) are snapshots and are NOT edited.
- Internal enum `Blame`/`DIAG_BLAME_*` ints are untouched; only strings change. `vow/src/cex_eval.rs:285` and `counterexample.rs:714/1143/1178` use "caller"/"callee" as *function names* - leave alone.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `vow/src/report.rs` | diagnostic JSON blame producer + unit tests | 40-42, 386-463 |
| `vow/src/counterexample.rs` | `resolve_ce_blame` returns blame string; `blame == "caller"` checks; unit tests | 175-185, 266, 296, 939-2045 |
| `vow/src/verify_outcome.rs` | `blame_to_error_code`, `blame_to_diag_blame` parse the CE string | 285-297 |
| `vow/src/main.rs` | StructuredCounterexample fixtures/tests with `blame: "caller"`; JSON assertion | 2142, 2602, 2695, 2729, 2880, 2907, 3573, 3602, 3694 |
| `vow/src/replay.rs` | compares runtime blame to `ce.blame` via `eq_ignore_ascii_case`; test fixture | 473, 701 |
| `vow/src/contracts.rs` | already PascalCase; test fixtures use `"callee"` strings for CE | 319, 364, 397 |
| `compiler/diag.vow` | `diag_blame_name` (Pascal) vs `diag_blame_name_json` (lower); diag JSON 495-497; CE JSON 659-664 | 273-291, 495, 659 |
| `compiler/tests/test_diag_verify_json.vow` | asserts `"blame":"callee"` | 56, 63 |
| `docs/spec/cli.md` | blame examples/prose | 383, 399, 435 |
| `docs/spec/contracts.md` | `blame: "caller"` / `"callee"` | 138, 450 |
| `docs/spec/schemas/counterexample.schema.json` | enum `["caller","callee","none"]` | 45 |
| `docs/spec/schemas/diagnostic.schema.json` | enum `["caller","callee"]` | 78 |
| `skills/vow/**`, `compiler/main.vow`, `vow/src/skill.rs` | generated copies of spec/schemas | via `scripts/generate_help.py` |
| `tests/**/*.vow` (89 fixtures: 49 callee, 7 caller, 33 none), incl. `tests/verify-native/fail/` | `// TEST: counterexample-blame <lower>` | - |
| `tests/run_tests.sh`, `scripts/full_test.sh` | exact-match blame directive against JSON | run_tests.sh 218-220, 750; full_test.sh 1351-1366 |
| `scripts/verify_eval.py` | `VALID_BLAME`, `.lower()`, `(blame or "none").lower()`, `cex blame=` directive, docs | 66, 113-121, 191-198, 221-222, 286-290 |
| `scripts/test_verify_eval.py`, `test_verify_diff.py`, `test_parity.py`, `test_equivalence.py` | fixtures mirroring wire shape | see grep |
| `tests/verify-fail/const_u64_requires_caller.vow` | `counterexample-fn "caller"` is a fn name - leave | 2 |

## Steps (TDD slices; each slice ends green on its own)

### 1. Rust diagnostic blame -> PascalCase (red/green)
- **Red**: in `vow/src/report.rs` tests (393, 412) change expectations to `Some("Caller")`/`Some("Callee")`.
- **Green**: `report.rs:40-41` produce `"Caller"`/`"Callee"`. Better (deep, single source): add `pub fn Blame::as_str(self) -> &'static str` (`"Caller"|"Callee"|"None"`) in `vow-diag/src/lib.rs` (exhaustive match) and use it here and in `contracts.rs:220-224` (replacing its local match) and `human emitter` line 220 is `{:?}` - leave. Unit test in `vow-diag` for `as_str`.

### 2. Rust counterexample blame -> PascalCase
- **Red**: update `counterexample.rs` tests (`sce.blame == "Caller"/"Callee"/"None"`, `resolve_ce_blame` asserts at 2035-2045, `main.rs:2907` -> `"Caller"`) and fixture strings in `report.rs:436/463`, `main.rs` (2142, 2602, 2695, 2729, 2880, 3573, 3602, 3694), `replay.rs:701`, `contracts.rs:319/364/397`.
- **Green**: `resolve_ce_blame` returns `Blame::as_str()` results (`"Caller"` for caller_precondition; `None|None` -> `"None"`); `counterexample.rs:266,296` compare to `"Caller"`; `verify_outcome.rs:285-297` match `"Caller"`/`"Callee"`. Keep the documented fallback asymmetry comment accurate.
- `replay.rs:473`: change `eq_ignore_ascii_case` to `==` (both sides now PascalCase) and keep `parse_vow_violation_line` test; this makes replay a strict check.
- Run `cargo test -p vow` (build/vow CLI integration tests asserting JSON, e.g. `vow/tests/*.rs`; grep `"blame"` there once more and fix any lowercase expectation).

### 3. Self-hosted compiler (dual-compiler rule)
- **Red**: `compiler/tests/test_diag_verify_json.vow:56,63` -> `\"blame\":\"Callee\"` (and add a case for caller `"Caller"` and CE-none `"None"` if not covered).
- **Green**: `compiler/diag.vow`: delete `diag_blame_name_json` (273-291 region), use `diag_blame_name` at 497 and 661; replace the `else { "none" }` at 659-664 with `"None"` (`diag_blame_name` already returns `"None"` for NONE, so the branch collapses to a single call). Diagnostic omission when `d.blame == DIAG_BLAME_NONE()` stays.
- `build/vowc test compiler/tests/test_diag_verify_json.vow` (and `test_verify_report.vow`).

### 4. Spec, schemas, generated copies
- Edit sources only: `docs/spec/cli.md` (383, 399, 435, and the field table: state canonical `Caller|Callee|None`; diagnostics omit `blame` when none), `docs/spec/contracts.md` (138, 450), `docs/spec/schemas/counterexample.schema.json` (enum `["Caller","Callee","None"]`), `docs/spec/schemas/diagnostic.schema.json` (enum `["Caller","Callee"]`). Add one sentence in `cli.md` / `errors.md` blame section: "`blame` is `Caller`/`Callee`(/`None` in counterexamples) on every surface: diagnostics, counterexamples, `vow contracts`, runtime VowViolation."
- Run `uv run python scripts/generate_help.py`; confirm `git diff --stat` shows `skills/vow/**`, `compiler/main.vow`, `vow/src/skill.rs` updated; if `skills/vow/schemas/*` are not generated, copy by hand (they must stay identical to `docs/spec/schemas/`). Then `python3 scripts/check_help_coverage.py`.

### 5. Fixtures + harnesses (strict, no case-folding)
- Mechanical: `sed -i 's|^// TEST: counterexample-blame callee$|... Callee|'` etc. over `tests/` (caller->Caller, callee->Callee, none->None). Commit as part of this slice only.
- `tests/run_tests.sh` / `scripts/full_test.sh`: no logic change needed (exact compare); update comments if they cite lowercase values.
- `scripts/verify_eval.py`: `VALID_BLAME = {"Caller","Callee","None"}`; drop `.strip().lower()`/`val.lower()` (a lowercase directive must now be a load-time error - fail closed); `actual_cex` uses `blame or "None"` without `.lower()`; update usage strings (`<Caller|Callee|None>`) and docstring at line 103. Update `scripts/test_verify_eval.py`, `test_verify_diff.py`, `test_parity.py`, `test_equivalence.py` fixtures and add a test that a lowercase directive is rejected.
- `scripts/cli_compat_test.sh:69` compares raw rust vs self - no change.

### 6. Cross-compiler guard (regression pin)
- Add/extend a `scripts/test_parity.py`-style or `vow/tests` integration test (or a `scripts/full_test.sh` check over `tests/verify-fail/*.vow`) asserting every `blame` value in verify JSON from both compilers is in `{"Caller","Callee","None"}` - this is the "one canonical casing" invariant. Prefer extending the existing `counterexample-blame` pass (already runs both compilers) rather than new machinery.

## Testing / verification
- `cargo test -p vow-diag -p vow`, `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all`.
- `build/vowc test compiler/` (or the two touched files) after `scripts/bootstrap.sh --skip-cargo`; then `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA (record SHA in PR).
- `python3 -m unittest` for touched `scripts/test_*.py` (or project's pytest invocation); pinned ruff on changed Python.
- `scripts/full_test.sh` (~40 min, background + poll, VOW_FULL_TEST_SKIP_CARGO=1 allowed) covering verify-fail parity and `generate_operations.py --check`.
- Manual: `build/vowc verify tests/verify-fail/<caller fixture>` and `target/release/vow` on the same file: all `blame` fields PascalCase.

## Risks
- **Hidden consumers of lowercase strings**: `verify_outcome.rs` re-parses the CE blame string; `bench/`, `scripts/verify_diff.py`, `scripts/equivalence.py`, `scripts/parity.py` read it. Mitigation: final `git grep -n -i '"caller"\|"callee"\|"none"'` sweep over code (excluding fn-name uses and historical docs) before finishing.
- **Verify cache** (`vow/src/cache.rs`) stores failures; confirm `CachedFailure` does not persist the CE blame string. If it does, bump the cache format/key version so stale lowercase entries are not served (verified: `cache.rs` has no blame field, so N/A).
- **Generated-file drift**: forgetting `generate_help.py` fails `check_help_coverage.py` / CI.
- **C parity / fixed point**: no emitter, IR, codegen or lowering change; `diag.vow` change only affects JSON strings. `diag_blame_name_json` removal must leave no callers (`git grep`). Bootstrap fixed point should be unaffected; still re-run.
- **Fixture sed scope**: restrict to `// TEST: counterexample-blame` lines so fn-name directives (`counterexample-fn "caller"`) are untouched.
- Clippy test-target gate applies to the `vow` unit-test edits.

## Out of scope
- Renaming internal `Blame`/`DIAG_BLAME_*`, adding `hints`/`secondary`/`blame` to schemas beyond the enum casing (separate audit items), dedupe of `replay_blame_runtime_name` in `compiler/main.vow:686` with `diag_blame_name`, the dead caller-blame static-path finding, editing historical audit docs, other casing normalisation (`severity`, statuses), release/versioning changes.
