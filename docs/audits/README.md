# Vow Audit Log

A recurring, monthly self-audit of the Vow language and compiler against its own thesis: the compiler
proves correctness or hands an agent a counterexample it can act on, until the program is proven correct.
Each cycle checks how much of that is real, what changed since the previous cycle, and what's newly
wrong — design coherence, verification soundness, self-hosted compiler parity, test/CI health, and the
CEGIS loop's end-to-end behavior.

Run via `/monthly-audit` (see `.claude/commands/monthly-audit.md` for the full brief). Each cycle
produces two files under `docs/audits/<YYYY-MM>/`:

- **`report.md`** — the full audit: everything checked, with evidence.
- **`recap.md`** — a short delta against the previous cycle: what got fixed, what regressed, what's new.

Predecessor: `docs/audit-20260610/vow-analysis.md` (one-off, pre-dates this recurring series).

## Index

| Month | Verdict | Report | Recap |
|---|---|---|---|
| 2026-07 | Cycle 1 (baseline). Loops verify honestly, collections don't — a bounded assume silently converts `unknown` into `proven`. Checked-arithmetic overflow and self-hosted linear tracking confirmed as real open soundness holes; match exhaustiveness found broken in both compilers. Tests/CI fully green, bootstrap fixed point holds. 278 open issues, ~40–55 already fixed but never closed. | [report](2026-07/report.md) | [recap](2026-07/recap.md) |

<!--
Add a row per cycle, most recent first, e.g.:
| 2026-08 | 2 soundness holes fixed, 1 new regression in self-hosted linear checker | [report](2026-08/report.md) | [recap](2026-08/recap.md) |
-->
