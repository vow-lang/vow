# M06: Vec Max

## Problem

Implement a function `vec_max` that finds the maximum element in a non-empty Vec.

## Signature

```vow
fn vec_max(v: Vec<i64>) -> i64
```

## Contracts

- `requires: v.len() > 0` — Vec must be non-empty
- `ensures: result >= v[0]` — the maximum is at least the first element
- Loop `invariant: best >= v[0]`
- Loop `invariant: i >= 1`
- Loop `invariant: i <= v.len()`

## Constraints

- Initialize `best` to `v[0]`, scan from index 1
- Update `best` when `v[i] > best`

## Hints

- Since `best` starts at `v[0]` and only increases, `best >= v[0]` is maintained
- Start loop from `i = 1` since `best` is already `v[0]`
- Verifier unwind and Vec-model limits are not source preconditions
