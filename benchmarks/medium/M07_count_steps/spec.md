# M07: Count Steps

## Problem

Implement a function `count_steps` that counts from 0 to `n` using a loop, proving the count equals `n`.

## Signature

```vow
fn count_steps(n: i64) -> i64
```

## Contracts

- `requires: n >= 0` — `n` is non-negative
- `requires: n <= 8` — bounded for verification
- `ensures: result == n` — result equals `n`
- Loop `invariant: i >= 0`
- Loop `invariant: i <= n`

## Constraints

- Use a while loop incrementing `i` from 0 to `n`
- Return `i` after the loop

## Hints

- The loop condition is `i < n`; after the loop `i == n`
- The invariant `i <= n` is key to proving the ensures clause
