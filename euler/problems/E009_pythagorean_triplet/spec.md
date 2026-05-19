# E009: Special Pythagorean Triplet

## Problem (Project Euler #9)

There exists exactly one Pythagorean triplet for which a + b + c = 1000.
Find the product abc.

**Answer:** 31875000

## Task

Implement `pythagorean_product(target: i64) -> i64` that finds the
Pythagorean triplet (a, b, c) where `a + b + c == target` and
`a*a + b*b == c*c`, then returns `a * b * c`.

## Contracts

- `requires: target > 0`
- `ensures: result > 0`

## Constraints

- Use nested `while` loops to search for `a` and `b`
- `c = target - a - b` (derived from the sum constraint)
- Check the Pythagorean condition: `a*a + b*b == c*c`
- `main()` must call `pythagorean_product(1000)` and print the result

## Hints

- Iterate `a` from 1 to `target/3`, `b` from `a+1` to `target/2`
- Compute `c = target - a - b`, check `a*a + b*b == c*c`
- Return immediately when found
