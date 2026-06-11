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

This resolves to `<rootdir>/foo/bar.vow` relative to the main source file.

## Const Declarations

Named constants with compile-time values:

```vow
const MAX_SIZE: i64 = 1024;
const NEG_ONE: i64 = -1;
const DEBUG: bool = true;
```

Supported value forms: integer literals, boolean literals, negated integer literals. Constants are inlined at every use site (zero runtime cost). The type must be any of the 10 integer types (`i8`, `i16`, `i32`, `i64`, `i128`, `u8`, `u16`, `u32`, `u64`, `u128`) or `bool`. Integer constants are subject to the same compile-time range check as integer literals. Constants are referenced by name in expressions like any other identifier.

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

`where` constraints on parameters become additional `requires` in verification. Each `where` clause can only reference its own parameter — it cannot reference other parameters.

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
| `()`   | Unit type                |
| `!`    | Never type (diverges)    |

There is no `isize`/`usize`. Vow targets 64-bit only; `Vec::len()` returns `i64`,
indices are `i64`. This is deliberate — it preserves binary fixed point
reproducibility across compilations. See [ADR 0001](../adr/0001-numeric-tower-narrow-ints.md).

**128-bit verification:** `i128`/`u128` arithmetic codegens via Cranelift's
`I128` and verifies via ESBMC's `__int128`. Predicates over 128-bit values may
exceed reasonable SMT solver timeouts; the `--no-128-verify` flag skips
verification for functions whose contracts mention 128-bit values while still
generating native code for them.

**Struct field layout:** every struct field is an 8-byte slot regardless of
declared type. Narrow ints stored in struct fields are padded. There is no
packing or natural-alignment layout today; FFI structs that need a specific C
layout must shim through `Vec<u8>` or extern wrappers.

### Built-in Parameterized Types

| Type               | Description                     |
|--------------------|---------------------------------|
| `Vec<T>`           | Growable array                  |
| `Option<T>`        | Optional value (Some/None)      |
| `Result<T, E>`     | Success or error                |
| `String`           | UTF-8 string (backed by Vec<u8>)|
| `HashMap<K, V>`    | Key-value map (linear scan)     |
| `BTreeMap<K, V>`   | Sorted key-value map (binary search; ascending iteration). Phase 1: `K = V = i64` only |

### User-Defined Types

Structs and enums (see below).

## Literals

### Integer Literals

```vow
42
-1
0
```

Unsuffixed integer literals default to `i64` in expression position, and
**context-coerce** to a narrower or unsigned annotated type when the
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

### Float Literals

```vow
3.14
-0.5
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

Wrapping operators silently wrap on overflow. For `u64` operands, division and remainder use unsigned semantics.

### Checked Arithmetic

| Operator | Meaning           |
|----------|-------------------|
| `+!`     | Add (checked)     |
| `-!`     | Sub (checked)     |
| `*!`     | Mul (checked)     |
| `/!`     | Div (checked)     |
| `%!`     | Rem (checked)     |

Checked operators abort with `ArithmeticOverflow` on overflow.

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

**Shift count type.** The right operand of `<<` and `>>` is `u32`. Unsuffixed
integer literals on the right side context-coerce to `u32`: given
`let x: u8 = ...`, `x << 3` is well-typed (`3` coerces to `u32`). The left
operand keeps its own integer type; the shift result has the left operand's
type.

**Shift count range.** A const-expression shift count `>= bit-width(LHS)` is a
compile-time error (`ShiftCountOutOfRange`). For example, `(x: u8) << 8` does
not compile. Dynamic shift counts (`x << n` where `n` is not a const
expression) get a contract on the operation that ESBMC checks: the count must
be less than the LHS width at the point of the shift.

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

Unary `-`, `!`, `&`, and `?` bind tighter than every binary operator.

Single `&` is overloaded by position: prefix `&expr` is borrow, while infix `lhs & rhs` is bitwise AND.

### Unary Operators

| Operator | Meaning    |
|----------|------------|
| `-`      | Negation (not allowed on `u64`) |
| `!`      | Logical NOT|
| `&`      | Borrow     |
| `?`      | Unwrap (propagate error) |

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

No implicit conversions: `i64 + u64` and `u8 + i32` are type errors. The
operands must already have the same type. The compiler does not coerce
across integer types at operator sites — only literals coerce, per the
[Integer Literals](#integer-literals) rules.

## Let Bindings

### Immutable

```vow
let x: i64 = 42;
```

### Mutable

```vow
let mut i: i64 = 0;
i = i + 1;
```

### Pattern Destructuring

```vow
let (a, b): (i64, i64) = (1, 2);
```

## Control Flow

### If / Else

```vow
if x > 0 {
    x
} else {
    0 - x
}
```

`if`/`else` is an expression — both branches must have the same type. There is no `else if` keyword; nest `if` inside `else`:

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
while i < n vow {
    invariant: i >= 0,
    invariant: i <= n
} {
    v.push(i);
    i = i + 1;
}
```

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
        break i;
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

### Struct Literals

Struct literal names must be PascalCase:

```vow
let p: Point = Point { x: 1, y: 2 };
```

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

**Note:** For `linear struct` types, passing the value to a function consumes it; the caller cannot access it afterward. Returning a linear value transfers the obligation to the caller, so this is the normal way to hand an updated linear value back out of a function.

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

Match is an expression. All arms must return the same type. Patterns must be exhaustive.

### Pattern Kinds

| Pattern                      | Example                          |
|------------------------------|----------------------------------|
| Wildcard                     | `_`                              |
| Identifier binding           | `x`                              |
| Mutable identifier           | `mut x`                          |
| Literal                      | `0`, `true`, `"hello"`           |
| Tuple                        | `(a, b)`                         |
| Enum variant (unit)          | `Option::None`                   |
| Enum variant (tuple)         | `Option::Some(x)`                |
| Enum variant (struct)        | `Shape::Named { x, y }`         |
| Or pattern                   | `0 \| 1 \| 2`                   |
| Struct pattern               | `Point { x, y }`                |

## Method Calls

```vow
v.push(42);
v.len()
s.byte_at(0)
m.contains_key(k)
```

### Vec<T> Methods

| Method         | Signature                        |
|----------------|----------------------------------|
| `Vec::new()`   | `() -> Vec<T>`                   |
| `Vec::from_raw_parts_copy(ptr, len)` | `(i64, i64) -> Vec<T>` for flat scalar `T` |
| `.push(val)`   | `(T) -> ()`                      |
| `.pop()`       | `() -> ()`                       |
| `.len()`       | `() -> i64`                      |
| `.clear()`     | `() -> ()` — frees buffer, resets to empty |
| `.truncate(n)` | `(i64) -> ()` — shrinks to n elements, frees excess memory |
| `v[i]`         | Index read — copies slot value; aliases heap types (panics if out of bounds) |
| `v[i] = val`   | Index write — copies value into slot |

### String Methods

| Method              | Signature                   |
|---------------------|-----------------------------|
| `String::from(s)`   | `(String) -> String` — mutable copy |
| `String::new()`     | `() -> String`              |
| `String::from_raw_parts_copy(ptr, len)` | `(i64, i64) -> String` |
| `.len()`            | `() -> i64`                 |
| `.byte_at(i)`       | `(i64) -> i64`              |
| `.push_byte(b)`     | `(i64) -> ()`               |
| `.push_str(s)`      | `(String) -> ()`            |
| `.clear()`          | `() -> ()` — frees buffer, resets to empty |
| `.contains(s)`      | `(String) -> bool`          |
| `.eq(s)`            | `(String) -> bool`          |
| `.substring(start, end)` | `(i64, i64) -> String` |
| `.parse_i64()`      | `() -> Option<i64>`         |
| `.parse_u64()`      | `() -> Option<u64>`         |

### HashMap<K, V> Methods

| Method              | Signature                   |
|---------------------|-----------------------------|
| `HashMap::new()`    | `() -> HashMap<K, V>`       |
| `.insert(k, v)`     | `(K, V) -> ()`              |
| `.get(k)`           | `(K) -> V`                  |
| `.contains_key(k)`  | `(K) -> bool`               |
| `.remove(k)`        | `(K) -> ()`                 |
| `.len()`            | `() -> i64`                 |

### BTreeMap<K, V> Methods

In Phase 1, both `K` and `V` must be `i64`. K violations raise `BTreeMapKeyTypeMustBeI64`; V violations raise `BTreeMapValueTypeMustBeI64`.
The runtime helpers and ESBMC C model are hard-coded to i64 keys + i64 values; widening V
to support struct payloads is a planned follow-up.
Storage is two parallel sorted arrays (binary-search lookup, sorted-insert writes).
Iteration order is ascending by key and is **deterministic across runs and compilers** —
prefer `BTreeMap` over `HashMap` for any map whose iteration affects compiler output.

| Method              | Signature                   |
|---------------------|-----------------------------|
| `BTreeMap::new()`   | `() -> BTreeMap<K, V>`      |
| `.insert(k, v)`     | `(K, V) -> Option<V>` (returns the previous value bound to `k`, if any) |
| `.get(k)`           | `(K) -> Option<V>` (returns the value bound to `k`, or `None`)          |
| `.contains(k)`      | `(K) -> bool`               |
| `.len()`            | `() -> i64`                 |

### Option<T> Methods

| Method      | Signature                              |
|-------------|----------------------------------------|
| `.unwrap()` | `() -> T` (panics on None; requires `[panic]` effect) |

The `?` operator on `Option<T>` or `Result<T, E>` propagates `None`/`Err` to the caller (the calling function must return `Option` or `Result`).

## Indexing

```vow
let val: i64 = v[0];
v[i] = new_val;
```

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
extern "C" vow {
    requires: fd >= 0
    ensures: return >= 0
}
{
    fn write(fd: i32, ptr: i64, len: i64) -> i64 [io]
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

### Builtin Function Signatures

#### FFI Wrapper Intrinsics

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `pin_to_root`    | `fn(value: String) -> String` and `fn<T>(value: Vec<T>) -> Vec<T>` for flat scalar `T` | `[]` |

`pin_to_root` is a compiler intrinsic, not a user-defined generic. Each call site is monomorphised from the argument type. It always deep-copies the supported heap value into root storage; it does not inspect descriptor tags and does not claim idempotency. The current supported forms are `String` and `Vec<T>` where `T` is a flat scalar slot type (`i*`, `u*`, `f32`, `f64`, `bool`). Pointer-containing payloads, user structs, enums, and maps require hand-written deep-copy wrappers at the FFI boundary.

`String::from_raw_parts_copy(ptr: i64, len: i64)` copies `len` bytes from a raw C pointer into a fresh `String`. `Vec::from_raw_parts_copy(ptr: i64, len: i64)` copies `len` flat scalar slots into a fresh `Vec<T>`. The surface length type is `i64`; the code generator converts pointer and length values to the platform pointer-sized ABI type at the FFI boundary. Both helpers have a `FreshInCaller` return summary.

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
| `string_substr`       | `fn(s: String, start: i64, len: i64) -> String`  | `[]`    |
| `string_split`        | `fn(s: String, delim: String) -> Vec<String>`    | `[]`    |
| `string_starts_with`  | `fn(s: String, prefix: String) -> i64`           | `[]`    |
| `string_ends_with`    | `fn(s: String, suffix: String) -> i64`           | `[]`    |
| `string_matches_literal_at` | `fn(s: String, pos: i64, literal: String literal) -> i64` | `[]` |
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
values outside the target type's range.

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

`memory_root_arena_bytes()` returns the current bytes retained by root-region arena chunks. `memory_peak_bytes()` returns the peak live bytes retained by all open arena chunks since process start. `memory_alloc_count_since_start()` returns the number of successful Vow arena allocation requests since process start. These queries do not allocate; they are effectful because they observe runtime process state.

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
