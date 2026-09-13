# Plan: Migrate filesystem, stdin, args, and stderr Builtin Operations to the Operation Catalogue (#1272)

## 0. Hard precondition — this plan is pinned to an unmerged dependency

Issue #1272 is `Blocked by: #1270` ("Tracer bullet: catalogue-driven print Builtin
Operations"). As of planning time (2026-09-13), #1270 is **not merged** — it has
no PR, its branch is `sym/vow/1270-tracer-bullet-catalogue-driven-print-builtin-operations-abi-runtime-symbol-facts`
at commit `473db70a`, and it is still labeled `sym:running`. This #1272 workspace
branch is based on `origin/main` (`0cf9d948`), which has **no** Operation
Catalogue at all.

Everything below (schema shape, `scripts/generate_ops.py` function names,
`vow-ir/src/lower/op_catalogue.rs` module shape) is described **as it exists on
#1270 @ 473db70a**. The implementation stage must:

1. **Not start until #1270 has actually merged to `main`.** If #1270 is still
   open when implementation begins, stop and re-check rather than building on
   top of an unmerged branch.
2. **Rebase this branch onto `origin/main` after #1270 merges** — never
   cherry-pick or merge #1270's feature branch directly, since its branch may
   still change before merge (e.g. squash message, minor fixups).
3. **Re-verify every file path, schema field, and function name cited here**
   against the merged shape before editing — this plan is a map of intent, not
   a guarantee the merged code matches byte-for-byte.
4. **Not silently absorb any of #1270's own unfinished acceptance criteria**
   into this slice (e.g. if #1270 lands without wiring `compiler/lower.vow`'s
   `GENERATE:OP_CATALOGUE` markers, that is #1270's gap to fix, not #1272's).
   The one deliberate exception is §5 slice 8 (CI freshness wiring), which is
   included here because #1272's own acceptance criteria depend on it and it
   is a one-line addition regardless of who lands it first.

A `gh issue comment` will be posted on #1272 noting this pin (see §7).

## 1. Problem restated

Vow's filesystem (`fs_read`, `fs_open`, `fs_read_line`, `fs_status`,
`fs_close`, `fs_write`, `fs_exists`, `fs_mkdir`, `fs_listdir`, `fs_remove`,
`fs_remove_dir`, `fs_is_dir`, `fs_is_symlink`, `fs_rename`), stdin
(`stdin_read`, `stdin_read_line`, `stdin_ready`), args (`args`), and stderr
(`eprintln_str`) Builtin Operations — 19 operations total — currently have
their runtime-symbol, Cranelift ABI, and IR-return-shape facts hand-duplicated
across `vow-ir/src/lower/mod.rs` (plus a hand-mirrored test table),
`vow-codegen/src/cranelift_backend.rs`, `vow-clif-shim/src/lib.rs`, and their
self-hosted counterparts in `compiler/lower.vow`'s `builtin_to_extern` /
`builtin_ret_ty` if-chains. Six of these operations (`fs_read`,
`fs_read_line`, `stdin_read`, `stdin_read_line`, `args`, `fs_listdir`) also
carry a hand-duplicated "heap result tag" (`String` or `Vec`) that
`pin_to_root` depends on, itself duplicated between `tag_builtin_result` in
Rust and inline `lctx_tag` calls in `compiler/lower.vow`. This slice moves all
of these facts into the Operation Catalogue introduced by #1270 (extending its
schema with a new heap-result-tag field, since #1270's `print_*` operations
were all `Unit`-returning and never needed one), regenerates both compilers'
projections from it, and removes the corresponding hand-written arms —
preserving behavior, effects, and return shapes exactly.

## 2. Files to touch

### Catalogue data + schema (source of truth)
- `docs/spec/operations.json` — add 19 entries (fs_*, stdin_*, args, eprintln_str).
- `docs/spec/schemas/operation-catalogue.schema.json` — add a new required,
  nullable `heap_result_tag` field (`null | "String" | "Vec"`); backfill
  `"heap_result_tag": null` on the 3 existing `print_*` entries so the schema
  stays uniform across all entries.

### Generator
- `scripts/generate_ops.py`:
  - `render_lower_rs` / the Rust `op_catalogue.rs` output: add a second
    generated function, e.g. `pub(crate) fn heap_tag(name: &str) -> Option<&'static str>`,
    rather than widening `lookup`'s existing `(symbol, Ty)` tuple (avoids
    touching every call site of `lookup`/`vow_builtin_to_runtime`).
  - `render_lower_vow_block`: add `op_catalogue_heap_tag(name: String) -> String`
    (empty-string sentinel = no tag), alongside the existing
    `op_catalogue_extern` / `op_catalogue_ret_ty` functions.
  - `check_env_rs_presence`: fix the `f'def("{name}"'` substring match to a
    regex tolerant of rustfmt line-wrapping (`def(\s*"name"`) — `fs_write`
    and `fs_listdir`'s multi-arg `def(...)` calls wrap the name onto its own
    line in `vow-types/src/env.rs` today, which the current literal-substring
    check would falsely flag as missing.
  - `check_grammar_presence`: generalize from the single hardcoded `"Print /
    IO"` table to scan every `####`-level table under the `## Builtin
    Function Signatures` section (union of rows across all such tables), since
    this slice's 19 operations span three tables (`Print / IO`, `Filesystem`,
    `Input`). Prefer this over adding a per-entry `grammar_section` catalogue
    field — it's less schema churn and generalizes for free to future slices
    (#1271, #1273).
  - `compute_drift` / freshness check: no structural change needed beyond
    picking up the new `heap_tag` render output.
- `scripts/test_generate_ops.py`: extend fixtures/tests for the new field,
  the regex fix, and the generalized grammar cross-check (see §3).

### Rust compiler consumption
- `vow-ir/src/lower/mod.rs`:
  - Remove the 19 migrated arms from `vow_static_builtin_to_runtime`'s match
    (mirroring how #1270 removed `print_str`/`print_i64`/`print_u64`), letting
    `vow_builtin_to_runtime` fall through to `op_catalogue::lookup`.
  - `tag_builtin_result`: for the 6 heap-tagged names, consult
    `op_catalogue::heap_tag(name)` instead of the hardcoded match arms at
    lines ~199 (`fs_read`, `fs_read_line`, `stdin_read`, `stdin_read_line`)
    and ~206 (`args`, `fs_listdir`). Leave the other names in that match
    (`string_*`, `process_*`, `hex_encode`, etc.) untouched — out of scope.
  - Keep the existing `builtins_lower_to_runtime_symbols_and_return_types`
    test's 19 rows for these operations as-is — it is a behavior-preservation
    regression test for `vow_builtin_to_runtime`'s public contract, not a
    duplicated fact, and must keep passing unchanged.
  - Add a new test (alongside #1270's
    `op_catalogue_print_builtins_resolve_from_the_generated_projection`, or
    extending it) asserting `op_catalogue::lookup` and `op_catalogue::heap_tag`
    directly for all 19 names — this is what actually exercises the
    catalogue-driven path rather than just the pre-existing behavior test.
- `vow-codegen/src/cranelift_backend.rs`: remove the 19 hand-written ABI
  match arms (`__vow_fs_*` at ~2620–2677, `__vow_eprintln_str` /`__vow_args`/
  `__vow_stdin_*` at ~2837–2857), replacing with a call into
  `cranelift_backend::op_catalogue::abi_sig` (mirroring #1270's `print_*`
  wiring) before falling back to the remaining hand-written match.
- `vow-clif-shim/src/lib.rs`: same removal for its byte-identical duplicate
  ABI arms (`__vow_fs_*` at ~3536–3589, `__vow_eprintln_str`/`__vow_args`/
  `__vow_stdin_*` at ~3732–3752), consulting `vow_clif_shim::op_catalogue::abi_sig`.

### Self-hosted compiler consumption
- `compiler/lower.vow`:
  - Remove the 19 arms from `builtin_to_extern` (~1426–1440 for fs_*, ~1510–1513
    for args/stdin_*) and `builtin_ret_ty` (~1549–1563, ~1616–1619), letting
    the generated `// GENERATE:OP_CATALOGUE` block (produced by
    `scripts/generate_ops.py`, never hand-edited) supply them.
  - Replace the 6 inline heap-tag `if` blocks at ~2528–2536
    (`fs_read`/`fs_read_line`/`stdin_read`/`stdin_read_line` → `"String"`;
    `args`/`fs_listdir` → `"Vec"`) with a single call to the generated
    `op_catalogue_heap_tag(fn_name)`, falling back to the existing inline ifs
    for the remaining non-migrated names (`string_*`, `process_*`, etc.).
  - **Do not hand-edit inside the `GENERATE:OP_CATALOGUE` markers.** Run
    `scripts/generate_ops.py` to regenerate that block after editing
    `docs/spec/operations.json`.

### Docs
- No `docs/spec/grammar.md` content changes — signatures, effects, and
  descriptions for these 19 operations are unchanged; the plan only adds a
  cross-check that the catalogue's `surface_name`s already appear in
  `grammar.md`'s existing `Filesystem` and `Input` tables (see §2 generator
  changes). If the generalized `check_grammar_presence` scan finds any
  mismatch against the *current* wording of those tables, fix the catalogue
  entry to match — never edit `grammar.md` to match a wrong catalogue entry.
- `compiler/main.vow`'s help-JSON, the compact `BUILTINS` line, and
  `vow/src/skill.rs`'s embedded skill doc are already generated from
  `docs/spec/grammar.md` by `scripts/generate_help.py` (see `CLAUDE.md`'s
  "Canonical Source of Truth" section) — they are not a fourth hand-duplicated
  fact site for this slice and need no direct edits. Run
  `uv run python scripts/generate_help.py` only if unrelated drift is detected
  by `scripts/check_help_coverage.py`; do not fold unrelated help regeneration
  into this PR.
- `scripts/full_test.sh`: see §5 slice 8 — wire `generate_ops.py --check` if
  not already wired by the time #1270 lands.

## 3. TDD slices

Each slice is red (write/extend a failing test) → green (minimal production
change) → refactor. Recommended order: catalogue/schema/generator first
(cheap, pure-Python, fast feedback), then Rust, then self-hosted, then the
CI-wiring check last.

1. **Schema accepts `heap_result_tag`.**
   Test: `scripts/test_generate_ops.py::ValidateCatalogueTest` — add
   `test_heap_result_tag_null_is_valid`, `test_heap_result_tag_string_is_valid`,
   `test_heap_result_tag_vec_is_valid`, `test_unknown_heap_result_tag_rejected`,
   `test_missing_heap_result_tag_rejected` (now required).
   Production: extend `operation-catalogue.schema.json`'s `required` array and
   `properties` with the new enum field.

2. **Existing `print_*` entries stay valid after the schema tightens.**
   Test: `LoadCatalogueTest::test_valid_catalogue_loads` continues to pass once
   `operations.json`'s 3 `print_*` entries gain `"heap_result_tag": null`.
   Production: backfill those 3 entries.

3. **`check_env_rs_presence` survives rustfmt-wrapped `def(...)` calls.**
   Test: `scripts/test_generate_ops.py::CrossCheckTest` — add
   `test_wrapped_def_call_is_still_found`, feeding a fixture `env_rs` string
   where the `def(` and `"name"` are on separate lines (as `fs_write` and
   `fs_listdir` actually appear in `vow-types/src/env.rs`).
   Production: change the substring check to a regex tolerant of whitespace/
   newlines between `def(` and the quoted name.

4. **`check_grammar_presence` covers `Filesystem` and `Input`, not just `Print / IO`.**
   Test: extend `CrossCheckTest` with a fixture `grammar` string containing all
   three tables; assert a name present only in `Filesystem` (e.g. `fs_read`)
   or only in `Input` (e.g. `stdin_read`) is found, and a name absent from all
   three is still reported missing.
   Production: rewrite `check_grammar_presence` to scan every `####` table
   under the `## Builtin Function Signatures` `##` section and union their
   rows, rather than calling `extract_table` with one fixed heading.

5. **Catalogue grows to 22 entries and cross-checks pass against the real repo.**
   Test: update `LoadCatalogueTest::test_valid_catalogue_loads`'s expected
   name set to the union of `print_*` and the 19 new names (or relax it to an
   `issuperset` check against a documented baseline, whichever reads clearer);
   `CrossCheckTest::test_real_catalogue_matches_env_rs` and
   `test_real_catalogue_matches_grammar_print_io_table` (rename to reflect the
   broadened scan) must pass unmodified against the real files.
   Production: add the 19 entries to `docs/spec/operations.json`, each field
   transcribed verbatim from the existing hand-written sources — do not infer
   ABI types from the surface-level Vow signature. In particular:
   - `stdin_ready`: `ir_return_shape: "Bool"` but `abi_return: "I64"` (the
     Cranelift extern signature returns bool-as-i64, per
     `vow-codegen/src/cranelift_backend.rs`'s `"__vow_stdin_ready"` arm — do
     not write `"I8"` by pattern-matching on the Vow `bool` surface type).
     This is also the first catalogue entry to exercise the `ITY_BOOL` render
     path in `_ITY_FN`.
   - `heap_result_tag`: `"String"` for `fs_read`, `fs_read_line`,
     `stdin_read`, `stdin_read_line`; `"Vec"` for `args`, `fs_listdir`; `null`
     for the remaining 13 (no `_in_arena` variants exist for any of these 19
     runtime symbols, so none of them are ever root-vs-candidate-arena routed
     — this family is root-only, unlike e.g. `String::push_str`).
   - All 19 use uniform `I64` ABI params/returns (or none) — transcribe
     directly from `vow-codegen/src/cranelift_backend.rs` lines ~2620–2677
     (`fs_*`) and ~2837–2857 (`eprintln_str`/`args`/`stdin_*`).

6. **Generated Rust projection exposes `heap_tag`.**
   Test: `RenderTest::test_render_lower_rs` (or a new test) asserts the
   rendered `op_catalogue.rs` contains a `heap_tag` function whose match arms
   return `Some("String")` / `Some("Vec")` / fall through to `None` per entry.
   Production: implement `render_lower_rs`'s (or a sibling render function's)
   `heap_tag` output.

7. **Generated self-hosted projection exposes `op_catalogue_heap_tag`.**
   Test: a `render_lower_vow_block` test asserting the generated block
   contains `fn op_catalogue_heap_tag(name: String) -> String` with the
   expected `if` arms and `""` fallback.
   Production: implement in `render_lower_vow_block`.

8. **Rust: `vow_builtin_to_runtime` still resolves all 19 names after removal.**
   Test: keep `builtins_lower_to_runtime_symbols_and_return_types`'s 19 rows
   unchanged (still green); add the new catalogue-direct test from §2
   asserting `op_catalogue::lookup("fs_read")`, etc.
   Production: delete the 19 arms from `vow_static_builtin_to_runtime`'s match
   in `vow-ir/src/lower/mod.rs`.

9. **Rust: `tag_builtin_result` consults `op_catalogue::heap_tag` for the 6 heap-tagged names.**
   Test: a lowering-level test (existing IR lowering tests that already
   exercise `fs_read`/`args`/etc. producing a tagged `String`/`Vec` result —
   locate via `grep -n "inst_struct_type" vow-ir/src/lower/mod.rs`'s test
   module) must keep passing; add one if none currently pins this behavior
   directly for this family.
   Production: in `tag_builtin_result`, consult `op_catalogue::heap_tag(name)`
   first for names in this family; remove the corresponding hardcoded arms.

10. **Rust: Cranelift ABI tables resolve the 19 symbols from the catalogue.**
    Test: existing codegen/link-level tests that already exercise these
    builtins end-to-end (compile+run a `.vow` fixture using `fs_write`/
    `fs_read`, `args`, `stdin_read_line`, `eprintln_str`) must keep passing
    unchanged — this is the real regression backstop, since ABI mismatches
    surface as crashes or link errors, not type errors.
    Production: in `vow-codegen/src/cranelift_backend.rs` and
    `vow-clif-shim/src/lib.rs`, consult `op_catalogue::abi_sig` before the
    remaining hand-written match arms; delete the 19 migrated arms from each.

11. **Self-hosted: `builtin_to_extern`/`builtin_ret_ty`/heap-tagging resolve from the generated block.**
    Test: existing self-hosted test coverage exercising these builtins
    (`compiler/tests/*.vow` fixtures, plus the bootstrap triple test) must
    keep passing. If no existing self-hosted unit test directly pins
    `builtin_to_extern("fs_read")` / `builtin_ret_ty("fs_read")`, add one in
    `compiler/tests/` following the existing pattern for other builtins.
    Production: run `scripts/generate_ops.py` to regenerate the
    `GENERATE:OP_CATALOGUE` block in `compiler/lower.vow`; remove the 19
    hand-written arms from `builtin_to_extern`/`builtin_ret_ty` and the 6
    inline heap-tag `if` blocks, replacing the latter with a call to the
    generated `op_catalogue_heap_tag`.

12. **CI freshness enforcement covers the grown catalogue.**
    Preflight (not a test to write, a check to run): grep
    `scripts/full_test.sh` for `generate_ops.py`. If absent (true as of
    #1270 @ 473db70a), add a step invoking
    `uv run python scripts/generate_ops.py --check`, mirroring the existing
    `check_help_coverage.py` wiring at `scripts/full_test.sh`'s help-drift
    section (~line 1321). If #1270 already added this wiring by merge time,
    skip this — just confirm the check passes with the grown catalogue.

## 4. Verification surface

This slice does not touch contracts, `vow-verify`, or the C model. All 19
operations carry non-empty effect sets in
`vow-types/src/env.rs::builtin_free_fn_signatures()` (`Effect::Read` for
`fs_read`/`fs_open`/`fs_read_line`/`fs_status`/`fs_close`/`fs_exists`/
`fs_is_dir`/`fs_is_symlink`/`args`/`stdin_read`/`stdin_read_line`/
`stdin_ready`, at lines ~159–182 and ~237–240; `Effect::Write` for `fs_write`
at ~164; `Effect::IO` for `fs_mkdir`/`fs_remove`/`fs_remove_dir`/`fs_rename`/
`eprintln_str` at ~171/178/179/182/236). `vow-verify/src/c_emitter.rs::is_modelable`
short-circuits (`if !func.effects.is_empty() { return false; }`) before ever
reaching `is_known_builtin`, so none of these 19 operations exercise the
verifier's known/modelable classifier — matching the issue body's own framing
("does not exercise the verifier's `is_known_builtin`/`is_modelable` gate").
**No verifier-model category is recorded for this slice; the acceptance
criterion "if any migrated operation turns out to be pure ... record its
category" is satisfied by the negative: none are pure.**

Arena-routing: none of these 19 runtime symbols have `_in_arena` variants
(confirmed by grep across `vow-runtime/src/lib.rs`), so all are root-arena-only
— there is no root-vs-candidate-arena routing choice to record for this
family, only the heap-result *tag* (`String`/`Vec`, for `pin_to_root`) covered
in §2/§3. No new `tests/run/` or `examples/` fixtures should be needed since
existing fixtures already exercise these builtins end-to-end; if the
implementer finds a gap (e.g. no existing `.vow` fixture round-trips
`fs_listdir` or `args` through a full compile+run), add a minimal one under
`tests/run/` rather than skipping coverage.

## 5. Risk areas

- **Binary fixed point:** `render_lower_vow_block`'s output is sorted by
  `surface_name` (per `load_catalogue`'s explicit sort), so adding 19 entries
  changes the generated block's content but not its determinism. Run the
  bootstrap triple test (`scripts/concat_vow.sh` + stage 0/1/2 SHA comparison)
  after regenerating, since `compiler/lower.vow` changes are exactly the kind
  of self-hosted edit that can silently break the fixed point if the
  generated block is hand-touched instead of regenerated.
- **rustfmt reformatting `op_catalogue.rs`:** after `generate_ops.py` writes
  the Rust projection files, run `cargo fmt --all` before `cargo build` (the
  generator's own trailing instructions already say this) — otherwise CI's
  `cargo fmt --check` (if any) or review diffs will show spurious formatting
  churn mixed with real content changes.
- **`cargo clippy --all -- -D warnings`:** the new `heap_tag`/`abi_sig`
  consultation sites in `tag_builtin_result` and the Cranelift backends
  introduce a new early-return/match-and-fallback shape; make sure the
  fallback match on the now-smaller hand-written arm lists doesn't trip
  `clippy::match_single_binding` or similar if a match shrinks to very few
  arms. Test-only lints are out of scope per project convention (CI runs
  `cargo clippy --all -- -D warnings`, no `--all-targets`).
- **`parse → print → parse` idempotency:** unaffected — this slice changes no
  syntax, AST, or printer behavior.
- **Self-hosted `GENERATE:OP_CATALOGUE` marker drift:** if #1270 lands with a
  different marker shape or anchor than described here (e.g. a different
  insertion point in `compiler/lower.vow`), re-run `generate_ops.py` fresh
  rather than hand-patching around a stale assumption.
- **`check_grammar_presence` generalization regressing #1270's own test:**
  the rename/generalization in slice 4 must keep
  `test_real_catalogue_matches_grammar_print_io_table` (or its renamed
  successor) passing for `print_*`'s existing `"Print / IO"` table membership
  — verify this explicitly before adding the new tables.

## 6. Out of scope

- `gzip_write_file` — not `fs_`-prefixed, not listed in `grammar.md`'s
  `Filesystem` table (it lives under a different section), and not named in
  this issue's title. Leave for #1271 (query/utility) or a future slice.
- `process_get_stderr` / `process_stderr_for` — these read *subprocess*
  captured stderr, not direct stderr output; they belong to #1273's process
  family, not this issue's "stderr" (which is `eprintln_str`, the direct
  stderr-output builtin analogous to `print_str`).
- `debug_str` / `debug_i64` / `debug_u64` — routed through a distinct
  `IOP_DEBUG_CALL` path (`vow_debug_builtin_to_runtime`), not the generic
  direct-call path this catalogue slice covers; explicitly excluded by the
  PRD's "first vertical slice" framing and not named in this issue.
- `string_*` helpers, `parse_*`, numeric conversion intrinsics, `vec_sort`,
  `time_*`, `hex_*`, `memory_*` — all belong to #1271 (query/utility slice),
  not this issue.
- Verifier classifier changes (`is_known_builtin`/`is_modelable`) — belongs to
  #1274; this slice's operations are all effectful and never reach that gate.
- Any change to `docs/spec/grammar.md`'s prose, signatures, or effects for
  these 19 operations — behavior is preserved exactly; the catalogue is
  populated from what already exists, not the other way around.
- Broader "harden the catalogue" work (schema documentation polish, additional
  generic validation beyond what this slice's own entries require) — belongs
  to #1275. The `check_env_rs_presence` regex fix and `check_grammar_presence`
  generalization in this plan are included only because #1272's own 19
  entries cannot pass the existing cross-checks without them, not as general
  hardening.
- Rewriting or refactoring unrelated parts of `compiler/lower.vow`,
  `vow-codegen/src/cranelift_backend.rs`, or `vow-ir/src/lower/mod.rs` beyond
  the specific arms/tests this slice touches.

## 7. Operating-contract note

Planning against an unmerged blocking issue (#1270) is a judgement call made
under the "make a best-effort decision and document it" contract. A comment
will be posted on #1272 stating: this plan is pinned to #1270's branch shape
as of commit `473db70a` (still `sym:running`, unmerged); implementation must
not begin until #1270 merges to `main`, must rebase onto post-merge `main`
rather than building on #1270's feature branch directly, and must re-verify
every cited path/name against the actually-merged shape.
