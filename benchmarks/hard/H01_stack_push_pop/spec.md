# H01: Stack Push Pop

## Problem

Implement stack data structure operations. Verified functions use `u64` sizes and `i64` values/flags for contracts.

## Signatures

```vow
struct Stack { data: Vec<i64>, size: u64 }
fn stack_new() -> Stack
fn stack_push(s: Stack, val: i64) -> Stack
fn stack_size_bounded(size: u64) -> u64
fn stack_is_empty(size: u64) -> i64
fn stack_peek_safe(v: Vec<i64>, size: u64) -> i64
```

## Contracts

- `stack_size_bounded`: no contract; `size` is a `u64`, so non-negativity is carried by the type
- `stack_is_empty`: `ensures: result >= 0, ensures: result <= 1` (the `0`/`1` flag is an `i64`)
- `stack_peek_safe`: `requires: size > 0, requires: size <= v.len()`

## Constraints

- Stack struct with data Vec and size tracking
- Verified functions use u64 size / i64 value / Vec params for contracts
- Multiple interacting functions

## Hints

- `stack_new` creates Stack with empty Vec and size 0
- `stack_push` pushes to data Vec and increments size
- `stack_size_bounded` returns the size parameter directly
- `stack_peek_safe` reads `v[size - 1]`
