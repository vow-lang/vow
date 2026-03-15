# HE034: Sort Seq

**Origin:** HumanEval-034 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: method SortSeq(s: seq<int>) returns (sorted: seq<int>)
Sort elements. Ensures: the result is sorted according to the ordering relation; returns the correct size/count; returns a sorted permutation of the input.

## Signature

```vow
fn sort_seq(s: Vec<i64>) -> Vec<i64>
```

## Contracts

- `requires: s.len() >= 0`
- `requires: s.len() <= 8`
- `ensures: result.len() >= 0`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method SortSeq(s: seq<int>) returns (sorted: seq<int>)

  ensures forall i, j :: 0 <= i < j < |sorted| ==> sorted[i] <= sorted[j]
  ensures |sorted| == |s|
  ensures multiset(s) == multiset(sorted)
```

## Hints

- TODO: add implementation hints
