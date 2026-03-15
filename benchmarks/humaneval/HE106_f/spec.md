# HE106: F

**Origin:** HumanEval-106 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This task requires implementing a method that generates a sequence of natural numbers based on position-dependent calculations. For each position i (0-indexed), if (i+1) is even, the element should be the factorial of (i+1); if (i+1) is odd, the element should be the sum of integers from 1 to (i+1).

## Signature

```vow
fn f(n: i64) -> Vec<i64>
```

## Contracts

- `requires: n >= 0`
- `requires: n <= 8`
- `ensures: result.len() >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method f(n: nat) returns (result: seq<nat>)
    ensures ValidResult(n, result)
```

## Hints

- TODO: add implementation hints
