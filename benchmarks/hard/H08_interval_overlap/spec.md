# H08: Interval Overlap

## Problem

Implement interval operations: creation, containment check, width, and overlap detection. Verified functions use i64 parameters for contracts.

## Signatures

```vow
struct Interval { lo: i64, hi: i64 }
fn interval_new(lo: i64, hi: i64) -> Interval
fn interval_contains(lo: i64, hi: i64, x: i64) -> i64
fn interval_width(lo: i64, hi: i64) -> i64
fn intervals_overlap(a_lo: i64, a_hi: i64, b_lo: i64, b_hi: i64) -> i64
```

## Contracts

- `interval_new`: `requires: lo <= hi`
- `interval_contains`: `requires: lo >= 0, hi >= 0, lo <= hi, hi <= 1000`, `ensures: result >= 0, ensures: result <= 1`
- `interval_width`: `requires: lo >= 0, hi >= 0, lo <= hi, hi <= 1000`, `ensures: result >= 0`
- `intervals_overlap`: `requires: a_lo >= 0, a_hi >= 0, a_lo <= a_hi, a_hi <= 1000, b_lo >= 0, b_hi >= 0, b_lo <= b_hi, b_hi <= 1000`, `ensures: result >= 0, ensures: result <= 1`

## Constraints

- Struct kept for data grouping; verified functions use i64 params
- Bounded inputs prevent overflow in arithmetic
- Overlap detection: two intervals overlap if `a_lo <= b_hi` and `b_lo <= a_hi`

## Hints

- `interval_new` returns `Interval { lo: lo, hi: hi }`
- `interval_contains`: check `x >= lo` and `x <= hi`
- `interval_width`: `hi - lo`
- `intervals_overlap`: check if NOT disjoint — disjoint when `a_hi < b_lo` or `b_hi < a_lo`
