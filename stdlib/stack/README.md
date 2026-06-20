# stdlib/stack

A `Vec<i64>`-backed LIFO stack (value type). `stack.vow` is the library; `node.vow` is
a vestigial `Node` struct kept only for the demo — the stack does not use it.

Public API: `stack_new`, `stack_push`, `stack_peek`, `stack_size`, `stack_is_empty`.
Full signatures and contracts:
[docs/spec/stdlib.md#stack](../../docs/spec/stdlib.md#stack).

## Usage

```
ulimit -v 2000000; build/vowc build stdlib/stack/main.vow -o /tmp/stack_demo && /tmp/stack_demo
```

## Gotchas / known gaps

This module was moved into `stdlib/` verbatim; these are tracked follow-ups, not
finished work:

- No `stack_pop` — only `push`/`peek`.
- No size-shadow invariant (`size == data.len()`) like `heap` has, and `stack_peek`
  has no `ensures` relating the result to the top element.
- Functions are not marked `pub`.
- `node.vow` is unused dead weight (the stack is Vec-backed, not a linked list).

## Verification

`vow verify stdlib/stack/main.vow` reports `Skipped`: `stack_push` allocates a `Vec`
(`RegionAlloc`), which the verifier cannot model, so the contracts are documentary —
still enforced at runtime in `--mode debug`. See
[docs/spec/stdlib.md#verification-status](../../docs/spec/stdlib.md#verification-status).
