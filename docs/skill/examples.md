# Worked Examples

Verification workflow examples. The first three demonstrate Counterexample-Guided Inductive Synthesis (CEGIS) cycles: write spec, build, read JSON, diagnose, fix, verify. The fourth shows break-with-value in loop expressions.

## 1. Safe Division — Requires Pattern

### Goal

Write a division function that is safe (cannot divide by zero).

### Step 1: Write the spec

```vow
module Divide

fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
} {
    x / y
}

fn main() -> i32 [io] {
    divide(10, 0);
    0
}
```

### Step 2: Build and verify

```
$ vow build examples/divide.vow
```

```json
{"status":"Verified","executable":"examples/divide","diagnostics":[],"counterexamples":[]}
```

ESBMC proves the contract: whenever `y != 0` holds, the division is safe.

### Step 3: Runtime behavior (debug mode)

The `main()` calls `divide(10, 0)` which violates `requires: y != 0`. In debug mode:

```
$ vow build --mode debug --no-verify examples/divide.vow -o /tmp/divide_debug
$ /tmp/divide_debug
```

Stderr:
```json
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"examples/divide.vow","offset":56,"values":{"y":0}}
```

The `blame: "Caller"` tells you: `main()` passed `y=0`, which violates the precondition.

---

## 2. CEGIS Broken → Fixed — The Core Workflow

### Goal

Write `safe_sub(a, b)` that always returns a non-negative result.

### Step 1: Initial attempt (broken)

```vow
module CegisBroken

fn safe_sub(a: i64, b: i64 where b >= 0) -> i64 vow {
    ensures: result >= 0
} {
    a - b
}

fn main() -> i32 [io] {
    print_i64(safe_sub(10, 3));
    0
}
```

### Step 2: Build

```
$ vow build examples/cegis_broken.vow
```

```json
{
  "status": "VerifyFailed",
  "executable": "examples/cegis_broken",
  "diagnostics": [],
  "function": "safe_sub",
  "counterexample": "[Counterexample]",
  "counterexamples": [
    {
      "function": "safe_sub",
      "inputs": { "a": "-9223372036854775808", "b": "0" },
      "violation": "ensures result >= 0",
      "vow_id": 1,
      "source": { "file": "examples/cegis_broken.vow", "offset": 76, "length": 20 }
    }
  ]
}
```

### Step 3: Diagnose

The counterexample shows `a = -9223372036854775808` (i64 min), `b = 0`. Then `a - b = a`, which is negative. The `ensures: result >= 0` is violated.

**Root cause:** We need `a >= b` to guarantee a non-negative result, and `a >= 0` to prevent negative inputs.

### Step 4: Fix

```vow
module CegisFixed

fn safe_sub(a: i64 where a >= 0, b: i64 where b >= 0) -> i64 vow {
    requires: a >= b,
    ensures: result >= 0
} {
    a - b
}

fn main() -> i32 [io] {
    print_i64(safe_sub(10, 3));
    0
}
```

### Step 5: Verify

```
$ vow build examples/cegis_fixed.vow
```

```json
{"status":"Verified","executable":"examples/cegis_fixed","diagnostics":[],"counterexamples":[]}
```

Verified. With `a >= 0`, `b >= 0`, and `a >= b`, ESBMC proves `result >= 0`.

---

## 3. Vec Fill — Loop Invariant

### Goal

Fill a vector with `n` elements and prove its length equals `n`.

### Step 1: Write the spec

```vow
module VecFill

fn fill_vec(n: i64) -> Vec<i64> vow {
    requires: n >= 0,
    requires: n <= 8,
    ensures: result.len() == n
} {
    let v: Vec<i64> = Vec::new();
    let mut i: i64 = 0;
    while i < n vow {
        invariant: i >= 0,
        invariant: i <= n
    } {
        v.push(i);
        i = i + 1;
    }
    v
}

fn main() -> i32 [io] {
    let v: Vec<i64> = fill_vec(5);
    print_i64(v.len());
    0
}
```

### Step 2: Build and verify

```
$ vow build examples/vec_fill.vow
```

```json
{"status":"Verified","executable":"examples/vec_fill","diagnostics":[],"counterexamples":[]}
```

**Key points:**
- `requires: n <= 8` keeps iterations within ESBMC's unwind bound (10)
- `invariant: i >= 0, invariant: i <= n` is inductive: true on entry, preserved by the loop body
- The Vec model tracks `len`, so ESBMC can reason about `result.len() == n`

---

## 4. Linear Search — Break-with-Value

### Goal

Search a vector for a target value and return its index, or `-1` if not found. Uses `loop` with `break <value>` to produce a result directly from the loop expression.

### Step 1: Write the spec

```vow
module Search

fn linear_search(data: Vec<i64>, target: i64) -> i64
    vow { requires: data.len() > 0 }
{
    let mut i: i64 = 0;
    let n: i64 = data.len();
    let result: i64 = loop {
        if i >= n {
            break -1;
        }
        if data[i] == target {
            break i;
        }
        i = i + 1;
    };
    result
}

fn main() -> i32 [io] {
    let data: Vec<i64> = Vec::new();
    data.push(10);
    data.push(20);
    data.push(30);
    data.push(40);
    data.push(50);

    let idx: i64 = linear_search(data, 30);
    print_i64(idx);

    let idx2: i64 = linear_search(data, 99);
    print_i64(idx2);
    0
}
```

### Step 2: Build and verify

```
$ vow build examples/search.vow
```

```json
{"status":"Verified","executable":"examples/search","diagnostics":[],"counterexamples":[]}
```

**Key points:**
- `loop { ... break <value>; ... }` is an expression that evaluates to the break value
- All `break` expressions in a `loop` must produce the same type (`i64` here)
- `break <value>` is only allowed in `loop`, not in `while` (which always evaluates to `()`)
- The result is bound with `let result: i64 = loop { ... };`
