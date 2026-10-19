
### UnsupportedFeature

**Phase:** Parser (`&expr`), Type Checker (everything else)
**Meaning:** A language feature that is not supported in Vow was used, or a `HashMap`/`BTreeMap` was written with a key or value type the runtime cannot store (see [Map key and value types](#map-key-and-value-types)).

**Wrong:**

```vow
trait Foo {
    fn bar() -> i64;
}
```

**Output:** `trait blocks are not supported in Vow`

**Fix:** Remove the unsupported construct. Vow does not support traits or impl blocks.

**Right:**

```vow
fn bar() -> i64 {
    0
}
```

A prefix `&expr` (or `&mut expr`, `&&expr`) borrow expression is rejected by the parser, at the `&` or `&&` token, with
the hint ``Vow has no borrow expressions: pass the value itself; `&` is only the binary bitwise
AND operator``; the operand is still parsed, so the diagnostic is reported once:

**Wrong:**

```vow
fn f(x: i64) -> i64 {
    let r: i64 = &x;
    r
}
```

**Output:** ``borrow expressions (`&expr`) are not supported in Vow``

**Fix:** Use the value directly. `&` is the bitwise AND operator between two operands (`x & 1`).

**Right:**

```vow
fn f(x: i64) -> i64 {
    let r: i64 = x;
    r
}
```

A tuple expression, including the empty tuple `()`, inside a contract clause is also
`UnsupportedFeature`, because tuples have no runtime or verifier representation:

**Wrong:**

```vow
fn f(a: i64, b: i64) -> i64 vow {
    requires: (a, b) != (1, 2)
} {
    a
}
```

**Output:** `tuple expressions are not supported in contract predicates`

**Fix:** Compare the elements individually, e.g. `requires: a != 1 || b != 2`.

**Right:**

```vow
fn f(a: i64, b: i64) -> i64 vow {
    requires: a != 1 || b != 2
} {
    a
}
```

#### Map key and value types

The same code reports a map type whose key or value cannot be stored in the runtime's single 64-bit map slot: a `HashMap` key that is not an integer type of at most 64 bits or `bool`, a `HashMap` or `BTreeMap` value of type `i128`, `u128`, `f32`, or `f64`, and a `HashMap` value that is or contains a `linear struct` (see [HashMap key and value types](grammar.md#hashmap-methods)).

**Wrong:**

```vow
fn f() -> () {
    let m: HashMap<String, i64> = HashMap::new();
}
```

**Output:** ``HashMap key type `String` is not supported: keys are compared by value as a single machine word``

**Fix:** Hash or intern the key to a `u64` at the call site and keep a side table for the originals. For a 128-bit value, store the two `u64` halves separately. For a linear value, keep it in a local binding and store only an integer handle in the map.

**Right:**

```vow
fn f() -> () {
    let m: HashMap<u64, i64> = HashMap::new();
}
```

#### Linear collection elements

The same code reports a `Vec<T>` whose element type is or contains a linear owner (a `linear struct`, or an `Option`, `Result`, or user enum wrapping one). Collections copy and shift elements bitwise, so a stored linear value would be duplicated or would escape the checker's consume-once tracking. A reference element (`Vec<&Token>`) borrows rather than owns and is accepted; a nested collection is reported once, where the innermost `Vec` is written, and a type alias once, at its definition.

**Wrong:**

```vow
linear struct Token { id: i64 }

fn f() -> () {
    let v: Vec<Token> = Vec::new();
}
```

**Output:** `Vec element type must be non-linear; found 'Token'`

**Fix:** Keep the linear value in a local binding and store an integer handle in the `Vec`. Consume a linear value that is no longer needed with `drop(value)`.

**Right:**

```vow
linear struct Token { id: i64 }

fn f(t: Token) -> () {
    let ids: Vec<i64> = Vec::new();
    drop(t);
}
```

#### Slice types

The same code reports a slice type `[T]` (see [Slice Types](grammar.md#slice-types)). Each bracket pair is reported once (`[[i64]]` gives two diagnostics, an alias once at its definition), with the hint ``use `Vec<T>` to hold a sequence of values``.

**Wrong:**

```vow
fn probe(sl: [i64]) -> i64 { 0 }
```

**Output:** ``slice types (`[T]`) are not supported in Vow``

**Fix:** Take a `Vec<T>` instead.

**Right:**

```vow
fn probe(sl: Vec<i64>) -> i64 { 0 }
```

#### Reserved type names

The same code reports a `struct`, `enum` or `type` alias named after a builtin type (see [User-Defined Types](grammar.md#user-defined-types)). It is reported once per declaration, at the declaration, with the hint `choose a different name for this type`.

**Wrong:**

```vow
struct Vec { x: i64 }
```

**Output:** `` `Vec` is a builtin type name and cannot be declared as a user type ``

**Fix:** Rename the type.

**Right:**

```vow
struct Coord { x: i64 }
```

#### Unit parameters

The same code reports a parameter of type `()` (directly or through a type alias). The unit value carries no information and has no ABI slot, so such a parameter is rejected at the declaration.

**Wrong:**

```vow
fn f(u: ()) -> i64 { 0 }
```

**Output:** ``parameter `u` has type `()`: unit parameters are not supported``

**Fix:** Remove the parameter.

**Right:**

```vow
fn f() -> i64 { 0 }
```

A 128-bit integer as an `Option`, `Result`, or `enum` payload is supported, so `HashMap<i64, Option<u128>>` is a valid map type (the map stores a pointer to the payload cell). A bare `u128` or `i128` map value is still rejected with the error above, because a map value occupies a single 64-bit slot. A 128-bit struct field or `Vec` element is not a map restriction: aggregates hold 128-bit values in two slots, so only a bare 128-bit map value is rejected.

### BTreeMapKeyTypeMustBeI64

**Phase:** Type Checker
**Meaning:** A `BTreeMap<K, V>` was instantiated with `K` not equal to `i64`. Phase 1 of the BTreeMap stdlib only supports `i64` keys; the runtime helpers and the verifier's model are hard-coded to i64.

**Wrong:**

```vow
fn f() -> () {
    let m: BTreeMap<bool, i64> = BTreeMap::new();
    m.insert(true, 1);
}
```

**Output:** `BTreeMap key type must be i64; found 'bool'`

**Fix:** Use `BTreeMap<i64, V>`. If you need string or struct keys, hash or intern them to `i64` at the call site and keep a side-table for the originals.

**Right:**

```vow
fn f() -> () {
    let m: BTreeMap<i64, i64> = BTreeMap::new();
    m.insert(1, 1);
}
```

### BTreeMapValueMustBeNonLinear

**Phase:** Type Checker
**Meaning:** A `BTreeMap<K, V>` was instantiated with a `V` that is or transitively contains a `linear struct`. Non-linear containers like `BTreeMap`, `Vec`, and `HashMap` cannot hold linear values because their internal shift/copy operations are bitwise and would silently duplicate the linear ownership obligation.

**Wrong:**

```vow
linear struct Token { id: i64 }
fn f() -> () {
    let m: BTreeMap<i64, Token> = BTreeMap::new();
}
```

**Output:** `BTreeMap value type must be non-linear; found 'Token'`

**Fix:** Either drop the `linear` qualifier on the struct, or keep handles in a `Vec<i64>` indirection and consume the linear values via direct function calls outside the map.

**Right:**

```vow
linear struct Token { id: i64 }

fn f(t: Token) -> () {
    let m: BTreeMap<i64, i64> = BTreeMap::new();
    drop(t);
}
```

### MissingContract

**Phase:** Type Checker
**Meaning:** An `extern "C"` block was declared without a `vow { ... }` contract. Every foreign function call requires a mandatory contract specifying expected behavior.

**Wrong:**

```vow
extern "C" {
    fn write_thing(fd: i32, ptr: i64, len: i64) -> i64 [io];
}
```

**Output:** `extern block requires a vow contract`

**Fix:** Add a `vow { ... }` block to the extern declaration with `requires` and/or `ensures` clauses.

**Right:**

```vow
extern "C" {
    vow {
        requires: fd >= 0
    }
    fn write_thing(fd: i32, ptr: i64, len: i64) -> i64 [io];
}
```

### ContractTypeMismatch

**Phase:** Type Checker
**Meaning:** A `requires`, `ensures`, or `invariant` clause expression, or a parameter `where` clause, does not have type `bool`.

**Wrong:**

```vow
fn add(a: i64, b: i64) -> i64 vow {
    requires: a + b
} {
    a + b
}
```

**Output:** `` `requires` clause has type `i64` but must be `bool` ``

**Fix:** Ensure every contract clause is a boolean expression (comparison, logical operator, or a call to a predicate function returning `bool`).

**Right:**

```vow
fn add(a: i64, b: i64) -> i64 vow {
    requires: a >= 0 && b >= 0
} {
    a + b
}
```

### VowRequiresViolated

**Phase:** Verification
**Meaning:** The verifier found inputs that violate a `requires` precondition. This is a **static** verification error — it means the function's callers can reach it with invalid arguments.

**Wrong:**

```vow
fn dec(n: u64) -> u64 vow {
    requires: n > 0
} {
    n - 1
}

fn caller(x: u64) -> u64 {
    dec(x)
}
```

**Fix:** Fix the callers to pass valid arguments, or, if the function is correct on a wider domain, state that real domain in the `requires` clause.

**Right:** (`dec` unchanged)

```vow
fn caller(x: u64) -> u64 {
    if x > 0 {
        dec(x)
    } else {
        0
    }
}
```

### VowEnsuresViolated

**Phase:** Verification
**Meaning:** The verifier found inputs where the function's return value does not satisfy the `ensures` postcondition.

**Wrong:**

```vow
fn max(a: i64, b: i64) -> i64 vow {
    ensures: result >= a && result >= b
} {
    a
}
```

**Fix:** Fix the function body to satisfy the postcondition, or weaken the `ensures` clause.

**Right:**

```vow
fn max(a: i64, b: i64) -> i64 vow {
    ensures: result >= a && result >= b
} {
    if a > b { a } else { b }
}
```

### VowInvariantViolated

**Phase:** Verification
**Meaning:** The verifier found a loop iteration where the `invariant` does not hold.

**Wrong:**

```vow
fn count_up() -> u64 {
    let mut i: u64 = 0;
    while i < 10 vow {
        invariant: i <= 10
    } {
        i = i + 3;
    }
    i
}
```

**Fix:** Strengthen the invariant or fix the loop body. No compiler emits `VowInvariantViolated` today: a loop-invariant counterexample is reported with `error_code` `VowEnsuresViolated`, the message reads ``contract violation in `f`: invariant <predicate>``, and the counterexample `violation` starts with `invariant`.

**Right:**

```vow
fn count_up() -> u64 {
    let mut i: u64 = 0;
    while i < 10 vow {
        invariant: i <= 10
    } {
        i = i + 1;
    }
    i
}
```

### EsbmcNotFound

**Phase:** Verification
**Meaning:** ESBMC is not installed or not on `$PATH`. When verification is enabled with the ESBMC backend (the default for `vowc build`, always for `vowc verify`), the compiler checks for ESBMC upfront before compilation. If ESBMC is not found, the build aborts immediately with exit code 1. Under `--backend native` the verifier is Bitwuzla instead: a missing `bitwuzla` is reported per function as `VerifyFailed` with `verify_status: "tool_not_found"` (clause `status` `error` in `vowc contracts --verify`), and ESBMC is never consulted.

**Wrong:**

```sh
vowc build f.vow
```

**Fix:** Install ESBMC, or use `--no-verify` to skip verification: `vowc build --no-verify <file>`.

**Right:**

```sh
vowc build --no-verify f.vow
```
