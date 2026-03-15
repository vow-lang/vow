# HE041: Car Race Collision

**Origin:** HumanEval-041 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

Given n cars moving left-to-right and n cars moving right-to-left on a straight
infinite road, all at the same speed, calculate the total number of collisions.
Cars pass through each other when they collide and continue moving. Each
left-moving car will collide with each right-moving car exactly once, resulting
in n * n total collisions.

## Signature

```vow
fn car_race_collision(n: i64) -> i64
```

## Contracts

- `requires: n >= 0` — non-negative input
- `requires: n <= 1000` — bounded input
- `ensures: result == n * n` — exact collision count
- `ensures: result >= 0` — non-negative result

## Contract Fidelity

**EXACT** — the Vow contracts fully capture the Dafny specification.

## Hints

- The result is simply `n * n`
