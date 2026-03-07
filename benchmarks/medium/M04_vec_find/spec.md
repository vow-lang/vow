# M04: Vec Find

## Problem

Implement a function `vec_find` that searches for a value in a Vec, returning its index or -1 if not found.

## Signature

```vow
fn vec_find(v: Vec<i64>, target: i64) -> i64
```

## Contracts

- `requires: v.len() >= 0`
- `requires: v.len() <= 8` — bounded for verification
- `ensures: result >= 0 - 1` — result is -1 or a valid index
- `ensures: result < v.len()` — if found, index is valid (also true for -1 < len when len >= 0)
- Loop `invariant: i >= 0`
- Loop `invariant: i <= v.len()`

## Constraints

- Linear scan with early return on match
- Return -1 if not found

## Hints

- Use a mutable `found` variable initialized to -1
- Set `found = i` when `v[i] == target`
- Return `found` after the loop
