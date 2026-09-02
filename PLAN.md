# Plan: issue #1167 — object-write `IoError` divergence + missing path in message

## 1. Problem restated

When `-o` (or the default `build/<stem>` output path) names a location whose
parent directory does not exist, the Rust driver (`vow/src/main.rs`) creates
it with `std::fs::create_dir_all` (silently discarding any error via `let _
=`) and succeeds, while the self-hosted shim
(`vow-clif-shim/src/lib.rs::__vow_clif_finish`) does not create the directory
and fails closed with `IoError` — so identical source and identical `-o`
flags produce different `BuildStatus` across the two compilers.
`docs/spec/errors.md` also tells agents to "check the path in the diagnostic
message" for `IoError`, which is only actionable if the structured JSON
actually carries the path. Both defects were deferred from PR #1163, which
introduced the `IoError` classification that made them externally visible.

## 2. Direction decision (read before implementing — this reverses the naive fix)

The issue frames "which side is right?" as open. The correct answer is: **the
self-hosted shim must start creating the parent directory too — the Rust
driver's auto-create must be kept, not removed.**

Do not attempt the opposite (deleting `create_dir_all` from
`vow/src/main.rs`) — it looks smaller but silently breaks the zero-flag
default path. Verified by reading both drivers' default-output logic:

- Rust: `vow/src/main.rs:500-501` — with no `-o`, `output_path` defaults to
  `Path::new("build").join(stem)`.
- Self-hosted: `compiler/main.vow:107-137` (`default_output`) returns
  `"build/" + stem` for the same no-`-o` case, and `run_build`
  (`compiler/main.vow:1672` onward) has **no** `fs_mkdir` call anywhere
  before codegen (`fs_mkdir("build")` exists only in the unrelated `test`
  subcommand at `compiler/main.vow:2017`).

So today, **self-hosted `vowc build foo.vow` run with no `-o` in a directory
that has no pre-existing `build/` already fails with `IoError`** — a
pre-existing, previously-undiscovered self-hosted gap on the exact workflow
CLAUDE.md documents as the primary day-to-day usage
(`build/vowc build examples/divide.vow`). Deleting Rust's `create_dir_all`
would make the *Rust* driver fail the same way on the same common case (any
fresh working directory, e.g. an agent sandbox with no `build/` yet) — trading
one bug (cross-compiler divergence) for a worse one (both compilers now
reject the default happy path in a fresh directory). This is exactly the kind
of regression the issue's own "silently discards its own error" aside was
warning about interpreting too literally: the *swallowing* is the defect, not
the *creating*.

**Fix, in both Rust crates, no `.vow`/self-hosted source changes required:**

1. `vow/src/main.rs` (~line 581-583): stop discarding `create_dir_all`'s
   result. On failure, return a proper `IoError` `CodegenError::Io` (same
   pattern already used two lines below for the write failure), instead of
   silently falling through to a less specific write failure.
2. `vow-clif-shim/src/lib.rs::__vow_clif_finish` (~line 2893-2917): add the
   same `create_dir_all` call, in the same place in the sequence (after
   `product.emit()` succeeds, before `std::fs::write`), returning
   `CLIF_ERR_IO` on failure — mirroring the fixed Rust behavior exactly, this
   time never having swallowed the error in the first place.

Because both `vow-codegen`'s Cranelift backend (used by the Rust driver) and
`vow-clif-shim` (used by the self-hosted driver via FFI) are independent Rust
implementations, this requires editing both Rust crates — but no *hand*-edits
to `compiler/*.vow` logic, since `clif_emit_module` (`compiler/clif.vow:531`)
and `codegen_failure_diagnostic` (`compiler/main.vow:1626`) already classify
`CLIF_ERR_IO` correctly and already thread the object path into the message
(see section 3). `compiler/main.vow` will still change as a *build artifact*:
`docs/spec/errors.md`'s `IoError` section is embedded verbatim into
`compiler/main.vow`'s `GENERATE:SKILL_FULL` block (and into
`vow/src/skill.rs`) by `scripts/generate_help.py`, so the docs edit in
section 4 regenerates both files mechanically. This means a `bootstrap.sh`
rerun — and the bootstrap triple test — is a required step for this PR, not
optional insurance; see Slice 8 and section 7.

## 3. Sub-issue 2 (path omitted from self-hosted message) — separately verified, likely already fixed

Independent of the direction above: static reading of current `main`
suggests this half of the issue is already resolved and just needs a
regression test, not a fix.

- `compiler/main.vow:1626-1650` (`codegen_failure_diagnostic`) already builds
  the `IoError` message as `str2(String::from("cannot write the generated
  object file: "), obj_path)`, where `obj_path` is computed identically to
  the shim's own object path (`str2(output_path, String::from(".o"))` in both
  `compiler/clif.vow:591` and `compiler/main.vow:1665`).
- `git log -S` on that message string shows it was introduced exactly this
  way in `9b65f580` (#1163 itself) and has not changed since.
- On the Rust side, `vow/src/main.rs:585-591` wraps the write error as
  `CodegenError::Io(format!("{}: {e}", obj_path.display()))`, and
  `CodegenError::to_diagnostic`'s `message` field uses `Display`
  (`vow-codegen/src/lib.rs:89-99`: `"I/O error: {path}: {e}"`), which does
  carry the path into `diagnostics[].message`.

Both compilers therefore appear to already put the object path into the
structured diagnostic message today. **This must still be verified
empirically (Slice 0) before deciding the PR needs no production fix for
sub-issue 2** — if the bootstrap-and-repro check in Slice 0 confirms it, the
deliverable is a regression test locking in the already-correct behavior,
not a source change; say so explicitly in the PR body rather than claiming a
fix that isn't one. If Slice 0 instead finds the path really is missing from
the self-hosted JSON, stop and re-read `compiler/diag.vow`'s JSON emission
path before touching anything — do not guess at a fix site.

## 4. Files to touch

- `vow/src/main.rs` — fix the swallowed `create_dir_all` error
  (~line 581-583) to return a proper `IoError` diagnostic on mkdir failure,
  reusing the existing `CodegenError::Io` / `codegen_error_to_output` path
  used for the write failure right below it.
- `vow/tests/cli_dispatch.rs` — two new integration tests (see Slice 1/2):
  one proving a missing multi-level `-o` parent directory is now created and
  the build proceeds (new coverage — this scenario was previously untested
  even though Rust's behavior here isn't changing), and one proving a
  genuine directory-creation failure (a path component that collides with an
  existing file) now surfaces `IoError` with the failing path in the message
  (this one *is* new behavior — currently silently ignored).
- `vow-clif-shim/src/lib.rs` — add the matching `create_dir_all` call to
  `__vow_clif_finish` (~line 2893-2917); modify the existing
  `finish_reports_an_unwritable_object_path_as_io` test (~line 5810-5839),
  whose current "missing directory" construction will no longer produce
  `IoError` once the fix lands — repoint it at a genuinely uncreatable path
  (a parent path component that already exists as a regular file, so
  `create_dir_all` itself errors) so it keeps testing what its name promises;
  add a new test proving the shim now creates missing nested directories and
  the write succeeds, closing the divergence. Also fix the unrelated
  test-hygiene item from the issue (item 3) in the same file: the `f32` arm
  of `float_remainder_is_refused_as_unsupported` (~line 5785) uses
  `IOP_CONST_F64`/`IDATA_CONST_F64` instead of the `f32` constants — land
  this as a separate small commit even though it's the same file, since it's
  an unrelated change (a hand-built-IR test's constant types, not the
  directory-creation defect).
- `docs/spec/errors.md` — `IoError` section: rewrite the object-write half of
  the `Fix` paragraph. It currently tells agents to "verify the `-o`
  destination's parent directory exists" — that advice becomes actively
  wrong once both compilers auto-create it. The surviving failure mode after
  this fix is directory-creation-itself-failing or the final write failing
  (permission-denied ancestor, a path component that is a non-directory
  file, read-only filesystem, full disk), so the Fix text must describe
  that instead. This is a behavior change to CLI output-path handling, so
  per CLAUDE.md's spec-sync rule it needs this doc update.
- `compiler/main.vow` and `vow/src/skill.rs` — **regenerated, not
  hand-edited**, by `uv run python scripts/generate_help.py` after the
  `errors.md` edit (both embed the spec's `IoError` section verbatim inside
  `GENERATE:...` markers). Followed by `cargo build --release -p vow` and
  `scripts/bootstrap.sh --skip-cargo` per CLAUDE.md's documented sequence.
  No other logic in either file changes.
- No hand-edited `compiler/*.vow` logic changes anticipated (see section
  2/3); revisit only if Slice 0 contradicts the static analysis above.

## 5. TDD slices

0. **Confirm current behavior empirically (spike, not a commit).** Build
   `target/release/vow` (`cargo build --release -p vow`) and bootstrap
   `build/vowc` (`scripts/bootstrap.sh --skip-cargo`) against current `main`
   (pre-fix). Run, from a fresh empty directory:
   - `vow build --no-verify ok.vow` (no `-o`) with no pre-existing `build/`
     — confirm it succeeds (Rust) today (sanity-checks section 2's premise
     that this is the behavior worth preserving).
   - the equivalent `build/vowc build --no-verify ok.vow` — confirm it
     currently fails with `IoError` (sanity-checks the self-hosted gap
     identified in section 2).
   - the issue's exact `-o /tmp/missing_parent/deep/out.bin` repro against
     both binaries, capturing full JSON, to confirm sub-issue 2's message
     already contains the object path on the self-hosted side. If any of
     these three checks contradicts the static reading in sections 2-3, stop
     and revise the plan before writing any test or production code.

1. **Red: Rust integration test for directory-creation-failure reporting.**
   In `vow/tests/cli_dispatch.rs`, add a test (e.g.
   `build_reports_io_error_when_output_directory_cannot_be_created`) that:
   creates a `TempDir`, writes a trivial valid program, creates a *regular
   file* at `<tmp>/blocker`, then runs `vow build --no-verify <src> -o
   <tmp>/blocker/sub/out.bin` (a path whose parent-to-be, `blocker`, is not a
   directory). Assert: exit code 1, JSON `status == "CompileFailed"`,
   `diagnostics[]` contains an entry with `error_code == "IoError"`.
   The discriminating assertion — the one that actually distinguishes
   pre-fix from post-fix behavior — is *whether the message names the
   object filename*, not whether it mentions `blocker/sub` (both versions
   do, since that's a path prefix common to both). Concretely, with `-o
   <tmp>/blocker/sub/myout` (deliberately no `.` in the stem, to sidestep
   `Path::with_extension`'s replace-vs-append distinction):
   `obj_path` is `<tmp>/blocker/sub/myout.o`
   (`vow/src/main.rs:503`, `output_path.with_extension("o")`). Pre-fix, the
   swallowed mkdir error falls through to the *write* failure, so the
   message is built from `obj_path` and names `myout.o`
   (`vow/src/main.rs:589`). Post-fix, the mkdir failure is reported
   directly and the message is built from the mkdir target
   (`<tmp>/blocker/sub`) — it does not mention `myout.o` at all. Assert the
   message does **not** contain `"myout.o"` — that assertion fails pre-fix
   (message contains it) and passes post-fix (message doesn't), which is a
   genuine red/green pair. A bare `message.contains("blocker")` assertion
   passes on both sides and is not a valid red state; do not use it.
   No `libvow_runtime.a` dependency: this fails before linking, so unlike
   `is_runtime_link_failure`-tolerant tests, exit code and error code are
   fully deterministic in CI.

2. **Green: propagate the mkdir error in `vow/src/main.rs`.** Replace the
   swallowed `let _ = std::fs::create_dir_all(parent);` with error
   propagation into a `CodegenError::Io`, mirroring the pattern used for the
   write failure immediately below it. Re-run the Slice 1 test — it should
   pass. Run `cargo test -p vow` in full.
   This is a diagnostic-clarity fix, not a cross-compiler parity fix — the
   self-hosted structured message always names the object path
   (`compiler/main.vow:1646`), so post-fix the two compilers still name
   different paths in this specific message (Rust names the mkdir target,
   self-hosted names the object file). That's fine; nothing in `errors.md`
   requires identical wording, only that the message contain an actionable
   path (see section 8's "out of scope" note on wording).

3. **Red: Rust integration test for the (already-working, newly-tested)
   directory-creation-succeeds path.** Add a second test (e.g.
   `build_creates_missing_output_parent_directory`) with a *genuinely*
   missing multi-level `-o` parent (`<tmp>/missing/deep/out.bin`, no
   blocking file). Assert `<tmp>/missing/deep` exists as a directory after
   the run — **not** that the `.o` file exists: `link_obj`
   (`vow/src/main.rs:345`, `std::fs::remove_file(obj_path)`) deletes the
   object file after a successful link, so an `.o`-existence assertion would
   fail exactly when the build fully succeeds. Directory existence is the
   property actually under test (it isolates "did codegen reach the write
   step" from "did linking succeed") and holds regardless of link outcome
   or object cleanup. Tolerate a link-only failure the same way
   `is_runtime_link_failure` already does elsewhere in this file (an
   environment without a prebuilt `libvow_runtime.a` must not fail this
   test). This test should already pass on current `main` (Rust's existing,
   unmodified-by-this-plan behavior) — it exists to lock in behavior that
   was previously untested, not to prove a fix. Confirm it passes before and
   after Slice 2's change (it is unaffected by that change).

4. **Green: add matching directory creation to the shim.** In
   `vow-clif-shim/src/lib.rs::__vow_clif_finish`, add `create_dir_all` for
   the object path's parent, before `std::fs::write`, returning
   `CLIF_ERR_IO` on failure (never silently swallowed, unlike the Rust bug
   this plan is fixing in Slice 2). Two test changes:
   - Update the existing `finish_reports_an_unwritable_object_path_as_io`
     test to construct a genuinely uncreatable path (write a regular file at
     `<tmp>/blocking_file`, then use `<tmp>/blocking_file/noop.o` as the
     object path — `create_dir_all` on a path whose ancestor is a
     non-directory file must fail) instead of its current "just missing"
     construction, which stops producing `IoError` once this slice lands.
     This test verifies the shim still rejects a genuinely uncreatable path.
   - Add a new test, `finish_creates_missing_parent_directories`, asserting
     a multi-level *genuinely* missing directory now gets created and
     `__vow_clif_finish` returns `0` with the object file present. This is
     the real red/green pair for this slice: red before the
     `create_dir_all` call is added, green after.

5. **Cross-compiler manual verification.** Rebuild both binaries
   (`cargo build --release -p vow`, `scripts/bootstrap.sh --skip-cargo`) and
   re-run the issue's exact repro commands. Both must now succeed
   (`"status":"Unverified"` under `--no-verify`) for the missing-parent-dir
   case, and both must produce `IoError` with the failing path in the
   message for the blocking-file case. Record the output in the PR
   description as the direct acceptance evidence — there is no automated
   cross-compiler CLI/filesystem differential harness to lean on instead
   (`scripts/parity.py` / `scripts/test_parity.py` compare `.vow`-source-
   triggered diagnostics, not `-o`-flag/filesystem scenarios).

6. **Sub-issue 2: regression test only, conditional on Slice 0.** If Slice 0
   confirms the self-hosted message already contains the object path (the
   expected outcome), no `compiler/*.vow` change is needed. Add no new
   automated coverage for the message text specifically — `compiler/` has no
   unit-test framework of its own for CLI-driver-level string composition
   (self-hosted tests are `.vow`-source-triggered diagnostic fixtures run
   via `vow test`, which do not control `-o` paths), and building one is out
   of scope for this PR. Record Slice 0's manual repro output as the
   evidence in the PR body, and note explicitly that this is a known
   coverage gap traceable to the missing self-hosted CLI-flag test harness,
   not a silently-skipped check. If Slice 0 contradicts this and the path
   really is missing, stop and follow section 3's escalation instead of
   proceeding with this slice.

7. **Test-hygiene cleanup (issue item 3), separate commit.** In
   `vow-clif-shim/src/lib.rs`'s `float_remainder_is_refused_as_unsupported`
   test (~line 5785), change the loop to carry per-type const-opcode/data-
   kind pairs instead of hardcoding `IOP_CONST_F64`/`IDATA_CONST_F64` for
   both cases:
   ```rust
   for (name, op, ty, const_op, const_dk) in [
       ("f32", IOP_REM_F32, ITY_F32, IOP_CONST_F32, IDATA_CONST_F32),
       ("f64", IOP_REM_F64, ITY_F64, IOP_CONST_F64, IDATA_CONST_F64),
   ] {
       ...
       add_test_inst(ctx, 0, const_op, ty, const_dk, 0, 1, &[]);
       add_test_inst(ctx, 1, const_op, ty, const_dk, 0, 2, &[]);
       ...
   }
   ```
   This is a refactor of a passing test (both arms already assert
   `CLIF_ERR_UNSUPPORTED`), not a new assertion — run before and after to
   confirm behavior is unchanged.

8. **Docs.** Rewrite `docs/spec/errors.md`'s `IoError` `Fix` paragraph per
   section 4. Then regenerate the embedded copies per CLAUDE.md's documented
   sequence — this is required, not optional, because the section is
   embedded verbatim in `compiler/main.vow`'s `GENERATE:SKILL_FULL` block
   and in `vow/src/skill.rs`:
   ```bash
   uv run python scripts/generate_help.py
   cargo build --release -p vow
   scripts/bootstrap.sh --skip-cargo
   ```
   Confirm `git diff` for `compiler/main.vow` / `vow/src/skill.rs` touches
   only the regenerated `IoError` text inside the `GENERATE` markers, and
   run the bootstrap triple test (`scripts/concat_vow.sh` + three-stage
   `sha256sum`) once to confirm the regenerated text still reproduces a
   binary fixed point (see section 7).
   `scripts/check_help_coverage.py` is a different check (grammar.md-vs-
   `--help`-JSON feature coverage) and does not verify this regeneration
   step; do not substitute it for the sequence above.

## 6. Verification surface

- No contracts, IR, or C-model changes. This is CLI-driver/filesystem
  behavior across two independent Rust crates (`vow`, `vow-clif-shim`) plus
  one Rust unit-test hygiene fix — ESBMC is not involved in any of it.
- No `tests/run/` or `examples/` fixtures need to grow: the scenario is
  filesystem state around the `-o` flag, not Vow source semantics, so it
  cannot be expressed as a `.vow` program both compilers execute
  identically — it must be tested at the CLI/process level (Rust integration
  tests) and the FFI-shim unit-test level, which is exactly what Slices 1-4
  do.
- `scripts/parity.py` / `scripts/test_parity.py` are out of scope: they
  compare diagnostics triggered by `.vow` source content across compilers,
  not `-o`-flag/filesystem-state scenarios. Do not route this fix's
  regression coverage through them.

## 7. Risk areas

- **Binary fixed point:** no hand-edited `compiler/*.vow` logic changes are
  planned, but `compiler/main.vow` *does* change as a regenerated build
  artifact once `docs/spec/errors.md` is edited (see Slice 8) — its
  `GENERATE:SKILL_FULL` block embeds the `errors.md` text verbatim. The
  bootstrap triple test is therefore a required step for this PR, not
  optional insurance: run it once after Slice 8's regeneration. Since the
  only change to `compiler/main.vow` is the embedded doc text (not codegen
  logic, stack-slot layout, or `BTreeMap` ordering), it is expected to
  reproduce a byte-identical fixed point, but this must be confirmed, not
  assumed. `scripts/bootstrap.sh` must also be rerun (independent of the
  docs regeneration) to link the self-hosted binary against the updated
  `vow-clif-shim` from Slice 4 — do this before Slice 5's manual
  verification.
- **`cargo clippy --all -- -D warnings`:** both production-code changes are
  small, mechanical (error propagation via an existing pattern, one new
  `create_dir_all` call using the existing `CLIF_ERR_IO` return convention)
  and unlikely to introduce new lints, but run the gate as normal.
- **Test environment dependency on `libvow_runtime.a`:** Slice 3's
  "directory gets created" test reaches the link stage on success, which
  needs a prebuilt runtime archive. Follow the existing
  `is_runtime_link_failure` tolerance pattern already used elsewhere in
  `cli_dispatch.rs` rather than requiring a full link to pass — assert
  the created *directory's* existence as the primary signal (not the `.o`
  file, which a successful link deletes, and not overall exit code).
- **User-visible behavior change:** the self-hosted compiler's `-o`/default-
  output handling changes from "fail closed on missing directory" to
  "create it", and the Rust driver's *silent* mkdir-failure handling becomes
  a *reported* `IoError`. Flag both explicitly in the PR body — the second
  one especially, since it means a scenario that previously proceeded (badly)
  to a generic write-failure message now stops earlier with a clearer one;
  this is a strict diagnostic-quality improvement but still worth calling
  out as a behavior change per the autonomous-run operating contract.
- **Sub-issue 2 uncertainty:** section 3's conclusion (already fixed) is
  based on static reading only, not a bootstrap-verified run. Slice 0 must
  not be skipped.

## 8. Out of scope

- Do not touch `clif_emit_module` / `codegen_failure_diagnostic` in
  `compiler/clif.vow` / `compiler/main.vow` unless Slice 0 proves section 3
  wrong.
- Do not unify the exact wording of the two compilers' `IoError` messages
  ("I/O error: {path}: {e}" on Rust vs. "cannot write the generated object
  file: {path}" on self-hosted) — both already carry the path, which is the
  actual requirement from `errors.md`; making the strings byte-identical is
  unrequested polish.
- Do not build a general-purpose CLI-flag/filesystem differential-test
  harness for `scripts/parity.py`/`scripts/test_parity.py` as part of this
  fix, even though the investigation above notes such a harness does not
  exist. That is a larger, separate infrastructure project; mention it as a
  possible follow-up in the PR body if it seems warranted, but do not build
  it here.
- Do not bundle the item-3 test-hygiene fix and the item-1 behavior fix into
  a single commit; land them as separate, independently revertable commits
  even though they end up in the same file.
- Do not add directory-creation retry/backoff, disk-space pre-checks, or any
  other robustness beyond "create the directory, report failure honestly if
  you can't" — that would be scope creep beyond what the issue asks for.
