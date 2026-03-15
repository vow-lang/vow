# HE083: Starts One Ends

**Origin:** HumanEval-083 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Count n-digit positive integers that start with 1 OR end with 1 using inclusion-exclusion principle. The task requires implementing a function that uses the inclusion-exclusion principle to count numbers that either start with 1, end with 1, or both, avoiding double-counting those that satisfy both conditions.

## Signature

```vow
fn starts_one_ends(n: i64) -> i64
```

## Contracts

- `requires: n >= 1`
- `requires: n <= 8`
- `ensures: result >= 1`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method starts_one_ends(n: int) returns (result: int)
  requires ValidInput(n)
  ensures result == CountStartsWith1(n) + CountEndsWith1(n) - CountStartsAndEndsWith1(n)
  ensures result >= 0
```

## Hints

- TODO: add implementation hints
