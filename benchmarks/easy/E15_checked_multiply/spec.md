# E15: Checked Multiply

## Problem

Implement a function `checked_mul` that multiplies two bounded non-negative integers.

## Signature

```vow
fn checked_mul(a: i64, b: i64) -> i64
```

## Contracts

- `requires: a >= 0, a <= 1000` — `a` is bounded non-negative
- `requires: b >= 0, b <= 1000` — `b` is bounded non-negative
- `ensures: result == a * b` — result equals the product

## Constraints

- Single multiplication expression
- Bounds prevent overflow
- The function is pure

## Hints

- With `a, b` in `[0, 1000]`, the product is at most 1,000,000, well within i64 range
