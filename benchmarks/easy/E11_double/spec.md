# E11: Double

## Problem

Implement two functions: `twice` that doubles a value, and `negate` that negates a value. Both must satisfy functional equality contracts.

## Signatures

```vow
fn twice(x: i64) -> i64
fn negate(x: i64) -> i64
```

## Contracts

- `twice`: `ensures: result == x + x`
- `negate`: `ensures: result + x == 0`

## Constraints

- Each function is a single expression
- Both functions are pure

## Hints

- `twice` returns `x + x`
- `negate` returns `0 - x`
