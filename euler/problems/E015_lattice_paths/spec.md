# E015: Lattice Paths

## Problem (Project Euler #15)

How many routes are there through a 20x20 grid, starting at the top-left
corner and only being able to move right or down?

**Answer:** 137846528640

## Task

Implement `lattice_paths(n: i64) -> i64` that computes the number of
lattice paths through an n x n grid. This equals C(2n, n) — the central
binomial coefficient.

## Contracts

- `requires: n >= 0`
- `requires: n <= 30` (overflow guard)
- `ensures: result >= 1`

## Constraints

- Compute C(2n, n) iteratively: `result = result * (n + 1 + i) / (i + 1)` for i in 0..n
- Use a `while` loop with invariant: `result >= 1`
- Multiply before dividing at each step to maintain integer exactness
  (the intermediate product is always divisible)
- `main()` must call `lattice_paths(20)` and print the result

## Hints

- C(2n, n) = (2n)! / (n!)^2
- Iterative: start with 1, multiply by (n+1+i)/(i+1) for i = 0..n-1
- The result fits in i64 for n <= 30
