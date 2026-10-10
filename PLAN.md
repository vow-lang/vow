# Plan: point the SKILL.md entrypoint at `skill print --bundle`

## Goal
Add one line to the generated entrypoint `SKILL.md` telling non-Claude-Code / raw-API harnesses to run `build/vowc skill print --bundle` for a self-contained skill document (issue #458, item 5 in `docs/comparisons/index.html`). Docs-only; no language, CLI, or verifier change.

## Assumptions
- Size is Small: one template edit, then regeneration of three checked-in mirrors; no subagents needed.
- Wording uses `build/vowc` (matches the entrypoint's own workflow step 2, not the issue's `vowc`) and is pure ASCII: the self-hosted driver embeds ASCII-only copies of the entrypoint (`tests/skill-install/tests.sh:57` comment), so non-ASCII would break Rust/Vow byte parity.
- Placement: a standalone paragraph after the `## Reference files` bullet list (not a bullet; the list is all links). Exact line:
  `Outside Claude Code (raw API or custom agent harness that cannot load these files): run \`build/vowc skill print --bundle\` for one self-contained document with all of the above inlined.`
- Not touched: `docs/spec/*.md`. `docs/spec/cli.md:107,113` and `README.md:136-146` already document `--bundle`; the entrypoint is built only in `scripts/generate_help.py`, and the bundle is built from `docs/spec/index.md` + spec files, so the bundle content is unchanged.
- The stale "Item 5" row in `docs/comparisons/index.html:722-727` is a historical comparison snapshot; left alone.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `scripts/generate_help.py` | single source: `build_skill_entrypoint()` template | 988-1036 (`## Reference files` list ends at ~1033 with `schemas/`) |
| `skills/vow/SKILL.md` | generated mirror read by `npx skills add vow-lang/vow` | tail of file, after `JSON schemas` bullet |
| `vow/src/skill.rs` | generated `skill_entrypoint_markdown()` inside `GENERATE:SKILL_FULL` block; also hosts unit tests | 1415-1470 (generated), 13631-13705 (tests) |
| `compiler/main.vow` | generated `skill_entrypoint()` (single `String::from` literal) | 2691-2692 |
| `tests/skill-install/tests.sh` | dual-compiler shell harness (Section 4k of `scripts/full_test.sh:1625`) run for Rust and self-hosted binaries | `run_in` helper ~L50; end-of-file ~L195-225 |

Do not hand-edit the three generated files; regenerate.

## Steps

### 1. RED: add failing tests first
- **Rust unit test** in `vow/src/skill.rs` tests module (near `skill_install_writes_concise_entrypoint_and_support_files`, ~L13645): assert `entrypoint_markdown()` contains `skill print --bundle`. Fails until step 2+3.
- **Dual-compiler shell test** in `tests/skill-install/tests.sh` (before the final `failures` check): `run_in "$(new_project)" skill print`; `expect "print exit" "$rc" "0"`; assert `$out` contains `skill print --bundle` via the file's existing `case ... *"..."*) ;; *) fail` idiom. Covers both compilers because the harness runs with `VOWC_KIND=rust` and `self`.
- Run: `cargo test -p vow skill::tests -j2` (expect the new test red).

### 2. GREEN: edit the template
- **File**: `scripts/generate_help.py` in `build_skill_entrypoint()`, after the `"- JSON schemas: [schemas/](schemas/)",` entry.
- **Change**: append `""` then the single line above (keep the trailing `""` that gives the file its final newline).

### 3. Regenerate mirrors
- `uv run python scripts/generate_help.py` updates `vow/src/skill.rs`, `compiler/main.vow`, `skills/vow/SKILL.md`.
- `git diff --stat` must show exactly those three files plus the step-1 test files and `scripts/generate_help.py`; the generated diffs should be the one added line only (confirms no unrelated drift in `--help` JSON).
- `python3 scripts/generate_help.py --check` and `python3 scripts/check_help_coverage.py` must pass.

### 4. Verify
- `cargo test -p vow skill::` (includes `checked_in_skills_vow_matches_install_output`, `<500 lines`, no ```` ```json ```` inlining).
- `cargo build --release -p vow` then `VOWC_BIN=target/release/vow VOWC_KIND=rust bash tests/skill-install/tests.sh`.
- `scripts/bootstrap.sh --skip-cargo` then `bash tests/skill-install/tests.sh` (self-hosted `build/vowc`). Run in the foreground with an explicit timeout (bootstrap ~5 min); use `VOW_CACHE_DIR=$(mktemp -d)` to avoid the stale compile cache.
- `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all --check` (only test code added in Rust).

### 5. Ship
- Commit type `docs(skill)`; subject lower-case, e.g. `docs(skill): point entrypoint at skill print --bundle` (<92 chars incl. ` (#N)`).
- `git rm PLAN.md` before opening the PR (plan is a stage-handoff artefact).

## Testing
- New: Rust unit assertion + dual-compiler shell assertion (step 1).
- Existing guards that must stay green: `checked_in_skills_vow_matches_install_output`, `scripts/full_test.sh` `help/skills-dir-drift` (~L2007), `ops/catalogue-drift`, bootstrap fixed point.

## Verification surface
No contracts, codegen, or C-model change; ESBMC and `c_emitter.{rs,vow}` parity are unaffected. No new `tests/run/` or `examples/` fixtures. The only compiled-code change is a longer string literal in `compiler/main.vow`.

## Risks
- Non-ASCII in the line -> Rust vs self-hosted embedded-copy mismatch; mitigated by ASCII-only wording (no em-dash, no curly quotes).
- Raw-string delimiter in `skill.rs`: `_rust_raw_string` picks `r#"..."#` and the line contains backticks only, no `"#`; safe.
- Binary fixed point: only a data literal changes; bootstrap triple test still expected to converge. Record head SHA if claiming green.
- Entrypoint size (`lines().count() < 500`): +2 lines, far under.
- Generator may rewrite unrelated regions if the checked-in generated blocks were already stale; step 3's `git diff --stat` check catches this, and unrelated drift must be split out rather than bundled.
- Known env flakes (vow e2e SKIP-panic, `u64_marker_propagation`, `contracts_tmp_cleanup`) reproduce on clean main; verify before blaming this change.

## Out of scope
- Any change to bundle contents, `--bundle` behaviour, or `skill install`.
- Item 6 ("Do nots" page) and other `docs/comparisons` roadmap items.
- Editing `docs/comparisons/index.html` or README prose.
- Refactors/formatting of `generate_help.py` or the generated blocks.
