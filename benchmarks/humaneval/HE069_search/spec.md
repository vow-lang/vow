# HE069: Search

**Origin:** HumanEval-069 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task involves finding the greatest integer in a list whose frequency is greater than or equal to its own value. Given a non-empty list of positive integers, the implementation should return this greatest qualifying integer, or -1 if no such integer exists.

The task requires building a frequency map for all elements in the list, then identifying which elements have frequencies meeting the criteria, and finally selecting the maximum among those valid elements.

## Signature

```vow
fn search(lst: Vec<i64>) -> i64
```

## Contracts

- `requires: lst.len() >= 0`
- `requires: lst.len() <= 8`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method search(lst: seq<int>) returns (result: int)
    requires ValidInput(lst)
    ensures ValidResult(lst, result)
```

## Hints

- TODO: add implementation hints
