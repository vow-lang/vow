# Issue #1055 implementation plan

## Accepted design

Integer literals store an unsigned 128-bit magnitude. Negation remains a separate
unary AST expression. Rust syntax uses native `u128`; serialized IR and the
self-hosted compiler agree on two limbs `(lo: u64, hi: u64)`, least-significant
limb first. The self-hosted token and AST arenas use dedicated limb payloads;
`int_val2` remains the suffix tag.

This seam adds representation only. It does not implement i128/u128 arithmetic,
Cranelift emission, or ESBMC emission. Existing i8..u64 suffix behavior is not
changed; its known parser suffix-loss bug is tracked separately.

## Vertical slices

1. Rust syntax red/green
   - Add lexer tests for `i128::MAX`, `u128::MAX`, and true overflow.
   - Widen token and AST magnitudes to `u128`.
   - Add only the new explicit `i128`/`u128` parser suffix paths alongside `u64`.
   - Confirm unary minus is still parsed as a separate prefix expression.

2. Rust IR red/green
   - Add IR serialization/printer round-trip coverage for both new constants.
   - Add `ConstI128(i128)` and `ConstU128(u128)` and deterministic two-limb
     serialization.
   - Route resolved i128/u128 literals directly to the new constants and add
     required printer/serializer/validator consumers.
   - Keep codegen and verification semantics deferred; add fail-closed stubs only
     where exhaustive Rust matches require them to compile.

3. Self-hosted red/green
   - Add a self-hosted lexer test for small, cross-i64, max-u128, and overflow
     inputs.
   - Add overflow-aware `value = value * 10 + digit` over `(lo, hi)` using
     checked-by-construction 64-bit chunks.
   - Thread limbs through Token, AstArena, parser, checker/lowerer, and IR data.
   - Add I128/U128 opcode and data-kind identities without backend arithmetic.

4. Public fixtures and documentation
   - Add parse/print-only i128/u128 round-trip fixtures covering boundary values.
   - Document the magnitude/limb/sign representation in `docs/spec/grammar.md`.
   - Regenerate embedded help/skill artifacts.
   - File the required follow-up for pre-existing non-u64 suffix loss.

5. Quality gate and delivery
   - Run `cargo fmt --all`, `cargo clippy --all -- -D warnings`,
     `cargo test --all`, `scripts/full_test.sh`, and `build/vowc test compiler/`
     separately. Prefix direct self-compiled binary invocations with
     `ulimit -v 2000000`.
   - Review the diff, commit focused slices as `p@ocmatos.com`, push the assigned
     branch, open a non-draft PR, and remove the readiness label if present.
