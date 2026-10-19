
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

`v[i] = new_val` evaluates `v`, then `i`, then `new_val`, left to right, in both compilers. This
order is observable whenever any of the three sub-expressions has a side effect — not only a
declared `[io]`/`[read]`/`[write]`/`[panic]`/`[unsafe]` effect, but also a local-variable mutation
that a sibling sub-expression later reads, such as `v[i] = { i = i + 1; 7 }`: `i` is read as its
pre-mutation value because `i` is evaluated before `new_val`.

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

| Effect   | Meaning                                                                    |
|----------|----------------------------------------------------------------------------|
| `io`     | Output and process/system interaction (print, process, time, mkdir, remove, rename) |
| `read`   | Reads of external state (file system, environment, arguments, stdin)       |
| `write`  | File system writes (`fs_write`)                                            |
| `panic`  | May panic (unwrap, etc.)                                                   |
| `unsafe` | Unsafe operations (FFI, raw memory)                                        |

Each effect is independent — `io` is not a superset of `read` or `write`. A function that prints and also reads a file declares both: `[io, read]`.

`.unwrap()` requires `[panic]`. Out-of-bounds indexing, the checked operators (`+! -! *! /! %!`) and the `/`, `%` zero-divisor traps do not: their aborts are verification obligations of pure functions, and declaring an effect would remove the function from the verifier model.

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
| `pin_to_root`    | `fn(value: String) -> String` and `fn<T>(value: Vec<T>) -> Vec<T>` for one-slot scalar `T` | `[]` |

`pin_to_root` is a compiler intrinsic, not a user-defined generic. Each call site is monomorphised from the argument type. It always deep-copies the supported heap value into root storage; it does not inspect descriptor tags and does not claim idempotency. The current supported forms are `String` and `Vec<T>` where `T` is a one-slot scalar type (`i8`..`i64`, `u8`..`u64`, `f32`, `f64`, `bool`; not `i128`/`u128`, whose elements take two slots). Pointer-containing payloads, user structs, enums, and maps require hand-written deep-copy wrappers at the FFI boundary.
