# E13: Fibonacci Bounded

## Problem

Implement a function `fib` that computes the n-th Fibonacci number using a loop.

## Signature

```vow
fn fib(n: i64) -> i64
```

## Contracts

- `requires: n >= 0` — index is non-negative
- `requires: n <= 8` — bounded to stay within unwind limit
- `ensures: result >= 0` — Fibonacci numbers are non-negative

## Constraints

- Use a while loop with two accumulators
- Include loop invariants

## Hints

- Use two variables `a = 0, b = 1`; in each iteration set `a, b = b, a + b`
- After `n` iterations, `a` holds fib(n)
- Loop invariants: `a >= 0`, `b >= 1`, `i >= 0`, `i <= n`
