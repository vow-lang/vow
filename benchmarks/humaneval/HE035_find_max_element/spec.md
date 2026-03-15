# HE035: Find Max Element

**Origin:** HumanEval-035 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Find and return the maximum element in a given list of numbers. The list must be non-empty, and the maximum element is the largest value present in the list, which must be an actual element of the list.

## Signature

```vow
fn find_max_element(l: Vec<i64>) -> i64
```

## Contracts

- `requires: l.len() >= 0`
- `requires: l.len() <= 8`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method FindMaxElement(l: seq<int>) returns (max_val: int)
    requires ValidInput(l)
    ensures IsMaxElement(l, max_val)
```

## Hints

- TODO: add implementation hints
