# HE139: Special Factorial

**Origin:** HumanEval-139 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Compute the special factorial of a positive integer n, defined as the product of all factorials from 1! to n!: special_factorial(n) = n! × (n-1)! × (n-2)! × ... × 1!. The implementation should use an iterative approach with proper loop invariants to ensure correctness.

## Signature

```vow
fn special_factorial(n: i64) -> i64
```

## Contracts

- `requires: n >= 0`
- `requires: n <= 8`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method special_factorial(n: int) returns (result: int)
    requires n >= 0
    ensures result == special_factorial_func(n)
    ensures result > 0
```

## Hints

- TODO: add implementation hints
