# M14: Selection Min

## Problem

Implement a function `find_min_idx` that finds the index of the minimum element in a Vec.

## Signature

```vow
fn find_min_idx(v: Vec<i64>) -> i64
```

## Contracts

- `requires: v.len() > 0` — Vec must be non-empty
- `requires: v.len() <= 8` — bounded for verification
- `ensures: result >= 0` — valid index
- `ensures: result < v.len()` — within bounds
- Loop `invariant: min_idx >= 0`
- Loop `invariant: min_idx < v.len()`
- Loop `invariant: i >= 1`
- Loop `invariant: i <= v.len()`

## Constraints

- Linear scan tracking the index of the minimum

## Hints

- Initialize `min_idx = 0`, scan from index 1
- Update `min_idx` when `v[i] < v[min_idx]`
