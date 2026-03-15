# HE055: Fib

**Origin:** HumanEval-055 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This task involves computing the n-th Fibonacci number using 1-based indexing, where fib(1) = 1 and fib(2) = 1. The implementation should efficiently calculate the result for positive integers n.

The solution uses an iterative approach with loop invariants to maintain correctness while avoiding the exponential time complexity of a naive recursive implementation.

## Signature

```vow
fn fib(n: i64) -> i64
```

## Contracts

- `requires: n >= 0`
- `requires: n <= 10`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method fib(n: int) returns (result: int)
    requires ValidInput(n)
    ensures result == fib_spec(n)
    ensures result > 0
```

## Hints

- TODO: add implementation hints
