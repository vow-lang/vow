# HE073: Smallest Change

**Origin:** HumanEval-073 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task involves finding the minimum number of elements that must be changed to make an array palindromic. A palindromic array reads the same forwards and backwards. The solution should count the number of mismatched pairs between corresponding positions from the start and end of the array.

## Signature

```vow
fn smallest_change(arr: Vec<i64>) -> i64
```

## Contracts

- `requires: arr.len() >= 0`
- `requires: arr.len() <= 8`
- `ensures: result >= 0`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method smallest_change(arr: seq<int>) returns (changes: int)
    ensures changes >= 0
    ensures changes <= |arr| / 2
    ensures changes == count_mismatched_pairs(arr)
    ensures (|arr| <= 1) ==> (changes == 0)
    ensures forall c :: 0 <= c < changes ==> !can_make_palindromic_with_changes(arr, c)
    ensures can_make_palindromic_with_changes(arr, changes)
```

## Hints

- TODO: add implementation hints
