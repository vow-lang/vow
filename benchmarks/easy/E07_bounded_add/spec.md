# E07: Bounded Add

## Problem

Implement a function `bounded_add` that adds two bounded non-negative integers.

## Signature

```vow
fn bounded_add(a: i64 where a >= 0, b: i64 where b >= 0) -> i64
```

## Contracts

- `where a >= 0` — `a` is non-negative
- `where b >= 0` — `b` is non-negative
- `requires: a <= 100` — `a` is bounded
- `requires: b <= 100` — `b` is bounded
- `ensures: result >= 0` — result is non-negative
- `ensures: result <= 200` — result is bounded

## Constraints

- Single addition expression
- The function is pure

## Hints

- With both inputs in `[0, 100]`, the sum is in `[0, 200]`
