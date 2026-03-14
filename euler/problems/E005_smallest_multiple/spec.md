# E005: Smallest Multiple

## Problem (Project Euler #5)

What is the smallest positive number that is evenly divisible by all of the
numbers from 1 to 20?

**Answer:** 232792560

## Task

Implement:

1. `gcd(a: i64, b: i64) -> i64` — greatest common divisor via Euclid's algorithm
2. `lcm(a: i64, b: i64) -> i64` — least common multiple using `a / gcd(a, b) * b`
3. `smallest_multiple(n: i64) -> i64` — LCM of all numbers from 1 to n

## Contracts

- `gcd`: `requires: a > 0`, `requires: b > 0`, `ensures: result > 0`
- `lcm`: `requires: a > 0`, `requires: b > 0`, `ensures: result > 0`
- `smallest_multiple`: `requires: n >= 1`, `ensures: result >= 1`

## Constraints

- `gcd` must use a `while` loop (Euclidean algorithm)
- Loop invariant for `gcd`: both operands remain positive
- `main()` must call `smallest_multiple(20)` and print the result

## Hints

- Euclid: `while b != 0 { let t = b; b = a % b; a = t; }`
- LCM: compute `a / gcd(a, b) * b` (divide first to avoid overflow)
- Fold LCM across 1..n
