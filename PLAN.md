# Plan: Make `vow skill install` / auto-install atomic (issue #361)

## Goal
Make both skill installers (Rust `vow/src/skill.rs`, self-hosted `compiler/main.vow`) build the whole
`vow` skill tree in a staging directory next to the target and commit it by rename, so no failure
can leave a `SKILL.md` that links to missing `reference/`, `examples/`, `schemas/` files. Auto-install
stays silent on failure and never fails the build.

## Findings that shape the design
- Today both installers write `SKILL.md` first, then support files (`vow/src/skill.rs:40-67`,
  `compiler/main.vow:3162-3205`). Auto-install is guarded on `SKILL.md` existing
  (`skill.rs:160-163`, `main.vow:3300-3303`), so a half-written tree is **never repaired** by the next
  build either. Writing `SKILL.md` last fixes that for free.
- `fs_rename` is `std::fs::rename` (`vow-runtime/src/lib.rs:4259`): on Linux/macOS dir-over-dir
  succeeds only onto an *empty* directory, so re-installing over an existing non-empty tree needs a
  two-rename swap with a backup. `fs_mkdir` is `create_dir_all` (`lib.rs:4093`, succeeds if the dir
  exists, so it cannot be used for exclusive reservation). `fs_remove_dir_all` unlinks a top-level
  symlink without following it (`lib.rs:4193`).
- Self-hosted Vow has no exclusive-create builtin and no `getpid`; `skill_home_dir` already shells out
  via `process_run` (`main.vow:3209`), so spawning `mkdir` has precedent. A new builtin would touch the Operation Catalogue,
  both compilers, runtime and spec — out of proportion for this issue.
- Stage must be a *sibling of the target* inside `.claude/skills/` so rename never crosses a
  filesystem even when `.claude/skills` is a symlink (stow/dotfiles setups). Claude Code scans
  `.claude/skills/*/SKILL.md`; a dot-prefixed stage whose `SKILL.md` is written last is never
  recognised mid-build.
- Existing tests: `vow/src/skill.rs` `mod tests` (from line 13477; `tempfile` is a dev-dep) incl.
  `checked_in_skills_vow_matches_install_output`, `auto_install_*`, `skill_install_*`. Black-box
  both-compiler harness pattern: `tests/cli-flags/tests.sh`, wired at `scripts/full_test.sh:1535-1553`.

## Design (identical in both compilers)
Let `target = <root>/.claude/skills/vow`, `parent = <root>/.claude/skills`.
1. `create_dir_all(parent)`.
2. If `target` is a symlink: write the tree **in place** (support files first, `SKILL.md` last) and
   return. Preserves dotfile-manager symlinks; no swap is attempted through a link.
3. Reserve a unique stage `parent/.vow-install-<token>` with an *exclusive* mkdir (retry with a new
   token up to 8 times). Rust: `std::fs::create_dir`, token = pid + nanos + attempt. Self-hosted:
   `process_run("mkdir", [path])` (plain `mkdir` without `-p` is exclusive; its output is captured, not printed), token = `time_unix_ms` + attempt.
4. Write every support file into the stage (creating subdirs), then `SKILL.md` **last**. Any failure:
   `remove_dir_all(stage)` best-effort, report the same `cannot create|write <path>` message as today.
5. Commit by rename:
   - target absent: `rename(stage, target)`.
   - target present (explicit re-install / repair): reserve an exclusive backup name the same way,
     `rename(target, backup)`, `rename(stage, target)`; if the second rename fails, `rename(backup,
     target)` to restore and error; on success `remove_dir_all(backup)` best-effort.
   - commit failure: clean up the stage, new message `vow skill install: cannot install <target>`.
6. Return `<target>/SKILL.md`; success output and exit codes unchanged.
Guarantee: a fresh install is all-or-nothing; a replace is a two-rename swap (target missing for
microseconds, restored on failure), never a mixed tree. Auto-install keeps `emit_errors=false` /
`let _ =` and never panics.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `vow/src/skill.rs` | Rust installer: rewrite `install_skill_tree_to`; add helpers + tests | 40-67, 152-165, tests 13477+ |
| `compiler/main.vow` | Self-hosted installer: `install_skill_tree` + helpers | 3148-3205, 3296-3305 |
| `docs/spec/cli.md` | Authoritative spec for `vow skill install` atomicity | 99-120 |
| `README.md` | Skill install prose (mention atomic install) | 90-118 |
| `scripts/generate_help.py` | Regenerates help/skill payloads from the spec (run, do not edit) | — |
| `skills/vow/reference/cli.md` | Generated mirror of `docs/spec/cli.md` | regenerated |
| `tests/skill-install/tests.sh` [new] | Black-box harness run against both compilers | — |
| `scripts/full_test.sh` | Wire the new harness next to Section 4h | after 1553 |

## Steps (TDD slices, in order)

### Slice 1 — Rust: failed install leaves nothing (red → green)
- **Test** (`vow/src/skill.rs` `mod tests`): `skill_install_failure_leaves_no_partial_tree`. Introduce
  seam `install_tree_atomic(root: &Path, entrypoint: &str, files: &[(&str, &str)]) -> io::Result<PathBuf>`
  (the test does not compile until it exists). Feed `[("a","x"),("a/b.md","y")]` (file `a` blocks
  dir `a/`) → `Err`; assert `.claude/skills/vow` absent and `.claude/skills` has no entries (no stage).
- **Code**: add `install_tree_atomic` + `write_skill_files(dir, entrypoint, files)` (support files
  first, entrypoint last) + `reserve_unique_dir(parent, stem)` (exclusive `create_dir`);
  `install_skill_tree_to` becomes a thin call with `skill_entrypoint_markdown()` /
  `skill_support_files()`. Preserve error-message wording (`cannot create`/`cannot write`).

### Slice 2 — Rust: success path + ordering
- **Tests**: success leaves exactly `vow` in `.claude/skills` (no `.vow-install-*`), tree equals
  `skill_support_files()`; existing `checked_in_skills_vow_matches_install_output` and
  `skill_install_*`/`auto_install_*` stay green. Ordering test: `write_skill_files` into a dir with a
  failing support write leaves no `SKILL.md`.

### Slice 3 — Rust: replace existing tree
- **Tests**: pre-seed `.claude/skills/vow/{SKILL.md:"old", reference/stale.md}`; (a) successful install
  replaces with the full new tree, no backup/stage leftovers, `stale.md` gone; (b) injected failure
  (bad file list) leaves the old tree byte-identical; (c) commit-phase failure: `commit_stage(stage,
  target, rename_fn)` with an injected rename that fails on the second call restores the old tree
  (this seam exists so the restore branch is covered — the `codecov/patch` 95% gate counts it).
- **Code**: `commit_stage` with backup swap and restore.

### Slice 4 — Rust: symlinked target + auto-install silence
- **Tests** (`#[cfg(unix)]`): `.claude/skills/vow` → symlink to a real dir: install writes into the
  link target, link preserved, `SKILL.md` last (failure leaves no `SKILL.md`). Auto-install with
  `.claude/skills` a regular file: `maybe_auto_install` returns without panic, creates nothing.
  Perm test (`chmod 0555` on `.claude/skills` with an existing install): `Err`, old tree intact;
  skip when `id -u == 0`; restore mode `0755` before the `TempDir` drops (a guard struct) so cleanup
  does not leak.
- **Code**: symlink branch in `install_tree_atomic` (`symlink_metadata().is_symlink()`).

### Slice 5 — Black-box harness for both compilers (red against self-hosted)
- **New** `tests/skill-install/tests.sh` (model on `tests/cli-flags/tests.sh`; `VOWC_BIN`, `VOWC_KIND`;
  temp project with `.git/` + `.claude/`). **Regression case for #361 (the red test, run for explicit
  install and for auto-install):** pre-create `.claude/skills/vow/` containing a regular file named
  `reference` and no `SKILL.md`. Today `SKILL.md` is written and then `mkdir reference/` fails, leaving
  an orphan entrypoint that auto-install then skips forever (it keys on `SKILL.md`); after the fix the
  swap yields a complete tree and exit 0 (explicit) / complete tree (auto). Mirror it as a Rust unit
  test in Slice 3. Other cases: fresh `skill install --local` has `SKILL.md` and
  every path `skill print`'s tree lists (`reference/cli.md`, `examples/examples.md`,
  `schemas/build-result.schema.json`, …), no `.vow-install-*`; re-run idempotent; a pre-seeded
  partial tree (only `SKILL.md`) is repaired by explicit install; `chmod 555 .claude/skills` with an
  existing tree → exit 1, old files byte-identical, no stage (skip as root); auto-install via
  `build --no-verify <abs>/examples/hello.vow -o <tmp>/out` in a dir with `.claude/` → complete tree,
  stderr silent; with `.claude/skills` a regular file → build exit 0, no skill message; stage
  cleanup after a failed run.
- **Wire** into `scripts/full_test.sh` as a new section after 4h for `$RUST` and `$SELF`, mirroring
  lines 1542-1553 (`pass`/`fail` names `skill-install/rust`, `skill-install/self-hosted`).

### Slice 6 — Self-hosted installer (green for slice 5)
- **File**: `compiler/main.vow` 3148-3205. Replace the body of `install_skill_tree(root, emit_errors)`
  with small helpers (small-functions rule), all `[io, read, write]`:
  `skill_reserve_dir(parent, stem) -> String` (exclusive `mkdir`, 8 attempts, `time_unix_ms`
  token via `int_to_string`, `time_unix_ms` returns `i64`), `skill_write_tree(dir, emit_errors) -> bool` (loop over
  `skill_support_count()`/`skill_support_path(i)`/`skill_support_content(i)`, `SKILL.md` last,
  reusing `skill_parent_dir`), `skill_commit_stage(stage, target, emit_errors) -> bool`
  (`fs_rename`/`fs_remove_dir_all`, backup+restore), orchestrator keeps the `String` return
  (`""` = failure) so `run_skill` (3258-3294) and `maybe_auto_install_skill` (3296-3305) are
  unchanged apart from effect lists. Symlink branch via `fs_is_symlink`. Keep existing error strings;
  add `vow skill install: cannot install <target>` byte-identical to Rust.
- No contracts added (effectful functions are `Skipped` by the native verifier like
  `install_skill_tree` today); do not touch `skill_support_path`'s `vow` block.
- `main.vow` has no unit-test seam; the Slice 5 black-box harness is the test for this slice.

### Slice 7 — Spec and docs
- `docs/spec/cli.md` (99-120): under `install`, state that the tree is staged in a sibling
  `.vow-install-*` directory and renamed into place; an interrupted/failed install never leaves
  `SKILL.md` without its support files; a re-install swaps the old tree in two renames and restores it
  on failure; `SKILL.md` is written last; symlinked `skills/vow` is written in place. Auto-install
  paragraph: add that a failed auto-install leaves no `SKILL.md` and is therefore retried on the next
  build. Add the "Behaviour changes" bullets above verbatim in spirit.
- `README.md` 90-118: one sentence matching the spec.
- Regenerate: `uv run python scripts/generate_help.py` (updates the GENERATE blocks in
  `vow/src/skill.rs` and `compiler/main.vow`, and `skills/vow/**`). The generated diff is large and
  mechanical; commit it separately from hand-written code.

## Testing / verification commands
- `cargo test -j4 -p vow skill::` then `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all`.
- `uv run python scripts/generate_help.py` then `uv run python scripts/generate_help.py --check`
  (the `help/skills-dir-drift` gate, `full_test.sh:1955-1961`); `python3 scripts/generate_operations.py --check`.
- `cargo build -j4 --release -p vow` then `scripts/bootstrap.sh --skip-cargo --no-cache` (~5 min; run in
  the foreground with an explicit timeout ≤ 10 min). Record the PR's final head SHA in the checklist.
- `VOWC_BIN=target/release/vow VOWC_KIND=rust bash tests/skill-install/tests.sh` and
  `VOWC_BIN=build/vowc VOWC_KIND=self bash tests/skill-install/tests.sh`.
- `VOW_FULL_TEST_SKIP_CARGO=1 VOW_FULL_TEST_TIER15_ONLY=1 scripts/full_test.sh` as the fast
  checkpoint; the full suite (~40 min) is too long for one foreground call — run only if the turn
  budget allows, else rely on CI.

## Verification surface (ESBMC / C model)
- No contract, codegen, emitter or IR change. New Vow functions are effectful → `Skipped`
  (`function-has-effects`) like the function they replace; no new proof obligations, no
  `tests/run/` or `examples/` fixtures need to grow (`examples/hello.vow` is only used as a build input).
- No change to `vow-verify/src/c_emitter.rs` / `compiler/c_emitter.vow` → parity unaffected.

## Behaviour changes to document (docs/spec/cli.md, Slice 7)
- A re-install replaces the whole `skills/vow/` directory: stale files from older toolchains **and any
  file a user added under `skills/vow/` are removed** (before, they were left alone).
- Re-install needs write access to `.claude/skills/` itself (before: only to `skills/vow/`). Pinned by
  the `chmod 555` cases.
- A failed install now leaves no `SKILL.md`; a pre-existing partial tree is repaired by the next
  explicit install or build (auto-install).

## Risks
- **Symlinked target** (dotfile managers): handled by in-place branch; without it a swap would replace
  the user's link with a real directory. Covered by Slice 4/6 tests.
- **Cross-filesystem rename**: avoided by staging as a sibling in `.claude/skills/`; a mount-point
  target makes the rename fail → clean error, old tree restored.
- **Replace is not a single atomic syscall**: Linux cannot rename a dir over a non-empty dir. The
  window is two renames; documented in the spec instead of claiming full atomicity.
- **Concurrent installers** (parallel `vow build`s): exclusive stage names prevent interleaved
  writes; a loser's commit rename fails (ENOTEMPTY) and is silent for auto-install, an error for
  explicit install. Never a mixed tree.
- **Killed process** can orphan `.claude/skills/.vow-install-*` (hidden, no `SKILL.md`, not loaded by
  Claude Code). Not swept; documented as out of scope.
- **`mkdir` not on `PATH`** in the self-hosted path → reservation fails → install fails (silent for
  auto-install). Same class of dependency as `sh` in `skill_home_dir`.
- **`process_run` output state** is global; auto-install runs before the build so it cannot clobber a
  later `process_get_stdout` read. Verify in the diff that no `process_*` use precedes it.
- **Binary fixed point**: plain Vow code, no `HashMap` iteration or codegen-order changes;
  `time_unix_ms` is runtime-only. Re-run bootstrap and check the fixed-point hash.
- **Dual-compiler rule**: both installers change in the same PR; messages must be byte-identical
  (harness compares substrings).
- **`codecov/patch` (95%, blocking; `codecov.yml`)**: `compiler/` is in the `ignore` list so the
  `main.vow` lines do not count; the shell harness runs uninstrumented binaries, so every new Rust
  branch must be reached by a `skill.rs` unit test. Make the token source injectable
  (`reserve_unique_dir(parent, stem, token_fn)`) so retry exhaustion is testable; funnel `io::Error`
  context through one `ctx(op, path, e)` helper; unit-test: backup-name reservation failure, retry
  exhaustion, stage cleanup after commit failure (injected rename), symlink-branch write failure.
- **parse → print → parse idempotency**: unaffected (no syntax, printer or AST change).
- **Clippy `--all-targets`**: tests use `unwrap`/`expect` per the module's existing style.

## Assumptions (decided without an operator)
- Stage under `.claude/skills/` (not `.claude/`) to guarantee same filesystem; hidden name + `SKILL.md`
  last prevents Claude Code discovery. Alternative (stage in `.claude/`) rejected: breaks on a
  symlinked `skills/`.
- Exclusive `mkdir` via `process_run` instead of a new `fs_mkdir_exclusive` builtin: avoids Operation Catalogue,
  spec and dual-compiler builtin work for a single caller.
- No ADR: this is an implementation hardening, not an architecture decision.
- Windows is not a target of the self-hosted installer today; `fs_rename` dir-over-dir semantics are
  POSIX.

## Out of scope
- Sweeping stale `.vow-install-*` orphans.
- New runtime builtins (`fs_mkdir_exclusive`, `getpid`), changes to `fs_*` semantics.
- Changing auto-install's `SKILL.md`-exists guard, the `--local`/`--global` rules, or print/bundle.
- Refactoring the generated-doc machinery or the 13k-line `skill.rs` layout.
- Any language, verifier, emitter or contract change.
