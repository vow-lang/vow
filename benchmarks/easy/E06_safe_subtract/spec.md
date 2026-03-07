# E06: Safe Subtract

## Problem

Implement a function `safe_sub` that subtracts `b` from `a`, guaranteeing a non-negative result.

## Signature

```vow
fn safe_sub(a: i64 where a >= 0, b: i64 where b >= 0) -> i64
```

## Contracts

- `where a >= 0` — `a` is non-negative
- `where b >= 0` — `b` is non-negative
- `requires: a >= b` — `a` must be at least `b`
- `ensures: result >= 0` — result is non-negative

## Constraints

- Single subtraction expression
- The function is pure

## Hints

- The `where` clauses constrain parameter ranges inline
- With `a >= b` and both non-negative, `a - b >= 0` follows directly
