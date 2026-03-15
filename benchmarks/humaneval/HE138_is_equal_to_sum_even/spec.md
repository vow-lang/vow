# HE138: Is Equal To Sum Even

**Origin:** HumanEval-138 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task determines whether a given positive integer n can be expressed as the sum of exactly 4 positive even numbers. The key insight is that the minimum sum is 8 (2+2+2+2), and only even numbers can be expressed this way since the sum of 4 even numbers is always even.

## Signature

```vow
fn is_equal_to_sum_even(n: i64) -> i64
```

## Contracts

- `requires: n >= 0`
- `requires: n <= 100`
- `ensures: result >= 0`
- `ensures: result <= 1`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method is_equal_to_sum_even(n: int) returns (result: bool)
    requires ValidInput(n)
    ensures result == CanBeSumOfFourPositiveEvens(n)
```

## Hints

- TODO: add implementation hints
