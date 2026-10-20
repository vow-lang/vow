# ADR-2026-10-20-1848: the corpus sweep compares only fixtures both backends decide

Status: proposed, needs maintainer decision (issue #1848, epic #1398)

Native verifier only (no Rust twin,
[ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md)). Touches the harness
`scripts/verify_perf.py` and its gate module `scripts/perf_gate.py`, no verifier code.

## Context

The corpus sweep (`scripts/verify_perf.py`) gates on the geometric mean and on a worst-case bound
of 1.25x, native over ESBMC, wall and RSS. It admits every row the two backends classify as
`match`. That includes rows where neither backend decides: ESBMC answers `Skipped` or a quick
`inconclusive`, native answers `inconclusive` or `Skipped`. Such a row times how fast each backend
declines, not how fast it verifies.

At the final-SHA gate for #1433 the geomeans passed by a wide margin (wall 0.078x, RSS 0.235x,
465 comparable rows) and the worst-case bound failed on eight fixtures. Seven of them are of that
kind: ESBMC declines in 3 to 7 ms; native pays process start (driver, worker re-exec, one solver
process per query, 9 to 15 ms on the tiny ones) before it can say the same. `verify_compiler.py`
has a measurement floor for this; the corpus sweep has none.

## Decision (proposed)

1. A sweep row is **like-for-like** only when both backends reach a deciding verdict (`proven` or
   `refuted`) on the same functions (class `match`, same skipped set, as before). Only those rows
   enter the geomean and the worst-case bound.
2. Every other gating row (a backend skipped or inconclusive, whichever way round, and whatever its
   class) is left out of the gate and **listed**: `undecided[]` in the JSON report and a
   `[not decided: ...]` line in the summary carry both verdicts, both wall times and their ratio.
   Nothing is dropped silently; the cost of declining stays visible.
3. Rows both backends decide are never left out. A refutation where native is slower than ESBMC
   (`fail/string_contains_not_symmetric`, 1.46x) keeps gating, and so does any native-only cost
   on a proven row.
4. Timeouts keep their own criterion over every row, and unstable rows keep their own reason; the
   rule does not touch either.
5. The thresholds (1.0 geomean, 1.25 worst) and the five-run median are unchanged.

## Consequences

- The seven tiny `skip`/`unknown` offenders leave the gate and appear in `undecided[]`. The
  worst-case bound is then decided by rows where a verdict was actually computed.
- A regression that makes native slower at *declining* is no longer a gate failure. It is
  visible in `undecided[]` and the summary; the maintainers can add a separate, generous bound
  on that list if they want one (not proposed here: absolute times of 10 to 80 ms are noise on a
  shared host).
- `verify_compiler.py` reuses `evaluate_gate`, which is unchanged; its like-for-like basis
  (per-function, both backends decided) is the same idea one level down.

## Alternatives

- Add `verify_compiler.py`'s measurement floor to the sweep: hides absolute-cost regressions on
  decided rows too; a floor is a number to argue about, a verdict is not.
- Make native cheaper on tiny fixtures: done where it was free (#1856, #1857). The rest is the
  process-per-query design (start of parent, worker and solver), not idle time.
- Leave the sweep as is and accept a red worst-case row: keeps the gate permanently failing on
  a cost that no verdict depends on.
