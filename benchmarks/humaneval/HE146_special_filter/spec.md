# HE146: Special Filter

**Origin:** HumanEval-146 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Count the numbers in an array that satisfy all three conditions: 1) Greater than 10, 2) First digit is odd (1, 3, 5, 7, 9), and 3) Last digit is odd (1, 3, 5, 7, 9).

## Signature

```vow
fn special_filter(nums: Vec<i64>) -> i64
```

## Contracts

- `requires: nums.len() >= 0`
- `requires: nums.len() <= 8`
- `ensures: result >= 0`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method SpecialFilter(nums: seq<int>) returns (count: int)
  requires ValidInput(nums)
  ensures count >= 0
  ensures count <= |nums|
  ensures count == |set i | 0 <= i < |nums| && SatisfiesCondition(nums[i])|
  ensures nums == [] ==> count == 0
  ensures forall i :: 0 <= i < |nums| && SatisfiesCondition(nums[i]) ==> nums[i] > 10 && IsOdd(FirstDigit(nums[i])) && IsOdd(LastDigit(nums[i]))
```

## Hints

- TODO: add implementation hints
