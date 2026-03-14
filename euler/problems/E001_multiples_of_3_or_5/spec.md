# E001: Multiples of 3 or 5

## Problem (Project Euler #1)

Find the sum of all multiples of 3 or 5 below 1000.

**Answer:** 233168

## Task

Implement `sum_multiples(limit: i64) -> i64` that returns the sum of all
natural numbers below `limit` that are multiples of 3 or 5.

## Contracts

- `requires: limit >= 0` — limit must be non-negative
- `ensures: result >= 0` — sum of positive numbers is non-negative

## Constraints

- Use a `while` loop with a loop invariant tracking the accumulator
- The loop invariant must state that the running sum is non-negative
- `main()` must call `sum_multiples(1000)` and print the result

## Hints

- Iterate `i` from 0 to `limit - 1`
- Check `i % 3 == 0 || i % 5 == 0`
- Accumulate into a mutable sum variable
