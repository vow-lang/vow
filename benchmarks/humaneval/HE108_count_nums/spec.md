# HE108: Count Nums

**Origin:** HumanEval-108 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Given an array of integers, count how many elements have a positive sum of digits. For digit sum calculation: positive numbers sum all digits normally, negative numbers have the first digit as negative and remaining digits as positive, and zero has digit sum 0. Return the count of numbers whose digit sum is greater than 0.

## Signature

```vow
fn count_nums(arr: Vec<i64>) -> i64
```

## Contracts

- `requires: arr.len() >= 0`
- `requires: arr.len() <= 8`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method count_nums(arr: seq<int>) returns (count: int)
    requires ValidInput(arr)
    ensures ValidOutput(arr, count)
```

## Hints

- TODO: add implementation hints
