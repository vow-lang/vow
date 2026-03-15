# HE163: Generate Integers

**Origin:** HumanEval-163 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

function_signature: method generate_integers(a : int, b : int) returns (result: seq<int>)
Generate elements. Ensures: the condition holds for all values; the condition holds for all values.

## Signature

```vow
fn generate_integers(a: i64, b: i64) -> Vec<i64>
```

## Contracts

- `requires: a >= 0`
- `requires: a <= 100`
- `requires: b >= 0`
- `requires: b <= 100`
- `ensures: result.len() >= 0`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method generate_integers(a : int, b : int) returns (result: seq<int>)
  ensures forall i :: 0 <= i < |result| ==> result[i] in {2, 4, 6, 8}
  ensures forall i :: 0 <= i < |result| - 1 ==> result[i] < result[i + 1]
  ensures forall x :: x in result ==> (x >= a && x <= b) || (x >= b && x <= a)
  ensures forall x :: x in {2, 4, 6, 8} && ((x >= a && x <= b) || (x >= b && x <= a)) ==> x in result
```

## Hints

- TODO: add implementation hints
