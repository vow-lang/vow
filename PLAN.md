# Plan: #1148 — unchecked division/remainder by zero aborts silently

## 1. Problem restated

Unchecked `/` and `%` (and signed `MIN / -1`) currently reach either a bare
native hardware fault (narrow widths `i8`..`u64`: Cranelift's `sdiv`/`udiv`/
`srem`/`urem` trap unguarded, no manual check exists at all) or a
compiler-emitted bare Cranelift `trap` (wide widths `i128`/`u128`: guarded by
`emit_divisor_traps`, which traps directly instead of calling the runtime
reporter). Both produce zero bytes on stderr and an exit status that is
`132`/`136` depending on signal, never the spec's reserved `134`. The sibling
bug for *checked* operators (`/!`, `%!`) was fixed in #1137 by routing through
`emit_overflow_check`, which calls `__vow_arithmetic_overflow()` (exit `134`,
JSON envelope) before trapping. This issue routes the unchecked path through
the same mechanism.

## 2. Design decision (lock this in before coding)

**Reuse the existing `ArithmeticOverflow` error kind for unchecked `/`/`%`
aborts too. Do not introduce `DivisionByZero`.**

Evidence, gathered during planning:
- `vow-runtime/src/lib.rs:2997-3018` (`define_wide_div_rem!`) — the wide
  `__vow_i128_div`/`__vow_i128_rem`/`__vow_u128_div`/`__vow_u128_rem` helpers
  *already* call `__vow_arithmetic_overflow()` unconditionally on `b == 0`.
  This runtime plumbing is dead today only because the codegen intercepts the
  zero-divisor case first with its own bare trap (`emit_divisor_traps`,
  `vow-codegen/src/cranelift_backend.rs:333-354`; mirrored in
  `vow-clif-shim/src/lib.rs`). It already treats unchecked-wide-div-by-zero as
  `ArithmeticOverflow`.
- `docs/spec/errors.md:603` — the `ArithOverflowReachable` verifier warning
  already names `divisor is zero` and `quotient is not representable (MIN /
  -1)` as `ArithmeticOverflow` causes, with no restriction to checked
  operators.
- `docs/spec/grammar.md:297-298` — "Division or remainder by zero aborts, as
  does `/` and `/!` on `MIN / -1`" already documents `/` (unchecked) and `/!`
  (checked) as having *identical* abort conditions. Unlike `+`/`-`/`*`, where
  wrapping has a well-defined silently-wrapped result, division has no
  representable result for a zero divisor or `MIN / -1` — so there is no
  "wrapped" behavior for `/`/`%` to fall back to. The wrapping/checked
  distinction that matters for the other three operators doesn't apply here.
- Consequence confirmed in `vow-verify/src/c_emitter.rs:2894-2904`
  (`emit_checked_arith`): the ESBMC C model's `MIN / -1` guard is already
  keyed on `int_ty.signedness == Signed` with **no width restriction** (128-bit
  is excluded earlier by the `is_modelable` gate, not by this check) — i.e.
  the *verifier* already treats narrow signed `/!` `MIN / -1` as a proof
  obligation. Only the Cranelift **codegen** side never added the matching
  narrow check (`cranelift_backend.rs:1140`: `if inst.ty == IrTy::I128 &&
  inst.opcode == Opcode::CheckedDiv`, i.e. I128-only). So `i32::MIN /! -1` and
  `i64::MIN /! -1` silently trap via hardware fault *today, on `main`,
  independent of this issue* — a real residual gap in #1137, not a new one
  this plan invents. Unifying the fix closes it as a side effect; see §3 and
  §6.

Rejected alternative: a distinct `DivisionByZero` error kind. Would require a
new JSON shape, a new/parameterized runtime emitter, and edits to all four
mirrored doc locations (§2 below) for no behavioral benefit — `errors.md`'s
own `ArithOverflowReachable` prose already treats this as one error family.
Exit status is `134` either way per the issue text, so this choice is pure
diagnostic-shape, and reuse is the smaller, more consistent surface.

**Corollary: after this fix, `WrappingDiv`/`WrappingRem` and
`CheckedDiv`/`CheckedRem` compute the identical Cranelift lowering** (same
zero-divisor / `MIN÷-1` trap condition, same `emit_overflow_check` call, same
underlying native op or wide-helper call). The IR keeps them as distinct
opcodes (verification treats them very differently — `WrappingDiv` emits plain
C `/` for ESBMC's own UB checker in `c_emitter.rs:1034-1041`; `CheckedDiv`
explicitly models the abort via `emit_checked_arith` — that distinction is
real and stays), but the two Cranelift-backend match arms converge to one
body. Extract a shared helper (e.g. `emit_div_or_rem`) called from both arms
rather than duplicating ~30 lines twice — do not literally merge the match
patterns into one arm, since keeping `WrappingDiv | WrappingRem` and
`CheckedDiv | CheckedRem` as separate arms (both calling the shared helper)
preserves the table's readability and leaves a seam if the two ever need to
diverge again (e.g. if `DivisionByZero` is introduced later).

## 3. Files to touch

**Rust compiler backend:**
- `vow-codegen/src/cranelift_backend.rs`
  - `Opcode::WrappingDiv | Opcode::WrappingRem` arm (currently 1077-1091):
    route through the same trap-condition + `emit_overflow_check` logic
    `CheckedDiv | CheckedRem` uses (currently 1128-1161), via a shared helper.
  - Extend the `MIN / -1` check to every signed width, not just `IrTy::I128`
    (currently gated at line 1140 `inst.ty == IrTy::I128`). Needs a small
    per-width `MIN` constant lookup for `I8`/`I16`/`I32`/`I64` (analogous to
    `c_int_min_literal` in `vow-verify/src/c_emitter.rs`, but there is no
    existing Cranelift-side equivalent — write one; e.g.
    `narrow_signed_min_iconst(builder, cl_ty) -> Value`).
  - Delete `emit_divisor_traps` (317-354) and `emit_conditional_trap`
    (356-368) once nothing calls them — dead code fails
    `cargo clippy --all -- -D warnings`.
  - `wide_helper_symbol` (283-298) needs no change: it already maps
    `WrappingDiv`/`CheckedDiv` (and `WrappingRem`/`CheckedRem`) to the same
    symbol names.

**Self-hosted compiler's Cranelift shim (Rust crate, FFI target of
`compiler/clif.vow`):**
- `vow-clif-shim/src/lib.rs`
  - `IOP_WDIV | IOP_WREM` arm (currently ~2112-2132): same fix, mirrored.
  - `IOP_CDIV | IOP_CREM` arm (currently ~2161-2192): extend narrow `MIN/-1`
    the same way.
  - Delete `emit_divisor_traps` (~3049 onward) once unused. Check whether
    `wide_divisor_trap_condition` (~3019-3037, currently only called from the
    checked arm for `I128`) becomes the single shared trap-condition builder
    for *all* widths once unchecked routes through it too — if so, generalize
    it rather than keeping two condition-builders; if the narrow/`I128` shapes
    diverge enough (different `iconst` calls), keep it 128-bit-scoped and add
    a sibling for narrow widths, called from the same shared helper as above.

**`compiler/*.vow` — no change required.** `compiler/clif.vow` never emits
Cranelift instructions itself; every instruction is dispatched by ID through
`__vow_clif_fn_inst` (`compiler/clif.vow:480-484`) to the shim above. Fixing
`vow-clif-shim` fixes `build/vowc`'s compiled output without touching any
`.vow` source. (`compiler/clif.vow:404-420` only lists wide-helper *symbol
names* for import declarations — those names are unchanged.) Note this
explicitly in the PR description so a reviewer doesn't go looking for a
`.vow` diff that satisfies "both compilers" — both *binaries* (`vow` and
`build/vowc`) are fixed because both link against corrected Cranelift-lowering
code; `build/vowc` just gets there via `vow-clif-shim` rather than
`vow-codegen`.

**Runtime — comment-only, no functional change:**
- `vow-runtime/src/lib.rs:2976-2996` (the doc comment above
  `define_wide_div_rem!`) currently says the codegen's "divisor traps" make
  these guards unreachable. That remains *true* after the fix (the codegen
  still intercepts before the call — via `emit_overflow_check` now, not a bare
  trap), so the comment's core claim doesn't need to change, but its wording
  ("the same divisor traps Cranelift itself inserts") should be updated to
  reflect that the interception is now the shared `emit_overflow_check`
  reporter path, not a bare `TrapCode`. Reword, don't remove — the guards stay
  genuinely unreachable and still need to exist as a fail-closed backstop
  against an FFI panic.

**Docs (edit only the source; regenerate the rest):**
- `docs/spec/errors.md:640` — broaden the `ArithmeticOverflow` "When:" line
  from "A checked arithmetic operator (...) overflows at runtime" to also
  cover unchecked `/`/`%` dividing by zero or evaluating `MIN / -1`.
- `docs/spec/grammar.md` — add one clarifying sentence near line 267-268 (end
  of the "Wrapping Arithmetic" section, before the 128-bit paragraph steals
  the only mention): `/` and `%` have no representable wrapped result for a
  zero divisor or `MIN / -1`, so — unlike `+`/`-`/`*` — they abort with
  `ArithmeticOverflow` even unchecked, at every width. Leave lines 294-300
  as-is (still accurate) but note in-line that this is no longer 128-bit-only
  phrasing once the sentence above exists.
- `docs/spec/cli.md:259-265` — add "division by zero" alongside "arithmetic
  overflow" in the list of runtime-abort categories reserved to exit `134`.
- After editing the three files above, run
  `uv run python scripts/generate_help.py` to regenerate `vow/src/skill.rs`,
  `compiler/main.vow`'s embedded skill bundle, and `skills/vow/reference/*.md`
  — confirmed generated (not hand-maintained) by reading
  `scripts/generate_help.py:29-34,1061` (`ERRORS = SPEC_DIR / "errors.md"`;
  `"reference/errors.md": ERRORS.read_text()...`) and `inject_rust`/
  `inject_vow` (856-930ish). PR #1137 (`git show 35a6a273 --stat`) touched
  `compiler/main.vow` and `vow/src/skill.rs` by +4/+8 lines each — consistent
  with generator output, not hand edits. Do **not** hand-edit those two files
  or `skills/vow/reference/errors.md`; let the generator write them, then
  `git diff` to confirm only the expected mechanical changes landed. Follow
  with `cargo build --release -p vow` and `scripts/bootstrap.sh --skip-cargo`
  per `CLAUDE.md`'s documented sequence (also needed anyway for the fixed
  point — see §5).
- `scripts/check_help_coverage.py` (already run by `full_test.sh`) catches
  drift if a doc edit is missed.

**Tests — two lanes, one per backend (confirmed by reading the harnesses):**
- `tests/run_tests.sh` invokes `build/vowc` (line 9: `VOWC="$ROOT_DIR/build/vowc"`)
  — exercises **`vow-clif-shim`** (self-hosted compiler backend).
- `vow/tests/checked_overflow_diagnostic.rs` and
  `vow/tests/wide_literal_aggregates.rs` invoke `CARGO_BIN_EXE_vow` (line 20 /
  line 6 respectively) — exercises **`vow-codegen`** (Rust compiler backend).
  Both lanes need red→green coverage; neither alone proves the other backend
  is fixed.

  Fixture updates (self-hosted lane, `tests/run/*.vow`):
  - `i128_div_by_zero.vow`, `i128_rem_by_zero.vow`, `u128_div_by_zero.vow`,
    `i128_div_min_by_neg_one.vow`: flip `// TEST: exit 132` to
    `// TEST: exit 134` + `// TEST: stderr "{\"error\":\"ArithmeticOverflow\"}"`
    (exact pragma syntax confirmed against
    `tests/run/i128_checked_div_by_zero.vow`, which already uses this shape
    post-#1137). Update each file's explanatory comment (currently describes
    the *old*, undiagnosed behavior — e.g. "the divisor-zero trap is emitted
    by the backend" needs to say "reported via `__vow_arithmetic_overflow`").
  - `i128_rem_min_by_neg_one.vow` stays as-is (`// TEST: stdout "0 0"`, no
    abort) — it is the guard against over-eager `MIN % -1` trapping and must
    keep passing unmodified.
  - New fixtures (net-new coverage per the issue's evidence table, at
    representative narrow widths — not exhaustive across all 8 narrow
    widths, see §6):
    - `i32_div_by_zero.vow`, `i64_div_by_zero.vow` (unchecked `/`)
    - `u32_div_by_zero.vow` or `u64_div_by_zero.vow` (unsigned unchecked `/`,
      one is enough — signedness only affects the `MIN/-1` branch, which
      unsigned never takes)
    - `i32_rem_by_zero.vow` or `i64_rem_by_zero.vow` (unchecked `%`, one
      narrow width suffices — `%` has no width-varying special case)
    - `i32_div_min_by_neg_one.vow`, `i64_div_min_by_neg_one.vow` (unchecked
      `MIN / -1`, narrow — this is the part of the acceptance criteria that
      needs codegen not previously exercised at any narrow width)
    - `i64_checked_div_min_by_neg_one.vow` (checked `/!` — closes the latent
      narrow-checked gap identified in §2; add its verification-model
      counterpart is already correct per `c_emitter.rs:2898`, so this is a
      codegen-only red test)
  - All new fixtures follow the `// TEST: exit 134` +
    `// TEST: stderr "{\"error\":\"ArithmeticOverflow\"}"` pragma shape.

  Rust-lane updates (`vow/tests/`):
  - `wide_literal_aggregates.rs::wide_division_by_zero_traps` (~582-640): flip
    `diagnosed` to `true` for the `"div"`/`"rem"` rows (currently `false`,
    asserting `status.code() == None`), so all four rows assert
    `Some(134)` + stderr `{"error":"ArithmeticOverflow"}`. Rewrite the
    function's doc comment, which currently documents the checked/unchecked
    split as intentional — it becomes: all four abort identically now, only
    the operator (not diagnosis) differs.
  - Extend `checked_overflow_diagnostic.rs` (the #1137-added, purpose-built
    exit-134-plus-stderr harness — use it as the template rather than
    reinventing assertions) with cases mirroring the new narrow `tests/run/`
    fixtures above: unchecked narrow div/rem-by-zero, unchecked narrow
    `MIN/-1`, checked narrow `MIN/-1`.

## 4. TDD slices

Ordered so each step is independently compilable and each red test is written
*before* the production change that turns it green. Both backends' production
code changes happen together per slice (issue requires "both compilers... in
the same PR"; splitting Rust-only and shim-only commits would leave one
backend broken mid-sequence).

1. **Red:** Flip `tests/run/i128_div_by_zero.vow`,
   `tests/run/i128_rem_by_zero.vow`, `tests/run/u128_div_by_zero.vow` to
   `exit 134` + stderr pragma; flip `wide_division_by_zero_traps`'s `"div"`/
   `"rem"` rows to `diagnosed: true` in `wide_literal_aggregates.rs`. Both
   fail against current `main` (still exit 132/136, no stderr).
   **Green:** In both `cranelift_backend.rs` and `vow-clif-shim/src/lib.rs`,
   change the `WrappingDiv | WrappingRem` arm to compute
   `trap_if = icmp(Equal, divisor, zero)` and call `emit_overflow_check`
   before the wide-helper call / native op, for the wide (`i128`/`u128`)
   case only initially — reuse (don't yet delete) `emit_divisor_traps` for
   the narrow path so this slice stays small. Confirms the reporter path
   works end-to-end for both backends before touching narrow codegen.

2. **Red:** Flip `tests/run/i128_div_min_by_neg_one.vow` to `exit 134` +
   stderr pragma. Confirm `tests/run/i128_rem_min_by_neg_one.vow` still passes
   unmodified (it must — no production change should touch the `%` MIN/-1
   exclusion).
   **Green:** No new production change if slice 1's `trap_if` already ORs in
   the wide MIN/-1 condition for `WrappingDiv` (it should, since
   `wide_divisor_trap_condition`/inline logic already computes this for the
   checked arm — reuse it for wrapping instead of `emit_divisor_traps`'s
   separate MIN/-1 branch at `cranelift_backend.rs:344-353`). If this slice
   needs its own change, that means slice 1's helper wasn't shared correctly
   with the checked path — fix before proceeding, don't duplicate.

3. **Red:** Write `tests/run/i32_div_by_zero.vow`, `i64_div_by_zero.vow`,
   `u32_div_by_zero.vow` (or `u64`), `i32_rem_by_zero.vow` (or `i64`) — new
   fixtures, narrow unchecked div/rem-by-zero. All currently exit 132/136
   with no stderr (unguarded native `sdiv`/`udiv`/`srem`/`urem`).
   **Green:** Extend the `WrappingDiv | WrappingRem` arm's fix to the narrow
   (non-wide-helper) path: compute the same `trap_if` (zero-divisor only,
   using `iconst(cl_ty, 0)`) and call `emit_overflow_check` before the native
   op, mirroring what `CheckedDiv | CheckedRem`'s narrow path already does at
   `cranelift_backend.rs:1131-1136`. Both files.

4. **Red:** Write `tests/run/i32_div_min_by_neg_one.vow`,
   `i64_div_min_by_neg_one.vow` — new fixtures, narrow unchecked `MIN / -1`.
   **Green:** Extend the narrow `trap_if` computation to OR in the MIN/-1
   condition when the opcode is `WrappingDiv`/`CheckedDiv` and the type is
   signed, for *every* width, not just `I128` (widen the guard at
   `cranelift_backend.rs:1140` from `inst.ty == IrTy::I128` to "any signed
   integer type"). Write the small per-width `MIN` `iconst` helper here.

5. **Red:** Write `tests/run/i64_checked_div_min_by_neg_one.vow` — new
   fixture, narrow *checked* `/!` on `MIN / -1`. This currently also fails
   (exit 136, no stderr) — the pre-existing gap identified in §2.
   **Green:** Should already pass once slice 4's widened guard lands, since
   `CheckedDiv` and `WrappingDiv` now share the trap-condition helper. If it
   doesn't, the helper wasn't actually shared — fix the sharing, don't patch
   `CheckedDiv` separately.

6. **Refactor (behavior-preserving):** Extract the shared trap-condition +
   `emit_overflow_check` + dispatch logic into one helper function per file
   (`cranelift_backend.rs`, `vow-clif-shim/src/lib.rs`), called from both the
   `WrappingDiv | WrappingRem` and `CheckedDiv | CheckedRem` arms. Delete
   `emit_divisor_traps` / `emit_conditional_trap` (and
   `wide_divisor_trap_condition` if fully subsumed) once no call site
   remains. Re-run every test from slices 1-5 to confirm the refactor changed
   nothing observable. `cargo clippy --all -- -D warnings` must pass (catches
   the now-dead old helpers if step is skipped).

7. **Rust-lane parity:** Add the narrow cases from slices 3-5 to
   `checked_overflow_diagnostic.rs`, following its existing pattern (build
   with `CARGO_BIN_EXE_vow`, run, assert `status.code() == Some(134)` +
   stderr substring). Confirms `vow-codegen`'s narrow path independently of
   the self-hosted `tests/run/` lane.

8. **Docs:** Edit `docs/spec/errors.md`, `docs/spec/grammar.md`,
   `docs/spec/cli.md` per §3. Run `uv run python scripts/generate_help.py`,
   diff the regenerated files, then `cargo build --release -p vow` and
   `scripts/bootstrap.sh --skip-cargo` (needed regardless — see §5). Run
   `scripts/check_help_coverage.py` to confirm no drift.

## 5. Verification surface

**No ESBMC / C-model changes required.** Confirmed by reading
`vow-verify/src/c_emitter.rs:1034-1041`: `WrappingDiv`/`WrappingRem` already
emit plain C `v = a / b;` / `v = a % b;`, unguarded — ESBMC's own built-in
division-by-zero and signed-overflow UB checks (enabled by default; no
`--no-div-by-zero-check` flag appears in `vow-verify/src/esbmc.rs`'s ESBMC
invocation) already catch these at verification time, independent of runtime
codegen. This issue changes only what the **compiled binary** does when a
zero-divisor/`MIN÷-1` case executes (whether verification ran or was skipped)
— it does not change what property ESBMC proves or how. `CheckedDiv`/
`CheckedRem`'s C model (`emit_checked_arith`,
`c_emitter.rs:2877-2905`) is also untouched: it already models the abort
explicitly via `__ESBMC_assert`/`__ESBMC_assume`, already correctly scoped
to every signed width for the `MIN/-1` guard (line 2898, no `I128`-only
restriction — the runtime codegen gap in §2 was purely a Cranelift-backend
omission, never a verification-model one).

No new contracts, no new `requires`/`ensures`, no ESBMC bound changes. This is
a pure backend-codegen fix; the IR opcodes (`WrappingDiv`, `WrappingRem`,
`CheckedDiv`, `CheckedRem`) are unchanged, so `vow-ir`, `vow-types`, the
parser, and the printer are all untouched — `parse → print → parse`
idempotency is not implicated.

## 6. Risk areas

- **Compile-object cache staleness.** `tests/run_tests.sh`'s pragma lane runs
  `vowc build --no-verify` (confirmed at `tests/run_tests.sh:350`), and the
  on-disk compile-object cache (`vow/src/cache.rs:31,252`, keyed by content
  hash) is only bypassed when ESBMC verification is active — it is **active**
  here. Every red→green cycle against `tests/run/` after touching
  `vow-clif-shim` must use `VOW_CACHE_DIR=$(mktemp -d)`, or a stale cached
  `.o` from before the shim rebuild will be linked, and the test will
  misreport 132/136 as still-failing (or misreport success from a stale
  correct-looking cache) even though the shim source changed.
- **Build order for `vow-clif-shim`.** It's a Rust crate; `build/vowc` links
  it as a native dependency via FFI. `scripts/bootstrap.sh --skip-cargo`
  reuses whatever `vow-clif-shim` object was last built — it will **not**
  pick up a shim-source edit unless `cargo build --release` (or equivalent)
  runs first. Sequence per slice: `cargo build --release` →
  `scripts/bootstrap.sh --skip-cargo` → run the affected lane with a fresh
  `VOW_CACHE_DIR` → only then trust a green/red result.
- **Dead code after extracting the shared helper (§4 slice 6).**
  `emit_divisor_traps`/`emit_conditional_trap` (and possibly
  `wide_divisor_trap_condition`) become unreferenced once both call sites
  route through the new shared path. `cargo clippy --all -- -D warnings` (the
  CI gate, confirmed scoped to non-test targets by prior project memory) will
  fail on unused private functions — delete them in the same slice that stops
  calling them, don't leave a cleanup slice for later.
- **Per-width `MIN` immediate correctness.** The new narrow `MIN` `iconst`
  helper must produce `i8::MIN`/`i16::MIN`/`i32::MIN`/`i64::MIN` correctly
  sign-extended/truncated to `cl_ty`'s bit width via Cranelift's `Imm64`
  encoding — verify this empirically (a passing `i8` or `i16` `MIN/-1` test,
  if one is added) rather than assuming `iconst(I8, -128i64)` truncates
  correctly by inspection alone; Cranelift's `iconst` immediate handling for
  sub-32-bit types has had surprises in other parts of this codebase
  (`vow-codegen/src/cranelift_backend.rs:174-178` already special-cases I8
  width mapping to Cranelift's `I8` type for a related reason — read that
  context before assuming narrow-width `iconst` "just works" the same as at
  I32/I64).
- **Binary fixed point.** This changes codegen output (new basic blocks / new
  `call` instructions in the compiled object for every `/`/`%` site, and for
  every self-hosted-compiler `/`/`%` site too, since `compiler/*.vow` itself
  divides/computes remainders in its own source). Re-run the full
  bootstrap-triple SHA-256 check (`scripts/concat_vow.sh` →
  stage0/1/2 → `sha256sum`) after the shim change — this is exactly the kind
  of change (new instruction sequences emitted by the shim) that could
  desync stage1/stage2 if the new code is nondeterministic (it isn't — no
  `HashMap` iteration or unordered collection is introduced — but the fixed
  point re-check is the actual proof, not code inspection).
- **`clif.vow`'s stack-slot / dominance model is unaffected.** The new
  `emit_overflow_check`/trap-condition blocks are ordinary
  `brif`/trap-block/cont-block control flow, the same shape
  `CheckedDiv`/`CheckedRem` and `CheckedAdd`/`CheckedSub`/`CheckedMul` already
  use successfully today — no new cross-block value reference is introduced
  that would need a stack slot the existing machinery doesn't already provide.
- **`BTreeMap` determinism is unaffected** — no new map insertion ordering is
  introduced by this change; `slot_map`'s ordering guarantee is untouched.

## 7. Out of scope

- **`ArithOverflowReachable` static-verification warnings for unchecked
  `/`/`%`.** The issue is about runtime diagnosis of a trap that already
  fires deterministically at those two conditions, not about extending
  static reachability analysis to unchecked operators. `WrappingDiv`/
  `WrappingRem` continue to rely on ESBMC's own built-in div-by-zero/overflow
  UB checks (§5) rather than gaining a Vow-specific
  `ArithOverflowReachable`-style warning. A follow-up issue could consider
  this, but it's a distinct verifier feature, not a bug fix.
- **A `DivisionByZero` error kind.** Rejected in §2 — reusing
  `ArithmeticOverflow` is the smaller, more consistent surface. If a future
  need for a distinct kind emerges, that's a separate spec-and-implementation
  change, not bundled here.
- **Exhaustive narrow-width test coverage (`i8`/`u8`/`i16`/`u16`).** The fix
  is width-generic (the shared helper operates on `IrTy`/`cl_ty`, not
  width-specific branches beyond signed-vs-unsigned and
  wide-vs-native-lowering), so `i8`/`i16`/`u8`/`u16` get the same fix "for
  free." §3/§4 add representative fixtures at `i32`/`i64`/`u32`/`u64` (per the
  issue's own evidence table) plus the existing `i128`/`u128`, not all ten
  widths — adding all ten would be test-volume padding, not additional
  coverage of a distinct code path.
- **Refactoring `wide_helper_symbol`, `call_wide_helper`, or any other
  neighboring helper** beyond what §3/§4 require. These are correct today and
  untouched by this fix.
- **`compiler/*.vow` changes.** Established in §3 — the self-hosted compiler
  never implements Cranelift lowering itself; there is nothing to change
  there for this issue.
- **`vow-runtime`'s wide-helper functions themselves.** Already correct
  (§2) — only the doc comment above them is stale and gets reworded, not the
  logic.
