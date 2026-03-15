# HE025: Prime Factorization

**Origin:** HumanEval-025 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Given a positive integer n, return a Vec of its prime factors in ascending
order. Each prime factor should appear according to its multiplicity. For
example, `factorize(12)` returns `[2, 2, 3]`.

## Signature

```vow
fn factorize(n: i64) -> Vec<i64>
```

## Contracts

- `requires: n >= 2` — input at least 2
- `requires: n <= 100` — bounded for verification
- `ensures: result.len() >= 1` — at least one factor

## Contract Fidelity

**PARTIAL** — the Dafny spec additionally ensures: `product(factors) == n`
(factors multiply to n), all factors are prime, factors are non-decreasing,
and all factors >= 2. Vow cannot call spec functions (like `product`) in
ensures clauses and cannot express universal quantifiers. Only the minimum
length is verified.

## Hints

- Start with divisor 2, divide n repeatedly while divisible
- Move to the next divisor when n is no longer divisible
- Push each divisor to the result Vec as you extract it
- Stop when divisor * divisor > n; if n > 1, push the remaining n
