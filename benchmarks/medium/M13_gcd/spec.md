# M13: GCD

## Problem

Implement the Euclidean GCD algorithm for positive inputs.

## Signature

```vow
fn gcd(a: i64, b: i64) -> i64
```

## Contracts

- `requires: a > 0` — `a` is positive
- `requires: b > 0` — `b` is positive
- `ensures: result > 0` — GCD is always positive

## Constraints

- Use the Euclidean algorithm with modulo
- The mathematical contract is independent of verifier unwind limits

## Hints

- While `b > 0`: `tmp = b; b = a % b; a = tmp`
- After the loop, `a` is the GCD
