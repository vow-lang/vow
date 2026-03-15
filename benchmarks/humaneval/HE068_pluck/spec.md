# HE068: Pluck

**Origin:** HumanEval-068 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task implements a function that finds the smallest even value in an array of non-negative integers and returns it along with its index. If multiple occurrences of the same smallest even value exist, it should return the one with the smallest index. If no even values exist or the array is empty, it returns an empty list.

The implementation must correctly handle edge cases and maintain loop invariants to prove that the returned result satisfies all the postconditions, including finding the true minimum even value and the earliest index for that value.

## Signature

```vow
fn pluck(arr: Vec<i64>) -> Vec<i64>
```

## Contracts

- `requires: arr.len() >= 0`
- `requires: arr.len() <= 8`
- `ensures: result.len() >= 0`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method pluck(arr: seq<int>) returns (result: seq<int>)
    requires ValidInput(arr)
    ensures |arr| == 0 ==> |result| == 0
    ensures !HasEvenValue(arr) ==> |result| == 0
    ensures HasEvenValue(arr) ==> |result| == 2
    ensures |result| == 2 ==> 0 <= result[1] < |arr|
    ensures |result| == 2 ==> arr[result[1]] == result[0]
    ensures |result| == 2 ==> result[0] % 2 == 0
    ensures |result| == 2 ==> forall i :: 0 <= i < |arr| && arr[i] % 2 == 0 ==> result[0] <= arr[i]
    ensures |result| == 2 ==> forall i :: 0 <= i < |arr| && arr[i] % 2 == 0 && arr[i] == result[0] ==> result[1] <= i
```

## Hints

- TODO: add implementation hints
