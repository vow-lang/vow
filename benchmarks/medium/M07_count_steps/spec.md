# M07: Count Steps

## Problem

Implement a function `count_steps` that counts from 0 to `n` using a loop, proving the count equals `n`.

## Signature

```vow
fn count_steps(n: i64) -> i64
```

## Contracts

- `requires: n >= 0` — `n` is non-negative
- `ensures: result == n` — result equals `n`
- The loop counter `i` is a `u64` local, so it needs no `invariant: i >= 0`; bridge with `n as u64` and `i as i64`
- Loop `invariant: i <= n`

## Constraints

- Use a while loop incrementing `i` from 0 to `n`
- Return `i` after the loop

## Hints

- The loop condition is `i < n`; after the loop `i == n`
- The invariant `i <= n` is key to proving the ensures clause
- Verifier unwind limits are not source preconditions
