# E15: Checked Multiply

## Problem

Implement a function `checked_mul` that multiplies two non-negative integers
when their product fits in `i64`.

## Signature

```vow
fn checked_mul(a: i64, b: i64) -> i64
```

## Contracts

- `requires: a >= 0` — `a` is non-negative
- `requires: b >= 0` — `b` is non-negative
- `requires: b == 0 || a <= 9223372036854775807 / b` — exact overflow guard
- `ensures: result == a * b` — result equals the product

## Constraints

- Single multiplication expression
- The precondition prevents overflow for the complete non-negative `i64` domain
- The function is pure

## Hints

- Guard division by zero before comparing `a` with `i64::MAX / b`
