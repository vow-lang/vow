# HE096: Count Up To

**Origin:** HumanEval-096 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task involves implementing a function that returns all prime numbers strictly less than a given non-negative integer n, in ascending order. The implementation should correctly identify prime numbers using a helper method and build the result sequence while maintaining the sorted order.

## Signature

```vow
fn count_up_to(n: i64) -> Vec<i64>
```

## Contracts

- `requires: n >= 0`
- `requires: n <= 10`
- `ensures: result.len() >= 0`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method count_up_to(n: int) returns (result: seq<int>)
    requires n >= 0
    ensures forall i :: 0 <= i < |result| ==> is_prime_number(result[i])
    ensures forall i :: 0 <= i < |result| ==> result[i] < n
    ensures forall p :: 2 <= p < n && is_prime_number(p) ==> p in result
    ensures forall i, j :: 0 <= i < j < |result| ==> result[i] < result[j]
```

## Hints

- TODO: add implementation hints
