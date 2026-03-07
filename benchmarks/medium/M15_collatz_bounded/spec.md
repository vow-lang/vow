# M15: Collatz Bounded

## Problem

Implement a function that counts Collatz steps for small inputs.

## Signature

```vow
fn collatz_steps(n: i64) -> i64
```

## Contracts

- `requires: n >= 1` — starting value is positive
- `requires: n <= 4` — bounded for verification (n=3 takes 7 steps)
- `ensures: result >= 0` — step count is non-negative

## Constraints

- While `val != 1`: if even, `val = val / 2`; if odd, `val = 3 * val + 1`
- Count iterations
- Small inputs keep values within manageable range

## Hints

- For n <= 4, the largest intermediate Collatz value is 16 (from n=3: 3 -> 10 -> 5 -> 16 -> 8 -> 4 -> 2 -> 1)
- The loop terminates within 8 iterations for n <= 4
