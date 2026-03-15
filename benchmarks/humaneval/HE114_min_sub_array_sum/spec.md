# HE114: Min Sub Array Sum

**Origin:** HumanEval-114 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: def minSubArraySum(nums : list[int]) -> int
Given an array of integers nums, find the minimum sum of any non-empty sub-array of nums.

## Signature

```vow
fn min_sub_array_sum(a: Vec<i64>) -> i64
```

## Contracts

- `requires: a.len() >= 0`
- `requires: a.len() <= 8`
- `ensures: result >= 0`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method minSubArraySum(a: seq<int>) returns (s: int)

  ensures forall p,q :: 0 <= p <= q <= |a| ==> Sum(a, p, q) >= s
  ensures exists k, m :: 0 <= k <= m <= |a| && s == Sum(a, k, m)
```

## Hints

- TODO: add implementation hints
