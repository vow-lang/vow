# HE031: Is Prime

**Origin:** HumanEval-031 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Implement a function to determine if a positive integer is a prime number. A
prime number is greater than 1 and has no divisors other than 1 and itself.
Return 1 if the number is prime, 0 otherwise.

## Signature

```vow
fn is_prime(n: i64) -> i64
```

## Contracts

- `requires: n >= 0` — non-negative input
- `requires: n <= 100` — bounded for verification
- `ensures: result >= 0` — boolean result
- `ensures: result <= 1` — boolean result

## Contract Fidelity

**WEAK** — the Dafny spec ensures `result <==> is_prime_number(n)` where
`is_prime_number` is a recursive predicate using `forall k :: 2 <= k < n ==>
n % k != 0`. Vow cannot call spec functions in ensures clauses, so the contract
only checks the result is 0 or 1. ESBMC still verifies loop invariants and
absence of undefined behavior.

## Hints

- Check divisibility from 2 up to the square root of n
- Numbers less than 2 are not prime
- Use a while loop with an invariant tracking the loop counter bounds
