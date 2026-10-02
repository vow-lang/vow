# H04: Sorted Insert (Stretch)

## Problem

Implement a function that inserts a value into a sorted Vec while maintaining sorted order.

## Signatures

```vow
fn sorted_insert(v: Vec<i64>, val: i64) -> Vec<i64>
```

## Contracts

- `ensures: result.len() == v.len() + 1` — one element added
- `i` is a `u64` local, so it needs no `>= 0` invariant; bridge with `v.len() as u64`
- Loop `invariant: i <= v.len() as u64`

## Constraints

- Find the correct position, shift elements, insert
- This is a Stretch problem — may exceed ESBMC's current capabilities

## Hints

- Find insertion point by scanning until `v[i] >= val`
- Build new Vec: copy elements before insertion point, insert val, copy remaining
- Verifying sorted order across the entire Vec is hard for bounded model checking
- Verifier unwind and Vec-model limits are not source preconditions
