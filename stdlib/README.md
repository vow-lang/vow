# Vow Standard Library

Reusable, contract-annotated Vow modules. Each subdirectory is a self-contained
module: one or more library `.vow` files plus a `main.vow` that demonstrates the API.

This is a **reference collection of vendored source**, not a packaged library — Vow
has no module search path yet, so you consume a module by running its demo in place
or copying its `.vow` file(s) into your own project (see [Using a module](#using-a-module)).

The authoritative, agent-facing reference (full signatures, contracts, and per-module
verification notes) is **[docs/spec/stdlib.md](../docs/spec/stdlib.md)**. This file is
the human-facing map.

## Modules

| Module | Files | What it provides | `vow verify` today |
|--------|-------|------------------|--------------------|
| [math](math/README.md) | `arithmetic`, `number_theory`, `vec_math` | Integer + vector math with overflow-guarded contracts | Blocked (env: `abs`/libc) |
| [heap](heap/README.md) | `min_heap`, `max_heap` | Binary heaps over `i64` (size-shadow invariant) | Partial / blocked |
| [stack](stack/README.md) | `stack` (+ vestigial `node`) | Vec-backed LIFO stack over `i64` | Skipped (documentary) |
| [geometry](geometry/README.md) | `point`, `shape` | 2D points; circle/rectangle area & perimeter | **Verified** |
| [bignum](bignum/README.md) | `bignum` | Arbitrary-precision signed integers (base 10⁹) | Skipped (documentary) |
| [gc](gc/README.md) | `gc` | Mark-and-sweep GC over `i64` slots | VerifyFailed (#764) |

Only `geometry` currently passes `vow verify` — and that proves the *vowed* checks
reachable from its demo, not the whole API (`point_distance_sq` carries no contract).
For the rest, contracts are
precise specifications enforced at runtime in `--mode debug`; static proof is gated on
verifier-model improvements. The statuses are pre-existing properties of the code and
the verifier — see [docs/spec/stdlib.md](../docs/spec/stdlib.md#verification-status)
for the full explanation.

## Using a module

`use foo` resolves to `<entry-file-dir>/foo.vow`. There is no search path, and
`--module-root` exists only on `vow test`. So:

**Run a module's demo in place:**
```
ulimit -v 2000000; build/vowc build --no-verify stdlib/math/main.vow -o /tmp/math_demo && /tmp/math_demo
```

**Copy a module into your project** (copy every sibling `.vow` for multi-file modules):
```
cp stdlib/math/arithmetic.vow myproject/arithmetic.vow
```
```vow
module Main
use arithmetic

fn main() -> i32 [io] {
    print_i64(clamp(15, 0, 10));   // 10
    0
}
```

## Tests

The multi-module section of `scripts/full_test.sh` builds, verifies, and runs every
`stdlib/<module>/main.vow` with both the Rust and self-hosted compilers, asserting
byte-for-byte output parity.
