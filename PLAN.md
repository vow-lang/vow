# Plan: clarify and pin migration for existing monolithic Vow skill installs (#358)

## Goal
Make the migration path from the old monolithic `.claude/skills/vow/SKILL.md` to the split layout
(`SKILL.md` + `reference/`, `examples/`, `schemas/`) an explicit, documented, tested contract —
without changing install behaviour or adding CLI surface.

## Assumptions
- **Path history: the dir was renamed.** The issue text says `.claude/skills/vow-toolchain/`; that
  was the install path until commit `7c26ceba` (#445, "rename to `vow`"). Both compilers now use
  `.claude/skills/vow/` (`vow/src/skill.rs:41,160`, `compiler/main.vow:16005-16009,16157`). So
  there are two legacy cases, both handled by documentation only: (1) monolithic `SKILL.md` in
  `skills/vow/`; (2) a skill dir at `.claude/skills/vow-toolchain/` from before the rename, which
  auto-install ignores (it would add a fresh `skills/vow/` next to it, leaving two skills that
  both match `.vow` work). Guidance: after `vow skill install`, delete the stale
  `.claude/skills/vow-toolchain/` directory by hand. The compiler never deletes it. (verified in
  git history)
- **Chosen option: (a) document + (c) test; reject (b).** The explicit `vow skill install
  --local|--global` already *is* the opt-in refresh path: it unconditionally rewrites `SKILL.md`
  and every support file. A new detection/refresh flag would add a CLI axis to both compilers, a
  heuristic for "old layout" (content sniffing a user-editable file), and silent-overwrite risk,
  for a monolithic file that is self-contained and still works. Does not meet the CLAUDE.md bar
  of "smallest surface that closes the issue". (best guess)
- **Auto-install semantics unchanged:** skipped whenever `.claude/skills/vow/SKILL.md` exists,
  whatever its content. This also means a *partial* split install (new `SKILL.md`, support files
  deleted) is not healed by auto-install; documented as "run `vow skill install` to repair", not
  changed. (best guess)
- **No version-staleness handling.** A split-layout install from an older compiler is equally
  skipped by auto-install; the same "re-run explicit install" guidance covers it. Version stamping
  is a separate feature; listed under Out of scope.
- Both explicit-install overwrite semantics are intentional: user edits to `SKILL.md` and
  support files under the skill dir are overwritten by explicit install (that is what opting in
  means). Files in the dir the compiler does not own are left alone.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `docs/spec/cli.md` | Authoritative `vow skill` spec; add migration paragraph | 104-119 (`### vow skill`, "Auto-install on build") |
| `README.md` | User-facing install section; add one-paragraph migration note | 92-120 |
| `vow/src/skill.rs` | Generated `SKILL_FULL` cli.md copy (regenerated, not hand-edited) + `mod tests` (new tests) | 9175-9185 (generated), 13660-13690 (auto-install tests), 13499-13527 (install-tree test) |
| `compiler/main.vow` | Generated copy of cli.md text in both `skill_*` blocks (regenerated) | 5600-5611, 11666-11677; behaviour 16005-16162 (unchanged) |
| `skills/vow/reference/cli.md` | On-disk mirror of cli.md (regenerated; drift-gated) | 108-119 |
| `scripts/generate_help.py` | Regenerates the three copies above; run, do not edit | 1051, 1199-1289 |
| `tests/skill-install/tests.sh` `[new]` | Cross-compiler behavioural fixture (mirrors `tests/cli-flags/tests.sh`) | n/a |
| `scripts/full_test.sh` | Wire new fixture for both compilers next to Section 4h | 1526-1543 |

## Steps

### 1. Pin behaviour first with Rust unit tests (red/green on the contract, no production change)
- **File**: `vow/src/skill.rs` `mod tests`, next to `auto_install_skill_leaves_existing_file_untouched` (~13675)
- **Change**: add tests using `TempDir` + existing `maybe_auto_install` / `run_skill_install_scoped`:
  1. `auto_install_skill_does_not_add_support_files_to_monolithic_install` — write monolithic
     `SKILL.md` ("user-managed content"), run `maybe_auto_install`, assert `SKILL.md` bytes
     unchanged **and** `reference/`, `examples/`, `schemas/` do not exist.
  2. `explicit_install_migrates_monolithic_skill_to_split_layout` — pre-write monolithic
     `SKILL.md`, call `run_skill_install_scoped(.., local=true, ..)` (with `.claude` + `.git`
     fixture as in `skill_install_local_writes_into_current_git_project`), assert `SKILL.md ==
     entrypoint_markdown()` and every `skill_support_files()` path exists with embedded contents.
  3. `explicit_install_repairs_missing_support_files` — install, delete `reference/`, re-install,
     assert restored; also assert an unrelated user file in the skill dir survives.
- **Reuses**: `install_skill_tree_to` (`skill.rs:40`), `skill_support_files` (`skill.rs:7437`),
  `entrypoint_markdown`.
- **Expectation**: all three pass immediately (they document existing behaviour); if any fails the
  premise of this plan is wrong and the plan must be revisited before any doc text is written.

### 2. Cross-compiler behavioural fixture
- **File**: `tests/skill-install/tests.sh` `[new]`, modelled on `tests/cli-flags/tests.sh` (same
  `VOWC_BIN`/`VOWC_KIND` env contract, `fail`/`expect` helpers, trap cleanup).
- **Change**: in a `mktemp -d` project: `mkdir .claude/skills/vow`, write a monolithic
  `SKILL.md`, copy `examples/hello.vow`, run `"$VOWC_BIN" build --no-verify -o out hello.vow`
  (triggers auto-install; `cd` into the temp dir, absolute binary path). Assert: `SKILL.md`
  unchanged, no `reference/` dir. Then `"$VOWC_BIN" skill install --local` (needs `.git`
  — `mkdir .git`/`git init -q`), assert exit 0, `SKILL.md` starts with `---\nname: vow\n`, and
  `reference/cli.md`, `examples/examples.md`, `schemas/build-result.schema.json` exist. Also
  assert a fresh dir with `.claude/` but no skill dir gets the full split tree from auto-install.
  Assert on file state only: auto-install runs before frontend/link in both drivers
  (`vow/src/main.rs:859,1075`, `compiler/main.vow:16715,16740`), so a sandbox link failure cannot
  skip it — do not gate the checks on the build's exit code. Plain `mkdir .git` satisfies
  `--local`'s `.git` existence check (no `git init`). Also add a case: `.claude/skills/vow-toolchain/
  SKILL.md` present, no `skills/vow/` → auto-install still creates `skills/vow/` and leaves the
  legacy dir untouched (pins the documented behaviour).
- **Why both compilers**: `compiler/main.vow:16153` and `skill.rs:155` are independent
  implementations of the same contract; CLAUDE.md requires parity.

### 3. Wire the fixture into the CI-gating harness
- **File**: `scripts/full_test.sh` after Section 4h (line ~1543)
- **Change**: copy the `cli-flags` rust/self-hosted `pass`/`fail` block for `skill-install`
  (`skill-install/rust`, `skill-install/self-hosted`), same log-tail-on-fail pattern.
- **Reuses**: `$RUST`, `$SELF`, `$TMPDIR` already defined there.

### 4. Document the migration in the spec
- **File**: `docs/spec/cli.md` (end of the "Auto-install on build" paragraph, line 119)
- **Change**: add a short **Migrating an existing install** paragraph stating, normatively:
  auto-install never touches an existing `.claude/skills/vow/SKILL.md` — including the old
  single-file layout, whose content is self-contained and keeps working; it does not add the
  `reference/`, `examples/`, `schemas/` files. To move to the split layout (or to refresh after a
  compiler upgrade, or to repair a skill dir with missing support files) run
  `vow skill install --local` or `--global`; explicit install rewrites `SKILL.md` and all support
  files, overwriting local edits to them, and leaves other files in the directory alone.
  Project installs are usually committed — review the diff after refreshing. Installs made
  before the rename (`.claude/skills/vow-toolchain/`) are not touched or detected; after
  installing, delete that directory by hand so only one Vow skill remains.
- Keep wording line-compatible with the existing paragraph style (single long paragraph, no new
  headings) so the generated Vow `push_str` lines stay one-line-per-source-line.

### 5. README note
- **File**: `README.md` (after the install paragraph ending line ~120)
- **Change**: 2-3 sentences mirroring Step 4 and pointing at `vow skill install --local|--global`.
  README is not generated, so edit by hand.

### 6. Regenerate embedded docs in both compilers
- **Command**: `uv run python scripts/generate_help.py` (updates `vow/src/skill.rs`,
  `compiler/main.vow` `GENERATE:SKILL_*` blocks, and `skills/vow/reference/cli.md`), then
  `uv run python scripts/generate_help.py --check` (or the drift mode `full_test.sh:1946-1951`
  uses) to confirm in sync.
- **Do not hand-edit** inside `// GENERATE:` markers. Expected diff: only the one new paragraph
  in each of the three copies (one per generated copy of cli.md in `compiler/main.vow`, currently 2 at ~5611 and ~11677).
- Then rebuild per CLAUDE.md: `cargo build --release -p vow`, `scripts/bootstrap.sh --skip-cargo`.
  Because `compiler/main.vow` changed text only, the fixed point must still hold.

## Testing
- `cargo test -p vow skill::` (new unit tests + existing `skills/vow/` drift test at
  `skill.rs:13530-13556`).
- `VOWC_BIN=target/release/vow VOWC_KIND=rust bash tests/skill-install/tests.sh` and
  `VOWC_BIN=build/vowc bash tests/skill-install/tests.sh`.
- `uv run python scripts/generate_help.py --check`, `python3 scripts/check_help_coverage.py`.
- `cargo fmt --all`, `cargo clippy --all --all-targets -- -D warnings`.
- `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA before ticking any
  "bootstrap green" checklist item (CLAUDE.md rule; record the SHA).
- Run long gates in the foreground with explicit timeouts (bootstrap ~5 min, full_test ~40 min
  — run only the relevant section if the budget does not allow the whole script).

## Verification surface
No contracts, codegen, IR or C-model changes → no new ESBMC obligations, `vow-verify/src/c_emitter.rs`
/ `compiler/c_emitter.vow` parity untouched, no `tests/run/` or `examples/` fixture growth. The only
`compiler/` change is generated string text inside `compiler/main.vow`.

## Risks
- **Binary fixed point**: `compiler/main.vow` string-literal growth changes the binary, but both
  stages compile the same source, so the stage-2/3 fixed point is unaffected; confirm with the
  bootstrap run. Only generator output may touch those blocks.
- **Help-drift gates**: `check_help_coverage.py` and `skills/vow/` drift tests fail if only some
  of the three copies are regenerated — always run the generator, never patch by hand.
- **Stale compile cache** (memory: vow compile cache ignores compiler changes): use a fresh
  `VOW_CACHE_DIR=$(mktemp -d)` when validating the self-hosted fixture after rebuild.
- **Test isolation**: the shell fixture must `cd` into its temp project and must never write to
  the real `$HOME` — only use `--local`, never `--global`, in the shell test; the Rust global
  test already injects `home`.
- **Packaged toolchain**: `scripts/package-toolchain.sh` and `tests/install_toolchain/` do not
  reference the skill, so no installer-driven migration path exists to document.
- **Auto-install during unrelated tests**: `vow build` in a cwd containing `.claude/` triggers
  it; the fixture's temp dir is the only place it is created, so the repo's own `.claude/` is safe.
- **Commit hygiene**: commit/PR title must be lower-case Conventional Commits, e.g.
  `docs(skill): document migrating monolithic skill installs to the split layout`
  (`docs` type; adds tests but no behaviour change). Remove `PLAN.md` (`git rm`) before the PR.
- **codecov/patch**: new Rust code is test-only and the doc edits carry no executable lines;
  no coverage exposure.

## Out of scope
- Any new flag, prompt, or automatic old-layout detection/refresh (option (b)).
- Making auto-install heal partial or version-stale installs; skill version stamping.
- Removing/renaming files in the skill dir, or changing which files `install` writes.
- Changing `--local` requirements (`.git` + `.claude/`) or auto-install gating.
- Refactoring `install_skill_tree_to` / `install_skill_tree`, formatting, unrelated spec cleanup,
  or fixing the `--help` JSON's missing `command_details["skill"]` (docs/audit finding).
