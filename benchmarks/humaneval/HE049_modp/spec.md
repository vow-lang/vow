# HE049: Modular Exponentiation (2^n mod p)

**Origin:** HumanEval-049 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Compute `2^n mod p` using modular exponentiation. The result should be
computed efficiently (e.g., using repeated squaring or iterative multiplication
with modular reduction at each step).

## Signature

```vow
fn modp(n: i64, p: i64) -> i64
```

## Contracts

- `requires: n >= 0` — non-negative exponent
- `requires: n <= 8` — bounded for verification
- `requires: p >= 2` — modulus at least 2
- `requires: p <= 1000` — bounded modulus
- `ensures: result >= 0` — non-negative result
- `ensures: result < p` — result is in valid range

## Contract Fidelity

**PARTIAL** — the Dafny spec ensures `result == power(2, n) % p` using a
recursive spec function `power`. Vow cannot call spec functions in ensures.
The range contracts (`0 <= result < p`) verify the result is a valid modular
residue, but not that it equals the correct power.

## Hints

- Start with `result = 1`, multiply by 2 and take mod p at each step
- Use a while loop from 0 to n
- Loop invariant: `result >= 0` and `result < p`
