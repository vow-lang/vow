# H10: Expression Eval (Stretch)

## Problem

Implement a stack-based expression evaluator that processes a sequence of integer operations, maintaining a stack depth invariant.

## Signatures

```vow
fn eval_rpn(ops: Vec<i64>) -> i64
```

## Encoding

Operations are encoded as integers:
- Values 0-99: push the value onto the stack
- 100: add (pop two, push sum)
- 101: negate (pop one, push negation)

## Contracts

- `requires: ops.len() >= 0`
- `requires: ops.len() <= 6` — bounded for verification
- `ensures: result >= 0` — final stack value is non-negative (given non-negative inputs)
- Loop `invariant: i >= 0`
- Loop `invariant: i <= ops.len()`
- Loop `invariant: sp >= 0`

## Constraints

- Use a Vec as a stack with a stack pointer `sp`
- This is a Stretch problem — verifying stack depth consistency across operations is complex

## Hints

- Pre-allocate stack Vec to max size
- Track stack pointer `sp` for push/pop
- Push: `stack[sp] = val; sp = sp + 1`
- Add: `sp = sp - 1; stack[sp-1] = stack[sp-1] + stack[sp]`
