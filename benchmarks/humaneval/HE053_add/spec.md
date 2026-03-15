# HE053: Add

**Origin:** HumanEval-053 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task implements a simple addition function that takes two integers as input and returns their sum. The implementation should correctly add the two input integers and satisfy the postcondition that the result equals the mathematical sum of the inputs.

## Signature

```vow
fn add(x: i64, y: i64) -> i64
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
method add(x: int, y: int) returns (result: int)
    requires ValidInput(x, y)
    ensures result == CorrectSum(x, y)
```

## Hints

- TODO: add implementation hints
