# E08: Sign

## Problem

Implement a function `sign` that returns -1 for negative numbers, 0 for zero, and 1 for positive numbers.

## Signature

```vow
fn sign(x: i64) -> i64
```

## Contracts

- `ensures: result >= -1` — result is at least -1
- `ensures: result <= 1` — result is at most 1

## Constraints

- Use nested `if`/`else` branching
- The function is pure

## Hints

- Three cases: `x < 0` returns -1, `x == 0` returns 0, `x > 0` returns 1
- In Vow, write negative literals as `0 - 1`
