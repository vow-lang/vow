# PLAN: Migrate process Builtin Operations to the Operation Catalogue (#1273)

## §0. Preflight — #1270 must land first

As of 2026-09-13, `origin/main` HEAD is `0cf9d948`. Issue #1270 (the Operation Catalogue
tracer bullet: `docs/spec/operations.json`, `docs/spec/schemas/operation-catalogue.schema.json`,
`scripts/generate_ops.py`, the generated `op_catalogue.rs` modules, and the
`// GENERATE:OP_CATALOGUE` block in `compiler/lower.vow`) is **not merged** — no PR exists for
it, and `origin/main` has none of this infrastructure. Its work-in-progress branch
`sym/vow/1270-tracer-bullet-catalogue-driven-print-builtin-operations-abi-runtime-symbol-facts`
exists locally (shared object store across worktrees) with 5 commits and is based exactly on
`origin/main`'s current tip, so it is a clean, non-diverged preview of what #1270 will land.
Sibling issues #1271 and #1272 hit this identical blocker during their own planning/preflight
runs and independently concluded: do not scaffold the catalogue infrastructure inside this
issue's slice (it would race #1271/#1272, who would build the same files), and do not cherry-pick
#1270's commits into this branch.

This plan is written against **#1270's branch shape** (verified directly by reading its diff
against `origin/main`, commit `473db70a` et al.), citing concrete function/file names, but by
**role**: implementation must re-verify every cited path and shape against whatever actually
lands on `main` once #1270 merges, since names could still change before that PR is reviewed.

**Implementation-stage step 1 (hard precondition, before any other slice):**
1. `git log origin/main --grep 1270` and `gh pr list --search 1270 --state all` — confirm a
   merged PR exists.
2. `git show origin/main:docs/spec/operations.json` and
   `git show origin/main:scripts/generate_ops.py` — confirm both exist on `main`.
3. If either check fails, do **not** build the catalogue scaffolding yourself. Post
   `gh issue comment 1273` stating #1270 is still unmerged, and exit cleanly without applying a
   handoff label (matching #1271/#1272's precedent) — the branch keeps this `PLAN.md`, ready to
   execute once #1270 lands.
4. If both checks pass, `git rebase` (or merge) this branch onto post-merge `main`, then
   re-read the actual `docs/spec/operations.json`, `scripts/generate_ops.py`,
   `vow-ir/src/lower/op_catalogue.rs`, and `compiler/lower.vow`'s `GENERATE:OP_CATALOGUE` block
   to confirm the shapes below still match before starting slice 2.

A `gh issue comment 1273` has been posted from the planning stage recording the same
assumption and the two scope decisions in §7/§8 below, so a reviewer doesn't have to re-derive
them from this file alone.

## §1. Problem restated

Eleven `process_*` Builtin Operations (`process_exit`, `process_run`, `process_get_stdout`,
`process_get_stderr`, `process_start`, `process_wait`, `process_wait_timeout`,
`process_poll_wait`, `process_kill`, `process_stdout_for`, `process_stderr_for`) each have their
runtime-symbol, Cranelift ABI signature, and IR return-shape facts hand-duplicated across five
Rust sites (`vow-ir/src/lower/mod.rs::vow_static_builtin_to_runtime`, its lockstep test table,
`vow-codegen/src/cranelift_backend.rs::make_extern_sig`, `vow-clif-shim/src/lib.rs::make_extern_sig`)
and two self-hosted sites (`compiler/lower.vow`'s `builtin_to_extern`/`builtin_ret_ty` if-chains).
All eleven are `[Effect::IO]`-tagged, so none reach the verifier's `is_known_builtin`/`is_modelable`
gate — this slice is a pure ABI/runtime-symbol/return-shape/doc migration, structurally identical
to the print tracer bullet (#1270) and the filesystem/stdin/args/stderr slice (#1272), just with
more entries and four `Ptr`-returning (heap) operations instead of print's three `Unit`-returning
ones. The fix is to add these eleven operations to the Operation Catalogue
(`docs/spec/operations.json`) and make both compilers' lowering/codegen call paths consult the
catalogue's generated projection instead of their hand-written match arms, exactly as #1270 did
for `print_*`.

## §2. Concrete operation inventory (verified against `origin/main`)

| surface_name | runtime_symbol | ir_return_shape | abi_params | abi_return | vow-types signature (env.rs:241-271) |
|---|---|---|---|---|---|
| `process_exit` | `__vow_process_exit` | `Unit` | `[I64]` | `null` | `(i64) -> Never [IO]` |
| `process_run` | `__vow_process_run` | `I64` | `[I64, I64]` | `I64` | `(Str, Vec<Str>) -> I64 [IO]` |
| `process_get_stdout` | `__vow_process_get_stdout` | `Ptr` | `[]` | `I64` | `() -> Str [IO]` |
| `process_get_stderr` | `__vow_process_get_stderr` | `Ptr` | `[]` | `I64` | `() -> Str [IO]` |
| `process_start` | `__vow_process_start` | `I64` | `[I64, I64]` | `I64` | `(Str, Vec<Str>) -> I64 [IO]` |
| `process_wait` | `__vow_process_wait` | `I64` | `[I64]` | `I64` | `(I64) -> I64 [IO]` |
| `process_wait_timeout` | `__vow_process_wait_timeout` | `I64` | `[I64, I64]` | `I64` | `(I64, I64) -> I64 [IO]` |
| `process_poll_wait` | `__vow_process_poll_wait` | `I64` | `[I64, I64]` | `I64` | `(I64, I64) -> I64 [IO]` |
| `process_kill` | `__vow_process_kill` | `I64` | `[I64]` | `I64` | `(I64) -> I64 [IO]` |
| `process_stdout_for` | `__vow_process_stdout_for` | `Ptr` | `[I64]` | `I64` | `(I64) -> Str [IO]` |
| `process_stderr_for` | `__vow_process_stderr_for` | `Ptr` | `[I64]` | `I64` | `(I64) -> Str [IO]` |

Pinned facts that must not change during this migration:
- `process_exit`'s vow-types signature returns `Ty::Never`, but `vow-ir::Ty` has no `Never`
  variant (checked `vow-ir/src/types.rs:236-253`) and the lowering site has always recorded
  `Ty::Unit` for it (`vow-ir/src/lower/mod.rs:123`, `compiler/lower.vow:1620`). The catalogue
  entry records `ir_return_shape: "Unit"` to preserve this pre-existing behavior; this is not a
  bug this slice fixes.
- `docs/spec/grammar.md`'s `#### Process Management` table (line ~1213) currently lists only
  **10** of the 11 operations — `process_poll_wait` is missing (also missing from
  `vow/src/skill.rs`'s hand-listed JSON, which is generated from this same table, so the gap is
  consistent, not a separate divergence). This is a genuine pre-existing spec gap uncovered by
  the catalogue's own `check_grammar_presence` cross-check once `process_poll_wait` is added to
  the catalogue. Fixed in slice 2 below (in scope — the catalogue can't validate this operation
  otherwise, unlike #1271's unrelated `#1276` finding which was filed separately because nothing
  in that slice depended on it).

## §3. Files to touch

**Rust (`crates/` equivalents at repo root):**
- `docs/spec/operations.json` — append the 11 entries from §2.
- `docs/spec/grammar.md` — add the missing `process_poll_wait` row to `#### Process Management`.
- `scripts/generate_ops.py` — broaden `check_grammar_presence` from the hardcoded
  `heading="Print / IO", heading_level=4` to `heading="Builtin Function Signatures",
  heading_level=3` (this H3 already accumulates every H4 subsection's tables, including
  `Process Management` — confirmed via `extract_table`'s level-comparison logic in
  `scripts/generate_help.py`, which already uses this same H3 to build the master `--help`
  builtins table). Update the function's error message accordingly (it currently says
  `"Print / IO table"`).
- `scripts/test_generate_ops.py` — update `LoadCatalogueTest.test_valid_catalogue_loads`'s
  hardcoded `{"print_str", "print_i64", "print_u64"}` set to the full 14-name union; extend
  `RenderTest`'s fixture with a non-`Unit`/non-null-return case (`process_run`: `abi_return: I64`)
  and a `Ptr`-return case (`process_get_stdout`) so `render_lower_rs`/`render_abi_rs`'s currently
  print-only-exercised branches get real coverage; rename or extend
  `test_real_catalogue_matches_grammar_print_io_table` to reflect the broadened heading (cosmetic,
  but keep the assertion of an empty error list against the *real* catalogue + grammar.md).
- `vow-ir/src/lower/mod.rs` — remove the 11 `process_*` arms from
  `vow_static_builtin_to_runtime` (currently lines 123-133); add a dedicated
  `op_catalogue_process_builtins_resolve_from_the_generated_projection` test mirroring #1270's
  print test; leave `process_*` entries in the existing
  `builtins_lower_to_runtime_symbols_and_return_types` lockstep table untouched (that table is the
  single end-to-end behavior pin for every builtin, print or process).
- `vow-codegen/src/cranelift_backend.rs` — remove the 11 `"__vow_process_*"` arms from
  `make_extern_sig`'s hand-written match (currently ~lines 2858-2896); add
  `process_builtins_extern_sigs_come_from_the_operation_catalogue` test mirroring #1270's.
- `vow-clif-shim/src/lib.rs` — same removal (currently ~lines 3753-3790) and same test, kept
  byte-identical to the codegen crate's copy per the existing "keep in sync" convention.

**Self-hosted (`compiler/`):**
- `compiler/lower.vow` — after running `scripts/generate_ops.py` to regenerate the
  `GENERATE:OP_CATALOGUE` block (now containing 14 `if` lines per helper instead of 3), manually
  remove the 11 now-redundant hand-written `if name == String::from("process_*") { return ...; }`
  lines from `builtin_to_extern` (currently lines 1514-1524) and `builtin_ret_ty` (currently lines
  1620-1630) — mirroring exactly how #1270 hand-deleted the `print_*` lines after regenerating the
  marker block (the generator only rewrites content *between* the markers; it does not touch the
  surrounding hand-written chains).
- `compiler/tests/test_op_catalogue.vow` — extend with `process_*` assertions analogous to the
  existing `print_*` checks (`op_catalogue_extern`/`op_catalogue_ret_ty` direct checks, plus
  `builtin_to_extern`/`builtin_ret_ty` end-to-end checks for a couple of `process_*` names) and
  keep the existing "unrelated builtin still resolves via fallback" check as-is.

**Tests:**
- `tests/run/process_catalogue_smoke.vow` — new e2e fixture (see §4 slice 10).
- `vow-verify/src/c_emitter.rs` — one small regression test (see §4 slice 9).

**No changes needed to:** `vow-runtime/src/lib.rs` (runtime symbol implementations and linkage
are unchanged — only the lowering/codegen *tables that name them* move), `vow-types/src/env.rs`
(signatures/effects are read, not modified, per the PRD's explicit scoping),
`vow-verify/src/c_emitter.rs::is_known_builtin`/`is_modelable` bodies (confirmed no `process_*`
symbol appears there today — see §5 slice 9), `compiler/c_emitter.vow` (same), and
`compiler/main.vow`'s hand-written help-JSON `push_str` calls (fully regenerated by
`scripts/generate_help.py` from `grammar.md`; #1270 needed no direct edit there either, since the
values were already correct — same expectation here once the `process_poll_wait` row is added).

## §4. TDD slices

1. **Preflight** (§0) — confirm #1270 merged, rebase, re-verify shapes. Not a code change; gates
   every slice below.
2. **Doc gap: add the missing `process_poll_wait` grammar row.** Red: none yet (this is additive
   prose, not test-driven in the usual sense) — but it becomes a hard prerequisite the moment
   slice 5's catalogue entry for `process_poll_wait` is added, since `check_grammar_presence`
   would otherwise fail. Add the row to `docs/spec/grammar.md`'s `Process Management` table, then
   run `uv run python scripts/generate_help.py` and confirm the only diff is the new
   `process_poll_wait` line propagating into `vow/src/skill.rs`,
   `skills/vow/reference/grammar.md`, and `compiler/main.vow`'s help JSON — no unrelated
   reformatting.
3. **Broaden `check_grammar_presence`'s heading target.** Red: write a `test_generate_ops.py`
   case that calls `check_grammar_presence` with a fake entry whose `surface_name` lives only
   under a non-`Print / IO` H4 (e.g. `"process_exit"`, expected to already be present in
   `Process Management` once slice 2 lands) and assert it currently fails against the old
   hardcoded heading. Green: retarget the heading to `"Builtin Function Signatures"`/H3, rerun —
   passes. Update the existing `test_real_catalogue_matches_grammar_print_io_table` name/assert.
4. **Catalogue data: append the 11 `process_*` entries.** Red: `uv run python
   scripts/generate_ops.py --check` fails (entries present in `operations.json` and cross-checks
   pass, but generated files haven't been regenerated yet — drift detected). Also update
   `LoadCatalogueTest.test_valid_catalogue_loads`'s hardcoded name set and add
   `process_run`/`process_get_stdout` to `RenderTest`'s fixture coverage (red until the renderer
   assertions for non-`Unit` return shape and non-null `abi_return` are added). Green: run
   `uv run python scripts/generate_ops.py` to regenerate `vow-ir/src/lower/op_catalogue.rs`,
   `vow-codegen/src/cranelift_backend/op_catalogue.rs`, `vow-clif-shim/src/op_catalogue.rs`, and
   `compiler/lower.vow`'s marker block; `--check` now passes.
5. **Rust runtime-symbol/return-shape consumption.** Red: add
   `op_catalogue_process_builtins_resolve_from_the_generated_projection` in
   `vow-ir/src/lower/mod.rs` asserting `op_catalogue::lookup("process_exit")` etc. — passes
   immediately since slice 4 already regenerated `op_catalogue.rs` (the "red" here is really
   slice 4's `--check`). Green/refactor: remove the 11 `process_*` arms from
   `vow_static_builtin_to_runtime`; rerun `cargo test -p vow-ir` and confirm
   `builtins_lower_to_runtime_symbols_and_return_types` (the lockstep table) still passes
   end-to-end via `vow_builtin_to_runtime`, proving the catalogue path is a transparent
   substitution.
6. **Rust ABI consumption.** Same pattern for `vow-codegen/src/cranelift_backend.rs` and
   `vow-clif-shim/src/lib.rs`: add `process_builtins_extern_sigs_come_from_the_operation_catalogue`
   asserting `extern_sig("__vow_process_run")` etc. match the ABI table in §2, then remove the 11
   hand-written match arms from each file's `make_extern_sig`.
7. **Self-hosted consumption.** After slice 4's regeneration, `compiler/lower.vow`'s
   `GENERATE:OP_CATALOGUE` block already has the 14 entries. Red: `compiler/tests/test_op_catalogue.vow`
   asserts `op_catalogue_extern(String::from("process_exit"))` etc. before the hand-written
   `builtin_to_extern`/`builtin_ret_ty` lines are removed — passes trivially since the catalogue
   helper is additive. Green/refactor: remove the 11 redundant `process_*` lines from
   `builtin_to_extern`/`builtin_ret_ty`'s hand-written chains; rerun
   `build/vowc build --no-verify compiler/tests/test_op_catalogue.vow` (or the project's
   self-hosted test runner) and confirm the "unrelated builtin still resolves via fallback" case
   (`fs_read`) still passes, proving the fallback chain is intact.
8. **Regression guard for the untouched pin_to_root heap-tag chain (no production change).**
   §7 explains why this slice deliberately does *not* touch `tag_builtin_result` (Rust) or the
   `lctx_tag` if-chain (self-hosted) that mark `process_get_stdout`/`process_get_stderr`/
   `process_stdout_for`/`process_stderr_for`'s call results as heap `"String"` for `pin_to_root`.
   Add one regression test that pins this today-correct, unmodified behavior so a future refactor
   of either lowering table can't silently drop the tag: assert (via the existing lowering-test
   harness pattern used elsewhere in `vow-ir/src/lower/mod.rs`'s test module) that lowering a
   direct call to each of the four operations still produces a result tagged `"String"` in
   `ctx.inst_struct_type`.
9. **Verifier purity confirmation (closes AC #6).** Red: none of the 11 `__vow_process_*` symbols
   currently appear in `vow-verify/src/c_emitter.rs::is_known_builtin` (confirmed by direct grep
   during planning — zero hits). Add a small test asserting
   `!is_known_builtin("__vow_process_exit")` (and 1-2 more representative symbols) to freeze this
   invariant; this is the concrete, testable form of "verified: no process_* operation is pure, so
   no verifier-classifier category is recorded and no changes to `is_known_builtin`/
   `is_modelable` or their self-hosted mirror `compiler/c_emitter.vow` are needed."
10. **End-to-end smoke test.** `tests/run/process_catalogue_smoke.vow`, using the portable
    `process_start(String::from("sh"), ...)` pattern already established in
    `compiler/main.vow:2300`. Cover both call families so all 11 symbols get at least one
    exercised path: `process_run("sh", ["-c", "echo hello"])` + `process_get_stdout()`/
    `process_get_stderr()` (synchronous path), and `process_start("sh", ["-c", "echo world"])` +
    `process_wait(pid)` + `process_stdout_for(pid)` (async path); assert via `// TEST: stdout`
    only (the project's `full_test.sh` harness ignores `TEST: stderr` — captured to `/dev/null` —
    per this repo's documented two-harness split, so don't rely on stderr assertions for CI
    coverage; use `tests/run_tests.sh`-only `TEST: stderr` lines only as a local-developer bonus,
    not the primary assertion). `process_kill`/`process_wait_timeout`/`process_poll_wait` already
    have dedicated Rust unit coverage in `vow-runtime/src/lib.rs`'s test module
    (`process_poll_wait_captures_stdout`, `process_poll_wait_does_not_kill_running_child`) and
    don't need duplicate `.vow`-level fixtures for this slice; `process_exit` already has
    `tests/run/issue850_never_call_statement.vow`.
11. **Full rebuild and quality gates**, run as separate commands (not `&&`-chained):
    `cargo fmt --all`; `cargo build --release -p vow` (cap `-j` to the attempt's memory share);
    `scripts/bootstrap.sh --skip-cargo`; `cargo test --all` (or targeted
    `-p vow-ir -p vow-codegen -p vow-clif-shim -p vow-verify -p vow` first, then the full suite);
    `cargo clippy --all -- -D warnings`; `uv run python scripts/generate_ops.py --check`;
    `uv run python scripts/generate_help.py --check`; `uvx ruff@<CI-pinned version>` over touched
    Python files; the self-hosted test runner(s) (`scripts/full_test.sh` and, locally,
    `tests/run_tests.sh`) covering the new `process_catalogue_smoke.vow` fixture and
    `compiler/tests/test_op_catalogue.vow`.

## §5. Verification surface

None of the 11 `process_*` operations carry Vow contracts (`requires`/`ensures`) — they are
runtime-implemented builtins, not user-authored Vow functions, so this slice introduces no new
`requires`/`ensures` clauses and no new ESBMC proof obligations. All 11 are `[Effect::IO]`
(confirmed in §2/§3), so `vow-verify/src/c_emitter.rs::is_modelable`'s effect-emptiness gate
already excludes every one of them from the C model — this was true before this migration and
stays true after it (slice 9 adds a regression test pinning that fact, closing AC #6 with
evidence rather than assertion). No `tests/verify/` or `tests/verify-fail/` fixtures need to grow,
and no verifier-model category is recorded in the catalogue for this slice, consistent with the
issue body's explicit statement that this slice is "not expected to exercise the verifier's
`is_known_builtin`/`is_modelable` classifier."

## §6. Risk areas

- **Cross-issue collision on shared files.** #1271 (query/utility) and #1272
  (filesystem/stdin/args/stderr) are also blocked only by #1270, `ready-for-agent`, and their own
  planning runs (already committed to their branches) touch the *same* hybrid files this plan
  touches: `docs/spec/operations.json`, `scripts/generate_ops.py`,
  `vow-ir/src/lower/mod.rs::vow_static_builtin_to_runtime`, both Cranelift extern-sig tables, and
  `compiler/lower.vow`'s `builtin_to_extern`/`builtin_ret_ty`. Whichever of #1271/#1272/#1273
  lands first will shape what the others rebase onto (e.g. #1272 independently designed a new
  `heap_result_tag` catalogue field for its own heap-returning operations — see §7 for why this
  plan deliberately does not need that field, and what to do if #1272's version lands first).
  Before starting implementation, re-check whether either sibling has merged and rebase
  `docs/spec/operations.json`'s new entries as a simple array append (low conflict surface) rather
  than resolving a structural rewrite.
- **Self-hosted determinism.** `scripts/generate_ops.py` sorts catalogue entries by
  `surface_name`/`runtime_symbol` before rendering, so the regenerated `GENERATE:OP_CATALOGUE`
  block and both `op_catalogue.rs` files are deterministic across regenerations — this preserves
  `compiler/lower.vow`'s codegen ordering guarantees and does not put the bootstrap triple test's
  binary fixed point at risk, since no `BTreeMap`/`HashMap` choice or stack-slot layout in
  `vow-clif-shim` is touched by this slice.
- **`parse → print → parse` idempotency** is unaffected — no parser, AST, or printer changes.
- **`cargo clippy --all -- -D warnings`** — removing 11 match arms from three separate
  hand-written matches risks leaving now-unreachable `_ =>` fallback patterns or unused imports if
  a file's only remaining process-related code was those arms; run clippy per-crate after each
  removal (slices 5-6), not just once at the end.
- **The self-hosted `if` line combining `process_*` and `proc_sample` in `compiler/lower.vow`'s
  heap-tag chain** (~line 2546: `if fn_name == "process_get_stdout" || ... || fn_name ==
  "proc_sample" { lctx_tag(...) }`) is a single compound condition today. Because §7 deliberately
  excludes heap-tag migration from this slice, this line is **not edited** — it is only relevant
  as a "do not touch" landmark, and as context for why slice 8's regression test exists (to catch
  anyone who later refactors this line and accidentally drops one of the four `process_*` arms).

## §7. Scope decision: heap-tag (`pin_to_root`) facts are *not* migrated in this slice

Four operations (`process_get_stdout`, `process_get_stderr`, `process_stdout_for`,
`process_stderr_for`) return a heap-allocated `String`, and their call results are separately
tagged `"String"` by `vow-ir/src/lower/mod.rs::tag_builtin_result` and by a parallel hand-written
`if` chain in `compiler/lower.vow` (~line 2527-2551) so that `pin_to_root` knows to treat them as
heap values. This is a real hand-duplicated fact, structurally similar to the runtime-symbol/ABI
facts this slice does migrate — and sibling issue #1272 (whose fs_read/fs_listdir/stdin_read/args
operations have the same shape) independently chose to add a new nullable `heap_result_tag`
catalogue field to cover it.

This plan deliberately does **not** do the same for `process_*`, for three reasons:
1. The issue body's own scope sentence is explicit and narrower: "this migrates
   runtime-symbol/ABI/return-shape/doc facts" — it does not mention heap/arena tagging facts, and
   none of the acceptance criteria name `tag_builtin_result`, `pin_to_root`, or `lctx_tag`.
   AC #4 ("Heap-returning process operations keep correct return shape metadata in both
   compilers") reads naturally as "the migrated `ir_return_shape: Ptr` fact must stay correct,"
   which slices 5-6 already guarantee — not as "own the heap-tag classification."
2. `#1272`'s `heap_result_tag` design is not yet landed or reviewed. Speculatively adopting an
   unreviewed sibling's schema extension risks a real naming/shape mismatch if #1272's actual PR
   changes shape during review, and blocks this issue's implementation on #1272's landing order
   for no benefit `process_*` needs today.
3. Leaving `tag_builtin_result` and the self-hosted `lctx_tag` chain untouched keeps this slice's
   diff exactly as small as the runtime-symbol/ABI/return-shape/doc facts it's chartered to move,
   consistent with "many small changes beat one large change." Slice 8 adds a regression test so
   the untouched behavior is still verified, not merely assumed.

**Follow-up note for whoever implements this issue:** if #1272 has already merged its
`heap_result_tag` field by the time this issue is implemented, it is a trivial, optional follow-on
(not required by this plan) to also add `heap_result_tag: "String"` to the four Ptr-returning
`process_*` catalogue entries and let `tag_builtin_result`/the self-hosted chain consult it —
purely additive, doesn't change this plan's slices 1-11, and can be its own small commit if done.

Also out of scope for the same "not this slice's job" reason: `proc_sample` (not a `process_*`-
prefixed builtin; its Rust `tag_builtin_result` parity bug was already filed as `#1277` and
explicitly claimed by #1271's plan, which is the actual owner of that operation's family) and
`__vow_string_parse_u64_opt`'s self-hosted verifier-classifier gap (`#1276`, unrelated to
`process_*` entirely).

## §8. Out of scope

- Migrating any non-`process_*` Builtin Operations (query/utility: #1271; filesystem/stdin/args/
  stderr: #1272).
- The `heap_result_tag`/`pin_to_root` heap-tag migration for the four `Ptr`-returning `process_*`
  operations — see §7.
- `proc_sample`'s Rust/self-hosted `tag_builtin_result` parity bug (`#1276`) and
  `__vow_string_parse_u64_opt`'s self-hosted verifier gap (`#1277`) — pre-existing, unrelated,
  owned by other issues.
- Any change to `vow-verify/src/c_emitter.rs::is_known_builtin`/`is_modelable` or
  `compiler/c_emitter.vow` — confirmed unnecessary (§5).
- Any change to `vow-runtime/src/lib.rs`'s `process_*` implementations, linkage, or the
  `process_map_init`/handle-table mechanism — only the lowering/codegen tables that *name* these
  symbols move; their behavior is unchanged.
- Rewriting or regenerating `compiler/main.vow`'s help-JSON `push_str` block by hand — it is
  fully derived by `scripts/generate_help.py` from `grammar.md`; this slice only needs the
  `process_poll_wait` grammar row fixed (§3/§4 slice 2) and a regeneration run.
- Broader hardening of `scripts/generate_ops.py`'s cross-checks beyond the one heading-target
  fix this slice actually needs to pass (`check_grammar_presence`) — general robustness work is
  left to whichever issue in the PRD family (#1274/#1275) is chartered for it.
- Any formatting-only or unrelated cleanup in files this slice must touch anyway (e.g. no
  reflowing of unrelated `make_extern_sig` match arms while removing the 11 `process_*` ones).
