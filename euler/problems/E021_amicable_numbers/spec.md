# E021: Amicable Numbers

## Problem (Project Euler #21)

Evaluate the sum of all amicable numbers under 10000.

Two numbers a and b are amicable if:
- `d(a) == b` and `d(b) == a`, where `a != b`
- `d(n)` is the sum of proper divisors of n

**Answer:** 31626

## Task

Implement:

1. `sum_divisors(n: i64) -> i64` — returns the sum of proper divisors of n
   (all divisors less than n)
2. `sum_amicable(limit: i64) -> i64` — returns the sum of all amicable numbers
   below `limit`

## Contracts

- `sum_divisors`: `requires: n >= 1`, `ensures: result >= 0`
- `sum_amicable`: `requires: limit >= 1`, `ensures: result >= 0`

## Constraints

- `sum_divisors` iterates from 1 to n-1 checking `n % i == 0`
- `sum_amicable` iterates from 2 to limit-1, checking the amicable condition
- For each `a`, compute `b = sum_divisors(a)`, then check
  `sum_divisors(b) == a && b != a`
- `main()` must call `sum_amicable(10000)` and print the result

## Hints

- First amicable pair: (220, 284)
- `sum_divisors(220) = 284`, `sum_divisors(284) = 220`
- Optimize `sum_divisors` by only checking up to sqrt(n) if desired
