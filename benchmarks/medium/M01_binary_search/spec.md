# M01: Binary Search

## Problem

Implement a function `bisect` that performs binary search narrowing between `lo` and `hi` bounds.

## Signature

```vow
fn bisect(lo: i64, hi: i64) -> i64
```

## Contracts

- `requires: lo >= 0` — lower bound is non-negative
- `requires: hi >= lo` — valid range
- `requires: hi <= 100` — bounded to prevent overflow
- Loop `invariant: lo >= 0`
- Loop `invariant: hi >= lo`
- Loop `invariant: hi <= 100`

## Constraints

- Use a while loop with `lo + 1 < hi` condition
- Compute midpoint as `lo + (hi - lo) / 2`
- Return `lo` after convergence

## Hints

- The midpoint formula avoids overflow
- Bounds on `lo` and `hi` are needed so the invariant `hi >= lo` is provable without overflow
