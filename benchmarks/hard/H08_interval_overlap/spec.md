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
- `interval_contains`: `requires: lo <= hi`, `ensures: result >= 0, ensures: result <= 1`
- `interval_width`: `requires: lo <= hi, lo >= 0 || hi <= 9223372036854775807 + lo`, `ensures: result >= 0`
- `intervals_overlap`: `requires: a_lo <= a_hi, b_lo <= b_hi`, `ensures: result >= 0, ensures: result <= 1`

## Constraints

- Struct kept for data grouping; verified functions use i64 params
- Valid ordering is the only domain restriction for operations that do not
  compute a width
- Width additionally uses the exact guard that keeps `hi - lo` in `i64`
- Overlap detection: two intervals overlap if `a_lo <= b_hi` and `b_lo <= a_hi`

## Hints

- `interval_new` returns `Interval { lo: lo, hi: hi }`
- `interval_contains`: check `x >= lo` and `x <= hi`
- `interval_width`: `hi - lo`
- `intervals_overlap`: check if NOT disjoint — disjoint when `a_hi < b_lo` or `b_hi < a_lo`
