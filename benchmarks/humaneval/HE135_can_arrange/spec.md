# HE135: Can Arrange

**Origin:** HumanEval-135 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

The task is to find the largest index in an array of distinct integers where an element is smaller than the element immediately before it. If no such index exists (i.e., the array is non-decreasing), return -1. The implementation should scan from right to left to efficiently find the largest such index.

## Signature

```vow
fn can_arrange(arr: Vec<i64>) -> i64
```

## Contracts

- `requires: arr.len() >= 0`
- `requires: arr.len() <= 8`
- `ensures: result >= 0`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method can_arrange(arr: seq<int>) returns (result: int)
    requires ValidInput(arr)
    ensures result == -1 || (0 < result < |arr|)
    ensures result == -1 ==> IsNonDecreasing(arr)
    ensures result != -1 ==> IsLargestDecreaseIndex(arr, result)
    ensures result != -1 ==> (exists i :: HasDecreaseAt(arr, i))
```

## Hints

- TODO: add implementation hints
