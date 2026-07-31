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

- `requires: ops.len() >= 1` — an expression must contain at least one operation
- No result postcondition is stated until the contract language can express
  validity and semantics of the encoded operation sequence
- Loop `invariant: i >= 0`
- Loop `invariant: i <= ops.len()`
- Loop `invariant: sp >= 0`

## Constraints

- Use a Vec as a stack with a stack pointer `sp`
- This is a Stretch problem — verifying stack depth consistency across operations is complex
- The previous non-negativity postcondition was false for the negate operation

## Hints

- Pre-allocate the stack Vec to `ops.len()`
- Track stack pointer `sp` for push/pop
- Push: `stack[sp] = val; sp = sp + 1`
- Add: `sp = sp - 1; stack[sp-1] = stack[sp-1] + stack[sp]`
- Verifier unwind and Vec-model limits are not source preconditions
