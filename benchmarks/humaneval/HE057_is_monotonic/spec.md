# HE057: Is Monotonic

**Origin:** HumanEval-057 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task implements a function to determine if a list of numbers is monotonic. A list is monotonic if it is either entirely non-decreasing (monotonically increasing) or entirely non-increasing (monotonically decreasing). Empty lists and single-element lists are considered monotonic, and lists with equal consecutive elements are allowed.

## Signature

```vow
fn is_monotonic(l: Vec<i64>) -> i64
```

## Contracts

- `requires: l.len() >= 0`
- `requires: l.len() <= 8`
- `ensures: result >= 0`
- `ensures: result <= 1`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method IsMonotonic(l: seq<int>) returns (result: bool)
    ensures result == monotonic(l)
```

## Hints

- TODO: add implementation hints
