# E014: Longest Collatz Sequence

## Problem (Project Euler #14)

Which starting number, under one million, produces the longest Collatz sequence?

**Answer:** 837799

## Task

Implement:

1. `collatz_length(n: i64) -> i64` — returns the length of the Collatz
   sequence starting at `n` (counting the starting number)
2. `longest_collatz(limit: i64) -> i64` — returns the starting number below
   `limit` that produces the longest Collatz sequence

## Contracts

- `collatz_length`: `requires: n >= 1`, `ensures: result >= 1`
- `longest_collatz`: `requires: limit >= 2`, `ensures: result >= 1`,
  `ensures: result < limit`

## Constraints

- Collatz rule: if even, `n / 2`; if odd, `3 * n + 1`
- `collatz_length` uses a `while` loop until `n == 1`
- Loop invariant: `n >= 1` (Collatz conjecture: always positive)
- `longest_collatz` iterates candidates and tracks the best
- `main()` must call `longest_collatz(1000000)` and print the result

## Hints

- The sequence for 13: 13 → 40 → 20 → 10 → 5 → 16 → 8 → 4 → 2 → 1 (length 10)
- Track `best_start` and `best_len` in the outer loop
