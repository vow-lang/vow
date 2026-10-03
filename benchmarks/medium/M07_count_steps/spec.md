# M07: Count Steps

## Problem

Implement a function `count_steps` that counts from 0 to `n` using a loop, proving the count equals `n`.

## Signature

```vow
fn count_steps(n: u64) -> u64
```

## Contracts

- `n` is a `u64` count, so the type already says it is non-negative; no `requires: n >= 0` is needed
- `ensures: result == n` — result equals `n`
- The loop counter `i` is a `u64` local, so it needs no `invariant: i >= 0`
- Loop `invariant: i <= n`

## Constraints

- Use a while loop incrementing `i` from 0 to `n`
- Return `i` after the loop

## Hints

- The loop condition is `i < n`; after the loop `i == n`
- The invariant `i <= n` is key to proving the ensures clause
- Verifier unwind limits are not source preconditions
