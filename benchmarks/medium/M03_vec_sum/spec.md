# M03: Vec Sum

## Problem

Implement a function `vec_sum` that sums all elements of a Vec of non-negative integers.

## Signature

```vow
fn vec_sum(v: Vec<i64>) -> i64
```

## Contracts

- `vec_sum` is currently uncontracted because Vow cannot yet express the
  intended element-wise non-negative input predicate
- Loop `invariant: sum >= 0`
- Loop `invariant: i >= 0`
- Loop `invariant: i <= v.len()`

## Constraints

- Iterate with index, accumulate sum
- Elements are assumed non-negative (simplification for verification)
- This benchmark is Stretch until its intended element predicate is expressible

## Hints

- Start `sum = 0`, add `v[i]` each iteration
- The invariant `sum >= 0` holds because each element contributes non-negatively
- Verifier unwind and Vec-model limits are not source preconditions
