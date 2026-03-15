# HE052: Check Below Threshold

**Origin:** HumanEval-052 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task implements a method to check if all integers in a given list are strictly less than a specified threshold value. The method should return true if and only if every element in the sequence satisfies the threshold condition.

## Signature

```vow
fn check_below_threshold(l: Vec<i64>, t: i64) -> i64
```

## Contracts

- `requires: l.len() >= 0`
- `requires: l.len() <= 8`
- `requires: t >= 0`
- `requires: t <= 100`
- `ensures: result >= 0`
- `ensures: result <= 1`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method CheckBelowThreshold(l: seq<int>, t: int) returns (result: bool)
    ensures result == BelowThreshold(l, t)
```

## Hints

- TODO: add implementation hints
