# HE142: Sum Squares

**Origin:** HumanEval-142 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Transform each element in a list of integers based on its index position: square elements at indices that are multiples of 3, cube elements at indices that are multiples of 4 but not 3, and leave other elements unchanged. Return the sum of all transformed elements.

## Signature

```vow
fn sum_squares(lst: Vec<i64>) -> i64
```

## Contracts

- `requires: lst.len() >= 0`
- `requires: lst.len() <= 8`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method sum_squares(lst: seq<int>) returns (result: int)
    ensures result == sum_transformed(lst)
```

## Hints

- TODO: add implementation hints
