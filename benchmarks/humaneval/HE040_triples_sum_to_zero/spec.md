# HE040: Triples Sum To Zero

**Origin:** HumanEval-040 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task involves implementing a method to determine if there exist three distinct elements at different positions in a list of integers that sum to zero. The implementation should exhaustively check all possible combinations of three indices and return true if any triple sums to zero.

## Signature

```vow
fn triples_sum_to_zero(l: Vec<i64>) -> i64
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
method triples_sum_to_zero(l: seq<int>) returns (result: bool)
    ensures result == HasTripleSumToZero(l)
```

## Hints

- TODO: add implementation hints
