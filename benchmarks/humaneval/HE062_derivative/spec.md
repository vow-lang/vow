# HE062: Derivative

**Origin:** HumanEval-062 from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

This task involves computing the derivative of a polynomial given its coefficients. Given a sequence of coefficients where xs[i] represents the coefficient of x^i, the method should return the derivative coefficients where [a₀, a₁, a₂, ...] becomes [a₁, 2a₂, 3a₃, ...].

## Signature

```vow
fn derivative(xs: Vec<i64>) -> Vec<i64>
```

## Contracts

- `requires: xs.len() >= 0`
- `requires: xs.len() <= 8`
- `ensures: result.len() >= 0`

## Contract Fidelity

**EXACT** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
method derivative(xs: seq<int>) returns (result: seq<int>)
    requires ValidInput(xs)
    ensures CorrectDerivativeCoefficients(xs, result)
    ensures |result| == DerivativeLength(xs)
```

## Hints

- TODO: add implementation hints
