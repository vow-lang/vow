# HE102: Choose Num

**Origin:** HumanEval-102 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This task involves implementing a method to find the largest even integer in a given range [x, y] inclusive, where x and y are positive integers. If no even integer exists in the range, the method should return -1.

## Signature

```vow
fn choose_num(x: i64, y: i64) -> i64
```

## Contracts

- `requires: x >= 0`
- `requires: x <= 100`
- `requires: y >= 0`
- `requires: y <= 100`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method ChooseNum(x: int, y: int) returns (result: int)
    requires ValidInput(x, y)
    ensures CorrectResult(x, y, result)
```

## Hints

- TODO: add implementation hints
