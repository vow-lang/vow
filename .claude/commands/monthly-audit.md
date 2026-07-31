---
description: Run the recurring monthly Vow project audit — design coherence, verification soundness, self-hosted parity, test/CI health, and CEGIS-loop reality — against last month's baseline, then publish a report + recap to docs/audits/.
---

# Monthly Vow Project Audit

You are running the recurring audit of the Vow language and compiler against its own thesis: *the
compiler proves correctness, or hands an agent a counterexample it can act on, until the program is
proven correct.* Each run checks how much of that is still real, what changed since last month, and
what's newly wrong. The first full instance of this audit ran 2026-07-27 (see
`docs/audits/2026-07/report.md` once it exists, or ask the user for the transcript if this repo predates
that directory) — read it if present; it's your calibration for depth and tone, not a template to copy
verbatim.

**Configurable toggles (confirm with the user once at the top of the run if any is ambiguous, then
proceed without asking again mid-run):**
- `AUTO_FILE_ISSUES` (default: **no**) — if yes, file GitHub issues for newly-discovered defects directly;
  if no (default), list them in the recap under "Recommend filing" for the user to triage.
- `COMMIT_MODE` (default: **PR**) — if `PR`, open a dated branch + PR with the new docs for human review;
  never merge it yourself. If `direct`, commit straight to main (only use this if the user has explicitly
  pre-authorized it for this recurring job).

## Step 0 — Establish the baseline

1. Find the most recent `docs/audits/<YYYY-MM>/` directory. Read its `report.md` and `recap.md` in full.
   Note the HEAD commit it audited (recorded near the top of `report.md`).
2. `git log --oneline <last_audited_head>..HEAD | wc -l` and skim the shortlog to get a feel for the
   period's volume before diving in — a quiet month and a month with a major refactor deserve different
   depth.
3. Pull everything closed/merged/opened since that commit's date:
   `gh issue list --state all --search "updated:>=<last_audit_date>"` and the equivalent for `gh pr list`.
4. Do **not** assume anything in last month's report is still true. Every dimension below must be
   re-verified against current code and a live command run — the baseline tells you *where to look*
   and *what to compare against*, not what to believe.

## Step 1 — Fan out parallel investigations

Launch independent agents (the `Agent` tool with `general-purpose`, or the `Workflow` tool if the session
has multi-agent orchestration enabled — either is fine, but each investigation must be self-contained
with full context, since fresh agents share none of your conversation). Six standing dimensions, run in
parallel:

**A. Issue & PR tracker reality check.** List all open issues/PRs, diff against last month's list:
which closed, which opened, which are now stale (no activity, low signal). Spot-check a sample of
"probably already fixed" issues against current code exactly like a verify-and-close sweep — don't trust
issue titles. Flag anything that contradicts last month's "confirmed closed" or "confirmed open" calls.

**B. Design coherence vs CLAUDE.md's three-criteria bar.** Re-read `docs/spec/*.md` and CLAUDE.md's
Language Design Principles and Contract Authoring sections. Check whether any new feature landed this
month violates the "does not make verification harder / eliminates a class of agent bugs / makes agentic
coding easier" test. Re-check specifically whether the bounded-verifier-vs-unbounded-contract tension
(§ of the 2026-07-27 report) has moved at all — has an escape hatch shipped, has the collection-bound
assume been disclosed anywhere, has the `contract_fidelity` ratio in `benchmarks/manifest.toml` changed?
Scan for *new* internal spec contradictions the same way (grep across `docs/spec/*.md` and the shipped
`skills/vow/` bundle for self-contradicting authoring guidance) — don't just re-check the ones already
on file.

**C. Verification soundness ground truth.** Walk forward every soundness hole tracked as open in the
last report (checked-arithmetic overflow, self-hosted linear tracking, match exhaustiveness gating,
effect-violation error codes, sanitize-mode UAF, or whatever the current open list is) — for each, check
current code and state FIXED / STILL OPEN / REGRESSED with file:line evidence, not a repeat of last
month's citation without re-checking it still holds. Then spend real effort hunting for anything *new*:
a fresh soundness hole this audit hasn't seen before is worth more than re-confirming an old one.

**D. Self-hosted compiler parity & bootstrap health.** Re-run the CLAUDE.md-documented fixed-point triple
test (`concat_vow.sh` → Stage 0 → A → B → C, compare SHA-256) under `ulimit -v 2000000`, fresh-built —
do not reuse a stale `build/vowc`. Probe (compile-and-run, not grep) the same parity questions as last
time: linear double-consume, match exhaustiveness, effect/purity error codes, plus anything newly relevant
to what landed this month. Check whether mutation testing (`vowc mutants`) has been run to completion
since last month (`mutants.out/` artifacts, `docs/mutants.md` updates, relevant commits).

**E. Test, tooling, and CI health.** Run `scripts/full_test.sh`, `cargo test --all`,
`cargo clippy --all -- -D warnings`, and the staleness detector
(`scripts/check_help_coverage.py` / `generate_help.py --check`). Report exact current numbers against
last month's — flag any regression immediately as high-priority, growth-with-zero-failures as healthy.

**F. CEGIS loop & benchmark suite reality.** Run `bench/run.py validate-references` for current
pass/fail ground truth against last month's. Capture at least one live counterexample JSON by hand
(intentionally break a contract, run `vowc verify`) and assess actionability the same way as before. Check
whether a full benchmark run with a live model happened this period (new `bench/results/` or `reports/`
artifacts) — if the evidence is still stale/thin, say so again; don't let it quietly stop being flagged
just because it was flagged last time too.

## Step 2 — Reconcile, then adversarially verify anything new

- If two agents' findings conflict, resolve it yourself with a direct reproduction (compile something,
  run a command, read the exact code path) before writing either claim down. Never publish an unresolved
  contradiction — the 2026-07-27 report's `divide.vow` reconciliation is the model for this.
- For every **newly discovered** high/critical finding (not carried over from a prior report, so not yet
  battle-tested), spin up 2–3 independent skeptic sub-agents instructed to *refute by default* before
  it's written up as confirmed — mirroring the adversarial cross-check methodology in
  `docs/audit-20260610/vow-analysis.md`. A finding that survives is marked confirmed; one that doesn't is
  either dropped or kept and explicitly marked refuted, for transparency. Findings merely *carried
  forward* from last month's report only need re-verification, not a fresh adversarial panel, unless
  something about them changed.

## Step 3 — Write up

Produce two files (create `docs/audits/<YYYY-MM>/` if it doesn't exist):

**`docs/audits/<YYYY-MM>/report.md`** — the full thorough audit. Match the depth and structure of
`docs/audits/2026-07/report.md`: a scorecard/legend up top, one section per dimension above, a
prioritized "what actually blocks the dream" issue list (cross-checked, not copy-pasted from the
tracker), and a closing "paths onward" with near-term / next / strategic-fork recommendations. State the
HEAD commit and date clearly at the top — the next run depends on it.

**`docs/audits/<YYYY-MM>/recap.md`** — short (500–800 words), delta-only, for someone who read last
month's report and just wants to know what moved. Three buckets:
- **Fixed since last time** — with evidence, and correct any memory/doc that called it open.
- **Broken or regressed** — the highest-priority section; anything here should be loud.
- **New this month** — findings that didn't exist in any prior report.

Append a row to a running metrics table (test counts, benchmark pass rate, open issue count, mutation
status, bootstrap fixed-point status) so trend lines are visible across months without opening every
report.

Update `docs/audits/README.md`'s index table with the new month, one-line verdict, and links to both
files.

## Step 4 — Update project memory

Same discipline as every audit: correct any memory entries this run proved stale (wrong PR/issue status,
stale file:line citations, superseded claims), and write a fresh `audit_<YYYY_MM>` memory file summarizing
the run's key deltas, linked from `MEMORY.md`'s Audits section. Don't let the memory index grow stale the
way it did between June and July.

## Step 5 — Propose, don't act

- Never close issues, merge PRs, or push directly to `main` yourself.
- `COMMIT_MODE=PR` (default): create a dated branch (e.g. `audit/<YYYY-MM>`), commit the new/updated docs,
  open a PR for human review. Do not merge it.
- `AUTO_FILE_ISSUES=no` (default): list newly-discovered defects under "Recommend filing" in the recap
  instead of filing them.
- Reply in chat with a short verdict (2–4 sentences) and the PR link — do not paste the full report into
  chat; the docs are the deliverable.

## Scale guidance

This runs monthly and indefinitely — don't restart from zero every time. Use last month's file:line
citations as a starting hypothesis for dimension C/D, but always re-verify empirically before repeating
them; that's what keeps the report honest without re-deriving the whole codebase from scratch each cycle.
Six parallel agents is the normal footprint; scale up only if the period contained something structurally
different (a major refactor, a new subsystem, a big spec change) that warrants deeper fan-out.
