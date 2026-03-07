# H01: Stack Push Pop

## Problem

Implement stack data structure operations. Verified functions use i64 parameters for contracts.

## Signatures

```vow
struct Stack { data: Vec<i64>, size: i64 }
fn stack_new() -> Stack
fn stack_push(s: Stack, val: i64) -> Stack
fn stack_size_bounded(size: i64) -> i64
fn stack_is_empty(size: i64) -> i64
fn stack_peek_safe(v: Vec<i64>, size: i64) -> i64
```

## Contracts

- `stack_size_bounded`: `requires: size >= 0`, `ensures: result >= 0`
- `stack_is_empty`: `ensures: result >= 0, ensures: result <= 1`
- `stack_peek_safe`: `requires: size > 0, requires: size <= v.len()`

## Constraints

- Stack struct with data Vec and size tracking
- Verified functions use i64/Vec params for contracts
- Multiple interacting functions

## Hints

- `stack_new` creates Stack with empty Vec and size 0
- `stack_push` pushes to data Vec and increments size
- `stack_size_bounded` returns the size parameter directly
- `stack_peek_safe` reads `v[size - 1]`
