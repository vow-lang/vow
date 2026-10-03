# M03: Vec Sum

## Problem

Implement a function `vec_sum` that computes the wrapping `i64` sum of all
elements in a Vec.

## Signature

```vow
fn vec_sum(v: Vec<i64>) -> i64
```

## Contracts

- `vec_sum` is currently uncontracted because Vow cannot yet express a fold over
  all Vec elements
- The loop counter `i` is a `u64` local, so it needs no `invariant: i >= 0`; convert with `n as u64` in `make_vec`
- Loop `invariant: i <= n as u64` in `make_vec` and `invariant: i <= v.len()` in `vec_sum`

## Constraints

- Iterate with index, accumulate sum
- Addition uses Vow's normal wrapping `i64` semantics
- This benchmark is Stretch until the aggregate fold relation is expressible

## Hints

- Start `sum = 0`, add `v[i]` each iteration
- Verifier unwind and Vec-model limits are not source preconditions
