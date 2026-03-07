# E02: Max of Two

## Problem

Implement a function `max_of` that returns the maximum of two integers.

## Signature

```vow
fn max_of(a: i64, b: i64) -> i64
```

## Contracts

- `ensures: result >= a` — result is at least `a`
- `ensures: result >= b` — result is at least `b`

## Constraints

- Use branching, not built-in functions
- The function is pure

## Hints

- Compare `a > b` and return the larger value
