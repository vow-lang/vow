# Plan: #1147 — u64 checked arithmetic on literals never traps

## 1. Problem restated

`0 -! 1` (and `+!`, `*!`) with literal operands typed `u64` never traps, in every
mode. The issue's stated root cause — `lower_narrow_literal` omitting `Ty::U64`
from its narrow-literal-type `matches!` — is real but **incomplete**: it is one of
six independently-duplicated `matches!`/`if`-chain gates in each compiler's
lowerer that decide whether `lower_narrow_literal` gets called at all, and all six
omit `Ty::U64` for the same reason (u64 shares i64's register width, so it was
never "narrower than i64" in the sense the other gates were written for — but it
needs different *signedness*, which is exactly what these gates exist to fix).
Verified empirically against a release build of `target/release/vow` (`--mode
debug`, `--no-verify`): the bug reproduces in **four** distinct syntactic
positions — return-position (issue's own repro), `let` binding, assignment, and
call argument — each printing/exiting as if `u64::MAX` had wrapped silently
instead of trapping. Codegen (`vow-codegen::cranelift_backend` and
`vow-clif-shim`) is already signedness-aware for `U64` at the instruction-data
level (`integer_is_signed` / `ity_is_signed` both correctly treat `U64` as
unsigned); the entire defect is confined to IR lowering never producing a
`u64`-typed checked op in the first place.

## 2. Files to touch

**`vow-ir/src/lower/mod.rs`** (Rust stage-0 lowerer) — add `Ty::U64` to each of:
1. `lower_narrow_literal`'s own guard (~line 4666-4672) — the gate the issue names.
2. `emit_narrow_integer_constant` (~line 4552-4598) — add a `Ty::U64` arm
   (`Opcode::ConstU64` / `InstData::ConstU64`), mirroring the existing `Ty::U64`
   arm already present in the sibling function `emit_integer_zero`. Without this,
   opening gate #1 makes `lower_integer_marker_as` reach the `_ => unreachable!()`
   arm the first time a bare `u64` integer literal needs re-lowering.
3. Binary-op operand-narrowing gate inside `ExprKind::BinaryOp` (~line 1589-1596).
4. Call-argument narrowing gate inside `ExprKind::Call` (~line 1686-1698).
5. Assignment (`x = rhs`) narrowing gate (~line 2017-2028).
6. Match-arm/Phi narrowing gate (~line 3430-3433).
7. Function trailing-expr-as-return gate at the end of `lower_fn` (~line
   5014-5019) — **this is the exact code path the issue's repro hits**
   (`fn f() -> u64 { 0 -! 1 }` has no explicit `return`, so it flows through the
   trailing-expr path, not `Stmt::Return`).
8. `Stmt::Let`'s bespoke `"u64"` special case (~line 4794-4813): replace the
   cast-only block (`IntCast` from whatever `val`'s current type is to `u64`,
   with no re-lowering of the underlying marker expression) with an `else if
   type_name == "u64"` arm in the same chain as `"u8"`/`"u16"`/etc., calling
   `lower_narrow_literal(ctx, init, val, Ty::U64)`. This is not purely additive —
   see Risk Areas.

**`compiler/lower.vow`** (self-hosted lowerer) — the exact same eight edits,
using the self-hosted idiom (`if`/`==` chains, not `matches!`, and `ITY_U64()`
not `Ty::U64`):
1. `lower_narrow_literal` guard (~line 2056-2059).
2. `emit_narrow_integer_constant` (~line 1965-1969) — add an `ITY_U64()` branch
   emitting `IOP_CONST_U64()` / `IDATA_CONST_U64()`, mirroring
   `emit_integer_zero`'s existing `ITY_U64()` branch (~line 1986-1988).
3. Binary-op operand gate (~line 2315-2317, and the mirrored `result_ty`
   computation for unary neg at ~line 2360-2364 already includes `U64` — leave
   that one alone, it's already correct).
4. Call-argument gate (~line 2555-2558).
5. Assignment gate (~line 3373-3375).
6. Match-arm/Phi gate (~line 4399-4401).
7. Function trailing-expr-as-return gate (~line 5415-5417).
8. `Stmt::Let`'s `"u64"` special case (~line 4816-4819 in the current file, will
   shift after earlier edits) — same replacement as the Rust side.

**No `docs/spec/*.md` changes required.** `docs/spec/grammar.md` already states
"Checked operators abort with `ArithmeticOverflow` on overflow" without a `u64`
carve-out — the spec already describes the correct behavior; this PR makes the
implementation match it. No CLI flags, syntax, or semantics are changing.

**No `vow-codegen` or `vow-clif-shim` changes.** Confirmed by reading
`vow-codegen/src/cranelift_backend.rs:930-936` (`integer_is_signed` derived from
`InstData::Integer(IntegerType)`, and `IntegerType::U64` — `vow-ir/src/types.rs:80`
— is already `IntegerSignedness::Unsigned`) and `vow-clif-shim/src/lib.rs:123-125`
(`ity_is_signed` already excludes `ITY_U64`). Once the lowerer emits a
`u64`-typed `CheckedSub`/`CheckedAdd`/`CheckedMul`, both backends already select
`usub_overflow`/`uadd_overflow`/`umul_overflow` correctly.

## 3. TDD slices

Each slice is red (add fixture, confirm it fails against current `build/vowc`
and `target/release/vow`) then green (make the minimal edit from §2 that fixes
it) then confirmed in both compilers before moving on. Fixture naming mirrors
the existing `narrow_checked_expression_overflow.vow` pattern — direct literal
operands, no `seed()` indirection (the existing `u64_checked_*.vow` fixtures
deliberately route through a call and must stay green throughout, since they
prove the non-literal path already worked).

1. **Return position** (`tests/run/u64_checked_literal_sub_underflow.vow`):
   `fn f() -> u64 { 0 -! 1 } fn main() -> i32 { f(); 0 }` — `// TEST: exit 134`,
   `// TEST: stderr "{\"error\":\"ArithmeticOverflow\"}"`. This is the issue's
   exact repro. Red against both compilers today (confirmed empirically: exit
   0). Green after edits #1, #2, #7 (Rust) / #1, #2, #7 (self-hosted) from §2.
   Also add `+!` and `*!` siblings in the same fixture file or as three small
   fixtures — three files, not a cross-product of every position.

2. **`let`-binding position** (`tests/run/u64_checked_literal_let_underflow.vow`):
   `let x: u64 = 0 -! 1;` then use `x` (e.g. `print_u64`). Red today (confirmed:
   prints `18446744073709551615`, exit 0). Green after edit #8. This slice is
   the one that *replaces* behavior rather than adding a gate disjunct — run
   `tests/run/u64_literal_coercion.vow` and `tests/run/u64_marker_propagation.vow`
   immediately after this edit, before moving to the next slice. Those two
   fixtures exercise the exact paths the replaced special-case block used to
   serve (`let x: u64 = 100;`, `let a: u64 = n;` where `n` is a marker,
   `let b: u64 = if cond { 7 } else { 0 };`, `let c: u64 = loop { break 9; };`).
   If either shifts from its current passing output, the replacement isn't
   behavior-preserving for the non-buggy cases and needs to fall back to an
   additive fix (keep the cast block as a fallback after trying
   `lower_narrow_literal`) instead of a full replacement.

3. **Assignment position** (`tests/run/u64_checked_literal_assign_underflow.vow`):
   `let mut x: u64 = 5; x = 0 -! 1;`. Red today (confirmed). Green after edit #5.

4. **Call-argument position** (`tests/run/u64_checked_literal_arg_underflow.vow`):
   `fn takes(x: u64) -> u64 { x } ... takes(0 -! 1)`. Red today (confirmed).
   Green after edit #4.

5. **`--dump-ir` sanity check** (not a `tests/run/` fixture — an ad hoc manual
   check during implementation, same as the issue's own evidence): confirm
   `--dump-ir` on slice 1's fixture shows `CheckedSub[u64]` post-fix, matching
   the `CheckedSub[u32]` shape already shown for `u32` in the issue body. This
   is the cheapest signal that the gate opened, independent of whether the trap
   actually fires — keep it as a manual verification step, not a committed test,
   since `tests/run/` fixtures assert on process exit/stdout/stderr, not IR dumps.

6. **Regression sweep**: run `u64_checked_add_overflow.vow`,
   `u64_checked_mul_overflow.vow`, `u64_checked_sub_underflow.vow` (the existing
   `seed()`-indirected fixtures), `u64_basic.vow`, `u64_literal_coercion.vow`,
   `u64_marker_propagation.vow`, `issue840_u64_loop_carried.vow`,
   `issue843_u64_match_expr_phi.vow`, `issue851_option_u64_match_binding.vow`,
   and every `narrow_checked_expression_overflow.vow`-style fixture for the
   other integer widths (`i128`/`u128`/`i64` checked-overflow fixtures) after
   each edit. These prove the six gate edits are additive (`|| ty == U64`) and
   don't perturb i8/u8/i16/u16/i32/u32/i128/u128 behavior.

7. **`--mode release` parity**: rerun slices 1-4 with `--mode release` (per
   acceptance criterion 2) — `vow build` release mode still emits the abort path
   for checked arithmetic (only debug-mode adds *other* runtime vow checks; the
   arithmetic overflow trap itself is unconditional per `grammar.md`). Confirm
   exit 134 / `ArithmeticOverflow` in release mode too, not just debug.

## 4. Verification surface

No contract, `requires`/`ensures`, or C-model changes — this is pure IR-lowering
width/signedness selection, not new checked semantics. ESBMC's own
`u64` checked-arithmetic modeling (`docs/spec/grammar.md`: "`i64`/`u64` are
modelled; 128-bit checked arithmetic is reported `Skipped`") is unaffected: the
verifier consumes the same `CheckedSub`/`CheckedAdd`/`CheckedMul` opcodes with
`InstData::Integer(IntegerType)` that codegen does, so once the lowerer emits a
correctly-`u64`-typed op, `vow-verify`'s extraction of the verification
condition should already treat it as unsigned — worth a spot-check but not
expected to need a code change, since `vow-verify` does not have its own copy of
the narrow-literal gate list (it consumes post-lowering IR). Run `vow verify` on
one of the new `tests/run/` fixtures (with `+!`/`-!`/`*!` on `u64` literals) to
confirm the `ArithOverflowReachable` warning still fires correctly and the
verifier doesn't silently pass a now-genuinely-reachable trap. `tests/run/`
fixtures are the right home for the behavioral coverage (per acceptance
criteria); no new `examples/` needed.

## 5. Risk areas

- **Binary fixed point (`compiler/lower.vow`)**: every self-hosted edit must be
  an in-place `|| ty == ITY_U64()` / `else if type_name == String::from("u64")`
  disjunct added to an existing boolean expression or `if`-chain — **do not**
  introduce a new shared helper function to de-duplicate the six gates, even
  though the duplication is exactly how this bug happened and a shared predicate
  would be the better long-term shape. A new function changes function
  ordering, symbol emission, and Cranelift codegen sequencing in the self-hosted
  binary, which risks breaking the stage-1/stage-2 SHA-256 fixed point the
  acceptance criteria require (`scripts/bootstrap.sh --skip-cargo` binary
  reproduction). Keep the two compilers structurally isomorphic — each Rust
  `matches!` arm gets exactly one corresponding self-hosted `||`/`if` edit, in
  the same relative position. File a follow-up issue for centralizing the
  predicate once this lands green.
- **`Stmt::Let`'s replaced special case (edit #8, both compilers)**: this is the
  one edit in the set that removes existing behavior (the standalone `IntCast`
  block) rather than widening a guard. Slice 2 above is the gate on this: if
  `u64_literal_coercion.vow` or `u64_marker_propagation.vow` regress, the
  replacement is not behavior-equivalent and the fix must become additive
  (attempt `lower_narrow_literal` first, fall back to the old `IntCast` only
  when `lower_integer_marker_as` returns `None` and the type still doesn't
  match — which is in fact already `lower_narrow_literal`'s own fallback
  behavior, so a true regression here would be surprising, but must be checked,
  not assumed).
- **`emit_narrow_integer_constant`'s `unreachable!()` (Rust)**: this function is
  called from more than one place (`lower_integer_marker_as`, and directly at
  line ~1393 for enum-payload construction gated to `Ty::I128 | Ty::U128` only —
  that call site is unaffected since it's gated away from `U64`). Confirm no
  other caller reaches `Ty::U64` through a path not covered by this plan's
  edits, or the `unreachable!()` becomes a live panic instead of a silent bug.
- **`cargo clippy --all -- -D warnings`**: adding a `Ty::U64` arm to a `matches!`
  pattern list is unlikely to trigger new lints, but widening several `if`
  conditions increases branch count in already-long functions — watch for any
  new `clippy::too_many_lines` or similar if a function crosses a threshold
  (unlikely given the functions are already well over any such threshold and
  already allowed).
- **`parse → print → parse` idempotency**: not implicated — no AST or printer
  changes, this is IR lowering only.
- **Verification (ESBMC) surface**: low risk per §4, but confirm empirically
  rather than assuming, since a previously-unreachable-in-practice `u64`
  checked-op path becoming reachable is exactly the kind of change that could
  newly expose an existing `vow-verify` gap.

## 6. Out of scope

- **The four `Ty::I128 | Ty::U128`-only narrowing sites** (struct-field
  assignment via `.field = rhs`, struct-field construction, enum-payload
  construction, `Vec::push` element) — confirmed by code reading that these
  sites gate on `I128`/`U128` only and exclude `i8`/`u16`/`u32` too, not just
  `u64`. This contradicts the issue's own framing ("u64 is the only integer
  type affected") — if these are bugs, they're a pre-existing, non-u64-specific
  gap affecting every narrow type through those four syntactic positions, and
  belong to a separate issue, not this one.
- **The `contextual_narrow_literal_ty` fallback asymmetry** in the binary-op
  operand-type resolution (`vow-ir/src/lower/mod.rs` ~line 1566-1586): the
  final `else { lhs_ty }` fallback always prefers the left operand's type even
  when the *right* operand is the concretely-typed (non-literal) side and the
  left is the bare literal — e.g. `1 -! seed()` where `seed(): u64` would
  resolve `operand_ty` from the literal's default-`i64` left side rather than
  the call's `u64` right side. This is a distinct, likely pre-existing,
  non-u64-specific asymmetry (verified it does not affect this issue's repro
  cases, since `lhs_ty` already correctly equals `Ty::U64` whenever the
  concretely-typed operand is on the left, which is how all of this issue's and
  the existing `seed()`-indirected fixtures' cases are shaped). Do not fix here.
- **Centralizing the six duplicated gate lists into a shared predicate** — see
  Risk Areas. The right fix long-term, wrong fix for a PR that must preserve
  the self-hosted binary fixed point. Follow-up issue, not this PR.
- **No formatting, unrelated refactor, or cleanup** bundled into this change.
  Every edit in §2 is a single-token or single-line-condition addition (`||
  ty == ITY_U64()` / `Ty::U64`) except edit #8, which is a small, scoped
  replacement justified by §5.
