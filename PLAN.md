# Plan: issue #982 — self-hosted `IOP_SHL`/`IOP_SHR` dispatch has no `ITY_U32()` case

## 0. Finding that reframes this issue

**The dispatch gap described in #982 no longer exists in `compiler/c_emitter.vow`.** It was closed as
a side effect of `f5d2a16e8` ("feat(numeric): make remaining narrow integers first-class", merged as
part of PR #995 on 2026-08-17), which landed *after* #982 was filed (2026-07-30) but never cross-linked
or closed it.

Evidence, read directly off this branch's HEAD:

- `compiler/c_emitter.vow:1476-1497` — the `IOP_SHL()`/`IOP_SHR()` dispatch's `narrow_shift` predicate
  (line 1477-1479) already includes `inst.dv == ITY_U32()` alongside `I8`/`U8`/`I16`/`U16`. For `U32` it
  computes `width = 32`, emits `__ESBMC_assert(v<count> < 32, "integer shift count")`, and calls
  `__vow_shl_u32`/`__vow_shr_u32` — exactly the shape the issue says is missing.
- `compiler/c_emitter.vow:3326-3332` (`narrow_shift_ty_at`) — the generic narrow-shift-helper
  enumeration iterates `I8, U8, I16, U16, U32` (index 4 falls through to `ITY_U32()`), so
  `cemit_narrow_shift_helper` (3334-3403) generates the `__vow_shl_u32`/`__vow_shr_u32` static-inline
  helpers whenever a module uses them. Body: `return value << (count & 31);` /
  `return value >> (count & 31);` — byte-for-byte the same shape as Rust's `emit_shift_helper`
  unsigned case (`vow-verify/src/c_emitter.rs:2613-2622`).
- All three helper-declaration call sites include the narrow-shift pass, so there is no path that can
  emit a `__vow_shl_u32`/`__vow_shr_u32` *call* without also emitting its *definition*:
  - `append_narrow_shift_helpers_for_fns` → whole-module build (`c_emitter.vow:3602`)
  - `append_narrow_shift_helpers_for_module_fns` → callee-inlined verify (`c_emitter.vow:3669`)
  - `append_narrow_shift_helpers_for_function` → single-function verify, the `callee_fis.len() == 0`
    branch of `emit_c_for_verify` (`compiler/verifier.vow:227-242`)
- Lowering actually produces a `U32`-tagged shift inst for the case the issue names (`count << 1` for
  `count: u32`): `compiler/lower.vow:2643-2670`, the `operand_ty = lhs_ty` fallback, sets
  `operand_ty = ITY_U32()` when `lhs_ty == ITY_U32()`; `binop_opcode` (line 1428-1429) maps `BINOP_SHL`
  to `IOP_SHL()` independent of `operand_ty`; the resulting inst carries `dk = IDATA_INTEGER()`,
  `dv = operand_ty = ITY_U32()` — exactly what the dispatch above keys on. The wiring is not dead code.
- The Rust emitter's dispatch (`vow-verify/src/c_emitter.rs:1121-1149`, `Opcode::Shl | Opcode::Shr`) is
  fully generic over `(signedness, width)` from `InstData::Integer`, so it has never had a u32-shaped
  gap — nothing to change there either.

**So this plan does not touch dispatch/codegen in either compiler.** What *is* still missing is
regression coverage that would catch a reintroduction of this exact gap:

- `tests/run/u32_basic.vow` exercises u32 `<<`/`>>` for runtime behavior only, via
  `scripts/full_test.sh`'s `run_promoted_run_tests`, which builds with `--no-verify`
  (`scripts/full_test.sh:253-254`). It never reaches ESBMC, so it cannot exercise the
  `__ESBMC_assert(... < 32, ...)` this issue is about.
- `tests/error/u32_shift_out_of_range.vow` / `u32_negative_shift_count.vow` test a *different*
  mechanism — the compile-time const-eval `ShiftCountOutOfRange` diagnostic for a **literal** shift
  count — not the ESBMC-provable dynamic bound.
- `tests/verify/narrowing_intrinsic_bounds.vow:57-62` (`shift_u32`) already proves the **positive**
  case for u32 **right**-shift only (`requires: count < 32`, verifies on both compilers via
  `scripts/full_test.sh` Section 4b). No u32 left-shift positive case, and no u32 case at all for the
  **violating** path.
- `tests/verify-fail/shift_count_unattributed.vow` proves the **violating** path (no bound on the
  count → ESBMC finds a counterexample, both compilers must agree byte-for-byte via `compare_json` in
  Section 4c) — but only for `x: i32, n: u32`. There is no u32-*operand* analog, for either `<<` or
  `>>`. This is the one scenario in the issue ("a u32 shift verifies/builds differently on the two
  compilers") that has zero coverage today.

Closing #982 for real means adding that missing cross-compiler regression coverage, so a future
regression in either emitter's `ITY_U32()` handling is caught by `scripts/full_test.sh` Section 4c
instead of silently diverging again.

## 1. Files to touch

- `tests/verify-fail/u32_shl_count_unattributed.vow` — **new**. Mirrors
  `tests/verify-fail/shift_count_unattributed.vow` but shifts a `u32` operand left by an unbounded
  `u32` count.
- `tests/verify-fail/u32_shr_count_unattributed.vow` — **new**. Same shape, right shift.

No other files change. Specifically:
- No `compiler/*.vow` or `vow-verify/src/*.rs` production code changes — the dispatch is already
  correct and symmetric in both compilers (see §0).
- No `docs/spec/*.md` changes — no syntax, semantics, builtin, operator, effect, or CLI change.
- No `vow-ir`, `vow-codegen`, `vow-clif-shim` changes.

## 2. TDD slices

Both slices are "coverage-locking" rather than classic red-green-refactor, since the production
behavior under test is already correct on this branch. "Red" here means *the scenario is currently
unverified by any test*, not *the code is broken*. Each slice is a single new fixture file, independently
reviewable and revertible.

1. **`tests/verify-fail/u32_shl_count_unattributed.vow`** — u32 left shift, unattributed violation.
   - Draft content:
     ```vow
     // TEST: category bounds
     // TEST: counterexample-fn "shift"
     // TEST: counterexample-blame none
     // TEST: counterexample-vow-id 4294967293
     // TEST: counterexample-violation "shift amount exceeds the operand's bit width"
     module U32ShlCountUnattributed

     fn shift(x: u32, n: u32) -> u32 vow {
       ensures: result == result
     } {
       x << n
     }
     ```
   - The `4294967293` sentinel is `verifier_unattributed_vow_id()` (`compiler/verifier_ids.vow:24-29`,
     mirrored from `vow_verify::c_emitter::UNATTRIBUTED_VOW_ID = u32::MAX - 2`) — it is
     type-independent, so the same constant from the existing i32 fixture applies here. The violation
     string is the generic "shift amount exceeds the operand's bit width" produced by
     `compiler/verifier.vow:726` and mirrored in `vow-verify/src/esbmc.rs:232,1996` — also
     type-independent.
   - **Before committing this as green**, run both binaries directly on the draft and diff against the
     directive values instead of trusting the copy from the i32 fixture:
     ```bash
     ./target/release/vow verify tests/verify-fail/u32_shl_count_unattributed.vow
     build/vowc verify tests/verify-fail/u32_shl_count_unattributed.vow
     ```
     If either disagrees with the drafted `// TEST:` directives, update the directives to match actual
     output — do not force the fixture to match a guess. (`cargo build --release -p vow` and
     `scripts/bootstrap.sh --skip-cargo` first if `target/release/vow`/`build/vowc` are stale or
     missing; see §4 of the wall-clock budget memory — bootstrap is the expensive step here, budget
     ~5 min, don't re-run it per slice.)
   - Confirms: `scripts/full_test.sh` Section 4c (`compare_json` on `tests/verify-fail/*.vow`) now
     exercises `ITY_U32()` through the real dispatch in both emitters, with the two JSON outputs
     required to match exactly.

2. **`tests/verify-fail/u32_shr_count_unattributed.vow`** — same shape, right shift (`x >> n`),
   module `U32ShrCountUnattributed`. Same sentinel vow-id and violation string expected (right-shift
   hits the same `narrow_shift` branch and the same `__ESBMC_assert` call site in both emitters, just
   with `opname = "shr"` instead of `"shl"`). Verify empirically the same way as slice 1.

Do them in either order; they are independent and do not block each other. No `addBlockedBy` relation
needed beyond "both block opening the PR."

## 3. Verification surface

- **Property under test**: for an unconstrained `n: u32`, ESBMC must find a counterexample where
  `n >= 32`, falsifying the compiler-inserted `__ESBMC_assert(n < 32, "integer shift count")` that
  guards `__vow_shl_u32`/`__vow_shr_u32`. This is the same property class already proven for `i32` by
  `shift_count_unattributed.vow` — no new property kind, just a new operand type exercising it.
- **No contract authoring involved.** Both new fixtures use `ensures: result == result` (trivially
  true), matching the existing i32 fixture's convention — the interesting assertion is the
  compiler-synthesized shift-bound check, not a user-authored vow clause. There is nothing here to
  weaken or distort to satisfy ESBMC; if ESBMC cannot find the counterexample, that is a real
  regression, not a bound to loosen.
- **Fixture growth**: exactly the two files in §1, both under `tests/verify-fail/`, picked up
  automatically by `scripts/full_test.sh` Section 4c's `tests/verify-fail/*.vow` glob and by
  `tests/run_tests.sh` Phase 3 — no harness script or manifest needs editing.
- **No `examples/` growth** — this is an internal compiler-parity concern, not a user-facing example.

## 4. Risk areas

- **Binary fixed point** (`scripts/concat_vow.sh` stage0→1→2 triple): unaffected. No `compiler/*.vow`
  production code changes, so the self-hosted compiler's own compiled output cannot change. Still must
  run through a real `build/vowc` (via `scripts/bootstrap.sh`) to exercise the new fixtures end to end,
  but that is a read of the existing fixed point, not a write to it.
- **`parse → print → parse` idempotency**: the two new fixture files should be written in canonical
  form from the start (mirror `shift_count_unattributed.vow`'s existing formatting exactly — same
  brace style, same `vow { ... } { ... }` layout) so they do not become the first failing case if a
  future canonical-form sweep ever includes `tests/verify-fail/`. No such sweep exists over this
  directory today, so this is a "don't be sloppy" note, not a blocking gate.
- **`cargo clippy --all -- -D warnings`**: unaffected — no Rust source changes.
- **Directive values are a best-effort prediction, not a verified fact.** The `4294967293` /
  `"shift amount exceeds the operand's bit width"` / `blame none` values in §2 are inferred from the
  shared sentinel constant and the existing i32 analog, not from having actually run ESBMC in this
  planning stage (no `build/vowc` or `target/release/vow` exists in this workspace yet — see §0's
  evidence trail, which is all static). The implementation stage must confirm them empirically per the
  "before committing this as green" step in each slice. A wrong guess only fails the local
  `tests/run_tests.sh` directive check (`compare_json` in CI's `full_test.sh` diffs Rust against
  self-hosted directly, independent of the `// TEST:` directives) — but should still be corrected
  before merge.
- **ESBMC must actually be installed** in the dev/CI environment for `verify` (not `--no-verify`) to
  produce real output instead of being skipped — already a precondition for every existing
  `tests/verify-fail/` fixture, not new risk introduced by this change.

## 5. Out of scope

- **No positive (`requires: count < 32`) u32 left-shift fixture.** `narrowing_intrinsic_bounds.vow`
  already proves the positive case for u32 right-shift, and the helper-emission machinery is shared
  (type-agnostic) between `shl`/`shr` — a left-shift positive case would exercise the same code path
  with near-zero marginal signal. Can be added later as a trivial follow-up if desired; not needed to
  close #982.
- **Editing `tests/verify/narrowing_intrinsic_bounds.vow`** to add that left-shift case is deliberately
  avoided — it's an existing, passing, unrelated fixture owned by the narrow-integer-first-class work
  (#995); bolting an unrelated case onto it would blur `git bisect` attribution for both issues.
- **i64/u64/i128/u128 dynamic shift-count parity** is explicitly issue #981's scope per #982's own
  issue body ("Related to (but distinct from) #981"). Not touched here even though `narrow_shift_ty_at`
  (and hence the shared helper machinery) stops at `U32` and does not cover `I128`/`U128` — that gap is
  real but belongs to #981, not this PR.
- **No refactor of the `IOP_SHL`/`IOP_SHR` dispatch chain** (e.g., collapsing the remaining
  `I64`/`U64`/`I32` special cases into the generic `narrow_shift_ty_at` enumeration) even though the
  chain now has two different mechanisms doing structurally similar things. That is a behavior-preserving
  cleanup with no test-coverage motivation of its own — out of scope for a coverage-only PR, and would
  bundle a refactor into what should stay a minimal, surgical change.
- **No `gh issue comment` / label changes are planned by this stage** — the implementation stage should
  link the PR with "Closes #982" and, in the PR body, note explicitly that the underlying dispatch gap
  was already closed by #995 (`f5d2a16e8`) and that this PR's contribution is the missing cross-compiler
  regression coverage. That context belongs in the PR description, not a separate issue comment, since
  the PR closing the issue is itself the traceable record.
