# HE126: Check Valid List

**Origin:** HumanEval-126 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task checks if a list of non-negative integers satisfies two conditions: (1) the list is sorted in non-decreasing (ascending) order, and (2) no number appears more than twice in the list. The implementation uses helper functions to check these conditions efficiently and returns true if both are met, false otherwise.

## Signature

```vow
fn check_valid_list(lst: Vec<i64>) -> i64
```

## Contracts

- `requires: lst.len() >= 0`
- `requires: lst.len() <= 8`
- `ensures: result >= 0`
- `ensures: result <= 1`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method CheckValidList(lst: seq<int>) returns (result: bool)
    requires ValidInput(lst)
    ensures result == ValidList(lst)
    ensures result == (IsSortedAscending(lst) && NoMoreThanTwoDuplicates(lst))
```

## Hints

- TODO: add implementation hints
