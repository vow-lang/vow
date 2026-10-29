
### Integer Literals

```vow
42
-1
0
```

Unsuffixed integer literals default to `i64` in expression position, and
**context-coerce** to any of the 10 integer types when the
surrounding context fixes one — `let` bindings, function arguments, struct
fields, and the typed operand of an arithmetic, bitwise, or comparison
operator. The same coercion applies to constant expressions composed entirely
of unsuffixed integer literals (e.g. `1 + 2`, `1 << 3`, `-5`).

Out-of-range literals in a typed context are a compile-time error:

```vow
let x: u8 = 300;   // error: LiteralOutOfRange — 300 does not fit in u8
let y: i8 = 200;   // error: LiteralOutOfRange — i8 range is -128..=127
```

**Suffixed integer literals** force the type at the literal:

```vow
42u8     42u16     42u32     42u64     42u128
42i8     42i16     42i32     42i64     42i128
```

Suffixed forms are supported for all 10 integer widths. A suffixed literal is
sugar for the cast `NNN as T` (so `42u8` and `42 as u8` are the same AST and
print as `42 as u8`). The suffix fixes the literal's type: it is never
coerced to the surrounding context, so `let x: i64 = 5u8;` is a `TypeMismatch`.
The value is range-checked against the suffix type, so `256u8` is a
`LiteralOutOfRange` error. A negated literal is checked as a negative value
(`-128i8` is valid), and negating an unsigned literal is a `TypeMismatch`
(`-1u32` is an error). Suffixes inside `match` patterns are not part of this rule.

There are no `usize`/`isize` suffixes, because there are no such types. A literal
carrying one is an `InvalidIntSuffix` error at lex time rather than a silently
dropped suffix:

```vow
let n: u64 = 5usize;   // error: InvalidIntSuffix — write 5u64
```

Integer tokens store their decimal digits as an unsigned magnitude. A leading
`-` is a separate unary-negation expression and is not part of the literal.
For `i128` and `u128`, compiler IR and serialized modules carry that value as
two unsigned 64-bit limbs `(lo, hi)`, least-significant limb first. This wire
layout is shared by the Rust bootstrap and self-hosted compilers.

### Float Literals

```vow
3.14
-0.5
```

Out-of-range literals are a compile-time error, same as the integer case: a
literal whose magnitude exceeds `f64::MAX` (~1.7976931348623157e308) parses
to infinity and is rejected as `InvalidCharacter` at lex time, rather than
silently becoming a float value that has no valid C double-literal
representation:

```vow
let x: f64 = 99999...9.0;   // 310 nines — error: InvalidCharacter — float literal out of range
```

### Boolean Literals

```vow
true
false
```

### String Literals

```vow
"hello, world"
"line one\nline two"
"tab\there"
"null\0byte"
"escaped\\backslash"
"escaped\"quote"
```

Supported escape sequences: `\n`, `\t`, `\r`, `\\`, `\"`, `\0`.

A literal that reaches end of file without a closing quote, or a trailing `\`
with nothing to escape, is an `UnterminatedString` error. A byte that starts no
token is an `InvalidCharacter` error. Neither is skipped silently.

String literals have type `String` and are backed by a read-only static
descriptor. Passing or returning a literal does not allocate. To obtain a
mutable, arena-owned copy, use `String::from("...")`.

## Operators

### Wrapping Arithmetic (default)

| Operator | Meaning        |
|----------|----------------|
| `+`      | Add (wrapping) |
| `-`      | Sub (wrapping) |
| `*`      | Mul (wrapping) |
| `/`      | Div (wrapping) |
| `%`      | Rem (wrapping) |

Wrapping operators silently wrap on overflow. For unsigned operands, including
`u8`, division and remainder use unsigned semantics.

Division and remainder are the exception when no wrapped result exists. A zero
divisor aborts with `ArithmeticOverflow` for `/`, `%`, `/!`, and `%!`; signed
`MIN / -1` likewise aborts for both `/` and `/!`. These rules apply at every
integer width. Signed `MIN % -1` is representable as `0` and does not abort.

For `f32` and `f64`, the unchecked `+`, `-`, `*`, and `/` operators lower to
native floating-point arithmetic. Unchecked `%` is accepted by the frontend
and lowers to a floating-point remainder opcode, but native backends do not yet
implement that opcode; a build fails closed with `CodegenUnsupported`. The
checked operators (`+!`, `-!`, `*!`, `/!`, `%!`) are rejected on `f32`/`f64`
operands at type-check time with `UnsupportedFeature`: "checked" means "abort
instead of wrapping on integer overflow," and IEEE-754 float arithmetic has no
wrapping semantics to check against, so there is no principled meaning to give
a checked float operator. Use the unchecked operators for float arithmetic.

### Checked Arithmetic

| Operator | Meaning           |
|----------|-------------------|
| `+!`     | Add (checked)     |
| `-!`     | Sub (checked)     |
| `*!`     | Mul (checked)     |
| `/!`     | Div (checked)     |
| `%!`     | Rem (checked)     |

Checked operators abort with `ArithmeticOverflow` on overflow. Operands must be
integer types; `f32`/`f64` operands are rejected at type-check time (see
above).

**In verification.** The abort is modelled, not ignored: a checked operator is a
strictly different proof obligation from its wrapping sibling. An execution that
overflows aborts and therefore never returns, so it cannot witness a violated
`ensures` or `invariant` — which is why replacing `+` with `+!` can turn a
counterexample into a proof. Whether such an aborting execution is *reachable* is
reported separately, as an
[`ArithOverflowReachable`](errors.md#arithoverflowreachable) warning, so a proof
never hides a program that can die at the operator. Checked operators do not require the `[panic]` effect (see [Effect Types](#effect-types)). Widths `i8`/`u8` through
`i64`/`u64` are modelled; 128-bit checked arithmetic is reported `Skipped`
(fail-closed) rather than modelled as wrapping. See
[`verifier-discipline.md`](../verifier-discipline.md).

**128-bit arithmetic.** All operators work on `i128`/`u128` operands. `/`, `%`,
`/!`, `%!`, and `*!` have no native lowering at 128-bit width, so the compiler
routes them through runtime helpers; this is invisible in the language, and
their trap behaviour matches the narrower widths exactly. Division or
remainder by zero aborts at every width, as does signed `/` and `/!` on
`MIN / -1`, whose quotient is not representable. `MIN % -1` is `0` and does
not abort.

128-bit values carry both limbs through locals, parameters, returns,
temporaries, struct fields, `Vec` elements, and the payloads of `enum`,
`Option`, and `Result` values (stored in two consecutive slots, see the
aggregate layout above). The width comes from the checker, so it holds wherever
the value came from: built in the same function, received as a parameter,
handed back by a call, or read out of a `Vec` element or a struct field; through
a `match` arm, `?`, or `.unwrap()`; at every payload position; and at every
`Vec` index read or write, `push`, and `for` loop. `pin_to_root` and
`Vec::from_raw_parts_copy` copy one 8-byte slot per element, so they reject
`Vec<i128>`/`Vec<u128>` with a `TypeMismatch`; build such a `Vec` with `push`.

Verification is a separate matter. 128-bit literals (in a body or in
`requires`/`ensures`/`invariant`) are verifiable, and are modelled exactly under
the bit-vector encoding (both backends), `i128::MIN`, `i128::MAX` and `u128::MAX` included; under
ESBMC a proof obtained by the `--encoding ir` timeout fallback is weaker (it is
reported as plain `proven`). Under the ESBMC backend every 128-bit aggregate access
is `Skipped` rather than modelled: a contracted function that reads or writes a
128-bit aggregate field or enum payload is reported `Skipped`, with `FieldGet at
128-bit width` or `FieldSet at 128-bit width`, rather than being modelled through
the verifier's 8-byte heap slot. `--backend native` models a 128-bit struct
field or enum payload exactly, as one 128-bit term (it occupies two slots, and a
narrow field declared after it lives in the shifted slot). A function that
reads, writes, or pushes a `Vec<i128>` or `Vec<u128>` element is `Skipped`
under ESBMC (an unsupported `Call extern` opcode); `--backend native` models the
element of a parameter, a local `Vec` or one held in a struct field exactly as one
128-bit value (a write through a struct field is `unsupported-opcode`). A call to `parse_i128` or
`parse_u128` is `Skipped` under ESBMC, whose 64-bit `Option` model cannot represent the
result (the native verifier gives the payload its 128 bits and models the call
as an opaque total function of the string). Contracts over 128-bit parameters
alone do verify.

Runtime violation values are *not* one of those gaps: a scalar `i128`/`u128`
binding captured by a `vow` block reports its full value in the runtime
`VowViolation` `values` map. See [`errors.md`](errors.md#vowviolation) for how
that value is encoded and what a consumer must do to read it exactly.

### Saturating Arithmetic

Saturating arithmetic uses named compiler intrinsics rather than a third
operator family. The `u8` intrinsics are:

| Function | Signature | Behavior |
|----------|-----------|----------|
| `add_sat_u8` | `fn(a: u8, b: u8) -> u8` | clamps sums above 255 to 255 |
| `sub_sat_u8` | `fn(a: u8, b: u8) -> u8` | clamps differences below 0 to 0 |
| `mul_sat_u8` | `fn(a: u8, b: u8) -> u8` | clamps products above 255 to 255 |

These functions are pure and have direct verifier semantics; they do not
lower to wrapping arithmetic.

### Comparison Operators

| Operator | Meaning                |
|----------|------------------------|
| `==`     | Equal                  |
| `!=`     | Not equal              |
| `<`      | Less than              |
| `<=`     | Less than or equal     |
| `>`      | Greater than           |
| `>=`     | Greater than or equal  |

For `f32` and `f64` operands, the six operators are IEEE-754 comparisons.
Any comparison with a NaN operand is false, except `!=`, which is true. Both
operands must have the same type.
