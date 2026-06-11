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
   runtime contract.

7. **128-bit verification.** `i128`/`u128` are first-class for source, IR,
   codegen (Cranelift `I128`), and ESBMC (`__int128`). Predicates over 128-bit
   values may time out in the SMT solver; a `--no-128-verify` opt-out is
   provided. Contract authoring rules (CLAUDE.md "Contract Authoring") still
   apply — never weaken contracts to fit the verifier.

8. **Format / parse.** Two formatter baselines: `int_to_string(x: i64) -> String`
   and `uint_to_string(x: u64) -> String`. Agents widen via `as` before
   formatting. Parsing exposes `parse_X(s: String) -> Option<X>` for every
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
