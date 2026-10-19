
### Bitwise Operators

| Operator | Meaning      |
|----------|--------------|
| `&`      | Bitwise AND  |
| `\|`     | Bitwise OR   |
| `^`      | Bitwise XOR  |
| `<<`     | Left shift   |
| `>>`     | Right shift  |

Bitwise `& | ^` require integer operands of the same type and work on all 10
integer widths. `>>` is **arithmetic** (sign-extending) for signed types
(`i8`..`i128`) and **logical** (zero-extending) for unsigned types
(`u8`..`u128`).

**Shift count type.** The right operand of `<<` and `>>` is `u32` for every
left-operand width (`i8`..`i128`, `u8`..`u128`): `let x: i64 = ...; let s: u32 = 3;`
`x << s` is well-typed and has type `i64`. Unsuffixed integer literals on the
right side context-coerce to `u32`: given `let x: u8 = ...`, `x << 3` is
well-typed (`3` coerces to `u32`). The left operand keeps its own integer type;
the shift result has the left operand's type. Any other count type is a
`TypeMismatch`. For 64- and 128-bit left operands only, a count of the same
type as the left operand (`i64 << i64`, `u64 >> u64`, `i128 << i128`,
`u128 >> u128`) is also accepted, because existing 64-bit bit-manipulation code
carries its count in the value's own type; narrow left operands accept `u32`
only.
An unsuffixed-literal left operand has no type of its own: with a non-literal
count it takes the count's type, and the pair must satisfy the rule above. So
`1 << n` is well-typed (and has type `u32`, `i64`, `u64`, ...) for a count `n`
of type `u32`, `i64`, `u64`, `i128` or `u128`, and a `TypeMismatch` for any
other count type (`i8`, `u8`, `i16`, `u16`, `i32`, `f64`, ...). Write `1u64 << n`
to fix the shifted type explicitly. The literal is not range-checked against
the count's type.

**Shift count range.** A const-expression shift count that is negative or
`>= bit-width(LHS)` is a compile-time error (`ShiftCountOutOfRange`), at every
width: `(x: u8) << 8` and `(x: i64) << 64` do not compile. Dynamic shift counts
(`x << n` where `n` is not a const expression) get a check on the operation
that the verifier proves: the count must satisfy `0 <= count < width(LHS)` at the point
of the shift, so a negative signed count is rejected as well. At runtime only
8-bit shifts trap (`i8`/`u8` with count `>= 8` aborts with `ArithmeticOverflow`);
wider shifts mask the count to the operand width, like the hardware shift.

Unsuffixed literal coercion still applies for `&`, `|`, `^` operands: with
`let x: u64 = ...`, `3 & x` and `x | 0xff` type-check because the literal
side coerces to `u64`. Use a suffix to force a different type explicitly.

### Logical Operators

| Operator | Meaning    |
|----------|------------|
| `&&`     | Logical AND (short-circuit) |
| `\|\|`   | Logical OR (short-circuit) |
| `!`      | Logical NOT|

`&&` and `||` use short-circuit evaluation: for `a && b`, `b` is only evaluated if `a` is true; for `a || b`, `b` is only evaluated if `a` is false.

### Operator Precedence

From loosest to tightest, Vow follows the usual C/Rust precedence for logical and bitwise operators:

`||`, `&&`, comparisons (`== != < <= > >=`), `|`, `^`, `&`, `<< >>`, `+ -`, `* / %`

Unary `-` and `!` bind tighter than every binary operator and tighter than
`as Type`, matching Rust: `-x as u64` is `(-x) as u64`; write `-(x as i64)` to
negate a cast result. The other postfix forms (`.field`, `.method()`,
`[index]`, `(args)`, `?`) bind tighter than unary, so `-v.len()` is
`-(v.len())`. `as Type` binds tighter than every binary operator, so
`a.len() as i64 + 1` is `(a.len() as i64) + 1`.

`&` is only the infix bitwise AND operator (`lhs & rhs`). There is no prefix
`&expr`: Vow has no borrow expressions, so `&x`, `&mut x`, `&&x` (and `x & &y`)
are `UnsupportedFeature` errors at the `&` (or `&&`) token, identically in both compilers (see
[errors.md](errors.md#unsupportedfeature)). Pass the value itself. The type
syntax `&T` is still accepted in signatures and annotations, but no expression
creates a value of that type: a `&T` parameter can only be passed on from
another `&T` parameter, so a program has no way to introduce one. Do not
declare reference-typed parameters.

### Unary Operators

| Operator | Meaning    |
|----------|------------|
| `-`      | Negation (not allowed on unsigned types); on `f32`/`f64` an IEEE sign flip, so `-0.0` is negative zero |
| `!`      | Logical NOT|
| `?`      | Unwrap (propagate error), postfix |

### Block-like Expressions and Parentheses

`if`, `match`, `while`, `for`, `loop`, and a `{ ... }` block are *block-like*.
An unparenthesised block-like expression ends the expression it starts: no
postfix operator (`.`, `[`, `(`, `?`, `as`) and no binary operator may follow it
directly, so `if c { 1 } else { 2 } as u64` and `if c { 1 } else { 2 } + 1` are
parse errors. As the right operand of a binary operator or the operand of a
unary operator it is fine (`3 * if c { 1 } else { 2 }`), but it still ends the
whole expression, so `3 * if c { 1 } else { 2 } as u64` is a parse error too. A parenthesised
expression is a primary expression whatever it contains, so every operator may
follow it:

```vow
let a: u64 = (if c { 1 } else { 2 }) as u64;
let b: i64 = (if c { 1 } else { 2 }) + 1;
let n: u64 = (if c { v } else { w }).len();
```

Parentheses are not an AST node: the canonical printer re-inserts them exactly
where a block-like expression, a binary or unary expression, an assignment, or
`break`/`return` is the left operand of a binary operator or the receiver of a
postfix operator, so `parse -> print -> parse` is idempotent.

An expression statement ends with `;`. Only two forms may omit it: the last
expression of a block (its value) and an unparenthesised block-like expression
(`if c { f(); } g();`). Any other statement without `;` is a parse error
(`UnexpectedToken`) at the next token, in both compilers, and parsing stops
there. A `let` statement's trailing `;` is optional.

An assignment's type as an expression is always `()`, independent of its
right-hand side's type — so a block, `if`-branch, or `match`-arm whose value is
a bare assignment (no `;`) is itself `()`-typed, not the assignment's RHS type.

A scalar type name after `as` (`i8` through `u128`, `f32`, `f64`, `bool`) never
takes generic arguments, so a following `<` is a comparison or shift:
`x as u64 < y` and `x as u64 << 1` mean `(x as u64) < y` and `(x as u64) << 1`.

### Type Cast

`as` is **widening-only** across integer types. Any narrower integer can be
cast to any wider integer; signed sources sign-extend, unsigned sources
zero-extend:

```vow
let a: i32 = -1;
let b: i64 = a as i64;     // sign-extend: -1_i64
let c: u8  = 200;
let d: u64 = c as u64;     // zero-extend: 200_u64
let e: u32 = 1;
let f: i64 = e as i64;     // unsigned-to-signed widening, value preserved
```

`as` between signed and unsigned of **the same width** is also allowed
(machine-level bit reinterpretation): `i64 as u64`, `u64 as i64`, `i32 as u32`,
etc.

**Narrowing via `as` is a compile-time error** (`NarrowingCastNotAllowed`):

```vow
let big: i64 = 300;
let small: u8 = big as u8;     // error — narrowing not allowed via `as`
```

To narrow, use a named intrinsic that makes the intent explicit. For every
narrowing pair `(src, tgt)` the compiler exposes three free functions:

| Intrinsic                         | Behavior on out-of-range input          |
|-----------------------------------|-----------------------------------------|
| `<src>_to_<tgt>_try(x) -> Option<tgt>` | returns `Option::None`             |
| `<src>_to_<tgt>_wrap(x) -> tgt`   | truncates (low bits, two's-complement)  |
| `<src>_to_<tgt>_sat(x) -> tgt`    | clamps to the target type's range       |

Example:

```vow
let big: i64 = 300;
match i64_to_u8_try(big) {
    Option::Some(b) => use_byte(b),
    Option::None    => fallback(),
}
```

These intrinsics are emitted by the compiler so the verifier sees their semantics
directly in its model (the C model under ESBMC, exact bit-vector terms under the
native verifier).

For the `u8` target, the available narrowing source types are `i16`, `i32`,
`i64`, `i128`, `u16`, `u32`, `u64`, and `u128`. Each source provides all three
forms, for example `u16_to_u8_try`, `u16_to_u8_wrap`, and `u16_to_u8_sat`.

For the `i32` target, the available narrowing source types are `i64`, `u32`,
`u64`, `i128`, and `u128`, each providing all three forms:
`i64_to_i32_try`/`_wrap`/`_sat`, `u32_to_i32_try`/`_wrap`/`_sat`,
`u64_to_i32_try`/`_wrap`/`_sat`, `i128_to_i32_try`/`_wrap`/`_sat`, and
`u128_to_i32_try`/`_wrap`/`_sat`.

The remaining executable sub-64-bit targets expose these complete families:

| Target | Narrowing source types |
|--------|------------------------|
| `i8`   | `i16`, `u16`, `i32`, `u32`, `i64`, `u64`, `i128`, `u128` |
| `i16`  | `i32`, `u32`, `i64`, `u64`, `i128`, `u128` |
| `u16`  | `i32`, `u32`, `i64`, `u64`, `i128`, `u128` |
| `u32`  | `i64`, `u64`, `i128`, `u128` |

Every listed source/target pair provides `_try`, `_wrap`, and `_sat`. Same-width
signedness changes use `as`, which reinterprets the bits and is not narrowing;
`i128`/`u128` additionally have the explicit same-width intrinsics described
below.

`i128` and `u128` appear in these tables only as sources: they are the widest
integer types, so no other width narrows into them. The one conversion between
them is the same-width sign change, which has `i128_to_u128_wrap`/`_sat` and
`u128_to_i128_wrap`/`_sat` (no `_try`, because 128-bit enum payloads are not
supported yet). There is no `i128`/`u128` to `i64`/`u64` narrowing family; use
`int128_to_string`/`uint128_to_string` to observe a full 128-bit value.

No implicit conversions: `i64 + u64` and `u8 + i32` are type errors. The
operands must already have the same type. The compiler does not coerce
across integer types at operator sites — only literals coerce, per the
[Integer Literals](#integer-literals) rules.

## Let Bindings

### Immutable

```vow
let x: i64 = 42;
x = 43;   // error[ImmutableAssignment]: declare it with `let mut x`
```

Bindings are immutable by default. Reassigning a binding that was not declared
`mut` is a compile error (`ImmutableAssignment`). `mut` is required **only** for
whole-binding reassignment `x = e`; field writes (`s.f = e`) and index writes
(`v[i] = e`) are permitted through any binding and do not require the base to be
`mut`.

### Mutable

```vow
let mut i: i64 = 0;
i = i + 1;
```

A `let mut` binding that is never reassigned is a compile error (`UnusedMut`) —
drop the `mut`. Because only whole-binding reassignment counts as a use of `mut`,
a binding mutated solely via `s.f = e`, `v[i] = e`, or a method call should be
declared `let`, not `let mut`.

### Pattern Destructuring

```vow
let (a, b): (i64, i64) = (1, 2);
let (a, (b, c)) = (1, (2, 3));
let (_, b) = (side_effecting_call(), 2);
```

`let` accepts identifier, wildcard (`_`), and tuple patterns, recursively for
nested tuples. There is no runtime tuple value: a tuple `let` pattern is a
compile-time desugaring into one independent `let` per leaf, so the
initializer must be a syntactic tuple literal of exactly the same arity at
every nesting level (`let (a, b) = f();`, where `f` returns a tuple, is
rejected — tuples are not yet first-class values). A wildcard leaf still
evaluates and lowers its corresponding initializer element for its effects;
it just binds no name. A tuple `let` pattern cannot bind a linear-typed
element. All other pattern kinds (literals, enum variants, struct patterns,
or-patterns) are refutable and are rejected in `let` position, since a `let`
pattern must always match. Rejected shapes produce `error[UnsupportedPattern]`
(see `errors.md`).

## Control Flow
