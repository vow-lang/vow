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
- Loop `invariant: lo >= 0`
- Loop `invariant: hi >= lo`

## Constraints

- Use a while loop with the overflow-safe `lo < hi - 1` condition
- Compute midpoint as `lo + (hi - lo) / 2`
- Return `lo` after convergence

## Hints

- The midpoint formula avoids overflow
- `hi - 1` is safe because `lo >= 0` and `hi >= lo`
