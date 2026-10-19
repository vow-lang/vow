
### Built-in Enums

`Option<T>` has variants `Some(T)` and `None`.
`Result<T, E>` has variants `Ok(T)` and `Err(E)`.
For scalar payloads supported by native aggregate codegen, `match` and
`.unwrap()` extract the declared type, including `f32` and `f64`; `?`
currently propagates `Option` payloads only. The ESBMC backend's C model moves `f32`
and `f64` enum payloads and struct fields through its integer slots by
IEEE-754 bit pattern (an `f64` as its raw 64 bits, an `f32` as its 32 bits
zero-extended), as native codegen does, so contracts over float payloads
verify under the default bit-vector encoding; `--encoding ir` does not
model the reinterpretation and can report a spurious counterexample.
Float equality follows IEEE-754: a NaN payload does not equal itself.

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
| `Vec::from_raw_parts_copy(ptr, len)` | `(i64, u64) -> Vec<T>` for one-slot scalar `T` (not `i128`/`u128`) |
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

**Key and value types.** The runtime stores each key and each value in one 64-bit slot and compares keys by value. A `HashMap` key must therefore be `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`, or `bool`; every other key type is an `UnsupportedFeature` error in both compilers. `String`, `Vec`, struct, enum, `Option`, and tuple keys are heap-backed handles that would compare by pointer, so a lookup with an equal-but-distinct `String` would silently miss (and a mutable `String` mutated after insertion would corrupt the map). `i128`/`u128` keys would be truncated, and `f32`/`f64` have no total equality. Hash or intern such keys to a `u64` at the call site (`hash_str` / `hash_u64`, see [Hashing](#hashing)) and keep a side table for the originals. A `HashMap` or `BTreeMap` value of type `i128`, `u128`, `f32`, or `f64` is likewise an `UnsupportedFeature` error: map values occupy a single 64-bit integer slot, so a 128-bit value would lose its high word and a float has no slot encoding. A `HashMap` value that is or transitively contains a `linear struct` is an `UnsupportedFeature` error for the same reason `BTreeMap` rejects it (`BTreeMapValueMustBeNonLinear`): the map copies values bitwise and `get` would hand out a second copy of the linear obligation. Narrow integer values (`i8` … `u32`) are stored widened and read back at their declared width. The check applies wherever the map type is written (annotations, parameters, returns, fields, aliases, constants), including nested inside `Vec`, `Option`, tuples, and other maps. A 128-bit integer nested inside an aggregate value (`Option<u128>`, a struct field) is not a map restriction: aggregates hold 128-bit fields in two slots, so only a bare 128-bit map value is rejected.

**Set idiom.** The unit type `()` is a valid map value, so `HashMap<K, ()>` and `BTreeMap<K, ()>` are sets: `s.insert(k, ());` adds a member, `s.contains_key(k)` (`s.contains(k)` for `BTreeMap`) tests membership, `s.remove(k)` deletes it, and `s.get(k)` returns `Option<()>`. The value `()` has type `()` (it checks against a `()` annotation or return type), and the runtime stores it in the usual 64-bit slot as `0`. A function cannot take a `()` parameter (`UnsupportedFeature`: the argument carries no information and has no ABI slot), so pass the set itself or a key instead.

**Representation and cost.** A `HashMap` is an open-addressing hash table with linear probing, at most 3/4 full, that doubles when it fills; removal leaves no tombstones, so lookup cost does not degrade after churn. `insert`, `get`, `contains_key` and `remove` take expected O(1) time (amortised for `insert`, which occasionally rehashes) and `len` is O(1). The hash is a fixed, seedless mixer, so a program's behaviour and its compiled binary are reproducible; the cost is that keys chosen to collide on purpose can lengthen probe chains, which is acceptable for compiler-style workloads but is not a defence against hash flooding. A `HashMap` has no iteration API and its slot order is unspecified; use `BTreeMap` when order matters.

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
