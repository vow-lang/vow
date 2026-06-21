# The CEGIS loop

CEGIS — *counterexample-guided inductive synthesis* — is the workflow Vow is designed
around:

> **Write** a contract → **verify** → if it fails, read the **counterexample** → **fix**
> the code (or the contract) → verify again, until `Verified`.

Because the compiler emits a concrete counterexample (not just "failed"), an agent can
act on it mechanically. Let's watch it happen.

## Write a function with a bug

Here is `max`, with a tight contract — and a bug in the body:

```vow
module Max

fn max(a: i64, b: i64) -> i64 vow {
    ensures: result >= a,
    ensures: result >= b,
    ensures: result == a || result == b
} {
    a   // BUG: ignores b
}

fn main() -> i32 [io] {
    print_i64(max(3, 7));
    0
}
```

## Verify

```console
$ ulimit -v 2000000; build/vowc verify max.vow
{"status":"VerifyFailed","function":"max",
 "counterexamples":[
   {"function":"max","vow_id":1,"blame":"Callee",
    "values":{"a":0,"b":1,"result":0}}
 ]}
```

The verifier found inputs that break the promise: with `a = 0, b = 1`, the body returns
`0`, which violates `ensures: result >= b`. Blame is `Callee` — the function failed to
deliver its postcondition. The `values` object is the witness.

!!! tip "This is the signal an agent fixes from"
    The counterexample names the failing function, the broken clause (via `vow_id`,
    resolved through the diagnostics), and concrete values. See the
    [CLI reference](../reference/cli.md) for the exact JSON schema.

## Fix and re-verify

Read the witness, fix the body:

```vow
fn max(a: i64, b: i64) -> i64 vow {
    ensures: result >= a,
    ensures: result >= b,
    ensures: result == a || result == b
} {
    if a >= b { a } else { b }
}
```

```console
$ ulimit -v 2000000; build/vowc verify max.vow
{"status":"Verified","executable":null,"diagnostics":[],"counterexamples":[]}
```

`Verified`. The loop is closed.

## Why tight contracts matter

If the postcondition had only been `result >= a && result >= b`, the buggy `return a`
would *still* have failed (good) — but a different bug, `return a + b + 100`, would have
**passed**, because it satisfies `>=` on both. The clause `result == a || result == b`
is what forces the answer to actually be one of the inputs. Weak contracts admit wrong
code; see [Contract methodology](../reference/contracts-methodology.md) for how to make
them tight.

Next: proving properties of code that **[loops](loop-invariants.md)**.
