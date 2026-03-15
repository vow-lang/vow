# HE152: Compare

**Origin:** HumanEval-152 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This verification task involves implementing a function that compares two arrays of equal length representing actual game scores and guessed scores. The implementation should calculate how far off each guess was from the actual result by computing the absolute difference between corresponding elements.

## Signature

```vow
fn compare(game: Vec<i64>, guess: Vec<i64>) -> Vec<i64>
```

## Contracts

- `requires: game.len() >= 0`
- `requires: game.len() <= 8`
- `requires: guess.len() >= 0`
- `requires: guess.len() <= 8`
- `ensures: result.len() >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method compare(game: seq<int>, guess: seq<int>) returns (result: seq<int>)
  requires ValidInput(game, guess)
  ensures ValidOutput(game, guess, result)
```

## Hints

- TODO: add implementation hints
