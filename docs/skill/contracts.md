# Contract Authoring and Verification

Vow uses ESBMC (bounded model checker) for static contract verification. This document covers contract patterns, verification behavior, and common pitfalls.

## Verification Pipeline

Codegen (Cranelift) and verification run in parallel:

```
Vow Source → Parse → Type Check → IR Lower ─┬─→ Cranelift → executable
                                              └─→ C Emit → ESBMC → proof / counterexample
```

Contract clauses become IR opcodes. The C emitter translates `requires` to `__ESBMC_assume()` (the verifier assumes preconditions hold) and `ensures`/`invariant` to `__ESBMC_assert()` (the verifier checks postconditions).

### ESBMC Configuration

- Loop unwind bound: **10** — loops are checked for up to 10 iterations
- Architecture: 64-bit
- Array bounds / pointer checks disabled (Vow handles these in its own model)

### Collection Models for Verification

ESBMC uses bounded models for collection types:

| Type              | Max Capacity | Supported Operations |
|-------------------|-------------|----------------------------------------------|
| `Vec<T>`          | 128         | `new`, `push`, `pop`, `len`, `get`, `set`    |
| `String`          | 256         | `from`, `len`, `push_byte`, `byte_at`        |
| `HashMap<K, V>`   | 64          | `new`, `insert`, `get`, `contains_key`, `len`|

These support the same operations as the runtime but with bounded storage. `String::from` produces a nondeterministic length (0 to 255) in verification.

## Blame Model

| Clause      | Blame  | Who is at fault                                    |
|-------------|--------|----------------------------------------------------|
| `requires`  | Caller | The caller passed invalid arguments                |
| `ensures`   | Callee | The function body doesn't satisfy the postcondition|
| `invariant` | Callee | The loop body breaks the invariant                 |

## Integer Contracts

### Non-zero Guard

```vow
fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
} {
    x / y
}
```

### Range Bounds

```vow
fn safe_add(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: a <= 100,
    requires: b >= 0,
    requires: b <= 100,
    ensures: result >= 0,
    ensures: result <= 200
} {
    a + b
}
```

### Equality Postcondition

```vow
fn twice(x: i64) -> i64 vow {
    ensures: result == x + x
} {
    x + x
}
```

### Negation

```vow
fn negate(x: i64) -> i64 vow {
    ensures: result + x == 0
} {
    0 - x
}
```

**Warning:** Fails for `x = -9223372036854775808` (i64 min) due to wrapping overflow. Add `requires: x > -9223372036854775808` if needed.

## Vec Contracts

### Bounds Check

```vow
fn get_element(v: Vec<i64>, i: i64) -> i64 vow {
    requires: i >= 0,
    requires: i < v.len()
} {
    v[i]
}
```

### Fill Pattern with Loop Invariant

See the worked CEGIS example in [examples.md](examples.md#3-vec-fill--loop-invariant).

## String Contracts

### Non-empty String

```vow
fn make_greeting() -> String vow {
    ensures: result.len() > 0
} {
    let s: String = String::from("");
    s.push_byte(72);
    s
}
```

## HashMap Contracts

### Contains Key After Insert

```vow
fn insert_and_check() -> HashMap<i64, i64> vow {
    ensures: result.contains_key(42)
} {
    let m: HashMap<i64, i64> = HashMap::new();
    m.insert(42, 100);
    m
}
```

## Loop Invariants

### Counter Bounds

The most common loop invariant pattern bounds the loop counter:

```vow
while i < n vow {
    invariant: i >= 0,
    invariant: i <= n
} {
    i = i + 1;
}
```

### Search Range

```vow
fn bisect(lo: i64, hi: i64) -> i64 vow {
    requires: hi >= lo
} {
    let mut lo: i64 = lo;
    let mut hi: i64 = hi;
    while lo + 1 < hi vow {
        invariant: hi - lo >= 0
    } {
        let mid: i64 = lo + (hi - lo) / 2;
        lo = mid;
    }
    lo
}
```

## Where Clause Patterns

Where clauses on parameters become refinement types (additional `requires` for verification):

```vow
fn bounded_add(a: i64 where a >= 0, b: i64 where b >= 0) -> i64 vow {
    requires: a <= 100,
    requires: b <= 100,
    ensures: result >= 0,
    ensures: result <= 200
} {
    a + b
}
```

Each `where` clause can only reference its own parameter.

## Anti-Patterns

### Over-Specifying

```vow
fn add(x: i64, y: i64) -> i64 vow {
    ensures: result == x + y
} {
    x + y
}
```

Fails when `x + y` overflows. The contract mirrors the implementation exactly — it verifies nothing useful and breaks on edge cases.

**Fix:** Add bounds (`requires: x >= 0, ...`) or verify a weaker property.

### Wrapping Arithmetic Overflow

Default arithmetic (`+`, `-`, `*`) wraps on overflow. Contracts that assume no overflow will be violated:

```vow
fn double(x: i64) -> i64 vow {
    ensures: result > x
} {
    x + x
}
```

ESBMC finds: `x = 4611686018427387904` → `result = -9223372036854775808` (wraps negative).

**Fix:** Bound the input or use checked arithmetic (`+!`).

### Non-Inductive Loop Invariant

An invariant must hold at the **start** of every iteration, not just at the end:

```vow
while i < n vow {
    invariant: v.len() == n
} { ... }
```

This is not inductive — `v.len() == n` is only true after the loop.

**Fix:** Use `invariant: i >= 0, invariant: i <= n`.

### Unbound Loop Iterations

Without a bound on loop iterations, ESBMC may timeout (unwind bound is 10):

```vow
fn fill(n: i64) -> Vec<i64> vow {
    requires: n >= 0,
    ensures: result.len() == n
} { ... }
```

**Fix:** Add `requires: n <= 8` (or another value below the unwind bound).

## Interpreting Counterexamples

A counterexample in the JSON output:

```json
{
  "function": "safe_sub",
  "inputs": { "a": "-9223372036854775808", "b": "0" },
  "violation": "ensures result >= 0",
  "vow_id": 1,
  "source": { "file": "cegis_broken.vow", "offset": 76, "length": 20 }
}
```

| Field       | Meaning                                                |
|-------------|--------------------------------------------------------|
| `function`  | Which function failed                                  |
| `inputs`    | Parameter values that trigger the violation            |
| `violation` | Which contract clause was violated                     |
| `vow_id`    | Internal ID linking to the specific vow clause         |
| `source`    | Byte offset in the source file of the violated clause  |

Variable names prefixed with `_esbmc_` are ESBMC internal variables; named inputs map directly to function parameters.
