# 0001. Numeric tower — narrow integer types

**Status:** accepted (2026-05-22)

## Context

Vow today documents `i32`, `i64`, `u8`, `u64`, `f32`, `f64` as primitive numeric
types. The Rust-side typechecker silently accepts `i8`, `i16`, `i128`, `u16`,
`u32`, `u128` (`vow-types/src/types.rs`) and the self-hosted compiler has
matching token-suffix and type-tag scaffolding (`compiler/token.vow`,
`compiler/types.vow`) but the IR (`vow-ir/src/types.rs`) only has `I32`, `I64`,
`U64`, `F32`, `F64`. Casts (`vow-types/src/check.rs` ~L1542) are limited to
`i32 → i64`, `i32 → u64`, `i64 ↔ u64`. Vec<u8> works as an opaque slot but `u8`
arithmetic doesn't lower. Agents writing parsers, hashes, byte protocols, or
FFI shims hit this inconsistency.

This ADR records the decisions from the 2026-05-22 design session for the
*narrow integer* slice of the numeric tower. Floats and big-number (BigInt /
Decimal / Rational) work are separate subprojects and out of scope here.

## Decisions

1. **Width set.** Commit to the full Rust fixed-width matrix as first-class:
   `i8`, `i16`, `i32`, `i64`, `i128`, `u8`, `u16`, `u32`, `u64`, `u128`. No
   `isize`/`usize`. `Vec::len() -> i64` stays. Vow remains 64-bit-only to
   preserve binary-fixed-point reproducibility.

2. **IR opcodes.** Refactor the per-width arithmetic opcodes
   (`WrappingAddI32`, `WrappingAddI64`, `WrappingAddU64`, ...) into
   width-parametric opcodes (`WrappingAdd` with width + signedness carried on
   the instruction). Same plan applies to comparison and bitwise ops.

3. **Casts.** `as` covers **widening** (any narrower int → wider int; signed
   sign-extends, unsigned zero-extends) and **same-width signed/unsigned
   reinterpretation** (`i64 as u64`, `u64 as i64`, `i32 as u32`, etc. —
   machine-level bit reinterpretation, no range check). Narrowing via `as` is a
   compile-time error. Narrowing intent must be spelled at the call site with one
   of three compiler-emitted free functions per `(src, tgt)` pair:
   `<src>_to_<tgt>_try(x) -> Option<tgt>` (range-checked),
   `<src>_to_<tgt>_wrap(x) -> tgt` (truncate),
   `<src>_to_<tgt>_sat(x) -> tgt` (clamp).
   Emitted as intrinsics so ESBMC sees their semantics directly.

4. **Literals.** Unsuffixed integer literals default to `i64` and
   context-coerce to the annotated target type when the surrounding context
   fixes one (`let`, fn arg, struct field, bitwise/arith with typed operand).
   Out-of-range literals (`let x: u8 = 300;`) are a compile-time error
   (`LiteralOutOfRange`). Suffixed literals `42u8`, `42i128`, etc. are
   supported for all 10 widths.

5. **Arithmetic operators.** Keep `+` (wrapping) and `+!` (checked / traps on
   overflow) only. Saturating arithmetic is exposed as compiler-emitted
   intrinsic free functions (`add_sat_u8(a, b) -> u8`, etc.) — needs verifier
   semantics so it can't be pure stdlib. No new operator family.

6. **Bitwise.** `& | ^ << >>` work on all 10 int widths. Shift count is `u32`
   with literal coercion. Right-shift is arithmetic for signed types and
   logical for unsigned. A shift count that is a const expression `>= width`
   is a compile-time error (`ShiftCountOutOfRange`); dynamic shifts get a
   runtime contract (see the amendment to this decision below for what is
   actually checked).

7. **128-bit verification.** `i128`/`u128` are first-class for source, IR,
   codegen (Cranelift `I128`), and ESBMC (`__int128`). Predicates over 128-bit
   values may time out in the SMT solver; those proofs use the ordinary
   verifier controls and retain fail-closed `timeout` or `unknown` outcomes.
   Vow provides no type-specific verification opt-out. Contract authoring rules
   (CLAUDE.md "Contract Authoring") still apply — never weaken contracts to fit
   the verifier.

8. **Format / parse.** Two formatter baselines: `int_to_string(x: i64) -> String`
   and `uint_to_string(x: u64) -> String`. Agents widen via `as` before
   formatting (128-bit values add their own pair; see the 2026-10-09
   amendment). Parsing exposes `parse_X(s: String) -> Option<X>` for every
   width (the narrow variants reject out-of-range).

9. **Struct field layout.** Struct fields up to 64 bits wide each occupy one
   8-byte slot (narrow ints stored padded); `i128`/`u128` fields occupy two
   consecutive 8-byte slots (16 bytes). No packing, no natural-alignment
   layout, no FFI layout matching in Phase 1.

10. **Rollout.** Tracer-bullet by width, not by layer. Phase 1: `u8`
    end-to-end (typechecker, IR, codegen, verifier, narrowing intrinsics for
    `u8`, `parse_u8`, docs). Subsequent phases: `i32` (already partial),
    `i8`/`i16`/`u16`/`u32`, then `i128`/`u128`. Each phase is a complete
    vertical slice that lands as a small PR set.

## Compiler vs stdlib boundary

The rule that emerged: **a numeric feature belongs in the compiler iff it
requires a new IR opcode, a new type-system axis, or special verifier
modeling; otherwise stdlib.**

- **Compiler:** types, IR opcodes, codegen, verifier C model, arithmetic /
  bitwise operators, literal coercion, widening `as`, narrowing intrinsics,
  saturating arithmetic intrinsics, format/parse baselines, hardware-mapped
  bit-count intrinsics (`leading_zeros`, `popcount`, `byte_swap`).
- **Stdlib:** width-generalised math helpers (`abs`, `min`, `max`, `clamp`)
  per width — repetitive but no special verifier needs.

This rule is provisional. It needs to survive contact with the float and
BigInt subprojects.

## Considered options (and why rejected)

- **Just add `u8`.** Smallest surface, but the typechecker / self-hosted
  compiler already name the rest; leaving them as silent ghost types is worse
  than committing to the full matrix.
- **Add `isize`/`usize`.** Breaks 64-bit-only determinism; cross-compilation
  would produce different binaries. Rejected to preserve binary fixed point.
- **Full Rust-style `as` (silent wrapping narrows).** Vow's mission is to
  eliminate agent bug classes; silent narrowing is exactly such a class.
  Rejected.
- **Method-style narrowing (`x.to_u8()`).** Requires primitive-type methods —
  a new type-system axis. Rejected per the compiler/stdlib rule.
- **Add saturating operators (`+|`, `-|`, ...).** New lexer surface and
  precedence rules for a rarely-needed third arithmetic mode. Rejected;
  saturating ships as named intrinsics.
- **Packed / naturally-aligned struct fields for narrow ints.** Real win for
  FFI and wire formats, but redesigns `FieldGet`/`FieldSet`. Deferred; revisit
  when there's a concrete agent-driven need.

## Open follow-ups

- Const declarations: grammar.md §Const Declarations already specifies all 10
  integer widths plus `bool`; the implementation widening (still `i32/i64/bool`)
  is tracked in #527 (mechanical).
- Validate Cranelift `I128` codegen on the supported backends.
- Benchmark ESBMC `__int128` predicate complexity on real Vow contracts.
- Floats and BigInt subprojects: separate ADRs.

## Amendments

- **2026-07-31 — Decision 7.** The original decision promised a
  `--no-128-verify` flag. Issue #697 retired that unimplemented, type-specific
  opt-out. The later normative verifier-decoupling decision in
  `docs/design/verifier-model-bounds.md` requires prover limitations to remain
  internal to the verifier rather than create language or CLI escape hatches.
  Resource-limited 128-bit proofs therefore use the ordinary fail-closed
  verifier outcomes; users may still skip static verification for an entire
  build with the existing `vow build --no-verify` option.

- **2026-08-31 — Decision 1.** The decision's second sentence,
  "`Vec::len() -> i64` stays", is reversed by
  [ADR 0003](0003-unsigned-size-types.md): lengths, indices, and capacities
  become `u64`. The first sentence, "No `isize`/`usize`", is unaffected and
  still holds. Decision 1 conflated two separable claims — pointer-width types
  break the binary fixed point, whereas fixed-width `u64` does not — and the
  rejection rationale recorded under *Considered options* ("Breaks 64-bit-only
  determinism; cross-compilation would produce different binaries") applies
  only to the former. See epic #1104.
- **2026-10-03 — Decision 6.** "Dynamic shifts get a runtime contract" described
  the verifier check, not a runtime trap. The shipped behaviour is: ESBMC proves
  `0 <= count < width` at every dynamic shift; at runtime only 8-bit shifts
  (`i8`/`u8`, count `>= 8`) trap with `ArithmeticOverflow`, in every build mode,
  and wider shifts mask the count to the operand width. Both compilers behave
  identically. For 64- and 128-bit left operands a count of the left operand's
  own type is also accepted (`docs/spec/grammar.md`, "Shift count type").
- **2026-10-09 — Decision 8.** Widening via `as` cannot format most 128-bit
  values, and routing through the `_try` narrowing intrinsics fails for them.
  Phase 4 therefore adds two scalar formatters, `int128_to_string(x: i128)` and
  `uint128_to_string(x: u128)`, per the #526 planning comment of 2026-07-19.
- **2026-10-09 — Decisions 8 and 9.** Decision 9's two-slot layout is
  implemented for `i128`/`u128` **enum payloads** (`Option`, `Result`, user
  enums): the low limb sits in the payload's slot and the high limb in the next
  one, so later payloads move up by one slot, and lowering marks these accesses
  with a distinct `WideSlot` instruction datum. Struct fields and `Vec` elements
  keep the 8-byte-slot representation and are still refused at codegen. The
  runtime's 128-bit `Option` cell is `[tag, lo, hi]`. Decision 8's
  `parse_i128`/`parse_u128` ship with it and remain unknown to the verifier's
  64-bit `Option` model, so functions using them are `Skipped`. See #1543.
- **2026-10-09 — Decision 9, struct fields and `Vec` elements.** The two-slot
  layout now covers `i128`/`u128` **struct fields** and `Vec<i128>`/`Vec<u128>`
  **elements** (#1569), so no aggregate position is refused at codegen any
  more. A struct has no tag, so its slots start at 0 and a wide field shifts
  every later field up by one slot; the allocation is `(slots + 1) * 8`, the
  extra slot being the guard. A `Vec` element is 16 bytes, low limb first,
  with `len` and `cap` still counting elements and the descriptor unchanged.
  Lowering reads and writes both through the `WideSlot` marker: struct accesses
  directly, `Vec` accesses through bounds-checked element-address helpers
  (`__vow_vec_push_wide_ptr`, `__vow_vec_get_wide_ptr`,
  `__vow_vec_set_wide_ptr`), so no 128-bit value crosses the extern ABI beside
  another argument. The element width comes from the checker, per `Vec` access.
  `pin_to_root` and `Vec::from_raw_parts_copy` copy one 8-byte slot per
  element and therefore reject 128-bit element types. The ESBMC backend still
  skips these accesses. `--backend native` models 128-bit struct fields and enum
  payloads (#1421); `Vec` elements stay skipped until the native `Vec` model
  (#1423) lands.
