# HE104: Unique Digits

**Origin:** HumanEval-104 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: def unique_digits(x: List[nat]) -> List[nat]
Given a list of positive integers x. return a sorted list of all elements that hasn't any even digit.

## Signature

```vow
fn unique_digits(x: Vec<i64>) -> Vec<i64>
```

## Contracts

- `requires: x.len() >= 0`
- `requires: x.len() <= 8`
- `ensures: result.len() >= 0`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method UniqueDigits(x: seq<int>) returns (result: seq<int>)

  ensures forall i :: 0 <= i < |result| ==> HasNoEvenDigit(result[i])
  ensures forall i, j :: 0 <= i < j < |result| ==> result[i] <= result[j]
  ensures forall e :: e in x && HasNoEvenDigit(e) ==> e in result
  ensures forall e :: e in result ==> e in x
```

## Hints

- TODO: add implementation hints
