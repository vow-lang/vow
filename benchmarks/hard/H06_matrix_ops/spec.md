# H06: Matrix Ops

## Problem

Implement 2x2 matrix operations with bounded inputs and non-negative result guarantees.

## Signatures

```vow
struct Mat2 { a: i64, b: i64, c: i64, d: i64 }
fn mat_new(a: i64, b: i64, c: i64, d: i64) -> Mat2
fn mat_add(m1: Mat2, m2: Mat2) -> Mat2
fn mat_trace(m: Mat2) -> i64
```

## Contracts

- `mat_new`: `requires: a >= 0, b >= 0, c >= 0, d >= 0, a <= 1000, b <= 1000, c <= 1000, d <= 1000`
- `mat_add`: `requires: m1.a >= 0, ... m2.a >= 0, ...` (all fields bounded `<= 1000`)
- `mat_trace`: `requires: m.a >= 0, m.d >= 0, m.a <= 1000, m.d <= 1000`, `ensures: result >= 0`

## Constraints

- 2x2 matrix stored as 4 fields (a=top-left, b=top-right, c=bottom-left, d=bottom-right)
- Bounds prevent overflow in addition

## Hints

- `mat_new` returns `Mat2 { a: a, b: b, c: c, d: d }`
- `mat_add` adds corresponding fields element-wise
- `mat_trace` returns `m.a + m.d` (sum of diagonal)
