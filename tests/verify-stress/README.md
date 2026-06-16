# `tests/verify-stress/` — non-gating verifier stress bucket

Programs here are **`unverifiable-by-design`** candidates: small Vow programs
that ESBMC is expected *not* to decide within the configured budget, and that
the verifier must therefore report **distinctly from `Verified`** (a
`verify_status` of `timeout` or `unknown`, never a silent accept).

**This directory is illustrative and is NOT wired into CI or
`scripts/full_test.sh`.** Its outcomes are budget- and host-dependent (they
turn on ESBMC's unwind bound, solver, and wall-clock limits), so asserting a
fixed result would make the build flaky. The *deterministic* cousin of this
category — functions the verifier marks `Skipped` because they use a
non-modelable construct — lives in `tests/verify-skip/`, which **does** gate
(`Skipped` is fail-closed and reproducible).

It exists so the verifier-evaluation suite (issue #334) can document, and let a
developer periodically re-check by hand, that the verifier degrades *safely*
under load: it fails closed instead of falsely accepting a program it cannot
fully reason about.

## How to run (manually)

```bash
cargo build --release -p vow
for f in tests/verify-stress/*.vow; do
  echo "== $f =="
  timeout 120 ./target/release/vow verify "$f"
done
```

Inspect `status` and `verify_status`: the acceptable outcomes are `VerifyFailed`
with `verify_status` in {`timeout`, `unknown`}. A `Verified` here would mean the
program turned out to be tractable after all (move it to `tests/verify/`); a
counterexample with no `verify_status` would mean it is actually decidable as a
failure (move it to `tests/verify-fail/`).

## Contents

- `unwind_loop.vow` — a large no-invariant loop with a true, tight postcondition
  (`ensures: result == n`) that ESBMC cannot discharge without a loop invariant,
  so it exhausts its unwind budget; expected `verify_status: "unknown"`.

## Relationship to the gating suite

See `docs/verifier-eval.md` for the full picture. The gating harness
(`scripts/verify_eval.py`) covers the deterministic categories; this bucket
holds only the genuinely-intractable cases that cannot be asserted reproducibly.
