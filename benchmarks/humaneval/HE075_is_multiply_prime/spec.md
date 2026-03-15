# HE075: Is Multiply Prime

**Origin:** HumanEval-075 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task implements a method to determine if a given integer (less than 100) is the product of exactly 3 prime numbers, counting repetitions. The method performs prime factorization and checks if exactly 3 prime factors (with repetitions) multiply to the original number.

The expected implementation uses trial division to find all prime factors, starting with factors of 2, then checking odd numbers up to the square root, and finally handling any remaining prime factor greater than the square root.

## Signature

```vow
fn is_multiply_prime(a: i64) -> i64
```

## Contracts

- `requires: a >= 0`
- `requires: a <= 100`
- `ensures: result >= 0`
- `ensures: result <= 1`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method is_multiply_prime(a: int) returns (result: bool)
  requires a >= 0 && a < 100
  ensures a < 8 ==> result == false
  ensures result == true <==> (exists p1: int, p2: int, p3: int :: 
    p1 >= 2 && p2 >= 2 && p3 >= 2 && 
    is_prime_number(p1) && is_prime_number(p2) && is_prime_number(p3) &&
    a == p1 * p2 * p3)
```

## Hints

- TODO: add implementation hints
