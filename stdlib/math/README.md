# stdlib/math

Integer and vector math with overflow-guarded contracts. Three independent modules
(no cross-`use`) — copy only the file you need.

- **`arithmetic.vow`** — `abs`, `min`, `max`, `clamp`, `sign`, the `safe_add`/`safe_sub`/`safe_mul`/`safe_div`/`safe_mod` family, `pow`, `midpoint`, `diff`, `divides`, `is_even`, `is_odd`.
- **`number_theory.vow`** — `gcd`, `lcm`, `is_prime`, `power_mod`, `factorial`, `fibonacci`, `isqrt`, `largest_divisor`, `count_divisors`.
- **`vec_math.vow`** — `vec_sum`, `vec_min`, `vec_max`, `vec_mean`, `vec_dot`, `vec_count`, `vec_all_in_range`, `vec_is_sorted`, `vec_prefix_sum`, `vec_reverse`.

All functions are `pub`. Full signatures and contracts:
[docs/spec/stdlib.md#math](../../docs/spec/stdlib.md#math).

## Usage

```
ulimit -v 2000000; build/vowc build --no-verify stdlib/math/main.vow -o /tmp/math_demo && /tmp/math_demo
```
To consume in your own program, copy the single file you need next to your entry file:
```
cp stdlib/math/arithmetic.vow myproject/arithmetic.vow
```
```vow
module Main
use arithmetic
fn main() -> i32 [io] { print_i64(gcd_or_clamp()); 0 }
fn gcd_or_clamp() -> i64 { clamp(15, 0, 10) }   // 10
```

## Gotchas

- The `safe_*` functions require **non-negative** inputs (`a >= 0, b >= 0`) and
  `safe_div`/`safe_mod` require `b > 0`. They are overflow-checked helpers, not
  general signed-arithmetic wrappers.
- `pow`, `factorial`, `fibonacci`, `lcm`, and the `vec_*` summations have **no
  overflow guard** on their running result — use within ranges that cannot overflow
  `i64`, or add `requires` bounds at the call site.
- `power_mod` requires `modulus <= 3037000499` (= `isqrt(i64::MAX)`) so the
  intermediate product cannot overflow.

## Verification

`vow verify stdlib/math/main.vow` reports `VerifyFailed`. The former `abs`/`<stdlib.h>`
collision is resolved — the verifier now namespaces user functions as `vow_user_fn_<id>`,
so a function named `abs` no longer clashes with `int abs(int)`. The remaining blocker is
a genuine contract gap: `pow`'s `ensures result >= 0` is refuted by an `i64` overflow
counterexample (a large `base`/`exp` wraps negative). All contracts are still enforced at
runtime in `--mode debug`. See
[docs/spec/stdlib.md#verification-status](../../docs/spec/stdlib.md#verification-status).
