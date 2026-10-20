# ADR: the compiler cost gate scores like-for-like

Date: 2026-10-20
Status: accepted (maintainer ruling on #1803)
Part of epic #1398; enables closing #1432.

## Context

`scripts/verify_compiler.py` measures native verification cost against ESBMC over the 11
compiler roots. On roots where ESBMC skips many functions as unmodelable
(`module_io.vow`: 25 skipped, 15 absent, 7 of 48 functions compared), whole-root wall/RSS
ratios measure how much of the root ESBMC declines, not the native checker's speed: at
`cb747e77` module_io is 1.066x wall like-for-like but ~15x whole-root, because native
proves 41 functions whose summaries ESBMC-skipped callers rest on.

## Decision

- Compared set: only functions both backends decide (verdict parity).
- Wave-proof cost: a proof wave run for root R's callees is counted against R (the #1809
  attribution), never left out of a compared root's cost.
- Left-out functions (ESBMC inconclusive or skipped where native decides) are listed per
  root with both verdicts and times; they are context, never part of a ratio.
- Whole-root wall/RSS are reported per root as context and never gate.
- Bounds unchanged: geomean <= 1.0, worst-root <= 1.25x wall and RSS, five-run median,
  cache off, quiet-host windows (other-runnable p95 <= 6), re-scored at the tagged SHA
  (the #1433 final-SHA process).

## Consequences

- module_io's margin is thin (1.066x at `cb747e77`); cost regressions on that root fail
  `worst_wall` and are release-blocking for the native-default flip (#1433).
- The basis is re-affirmed if ESBMC's skip set changes materially (a newer ESBMC may
  model more functions, moving the compared set).
