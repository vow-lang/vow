# HE030: Get Positive

**Origin:** HumanEval-030 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This task implements a function to filter positive numbers from a list of integers. The function should return a new sequence containing only the positive numbers (greater than 0) while preserving their original order from the input sequence.

## Signature

```vow
fn get_positive(l: Vec<i64>) -> Vec<i64>
```

## Contracts

- `requires: l.len() >= 0`
- `requires: l.len() <= 8`
- `ensures: result.len() >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method get_positive(l: seq<int>) returns (result: seq<int>)
    ensures AllPositive(result)
    ensures AllElementsFromOriginal(result, l)
    ensures ContainsAllPositives(result, l)
    ensures |result| == CountPositives(l)
    ensures PreservesOrder(result, l)
```

## Hints

- TODO: add implementation hints
