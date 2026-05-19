# E002: Even Fibonacci Numbers

## Problem (Project Euler #2)

Find the sum of the even-valued Fibonacci terms whose values do not exceed
four million.

**Answer:** 4613732

## Task

Implement `even_fib_sum(limit: i64) -> i64` that returns the sum of all
even Fibonacci numbers not exceeding `limit`.

## Contracts

- `requires: limit >= 2` — must include at least F(3) = 2
- `ensures: result >= 2` — at minimum the first even Fibonacci (2) is included

## Constraints

- Use a `while` loop to generate Fibonacci numbers
- Loop invariant: both Fibonacci state variables are positive
- Stop when the current Fibonacci number exceeds `limit`
- `main()` must call `even_fib_sum(4000000)` and print the result

## Hints

- Track two consecutive Fibonacci numbers `a` and `b`
- On each iteration: `let next = a + b; a = b; b = next;`
- Accumulate `a` when `a % 2 == 0`
