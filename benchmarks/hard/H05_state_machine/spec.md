# H05: State Machine

## Problem

Implement a state machine with valid transitions encoded as integer states and transition functions with contracts.

## Signatures

```vow
fn state_new() -> i64
fn state_advance(state: i64) -> i64
fn state_is_terminal(state: i64) -> i64
```

## Contracts

- `state_new`: `ensures: result == 0` — initial state is 0
- `state_advance`: `requires: state >= 0, requires: state <= 2`, `ensures: result >= 0, ensures: result <= 3` — valid transitions
- `state_is_terminal`: `ensures: result >= 0, ensures: result <= 1`

## States

- 0: Start → advances to 1
- 1: Processing → advances to 2
- 2: Complete → advances to 3
- 3: Terminal (no advance)

## Constraints

- States encoded as i64 values 0-3
- `state_advance` must not exceed state 3
- Contracts ensure valid state transitions

## Hints

- Use `if`/`else` chains for state transitions
- Each state maps to exactly one next state
