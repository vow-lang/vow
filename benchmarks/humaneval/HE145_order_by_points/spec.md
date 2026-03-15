# HE145: Order By Points

**Origin:** HumanEval-145 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: def order_by_points(nums: List[int]) -> List[int]
Write a function which sorts the given list of integers in ascending order according to the sum of their digits. Note: if there are several items with similar sum of their digits, order them based on their index in original list.

## Signature

```vow
fn order_by_points(s: Vec<i64>) -> Vec<i64>
```

## Contracts

- `requires: s.len() >= 0`
- `requires: s.len() <= 8`
- `ensures: result.len() >= 0`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method order_by_points(s: seq<int>) returns (sorted: seq<int>)

  ensures forall i, j :: 0 <= i < j < |sorted| ==> digits_sum(sorted[i]) <= digits_sum(sorted[j])
  ensures |sorted| == |s|
  ensures multiset(s) == multiset(sorted)
```

## Hints

- TODO: add implementation hints
