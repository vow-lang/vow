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

- Verification strategy: **incremental BMC** (`--incremental-bmc`) — base case plus forward condition, **not** k-induction (there is no inductive step). A contract is `proven` only when ESBMC's forward condition establishes completeness within the bound; otherwise the result is `unknown`, never a false `proven`
- Incremental BMC with `--max-k-step` (default: **50**) — loops are verified incrementally up to N iterations
- Architecture: 64-bit
- Array bounds / pointer checks disabled (Vow handles these in its own model)

### Collection Models for Verification

ESBMC is a *bounded* model checker, so it models collection types as
fixed-size arrays and reasons about them up to a finite capacity. These
capacities are an internal property of the verifier, not of the language:

| Type              | Model Capacity | Supported Operations |
|-------------------|----------------|----------------------------------------------|
| `Vec<T>`          | 128            | `new`, `push`, `pop`, `len`, `get`, `set`    |
| `String`          | 256            | `from`, `len`, `push_byte`, `push_str`, `byte_at`, `matches_literal_at` |
| `HashMap<K, V>`   | 64             | `new`, `insert`, `get`, `contains_key`, `len`|
| `BTreeMap<K, V>`  | 64             | `new`, `insert`, `get`, `contains_key`, `len`|

**These bounds are not a language feature and are not user-tunable.** A `Vec`
in a Vow program grows dynamically on the heap with no fixed maximum; the
capacity above only describes how far the *bounded* model checker reasons. The
language and its contracts are deliberately decoupled from what any particular
prover can prove: replace ESBMC with a stronger (or unbounded) checker and the
same source, the same contracts, and the same CLI keep working — the only
difference is that proof covers more (or all) of the state space. For this
reason a `requires`/`ensures` clause must never encode a verifier bound (e.g.
`requires: v.len() <= 128`); see "Verification-Driven Bounds (Anti-Pattern)"
below and `docs/design/verifier-model-bounds.md`.

These models support the same operations as the runtime but with bounded
storage. String literals carry their concrete length and bytes in verification,
and `String::from` copies that model from its source value. The effective string
model capacity is automatically at least the longest static string literal, so
literal byte initializers always fit the model array. Operations whose bytes are
not statically known, such as `String::from_cstr`, produce a nondeterministic
length (0 to max-1). `string_matches_literal_at` is modeled against the
literal's concrete bytes and byte length; the third argument must be a string
literal so the verifier never has to infer static text from a dynamic `String`.

## Blame Model

| Clause      | Blame  | Who is at fault                                    |
|-------------|--------|----------------------------------------------------|
| `requires`  | Caller | The caller passed invalid arguments                |
| `ensures`   | Callee | The function body doesn't satisfy the postcondition|
| `invariant` | Callee | The loop body breaks the invariant                 |

## Counterexample Replay (Differential Test)

`vow verify --replay-cex` (also `vow build --replay-cex`) cross-checks a counterexample against the executable's runtime semantics. After ESBMC reports a violation, Vow maps the symbolic assignment to concrete Vow inputs, builds a `--mode debug` harness that calls the failing function with them, and checks whether the runtime `VowViolation` matches — **same `vow_id` and same blame**.

This is a *differential test*, **not part of the proof**. The static verdict and exit code are unchanged whether or not replay is requested. Its purpose is to detect drift between the two independent lowerings of a contract: the verifier's C model (`requires` → `__ESBMC_assume`, `ensures`/`invariant` → `__ESBMC_assert`) and `vow-codegen`'s debug-mode runtime checks. A `confirmed` replay grounds the counterexample in real execution; a `diverged` replay flags either a model false-positive or values that do not reach the violation at runtime. See `docs/spec/cli.md` → "Counterexample replay" for the JSON shape and v1 input scope.

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

Use range bounds only when they reflect genuine semantic constraints (e.g., overflow prevention), not to appease the verifier:

```vow
fn safe_add(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: a <= 4611686018427387903,
    requires: b <= 4611686018427387903,
    ensures: result >= a,
    ensures: result >= b
} {
    a + b
}
```

The bounds here prevent `a + b` from overflowing `i64` — a legitimate semantic concern, not a verifier limitation.

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
    let hi: i64 = hi;
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
    requires: a <= 4611686018427387903,
    requires: b <= 4611686018427387903,
    ensures: result >= a,
    ensures: result >= b
} {
    a + b
}
```

Each `where` clause can only reference its own parameter.

## Anti-Patterns

### Tautological Contracts

A contract must constrain behavior the implementation could get wrong. A clause provable from the return type alone, or from a constant/literal body, verifies nothing.

```vow
fn IOP_CONST() -> i64 vow { ensures: result >= 0 } { 0 }
fn sentinel() -> i64 vow { ensures: result == -1 } { -1 }
```

The first is trivially true of the literal `0`; the second restates the body verbatim. Both prove nothing and only enlarge the proof surface.

**Fix:** delete the `vow` block. A postcondition earns its place only when it pins a property of a **computed** result — one that depends on the inputs or control flow and that a wrong implementation would violate (`ensures: result > 0` on a loop-computed `gcd`; `ensures: result == 0 || result == 1` on a branch-computed flag). Named-constant accessors and enum-tag functions returning a literal must carry no contract.

**Crisp rule:** if the clause is true without reading past the signature and a constant body, it is a non-contract — remove it. This is distinct from weakening a real contract (forbidden, see CLAUDE.md "Contract Authoring"): a tautology was never a contract, so deleting it loses no verification value.

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

Without a bound on loop iterations, ESBMC may timeout (default max-k-step is 50):

```vow
fn fill(n: i64) -> Vec<i64> vow {
    requires: n >= 0,
    ensures: result.len() == n
} { ... }
```

ESBMC will only verify this for small `n` values. **Do not** add `requires: n <= 8` to the contract — that would distort the semantic specification. The contract is correct as-is; ESBMC's bounded verification provides partial assurance.

### Verification-Driven Bounds (Anti-Pattern)

**Never** add artificial bounds to contracts solely to help ESBMC verify them:

```vow
// WRONG: bounds exist only to appease the verifier
fn gcd(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: a + b > 0,
    requires: a <= 15,   // <-- verifier artifact, not semantic
    requires: b <= 15,   // <-- verifier artifact, not semantic
    ensures: result > 0
} { ... }
```

```vow
// CORRECT: only genuine semantic constraints
fn gcd(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: a + b > 0,
    ensures: result > 0
} { ... }
```

Contracts express what is mathematically required for correctness. ESBMC verifies within its capabilities (bounded loops, bounded arithmetic, bounded collection models) — if it cannot fully prove a correct contract, that is acceptable. Partial verification is better than a distorted specification. The same rule is why the verifier's collection model capacities (see "Collection Models for Verification") are internal defaults rather than CLI flags or contract clauses: a bound that belongs to the prover must never leak into the language.

## Interpreting Counterexamples

A counterexample in the JSON output:

```json
{
  "function": "safe_sub",
  "values": { "a": "-9223372036854775808", "b": "0" },
  "violation": "ensures result >= 0",
  "vow_id": 1,
  "source": { "file": "cegis_broken.vow", "offset": 76, "length": 20 },
  "blame": "callee"
}
```

| Field       | Meaning                                                        |
|-------------|----------------------------------------------------------------|
| `function`  | Which function's verification query failed                     |
| `values`    | Source or ESBMC variable values in the counterexample           |
| `violation` | Which contract clause was violated                             |
| `vow_id`    | Function-local ID linking to the specific vow clause            |
| `source`    | Byte offset in the source file of the violated clause           |
| `blame`     | Whether the caller, callee, or neither party is responsible     |

When caller code violates a callee's `requires` clause, `violation` and
`vow_id` identify the callee clause. `call_sites` points back to the caller
expression, and `violating_args` identifies the callee parameter and caller
argument span when Vow can recover it.

Variable names prefixed with `_esbmc_` are ESBMC internal variables; named inputs map directly to function parameters.

## Unsigned Integer Contracts

The `u64` type works naturally in contracts. Use `as u64` to cast literal values in contract expressions:

```vow
fn safe_add(a: u64, b: u64) -> u64
vow {
    requires: a <= 1000 as u64
    requires: b <= 1000 as u64
    ensures: result >= a
    ensures: result >= b
}
{
    a + b
}
```

ESBMC verifies `u64` contracts using `uint64_t` and unsigned nondet values.

## Extern Block Contracts

Every `extern "C"` block **must** include a `vow { ... }` contract specifying the expected behavior of foreign functions. Omitting the contract is a `MissingContract` error.

```vow
extern "C" vow {
    requires: fd >= 0
    ensures: return >= 0
}
{
    fn write(fd: i32, ptr: i64, len: i64) -> i64 [io]
}
```

The contract applies to all functions declared in the block. ESBMC uses `requires` as assumptions and `ensures` as assertions when verifying callers of extern functions.
