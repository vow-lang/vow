---
description: Run the recurring adversarial equivalence review — Rust vs self-hosted compiler pair review with mechanical confirmation, plus a full-corpus differential sweep — then publish a report + recap to docs/equivalence/.
---

# Adversarial Equivalence Review

You are running tier 3 of the equivalence-validation programme (`docs/equivalence/README.md`). The
question this answers is narrow and load-bearing: **are the Rust bootstrap compiler and the self-hosted
`build/vowc` actually interchangeable?** Everything downstream — freezing the bootstrap, trusting the
fixed point as evidence of correctness — rests on the answer.

The binary fixed point proves `build/vowc` reproduces *itself* over exactly one input, the compiler's own
source. It says nothing about constructs that source never uses. That gap is not theoretical: the first
full-corpus sweep found `gcd` returning 0 for every input under the self-hosted compiler (#1087), a
total miscompile of a benchmark reference that had been invisible to the fixed point and to
`full_test.sh` for the entire life of the benchmark suite.

**Configurable toggles (confirm once at the top if ambiguous, then proceed without asking again):**
- `AUTO_FILE_ISSUES` (default: **no**) — if yes, file issues for confirmed divergences; if no, list them
  in the recap under "Recommend filing".
- `COMMIT_MODE` (default: **PR**) — dated branch + PR for human review; never merge it yourself.
- `MODEL` (default: `claude-sonnet-4-20250514`) — the reviewing model.

## Step 0 — Baseline and a trustworthy compiler pair

1. Read the most recent `docs/equivalence/<YYYY-MM>/report.md` if one exists, plus
   `docs/equivalence/ledger.json`. The ledger is the memory of this job: it says which divergences are
   already tracked and which module pairs were reviewed at what content hash.
2. **Bootstrap before measuring anything.** `cargo build --all --release` then
   `scripts/bootstrap.sh --skip-cargo`. The review must compare the *verified fixed point*, not a stage-1
   binary — a divergence found against stage 1 may live in the self-hosted source *or* in the Rust
   compiler's lowering of that source, and attributing a bug to the wrong compiler is worse than not
   finding it. Record both compilers' sha256 in the report.
3. If the bootstrap fails, stop and report that. A broken bootstrap invalidates the whole run; do not
   fall back to a stage-1 binary and quietly relabel the results.

## Step 1 — Full-corpus differential sweep

```bash
python3 scripts/equivalence.py --rust target/release/vow --self build/vowc \
  --output-dir /tmp/equivalence-<YYYY-MM>
```

Exit 0 means no *new* divergences and no tracked one that silently stopped reproducing. Read
`results.json`, not just the exit code: `new_divergences`, `known_divergences`, and
`no_longer_diverging` each need a different response.

A `no_longer_diverging` entry is good news that still fails the run **by design** — it means a tracked
bug got fixed and the ledger (plus any `// TEST: known-divergence` directive) is now stale. Update both.

## Step 2 — Adversarial pair review

Plan the credentialed calls first and include the per-pair chunk counts, byte totals, oversize units and
coverage from `results.json` in the report:

```bash
python3 scripts/pair_review.py --dry-run \
  --rust target/release/vow --self build/vowc \
  --chunk-bytes 120000 \
  --output-dir /tmp/pair-review-plan-<YYYY-MM>
```

Then run the review. `--max-chunks-per-pair N` is an explicit spend cap; omitted (or 0) means all
chunks. A capped pair is reported with deferred chunks and is not stamped in the ledger.

```bash
python3 scripts/pair_review.py --model <MODEL> \
  --rust target/release/vow --self build/vowc \
  --chunk-bytes 120000 \
  --output-dir /tmp/pair-review-<YYYY-MM> \
  --update-ledger --date <YYYY-MM-DD>
```

Unchanged pairs are skipped via the ledger's content hash; pass `--all` only if you deliberately want a
full re-review. Needs `ANTHROPIC_API_KEY` (or `OPENAI_API_KEY`).

**The rule, and it is not negotiable:** a claim is a hypothesis until `scripts/equivalence.py` confirms
the two compilers disagree on a concrete program. The harness enforces this — `confirmed` / `refuted` /
`inconclusive` — and you must preserve the distinction in the report. Do not promote a hypothesis to a
finding because it sounds plausible or because a model was confident. This repo has been burned: the
2026-06-12 verification-honesty pass found half of the preceding audit's severities overstated.

Every function unit is retained whole. If one exceeds `--chunk-bytes`, the plan marks it `oversize`
instead of truncating it. If a chunk cap or model error prevents full review, report the pair's exact
coverage and deferred/error chunk indexes rather than implying full coverage; the harness deliberately
leaves that pair's prior ledger hash untouched.

### Step 2b — C-model soundness variant

Run this as a separate question. It defaults to the `c_emitter` pair and asks whether either emitter's
`__ESBMC_assume` statements narrow the verifier model below language semantics. The gate runs each
candidate through both compilers independently and confirms only `Verified` plus a debug-runtime
`VowViolation`. Soundness results do not update the equivalence ledger.

```bash
python3 scripts/pair_review.py --mode soundness --dry-run --all \
  --chunk-bytes 120000 \
  --output-dir /tmp/pair-review-soundness-plan-<YYYY-MM>

python3 scripts/pair_review.py --mode soundness --model <MODEL> --all \
  --rust target/release/vow --self build/vowc \
  --chunk-bytes 120000 \
  --output-dir /tmp/pair-review-soundness-<YYYY-MM>
```

## Step 3 — Triage each confirmed divergence

For each one, in this order:

1. **Minimize.** Delta-debug to the smallest program that still diverges. A 9-line reproducer gets fixed;
   a 40-line benchmark reference gets deferred.
2. **Attribute.** Which compiler is wrong? Check the spec (`docs/spec/`) — the answer is not always
   "self-hosted". Run the same program against both to confirm which side contradicts the spec.
3. **Check for a known issue** before filing. Search open issues; several long-standing gaps
   (e.g. #588's missing self-hosted linear tracker) will resurface every run, which is what the ledger's
   `expected` status is for.
4. **Promote a fixture.** Commit the minimal reproducer to `tests/run/` or `tests/error/`. If it fails
   under the current `build/vowc`, add `// TEST: known-divergence <issue> "<why>"` so `full_test.sh`
   reports a loud SKIP instead of turning the tree red — and so it becomes a hard FAIL the moment the
   bug is fixed, forcing the directive's removal.
5. **Complete the ledger metadata.** The harness writes complete pair-review hashes, dates and outcomes.
   You write confirmed issue numbers, plus corpus rows and fixture paths created during triage.

## Step 4 — Publish

**`docs/equivalence/<YYYY-MM>/report.md`** — the full run. State the date and both compiler sha256s at
the top; the next run depends on them. One section per module pair plus one for the corpus sweep.
Confirmed findings and hypotheses must be visually separated, with the discriminating program inline for
every confirmed one.

**`docs/equivalence/<YYYY-MM>/recap.md`** — short (400–700 words), delta-only:
- **Fixed since last time** — with evidence; correct any doc or memory that called it open.
- **Newly confirmed** — the loud section.
- **Still tracked** — one line, pointing at the ledger rather than re-litigating each entry.

Append a row to a running metrics table: files compared, new divergences, tracked divergences, pairs
reviewed, pairs skipped-unchanged, confirmed findings, hypotheses, refuted. Trend lines matter more than
any single run — a month where `compared` drops sharply is a coverage regression even if divergences are
flat.

Update `docs/equivalence/README.md` with an index row per month.

## Step 5 — Report what you did not cover

Every run states its gaps explicitly: pairs skipped as unchanged, oversize function units, chunks
deferred by a spend cap, model-error chunks, corpus files skipped and why, and any shard or budget that
ran out. A run that silently examined a fraction of the surface must never read as a clean bill of
health. `vowc mutants`' `missed.txt` convention is the model here.

## Step 6 — Propose, don't act

- Never close issues, merge PRs, or push to `main` yourself.
- `COMMIT_MODE=PR` (default): dated branch (`equivalence/<YYYY-MM>`), commit the docs and any promoted
  fixtures, open a PR, leave it for review.
- Reply in chat with a 2–4 sentence verdict and the PR link. The docs are the deliverable, not the chat
  message.

## Scale guidance

The corpus sweep is cheap (~80s over ~560 files) and should run every time. The pair review is the
expensive half; the ledger's content hashing means a quiet month costs almost nothing. Spend the budget
on `lower` and `c_emitter` when they have changed — they are the largest pairs and the two where a
divergence is most likely to be a silent miscompile rather than a diagnostic difference.
