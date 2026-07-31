# E01: Absolute Value

## Problem

Implement a function `abs` that returns the absolute value of an integer.

## Signature

```vow
fn abs(x: i64) -> i64
```

## Contracts

- `requires: x > -9223372036854775808` — excludes the only `i64` value whose negation overflows
- `ensures: result >= 0` — the result is always non-negative

## Constraints

- Use branching (`if`/`else`), not built-in functions
- The function is pure (no effects)

## Hints

- Consider two cases: `x >= 0` and `x < 0`
- When `x < 0`, negate it with `0 - x`
- The precondition is the exact arithmetic domain of signed absolute value
