# HE031: Is Prime

**Origin:** HumanEval-031 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Implement a function to determine if a positive integer is a prime number. A
prime number is greater than 1 and has no divisors other than 1 and itself.
Return 1 if the number is prime, 0 otherwise.

A spec function `is_prime_check` is provided that performs trial division. Your
implementation must produce the same result for all inputs in range.

## Signature

```vow
fn is_prime(n: i64) -> i64
```

## Contracts

- `requires: n >= 0` — non-negative input
- `requires: n <= 100` — bounded for verification
- `ensures: result == is_prime_check(n)` — matches spec function

## Contract Fidelity

**EXACT** — the spec function `is_prime_check` implements trial division
(equivalent to Dafny's `is_prime_number` predicate). The ensures clause
verifies functional equivalence via `result == is_prime_check(n)`.

## Hints

- Check divisibility from 2 up to the square root of n
- Numbers less than 2 are not prime
- Use a while loop with an invariant tracking the loop counter bounds
