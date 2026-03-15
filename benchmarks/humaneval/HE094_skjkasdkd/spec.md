# HE094: Skjkasdkd

**Origin:** HumanEval-094 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task involves implementing a method that finds the largest prime number in a given list of integers and returns the sum of its digits. If no prime number exists in the list, the method should return 0. The implementation requires helper functions for prime checking and digit sum calculation.

## Signature

```vow
fn skjkasdkd(lst: Vec<i64>) -> i64
```

## Contracts

- `requires: lst.len() >= 0`
- `requires: lst.len() <= 8`
- `ensures: result >= 0`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method skjkasdkd(lst: seq<int>) returns (result: int)
    ensures result >= 0
    ensures (forall x :: x in lst ==> !is_prime_pure(x)) ==> result == 0
    ensures (exists x :: x in lst && is_prime_pure(x)) ==> 
        (exists largest :: (largest in lst && is_prime_pure(largest) && 
         (forall y :: y in lst && is_prime_pure(y) ==> y <= largest) &&
         result == sum_of_digits_pure(largest)))
```

## Hints

- TODO: add implementation hints
