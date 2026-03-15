# HE059: Largest Prime Factor

**Origin:** HumanEval-059 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task implements an algorithm to find the largest prime factor of a composite integer n (where n > 1 and n is not prime). The algorithm uses trial division, first removing all factors of 2, then checking odd factors up to the square root of the remaining number.

The implementation must ensure that the returned result is indeed a prime number, divides n, and is the largest such prime factor among all factors of n.

## Signature

```vow
fn largest_prime_factor(n: i64) -> i64
```

## Contracts

- `requires: n >= 0`
- `requires: n <= 100`
- `ensures: result >= 0`

## Contract Fidelity

**PARTIAL** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method largest_prime_factor(n: int) returns (result: int)
    requires n > 1
    requires !is_prime(n)
    ensures result > 1
    ensures n % result == 0
    ensures forall k :: k > result && n % k == 0 ==> !is_prime(k)
    ensures is_prime(result)
```

## Hints

- TODO: add implementation hints
