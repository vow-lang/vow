# stdlib/heap

Binary heaps over `i64`. `min_heap.vow` and `max_heap.vow` are structural mirrors
(min-heap vs max-heap); copy whichever you need. Both are **value types**: every
mutator takes a heap by value and returns a new one.

Public API (min-heap; `max_heap_*` mirrors it): `min_heap_new`, `min_heap_len`,
`min_heap_is_empty`, `min_heap_push`, `min_heap_peek`, `min_heap_pop`,
`min_heap_clear`, `is_min_heap`. Full signatures and contracts:
[docs/spec/stdlib.md#heap](../../docs/spec/stdlib.md#heap).

## Usage

```
ulimit -v 2000000; build/vowc build stdlib/heap/main.vow -o /tmp/heap_demo && /tmp/heap_demo
```

## Key idea: the size-shadow invariant

Every mutator carries `requires/ensures: size == data.len()`. This invariant is what
lets ESBMC prove all `data[i]` accesses are in bounds without a universal quantifier.
Preserve it if you extend the module.

## Gotchas

- **Heap-order is a runtime predicate.** Vow has no `forall`, so the ordering property
  `data[parent(i)] <= data[i]` cannot be an `ensures`. Call `is_min_heap` /
  `is_max_heap` to check it at runtime; the static contracts cover index safety and
  the size-shadow invariant only.
- `min_heap`/`max_heap` are duplicated code (comparator flipped). A single
  comparator-parameterized heap would be cleaner but needs generics Vow lacks today.

## Verification

`vow verify stdlib/heap/main.vow` reports `VerifyFailed`: most heap functions are
`Skipped` because `Vec`/region allocation (`RegionAlloc`) is not modelable, and a
`Vec`-typed argument to a helper hits a C-model type mismatch. The contracts are
enforced at runtime in `--mode debug`. See
[docs/spec/stdlib.md#verification-status](../../docs/spec/stdlib.md#verification-status).
