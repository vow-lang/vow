# HE097: Multiply

**Origin:** HumanEval-097 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This task implements a function to compute the product of the unit digits of two integers. The unit digit is defined as the ones place digit of the absolute value of a number. Given two integers (which can be positive, negative, or zero), the method should return the product of their respective unit digits.

## Signature

```vow
fn multiply(a: i64, b: i64) -> i64
```

## Contracts

- `requires: a >= 0`
- `requires: a <= 100`
- `requires: b >= 0`
- `requires: b <= 100`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method multiply(a: int, b: int) returns (result: int)
    ensures result == ProductOfUnitDigits(a, b)
    ensures ValidResult(result)
```

## Hints

- TODO: add implementation hints
