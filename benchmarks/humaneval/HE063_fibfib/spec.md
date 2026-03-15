# HE063: Fibfib

**Origin:** HumanEval-063 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task involves implementing the FibFib sequence, which is a modified Fibonacci sequence. The FibFib sequence is defined with base cases fibfib(0) = 0, fibfib(1) = 0, fibfib(2) = 1, and for n >= 3, fibfib(n) = fibfib(n-1) + fibfib(n-2) + fibfib(n-3). The expected implementation should efficiently compute the n-th element using dynamic programming rather than naive recursion.

## Signature

```vow
fn fibfib(n: i64) -> i64
```

## Contracts

- `requires: n >= 0`
- `requires: n <= 10`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method fibfib(n: int) returns (result: int)
    requires n >= 0
    ensures result == fibfib_spec(n)
    ensures n == 0 ==> result == 0
    ensures n == 1 ==> result == 0
    ensures n == 2 ==> result == 1
    ensures n >= 3 ==> result == fibfib_spec(n-1) + fibfib_spec(n-2) + fibfib_spec(n-3)
```

## Hints

- TODO: add implementation hints
