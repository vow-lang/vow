# E12: Power of Two

## Problem

Implement a function `pow2` that computes 2^n using a loop.

## Signature

```vow
fn pow2(n: i64) -> i64
```

## Contracts

- `requires: n >= 0` — exponent is non-negative
- `requires: n <= 62` — exact largest exponent whose result fits in `i64`
- `ensures: result >= 1` — 2^n is always at least 1

## Constraints

- Use a while loop that multiplies by 2 in each iteration
- Include a loop invariant

## Hints

- Start with `result = 1` and multiply by 2, `n` times
- Loop invariant: `r >= 1` (the accumulator is always positive)
- The upper bound prevents arithmetic overflow; it is not an unwind bound
