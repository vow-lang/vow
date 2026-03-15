# HE003: Below Zero

**Origin:** HumanEval-003 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Given a list of integers representing bank account operations (positive for
deposits, negative for withdrawals), determine if the account balance ever drops
below zero. The account starts with a balance of zero. Return 1 if the balance
goes below zero at any point, 0 otherwise.

## Signature

```vow
fn below_zero(ops: Vec<i64>) -> i64
```

## Contracts

- `requires: ops.len() >= 0` — valid input
- `requires: ops.len() <= 8` — bounded for verification
- `ensures: result >= 0` — boolean result
- `ensures: result <= 1` — boolean result

## Contract Fidelity

**WEAK** — the Dafny spec ensures `result <==> (exists i :: 0 < i <=
|operations| && sum_prefix(operations, i) < 0)`. Vow cannot express existential
quantifiers. The contract only checks the result is 0 or 1. ESBMC verifies
loop invariants and bounds.

## Hints

- Track a running balance in a mutable variable
- Iterate through the operations, adding each to the balance
- If the balance ever goes below zero, return 1
- Use a loop invariant to track that `i` stays in bounds
