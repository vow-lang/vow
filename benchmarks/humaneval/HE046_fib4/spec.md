# HE046: Fib4

**Origin:** HumanEval-046 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This task implements an iterative solution to compute the n-th element of the Fib4 sequence. The Fib4 sequence is defined with base cases fib4(0)=0, fib4(1)=0, fib4(2)=2, fib4(3)=0, and for n≥4, fib4(n) = fib4(n-1) + fib4(n-2) + fib4(n-3) + fib4(n-4).

The implementation must be iterative and efficient, using a sliding window approach to maintain the last 4 values instead of recursion, while proving equivalence to the recursive specification.

## Signature

```vow
fn fib4(n: i64) -> i64
```

## Contracts

- `requires: n >= 0`
- `requires: n <= 10`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method fib4(n: int) returns (result: int)
    requires n >= 0
    ensures result == fib4_func(n)
    ensures n == 0 ==> result == 0
    ensures n == 1 ==> result == 0
    ensures n == 2 ==> result == 2
    ensures n == 3 ==> result == 0
    ensures n >= 4 ==> result == fib4_func(n-1) + fib4_func(n-2) + fib4_func(n-3) + fib4_func(n-4)
```

## Hints

- TODO: add implementation hints
