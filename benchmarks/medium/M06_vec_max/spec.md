# M06: Vec Max

## Problem

Implement a function `vec_max` that finds the maximum element in a non-empty Vec of non-negative integers.

## Signature

```vow
fn vec_max(v: Vec<i64>) -> i64
```

## Contracts

- `requires: v.len() > 0` — Vec must be non-empty
- `requires: v.len() <= 8` — bounded for verification
- `ensures: result >= 0` — max of non-negative elements is non-negative
- Loop `invariant: best >= 0`
- Loop `invariant: i >= 1`
- Loop `invariant: i <= v.len()`

## Constraints

- Initialize `best` to `v[0]`, scan from index 1
- Update `best` when `v[i] > best`

## Hints

- Since all elements are non-negative, `best >= 0` is maintained
- Start loop from `i = 1` since `best` is already `v[0]`
