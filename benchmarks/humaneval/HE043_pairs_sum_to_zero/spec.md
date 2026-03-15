# HE043: Pairs Sum To Zero

**Origin:** HumanEval-043 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Given a list of integers, determine if there exist two distinct elements at different positions that sum to zero. This task requires implementing an efficient algorithm to check for the existence of such a pair.

## Signature

```vow
fn pairs_sum_to_zero(l: Vec<i64>) -> i64
```

## Contracts

- `requires: l.len() >= 0`
- `requires: l.len() <= 8`
- `ensures: result >= 0`
- `ensures: result <= 1`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method pairs_sum_to_zero(l: seq<int>) returns (result: bool)
    ensures result == HasPairSumToZero(l)
```

## Hints

- TODO: add implementation hints
