# HE122: Add Elements

**Origin:** HumanEval-122 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task involves computing the sum of all elements that have at most two digits among the first k elements of an array. An element has at most two digits if its absolute value is between 0 and 99 (inclusive).

## Signature

```vow
fn add_elements(arr: Vec<i64>, k: i64) -> i64
```

## Contracts

- `requires: arr.len() >= 0`
- `requires: arr.len() <= 8`
- `requires: k >= 0`
- `requires: k <= 100`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method add_elements(arr: seq<int>, k: int) returns (result: int)
  requires ValidInput(arr, k)
  ensures result == sum_valid_elements(arr, k)
```

## Hints

- TODO: add implementation hints
