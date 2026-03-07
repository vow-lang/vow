# E10: Safe Modulo

## Problem

Implement a function `safe_mod` that computes `x % m` safely with bounded result.

## Signature

```vow
fn safe_mod(x: i64 where x >= 0, m: i64) -> i64
```

## Contracts

- `where x >= 0` — input is non-negative
- `requires: x <= 1000` — bounded for verification
- `requires: m > 0` — modulus is positive
- `requires: m <= 100` — bounded for verification
- `ensures: result >= 0` — result is non-negative
- `ensures: result < m` — result is strictly less than modulus

## Constraints

- Single modulo expression
- The function is pure

## Hints

- With `x >= 0` and `m > 0`, the modulo `x % m` is always in `[0, m)`
