# E006: Sum Square Difference

## Problem (Project Euler #6)

Find the difference between the sum of the squares and the square of the sum
of the first 100 natural numbers.

**Answer:** 25164150

## Task

Implement three functions:

1. `sum_of_squares(n: i64) -> i64` — returns 1^2 + 2^2 + ... + n^2
2. `square_of_sum(n: i64) -> i64` — returns (1 + 2 + ... + n)^2
3. `difference(n: i64) -> i64` — returns `square_of_sum(n) - sum_of_squares(n)`

## Contracts

- `sum_of_squares`: `requires: n >= 0`, `ensures: result >= 0`
- `square_of_sum`: `requires: n >= 0`, `ensures: result >= 0`
- `difference`: `requires: n >= 1`, `ensures: result >= 0`

## Constraints

- Use `while` loops (no closed-form formulas — the point is loop verification)
- Loop invariants must track that accumulators are non-negative
- `main()` must call `difference(100)` and print the result

## Hints

- For `sum_of_squares`: accumulate `i * i` in a loop from 1 to n
- For `square_of_sum`: first sum 1..n, then square the result
- The difference is always non-negative for n >= 1
