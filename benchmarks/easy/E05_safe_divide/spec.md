# E05: Safe Divide

## Problem

Implement a function `divide` that divides `x` by `y`, with preconditions that exclude every input on which signed division aborts: a zero divisor and `i64::MIN / -1`.

## Signature

```vow
fn divide(x: i64, y: i64) -> i64
```

## Contracts

- `requires: y != 0` — divisor must be non-zero (caller's responsibility)
- `requires: x >= -9223372036854775807 || y != -1` — the quotient must be representable: `x / y` aborts for `x == i64::MIN` and `y == -1`

## Constraints

- Single division expression
- The function is pure

## Hints

- The contract prevents both aborting inputs of signed division at the caller site
- The body is simply `x / y`
