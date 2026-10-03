# M11: Bounded Counter

## Problem

Implement a bounded counter using pure functions over `u64` counts. The counter value is tracked as an integer parameter, not as a struct field.

## Signatures

```vow
fn counter_inc(count: u64, max: u64) -> u64
fn counter_is_zero(count: u64) -> i64
```

## Contracts

- `counter_inc`: `requires: count < max`, `ensures: result == count + 1, ensures: result <= max` (`count` is a `u64`, so it needs no `count >= 0`)
- `counter_is_zero`: `ensures: result >= 0, ensures: result <= 1` (the `0`/`1` flag is an `i64`)

## Constraints

- Pure integer operations with bounded counter semantics
- `counter_inc` must not exceed `max`

## Hints

- `counter_inc` returns `count + 1`
- `counter_is_zero` returns 1 if `count == 0`, else 0
