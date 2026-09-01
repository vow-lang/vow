# Plan: #1154 — c_emitter: build a per-function instruction-type table instead of rescanning f.blocks

## 1. Problem restated

`compiler/c_emitter.vow`'s `lookup_inst_ty(f: IrFunction, id: i64) -> i64` (currently
`c_emitter.vow:591-608`) resolves an operand instruction's IR type by nested-`while`-scanning
every block and every instruction of the enclosing function, returning `ITY_I64()` if no
instruction with `.id == id` is found. It is called once per indexed container access — from
`emit_vec_op` for `__vow_vec_get_val` and `__vow_vec_set_val` (currently lines 1827, 1854) and
from `emit_string_op` for `__vow_string_byte_at` (currently line 2084) — so a function with `N`
indexed operations does `O(N)` full-function rescans, i.e. `O(N²)` IR traversal, inside the
self-hosted C emitter that backs `vowc verify`. The fix is to build a dense per-function
`id -> type` lookup table once (mirroring the project's existing `build_id_to_inst`
(`compiler/region.vow:4155-4189`) and `lctx_inst_ty`/`inst_ty_by_id`
(`compiler/lower.vow:11,221-243`) idioms) and thread it into `emit_c_function` so all three call
sites become `O(1)` array reads instead of full rescans.

Line numbers throughout this plan are current as of this planning session; re-`grep` function
names rather than trusting the exact numbers by implementation time, since the file will have
shifted.

Verified during planning: `grep -rn "lookup_inst_ty\|emit_vec_op\|emit_string_op\|emit_inst("
compiler/ --include='*.vow'` finds every reference confined to `compiler/c_emitter.vow` itself —
no other module or test file calls `lookup_inst_ty`, `emit_vec_op`, or `emit_string_op` directly,
so the signature changes in §2 are self-contained to this one file plus its own test file.

## 2. Files to touch

- **`compiler/c_emitter.vow`** (production, self-hosted only):
  - Add `fn build_inst_ty_by_id(f: IrFunction) -> Vec<i64>` near `lookup_inst_ty` (~line 588).
    Guard against negative instruction ids when populating the table (`if inst.id >= 0 { ... }`,
    mirroring `build_id_to_inst` at `compiler/region.vow:4182` and `lctx_record_inst_ty`'s
    `if inst_id < 0 { return; }` at `compiler/lower.vow:232-234`) — an unguarded
    `tbl[inst.id] = inst.ty` with a negative id would panic during table construction.
  - Add `fn inst_ty_of(inst_ty_by_id: Vec<i64>, id: i64) -> i64` as a **new, separate** function
    (do not rewrite `lookup_inst_ty` in place yet — see §3 slice ordering) — a bounds-checked O(1)
    read that preserves the exact current default-to-`ITY_I64()` fallback (mirroring
    `compiler/lower.vow:221-229`'s `lctx_inst_ty` body). Once all three call sites are rewired to
    `inst_ty_of` in slice 3, delete the old `fn lookup_inst_ty(f: IrFunction, id: i64)` (~591-608)
    entirely — its linear-scan body is fully superseded, no dead code left behind.
  - `emit_vec_op` (~1765): replace the unused-after-this-change `f: IrFunction` parameter with
    `inst_ty_by_id: Vec<i64>`; update its 2 `lookup_inst_ty(f, idx)` call sites (~1827, 1854) to
    `inst_ty_of(inst_ty_by_id, idx)`.
  - `emit_string_op` (~1953): **add** `inst_ty_by_id: Vec<i64>` as a new parameter (keep `f` —
    it is still needed by the unrelated `__vow_string_matches_literal_at` scan at ~2147-2167,
    out of scope here); update its 1 `lookup_inst_ty(f, idx)` call site (~2084) to
    `inst_ty_of(inst_ty_by_id, idx)`.
  - `emit_inst` (~1300): add `inst_ty_by_id: Vec<i64>` parameter; thread it into both the
    `emit_vec_op(...)` call (~1604) and the `emit_string_op(...)` call (~1608).
  - `emit_c_function` (~2480): call `build_inst_ty_by_id(f)` once, right after
    `let blocks: Vec<IrBlock> = f.blocks;` (~2529), bind it to `inst_ty_by_id`, and pass it
    through both `emit_inst(...)` call sites (~2848, 2878).
- **`compiler/tests/test_c_emitter.vow`** (tests, self-hosted only): add unit coverage for the
  new table builder/accessor and one multi-block emission-level regression test (see §3).
- **No Rust crate changes.** `vow-verify/src/c_emitter.rs` already builds an equivalent
  `inst_by_id: HashMap<u32, &Inst>` once per function (`vow-verify/src/c_emitter.rs:2401-2405`)
  and reads it via `operand_ty` (`vow-verify/src/c_emitter.rs:162-163`) — this is the Rust side
  of the parity gap the issue describes, and it is already correct. Confirmed by reading the
  file directly; nothing to change there.
- **No `docs/spec/*.md` changes.** This is an internal codegen performance fix to the ESBMC-model
  emitter with no observable change to Vow syntax, semantics, builtins, operators, effects, or
  CLI flags — the emitted C text is required to be byte-identical to before (see §4).

Test execution command (confirmed from `docs/spec/cli.md` and `scripts/full_test.sh`'s Section
10b, which runs the same form against both compilers for parity): `vow test` auto-discovers every
`test_*.vow` under a directory, each with its own `main() -> i32` returning 0 on success. Run
`build/vowc test compiler/ --filter c_emitter` to build+run just `test_c_emitter.vow` (module
resolution against `compiler/` as the scan root, per the directory-scan rule in `cli.md`) during
slices 1-4, and the unfiltered `build/vowc test compiler/` (or `scripts/full_test.sh`'s Section
10b) as the final gate.

1. **`build_inst_ty_by_id` unit tests** (`compiler/tests/test_c_emitter.vow`, new `fn
   check_inst_ty_by_id_table()`, registered in `main()` as a new `r18` with base error codes
   starting at `200` — clear of the `70-140+` range `check_bounds_assert_signedness` already
   uses), production code: `build_inst_ty_by_id` in `c_emitter.vow`.
   - Single-block function: table entry at each real instruction's id equals `inst.ty`.
   - **Multi-block function** where the id being queried is defined in an earlier block than the
     block doing the lookup (use `tests.builders::mk_block`/`mk_function` with 2+ blocks) — this
     is the one behavior the existing test suite never exercises (every existing indexed-access
     fixture uses a single block), and it's exactly the cross-block case the old nested-`while`
     scan handled and the new table must preserve.
   - A negative instruction id present in a fixture's inst list (if the builders allow
     constructing one) must not be written into the table and must not panic — proves the
     `if inst.id >= 0` guard.
   - Empty-function edge case (no blocks / no instructions) returns an empty table without
     indexing errors — mirrors `build_id_to_inst`'s `max_id = -1` handling in `region.vow:4157`.
   This slice is genuinely red/green on its own: `build_inst_ty_by_id` does not exist yet, the
   test fails to compile, then the implementation makes it pass. It does not touch
   `lookup_inst_ty` or any existing call site, so nothing else in the file changes.
2. **`inst_ty_of(inst_ty_by_id, id)` unit tests**, production code: the new accessor function
   (added alongside the still-present, still-used-by-nobody-new `lookup_inst_ty` — see §2's note
   on why this is a distinct function rather than an in-place rewrite: Vow has no overloading, so
   rewriting `lookup_inst_ty`'s signature in place would break its 3 existing call sites before
   slice 3 gets a chance to rewire them, collapsing slices 2 and 3 into one non-independently-
   green unit).
   - In-range hit returns the stored type.
   - In-range gap (id within `[0, len)` but never written — shouldn't occur once slice 1 is
     correct, but the accessor's own bounds/default logic should be tested in isolation) and
     out-of-range id (negative, and `>= inst_ty_by_id.len()`) both return `ITY_I64()`.
   Red: `inst_ty_of` doesn't exist, test fails to compile. Green: implement it, matching
   `lctx_inst_ty`'s guard shape (`compiler/lower.vow:221-229`). `lookup_inst_ty` is untouched and
   still compiles and passes its own (unmodified) callers at this point.
3. **Wire the table through `emit_c_function` → `emit_inst` → `emit_vec_op`/`emit_string_op`,
   then delete `lookup_inst_ty`.** Production code: the signature/call-site changes in §2 —
   rewire all 3 `lookup_inst_ty(f, idx)` call sites to `inst_ty_of(inst_ty_by_id, idx)`, thread
   `inst_ty_by_id` through `emit_inst`/`emit_c_function`, then delete the now-dead
   `fn lookup_inst_ty(f: IrFunction, id: i64)` (~591-608) in the same slice — no dead code left
   behind. Test: this slice's "red" state is the self-hosted compiler failing to build
   `compiler/c_emitter.vow` (Vow is statically typed, so a missed call-site update or a leftover
   reference to the deleted `lookup_inst_ty` is a compile error, not a silent bug) — confirm with
   `build/vowc build --no-verify compiler/main.vow -o /tmp/vow_main` (or the equivalent
   `scripts/concat_vow.sh` + stage-0 build). "Green" is: that build succeeds, and
   `build/vowc test compiler/ --filter c_emitter` — the **entire existing**
   `compiler/tests/test_c_emitter.vow::main()` suite (17 checks, including
   `check_bounds_assert_signedness` at ~line 497, which already exercises all 8 signed/unsigned
   index types through `__vow_vec_get_val`/`__vow_vec_set_val`/`__vow_string_byte_at` and asserts
   on the exact emitted `__ESBMC_assert(...)` text) — still passes byte-for-byte unchanged. These
   existing tests are the regression guard that the table-based lookup produces identical output
   to the old linear scan for every case they already cover.
4. **New multi-block emission-level regression test** (`compiler/tests/test_c_emitter.vow`, new
   `fn check_bounds_assert_cross_block()` alongside `check_bounds_assert_signedness`, registered
   in `main()` as `r19` with its own base code, e.g. `220`): build an
   `indexed_container_function`-style fixture (reusing/extending the existing helper at ~463) but
   split across two blocks so the index-defining `mk_inst_arg` lives in block 0 and the
   `__vow_vec_get_val`/`__vow_vec_set_val`/`__vow_string_byte_at` call lives in block 1, then
   assert the emitted `__ESBMC_assert` text matches the same signed/unsigned form as the
   single-block case. This is the emission-level (not just unit-level) proof that threading
   `inst_ty_by_id` through `emit_c_function`/`emit_inst`/`emit_vec_op`/`emit_string_op` preserves
   cross-block resolution end to end. Run via `build/vowc test compiler/ --filter c_emitter`.

## 4. Verification surface

This change touches the C model the self-hosted emitter generates for ESBMC, but changes no
contract semantics and adds no new verification obligation — the goal is byte-identical emitted
C for every existing case, plus correct output for the previously-untested multi-block case.

- **Emitted-text regression, not new ESBMC properties.** The `__ESBMC_assert(...)` bounds checks
  emitted by `emit_bounds_assert` are unchanged in form; only how `idx_ty` is computed changes
  (O(1) table read vs. O(n) scan). No new `--property` class is introduced.
- **Differential check against the issue's own evidence.** The issue cites 26 `esbmc.c` files
  captured from `vowc verify examples/sat/solver.vow` as the most index-heavy contracted code in
  the repo. Re-run `vowc verify examples/sat/solver.vow` before and after the change (with
  `VOW_CACHE_DIR=$(mktemp -d)` per the known compile-cache staleness gotcha — stale objects can be
  served after a compiler rebuild at the same source rev) and diff the emitted `esbmc.c` output;
  it must be identical, and the verification result (proof/counterexample) must be unchanged.
- **`compiler/tests/test_c_emitter.vow` full suite** must pass (existing 17 checks + the 2 new
  ones from slices 1 and 4), via `build/vowc test compiler/ --filter c_emitter`.
- **`scripts/full_test.sh`** should be run at the end, since it is the project's standard gate
  and will catch any drift in other self-hosted suites that happen to exercise `c_emitter.vow`
  indirectly (e.g. any `tests/run/*.vow` fixtures with `vow`-blocked Vec/String indexing that go
  through `vowc verify`).
- No new fixtures are required under `tests/run/` or `examples/` — the existing `solver.vow`
  differential check plus the new unit/emission tests in `test_c_emitter.vow` give adequate
  coverage for a pure internal-lookup refactor with no observable semantic change.

## 5. Risk areas

- **Cross-block correctness** is the main semantic risk: the old scan walked *all* blocks, so the
  new table must too. `build_inst_ty_by_id` must iterate every block in `f.blocks`, not just the
  entry block — slice 1's and slice 4's multi-block tests exist specifically to catch a
  regression here.
- **Fallback-default preservation.** The original `lookup_inst_ty` returns `ITY_I64()` for *any*
  unresolved id (out of range or a genuine gap), and callers rely on this to avoid a crash on
  malformed/unexpected operand ids in `vowc verify` on arbitrary user programs. `inst_ty_of` must
  keep an explicit bounds check (`id < 0 || id >= inst_ty_by_id.len()`) before indexing — an
  unchecked `inst_ty_by_id[id]` would turn a soft fallback into a possible runtime panic, which is
  a robustness regression, not just a refactor. Mirror `lctx_inst_ty`'s guard
  (`compiler/lower.vow:221-229`) exactly. Symmetrically, `build_inst_ty_by_id` must guard against
  negative ids on the *write* side too (`if inst.id >= 0 { ... }`, mirroring
  `region.vow:4182`/`lower.vow:232-234`) — an unguarded write with a negative id panics during
  table construction, before any lookup even happens.
- **`emit_string_op` keeps `f`.** It's tempting to drop the `f: IrFunction` parameter entirely
  once `lookup_inst_ty` no longer needs it, but `emit_string_op` still uses `f.blocks` for the
  unrelated `__vow_string_matches_literal_at` literal-folding scan (~2147-2167, out of scope —
  see §6). Removing `f` there would be a compile error forcing a bigger diff than intended;
  `emit_vec_op` is the only one of the two where `f` becomes fully unused and removable.
- **Binary fixed point.** `compiler/c_emitter.vow` is one of the self-hosted compiler's own
  source modules, so this change flows through `scripts/bootstrap.sh`'s stage 0/1/2 triple build.
  The table is a plain `Vec<i64>` built by a single deterministic forward scan (no `HashMap`, no
  iteration-order dependence — reads are direct-index, not iteration), so it should not affect
  determinism, but re-run the triple-bootstrap `sha256sum` check
  (`scripts/concat_vow.sh` → stage 0/1/2 → compare) before calling this done, since it's a change
  to compiler source and the project's own discipline requires confirming this rather than
  assuming it.
- **No `parse → print → parse` idempotency risk** — no grammar, AST, or printer changes.
- **No `cargo clippy --all -- -D warnings` risk** — no Rust files touched.
- **Signature churn is compiler-checked, not silent.** Vow is statically typed; any missed
  call-site update to `emit_vec_op`/`emit_string_op`/`emit_inst` is a build failure in slice 3,
  not a latent bug — low risk of this landing broken, but it does mean the self-hosted compiler
  must actually be rebuilt (not just have its `.vow` source edited) to validate each slice.

## 6. Out of scope

- **`emit_string_op`'s `__vow_string_matches_literal_at` scan** (`c_emitter.vow:2147-2167`,
  inside `emit_string_op`): this is the *same class* of bug (a full `f.blocks` rescan per
  relevant instruction) but needs a full `Vec<IrInst>` table (to recover `candidate.dv`/`.op`,
  not just `.ty`), which `build_id_to_inst` in `region.vow:4155-4189` already provides as a
  precedent but is not itself reusable here without checking module visibility/import cost. It
  is not one of the three call sites named in issue #1154, and folding it in would turn a
  minimal, reviewable fix into a mixed-scope change. Left as an explicit fast-follow: file a
  follow-up issue citing this plan if the maintainer wants it addressed.
- **The other ~9 `f.blocks` full scans in `compiler/c_emitter.vow`** (`is_modelable`,
  `first_unsupported_opcode_name`, `collect_callees_dfs`, `collect_modelled_vars`,
  `collect_option_vars`, `collect_wide_vars`, `detect_const_fns`, `func_has_checked_arith_op`,
  `func_has_integer_shift_op`): each of these is invoked once (or a small constant number of
  times) per function during module emission, not once per instruction within that same
  function — so each contributes `O(n)` per function today, not `O(n²)`. They are not the bug
  this issue describes. The issue itself frames "convert all ~10 linear scans" as an optional
  bonus, not a requirement; per this project's "many small changes beat one large change"
  discipline, they are deliberately not bundled into this PR.
- **No Rust-side changes** — parity already exists (see §2).
- **No `docs/spec/*.md` changes** — no observable language/CLI change.
- **No renaming or formatting cleanup** of unrelated code in `c_emitter.vow` beyond the minimal
  signature threading described in §2.
