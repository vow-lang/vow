# HE121: Solution

**Origin:** HumanEval-121 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This task requires implementing a method that calculates the sum of all odd numbers located at even-indexed positions in a non-empty sequence of integers. The positions are 0-indexed, so we consider positions 0, 2, 4, etc.

## Signature

```vow
fn solution(lst: Vec<i64>) -> i64
```

## Contracts

- `requires: lst.len() >= 0`
- `requires: lst.len() <= 8`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method solution(lst: seq<int>) returns (result: int)
    requires |lst| > 0
    ensures result == SumOddAtEvenPositions(lst, 0)
```

## Hints

- TODO: add implementation hints
