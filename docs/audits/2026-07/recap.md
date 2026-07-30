# Recap — Cycle 1 (2026-07-27)

This is the first cycle of the recurring monthly audit — there's no prior cycle to diff against, so this
recap establishes the baseline that Cycle 2 will report changes against. Full detail: `report.md`.

## Baseline established

**Headline metrics** (Cycle 2 onward will append a row here each month):

| Cycle | Date | HEAD | Tests (`full_test.sh`) | `cargo test` | Bootstrap fixed point | Open issues | Benchmark verify | Mutation last run |
|---|---|---|---|---|---|---|---|---|
| 1 | 2026-07-27 | `3cbfe87b` | 602 / 0 / 3 | 1235 / 0 / 2 | ✅ holds | 278 (0 PRs) | 80/103 | never completed |

**Top open findings to track from here** (see `report.md` §6 for the full cross-checked list):
- #585 — checked-arithmetic overflow is a verifier no-op (most-corroborated finding this cycle)
- #588 — self-hosted has no linear consume-once tracker
- #609, #632, #589/#656, #590 — CEGIS-fidelity and soundness gaps, still open
- #467 — bounded quantifiers, the highest-value missing contract-expressiveness feature
- #617 — mutation-testing oracle has no baseline check, and no run has ever completed

**Recommend filing** (found this cycle, not yet tracked anywhere):
1. Rust's `ErrorCounter` (`vow-types/src/check.rs:206`) doesn't gate on `exhaustiveness::check_exhaustive`
   — a non-exhaustive match prints an error but the build still exits 0 and ships the executable.
2. Self-hosted compiler has zero match-exhaustiveness checking (silent, not just unwrapped).
3. `checker.vow:2382` misreports effect/purity violations as `TypeMismatch` instead of `EffectViolation`.
4. Static `vow verify` never checks calls made from uncontracted functions (e.g. `main()`) into
   contracted ones — the project's own `examples/divide.vow` doesn't statically verify what it's shown to
   demonstrate.

**Corrections applied to prior assumptions this cycle** (see `report.md` §6): PRs #887, #898, and #776
were believed open in project notes — all three are merged. #855 (the `CTY_I32`/literal-marker collision)
is fixed in both compilers. Issue #583 should stay open but re-scoped (Rust fixed, self-hosted crashes on
the same repro instead of diagnosing it); #661 should be downgraded (defense-in-depth, not a live
fail-open).

**Strategic question carried forward:** the collection-verification bound (`__ESBMC_assume(v.len <= 128)`
silently turning an honest `unknown` into a domain-restricted `proven`) is the deepest issue found this
cycle — see `report.md` §1 and §8 for the three-way fork on how to address it. Cycle 2 should check
whether any decision has been made on this before re-deriving the whole analysis.
