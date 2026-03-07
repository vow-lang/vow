# E09: Is Even

## Problem

Implement a function `is_even` that returns 1 if `x` is even, 0 if odd.

## Signature

```vow
fn is_even(x: i64 where x >= 0) -> i64
```

## Contracts

- `where x >= 0` — input is non-negative
- `ensures: result >= 0` — result is non-negative
- `ensures: result <= 1` — result is 0 or 1

## Constraints

- Use the modulo operator `%`
- Return an i64 (1 for even, 0 for odd)
- The function is pure

## Hints

- `x % 2` gives 0 for even, 1 for odd
- Use `if x % 2 == 0` to branch
