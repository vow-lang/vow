# M03: Vec Sum

## Problem

Implement a function `vec_sum` that sums all elements of a Vec of non-negative integers.

## Signature

```vow
fn vec_sum(v: Vec<i64>) -> i64
```

## Contracts

- `requires: v.len() >= 0` — valid Vec
- `requires: v.len() <= 8` — bounded for verification
- `ensures: result >= 0` — sum of non-negative elements is non-negative
- Loop `invariant: sum >= 0`
- Loop `invariant: i >= 0`
- Loop `invariant: i <= v.len()`

## Constraints

- Iterate with index, accumulate sum
- Elements are assumed non-negative (simplification for verification)

## Hints

- Start `sum = 0`, add `v[i]` each iteration
- The invariant `sum >= 0` holds because each element contributes non-negatively
