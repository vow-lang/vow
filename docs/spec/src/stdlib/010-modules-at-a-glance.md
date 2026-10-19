# Vow Standard Library

The standard library is a curated set of reusable, contract-annotated Vow modules
under `stdlib/`. Each module is a self-contained directory: one or more library
`.vow` files plus a `main.vow` that demonstrates and exercises the API.

This is a **reference collection**, not a globally-importable package set. Vow has
no module search path today (see [Consumption model](#consumption-model)). Modules
carry contracts, but only some are statically verifiable under the current ESBMC
backend model — read [Verification status](#verification-status) before relying on a
contract as a proof rather than a runtime check.

In all examples below, `vow` refers to `build/vowc`.

## Modules at a glance

| Module path           | Provides                                                                 | ESBMC-backend status |
|-----------------------|-------------------------------------------------------------------------|---------------------------|
| `stdlib/math`         | `arithmetic`, `number_theory`, `vec_math` — integer & vector math        | VerifyFailed (env)        |
| `stdlib/heap`         | `min_heap`, `max_heap` — binary heaps over `i64`                          | VerifyFailed (env)        |
| `stdlib/stack`        | `stack` — Vec-backed LIFO stack over `i64`                               | Skipped     |
| `stdlib/geometry`     | `point`, `shape` — 2D points, circles, rectangles                        | **Verified**              |
| `stdlib/bignum`       | `bignum` — arbitrary-precision signed integers                          | Skipped     |
| `stdlib/gc`           | `gc` — mark-and-sweep garbage collector over `i64` slots                 | VerifyFailed              |

These are the `vow verify <module>/main.vow` results, measured against ESBMC 8.3.0 (ESBMC backend; native results are not recorded here). `(env)` marks an environmental verifier limitation, not a contract
defect. The statuses reflect the verifier's memory model, **not** the soundness of
the contracts — see [Verification status](#verification-status).

## Consumption model

`use` declarations resolve to a single directory: `use foo` loads
`<dir>/foo.vow.d` when that declaration stub exists, and otherwise loads
`<dir>/foo.vow`, where `<dir>` is the directory of the **entry file** passed to
`vow build`/`vow verify`. A stub is used only when none of its declarations
carry a `vow` block — otherwise the sibling `<dir>/foo.vow` source is loaded
instead, so a declaration's contract is never silently dropped from
verification (a bodyless declaration cannot be checked at a call site).
All transitive `use`s in dependency modules resolve against that **same** directory.
There is no search path, and `--module-root` is only available on `vow test` — not
`vow build` or `vow verify`.

Two practical ways to use a stdlib module:

**1. Run the module's own demo in place.** Each module ships a `main.vow`. Build with
`--no-verify` — most stdlib modules do not pass `vow verify` yet (see
[Verification status](#verification-status)), and the point here is to *run* the demo,
not to verify it:
```
$ build/vowc build --no-verify stdlib/math/main.vow -o /tmp/math_demo
$ /tmp/math_demo
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

The verifier statuses below were measured with `vow verify` (ESBMC backend) against ESBMC 8.3.0.
They are **pre-existing properties of the code and the verifier**, unchanged by the
move into `stdlib/`. A `Skipped`/`VerifyFailed` status does not mean a contract is
wrong — in `--mode debug` every contract is still enforced at runtime via
`__vow_violation`.

| Module          | `vow verify` result | Why                                                                                                   |
|-----------------|---------------------|-------------------------------------------------------------------------------------------------------|
| `geometry`      | `Verified`          | The vowed shape functions use exact `i64` overflow bounds and are fully modelable. (`point_distance_sq` carries no contract, so it is not a proof obligation — see the geometry section.) |
| `math`          | `VerifyFailed`      | The old `abs`/`<stdlib.h>` collision is resolved — user functions are namespaced `vow_user_fn_<id>` in the emitted ESBMC model. The remaining blocker is genuine: `pow`'s `ensures result >= 0` is refuted by an `i64` overflow counterexample (a large `base`/`exp` wraps negative). A contract-hardening gap (overflow guard needed), not environmental. |
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
modeling and the #764 caller-`requires` fix). When you build
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
| `pow` | `(base: i64, exp: u64) -> i64` | `requires base >= 0`; `ensures result >= 0` | O(exp) — no fast exponentiation; no overflow guard on the running product. |
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
| `power_mod` | `(base: i64, exp: u64, modulus: i64) -> i64` | `requires base >= 0, modulus > 1, modulus <= 3037000499`; `ensures 0 <= result < modulus` | Modulus bound = `isqrt(I64_MAX)`, prevents `(r*b)` overflow. |
| `factorial` | `(n: i64) -> i64` | `requires n >= 0`; `ensures result >= 1` | No upper bound on `n` — product overflows past 20!. |
| `fibonacci` | `(n: i64) -> i64` | `requires n >= 0`; `ensures result >= 0` | Iterative; overflows past F(92). |
| `isqrt` | `(n: i64) -> i64` | `requires n >= 0`; `ensures result >= 0, result*result <= n` | Floor integer sqrt; postcondition is the real spec. |
| `largest_divisor` | `(n: i64) -> i64` | `requires n > 1`; `ensures 1 <= result < n` | Largest proper divisor. |
| `count_divisors` | `(n: i64) -> u64` | `requires n > 0`; `ensures result >= 1` | |

### math.vec_math

Operates on `Vec<i64>`. None of the summation helpers guard against accumulator
overflow — use on bounded data, or add `requires` bounds at the call site.

| Function | Signature | Key contracts | Notes |
|----------|-----------|---------------|-------|
| `vec_sum` | `(v: Vec<i64>) -> i64` | — | No overflow guard. |
| `vec_min` / `vec_max` | `(v: Vec<i64>) -> i64` | `requires v.len() > 0` | |
| `vec_mean` | `(v: Vec<i64>) -> i64` | `requires v.len() > 0` | Integer mean. |
| `vec_dot` | `(a, b: Vec<i64>) -> i64` | `requires a.len() == b.len()` | |
| `vec_count` | `(v: Vec<i64>, target: i64) -> u64` | `ensures result <= v.len()` | Invariant `count <= i`. |
| `vec_all_in_range` | `(v: Vec<i64>, lo, hi: i64) -> bool` | `requires lo <= hi` | |
| `vec_is_sorted` | `(v: Vec<i64>) -> bool` | — | Ascending. |
| `vec_prefix_sum` | `(v: Vec<i64>) -> Vec<i64>` | `ensures result.len() == v.len()` | |
| `vec_reverse` | `(v: Vec<i64>) -> Vec<i64>` | `ensures result.len() == v.len()` | |

---
