# E04: Clamp

## Problem

Implement a function `clamp` that restricts a value to a given range `[lo, hi]`.

## Signature

```vow
fn clamp(x: i64, lo: i64, hi: i64) -> i64
```

## Contracts

- `requires: lo <= hi` — the range must be valid
- `ensures: result >= lo` — result is at least `lo`
- `ensures: result <= hi` — result is at most `hi`

## Constraints

- Use nested `if`/`else` branching
- The function is pure

## Hints

- Three cases: `x < lo`, `x > hi`, or `x` is already in range
