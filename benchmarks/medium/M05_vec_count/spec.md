# M05: Vec Count

## Problem

Implement a function `vec_count_pos` that counts the number of positive elements in a Vec.

## Signature

```vow
fn vec_count_pos(v: Vec<i64>) -> i64
```

## Contracts

- `requires: v.len() >= 0`
- `requires: v.len() <= 8` — bounded for verification
- `ensures: result >= 0` — count is non-negative
- `ensures: result <= v.len()` — count is at most the Vec length
- Loop `invariant: count >= 0`
- Loop `invariant: count <= i`
- Loop `invariant: i >= 0`
- Loop `invariant: i <= v.len()`

## Constraints

- Linear scan, increment counter when `v[i] > 0`

## Hints

- `count` starts at 0 and is incremented at most once per iteration
- The invariant `count <= i` ensures `count <= v.len()` after the loop
