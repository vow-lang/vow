# HE116: Sort Array

**Origin:** HumanEval-116 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: def max_fill_count(grid : list[list[int]], capacity : int) -> int
Please write a function that sorts an array of non-negative integers according to number of ones in their binary representation in ascending order. For similar number of ones, sort based on decimal value.

## Signature

```vow
fn sort_array(s: Vec<i64>) -> Vec<i64>
```

## Contracts

- `requires: s.len() >= 0`
- `requires: s.len() <= 8`
- `ensures: result.len() >= 0`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method sort_array(s: seq<nat>) returns (sorted: seq<nat>)

  ensures forall i, j :: 0 <= i < j < |sorted| ==> popcount(sorted[i]) <= popcount(sorted[j])
  ensures |sorted| == |s|
  ensures multiset(s) == multiset(sorted)
```

## Hints

- TODO: add implementation hints
