# HE147: Get Max Triples

**Origin:** HumanEval-147 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task involves counting valid triples from a special array. Given a positive integer n, create an array where each element a[i] = i² - i + 1 for positions 1 to n. The goal is to count the number of triples (a[i], a[j], a[k]) where i < j < k and their sum is divisible by 3.

The implementation uses the mathematical insight that elements can be categorized by their modulo 3 value, and valid triples must either come from all elements with the same modulo value.

## Signature

```vow
fn get_max_triples(n: i64) -> i64
```

## Contracts

- `requires: n >= 0`
- `requires: n <= 100`
- `ensures: result >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method get_max_triples(n: int) returns (result: int)
  requires ValidInput(n)
  ensures result >= 0
  ensures result == count_valid_triples(n)
```

## Hints

- TODO: add implementation hints
