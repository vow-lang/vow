# M13: GCD

## Problem

Implement the Euclidean GCD algorithm with bounded inputs.

## Signature

```vow
fn gcd(a: i64, b: i64) -> i64
```

## Contracts

- `requires: a > 0` — `a` is positive
- `requires: b > 0` — `b` is positive
- `requires: a <= 8` — bounded for verification
- `requires: b <= 8` — bounded for verification
- `ensures: result > 0` — GCD is always positive

## Constraints

- Use the Euclidean algorithm with modulo
- Bounded inputs ensure termination within unwind limit

## Hints

- While `b > 0`: `tmp = b; b = a % b; a = tmp`
- After the loop, `a` is the GCD
- For inputs up to 8, the loop terminates within 4 iterations
