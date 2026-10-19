
### Division Guard

```vow
fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
    requires: x >= -9223372036854775807 || y != -1
} {
    x / y
}
```

Signed `/` has two abort cases, and the true domain of `divide` excludes both: a zero divisor, and `i64::MIN / -1`, whose quotient is not representable (the runtime aborts with `ArithmeticOverflow` for it exactly as for a zero divisor). The second clause is that overflow guard (`x >= -9223372036854775807` is `x != i64::MIN`), a real part of the function's domain and not a verifier bound; `requires: y != 0` alone is not the whole domain. The native backend proves each as an obligation, so it refutes the one-clause version with `x = i64::MIN`, `y = -1`. The ESBMC C model does not check `MIN / -1` (see `cli.md` → "Verdict divergence from ESBMC"). Unsigned division has only the zero-divisor case.

### Range Bounds

Use range bounds only when they reflect genuine semantic constraints, not to appease the verifier. An operand bound whose only job is to stop wrapping is **not** such a constraint — the checked operator already says that, and says it without narrowing the function's domain:

```vow
fn safe_add(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    ensures: result >= a,
    ensures: result >= b
} {
    a +! b
}
```

`a +! b` aborts rather than wraps, so every execution that *returns* satisfies both postconditions, for every non-negative `a` and `b`. The verifier models that abort, so no bound is needed. Neither `+!` nor indexing requires a `[panic]` effect: a function with any effect is not modelable, so gating them would remove exactly these functions from verification (see [Effect Types](grammar.md#effect-types)). Writing `requires: a <= 4611686018427387903` instead would be the [verification-driven bound](#verification-driven-bounds-anti-pattern) anti-pattern wearing a semantic disguise: it excludes inputs the function handles correctly (it aborts, which is a defined outcome) purely to make wrapping unreachable.

A range bound earns its place when it excludes inputs the function genuinely has no answer for — `requires: x > -9223372036854775807` on `abs`, whose result is not representable at `i64::MIN` under any operator.

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
fn get_element(v: Vec<i64>, i: u64) -> i64 vow {
    requires: i < v.len()
} {
    v[i]
}
```

### Fill Pattern with Loop Invariant

See the worked CEGIS example in [examples.md](examples.md#3-vec-fill-loop-invariant).

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

### Value After Insert

`HashMap::get` returns `Option<V>`, so a contract states the bound value through `.unwrap()`. The verifier proves it when the key is bound; guard it with an earlier `contains_key` clause, because a missing key is reported as a failed `unwrap()` on `None`, which carries no contract blame:

```vow
fn insert_and_read() -> HashMap<i64, i64> vow {
    ensures: result.contains_key(42),
    ensures: result.get(42).unwrap() == 100
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
    ensures: result >= a,
    ensures: result >= b
} {
    a +! b
}
```

Each `where` clause can only reference its own parameter (a sibling parameter or `result` is an undefined-variable `TypeMismatch`), and it obeys the same rules as a `requires` clause: it must be a pure `bool` predicate with no tuple expression. See `grammar.md` → "Where Clauses" for the full list.

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

The verifier finds: `x = 4611686018427387904` → `result = -9223372036854775808` (wraps negative).

**Fix:** use checked arithmetic (`+!`), or bound the input.

```vow
fn double(x: i64) -> i64 vow {
    ensures: result > x
} {
    x +! x
}
```

This verifies. `+!` aborts on overflow instead of wrapping, and the verifier models that: an aborting execution never returns, so it cannot witness a violated `ensures`, and the wrapped value the counterexample was built from is no longer reachable. The two operators have genuinely different models — switching one character changes the verdict.

What you get in exchange is a warning, not silence: because the abort is still *reachable* here (`x` near `i64::MAX`), the verifier reports [`ArithOverflowReachable`](errors.md#arithoverflowreachable) alongside the proof. Read it as "the postcondition holds whenever this returns, and it can fail to return." To rule the abort out as well, constrain the operands — and then the bound is a real precondition of the *caller*, not a bound invented for the verifier.

### Non-Inductive Loop Invariant

An invariant must hold at the **start** of every iteration, not just at the end:

```vow
while i < n vow {
    invariant: v.len() == n
} { ... }
```

This is not inductive — `v.len() == n` is only true after the loop.

**Fix:** Use `invariant: i >= 0, invariant: i <= n`.

Under the native backend the same mistake shows up differently: a wrong invariant is refuted by unwinding (a run that violates it inside the bound is `failed`), while an invariant that is true but not inductive leaves the verdict `unknown` with a `verify_message` that starts `unwinding assertion:`. A failing induction step is never `failed`.

### Unbound Loop Iterations

Without a bound on loop iterations, the verifier may not finish. Under ESBMC the unwind stops at `--max-k-step` (default 50); under the native backend every loop is unrolled to an internal bound of at most 64 and an unrolling that is not enough is `unknown` with an `unwinding assertion:` message:

```vow
fn fill(n: i64) -> Vec<i64> vow {
    requires: n >= 0,
    ensures: result.len() as i64 == n
} { ... }
```

The verifier unrolls only a bounded number of iterations, so for large `n` the verdict is `unknown` rather than `proven`. **Do not** add `requires: n <= 8` to the contract — that would distort the semantic specification. The contract is correct as-is; add a loop `invariant` so the native backend can prove it inductively, and accept an honest `unknown` otherwise.

### Verification-Driven Bounds (Anti-Pattern)

**Never** add artificial bounds to contracts solely to help the verifier prove them:

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

Contracts express what is mathematically required for correctness. The verifier checks within its configured model (bounded loops, and under ESBMC bounded arithmetic and collection models) — if it cannot establish a correct contract, that is acceptable. An honest inconclusive result is better than a distorted specification. This includes collection lengths, capacities, indices, and struct fields: keep real domain and representation constraints, but never cap them merely to fit the model. The same rule is why the verifier's collection model capacities (see "Collection Models for Verification") are internal defaults rather than CLI flags or contract clauses: a bound that belongs to the prover must never leak into the language.

**The overflow-guard bound is this anti-pattern's most common disguise.** A clause like `requires: a <= 4611686018427387903` on an adding function looks semantic — it does describe a real property of `i64` — but its only purpose is to keep wrapping unreachable, and it pays for that by excluding inputs the function handles perfectly well. Use the checked operator instead: `a +! b` aborts rather than wraps, the verifier models the abort, and the contract keeps its true domain. See [Range Bounds](#range-bounds) and [Wrapping Arithmetic Overflow](#wrapping-arithmetic-overflow).

## Interpreting Counterexamples

A counterexample in the JSON output:

```json
{
  "function": "safe_sub",
  "values": { "a": "-9223372036854775808", "b": "0" },
  "violation": "ensures result >= 0",
  "vow_id": 1,
  "source": { "file": "cegis_broken.vow", "offset": 76, "length": 20 },
  "blame": "Callee"
}
```

| Field       | Meaning                                                        |
|-------------|----------------------------------------------------------------|
| `function`  | Which function's verification query failed                     |
| `values`    | Source variable values in the counterexample (decimal text of the declared type)           |
| `violation` | Which contract clause was violated                             |
| `vow_id`    | Function-local ID linking to the specific vow clause            |
| `source`    | Byte offset in the source file of the violated clause           |
| `blame`     | Whether the caller, callee, or neither party is responsible     |

`source.offset` anchors differently depending on where the clause comes from: for a
clause inside a `vow { ... }` block (`requires`, `ensures`, `invariant`), it is the byte
offset of the clause keyword, not the predicate expression, in both compilers. For a
parameter's inline `where` refinement (also reported as `kind: "requires"`), both
compilers anchor on the byte offset of the parameter name instead.

When caller code violates a callee's `requires` clause, `violation` and
`vow_id` identify the callee clause. `call_sites` points back to the caller
expression, and `violating_args` identifies the callee parameter and caller
argument span when Vow can recover it. If `violating_args[].value` is `""`,
Vow could not statically recover the caller argument value; `arg_offset` and
`arg_length` still identify the argument expression.

When a callee verified alongside its caller fails its own `ensures` or
`invariant`, the counterexample is attributed to the callee: `function` is the
callee, `violation`, `source` and `blame` are the callee's clause, and `vow_id`
is the callee-local id (the id a debug-mode `VowViolation` for that callee would
report). It is never resolved against the verified caller's same-numbered clause.

A violation that is not a contract clause (a division or remainder by zero or an over-wide shift under either backend; `MIN / -1`, `unwrap()` on `None` and an out-of-bounds `Vec` index under the native backend only) carries the unattributed `vow_id` `4294967293`, blame `None` and a `violation` text naming the abort. It is not a `requires`, so there is no clause to blame; bound the operand with a real `requires` or guard the operation.

Under the ESBMC backend, variable names prefixed with `$esbmc$` are ESBMC internal variables; `$` cannot
appear in a Vow identifier, so the prefix cannot collide with a source name.
Named inputs map directly to function parameters, including source names that
begin with `_esbmc`.

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

Both backends treat `u64` as an unsigned 64-bit value (`uint64_t` and unsigned nondet values under ESBMC, a 64-bit bit-vector with unsigned comparisons natively), so a counterexample reports the magnitude.

## 128-bit Integer Contracts

`i128`/`u128` literals work in contracts and bodies. The native backend models
128-bit scalars, constants, checked arithmetic and struct fields or enum payloads
as 128-bit bit-vectors, so every value is exact (`i128::MIN`, `i128::MAX` and
`u128::MAX` included) and a counterexample reports the full 128-bit value. Under
the ESBMC backend 128-bit constants are modelled exactly under the bit-vector
encoding, and a proof from its `--encoding ir` timeout fallback is reported as
plain `proven` and is weaker; 128-bit checked arithmetic and aggregate fields
are `Skipped`. A `Vec<i128>` or `Vec<u128>` element is `Skipped` under the
ESBMC backend; `--backend native` models it exactly (a parameter or local `Vec`,
not one read through a struct field). A contract over 128-bit scalars and literals verifies under both:

```vow
fn at_least_one(x: u128) -> u128
vow {
    requires: x >= 1u128
    ensures: result >= 1u128
}
{
    x
}
```

## Extern Block Contracts

Every `extern "C"` block **must** include a `vow { ... }` contract specifying the expected behavior of foreign functions. Omitting the contract is a `MissingContract` error.

```vow
extern "C" {
    vow {
        requires: fd >= 0
        ensures: result >= 0
    }
    fn write_thing(fd: i32, ptr: i64, len: i64) -> i64 [io];
}
```

The contract applies to all functions declared in the block, and documents the expected behavior of foreign functions for readers of the code.

**Declaration-only today.** Extern-declared functions can be declared and type-checked, but not called: both compilers reject a call to an extern-declared function with `UnsupportedFeature`, because neither compiler yet lowers extern-declared calls to IR/codegen with their declared signatures, and neither propagates the block's contract into verification. ESBMC does not use `requires`/`ensures` on an extern block as assumptions or assertions yet — the contract's only effect today is gating the `MissingContract` check above.
