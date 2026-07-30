# M12: Swap Check

## Problem

Implement functions that demonstrate value selection and combination with equality contracts.

## Signatures

```vow
fn select_first(a: i64, b: i64) -> i64
fn select_second(a: i64, b: i64) -> i64
fn sum_pair(a: i64, b: i64) -> i64
```

## Contracts

- `select_first`: `ensures: result == a`
- `select_second`: `ensures: result == b`
- `sum_pair`: `requires: a >= 0, requires: b >= 0, requires: a <= 9223372036854775807 - b`, `ensures: result == a + b`

## Constraints

- Each function is a single expression
- All functions are pure

## Hints

- `select_first` returns `a`
- `select_second` returns `b`
- `sum_pair` returns `a + b` (the exact precondition prevents overflow)
