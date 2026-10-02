# Plan: issue #981 — dynamic shift-count ESBMC assert width matrix

## 0. Scope correction (read first)

The issue body's premise is **stale**. It was filed from PR #979's triage (2026-07-31,
i32-only fix), but two later PRs already closed most of the gap before this issue was ever
picked up:

- `f421acda` / `1c065b6b` (#976, #979): brought i32 to parity with u8.
- `f5d2a16e8` (#995, 2026-08-17): "make remaining narrow integers first-class" — added the
  dynamic-shift assert **and** a parametric helper generator
  (`narrow_shift_ty_at`, `cemit_narrow_shift_helper`,
  `append_narrow_shift_helpers_for_{fns,function,module_fns}`) covering i8, u8, i16, u16,
  u32 in `compiler/c_emitter.vow`.
- The Rust emitter (`vow-verify/src/c_emitter.rs`) has been **fully width-generic since
  before #979**: `scan_shift_needs`/`ModelHelpers`/`emit_shift_helper` (lines 2519–2710ish)
  walk the IR once per module and emit an assert + masked helper for every
  `(IntegerWidth, IntegerSignedness)` pair actually used, including W128. Verified by reading
  `Opcode::Shl | Opcode::Shr` at `vow-verify/src/c_emitter.rs:1121-1149` (one generic arm, no
  narrow/wide split) and the existing tests `emit_c_module_includes_only_needed_shift_helpers`
  / `emit_c_module_omits_shift_helpers_when_unused` (lines 4843, 4876).

**Actual current state (verified by reading code, not by re-stating the issue):**

| Width | Rust (`vow-verify/src/c_emitter.rs`) | Self-hosted (`compiler/c_emitter.vow`) |
|---|---|---|
| i8, u8, i16, u16, u32 | assert + helper (generic) | assert + helper (`narrow_shift` path, #995) |
| i32 | assert + helper (generic) | assert + helper (dedicated branch, #979) |
| **i64, u64** | assert + helper (generic) | **helper exists, assert MISSING** (lines ~1498–1511) |
| **i128, u128** | assert + helper (generic) | **completely unhandled** — falls to the final `else`, raw native `<<`/`>>` via `emit_cmp`, no mask, no assert |

So this is **not** "8 widths missing, symmetric across both emitters." It is a **self-hosted-only
parity gap**, confined to 4 widths, and the two halves of the gap are different in kind:
i64/u64 silently wrap (soundness gap — a real out-of-range dynamic shift can "verify" when it
shouldn't); i128/u128 hit real C undefined behavior with nothing to catch it.

No `docs/spec/*.md` update is needed: `grammar.md:410-414` already documents the contract
generically for all widths ("the count must be less than the LHS width"), with no
width-by-width enumeration to correct.

A `gh issue comment 981` will be posted recording this correction before implementation
starts, so a reviewer isn't confused when the diff touches only `compiler/`.

## 1. Problem restated

In the self-hosted C emitter (`compiler/c_emitter.vow`), dynamic (non-const) shift counts on
`i64`/`u64` operands are masked (`& 63`) but never bounds-checked, so ESBMC cannot catch an
out-of-range shift count as a contract violation for those two widths — it just silently wraps.
For `i128`/`u128`, the dynamic-shift path falls through to a generic comparison-emission helper
that emits a raw native `<<`/`>>` with no masking and no assert at all, which is real C undefined
behavior for out-of-range or negative counts that ESBMC's own UB checker may or may not
independently flag. Both gaps mean the documented spec contract ("dynamic shift count must be
less than the LHS width, checked by ESBMC") is not enforced for 4 of the 10 integer widths in
the self-hosted compiler, even though the Rust compiler already enforces it for all 10.

## 2. Files to touch

**Production code — self-hosted only** (Rust needs no production change; see §0):

- `compiler/c_emitter.vow`
  - `emit_inst`'s `IOP_SHL()/IOP_SHR()` handling (~line 1476 onward): add the
    `__ESBMC_assert(v<b> < 64, "integer shift count");` line to the existing `ITY_I64()` and
    `ITY_U64()` branches (~1498, ~1505), mirroring the i32 branch's existing assert line
    exactly (same label string, so `parse_assert_label`/counterexample mapping needs no change —
    confirmed the match is a literal string comparison, width-independent:
    `compiler/verifier.vow:725`, `vow-verify/src/esbmc.rs:232`).
  - Extend the existing narrow-shift parametric machinery to also cover i128/u128, since
    `cemit_narrow_shift_helper` already computes `c_ty` via `ir_ty_to_c` (which already maps
    `ITY_I128()`/`ITY_U128()` to `__int128`/`unsigned __int128`, confirmed at
    `compiler/c_emitter.vow:659-660`) and already derives bits/mask/signed/opname/prefix
    generically — the only gaps are the hardcoded `else { 32 }` / `else { 3 }`-style fallbacks:
    - `narrow_shift_ty_at(index)`: extend from 5 entries (I8,U8,I16,U16,U32) to 7
      (add I128, U128).
    - `cemit_narrow_shift_helper`: extend the `bits`/`u_ty`/`signed` computation to recognize
      128 (bits=128 when ty is I128/U128; `u_ty` = `"unsigned __int128"`; `signed` includes
      `ITY_I128()`).
    - `append_narrow_shift_helpers_for_fns` / `_for_function` / `_for_module_fns`: loop bound
      `while ti < 5` → `while ti < 7` (3 call sites).
    - `emit_inst`'s `narrow_shift` boolean (~line 1477) and its local `width`/`prefix`
      computation (~1481, ~1484): add `ITY_I128()`/`ITY_U128()` to the condition, add a `128`
      branch to `width`, add `ITY_I128()` to the signed-prefix check.
    - Since the helper family no longer means "narrow" once it includes a 128-bit (wider than
      i64) type, rename `narrow_shift` → `dynamic_shift`, `narrow_shift_ty_at` →
      `dynamic_shift_ty_at`, `cemit_narrow_shift_helper` → `cemit_dynamic_shift_helper`,
      `append_narrow_shift_helpers_for_*` → `append_dynamic_shift_helpers_for_*`. Purely
      mechanical, same commit, not a separate refactor — the old name becomes actively
      misleading otherwise.

**Tests:**

- `compiler/tests/test_c_emitter.vow` — extend/add unit tests next to
  `check_narrow_shifts_use_helpers` (line 216) using the existing `shift_function(id, name, ty,
  op)` builder (line 189):
  - i64/u64: assert both `" < 64, \"integer shift count\");"` lines appear for a function using
    each op/type, matching the existing i32 assert-label test's style at line 240.
  - i128/u128: assert the new helper signatures (`static inline __int128 __vow_shl_i128`,
    `static inline unsigned __int128 __vow_shr_u128`, etc.), the call sites
    (`__vow_shl_i128(v0, v1)`), and the `" < 128, \"integer shift count\");"` assert lines.
- `tests/verify-fail/` — 4 new small fixtures mirroring the existing
  `tests/verify-fail/shift_count_unattributed.vow` (i32 template, no `requires` on the count, an
  unprovable `ensures: result == result`-shaped vow so the shift-count assert is the only thing
  that can fail): `shift_count_unattributed_i64.vow`, `_u64.vow`, `_i128.vow`, `_u128.vow`. Same
  `TEST:` directives as the template (`counterexample-vow-id 4294967293` is a fixed sentinel —
  confirmed in `compiler/verifier_ids.vow:27-29`, not per-width — so it carries over unchanged).
- `tests/verify/` — extend `tests/verify/narrowing_intrinsic_bounds.vow` (or add a sibling
  fixture) with `shift_i64`/`shift_u64`/`shift_i128`/`shift_u128` functions following the
  existing `shift_i8`/`shift_i16`/`shift_u16`/`shift_u32` pattern (`requires: count < 64` /
  `< 128`, `ensures: result == value >> count`), and call them from `main`. This is the
  regression lock: it proves the new assert doesn't reject legitimately-bounded dynamic shifts,
  and — more importantly for i128/u128 — that the self-hosted compiler now *compiles and links*
  a program using a dynamic i128/u128 shift at all (today it does, via raw `<<`/`>>`, but
  silently without the contract).

No `vow-codegen`, `vow-clif-shim`, or `vow-runtime` changes: this issue is scoped to the ESBMC
verification C model only, not the Cranelift runtime codegen path (which already masks shift
counts at runtime for correctness, independent of the verifier's contract-checking assert).

## 3. TDD slices

1. **Red:** add `tests/verify-fail/shift_count_unattributed_i64.vow` and `_u64.vow`. Run both
   through `build/vowc verify` (self-hosted) — expect `Verified` today (wrong: the shift-count
   assert never fires because it's never emitted), where `$RUST verify` on the same fixture
   already returns `VerifyFailed`. This mismatch is the first concrete, reproducible red signal
   and should be confirmed before touching production code.
   **Green:** add the one-line `__ESBMC_assert(... < 64, "integer shift count");` to the
   `ITY_I64()` and `ITY_U64()` branches in `compiler/c_emitter.vow`. Re-run; both compilers now
   report `VerifyFailed` with the same sentinel vow-id.
2. **Red:** add `compiler/tests/test_c_emitter.vow` checks asserting the i64/u64 assert lines
   are present in `emit_c_module` output for a minimal shift function (same shape as
   `check_narrow_shifts_use_helpers`). This should already be green after slice 1's production
   fix if slice 1 lands first — reorder if doing pure unit-level TDD first is preferred; either
   ordering is fine since the two are testing the same code path at different levels.
3. **Red:** add `tests/verify-fail/shift_count_unattributed_i128.vow` and `_u128.vow`. Run
   through `build/vowc verify` — today this likely reports some non-`VerifyFailed` status (or a
   `VerifyFailed` for an unrelated/coincidental reason, since the raw `<<`/`>>` is UB that ESBMC
   may or may not flag on its own) while `$RUST verify` reports `VerifyFailed` attributed to the
   shift-count sentinel vow-id. Confirm the actual current self-hosted output before writing the
   assertion, since "unsound UB" can manifest as a crash, a different failure, or a silent pass
   depending on ESBMC's own analysis — don't assume which.
   **Green:** extend `dynamic_shift_ty_at`/`cemit_dynamic_shift_helper`/the three
   `append_dynamic_shift_helpers_for_*` loop bounds/the `emit_inst` dispatch condition to route
   i128/u128 through the same generator (per §2). Re-run; both compilers converge on the same
   `VerifyFailed` + sentinel vow-id.
4. **Red:** add `compiler/tests/test_c_emitter.vow` checks for the i128/u128 helper signatures,
   call sites, and assert lines (mirroring `check_narrow_shifts_use_helpers`).
   **Green:** should already pass from slice 3's production fix; this is the unit-level lock-in.
5. **Regression lock (no red/green — pure addition):** extend
   `tests/verify/narrowing_intrinsic_bounds.vow` with the four new bounded shift functions and
   wire them into `main`. Run `scripts/full_test.sh` Section 4b — expect `Verified` on both
   compilers, `compare_json` reporting no divergence.

Each slice is independently committable and reviewable; slice order 1→5 closes the more
severe/soundness-relevant gap (i64/u64 silent wrap) before the more mechanically-involved one
(i128/u128 new helper family), consistent with "many small changes."

## 4. Verification surface

- **Property ESBMC must prove (both directions), per width w ∈ {64, 128} × {signed, unsigned}:**
  an unconstrained dynamic shift (`count` not provably `< w` at the call site) must fail
  verification with `VerifyFailed`, blamed to the shift-count assert (sentinel vow-id
  `4294967293`, matching Rust's `UNATTRIBUTED_VOW_ID`), and a shift whose caller supplies
  `requires: count < w` must verify (`Verified`) with `ensures: result == value << count` (or
  `>> `) holding exactly — not a weakened `result <= ...`-style postcondition.
- **Backend-independence check:** per CLAUDE.md's contract-authoring rule, the `requires: count
  < 64` / `< 128` added to the new `tests/verify/` fixtures must be the *true* semantic
  precondition for a well-defined shift (matches the LHS width), not a verifier-convenience
  bound — it already is, since it's copied from the existing i8/i16/u16/u32 fixtures that
  establish this exact pattern.
- **Cross-compiler parity is the actual oracle here**, not a new standalone property: every new
  fixture lands in `tests/verify/` or `tests/verify-fail/`, both of which `scripts/full_test.sh`
  Sections 4b/4c run through `$RUST verify` and `run_self verify` and diff via `compare_json`
  (`scripts/full_test.sh:786-830`). A fixture that passes on Rust but silently diverges on
  self-hosted is exactly the bug class this issue is about, so the test harness itself is the
  right verification surface — no new harness mechanism needed.
- Build/verify any new or modified `.vow` fixture with `VOW_CACHE_DIR=$(mktemp -d)` per the
  standing memory note (stale cached objects after a compiler rebuild at the same source rev).

## 5. Risk areas

- **Binary fixed point:** `compiler/c_emitter.vow` is part of the self-hosted compiler's own
  source, compiled by itself during bootstrap. Renaming `narrow_shift_ty_at` → `dynamic_shift_ty_at`
  etc. and extending the loop bound from 5 to 7 changes emitted C text (new helper functions
  appear when i128/u128 shifts are present) but does not change control flow shape or ordering
  for existing widths — the i8/u8/i16/u16/u32/i32/i64/u64 code paths are untouched except for
  the two new assert lines. Low risk to the stage0→stage1→stage2 fixed point, but **must still
  run the full bootstrap triple test** (`scripts/concat_vow.sh` + 3-stage build + `sha256sum`
  compare) since this is a self-hosted compiler source change, per CLAUDE.md's "modify BOTH
  compilers" rule — here that means "re-verify the self-hosted compiler can still compile
  itself to a fixed point," not "add new Rust production code" (there is none to add).
- **`BTreeMap`/`HashMap`/stack-slot concerns (vow-clif-shim):** not applicable — this change is
  entirely within the ESBMC C-model emitter, not the Cranelift codegen path. No stack-slot or
  codegen-ordering risk.
- **`parse → print → parse` idempotency:** not applicable — no AST/grammar change, no printer
  change. The fix is confined to C-text emission in the verifier backend.
- **`cargo clippy --all -- -D warnings`:** no Rust production changes planned, so no new clippy
  surface. If the plan's §0 scope correction is wrong in some way not caught by this research
  (e.g., a width-specific Rust bug the generic scan somehow doesn't reach — verify this
  empirically as the very first implementation step, e.g. by running `$RUST verify` on the new
  `tests/verify-fail/shift_count_unattributed_i128.vow` fixture first, before writing any
  production code), the implementer must re-scope back to touching
  `vow-verify/src/c_emitter.rs` too, and clippy then applies.
- **Mutation testing:** the new assert lines are prime `contract-weaken`/`const-flip` mutation
  targets (e.g. a mutant flipping `< 64` to `<= 64` or `< 63`). Not required for this PR (mutation
  testing is local-only, per CLAUDE.md), but worth a follow-up `vowc mutants run` pass over
  `compiler/c_emitter.vow` once this lands.
- **Sentinel vow-id collision:** confirmed `4294967293` (`UNATTRIBUTED_VOW_ID` /
  `verifier_unattributed_vow_id`) is a single fixed sentinel shared by all unattributed assert
  failures, not derived per-callsite — so the new fixtures don't need unique IDs and won't
  collide with each other or with the existing i32/narrow fixtures.

## 6. Out of scope

- **Rust production code changes** — already complete for all 10 widths (§0); do not touch
  `vow-verify/src/c_emitter.rs` production logic. (A single small Rust-side confirmation test
  may be added if slice 1's initial red-signal check in the implementation stage reveals the
  Rust side needs a regression test lock too, but this is not expected to require new
  production code.)
- **Negative dynamic shift counts for signed count types** — `v < w` lets a negative count
  (e.g. `-1 < 64`) through, which then gets masked (`& 63`) into a large positive shift. This
  affects *all* widths equally (not something this issue's width-matrix fix introduces or
  worsens) and the checker already rejects *constant* negative shift counts
  (`tests/error/i16_negative_shift_count.vow`). Dynamic negative counts are a separate,
  pre-existing gap — file a follow-up issue, do not fold into this PR.
- **`vow-codegen`/`vow-clif-shim`/`vow-runtime` runtime shift-count checks** — this issue is
  scoped to the ESBMC verification C model (per its title). Runtime (`--mode debug`)
  enforcement of the same contract, if desired, is a separate unit of work.
- **Renaming/restructuring unrelated parts of `compiler/c_emitter.vow`** — the file is 3718
  lines; this PR touches only the shift-emission and shift-helper-generation functions. No
  broader module splitting, even though the file is large enough to arguably warrant it per
  CLAUDE.md's "small files" guidance — that's a separate, unrelated refactor.
- **`docs/spec/grammar.md` changes** — already accurate and width-generic; nothing to update.
