# stdlib/gc

A mark-and-sweep garbage collector over a heap of `i64` values with explicit roots
and reference edges (`struct GcHeap`). Single file: copy `gc.vow`. Slots are opaque
integer handles returned by `gc_alloc` — never fabricate them.

Public API: `gc_new`, `gc_alloc`, `gc_add_root`, `gc_remove_root`, `gc_add_ref`,
`gc_read`, `gc_write`, `gc_is_alive`, `gc_count`, `gc_collect`. Full signatures and
contracts: [docs/spec/stdlib.md#gc](../../docs/spec/stdlib.md#gc).

## Usage

```
ulimit -v 2000000; build/vowc build stdlib/gc/main.vow -o /tmp/gc_demo && /tmp/gc_demo
```

## Gotchas

- `gc_collect` invalidates every slot not reachable from a root; calling
  `gc_read`/`gc_write` on a freed slot violates its precondition. It returns the count
  of *newly* freed objects.
- Roots and references are not deduplicated — adding a root twice requires two
  `gc_remove_root` calls. (`gc_remove_root` deliberately does not require the slot to
  be alive, so you can unroot a slot freed by a prior collection.)
- The heap stores only `i64`; encode richer object graphs as indices/tagged integers.
- Mark/sweep handles cycles naturally via the mark bit — no separate cycle detection.

## Verification

`vow verify stdlib/gc/main.vow` reports `VerifyFailed`: ESBMC produces a `gc_add_root`
precondition counterexample tied to how in-module caller-`requires` are checked
(cf. issue #764). The contracts are enforced at runtime in `--mode debug`. See
[docs/spec/stdlib.md#verification-status](../../docs/spec/stdlib.md#verification-status).
