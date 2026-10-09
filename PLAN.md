# Plan: derive `-o, --output` help default from cli.md (issue #592)

## Goal
`--help` JSON/human text must report the real default (`build/<stem>`) for `vow build -o, --output`. Remove the hardcoded `output_default` strings in `scripts/generate_help.py` so the default is read from the `docs/spec/cli.md` option-table row and cannot drift again. Both compilers pick the fix up via regeneration.

## Assumptions
- Take the issue's "better" option (derive from the cli.md row) as the fix, applied to both `-o` call sites (build and decl); the minimal one-string edit would leave the same drift trap in the decl row (best guess).
- Issue's file refs are stale: the embedded help lives in `vow/src/skill.rs` (not `vow/src/main.rs`) and `compiler/main.vow` ~2605/2610/3110/3535 (not 1426-1431). These are generated blocks; never hand-edit them.
- The `vow decl` default (`<source>.vow.d`) is correct today (cli.md:192); the derived path must leave its generated output byte-identical.
- No spec change: `docs/spec/cli.md:18` already says `build/<stem>`; `skills/vow/reference/cli.md` already matches. The only drifted artefacts are the generated blocks.
- Language semantics unchanged, so no Rust/self-hosted logic change is needed. Dual-compiler rule is satisfied because `generate_help.py` rewrites both `vow/src/skill.rs` and `compiler/main.vow` in one run.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `scripts/generate_help.py` | generator; fix site | `normalize_option` 95-163 (`output_default` param, `-o` branch 113-116, 158-159); build call 228-246 (hardcode at 240); decl call 260-270 (hardcode at 267) |
| `docs/spec/cli.md` | source of truth | build row line 18 (`build/<stem>`), decl row line 192 (`<source>.vow.d`) |
| `vow/src/skill.rs` | generated (SKILL_JSON/SKILL_HUMAN blocks) | 234, 239, 739, 1164 |
| `compiler/main.vow` | generated (same markers) | 2605, 2610, 3110, 3535 |
| `scripts/test_check_help_coverage.py` | pattern for generator tests (imports `generate_help`, builds real help data) | 20-40 |
| `.github/workflows/ci.yml` | lists python unit-test steps | 100-101 |
| `scripts/full_test.sh` | `help/skills-dir-drift` gate runs `generate_help.py --check` | 1813-1818 |
| `vow/src/main.rs` | real default, no change | 499-504 |
| `compiler/main.vow` `default_output` | real default, no change | 94-115 |

## Steps (TDD slices)

### 1. RED: add generator test [new]
- **File**: `scripts/test_generate_help.py` [new], modelled on `scripts/test_check_help_coverage.py` (sys.path insert, `import generate_help`, read real `docs/spec/*.md`).
- **Tests**:
  - `test_build_output_default_is_build_stem`: `build_help_json(...)` -> the `-o, --output <path>` entry of the build command options has `default == "build/<stem>"` and its description ends `(default: build/<stem>)`. Find the entry via the same structure the generator emits (inspect the dict at implementation time: `build_option_entries`).
  - `test_output_default_tracks_cli_md`: replace the `build/<stem>` cell in the cli.md text with a sentinel (e.g. `` `out/<stem>.bin` ``), rebuild, assert the sentinel appears in the option default and description. This pins the "derived, not hardcoded" property.
  - `test_decl_output_default_unchanged`: decl `-o` default is `<source>.vow.d`.
- Run with `python3 scripts/test_generate_help.py`; first two fail (current output is `source without .vow extension`, sentinel ignored).

### 2. GREEN: derive the default from the table row
- **File**: `scripts/generate_help.py`
- **Change**: in `normalize_option`, drop the `output_default` parameter. In the `-o, --output` branch, use the already-stripped `default` cell: `description = f"{desc} (default: {default})"` when `default` is non-empty, and set `option["default"] = default` (replace the `elif output_default is not None` branch at 158-159; keep branch order so `--mode`/`--max-k-step` are unaffected). Remove the `output_default=` kwargs at lines 240 and 267.
- Keep comments minimal (project style).

### 3. Regenerate and rebuild
- Commands (separate invocations, not `&&`-chained): `uv run python scripts/generate_help.py`, then `uv run python scripts/generate_help.py --check`.
- Expected diff: only the four `skill.rs` and four `compiler/main.vow` lines above change `source without .vow extension` -> `build/<stem>` (incl. JSON `"default"`). `skills/vow/` mirror should be unchanged (skill text embeds cli.md, which is unmodified); confirm with `git status`.
- `git diff --stat` must show no other regenerated drift; if unrelated drift appears, stop and split it out.
- Then `cargo build --release -p vow -j2` and `scripts/bootstrap.sh --skip-cargo` (long-running; foreground with explicit bounded timeout, or poll; ~5 min).

### 4. Wire the test into CI
- **File**: `.github/workflows/ci.yml` after line 101: add step "Help generator unit tests" running `python3 scripts/test_generate_help.py`, matching neighbouring steps.

### 5. End-to-end check against both binaries
- `./target/release/vow --help` and `build/vowc --help` (JSON): build `-o` option `default` == `"build/<stem>"`; `--help --human` text shows `(default: build/<stem>)`; `vow decl` default still `<source>.vow.d`.
- Actually build `examples/divide.vow` without `-o` with both compilers and confirm the binary lands at `build/divide`.

## Testing
- `python3 scripts/test_generate_help.py` (new), `python3 scripts/test_check_help_coverage.py` (existing, must stay green), `uv run python scripts/generate_help.py --check`, `uv run python scripts/check_help_coverage.py docs/spec/grammar.md "$(build/vowc --help)"`.
- `cargo test -p vow` (skill.rs has a test asserting `skills/` mirror matches embedded skill; `vow/tests/agent_level1.rs` parses `--help` JSON). Per memory notes, ~8 run tests may fail in sandbox for environmental reasons; compare against clean origin/main before blaming this change.
- `cargo clippy --all --all-targets -- -D warnings` and `cargo fmt --all --check` (no Rust logic changed; guards the regenerated literal).
- Run `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA and record the SHA in the PR (CLAUDE.md "green locally" rule), since `main.vow` changes. Full `scripts/full_test.sh` (~40 min) only if time permits; the relevant gates are `help/skills-dir-drift` and the help coverage checks.

## Verification surface
- No contract, codegen, or C-model change; ESBMC and `c_emitter.{rs,vow}` parity untouched. No new `tests/run/` or `examples/` fixtures needed. Only string literals inside generated `skill_json`/`skill_human` bodies change.

## Risks
- Binary fixed point: `compiler/main.vow` string literals change, so the stage-1/stage-2 binaries differ from the previous commit but must still match each other; bootstrap confirms.
- Generated-block drift: hand edits to `skill.rs`/`main.vow` would be overwritten and fail `--check`; only regenerate.
- Table cell parsing: `default` cell is `` `build/<stem>` ``; `normalize_option` already strips backticks. A cell with `|` would need `\|` escaping; none here.
- Description rule: for `-o` the description now always appends the default; the `elif ... not desc.endswith(")")` branch is not used for `-o`. Verify decl description output is byte-identical (`Output declaration file path (default: <source>.vow.d)`).
- Stale audit ref: `docs/audit-20260610/vow-analysis.md` mentions the old string; it is historical, leave untouched.
- `parse -> print -> parse` idempotency, `BTreeMap`/stack-slot ordering, verifier C parity: not affected.

## Out of scope
- Rewriting how other options' defaults are derived, refactoring `generate_help.py`, or changing any default-output behaviour.
- Hand-editing generated blocks, spec/doc rewording, closing audit-doc staleness.
- Making `vow decl`/other subcommands' help reflect anything beyond the existing cli.md rows.

## PR notes
- Title (conventional, lowercase): `fix(help): report build/<stem> as the default for -o in --help`.
- Implementation stage must `git rm PLAN.md` before opening the PR.
