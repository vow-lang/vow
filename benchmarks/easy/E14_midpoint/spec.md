# E14: Midpoint

## Problem

Implement a function `midpoint` that computes the midpoint of two non-negative integers without overflow.

## Signature

```vow
fn midpoint(a: i64, b: i64) -> i64
```

## Contracts

- `requires: a >= 0` — `a` is non-negative
- `requires: b >= a` — `b` is at least `a`
- `ensures: result >= a` — midpoint is at least `a`
- `ensures: result <= b` — midpoint is at most `b`

## Constraints

- Avoid the naive `(a + b) / 2` which can overflow
- The function is pure

## Hints

- Use `a + (b - a) / 2` to avoid overflow
- Since `b >= a`, `b - a >= 0`, so the division is safe
