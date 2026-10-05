# Vow Grammar Reference

Complete grammar for the Vow programming language. Vow source files use the `.vow` extension.

**Line comments.** `//` starts a line comment extending to end of line. Comments are stripped during lexing and never enter the token stream. Block comments (`/* */`) are not supported. Machine-relevant intent belongs in contracts; comments are for non-semantic rationale.

## Module Declaration

Every file begins with a module declaration:

```
module <Name>
```

`<Name>` is a PascalCase identifier. There is no semicolon.

## Use Declarations

Import other modules with dot-separated paths:

```
use foo.bar
```

This resolves relative to the main source file. The module loader first uses
`<rootdir>/foo/bar.vow.d` when that declaration stub exists, and otherwise
falls back to `<rootdir>/foo/bar.vow`. If the stub's declarations carry a
`vow` block, the stub cannot be relied on: a bodyless declaration has no
implementation for the verifier to check a call site against, so a contract
there would otherwise be silently dropped from verification. In that case the
loader loads the sibling `<rootdir>/foo/bar.vow` source instead, where the
usual intra-module `requires`-as-assert/Caller-blame mechanism applies
unchanged. A stub whose declarations carry no `vow` block is unaffected and
is still preferred over source. A stub shipped with no sibling `.vow` source
at all (e.g. a library distributing only its interface) is also unaffected —
a call through it remains non-modelable in the verifier (`Skipped`, never
falsely `Verified`).

## Const Declarations

Named constants with compile-time values:

```vow
const MAX_SIZE: i64 = 1024;
const NEG_ONE: i64 = -1;
const DEBUG: bool = true;
const SLOTS: u64 = 8;
const MAX_BYTE: u8 = 255;
```

Supported value forms: integer literals, boolean literals, negated integer literals. Constants are inlined at every use site (zero runtime cost). The type must be any of the 10 integer types (`i8`, `i16`, `i32`, `i64`, `i128`, `u8`, `u16`, `u32`, `u64`, `u128`) or `bool`. Integer constants are subject to the same compile-time range check as integer literals: a value outside the declared type's range, including a negative value for an unsigned type, is `LiteralOutOfRange`. A constant keeps its declared type at every use (it does not context-coerce like an unsuffixed literal), so a `u64` constant indexes a `Vec` directly and an `i64` constant does not. Constants are referenced by name in expressions like any other identifier.

## Functions

### Pure Function

```vow
fn add(x: i64, y: i64) -> i64 {
    x + y
}
```

### Function with Effects

```vow
fn main() -> i32 [io] {
    print_str("hello");
    0
}
```

Effects appear in brackets after the return type: `[io]`, `[read, write]`, `[io, panic]`.

### Declaration-Only Function

```vow
fn add(x: i64, y: i64) -> i64;
```

A semicolon in place of the body declares only the function signature. The
type checker registers the signature, but body checking and IR lowering skip
the declaration. `vow decl` emits these declarations in `.vow.d` stubs.

### Function with Vow Block

```vow
fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
} {
    x / y
}
```

The `vow` block sits between the signature and the body. Clauses:
- `requires: <expr>` — precondition (blame: Caller)
- `ensures: <expr>` — postcondition (blame: Callee); use `result` for the return value
- `invariant: <expr>` — loop invariant (blame: Callee)

Multiple clauses are separated by commas:

```vow
fn clamp(x: i64, lo: i64, hi: i64) -> i64 vow {
    requires: lo <= hi,
    ensures: result >= lo,
    ensures: result <= hi
} {
    if x < lo { lo } else { if x > hi { hi } else { x } }
}
```

### Where Clauses (Refinement Types on Parameters)

```vow
fn safe_sub(a: i64 where a >= 0, b: i64 where b >= 0) -> i64 vow {
    requires: a >= b,
    ensures: result >= 0
} {
    a - b
}
```

`where` constraints on parameters become additional `requires` in verification (and Caller-blame runtime checks under `--mode debug`). A `where` clause is checked exactly like a `requires` clause, in a scope holding **only its own parameter** plus module constants and functions:

- It can only reference its own parameter. A sibling parameter or `result` is an undefined name (`TypeMismatch`, "undefined variable"), with a hint pointing at `requires`/`ensures` for conditions that span several parameters or the return value. Any other undefined name is the same error with the usual "did you mean" hint. This is a type error in both compilers; it never reaches IR lowering.
- It must evaluate to `bool` (`ContractTypeMismatch`, hint "parameter `where` clauses must evaluate to `bool`").
- It must be pure: no call to an effectful function and no heap write through an argument (`EffectViolation`, see "Contract Purity").
- It cannot contain a tuple expression (`UnsupportedFeature`, "tuple expressions are not supported in contract predicates").
- Integer literals are range-checked against the compared type, and the unsigned-comparison rules apply as in any other expression (`LiteralOutOfRange`, `TautologicalComparison`).

On a declaration-only function (`fn f(x: i64 where x > 0) -> i64;`) and on the parameters of an `extern` function the clause is checked by the same rules but has no body to enforce it in: state the foreign function's real preconditions in its `vow` contract.

### Public Functions

```vow
pub fn api_function(x: i64) -> i64 {
    x
}
```

## Types

### Primitive Types

| Type   | Description              |
|--------|--------------------------|
| `i8`   | 8-bit signed integer     |
| `i16`  | 16-bit signed integer    |
| `i32`  | 32-bit signed integer    |
| `i64`  | 64-bit signed integer    |
| `i128` | 128-bit signed integer (verifier may time out; see below) |
| `u8`   | 8-bit unsigned integer   |
| `u16`  | 16-bit unsigned integer  |
| `u32`  | 32-bit unsigned integer  |
| `u64`  | 64-bit unsigned integer  |
| `u128` | 128-bit unsigned integer (verifier may time out; see below) |
| `f32`  | 32-bit float (limited support — avoid in contracts) |
| `f64`  | 64-bit float (limited support — avoid in contracts) |
| `bool` | Boolean                  |
| `()`   | Unit type; its only value is also written `()` (not allowed as a parameter type) |
| `!`    | Never type (diverges)    |

Vow targets 64-bit only and has no `isize`/`usize`. Excluding pointer-width
types preserves binary fixed-point reproducibility across compilation hosts;
see [ADR 0001](../adr/0001-numeric-tower-narrow-ints.md). The signedness of a
length is independent of this determinism rationale, so
[ADR 0003](../adr/0003-unsigned-size-types.md) makes lengths fixed-width `u64`:
`.len()` on `Vec`, `String`, `HashMap`, and `BTreeMap` returns `u64`. A `Vec`
index expression has exactly the type `u64` (see [Indexing](#indexing)), so
`v[i]` with `i: u64` needs no cast, and `v[i]` with `i: i64` is a
`TypeMismatch`. `String` offsets (`byte_at`, `substr`, `substring`,
`matches_literal_at`) are `u64` too, and `push_byte` takes a `u8`; see
[String offsets](#string-offsets).

**128-bit implementation status:** `i128`/`u128` types and full-range literal
representation are available to the frontend and IR. Native code generation,
arithmetic, and ESBMC modelling remain unsupported; builds and verification
fail closed when those deferred operations reach a backend. Later numeric-tower
work will complete those paths. Never weaken contracts to fit the verifier.

**Struct field layout:** the current aggregate representation assigns one
8-byte slot to every field regardless of declared type (narrow ints are
padded). The two-slot layout for `i128`/`u128` accepted by
[ADR 0001](../adr/0001-numeric-tower-narrow-ints.md) is not implemented yet,
so the compiler refuses reads and writes of 128-bit fields instead of storing
them in an undersized slot. There is no packing or natural-alignment layout
today; FFI structs that need a specific C layout must shim through `Vec<u8>` or
extern wrappers.

### Built-in Parameterized Types

| Type               | Description                     |
|--------------------|---------------------------------|
| `Vec<T>`           | Growable array. `T` must be non-linear (see [Linear Structs](#linear-structs)) |
| `Option<T>`        | Optional value (Some/None)      |
| `Result<T, E>`     | Success or error                |
| `String`           | UTF-8 string (backed by Vec<u8>)|
| `HashMap<K, V>`    | Key-value map (linear scan). `K` must be an integer type of at most 64 bits or `bool`; `V` may be any non-linear type except `i128`/`u128`/`f32`/`f64` |
| `BTreeMap<K, V>`   | Sorted key-value map (binary search; ascending iteration). `K` must be `i64`; `V` may be any non-linear type except `i128`/`u128`/`f32`/`f64` |

### Slice Types

The syntax `[T]` is not a type in Vow. It parses, but the type checker rejects it
wherever a type is written (parameter, return, field, enum payload, `let`
annotation, cast target, alias or constant) with `UnsupportedFeature` ("slice types
(`[T]`) are not supported in Vow"), once per bracket pair. No expression creates,
indexes, iterates or measures a slice, so no value of that type could exist. Use
`Vec<T>` to hold a sequence of values. See
[Slice types](errors.md#slice-types) for the diagnostic.

### User-Defined Types

Structs and enums (see below). A struct, enum or type alias may not be named after
a type the language already binds: a primitive type (`i64`, `bool`, `String`, ...)
or one of `Vec`, `Option`, `Result`, `HashMap`, `BTreeMap`. The declaration is
rejected with `UnsupportedFeature` ("`Vec` is a builtin type name and cannot be
declared as a user type"), because the resolver binds those names
before any user type and the user type would otherwise alias the builtin in some
positions and shadow it in others. See
[Reserved type names](errors.md#reserved-type-names) for the diagnostic.

## Literals

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

Suffixed forms are supported for all 10 integer widths. They override context
coercion and are still subject to the same compile-time range check.

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
never hides a program that can die at the operator. Widths `i8`/`u8` through
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

128-bit values are also **scalar-only** for now. Locals, parameters, returns,
and temporaries carry both limbs correctly, but a 128-bit value placed inside
an aggregate does not: `Vec<i128>`/`Vec<u128>` elements are refused because the
element helpers are i64-only, and reading or writing a 128-bit struct field —
or constructing a 128-bit enum, `Option`, or `Result` payload — fails codegen
with a named limitation rather than a raw backend verifier dump, before an
8-byte slot can truncate the value or a 16-byte store can overwrite its
neighbour. The refusal is at the access, not the declaration: a struct or enum
may declare a 128-bit member and still compile as long as nothing touches it.
The refusal does not depend on where the value came from: a 128-bit payload
read out of an `enum`, `Option`, or `Result` value the function never built —
a parameter, or a value handed back by a call — is refused on the declared
payload width, at every payload position rather than just the first, and
whether the read goes through a `match` arm or `.unwrap()`. Do not store
128-bit values in aggregates yet.

These are backend gaps, not language rules; the type checker accepts all of
these at 128-bit width. Verification is a separate matter: a contracted
function whose body contains a 128-bit *constant* is reported as `Skipped`
with `unsupported opcode ConstI128`, because `ConstI128`/`ConstU128` are not
yet modelled in the verifier. A contracted function that reads or writes a
128-bit aggregate field is likewise reported `Skipped`, with `FieldGet at
128-bit width` or `FieldSet at 128-bit width`, rather than being modelled through
the verifier's 8-byte heap slot. Contracts over 128-bit parameters alone do
verify.

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
that ESBMC proves: the count must satisfy `0 <= count < width(LHS)` at the point
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

Unary `-` and `!` bind tighter than every binary operator. The postfix forms
(`.field`, `.method()`, `[index]`, `(args)`, `?`, and `as Type`) bind tighter
still, so `-x as u64` is `-(x as u64)` and `a.len() as i64 + 1` is
`(a.len() as i64) + 1`.

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
| `-`      | Negation (not allowed on unsigned types) |
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

These intrinsics are emitted by the compiler so ESBMC sees their semantics
directly in the verification C model.

For the `u8` target, the available narrowing source types are `i16`, `i32`,
`i64`, `i128`, `u16`, `u32`, `u64`, and `u128`. Each source provides all three
forms, for example `u16_to_u8_try`, `u16_to_u8_wrap`, and `u16_to_u8_sat`.

For the `i32` target, the available narrowing source types are `i64`, `u32`,
and `u64`, each providing all three forms: `i64_to_i32_try`/`_wrap`/`_sat`,
`u32_to_i32_try`/`_wrap`/`_sat`, and `u64_to_i32_try`/`_wrap`/`_sat`.

The remaining executable sub-64-bit targets expose these complete families:

| Target | Narrowing source types |
|--------|------------------------|
| `i8`   | `i16`, `u16`, `i32`, `u32`, `i64`, `u64` |
| `i16`  | `i32`, `u32`, `i64`, `u64` |
| `u16`  | `i32`, `u32`, `i64`, `u64` |
| `u32`  | `i64`, `u64` |

Every listed source/target pair provides `_try`, `_wrap`, and `_sat`. Same-width
signedness changes use `as`; they are bit reinterpretations, not narrowing.

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

### If / Else

```vow
if x > 0 {
    x
} else {
    0 - x
}
```

`if`/`else` is an expression — both branches must have the same type. The condition must have type `bool` (or `never`). There is no `else if` keyword; nest `if` inside `else`:

```vow
if x < lo {
    lo
} else {
    if x > hi {
        hi
    } else {
        x
    }
}
```

### While Loop

```vow
while i > 0 {
    i = i - 1;
}
```

### While Loop with Invariant

```vow
let mut i: u64 = 0u64;
while i < n vow {
    invariant: i <= n,
    invariant: v.len() == i
} {
    v.push(i);
    i = i + 1;
}
```

State the bound the loop actually maintains. `invariant: i >= 0` looks like a
lower bound but is always true once `i` is unsigned, and the type checker
rejects it as `TautologicalComparison`. `.len()` is `u64`, so a length clause
compares directly against a `u64` counter; a counter of any other type needs
an explicit `as` cast.

### Descending Loops

A descending index loop guards on `> 0` and decrements as the first statement
of the body:

```vow
let n: u64 = v.len();
let mut i: u64 = n;
let mut acc: u64 = 0u64;
while i > 0 vow { invariant: i <= n } {
    i = i - 1;
    acc = acc + v[i];
}
```

`i` starts at `n`, not at `n - 1`, and the decrement brings it into range
before the first use. The guard is therefore also the bounds check: every
`v[i]` in the body runs with `i < n`.

One constraint on the shape above is easy to miss: the body must stay pure.
A `print_*` call inside a contracted function gives that function an effect,
and the verifier model is restricted to pure functions, so the contract is
reported as `VerificationSkipped` rather than checked.

This idiom does not by itself make a length-bounded loop provable. ESBMC's
unwind bound sits below the modelled collection capacity, so this form and the
signed `while i >= 0` form both return `unknown`. What the idiom buys is a
*writable* invariant rather than a stronger proof: `i <= n` is expressible on
an unsigned index, whereas the signed form's companion clause `i >= -1` is not
— a negative literal does not fit `u64` at all.

#### Never `while i >= 0`

```vow
let mut i: u64 = v.len() - 1;   // wraps on an empty collection
while i >= 0 { ... }            // never exits; rejected by the checker
```

Both lines are broken independently. `i >= 0` is universally true on an
unsigned type, so the loop has no exit; the type checker rejects the
comparison outright as `TautologicalComparison`, in a loop condition just as
in a contract clause. The initializer is the half no diagnostic catches: on an
empty collection `v.len() - 1` wraps to `18446744073709551615` and the
first index read runs far out of bounds.

#### `-` inside the guard, `-!` outside it

Plain wrapping `-` is the correct decrement in the canonical form. The guard
`i > 0` has already discharged the underflow obligation, so no check is
needed, and plain `-` is exactly what ESBMC's bitvector encoding models — a
*wrong* guard therefore stays observable through `invariant: i <= n`. Reserve
`-!` for decrements whose non-negativity the guard does not establish.

Note one asymmetry while reading that advice. `-!` does trap at runtime under
`--mode debug`, reporting `{"error":"ArithmeticOverflow"}`, but the verifier
does not model checked arithmetic at all: `n -! 1` and `n - 1` emit the same C
and yield byte-identical counterexamples. Do not reach for `-!` expecting it
to make an underflow statically visible.

#### Why there is no reverse-range syntax

Vow has no numeric range form, and adding one is rejected rather than pending:

- `loop` is not an escape hatch. `break value` is restricted to `loop`, and
  ESBMC cannot verify an unbounded `loop` at all.
- A reverse for-each would cover very little. Most reverse scans need the
  *index*, not the element — they read a parallel array at `i`, or write a
  second collection at the same `i`.
- A range desugar would have to thread through index-type decisions that are
  hardcoded in both lowerers, where a miss is silent rather than loud: a
  signed comparison applied to unsigned operands raises no type error.
- A genuine range type would add a new type-system axis, which the language
  design rule rejects outright.

### For-Each Loop

```vow
for x in vec {
    print_i64(x);
}
```

Iterates over each element of a `Vec<T>`. The loop variable `x` is bound to each element in turn. Desugars to a `while` loop with index arithmetic — zero verification overhead.

### For-Each Loop with Invariant

```vow
for x in vec vow {
    invariant: total >= 0
} {
    total = total + x;
}
```

### Loop (Infinite)

`loop` creates an infinite loop. The expression returns the type of the `break` value:

```vow
let idx: i64 = loop {
    if data[i] == target {
        break i as i64;
    }
    i = i + 1;
    if i >= n { break -1; }
};
```

ESBMC cannot verify unbounded `loop` constructs — use `while` with invariants for verifiable loops.

### Break

`break` exits the innermost loop. Inside `loop`, `break value` sets the loop's result:

```vow
break;           // exit while or loop (loop returns Unit)
break value;     // exit loop with a value (only inside loop, not while)
```

### Continue

`continue` skips the remaining statements in the current loop iteration and jumps back to the loop header:

```vow
continue;        // skip to next iteration of while, loop, or for
```

Inside `while` and `loop`, `continue` emits back-edge values for any mutated variables. Inside `for`, it also advances the loop index.

### Return

```vow
return;
return value;
```

## Struct Definitions

```vow
struct Point {
    x: i64,
    y: i64,
}
```

### Linear Structs

```vow
linear struct FileHandle {
    fd: i64,
}
```

Linear struct values carry a linear obligation. The obligation must either be consumed before the value's owning region closes or transferred to the caller by returning the value.

Owned enum wrappers inherit that obligation transitively. A user enum,
`Option<T>`, or `Result<T, E>` is linear when one of its owned payload paths is
linear; matching such a value consumes the wrapper exactly once and transfers
the obligation to the selected bound payload. A reference type (`&T`) is never a
linear owner. Collection types do not acquire linear ownership from
their element type, and they cannot hold linear values: a `Vec<T>` element, a
`HashMap<K, V>` value, or a `BTreeMap<K, V>` value that is or transitively
contains a linear owner (a `linear struct`, or an `Option`, `Result`, or user
enum wrapping one) is rejected where the collection type is written. The
containers copy and shift entries bitwise, so storing a linear value would
duplicate its obligation or let it escape the checker. `Vec` and `HashMap` use
`UnsupportedFeature`; `BTreeMap` uses `BTreeMapValueMustBeNonLinear`. A nested
collection (`Vec<Vec<Token>>`) is reported once, at the innermost collection that
holds the linear value, and a type alias is reported once, at its definition. A
`Vec` element test is about ownership: `Vec<&Token>` borrows and is accepted,
while a tuple that holds a linear owner is rejected.
A linear value that is no longer needed is discharged with the intrinsic
`drop(value)` (see [Linear Intrinsics](#linear-intrinsics)). Passing it to a
function that consumes it, returning it, or matching it are the other ways to
satisfy the obligation; there is no implicit end-of-scope discharge, so a live
obligation at scope exit is `RegionLinear`.
An unbound `_` match catchall cannot discard a still-reachable linear payload:
every variant that owns a linear payload must first have an explicit arm that
binds and consumes or transfers that payload.

Struct fields cannot own linear values, even when the containing struct is
`linear`, because field access does not provide move-out semantics. Allowing an
owned field would let repeated reads transfer the same obligation more than
once. Reference-typed (`&T`) and collection fields do not become linear owners
under this rule.

### Struct Literals

Struct literal names must be PascalCase:

```vow
let p: Point = Point { x: 1, y: 2 };
```

Because of that, an identifier that does not start with an upper-case letter is
never a struct literal: in `while c { }` and `if c { }` the `{` opens the body.

### Field Access

```vow
p.x
```

### Field Assignment

```vow
p.x = 10;
```

### Passing Semantics

Structs are heap-allocated. A struct value is a pointer to a heap region, so passing a struct to a function passes the pointer — the function operates on the same heap data, not a copy. Field assignments inside the called function are visible to the caller:

```vow
fn shift_right(p: Point, dx: i64) {
    p.x = p.x + dx;
}

fn main() -> i32 [io] {
    let p: Point = Point { x: 0, y: 0 };
    shift_right(p, 5);
    print_i64(p.x);  // 5 — mutation visible to caller
    0
}
```

This enables in-place mutation patterns (e.g., make/unmake in search trees) without cloning. The same aliasing semantics apply when structs are stored in containers — see [Indexing](#indexing). To avoid aliasing, construct a fresh struct literal with the desired field values.

**Note:** For linear owner types (a `linear struct` or an owned enum wrapper that
contains one), passing the value to a function consumes it; the caller cannot
access it afterward. Returning a linear value transfers the obligation to the
caller, so this is the normal way to hand an updated linear value back out of a
function.

## Enum Definitions

```vow
enum Shape {
    Circle(i64),
    Rect(i64, i64),
    Empty,
}
```

Variant kinds: unit (`Empty`), tuple (`Circle(i64)`), struct (`Named { x: i64 }`).

### Enum Construction

```vow
let s: Shape = Shape::Circle(5);
let none: Option<i64> = Option::None;
let some: Option<i64> = Option::Some(42);
```

### Built-in Enums

`Option<T>` has variants `Some(T)` and `None`.
`Result<T, E>` has variants `Ok(T)` and `Err(E)`.

## Pattern Matching

```vow
match value {
    Pattern1 => expr1,
    Pattern2 => expr2,
    _ => default_expr,
}
```

Match is an expression. The scrutinee must have an enum type, including an
applied built-in enum such as `Option<T>` or `Result<T, E>`. All arms must
return the same type. Patterns must be exhaustive.

### Pattern Kinds

| Implemented pattern                         | Example              |
|---------------------------------------------|----------------------|
| Wildcard                                    | `_`                  |
| Immutable identifier binding                | `value`              |
| Qualified enum variant (unit)               | `Option::None`       |
| Qualified enum variant (tuple payload)      | `Option::Some(value)` |

Tuple-variant patterns must provide exactly one payload binding for every
declared payload, and each binding may be only `_` or an immutable identifier.
The qualified enum name must match the scrutinee's enum; a variant from another
enum neither binds payloads nor counts toward exhaustiveness.
Nested payload destructuring is not implemented. A catchall `_` or immutable
identifier arm must be the final arm because it matches every enum value.
For an enum that can own linear payloads, `_` is allowed only after explicit
arms have handled every variant with a linear payload; otherwise the catchall
would silently discard an outstanding linear obligation.

Mutable identifier, literal (integer, boolean, or string), tuple, struct,
enum-struct, or-pattern, unqualified enum-variant, and nested payload patterns
are not implemented. Parsed unsupported forms produce
`error[UnsupportedPattern]`; forms that the parser cannot represent produce
`error[UnexpectedToken]`. Both are compile-time failures and no executable is
produced.

## Method Calls

```vow
v.push(42);
v.len()
s.byte_at(0)
m.contains_key(k)
```

**Collection constructors take their type from an annotation.** `Vec::new()`, `HashMap::new()`, and `BTreeMap::new()` carry no element, key, or value type of their own; it comes from the binding's annotation (`let m: HashMap<i64, i64> = HashMap::new();`). Calling a method on a collection whose type was never written (`let m = HashMap::new(); m.insert(1, 2);`) is a `TypeMismatch` in both compilers (`cannot infer the collection type of the receiver of ...`), one error per call, so a map can never silently default its key and value to `i64` and bypass the [key and value type](#hashmap-methods) checks.

### Vec<T> Methods

| Method         | Signature                        |
|----------------|----------------------------------|
| `Vec::new()`   | `() -> Vec<T>`                   |
| `Vec::from_raw_parts_copy(ptr, len)` | `(i64, u64) -> Vec<T>` for flat scalar `T` |
| `.push(val)`   | `(T) -> ()`                      |
| `.pop()`       | `() -> ()`                       |
| `.len()`       | `() -> u64`                      |
| `.clear()`     | `() -> ()` — frees buffer, resets to empty |
| `.truncate(n)` | `(u64) -> ()` — shrinks to n elements, frees excess memory |
| `v[i]`         | Index read, `i: u64` — copies slot value; aliases heap types (panics if out of bounds) |
| `v[i] = val`   | Index write, `i: u64` — copies value into slot |

`Vec` indices and `.truncate(n)` take exactly `u64`; an unsuffixed integer literal coerces, and any other integer type needs an explicit `as u64`. See [Indexing](#indexing).

`Vec` has no `.get(i)` method: element access is `v[i]` only, and `v.get(i)` is an `UnknownMethod` error in both compilers.

### String Methods

| Method              | Signature                   |
|---------------------|-----------------------------|
| `String::from(s)`   | `(String) -> String` — mutable copy |
| `String::new()`     | `() -> String`              |
| `String::from_raw_parts_copy(ptr, len)` | `(i64, u64) -> String` |
| `.len()`            | `() -> u64`                 |
| `.byte_at(i)`       | `(u64) -> i64` — the byte at offset `i` in `0..=255`, or `-1` when `i >= len()` |
| `.push_byte(b)`     | `(u8) -> ()` — appends one byte |
| `.push_str(s)`      | `(String) -> ()`            |
| `.clear()`          | `() -> ()` — frees buffer, resets to empty |
| `.contains(s)`      | `(String) -> bool`          |
| `.eq(s)`            | `(String) -> bool`          |
| `.substring(start, end)` | `(u64, u64) -> String` — bytes `[start, end)`, both bounds clamped to `len()` |
| `.parse_i64()`      | `() -> Option<i64>`         |
| `.parse_u64()`      | `() -> Option<u64>`         |

`String` offsets are exactly `u64` and `push_byte`'s argument is exactly `u8`; see [String offsets](#string-offsets).

`push_byte`'s argument is a byte *value*, not an index. It is typed `u8`, so the byte range `0..=255` is a fact of the type: an unsuffixed literal outside that range is a `LiteralOutOfRange` error, and any other integer type (`i64`, `u64`, ...) is a `TypeMismatch`. A wider value is narrowed explicitly with a narrowing intrinsic (`i64_to_u8_wrap`, `u64_to_u8_wrap`, `i64_to_u8_sat`, `i64_to_u8_try`, ...) — `as u8` from a wider type is `NarrowingCastNotAllowed`. There is no silent truncation: `s.push_byte(300)` is rejected, and the explicit `s.push_byte(i64_to_u8_wrap(300))` appends `44`. `byte_at` returns an `i64` in `-1..=255`, where `-1` means the index is past the end: copy a whole range with `substring` or `push_str`, and when transforming byte by byte make `i < s.len()` a precondition, because wrapping the `-1` sentinel would append `0xFF`.

### HashMap<K, V> Methods

| Method              | Signature                   |
|---------------------|-----------------------------|
| `HashMap::new()`    | `() -> HashMap<K, V>`       |
| `.insert(k, v)`     | `(K, V) -> ()`              |
| `.get(k)`           | `(K) -> Option<V>`                                                       |
| `.contains_key(k)`  | `(K) -> bool`               |
| `.remove(k)`        | `(K) -> ()`                 |
| `.len()`            | `() -> u64`                 |

**Key and value types.** The runtime stores each key and each value in one 64-bit slot and compares keys by value. A `HashMap` key must therefore be `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`, or `bool`; every other key type is an `UnsupportedFeature` error in both compilers. `String`, `Vec`, struct, enum, `Option`, and tuple keys are heap-backed handles that would compare by pointer, so a lookup with an equal-but-distinct `String` would silently miss (and a mutable `String` mutated after insertion would corrupt the map). `i128`/`u128` keys would be truncated, and `f32`/`f64` have no total equality. Hash or intern such keys to a `u64` at the call site and keep a side table for the originals. A `HashMap` or `BTreeMap` value of type `i128`, `u128`, `f32`, or `f64` is likewise an `UnsupportedFeature` error: map values occupy a single 64-bit integer slot, so a 128-bit value would lose its high word and a float has no slot encoding. A `HashMap` value that is or transitively contains a `linear struct` is an `UnsupportedFeature` error for the same reason `BTreeMap` rejects it (`BTreeMapValueMustBeNonLinear`): the map copies values bitwise and `get` would hand out a second copy of the linear obligation. Narrow integer values (`i8` … `u32`) are stored widened and read back at their declared width. The check applies wherever the map type is written (annotations, parameters, returns, fields, aliases, constants), including nested inside `Vec`, `Option`, tuples, and other maps. A 128-bit integer nested inside an aggregate value (`Option<u128>`, a struct field) is not a map restriction: no aggregate can hold a 128-bit field yet (epic #526), so codegen rejects it with `CodegenUnsupported` wherever it appears.

**Set idiom.** The unit type `()` is a valid map value, so `HashMap<K, ()>` and `BTreeMap<K, ()>` are sets: `s.insert(k, ());` adds a member, `s.contains_key(k)` (`s.contains(k)` for `BTreeMap`) tests membership, `s.remove(k)` deletes it, and `s.get(k)` returns `Option<()>`. The value `()` has type `()` (it checks against a `()` annotation or return type), and the runtime stores it in the usual 64-bit slot as `0`. A function cannot take a `()` parameter (`UnsupportedFeature`: the argument carries no information and has no ABI slot), so pass the set itself or a key instead.

`HashMap::get` returns `Option<V>`, exactly like `BTreeMap::get`: a missing key is `None`, never a default value, so `let a: i64 = m.get(k);` is a `TypeMismatch` in both compilers. Handle both cases with `match` (or `?`), or call `.unwrap()` to assert the key is present: it aborts with `UnwrapOnNone` on a missing key and requires the `[panic]` effect. A contract can state a binding as `result.get(k).unwrap() == v`; guard it with an earlier `result.contains_key(k)` clause (as in the examples), because the verifier reports a missing key there as a failed `unwrap()` on `None`, which carries no contract blame.

### BTreeMap<K, V> Methods

Keys must be `i64` (K violations raise `BTreeMapKeyTypeMustBeI64`). Values may be any
non-linear type other than `i128`/`u128`/`f32`/`f64` (rejected with `UnsupportedFeature`, because a
value occupies a single 64-bit integer slot) — integers, `bool`, structs, `Vec<T>`, `Option<T>`, or nested combinations.
A `V` that is or transitively contains a `linear struct` is rejected with
`BTreeMapValueMustBeNonLinear`, because the runtime/verifier shift values bitwise and
would silently duplicate a linear obligation.
Storage is two parallel sorted arrays (binary-search lookup, sorted-insert writes).
Iteration order is ascending by key and is **deterministic across runs and compilers** —
prefer `BTreeMap` over `HashMap` for any map whose iteration affects compiler output.

| Method              | Signature                   |
|---------------------|-----------------------------|
| `BTreeMap::new()`   | `() -> BTreeMap<K, V>`      |
| `.insert(k, v)`     | `(K, V) -> Option<V>` (returns the previous value bound to `k`, if any) |
| `.get(k)`           | `(K) -> Option<V>` (returns the value bound to `k`, or `None`)          |
| `.contains(k)`      | `(K) -> bool`               |
| `.len()`            | `() -> u64`                 |

### Option<T> Methods

| Method      | Signature                              |
|-------------|----------------------------------------|
| `.unwrap()` | `() -> T` (takes no arguments; aborts with `UnwrapOnNone` on `None`; requires `[panic]` effect) |

`.unwrap()` is also available on `Result<T, E>`, where it returns the `Ok` payload
and aborts with the same `UnwrapOnNone` diagnostic on `Err`. The abort is emitted in
every build mode — `.unwrap()` is a partial operation, not a vow check.

The `?` operator on `Option<T>` or `Result<T, E>` propagates `None`/`Err` to the caller (the calling function must return `Option` or `Result`).

## Indexing

```vow
let val: i64 = v[0];
v[i] = new_val;
```

The index expression of a `Vec` read or write must have **exactly the type `u64`**. An unsuffixed integer literal coerces to `u64` (`v[0]` needs no suffix); a literal that does not fit, such as `v[-1]` or `v[18446744073709551616]`, is a `LiteralOutOfRange` error. Any other integer type (`i8` … `i128`, `u8` … `u32`, `u128`) and any non-integer index is a `TypeMismatch` error, in both compilers; widen or convert explicitly with `as` (`v[i as u64]`). The same rule applies to the index-shaped `Vec` method argument of `Vec::truncate`, which takes exactly `u64`.

The `Vec` runtime helpers take a pointer-width unsigned index, so a `u64` index is never reinterpreted as negative: an index at or beyond `v.len()` is out of bounds, including values above `i64::MAX`.

Indexing `v[i]` is defined only for `Vec<T>`. A `HashMap`, `BTreeMap`, `String`, `Option`, or any other type has no index operator: `m[k]` is a `TypeMismatch` ("index operation on non-indexable type") in both compilers, for reads and for assignments. Read a map entry with `m.get(k)`, which returns an `Option<V>` so a missing key is never a default value or a runtime trap, and write one with `insert`. Read a `String` byte with `byte_at`.

Lengths are `u64` (see [the Vec method table](#vec-methods)), so an index derived from one needs no conversion:

```vow
let n: u64 = v.len();
let mut i: u64 = 0;
while i < n vow { invariant: i <= n } {
    let x: i64 = v[i];
    i = i + 1;
}
```

Arithmetic and comparison do not mix signedness: `i64 + u64` and `u64 < i64` are `TypeMismatch`. Convert at the binding with `as`; same-width `as` casts between `i64` and `u64` are legal. A signed value that is already known to be a valid position is converted at the index site (`v[k as u64]`); a negative `k` becomes a huge `u64` and fails the bounds check.

### String offsets

`String` offsets are `u64`, exactly like `Vec` indices: a position or length into a `String` is never negative, and `String::len()` is already `u64`. Every offset or length argument has exactly the type `u64`:

- `s.byte_at(i)` — `i`
- `s.substring(start, end)` — `start` and `end`
- `string_substr(s, start, len)` — `start` and `len`
- `string_matches_literal_at(s, pos, literal)` — `pos`

An unsuffixed literal coerces to `u64` and is range-checked against it (`LiteralOutOfRange` when it does not fit); any other integer type, including `i64`, is a `TypeMismatch`. Convert at the binding with `as u64` (same-width casts between `i64` and `u64` are legal); a negative signed value cast to `u64` becomes a huge offset, which is out of range rather than silently clamped to the start. A 128-bit offset is a `TypeMismatch`, not a codegen question.

Because an offset cannot be negative, the runtime has no negative-offset behaviour. Offsets past the end of the string behave as follows, and this is the whole specification:

- `byte_at` returns `-1` when `i >= len()`. The `-1` is the out-of-range sentinel of the `-1..=255` result; it is not a reachable byte value.
- `substring(start, end)` clamps `start` to `len()` and then `end` into `[start, len()]`, so a reversed or oversized range yields the empty string or the tail, never a panic.
- `string_substr(s, start, len)` clamps `start` to `len()` and `len` to the bytes remaining after `start`.
- `string_matches_literal_at` returns `0` when `pos` plus the literal's byte length exceeds `len()` (including when that sum overflows `u64`).

The verifier is stricter than the runtime for `byte_at`: an index that is not provably `< len()` fails verification as `index out of bounds`, because reaching the `-1` sentinel is almost always an agent bug. `substring`, `string_substr` and `string_matches_literal_at` are modelled with exactly the clamping above on unsigned values. A length contract on the result, such as `ensures: result.len() <= s.len()`, proves.

`byte_at` returns a byte *value* in `-1..=255`, not a position, so it stays `i64`. `push_byte` takes a byte *value*, not an offset, and is `u8` (see the String method table).

Indexing uses **copy semantics**: `v[i]` copies the 8-byte slot value and `v[i] = val` copies a value into the slot. The base container is not consumed.

For primitive types (`i64`, `bool`), this is a genuine value copy — the result is independent of the container. For heap types (`Vec<T>`, `String`, structs, enums), the 8-byte slot holds a pointer, so indexing copies the pointer, creating an **alias**. Both the container slot and the local variable point to the same heap data:

```vow
let buckets: Vec<Vec<i64>> = Vec::new();
buckets.push(Vec::new());
let b: Vec<i64> = buckets[0];  // b aliases buckets[0]
b.push(42);                     // visible through buckets[0]
```

This aliasing is the intended behavior for arena and hash-table patterns where bucket contents are read and mutated repeatedly through index access.

## Extern Blocks

Declare external C functions (a `vow` contract block is required):

```vow
extern "C" {
    vow {
        requires: fd >= 0
        ensures: result >= 0
    }
    fn write_thing(fd: i32, ptr: i64, len: i64) -> i64 [io];
}
```

Omitting the `vow` block produces a `MissingContract` error (see [errors.md](errors.md)).

## Type Aliases

```vow
type Score = i64
```

## Effect System

Effects are explicit. Every function declares which side effects it may perform. Pure functions (no effects) need no annotation.

### Effect Types

| Effect   | Meaning                              |
|----------|--------------------------------------|
| `io`     | Standard I/O (print, stdin, network) |
| `read`   | File system reads                    |
| `write`  | File system writes                   |
| `panic`  | May panic (unwrap, etc.)             |
| `unsafe` | Unsafe operations (FFI, raw memory)  |

Each effect is independent — `io` is not a superset of `read` or `write`.

### Propagation

A function must declare every effect that any function it calls may produce:

```vow
fn do_io() -> () [io] {
    print_str("hi");
}

fn caller() -> () [io] {
    do_io();
}
```

If `caller` omitted `[io]`, the type checker would emit `EffectViolation`.

### Contract Purity

Contract expressions (`requires`, `ensures`, `invariant`) must be pure — they cannot call effectful functions.

They also cannot write through any argument, even when the write happens inside a helper that declares no effect. Declared effects (`read`, `write`, `io`, `panic`, `unsafe`) cover filesystem/stdio/panic/FFI only — a plain struct-field assignment, a `Vec`/map element write, or a mutating builtin method call (`push`, `insert`, `clear`, …) through a parameter is invisible to that check, since Vow passes structs, `Vec`, `String`, and maps by pointer. For example:

```vow
fn mark(p: Point) -> bool {
    p.x = 1;
    true
}

fn make_point(x: i64, y: i64) -> Point
vow {
    ensures: mark(result)
}
{
    Point { x: x, y: y }
}
```

`mark` declares no effect, so the declared-effect check alone would accept `ensures: mark(result)`. But `mark` writes `result.x` as a side effect of being evaluated for the check itself — a write that is not part of `make_point`'s own body and that a caller relying on `result.x == 1` would never see if contract evaluation were ever skipped. This is rejected with `EffectViolation`, the same diagnostic the declared-effect check uses, blamed on the callee. The check is transitive: a helper that only writes through another helper it calls is rejected too, and the search for a reachable write looks through — not into — control flow (`if`/`match`/loops inside a clause are still searched for writes, not forbidden outright). Builtin read-only methods (`len`, `get`, `contains`, `contains_key`, `eq`, `byte_at`, `substring`, `parse_i64`, `parse_u64`, `unwrap`) are unaffected; an unresolvable callee is rejected (fails closed) rather than silently assumed pure.

### Builtin Function Signatures

#### FFI Wrapper Intrinsics

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `pin_to_root`    | `fn(value: String) -> String` and `fn<T>(value: Vec<T>) -> Vec<T>` for flat scalar `T` | `[]` |

`pin_to_root` is a compiler intrinsic, not a user-defined generic. Each call site is monomorphised from the argument type. It always deep-copies the supported heap value into root storage; it does not inspect descriptor tags and does not claim idempotency. The current supported forms are `String` and `Vec<T>` where `T` is a flat scalar slot type (`i*`, `u*`, `f32`, `f64`, `bool`). Pointer-containing payloads, user structs, enums, and maps require hand-written deep-copy wrappers at the FFI boundary.

#### Linear Intrinsics

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `drop`           | `fn(value: L) -> ()` for a linear owner `L` | `[]`       |

`drop` is a compiler intrinsic, not a user-defined generic. `L` must be a linear owner: a `linear struct`, or an owned enum wrapper (`Option`, `Result`, or a user enum) that contains one. Any other argument type, or an argument count other than one, is a `TypeMismatch`. `drop` consumes the value exactly once (a second use is `LinearTypeViolation`) and discharges its obligation. It has no runtime effect beyond that: it runs no destructor, frees nothing, and lowers to no instruction other than the consume marker the type and region passes already track. It is verifier-neutral: the consume marker is a no-op in the C model, so a function that drops a linear value is verified exactly as if the call were absent. A user-defined function named `drop` takes precedence over the intrinsic.

`String::from_raw_parts_copy(ptr: i64, len: u64)` copies `len` bytes from a raw C pointer into a fresh `String`. `Vec::from_raw_parts_copy(ptr: i64, len: u64)` copies `len` flat scalar slots into a fresh `Vec<T>`. The pointer is `i64` and the length is `u64`, so a signed length must be converted explicitly (`n as u64`); the code generator converts pointer and length values to the platform pointer-sized ABI type at the FFI boundary. Both helpers have a `FreshInCaller` return summary.

For pointer-containing C payloads, a wrapper must be written per type: call the extern, recursively copy every Vow-owned heap subobject into the target region, free every C-owned pointer according to the extern's ownership contract, then return the Vow-placed value. A bytewise copy of a pointer-containing payload is unsound because it preserves stale pointers into C-owned storage.

#### Print / IO

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `print_str`      | `fn(s: String) -> ()`                      | `[io]`     |
| `print_i64`      | `fn(v: i64) -> ()`                         | `[io]`     |
| `print_u64`      | `fn(v: u64) -> ()`                         | `[io]`     |
| `eprintln_str`   | `fn(s: String) -> ()`                      | `[io]`     |

#### Debug

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `debug_str`      | `fn(s: String) -> ()`                      | `[]`       |
| `debug_i64`      | `fn(v: i64) -> ()`                         | `[]`       |
| `debug_u64`      | `fn(v: u64) -> ()`                         | `[]`       |

**Debug print semantics:** Debug prints are effect-free and callable from pure functions. In debug and sanitize modes (`--mode debug`, `--mode sanitize`), they write to stderr. In release and profile modes, the debug call itself is not emitted — no function call occurs. However, argument expressions are still evaluated (a direct literal such as `"label"` is static, while `String::from("label")` still allocates a mutable copy). They are also no-ops during verification. Use them to trace values inside pure kernel code without restructuring the effect hierarchy.

#### Filesystem

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `fs_read`        | `fn(path: String) -> String`               | `[read]`   |
| `fs_open`        | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_read_line`   | `fn(handle: i64) -> String`                | `[read]`   |
| `fs_status`      | `fn(handle: i64) -> i64`                   | `[read]`   |
| `fs_close`       | `fn(handle: i64) -> i64`                   | `[read]`   |
| `fs_write`       | `fn(path: String, data: String) -> i64`    | `[write]`  |
| `fs_exists`      | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_mkdir`       | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_listdir`     | `fn(path: String) -> Vec<String>`          | `[read]`   |
| `fs_remove`      | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_remove_dir`  | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_is_dir`      | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_is_symlink`  | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_rename`      | `fn(old: String, new: String) -> i64`      | `[io]`     |

#### String Operations

| Function              | Signature                                        | Effects |
|-----------------------|--------------------------------------------------|---------|
| `string_substr`       | `fn(s: String, start: u64, len: u64) -> String`  | `[]`    |
| `string_split`        | `fn(s: String, delim: String) -> Vec<String>`    | `[]`    |
| `string_starts_with`  | `fn(s: String, prefix: String) -> i64`           | `[]`    |
| `string_ends_with`    | `fn(s: String, suffix: String) -> i64`           | `[]`    |
| `string_matches_literal_at` | `fn(s: String, pos: u64, literal: String literal) -> i64` | `[]` |
| `string_trim`         | `fn(s: String) -> String`                        | `[]`    |
| `string_to_upper`     | `fn(s: String) -> String`                        | `[]`    |
| `string_to_lower`     | `fn(s: String) -> String`                        | `[]`    |
| `string_replace`      | `fn(s: String, from: String, to: String) -> String` | `[]` |
| `string_join`         | `fn(parts: Vec<String>, sep: String) -> String`  | `[]`    |

#### Conversion

**Formatting** uses two baselines; widen via `as` for narrower types:

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `int_to_string`  | `fn(v: i64) -> String`                     | `[]`       |
| `uint_to_string` | `fn(v: u64) -> String`                     | `[]`       |
| `i64_to_string`  | `fn(v: i64) -> String` (alias of `int_to_string`) | `[]` |

```vow
let small: u8 = 42;
print_str(uint_to_string(small as u64));  // widen then format
```

**Parsing** exposes a try-form for every integer width:

| Function       | Signature                                |
|----------------|------------------------------------------|
| `parse_i8`     | `fn(s: String) -> Option<i8>`            |
| `parse_i16`    | `fn(s: String) -> Option<i16>`           |
| `parse_i32`    | `fn(s: String) -> Option<i32>`           |
| `parse_i64`    | `fn(s: String) -> Option<i64>` (also see `String.parse_i64()`) |
| `parse_i128`   | `fn(s: String) -> Option<i128>`          |
| `parse_u8`     | `fn(s: String) -> Option<u8>`            |
| `parse_u16`    | `fn(s: String) -> Option<u16>`           |
| `parse_u32`    | `fn(s: String) -> Option<u32>`           |
| `parse_u64`    | `fn(s: String) -> Option<u64>` (also see `String.parse_u64()`) |
| `parse_u128`   | `fn(s: String) -> Option<u128>`          |

Each `parse_X` returns `Option::None` for malformed input, empty strings, or
values outside the target type's range. Parsing never substitutes a numeric
sentinel for failure; callers that need a fallback must choose it explicitly
when handling `Option::None`.

In particular, `parse_i8`, `parse_i16`, `parse_u8`, `parse_u16`, `parse_i32`,
and `parse_u32` enforce their exact signed or unsigned fixed-width ranges.

**Narrowing intrinsics** (per [Type Cast](#type-cast)): for every narrowing
pair the compiler emits `<src>_to_<tgt>_try`, `<src>_to_<tgt>_wrap`, and
`<src>_to_<tgt>_sat` free functions with the semantics described in that
section.

#### Collections

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `vec_sort`       | `fn(v: Vec<i64>) -> Vec<i64>`              | `[]`       |

#### Time

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `time_unix`      | `fn() -> i64`                              | `[io]`     |
| `time_unix_ms`   | `fn() -> i64`                              | `[io]`     |

#### System

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `num_cpus`       | `fn() -> i64`                              | `[io]`     |
| `memory_root_arena_bytes` | `fn() -> u64`                    | `[io]`     |
| `memory_peak_bytes` | `fn() -> u64`                           | `[io]`     |
| `memory_alloc_count_since_start` | `fn() -> u64`              | `[io]`     |

`num_cpus()` returns the number of available logical CPUs (from `std::thread::available_parallelism`), or `1` if the query fails. Used to size worker pools (e.g. the default `--verify-jobs` value).

`memory_root_arena_bytes()` returns the current bytes retained by root-region
arena chunks. It is a gauge, not a monotone counter: adding a root chunk raises
it, while reclaiming an abandoned single-resident oversized root chunk during
backing growth lowers it. `memory_peak_bytes()` returns the peak live bytes
retained by all open arena chunks since process start.
`memory_alloc_count_since_start()` returns the number of successful Vow arena
allocation requests since process start. Peak bytes and allocation count are
monotone non-decreasing and saturate at `u64::MAX` rather than wrapping. These
queries do not allocate; they are effectful because they observe runtime process
state.

#### Encoding

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `hex_encode`     | `fn(data: Vec<u8>) -> String`              | `[]`       |
| `hex_decode`     | `fn(s: String) -> Vec<u8>`                 | `[]`       |

#### Input

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `args`           | `fn() -> Vec<String>`                      | `[read]`   |
| `stdin_read`     | `fn() -> String`                           | `[read]`   |
| `stdin_read_line`| `fn() -> String`                           | `[read]`   |
| `stdin_ready`    | `fn() -> bool`                             | `[read]`   |

#### Process Management

| Function              | Signature                                        | Effects |
|-----------------------|--------------------------------------------------|---------|
| `process_exit`        | `fn(code: i64) -> !`                             | `[io]`  |
| `process_run`         | `fn(cmd: String, args: Vec<String>) -> i64`      | `[io]`  |
| `process_get_stdout`  | `fn() -> String`                                 | `[io]`  |
| `process_get_stderr`  | `fn() -> String`                                 | `[io]`  |
| `process_start`       | `fn(cmd: String, args: Vec<String>) -> i64`      | `[io]`  |
| `process_wait`        | `fn(pid: i64) -> i64`                            | `[io]`  |
| `process_wait_timeout`| `fn(pid: i64, timeout_ms: i64) -> i64`           | `[io]`  |
| `process_poll_wait`   | `fn(pid: i64, timeout_ms: i64) -> i64`           | `[io]`  |
| `process_kill`        | `fn(pid: i64) -> i64`                             | `[io]`  |
| `process_stdout_for`  | `fn(pid: i64) -> String`                         | `[io]`  |
| `process_stderr_for`  | `fn(pid: i64) -> String`                         | `[io]`  |

**`args` semantics:** `args()` returns all process arguments including the program name at index 0 (matching C `argv` and Rust `std::env::args()` conventions). For `./my_program foo bar`, `args()` returns `["./my_program", "foo", "bar"]`. Use `args[1]` onward for user-supplied arguments. The Vec is empty only if the OS provides no arguments (unusual). Returns an empty String element if an argument is empty (`""`). Non-UTF-8 arguments are included as-is (byte content preserved).

**`fs_read` semantics:** `fs_read(path)` opens the file at `path`, reads its entire contents, and returns a String. Returns `""` (empty String) on any error (file not found, permission denied, I/O error, non-UTF-8 path). Does not block on regular files. Callers should check `result.len() == 0` to detect failure.

**Streaming file input:** `fs_open(path)` opens a file for incremental reading and returns a positive handle, or `-1` on path/open error. `fs_read_line(handle)` reads one line from the current cursor and returns it as a String, including the trailing newline when present. It returns `""` at EOF, for an invalid handle, or after a read error. A blank line is returned as `"\n"`, so newline-delimited callers can distinguish a real blank line from EOF by content. After `fs_read_line(handle)` returns `""`, call `fs_status(handle)` to distinguish EOF from error: `0` means the handle is open with no EOF/error state, `1` means EOF, and `-1` means invalid handle or read error. `fs_status(handle)` reports the result of the most recent `fs_read_line(handle)` call on that open handle; read it immediately after a `""` return because later reads may update it. `fs_close(handle)` releases the handle and returns `0` on success or `-1` for an invalid/already-closed handle. Long-running programs must close handles they no longer need. All streaming handle operations use the `[read]` effect, including `fs_close`, because closing a read handle releases read-stream state and does not mutate filesystem contents. The current runtime stores streaming handles in one process-global table, and `fs_read_line` holds that table lock while it reads the next line. This keeps the API simple for single-stream file processing, but it is not intended for latency-sensitive concurrent reads from multiple slow handles.

**Filesystem return values:** `fs_write`, `fs_mkdir`, `fs_remove`, `fs_remove_dir`, and `fs_rename` return `i64`: 0 on success, non-zero on failure. `fs_open`, `fs_status`, and `fs_close` use the streaming status codes above. `fs_exists`, `fs_is_dir`, and `fs_is_symlink` are predicates: they return 1 for true, 0 for false. Errors (null pointer, invalid UTF-8) also return 0, so callers cannot distinguish "false" from "error". `fs_is_symlink` uses `lstat`-equivalent semantics: a symlink reports 1 even when its target is a regular file or directory.

**`string_starts_with` / `string_ends_with` / `string_matches_literal_at` return values:** Return `i64`: 1 if true, 0 if false.

**`string_matches_literal_at` literal operand:** The third argument must be written as a string literal at the call site. The compiler lowers that literal to static bytes plus an explicit byte length, so no temporary `String` allocation is created and embedded NUL bytes are preserved. Passing a variable or computed `String` as the third argument is a type-check error (`StaticLiteralRequired`). Use `string_starts_with`, `string_ends_with`, or `String` methods when the needle must be dynamic.

**`process_run` vs `process_start`:** `process_run(cmd, args)` runs a subprocess synchronously and returns its exit code. After it returns, `process_get_stdout()` and `process_get_stderr()` retrieve the captured output of the most recent `process_run` call. `process_start(cmd, args)` launches a subprocess asynchronously and returns a process ID. Use `process_wait(pid)` to wait for completion and get the exit code, and `process_stdout_for(pid)` / `process_stderr_for(pid)` to retrieve output.

**`process_wait_timeout`:** `process_wait_timeout(pid, timeout_ms)` polls a process started with `process_start` until it exits or the timeout (in milliseconds) elapses. Returns the exit code on completion, `-1` on error, or `-2` on timeout. After a timeout, the process is still running; use `process_kill(pid)` to terminate it.

**`process_poll_wait`:** `process_poll_wait(pid, timeout_ms)` waits up to `timeout_ms` milliseconds for a process started with `process_start` to exit, without killing it on timeout. Returns the exit code on completion, `-1` on an unknown process ID, or a large negative sentinel if the process is still running after the timeout (left alive, not killed). Unlike `process_wait_timeout`, callers can re-poll and impose their own watchdog deadline instead of the process being killed automatically.

**`process_kill`:** `process_kill(pid)` sends a kill signal to a running process and waits for it to exit. Returns 0 on success, -1 on error. No-op (returns 0) if the process has already completed.

**`stdin_read` vs `stdin_read_line`:** `stdin_read()` reads the entire stdin stream into a single String (unbounded memory). `stdin_read_line()` reads one line at a time, including the trailing newline. Returns `""` (empty string) at EOF. The returned String is runtime scratch storage valid until the next `stdin_read_line()` call. Process each line before reading the next one for bounded memory; use `pin_to_root(line)` before the next read when a line must be stored, returned, passed to a function that may store it, mutated, or otherwise retained. The direct scratch line is read-only. The scratch buffer keeps the largest line capacity seen so far, so memory is bounded by maximum line length rather than total input, but one very large line can retain that capacity for the process lifetime.

```vow
let lines: Vec<String> = Vec::new();
let mut line: String = stdin_read_line();
while str_len(line) > 0 {
    // Without pin_to_root, lines.push(line) would store the scratch alias, not a copy.
    lines.push(pin_to_root(line));
    line = stdin_read_line();
}
```

```vow
let mut line: String = stdin_read_line();
while str_len(line) > 0 {
    // process line (has trailing \n)
    line = stdin_read_line();
}
```

**`stdin_ready`:** `stdin_ready()` returns `true` if `stdin_read_line()` would return immediately without blocking, `false` otherwise. Uses a non-blocking poll with zero timeout. Use this in computation loops that must remain responsive to external input:

```vow
while !stdin_ready() && depth < max_depth {
    // continue searching
    depth = depth + 1;
}
if stdin_ready() {
    let cmd: String = stdin_read_line();
    // handle command
}
```

## Canonical Form

The canonical printer normalizes source: `parse → print → parse` is idempotent. Effects are sorted alphabetically, indentation uses 4 spaces, trailing expressions omit semicolons.
