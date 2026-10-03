# M02: Vec Fill

## Problem

Implement a function `fill_vec` that creates a Vec of `n` elements.

## Signature

```vow
fn fill_vec(n: u64) -> Vec<i64>
```

## Contracts

- `n` is a `u64` count, so the type already says it is non-negative; no `requires: n >= 0` is needed (and `TautologicalComparison` rejects one)
- `ensures: result.len() == n` — resulting Vec has exactly `n` elements
- The loop counter `i` is a `u64` local, so it needs no `invariant: i >= 0`; convert with `i as i64` when pushing
- Loop `invariant: i <= n`

## Constraints

- Create a Vec with `Vec::new()`, push elements in a loop
- The function has no effects

## Hints

- Push `i as i64` in each iteration; the Vec grows by 1 each time
- The loop invariant tracks `i` within `[0, n]`
- Verifier unwind and Vec-model limits are not source preconditions
