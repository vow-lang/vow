# HE100: Make A Pile

**Origin:** HumanEval-100 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task implements a method to create a pile of stones with n levels. The first level contains n stones, and each subsequent level contains the next number with the same parity (odd/even) as n. This creates an arithmetic sequence where each level has 2 more stones than the previous level.

## Signature

```vow
fn make_a_pile(n: i64) -> Vec<i64>
```

## Contracts

- `requires: n >= 0`
- `requires: n <= 8`
- `ensures: result.len() >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method make_a_pile(n: int) returns (pile: seq<int>)
    requires ValidInput(n)
    ensures ValidPile(pile, n)
```

## Hints

- TODO: add implementation hints
