# HE109: Move One Ball

**Origin:** HumanEval-109 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Given an array of unique integers, determine if it's possible to sort the array in non-decreasing order using only right shift operations. A right shift moves all elements one position to the right, with the last element moving to the first. The method should return True if the array is sortable via rotations, False otherwise, with empty arrays returning True.

## Signature

```vow
fn move_one_ball(arr: Vec<i64>) -> i64
```

## Contracts

- `requires: arr.len() >= 0`
- `requires: arr.len() <= 8`
- `ensures: result >= 0`
- `ensures: result <= 1`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method move_one_ball(arr: seq<int>) returns (result: bool)
    requires forall i, j :: 0 <= i < j < |arr| ==> arr[i] != arr[j]
    ensures |arr| == 0 ==> result == true
    ensures result == true ==> (|arr| == 0 || exists k :: 0 <= k < |arr| && is_sorted(rotate_right(arr, k)))
    ensures result == false ==> forall k :: 0 <= k < |arr| ==> !is_sorted(rotate_right(arr, k))
```

## Hints

- TODO: add implementation hints
