# M14: Selection Min

## Problem

Implement a function `find_min_idx` that finds the index of the minimum element in a Vec.

## Signature

```vow
fn find_min_idx(v: Vec<i64>) -> u64
```

## Contracts

- `requires: v.len() > 0` — Vec must be non-empty
- `ensures: result < v.len()` — within bounds; the `u64` return type already makes it a non-negative index
- `min_idx` and `i` are `u64` locals, so `min_idx` needs no `>= 0` invariant
- Loop `invariant: min_idx < v.len()`
- Loop `invariant: i >= 1`
- Loop `invariant: i <= v.len()`

## Constraints

- Linear scan tracking the index of the minimum

## Hints

- Initialize `min_idx = 0`, scan from index 1
- Update `min_idx` when `v[i] < v[min_idx]`
- Verifier unwind and Vec-model limits are not source preconditions
