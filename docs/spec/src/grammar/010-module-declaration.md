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
    requires: x >= -9223372036854775807 || y != -1
} {
    x / y
}
```

The `vow` block sits between the signature and the body. Clauses:
- `requires: <expr>` — precondition (blame: Caller)
- `ensures: <expr>` — postcondition (blame: Callee); use `result` for the return value
- `invariant: <expr>` — loop invariant (blame: Callee)

Multiple clauses are separated by commas; the comma may be omitted. A semicolon between clauses is a parse error:

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
| `f32`  | 32-bit float (IEEE-754; the native backend models `+ - * /` and comparisons, see [cli.md](cli.md), and skips `%`; the ESBMC backend has limited support) |
| `f64`  | 64-bit float (IEEE-754; the native backend models `+ - * /` and comparisons, see [cli.md](cli.md), and skips `%`; the ESBMC backend has limited support) |
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

**128-bit implementation status:** `i128`/`u128` are executable end to end:
full-range literals, arithmetic, casts, narrowing, and `parse_i128`/`parse_u128`
compile natively. A 128-bit value may be an `Option`, `Result`, or `enum`
payload, a struct field, or a `Vec` element (see the layout below). The ESBMC backend does not model 128-bit aggregates and constants, so
it reports `Skipped` rather than an unsound result; the native backend
(`--backend native`) models them exactly. Never weaken
contracts to fit the verifier.

**Aggregate layout:** every struct field and enum payload occupies one 8-byte
slot regardless of declared type (narrow ints are padded). An `i128`/`u128`
struct field or enum payload occupies two consecutive 8-byte slots, low limb
first ([ADR 0001](../adr/0001-numeric-tower-narrow-ints.md) decision 9), so
every field or payload after it moves up by one slot and the allocation grows
with it. A struct has no tag, so its first field is slot 0; an enum's tag takes
slot 0 and payloads start at slot 1. A `Vec<i128>`/`Vec<u128>` element is 16
bytes (two slots, low limb first); `len` and `cap` still count elements. There
is no packing or natural-alignment layout today; FFI structs that need a
specific C layout must shim through `Vec<u8>` or extern wrappers.

### Built-in Parameterized Types

| Type               | Description                     |
|--------------------|---------------------------------|
| `Vec<T>`           | Growable array. `T` must be non-linear (see [Linear Structs](#linear-structs)) |
| `Option<T>`        | Optional value (Some/None)      |
| `Result<T, E>`     | Success or error                |
| `String`           | UTF-8 string (backed by Vec<u8>)|
| `HashMap<K, V>`    | Key-value map (open-addressing hash table; expected O(1) amortised `insert`/`get`/`contains_key`/`remove`). `K` must be an integer type of at most 64 bits or `bool`; `V` may be any non-linear type except `i128`/`u128`/`f32`/`f64` |
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
