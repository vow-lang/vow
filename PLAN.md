# Plan: #1165 — create the output parent directory in the self-hosted driver

## 1. Problem restated

`vowc build -o <path>` succeeds under the Rust driver even when `<path>`'s parent
directory does not exist, because `vow/src/main.rs` calls
`std::fs::create_dir_all(parent)` before writing the object file
(`vow/src/main.rs:581-583`). The self-hosted driver has no equivalent: the FFI shim's
`__vow_clif_finish` (`vow-clif-shim/src/lib.rs:2893-2917`) calls `std::fs::write(obj_path,
&bytes)` directly, so a missing parent directory surfaces as `CLIF_ERR_IO` →
`EC_IO_ERROR` (`compiler/main.vow:1645-1646`). This is an accept/reject divergence
between the two compilers on the same input, invisible to `scripts/parity.py` because
every corpus fixture already builds into a directory that exists. The fix is to make the
self-hosted driver create the missing parent directory too, matching the Rust driver's
existing (already-shipped, non-breaking) behavior — this is also what the issue title
prescribes, rather than the alternative of making both compilers reject a missing
directory.

## 2. Files to touch

**Rust side (`vow-clif-shim/`):**
- `vow-clif-shim/src/lib.rs` — `__vow_clif_finish` (~line 2893-2917): add a
  `std::fs::create_dir_all` call on `obj_path`'s parent before `std::fs::write`, mirroring
  `vow/src/main.rs:581-583` verbatim in spirit (`let _ = std::fs::create_dir_all(parent);`
  — best-effort, the subsequent `write` reports any real failure as `IoError`).
- `vow-clif-shim/src/lib.rs` test module (~line 5809-5840) — the existing test
  `finish_reports_an_unwritable_object_path_as_io` currently uses a *missing* parent
  directory as its "unwritable" scenario. After this fix that scenario will start
  succeeding, so this test's premise breaks and it must be repointed at a genuinely
  unwritable (permission-denied) directory that still exists. See TDD slice 2.

**No change needed:**
- `compiler/clif.vow` — `clif_emit_module` (line 531-605) calls `__vow_clif_finish` then
  `__vow_clif_link` with `obj_path` and `output_path` that share the same parent directory
  (`obj_path = str2(output_path, ".o")` is a plain string concatenation, never a
  separator-crossing rename), so creating the directory once inside `__vow_clif_finish`
  covers both the object write and the later `cc -o <output_path>` link. No new Vow-level
  builtin, extern declaration, or effect annotation is required — `env.vow:550,555` already
  declare both `__vow_clif_finish` and `__vow_clif_link` as `[io]`, and their signatures
  (`(i64, i64) -> i64`) are unchanged.
- `vow/src/main.rs` — the Rust driver already does the right thing; not touched.
- `docs/spec/cli.md` — no flag or subcommand shape changes.

**Docs (and their generated embeds — do not skip the regeneration step):**
- `docs/spec/errors.md` (`### IoError`, lines 549-564) — the current text lists
  "unwritable output directory" as a cause and tells users to "verify the `-o`
  destination's parent directory exists". Once the parent is auto-created by both
  compilers, a *missing* parent directory is no longer a cause of `IoError`; update the
  **Meaning** and **Fix** text to say the parent directory is created automatically and
  that `IoError` now means the directory could not be created or written to (permission
  denied, full disk, read-only filesystem, or a path component that exists as a
  non-directory file) — not that it is merely absent.
- `compiler/main.vow` (`### IoError` prose duplicated at lines 6898 and 12025) and
  `vow/src/skill.rs` — both embed `docs/spec/errors.md`'s prose verbatim inside
  `// GENERATE:`-delimited blocks via `scripts/generate_help.py`. Confirmed by grep: the
  exact phrase "unwritable output directory" does **not** appear as a separate hand-edited
  copy in either file today — it is generated, not hand-maintained — so after editing
  `errors.md`, run `uv run python scripts/generate_help.py` to regenerate both, per the
  root `CLAUDE.md` "After updating a spec file..." instructions. Do not hand-edit the
  `// GENERATE:` blocks directly.

## 3. TDD slices

1. **Shim creates the missing parent directory (red → green on the core fix).**
   - Test: new `#[test]` in `vow-clif-shim/src/lib.rs`'s test module, e.g.
     `finish_creates_a_missing_parent_directory`. Reuse the same setup as
     `finish_reports_an_unwritable_object_path_as_io` (declare a no-op function, run
     `__vow_clif_fn_begin`/`add_test_block`/`add_test_inst`/`__vow_clif_fn_end`), then call
     `__vow_clif_finish` with `temp_dir.path().join("no_such_dir").join("noop.o")` as the
     object path.
   - Behavior under test: `__vow_clif_finish` returns `0` (success) and
     `object_path.exists()` is `true` afterward.
   - Production code: add `create_dir_all` on `Path::new(obj_path).parent()` in
     `__vow_clif_finish`, before the `std::fs::write` call.
   - This test is red before the production change (current code returns `CLIF_ERR_IO`
     and never creates the file) and green after.

2. **Fix the now-invalid existing test's premise.**
   - Test: rename/rework `finish_reports_an_unwritable_object_path_as_io` so its failure
     is one `create_dir_all` cannot paper over: create an empty regular *file* at
     `temp_dir/blocker`, then point the object path at `temp_dir/blocker/noop.o` (a path
     that treats an existing file as a directory component). `create_dir_all` fails
     because `blocker` exists and is not a directory, so the subsequent write still fails
     and `__vow_clif_finish` still returns `CLIF_ERR_IO`; assert the object file was not
     created. This is preferred over a permission-based (`chmod 0o555`) approach: it needs
     no `#[cfg(unix)]`/`PermissionsExt`, has no directory-cleanup edge case for `TempDir`
     to worry about, and — importantly — permission bits are ineffective against a
     root-run process (common in CI containers), which would make a chmod-based test
     spuriously pass shut for the wrong reason. It also matches the new docs wording from
     §2 ("a path component that exists as a non-directory file") exactly.
   - Behavior under test: a write failure caused by a path whose directory cannot be
     created is still classified as `CLIF_ERR_IO`, preserving the #1163 guarantee that
     backend write failures (as opposed to a *missing-but-creatable* parent directory, now
     handled) are `IoError`, not `CodegenFailed`.
   - Production code: none — this slice only repairs test coverage that the slice-1 fix
     invalidates. Do this in the same commit/PR as slice 1, not as a follow-up, since
     leaving the old test in place would make the suite lie about what `CLIF_ERR_IO` still
     covers.

3. **End-to-end accept/reject parity across both compilers.**
   - Test: extend `scripts/full_test.sh`. Add a small dedicated case near "Section 1:
     Build --no-verify" (same pattern as lines 405-425) — not inside "Section 7: Error
     Handling"'s `compare_error` loop (lines 1217-1234), since this case must now assert
     *success* on both sides, the opposite of that loop's contract. Reuse an existing
     trivial fixture (e.g. `examples/hello.vow`, or a fresh minimal one-liner matching the
     issue's repro) and run:
     ```
     $RUST build --no-verify "$vow_file" -o "$TMPDIR/rust_missing_parent/out"
     run_self build --no-verify "$vow_file" -o "$TMPDIR/self_missing_parent/out"
     ```
     where `$TMPDIR/rust_missing_parent` and `$TMPDIR/self_missing_parent` do not exist
     beforehand (do not `mkdir` them). Feed both JSON outputs through `compare_json` (the
     Section 1 helper), and additionally assert both produced executables actually exist
     on disk afterward (`[ -x "$TMPDIR/rust_missing_parent/out" ]` /
     `[ -x "$TMPDIR/self_missing_parent/out" ]`) — `compare_json` alone only proves the
     *diagnostics* match, not that the directory was actually created and the executable
     actually written, which is the entire point of this issue.
   - Behavior under test: both compilers report `"status":"Unverified"` (or whatever
     `build --no-verify` reports for a passing build) with no diagnostics, and both leave
     a runnable executable at the nonexistent-parent path.
   - This is the regression test the issue explicitly asks for ("add a corpus or
     `full_test.sh` case that builds into a nonexistent directory with both compilers").
     It is red today (self-hosted branch fails) and green once slice 1 lands.
   - Cache note (checked while planning, not a TODO for the implementer): the self-hosted
     driver has no on-disk compile-object cache at all — `grep -rn "VOW_CACHE_DIR"
     compiler/*.vow` only matches the generated `--help` prose, never an actual
     `cache_lookup`/`cache_store` call site — so there is no self-hosted cache path that
     bypasses `__vow_clif_finish`. The Rust driver's cache-hit path
     (`vow/src/main.rs:522-547`) does `std::fs::copy(&cached_obj, &obj_path)` *before*
     reaching `create_dir_all` (line 581); if the parent is missing that `copy` fails, the
     `if let ... && ... .is_ok()` chain short-circuits, and execution falls through to the
     ordinary codegen path below, which does hit `create_dir_all`. So a missing directory
     self-heals on the Rust side regardless of cache state, and no `VOW_CACHE_DIR=$(mktemp
     -d)` isolation is required for this test to be deterministic.

4. **Docs slice.**
   - No test — update `docs/spec/errors.md`'s `### IoError` section text per §2 above,
     then regenerate and rebuild per §2's "Docs" bullet:
     `uv run python scripts/generate_help.py`, `cargo build --release -p vow`,
     `scripts/bootstrap.sh --skip-cargo`. `scripts/check_help_coverage.py` checks
     `grammar.md` against `--help` structurally, not this prose section, so it won't catch
     a missed regeneration here — the generate_help step must be run manually and its
     output diff reviewed.

## 4. Verification surface

This change touches only host-side file-system I/O in the FFI shim (`vow-clif-shim`) and
a `full_test.sh` shell case — it does not touch:
- Vow contracts, `requires`/`ensures`/`invariant` semantics,
- the IR (`vow-ir`, `compiler/ir.vow`), the C emitter, or anything ESBMC models,
- any `tests/run/`, `tests/verify/`, or `examples/*.vow` Vow-source fixtures (the new test
  fixture is a plain existing example invoked with a different `-o` path from the shell
  harness, not a new `.vow` file).

No new ESBMC properties are needed. No `benchmarks/` or `tests/verify*/` fixtures need to
grow. The only "fixture" growth is the `full_test.sh` shell case in slice 3.

## 5. Risk areas

- **Binary fixed point:** `__vow_clif_finish` is host-side FFI, not part of the
  self-hosted IR or the object it emits — the bytes written to `obj_path` are unchanged
  (same `product.emit()` call, same `bytes` slice), only *whether the write is attempted
  against a directory that now exists* changes. This cannot perturb the bootstrap triple
  test (`compiler_a`/`compiler_b`/`compiler_c` sha256 fixed point) or Cranelift codegen
  ordering, since no instruction lowering, `BTreeMap` iteration order, or stack-slot
  layout is touched.
- **`parse → print → parse` idempotency:** not touched — no AST, parser, or printer
  change.
- **Existing accepted behavior:** the Rust driver's `create_dir_all` already ships and is
  unconditionally best-effort (`let _ = ...`); mirroring it in the shim cannot regress any
  currently-passing case, since previously-missing directories previously *failed* on the
  self-hosted side — this is strictly widening acceptance to match the Rust driver, not
  narrowing it.
- **`cargo clippy --all -- -D warnings`:** the new `create_dir_all` call in
  `__vow_clif_finish` and the blocker-file test in slice 2 are both plain, portable
  `std::fs` calls with no new imports or `#[cfg]` gates, so no new clippy or
  cross-platform surface is introduced.
- **`full_test.sh` TMPDIR reuse:** confirm the new nonexistent-parent-directory case picks
  directory names that don't collide with any other section's use of `$TMPDIR` (e.g. don't
  reuse `rust_missing_parent`/`self_missing_parent` names used elsewhere in the script).
- **Generated-file diff size:** regenerating `compiler/main.vow` and `vow/src/skill.rs`
  via `scripts/generate_help.py` (slice 4) will produce a mechanical diff wherever the
  `### IoError` prose is embedded, proportional to the wording change in `errors.md`, not
  to the size of the actual fix. This is expected and should be called out in the PR
  description so a reviewer isn't surprised by two touched `.vow`/`.rs` files for what is
  otherwise a one-paragraph doc edit; it carries no fixed-point or codegen risk since
  neither file's generated block affects compiled output, only `--help`/skill text.

## 6. Out of scope

- Removing `create_dir_all` from the Rust driver (the alternative "both compilers reject"
  resolution the issue floats) — rejected because it would be a breaking behavior change
  to an already-shipped, presumably-relied-upon Rust-driver capability, whereas adding the
  self-hosted equivalent is purely additive and matches the issue's own title.
  Documented as the deliberate choice; not a change to bundle here. The implementation
  stage should restate this rationale (create-and-match, not reject-and-match, plus why)
  in the PR body, since the issue explicitly asked for a deliberate choice between the two
  and that judgement call belongs on the record for a reviewer, per the run's operating
  contract.
- Reconciling the `obj_path` derivation difference between the two drivers
  (`output_path.with_extension("o")` in Rust vs. `str2(output_path, ".o")` string
  concatenation in `compiler/clif.vow`) — these differ for edge cases like an output path
  that already ends in `.something`, but that's an unrelated pre-existing divergence, not
  in scope for this issue.
- Extending `scripts/parity.py`'s corpus-fixture generation to systematically probe
  missing-parent-directory cases across all fixtures — the issue asks for *one* regression
  case, not a new parity dimension; a broader sweep is a separate, larger change.
- Any change to `vow-linker` (`link_reproducible_executable`) — it already receives a
  parent directory that exists by the time it runs, once slice 1 lands.
- Refactoring `__vow_clif_finish`/`__vow_clif_link` signatures, error codes, or the
  `CLIF_ERR_*` constant scheme — untouched, no new error code is introduced by this fix.
