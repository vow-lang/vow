# HE013: Greatest Common Divisor

**Origin:** HumanEval-013 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Implement the greatest common divisor (GCD) function for two positive integers.
The GCD is the largest positive integer that divides both numbers without
remainder. Use the Euclidean algorithm: GCD(a, b) = GCD(b, a % b) until one
operand becomes zero.

## Signature

```vow
fn gcd(a: i64, b: i64) -> i64
```

## Contracts

- `requires: a >= 1` — positive input
- `requires: b >= 1` — positive input
- `requires: a <= 50` — bounded for verification
- `requires: b <= 50` — bounded for verification
- `ensures: result >= 1` — GCD is positive
- `ensures: a % result == 0` — result divides a
- `ensures: b % result == 0` — result divides b

## Contract Fidelity

**PARTIAL** — the Dafny spec additionally requires that no larger integer
divides both a and b (maximality: `forall d :: d > 0 && divides(d, a) &&
divides(d, b) ==> d <= result`). Vow contracts cannot express universal
quantifiers, so only the divisibility properties are checked.

## Hints

- Euclidean algorithm: repeatedly replace `(a, b)` with `(b, a % b)` until `b == 0`
- The result is `a` when `b` reaches 0
