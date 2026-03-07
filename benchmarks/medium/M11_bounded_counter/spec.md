# M11: Bounded Counter

## Problem

Implement a bounded counter using pure i64 functions. The counter value is tracked as an integer parameter, not as a struct field.

## Signatures

```vow
fn counter_inc(count: i64, max: i64) -> i64
fn counter_is_zero(count: i64) -> i64
```

## Contracts

- `counter_inc`: `requires: count >= 0, requires: count < max, requires: max <= 100`, `ensures: result == count + 1, ensures: result <= max`
- `counter_is_zero`: `requires: count >= 0`, `ensures: result >= 0, ensures: result <= 1`

## Constraints

- Pure integer operations with bounded counter semantics
- `counter_inc` must not exceed `max`

## Hints

- `counter_inc` returns `count + 1`
- `counter_is_zero` returns 1 if `count == 0`, else 0
