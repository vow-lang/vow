# H04: Sorted Insert (Stretch)

## Problem

Implement a function that inserts a value into a sorted Vec while maintaining sorted order.

## Signatures

```vow
fn sorted_insert(v: Vec<i64>, val: i64) -> Vec<i64>
```

## Contracts

- `requires: v.len() >= 0`
- `requires: v.len() <= 6` — bounded for verification
- `ensures: result.len() == v.len() + 1` — one element added
- Loop `invariant: i >= 0`
- Loop `invariant: i <= v.len()`

## Constraints

- Find the correct position, shift elements, insert
- This is a Stretch problem — may exceed ESBMC's current capabilities

## Hints

- Find insertion point by scanning until `v[i] >= val`
- Build new Vec: copy elements before insertion point, insert val, copy remaining
- Verifying sorted order across the entire Vec is hard for bounded model checking
