# M05: Vec Count

## Problem

Implement a function `vec_count_pos` that counts the number of positive elements in a Vec.

## Signature

```vow
fn vec_count_pos(v: Vec<i64>) -> u64
```

## Contracts

- `ensures: result <= v.len()` — count is at most the Vec length; the `u64` return type already makes it non-negative
- `count` and `i` are `u64` locals, so they need no `>= 0` invariants
- Loop `invariant: count <= i`
- Loop `invariant: i <= v.len()`

## Constraints

- Linear scan, increment counter when `v[i] > 0`

## Hints

- `count` starts at 0 and is incremented at most once per iteration
- The invariant `count <= i` ensures `count <= v.len()` after the loop
- Verifier unwind and Vec-model limits are not source preconditions
