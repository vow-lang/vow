# E13: Fibonacci

## Problem

Implement a function `fib` that computes the n-th Fibonacci number using a loop.

## Signature

```vow
fn fib(n: u64) -> i64
```

## Contracts

- `n` is a `u64` index, so the type already says it is non-negative; no `requires: n >= 0` is needed
- `requires: n <= 92` — exact largest Fibonacci index whose result fits in `i64`
- `ensures: result >= 0` — Fibonacci numbers are non-negative

## Constraints

- Handle `n == 0`, then use a while loop with two accumulators
- Include loop invariants

## Hints

- Use `prev = 0, curr = 1`; each iteration advances both values
- Start at index 1 so computing `fib(92)` never computes the overflowing `fib(93)`
- Loop invariants: `prev >= 0`, `curr >= 1`, `i >= 1`, `i <= n`
- The upper bound prevents arithmetic overflow; it is not an unwind bound
