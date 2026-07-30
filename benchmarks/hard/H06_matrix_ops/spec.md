# H06: Matrix Ops

## Problem

Implement scalar operations used by a 2x2 matrix library with exact overflow
guards and functional result contracts.

## Signatures

```vow
fn mat_trace(a: i64, d: i64) -> i64
fn mat_add_element(x: i64, y: i64) -> i64
fn mat_scale(x: i64, k: i64) -> i64
fn mat_determinant_2x2(a: i64, b: i64, c: i64, d: i64) -> i64
```

## Contracts

- `mat_trace`: non-negative inputs with `a <= 9223372036854775807 - d`; result equals `a + d`
- `mat_add_element`: non-negative inputs with `x <= 9223372036854775807 - y`; result equals `x + y`
- `mat_scale`: non-negative inputs with `k == 0 || x <= 9223372036854775807 / k`; result equals `x * k`
- `mat_determinant_2x2`: non-negative inputs whose two products fit in `i64`; result equals `a * d - b * c`

## Constraints

- Preconditions cover the complete non-negative domains where the operations do not overflow

## Hints

- Guard additions with subtraction from `i64::MAX`
- Guard multiplications with division by the non-zero factor
- The determinant is `a * d - b * c`
