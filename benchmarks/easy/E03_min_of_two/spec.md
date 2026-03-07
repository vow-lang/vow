# E03: Min of Two

## Problem

Implement a function `min_of` that returns the minimum of two integers.

## Signature

```vow
fn min_of(a: i64, b: i64) -> i64
```

## Contracts

- `ensures: result <= a` — result is at most `a`
- `ensures: result <= b` — result is at most `b`

## Constraints

- Use branching, not built-in functions
- The function is pure

## Hints

- Compare `a < b` and return the smaller value
