# Your first contract

A contract is a `vow` block attached to a function. It states what must be true on the
way **in** (`requires`, a precondition) and on the way **out** (`ensures`, a
postcondition). The verifier proves these hold for *all* inputs — not just the ones
you happened to test.

## A precondition

Integer division by zero is undefined. Make that impossible to call wrong:

```vow
module Divide

fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
} {
    x / y
}

fn main() -> i32 [io] {
    print_i64(divide(10, 2));   // 5
    0
}
```

```console
$ ulimit -v 2000000; build/vowc build divide.vow
{"status":"Verified","executable":"divide","diagnostics":[],"counterexamples":[]}
```

`Verified` means the verifier proved that, given the contract, the body is safe — and
that `main`'s call `divide(10, 2)` satisfies `y != 0`.

## Blame: who is at fault?

Contracts assign **blame**. The rule is simple:

- A **`requires`** violation blames the **caller** — it passed bad arguments.
- An **`ensures`** (or `invariant`) violation blames the **callee** — the function
  failed to deliver what it promised.

To see it, compile in **debug mode**, which inserts runtime contract checks, and call
`divide` with a zero divisor:

```vow
fn main() -> i32 [io] {
    print_i64(divide(10, 0));   // violates requires: y != 0
    0
}
```

```console
$ ulimit -v 2000000; build/vowc build --mode debug divide.vow
$ ulimit -v 2000000; ./divide
{"error":"VowViolation","vow_id":0,"blame":"Caller",
 "description":"requires: y != 0","values":{"y":0}}
```

The `values` object reports the runtime values of every variable in the predicate —
here `y` was `0`. Blame is `Caller`, because a `requires` was broken.

!!! note "Debug vs release"
    `--mode debug` inserts these runtime checks; release builds omit them entirely.
    Static verification (`vow verify`, or the default `vow build`) is the stronger
    guarantee — it proves the contract for all inputs instead of checking one run.

## Tighten the postcondition

Preconditions guard the inputs; postconditions pin down the output. A good `ensures`
admits only correct implementations:

```vow
fn abs(x: i64) -> i64 vow {
    requires: x > -9223372036854775807,
    ensures: result >= 0,
    ensures: result == x || result == 0 - x
} {
    if x < 0 { 0 - x } else { x }
}
```

`result` is the keyword for the return value. The second `ensures` is what makes this
contract *tight*: `result >= 0` alone would also be satisfied by a function that always
returns `0`. Requiring `result == x || result == 0 - x` rejects that.

In the [next step](cegis.md) we deliberately get a contract wrong and let the verifier
catch it.
