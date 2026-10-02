# M04: Vec Find

## Problem

Implement a function `vec_find` that searches for a value in a Vec, returning its index or -1 if not found.

## Signature

```vow
fn vec_find(v: Vec<i64>, target: i64) -> i64
```

## Contracts

- `ensures: result >= 0 - 1` — result is -1 or a valid index
- `ensures: result < v.len() as i64` — if found, index is valid (also true for the -1 sentinel)
- The loop counter `i` is a `u64` local, so it needs no `invariant: i >= 0`; bridge with `v.len() as u64` and `found = i as i64`
- Loop `invariant: i <= v.len() as u64`
- Loop `invariant: found >= 0 - 1` and `invariant: found < v.len() as i64`

## Constraints

- Linear scan with early return on match
- Return -1 if not found

## Hints

- Use a mutable `found` variable initialized to -1
- Set `found = i as i64` when `v[i] == target`
- Return `found` after the loop
- Verifier unwind and Vec-model limits are not source preconditions
