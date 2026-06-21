# Loop invariants

A bounded model checker reasons about loops by *unwinding* them, but to prove a property
holds for **every** iteration it needs an **invariant**: a fact that is true before the
loop, preserved by each iteration, and strong enough to imply what you want afterwards.
In Vow you attach one to a loop with `invariant`.

## A loop without help

Consider summing `1 + 2 + ... + n`. Its only real precondition is `n >= 0`, plus an
overflow guard so the running total stays within `i64`:

```vow
module SumRange

fn sum_to(n: i64) -> i64 vow {
    requires: n >= 0,
    requires: n <= 4294967295,
    ensures: result >= 0
} {
    let mut total: i64 = 0;
    let mut i: i64 = 1;
    while i <= n {
        invariant: i >= 1,
        invariant: i <= n + 1,
        invariant: total >= 0
        total = total + i;
        i = i + 1;
    }
    total
}

fn main() -> i32 [io] {
    print_i64(sum_to(100));   // 5050
    0
}
```

```console
$ ulimit -v 2000000; build/vowc verify sumrange.vow
{"status":"Verified","executable":null,"diagnostics":[],"counterexamples":[]}
```

## What each invariant is doing

- `i >= 1` and `i <= n + 1` **bound the counter.** Together with the loop condition
  `i <= n`, they tell the verifier exactly where `i` lives at every step — on exit,
  `i == n + 1`.
- `total >= 0` **carries the property you want to the exit.** It's true initially
  (`total = 0`), each iteration adds a non-negative `i`, so it's preserved — and it
  directly discharges the function's `ensures: result >= 0`.

If you drop the invariants, the verifier cannot conclude `result >= 0` for an arbitrary
`n` and the proof fails. The invariant is the inductive bridge from "true before" to
"true after."

!!! note "Bounds vs. contracts"
    The `requires: n <= 4294967295` bound is **semantic**: it is the largest `n` for
    which `n*(n+1)/2` still fits in `i64`, so it is a genuine overflow guard. Do **not**
    instead add a bound purely to fit the verifier's unwinding limits — CLAUDE.md is
    explicit that *ESBMC bounds are not contracts*. A contract expresses what is
    mathematically required, not what the tool finds convenient. See
    [Contract methodology](../reference/contracts-methodology.md).

For more worked loops (binary search, Vec fills), see the
[worked examples](../reference/examples.md). Next: reuse verified code from the
**[standard library](using-stdlib.md)**.
