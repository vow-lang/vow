# HE024: Largest Divisor

**Origin:** HumanEval-024 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: def largest_divisor(n: int) -> int
For a given number n, find the largest number that divides n evenly, smaller than n

## Signature

```vow
fn largest_divisor(n: i64) -> i64
```

## Contracts

- `requires: n >= 0`
- `requires: n <= 10`
- `ensures: result >= 0`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method largest_divisor(n: int) returns (d : int)

  requires n > 1

  ensures 1 <= d < n
  ensures n % d == 0
  ensures forall k :: d < k < n ==> n % k != 0
```

## Hints

- TODO: add implementation hints
