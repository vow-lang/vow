# E007: 10001st Prime

## Problem (Project Euler #7)

What is the 10001st prime number?

**Answer:** 104743

## Task

Implement:

1. `is_divisible(n: i64, d: i64) -> bool` — returns whether `n % d == 0`
2. `is_prime(n: i64) -> bool` — trial division primality test
3. `nth_prime(n: i64) -> i64` — returns the n-th prime (1-indexed)

## Contracts

- `is_divisible`: `requires: d > 0`
- `is_prime`: `requires: n >= 2`
- `nth_prime`: `requires: n >= 1`, `ensures: result >= 2`

## Constraints

- `is_prime` must use a `while` loop checking divisors up to sqrt(n)
- `nth_prime` must use a `while` loop counting primes
- `main()` must call `nth_prime(10001)` and print the result

## Hints

- For `is_prime`: check divisors `d` from 2 while `d * d <= n`
- For `nth_prime`: start at candidate 2, increment, count primes found
