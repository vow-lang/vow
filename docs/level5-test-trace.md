# Level 5 Agent Capability Test — Trace

**Date:** 2026-03-13
**Compiler:** `build/vowc` (self-hosted, Phase 20.3 bootstrap)
**Rust compiler used:** Never (only `build/vowc` after bootstrap)
**Program:** Safe Math + Stats Library (`examples/safemath/`)

---

## Pre-flight

### Confirm `build/vowc` runs

```
$ ulimit -v 2000000; build/vowc --help --human
vow -- Vow compiler

USAGE
  vow build [OPTIONS] <source.vow>    Compile to native executable
  vow verify <source.vow>             Verify contracts only (no executable)
  ...
```

### Sanity check: single-module build + verify

```
$ ulimit -v 2000000; build/vowc build examples/clamp.vow -o /tmp/clamp_test
2 items, 0 errors
  clamp: PROVEN
{"status":"Verified","executable":"/tmp/clamp_test","diagnostics":[],"counterexamples":[]}
```

### Sanity check: multi-module build

```
$ ulimit -v 2000000; build/vowc build --no-verify stdlib/geometry/main.vow -o /tmp/geom_test
12 items, 0 errors
{"status":"Unverified","executable":"/tmp/geom_test","diagnostics":[],"counterexamples":[]}
```

---

## Step 1: Write `safemath.vow` — Pure Arithmetic Module

Five functions, ~10 contracts, all `i64 -> i64`.

```vow
module Safemath

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

fn safe_sub(a: i64, b: i64) -> i64 vow {
  requires: a >= 0,
  requires: b >= 0,
  requires: a >= b,
  ensures: result >= 0
} {
  a - b
}

fn safe_div(a: i64, b: i64) -> i64 vow {
  requires: b != 0
} {
  a / b
}

fn abs_val(x: i64) -> i64 vow {
  requires: x > -9223372036854775807 - 1,
  ensures: result >= 0
} {
  if x < 0 {
    0 - x
  } else {
    x
  }
}

fn clamp(val: i64, lo: i64, hi: i64) -> i64 vow {
  requires: lo <= hi,
  ensures: result >= lo,
  ensures: result <= hi
} {
  if val < lo {
    lo
  } else {
    if val > hi {
      hi
    } else {
      val
    }
  }
}
```

---

## Step 2: First Verification Attempt — CEGIS Iteration 1

Initial version had simpler contracts: `safe_add` had `requires: a >= 0, b >= 0`
without overflow bounds, and `abs_val` had no requires at all.

```
$ ulimit -v 2000000; build/vowc verify examples/safemath/safemath.vow
5 items, 0 errors
Verifying safe_add...
  safe_add: FAILED
    a = 6155143404008179657
    b = 7370724942473446543
Verifying safe_sub...
  safe_sub: PROVEN
Verifying safe_div...
  safe_div: PROVEN
Verifying abs_val...
  abs_val: FAILED
    x = -9223372036854775807 - 1
Verifying clamp...
  clamp: PROVEN
{"status":"VerifyFailed",...}
```

**Analysis:**
- `safe_add`: ESBMC found two large positive values whose sum overflows i64, producing
  a negative result. The `ensures: result >= a` postcondition fails. Fix: bound inputs
  to `<= i64::MAX / 2` (4611686018427387903).
- `abs_val`: ESBMC found `i64::MIN` (-9223372036854775808). `0 - i64::MIN` overflows
  since `i64::MAX` is 9223372036854775807. Fix: add `requires: x > -9223372036854775807 - 1`
  to exclude the single problematic value.

---

## Step 3: Verification After Fixes — CEGIS Iteration 2

```
$ ulimit -v 2000000; build/vowc verify examples/safemath/safemath.vow
5 items, 0 errors
Verifying safe_add...
  safe_add: PROVEN
Verifying safe_sub...
  safe_sub: PROVEN
Verifying safe_div...
  safe_div: PROVEN
Verifying abs_val...
  abs_val: PROVEN
Verifying clamp...
  clamp: PROVEN
{"status":"Verified","executable":null,"diagnostics":[],"counterexamples":[]}
```

All 5 functions PROVEN.

---

## Step 4: Write `stats.vow` — Cross-Module with Intentional Bug

Four functions. `mean_of` intentionally omits preconditions for the CEGIS demo.

```vow
module Stats
use safemath

fn min_of(a: i64, b: i64) -> i64 vow {
  ensures: result <= a,
  ensures: result <= b
} { ... }

fn max_of(a: i64, b: i64) -> i64 vow {
  ensures: result >= a,
  ensures: result >= b
} { ... }

fn bounded_sum(v: Vec<i64>) -> i64 vow {
  requires: v.len() >= 0,
  requires: v.len() <= 8
} { ... loop with invariants ... }

fn mean_of(a: i64, b: i64) -> i64 vow {
  ensures: result >= 0       <-- no requires!
} {
  safe_div(safe_add(a, b), 2)
}
```

---

## Step 5: Verify `stats.vow` — CEGIS Iteration 1

```
$ ulimit -v 2000000; build/vowc verify examples/safemath/stats.vow
9 items, 0 errors
  safe_add: PROVEN
  safe_sub: PROVEN
  safe_div: PROVEN
  abs_val: PROVEN
  clamp: PROVEN
  min_of: PROVEN
  max_of: PROVEN
  bounded_sum: PROVEN
  mean_of: FAILED
{"status":"VerifyFailed",...,
  "counterexamples":[{"function":"mean_of",
    "values":{"_esbmc_v4":"-9223372036854775807 - 1","_esbmc_v6":"0"},
    "violation":"ensures result >= 0","blame":"callee"}]}
```

8/9 PROVEN. `mean_of` fails: without preconditions, ESBMC finds inputs where
`safe_add(a, b)` can return negative (e.g., negative `a` or `b`).

---

## Step 6: Fix `mean_of` — CEGIS Iteration 2

Added preconditions. Also discovered modular verification subtlety: `safe_div`
has no `ensures` clause, so its result is unconstrained from the caller's
perspective. Changed `mean_of` body to inline `(a + b) / 2` so ESBMC can
reason about the full computation.

```vow
fn mean_of(a: i64, b: i64) -> i64 vow {
  requires: a >= 0,
  requires: b >= 0,
  requires: a <= 4611686018427387903,
  requires: b <= 4611686018427387903,
  ensures: result >= 0
} {
  (a + b) / 2
}
```

---

## Step 7: Re-verify `stats.vow` — CEGIS Iteration 3

```
$ ulimit -v 2000000; build/vowc verify examples/safemath/stats.vow
9 items, 0 errors
  safe_add: PROVEN
  safe_sub: PROVEN
  safe_div: PROVEN
  abs_val: PROVEN
  clamp: PROVEN
  min_of: PROVEN
  max_of: PROVEN
  bounded_sum: PROVEN
  mean_of: PROVEN
{"status":"Verified","executable":null,"diagnostics":[],"counterexamples":[]}
```

All 9 functions PROVEN.

---

## Step 8: Write `main.vow` and Full Build

```vow
module Main
use safemath
use stats

fn main() -> i32 [io] {
    print_i64(safe_add(10, 20));
    print_i64(safe_sub(20, 12));
    print_i64(safe_div(42, 3));
    print_i64(abs_val(-42));
    print_i64(clamp(50, 0, 100));
    print_i64(min_of(3, 7));
    print_i64(max_of(3, 7));
    let v: Vec<i64> = Vec::new();
    v.push(10);
    v.push(20);
    v.push(30);
    print_i64(bounded_sum(v));
    print_i64(mean_of(10, 20));
    0
}
```

```
$ ulimit -v 2000000; build/vowc build examples/safemath/main.vow -o /tmp/safemath
10 items, 0 errors
Starting verification of safe_add...
Starting verification of safe_sub...
Starting verification of safe_div...
Starting verification of abs_val...
Starting verification of clamp...
Starting verification of min_of...
Starting verification of max_of...
Starting verification of bounded_sum...
Starting verification of mean_of...
  safe_add: PROVEN
  safe_sub: PROVEN
  safe_div: PROVEN
  abs_val: PROVEN
  clamp: PROVEN
  min_of: PROVEN
  max_of: PROVEN
  bounded_sum: PROVEN
  mean_of: PROVEN
{"status":"Verified","executable":"/tmp/safemath","diagnostics":[],"counterexamples":[]}
```

All 9 contracts verified. Executable produced. Verification ran in parallel with codegen.

---

## Step 9: Run the Binary

```
$ ulimit -v 2000000; /tmp/safemath
30
8
14
42
50
3
7
60
15
```

All results correct:
| Call | Expected | Got |
|------|----------|-----|
| `safe_add(10, 20)` | 30 | 30 |
| `safe_sub(20, 12)` | 8 | 8 |
| `safe_div(42, 3)` | 14 | 14 |
| `abs_val(-42)` | 42 | 42 |
| `clamp(50, 0, 100)` | 50 | 50 |
| `min_of(3, 7)` | 3 | 3 |
| `max_of(3, 7)` | 7 | 7 |
| `bounded_sum([10,20,30])` | 60 | 60 |
| `mean_of(10, 20)` | 15 | 15 |

---

## Step 10: Debug Mode Demo — Runtime VowViolation

Added `safe_div(10, 0)` to `main.vow`. Built with `--mode debug --no-verify`:

```
$ ulimit -v 2000000; build/vowc build --mode debug --no-verify examples/safemath/main.vow -o /tmp/safemath_debug
10 items, 0 errors
{"status":"Unverified","executable":"/tmp/safemath_debug","diagnostics":[],"counterexamples":[]}
```

```
$ ulimit -v 2000000; /tmp/safemath_debug
30
8
14
42
50
3
7
60
15
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"requires b != 0","file":"","offset":0,"values":{"b":0}}
vow violation: requires b != 0, blame=Caller, file=, offset=0, b=0
```

Runtime correctly identifies:
- **Error:** `VowViolation` — a contract was violated at runtime
- **Blame:** `Caller` — the caller of `safe_div` passed an invalid argument
- **Description:** `requires b != 0` — the specific contract that was violated
- **Values:** `b = 0` — the actual value that violated the precondition

Reverted the violation call after capturing output.

---

## Summary

| Metric | Value |
|--------|-------|
| Compiler used | `build/vowc` (self-hosted) exclusively |
| Rust compiler invocations | 0 |
| Modules written | 3 (`safemath.vow`, `stats.vow`, `main.vow`) |
| Functions | 10 (5 pure math + 4 stats + 1 main) |
| Contracts | 12 vow blocks with ~25 clauses |
| CEGIS iterations (safemath) | 2 (overflow + MIN edge cases) |
| CEGIS iterations (stats) | 3 (missing requires → modular verify subtlety → proven) |
| Final verification | 9/9 PROVEN |
| Runtime output | All 9 values correct |
| Debug mode | VowViolation caught with correct blame |
| `ulimit -v 2000000` | On every invocation |

### Key Findings

1. **ESBMC catches real bugs.** The initial `safe_add` had no overflow protection;
   ESBMC immediately found inputs causing i64 wraparound. `abs_val` on `i64::MIN`
   is a classic edge case that ESBMC found in seconds.

2. **Modular verification has implications.** `mean_of` calling `safe_div` couldn't
   be verified because `safe_div` has no `ensures` clause — the verifier treats its
   return value as unconstrained. This is correct behavior: modular verification
   only uses the contract interface, not the implementation.

3. **The CEGIS loop works.** Each counterexample directly pointed to the fix:
   - Large `a + b` overflows → bound inputs
   - `i64::MIN` negation overflows → exclude it
   - Unconstrained `safe_div` result → inline the computation

4. **Cross-module verification works.** `stats.vow` imports `safemath` and all
   9 functions across both modules verify in a single `build/vowc verify` invocation.

5. **Parallel pipeline works.** `build/vowc build` launches all ESBMC instances in
   parallel with codegen. The full build (compile + verify 9 functions + link)
   completes in one command.
