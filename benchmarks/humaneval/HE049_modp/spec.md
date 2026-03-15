# HE049: Modular Exponentiation (2^n mod p)

**Origin:** HumanEval-049 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Compute `2^n mod p` using modular exponentiation. The result should be
computed efficiently (e.g., using repeated squaring or iterative multiplication
with modular reduction at each step).

A spec function `power_mod(base, exp, m)` is provided that computes
`base^exp mod m` iteratively. Your implementation must match `power_mod(2, n, p)`.

## Signature

```vow
fn modp(n: i64, p: i64) -> i64
```

## Contracts

- `requires: n >= 0` — non-negative exponent
- `requires: n <= 8` — bounded for verification
- `requires: p >= 2` — modulus at least 2
- `requires: p <= 100` — bounded modulus
- `ensures: result == power_mod(2, n, p)` — matches spec function

## Contract Fidelity

**EXACT** — the spec function `power_mod` implements iterative modular
exponentiation (equivalent to Dafny's recursive `power` function). The ensures
clause verifies `result == power_mod(2, n, p)`.

## Hints

- Start with `result = 1`, multiply by 2 and take mod p at each step
- Use a while loop from 0 to n
- Loop invariant: `result >= 0` and `result < p`
