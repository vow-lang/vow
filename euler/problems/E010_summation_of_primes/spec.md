# E010: Summation of Primes

## Problem (Project Euler #10)

Find the sum of all primes below two million.

**Answer:** 142913828922

## Task

Implement:

1. `is_prime(n: i64) -> bool` — trial division primality test
2. `sum_primes(limit: i64) -> i64` — sum of all primes below `limit`

## Contracts

- `is_prime`: `requires: n >= 2`
- `sum_primes`: `requires: limit >= 2`, `ensures: result >= 2`

## Constraints

- `is_prime` uses trial division with a `while` loop
- `sum_primes` iterates from 2 to `limit - 1`, accumulating primes
- Loop invariant: accumulator is non-negative
- `main()` must call `sum_primes(2000000)` and print the result

## Hints

- Reuse the `is_prime` pattern from E007
- The sum is large (>142 billion) but fits in i64
