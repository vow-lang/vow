# E05: Safe Divide

## Problem

Implement a function `divide` that divides `x` by `y`, with a precondition that `y` is non-zero.

## Signature

```vow
fn divide(x: i64, y: i64) -> i64
```

## Contracts

- `requires: y != 0` — divisor must be non-zero (caller's responsibility)

## Constraints

- Single division expression
- The function is pure

## Hints

- The contract prevents division by zero at the caller site
- The body is simply `x / y`
