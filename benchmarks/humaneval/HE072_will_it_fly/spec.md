# HE072: Will It Fly

**Origin:** HumanEval-072 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This task determines if an object will fly based on two conditions: the given list of numbers must be palindromic, and the sum of all elements must be within a specified weight limit. The implementation needs to check both conditions and return true only if both are satisfied.

## Signature

```vow
fn will_it_fly(q: Vec<i64>, w: i64) -> i64
```

## Contracts

- `requires: q.len() >= 0`
- `requires: q.len() <= 8`
- `requires: w >= 0`
- `requires: w <= 100`
- `ensures: result >= 0`
- `ensures: result <= 1`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method will_it_fly(q: seq<int>, w: int) returns (result: bool)
    ensures result == (is_palindrome(q) && sum_elements(q) <= w)
```

## Hints

- TODO: add implementation hints
