# Vow Standard Library

The standard library is a curated set of reusable, contract-annotated Vow modules
under `stdlib/`. Each module is a self-contained directory: one or more library
`.vow` files plus a `main.vow` that demonstrates and exercises the API.

This is a **reference collection**, not a globally-importable package set. Vow has
no module search path today (see [Consumption model](#consumption-model)). Modules
carry contracts, but only some are statically verifiable under the current ESBMC
model — read [Verification status](#verification-status) before relying on a
contract as a proof rather than a runtime check.

In all examples below, `vow` refers to `build/vowc`. Always run `ulimit -v 2000000`
before invoking the compiler or any binary it produces.

## Modules at a glance

| Module path           | Provides                                                                 | ESBMC status |
|-----------------------|-------------------------------------------------------------------------|---------------------------|
| `stdlib/math`         | `arithmetic`, `number_theory`, `vec_math` — integer & vector math        | VerifyFailed (env)        |
| `stdlib/heap`         | `min_heap`, `max_heap` — binary heaps over `i64`                          | VerifyFailed (env)        |
| `stdlib/stack`        | `stack` — Vec-backed LIFO stack over `i64`                               | Skipped     |
| `stdlib/geometry`     | `point`, `shape` — 2D points, circles, rectangles                        | **Verified**              |
| `stdlib/bignum`       | `bignum` — arbitrary-precision signed integers                          | Skipped     |
| `stdlib/gc`           | `gc` — mark-and-sweep garbage collector over `i64` slots                 | VerifyFailed              |

These are the `vow verify <module>/main.vow` results, measured against ESBMC 8.3.0. `(env)` marks an environmental verifier limitation, not a contract
defect. The statuses reflect the verifier's memory model, **not** the soundness of
the contracts — see [Verification status](#verification-status).

## Consumption model

`use` declarations resolve to a single directory: `use foo` loads `<dir>/foo.vow`,
where `<dir>` is the directory of the **entry file** passed to `vow build`/`vow verify`.
All transitive `use`s in dependency modules resolve against that **same** directory.
There is no search path, and `--module-root` is only available on `vow test` — not
`vow build` or `vow verify`.

Two practical ways to use a stdlib module:

**1. Run the module's own demo in place.** Each module ships a `main.vow`. Build with
`--no-verify` — most stdlib modules do not pass `vow verify` yet (see
[Verification status](#verification-status)), and the point here is to *run* the demo,
not to verify it:
```
$ ulimit -v 2000000; build/vowc build --no-verify stdlib/math/main.vow -o /tmp/math_demo
$ ulimit -v 2000000; /tmp/math_demo
```

**2. Copy the module's `.vow` file(s) into your project directory.** Because `use`
resolves against your entry file's directory, the library file must sit next to
your program. For a single-file module:
```
$ cp stdlib/math/arithmetic.vow myproject/arithmetic.vow
```
```vow
module Main
use arithmetic

fn main() -> i32 [io] {
    print_i64(clamp(15, 0, 10));   // 10
    0
}
```
For a multi-file module, copy **all** sibling files together — e.g. `stdlib/geometry`
ships `shape.vow` which internally does `use point`, so `point.vow` must be copied
alongside it.

> A real import mechanism (a module search path so `use std.math.arithmetic`
> resolves from any location) is future work. Until then, treat stdlib modules as
> vendored source you copy in, exactly like the self-hosted compiler's own modules.

## Verification status

The verifier statuses below were measured with `vow verify` against ESBMC 8.3.0.
They are **pre-existing properties of the code and the verifier**, unchanged by the
move into `stdlib/`. A `Skipped`/`VerifyFailed` status does not mean a contract is
wrong — in `--mode debug` every contract is still enforced at runtime via
`__vow_violation`.

| Module          | `vow verify` result | Why                                                                                                   |
|-----------------|---------------------|-------------------------------------------------------------------------------------------------------|
| `geometry`      | `Verified`          | The vowed shape functions use exact `i64` overflow bounds and are fully modelable. (`point_distance_sq` carries no contract, so it is not a proof obligation — see the geometry section.) |
| `math`          | `VerifyFailed`*     | `abs` collides with C `<stdlib.h>`'s `int abs(int)` in the emitted ESBMC model (parse error). Environmental; the contracts themselves are sound. |
| `heap`          | `VerifyFailed`*     | A `Vec`-typed argument to a helper hits a C-model type mismatch; most heap functions are `Skipped` because `Vec`/region allocation (`RegionAlloc`) is not modelable. |
| `stack`         | `Skipped`           | `stack_push` allocates a `Vec` (`RegionAlloc`), which the verifier cannot model; contracts are documentary. |
| `bignum`        | `Skipped`           | `Vec`-based limb arithmetic allocates per call (`RegionAlloc`); not modelable. 24 `RegionRootEscape` notes (the demo intentionally holds results for program lifetime). |
| `gc`            | `VerifyFailed`      | ESBMC produces a `gc_add_root` precondition counterexample related to in-module caller-`requires` checking (cf. issue #764). |

\* Environmental verifier limitation, not a contract defect.

**Takeaway for agents:** only `geometry`'s `vow verify` passes today — and that proves
the *vowed* checks reachable from its demo, not every function (e.g. `point_distance_sq`
carries no contract and is not a proof obligation). For
the others, the contracts are precise specifications that are enforced at runtime in
`--mode debug`; static proof is gated on verifier-model improvements (Vec/region
modeling, the `abs`/libc rename, and the #764 caller-`requires` fix). When you build
on these modules and need a *static* guarantee, prefer `geometry`'s pattern: keep
hot paths in plain `i64` with explicit overflow `requires`.

---

## math

Three modules under `stdlib/math/`. Each is independent (no cross-`use`); copy only
the one you need. All functions are `pub`.

### math.arithmetic

Integer primitives with overflow-guarded contracts. The `safe_*` family operates on
**non-negative** inputs only — they are overflow-checked unsigned-style helpers, not
general signed wrappers.

| Function | Signature | Key contracts | Notes |
|----------|-----------|---------------|-------|
| `abs` | `(x: i64) -> i64` | `requires x > -9223372036854775807`; `ensures result >= 0`; `ensures result == x \|\| result == 0 - x` | Guards `i64::MIN` negation overflow. |
| `min` | `(a, b: i64) -> i64` | `ensures result <= a`; `result <= b`; `result == a \|\| result == b` | Tight: result is one of the inputs. |
| `max` | `(a, b: i64) -> i64` | `ensures result >= a`; `result >= b`; `result == a \|\| result == b` | |
| `clamp` | `(x, lo, hi: i64) -> i64` | `requires lo <= hi`; `ensures lo <= result <= hi` | |
| `sign` | `(x: i64) -> i64` | `ensures -1 <= result <= 1` | -1 / 0 / 1. |
| `safe_add` | `(a, b: i64) -> i64` | `requires a >= 0, b >= 0, a <= I64_MAX - b`; `ensures result == a + b` | Non-negative inputs only. |
| `safe_sub` | `(a, b: i64) -> i64` | `requires a >= 0, b >= 0, a >= b`; `ensures 0 <= result <= a` | |
| `safe_mul` | `(a, b: i64) -> i64` | `requires a >= 0, b >= 0, b == 0 \|\| a <= I64_MAX / b`; `ensures result == a * b` | |
| `safe_div` | `(a, b: i64) -> i64` | `requires a >= 0, b > 0`; `ensures 0 <= result <= a` | `b > 0`, not just `b != 0`. |
| `safe_mod` | `(a, b: i64) -> i64` | `requires a >= 0, b > 0`; `ensures 0 <= result < b` | |
| `pow` | `(base, exp: i64) -> i64` | `requires base >= 0, exp >= 0`; `ensures result >= 0` | O(exp) — no fast exponentiation; no overflow guard on the running product. |
| `midpoint` | `(a, b: i64) -> i64` | `requires a >= 0, a <= b`; `ensures a <= result <= b` | Overflow-safe `a + (b-a)/2`. |
| `diff` | `(a, b: i64) -> i64` | `requires a >= 0, b >= 0`; `ensures result >= 0` | `|a - b|`. |
| `divides` | `(d, n: i64) -> bool` | `requires d != 0` | |
| `is_even` / `is_odd` | `(x: i64) -> bool` | — | |

Representative contract — overflow guard expressed in the precondition rather than
via checked arithmetic:
```vow
pub fn safe_mul(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: b == 0 || a <= 9223372036854775807 / b,
    ensures: result == a * b,
    ensures: result >= 0
}
```

### math.number_theory

| Function | Signature | Key contracts | Notes |
|----------|-----------|---------------|-------|
| `gcd` | `(a, b: i64) -> i64` | `requires a >= 0, b >= 0, a > 0 \|\| b > 0`; `ensures result > 0` | Euclid; loop invariants `x >= 0, y >= 0`. |
| `lcm` | `(a, b: i64) -> i64` | `requires a > 0, b > 0`; `ensures result > 0` | No overflow guard on `(a/g)*b`. |
| `is_prime` | `(n: i64) -> bool` | `requires n >= 0` | Trial division to `i*i <= n`. |
| `power_mod` | `(base, exp, modulus: i64) -> i64` | `requires base >= 0, exp >= 0, modulus > 1, modulus <= 3037000499`; `ensures 0 <= result < modulus` | Modulus bound = `isqrt(I64_MAX)`, prevents `(r*b)` overflow. |
| `factorial` | `(n: i64) -> i64` | `requires n >= 0`; `ensures result >= 1` | No upper bound on `n` — product overflows past 20!. |
| `fibonacci` | `(n: i64) -> i64` | `requires n >= 0`; `ensures result >= 0` | Iterative; overflows past F(92). |
| `isqrt` | `(n: i64) -> i64` | `requires n >= 0`; `ensures result >= 0, result*result <= n` | Floor integer sqrt; postcondition is the real spec. |
| `largest_divisor` | `(n: i64) -> i64` | `requires n > 1`; `ensures 1 <= result < n` | Largest proper divisor. |
| `count_divisors` | `(n: i64) -> i64` | `requires n > 0`; `ensures result >= 1` | |

### math.vec_math

Operates on `Vec<i64>`. None of the summation helpers guard against accumulator
overflow — use on bounded data, or add `requires` bounds at the call site.

| Function | Signature | Key contracts | Notes |
|----------|-----------|---------------|-------|
| `vec_sum` | `(v: Vec<i64>) -> i64` | — | No overflow guard. |
| `vec_min` / `vec_max` | `(v: Vec<i64>) -> i64` | `requires v.len() > 0` | |
| `vec_mean` | `(v: Vec<i64>) -> i64` | `requires v.len() > 0` | Integer mean. |
| `vec_dot` | `(a, b: Vec<i64>) -> i64` | `requires a.len() == b.len()` | |
| `vec_count` | `(v: Vec<i64>, target: i64) -> i64` | `ensures 0 <= result <= v.len()` | Invariant `count <= i`. |
| `vec_all_in_range` | `(v: Vec<i64>, lo, hi: i64) -> bool` | `requires lo <= hi` | |
| `vec_is_sorted` | `(v: Vec<i64>) -> bool` | — | Ascending. |
| `vec_prefix_sum` | `(v: Vec<i64>) -> Vec<i64>` | `ensures result.len() == v.len()` | |
| `vec_reverse` | `(v: Vec<i64>) -> Vec<i64>` | `ensures result.len() == v.len()` | |

---

## heap

`stdlib/heap/min_heap.vow` and `max_heap.vow` are structural mirrors (a min-heap and
a max-heap over `i64`), with the comparator flipped. Both are value types: every
mutator takes a heap by value and returns a new one.

The defining contract pattern is the **size-shadow invariant** `size == data.len()`,
threaded through every mutator. This is what lets ESBMC reason about in-bounds
`data[i]` access without a universal quantifier:
```vow
pub fn min_heap_push(h: MinHeap, val: i64) -> MinHeap vow {
    requires: h.size == h.data.len(),
    requires: h.size < 9223372036854775807,
    ensures: result.size == h.size + 1,
    ensures: result.size == result.data.len()
}
```

| Function (min; `max_*` mirrors) | Signature | Key contracts |
|---------------------------------|-----------|---------------|
| `min_heap_new` | `() -> MinHeap` | `ensures result.size == 0, result.data.len() == 0` |
| `min_heap_len` | `(h) -> i64` | `ensures result == h.size` |
| `min_heap_is_empty` | `(h) -> bool` | `ensures result == (h.size == 0)` |
| `min_heap_push` | `(h, val: i64) -> MinHeap` | size-shadow in/out; `ensures result.size == h.size + 1` |
| `min_heap_peek` | `(h) -> i64` | `requires h.size > 0, size-shadow`; `ensures result == h.data[0]` |
| `min_heap_pop` | `(h) -> MinHeap` | `requires h.size > 0, size-shadow`; `ensures result.size == h.size - 1` |
| `min_heap_clear` | `(h) -> MinHeap` | size-shadow in; `ensures result.size == 0` |
| `is_min_heap` | `(h) -> bool` | `requires size-shadow` — runtime check of the heap-order property |

**Heap-order is a runtime predicate, by design.** Vow has no universal quantifier, so
the property `∀i. data[parent(i)] <= data[i]` cannot be written as an `ensures`.
`is_min_heap` / `is_max_heap` check it at runtime instead; the static contracts cover
index safety and the size-shadow invariant only.

---

## stack

`stdlib/stack/stack.vow` — a `Vec<i64>`-backed LIFO stack (value type). `node.vow` in
the same directory is a vestigial `Node` struct kept for the demo; the stack does not
use it.

| Function | Signature | Key contracts |
|----------|-----------|---------------|
| `stack_new` | `() -> Stack` | — |
| `stack_push` | `(s, val: i64) -> Stack` | `ensures result.size == s.size + 1` |
| `stack_peek` | `(s) -> i64` | `requires s.size > 0` |
| `stack_size` | `(s) -> i64` | — |
| `stack_is_empty` | `(s) -> bool` | — |

**Known gaps (move-verbatim; tracked follow-up):** no `stack_pop`; no size-shadow
invariant (`size == data.len()`) like `heap` has; `stack_peek` has no `ensures`
relating the result to `data[size-1]`; functions are not marked `pub`; `node.vow` is
unused.

---

## geometry

`stdlib/geometry/point.vow` (a `Point` struct) and `shape.vow` (a `Shape` enum with
circle/rectangle area and perimeter). **The only module whose `vow verify` passes
today** — its shape functions use exact derived overflow bounds. Note this means the
*vowed* checks verify (`vow verify stdlib/geometry/main.vow` → `Verified`); it is not a
proof of the whole API, since `point_distance_sq` carries no contract (see Known gaps).

| Function | Signature | Key contracts |
|----------|-----------|---------------|
| `point_new` / `point_x` / `point_y` | `Point` accessors | — |
| `point_distance_sq` | `(a, b: Point) -> i64` | — (no overflow guard — gap for large coordinates) |
| `circle_area` | `(r: i64) -> i64` | `requires 0 <= r <= 1753413056`; `ensures result >= 0` |
| `rect_area` | `(w, h: i64) -> i64` | `requires w >= 0, h >= 0, h == 0 \|\| w <= I64_MAX / h`; `ensures result >= 0` |
| `circle_perimeter` | `(r: i64) -> i64` | `requires 0 <= r <= 1537228672809129301`; `ensures result >= 0` |
| `rect_perimeter` | `(w, h: i64) -> i64` | `requires w >= 0, h >= 0, w <= 4611686018427387903 - h`; `ensures result >= 0` |

Each magic bound is the exact threshold below which the arithmetic cannot overflow —
e.g. `circle_area` caps `r` at `floor(sqrt(I64_MAX/3))` because it computes `r*r*3`:
```vow
fn circle_area(r: i64) -> i64 vow {
    requires: r >= 0
    requires: r <= 1753413056
    ensures: result >= 0
}
```

**Known gaps:** the `Shape` enum is declared but the area/perimeter functions are
free functions that don't dispatch on it; `point_distance_sq` lacks an overflow
guard; `shape_at` is a demo artifact, not a real API.

---

## bignum

`stdlib/bignum/bignum.vow` — arbitrary-precision **signed** integers in base 10⁹
sign-magnitude form (`struct BigNum { digits: Vec<i64>, sign: i64 }`). Pure core
language; no builtins beyond `Vec`/`String`/`i64`.

**Public API (selected):**
- Construct: `bignum_zero`, `bignum_from_i64`, `bignum_from_string`
- Convert: `bignum_to_string`
- Predicates: `bignum_is_zero`, `bignum_is_negative`, `bignum_is_positive`
- Compare: `bignum_cmp`, `bignum_cmp_abs`, `bignum_eq`, `bignum_lt`, `bignum_gt`, `bignum_le`, `bignum_ge`
- Arithmetic: `bignum_negate`, `bignum_abs`, `bignum_add`, `bignum_sub`, `bignum_mul`, `bignum_div`, `bignum_mod`, `bignum_divmod`
- Higher-level: `bignum_pow(base, exp: i64)`, `bignum_gcd`, `bignum_factorial(n: i64)`

**Contracts present:** `bignum_sub_abs` requires `bignum_cmp_abs(a, b) >= 0`;
`bignum_div`/`bignum_mod`/`bignum_divmod` require `!bignum_is_zero(b)`; `bignum_pow`
requires `exp >= 0`; `bignum_factorial` requires `n >= 0`.

**Semantics to know:**
- The representation invariant (non-empty `digits`, no leading-zero limbs except the
  canonical zero `[0]` with `sign == 1`, `sign ∈ {-1, 1}`) is maintained internally
  but **not** stated as a struct invariant or `ensures`.
- Division truncates toward zero; the remainder's sign matches the dividend.
- `bignum_pow`/`bignum_factorial` take a native `i64` exponent/argument, not a BigNum.
- `bignum_gcd` operates on absolute values; the result is non-negative.
- Multiplication is O(n·m) schoolbook (no Karatsuba).
- Helpers prefixed for internal use (`bignum_strip_zeros`, `bignum_shift_limbs`,
  `i64_to_decimal*`, `bignum_divmod_long`, …) are not part of the public API.

**Verification:** `Skipped` — limb arithmetic allocates `Vec`s per call (`RegionAlloc`),
which the verifier cannot model. Contracts are runtime-enforced in `--mode debug`.

---

## gc

`stdlib/gc/gc.vow` — a mark-and-sweep garbage collector over a heap of `i64` values
with explicit roots and reference edges (`struct GcHeap`). Slots are opaque integer
handles returned by `gc_alloc`; never fabricate them.

| Function | Signature | Key contracts |
|----------|-----------|---------------|
| `gc_new` | `() -> GcHeap` | — |
| `gc_alloc` | `(h, val: i64) -> i64` | — (returns a slot; reuses freed slots) |
| `gc_add_root` | `(h, slot: i64)` | `requires 0 <= slot < values.len(), alive[slot] == 1` |
| `gc_remove_root` | `(h, slot: i64)` | `requires 0 <= slot < values.len()` (does **not** require alive — you may unroot a freed slot) |
| `gc_add_ref` | `(h, from, to: i64)` | `requires` both in range and alive |
| `gc_read` | `(h, slot: i64) -> i64` | `requires 0 <= slot < values.len(), alive[slot] == 1` |
| `gc_write` | `(h, slot, val: i64)` | `requires 0 <= slot < values.len(), alive[slot] == 1` |
| `gc_is_alive` | `(h, slot: i64) -> bool` | `requires 0 <= slot < values.len()` |
| `gc_count` | `(h) -> i64` | — |
| `gc_collect` | `(h) -> i64` | — (returns count of newly-freed objects) |

**Semantics to know:**
- `gc_collect` invalidates every slot not reachable from a root; calling
  `gc_read`/`gc_write` on a freed slot violates its precondition.
- Roots and references are not deduplicated — adding a root twice needs two
  `gc_remove_root` calls.
- The heap stores only `i64`; represent richer object graphs as indices/tagged ints.
- Mark/sweep handles cycles naturally via the mark bit; no separate cycle detection.

**Verification:** `VerifyFailed` — ESBMC produces a `gc_add_root` precondition
counterexample tied to how in-module caller-`requires` are checked (cf. issue #764).
Contracts are runtime-enforced in `--mode debug`.

---

## Known gaps and roadmap

These are tracked follow-ups, intentionally **not** addressed by the reorg that
created `stdlib/` (which moved code verbatim):

- **Static verifiability.** Make `Vec`/region-allocating functions modelable so
  `stack`, `bignum`, and most of `heap` can be statically verified; rename `abs` to
  avoid the C `<stdlib.h>` collision that blocks `math`; resolve the `gc_add_root`
  caller-`requires` counterexample (#764).
- **Contract hardening.** Add struct/representation invariants and `ensures` clauses
  to `bignum` and `gc`; add a size-shadow invariant and `stack_pop` to `stack`; add
  an overflow guard to `point_distance_sq`; wire the `Shape` enum into `geometry`'s
  area/perimeter functions.
- **Consistency.** Mark all intended-public functions `pub` (currently only `math`
  and `heap` do); remove or rebuild the vestigial `stack/node.vow`.
- **Distribution.** A module search path so stdlib modules can be imported without
  copying source into the consuming project.
