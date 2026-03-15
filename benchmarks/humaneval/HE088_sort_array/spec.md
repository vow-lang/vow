# HE088: Sort Array

**Origin:** HumanEval-088 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task implements a conditional sorting algorithm for arrays of non-negative integers. The sorting order is determined by the sum of the first and last elements: if the sum is odd, the array is sorted in ascending order; if the sum is even, it's sorted in descending order. The implementation must return a sorted copy without modifying the original array and preserve all elements (multiset equality).

## Signature

```vow
fn sort_array(arr: Vec<i64>) -> Vec<i64>
```

## Contracts

- `requires: arr.len() >= 0`
- `requires: arr.len() <= 8`
- `ensures: result.len() >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method sort_array(arr: seq<int>) returns (result: seq<int>)
    requires ValidInput(arr)
    ensures multiset(result) == multiset(arr)
    ensures CorrectlySorted(arr, result)
```

## Hints

- TODO: add implementation hints
