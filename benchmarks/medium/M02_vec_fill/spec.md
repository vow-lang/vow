# M02: Vec Fill

## Problem

Implement a function `fill_vec` that creates a Vec of `n` elements.

## Signature

```vow
fn fill_vec(n: i64) -> Vec<i64>
```

## Contracts

- `requires: n >= 0` — count is non-negative
- `ensures: result.len() as i64 == n` — resulting Vec has exactly `n` elements
- The loop counter `i` is a `u64` local, so it needs no `invariant: i >= 0`; convert with `n as u64` and `i as i64`
- Loop `invariant: i <= n as u64`

## Constraints

- Create a Vec with `Vec::new()`, push elements in a loop
- The function has no effects

## Hints

- Push `i as i64` in each iteration; the Vec grows by 1 each time
- The loop invariant tracks `i` within `[0, n]`
- Verifier unwind and Vec-model limits are not source preconditions
