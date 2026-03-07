# M02: Vec Fill

## Problem

Implement a function `fill_vec` that creates a Vec of `n` elements.

## Signature

```vow
fn fill_vec(n: i64) -> Vec<i64>
```

## Contracts

- `requires: n >= 0` — count is non-negative
- `requires: n <= 8` — bounded for verification
- `ensures: result.len() == n` — resulting Vec has exactly `n` elements
- Loop `invariant: i >= 0`
- Loop `invariant: i <= n`

## Constraints

- Create a Vec with `Vec::new()`, push elements in a loop
- The function has no effects

## Hints

- Push `i` in each iteration; the Vec grows by 1 each time
- The loop invariant tracks `i` within `[0, n]`
