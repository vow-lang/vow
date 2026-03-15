# HE120: Sort Seq

**Origin:** HumanEval-120 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: method SortSeq(s: seq<int>) returns (sorted: seq<int>)
Sort elements. Ensures: the result is sorted according to the ordering relation; returns the correct size/count; returns a sorted permutation of the input; the result is sorted according to the ordering relation; the result is sorted according to the ordering relation; the result is sorted according to the ordering relation; the result is sorted according to the ordering relation.

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
  ensures forall i :: 0 <= i < |s| ==> exists j :: 0 <= j < |sorted| && s[i] == sorted[j]
  ensures forall x :: x in s ==> x in sorted
  ensures forall i :: 0 <= i < |s| ==> exists j :: 0 <= j < |sorted| && sorted[i] == s[j]
  ensures forall x :: x in sorted ==> x in s
```

## Hints

- TODO: add implementation hints
