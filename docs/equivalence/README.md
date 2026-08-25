# Equivalence Validation

Vow has three implementations of one specification:

1. the **Rust bootstrap compiler** (`target/release/vow`)
2. the **self-hosted compiler** (`build/vowc`) — a verified binary fixed point
3. the **ESBMC C model** emitted by `c_emitter` — a third semantics, used only for proofs

They are meant to agree. This directory records how thoroughly that has actually
been checked, and when.

## Why the fixed point isn't enough

`bootstrap.sh` proves `sha256(vowc2) == sha256(vowc3)`: the self-hosted compiler
reproduces *itself*, byte for byte. That is a strong property over exactly one
input — the compiler's own source. It says nothing about any construct the
compiler's source never uses.

`full_test.sh` covers more, over a fixed corpus, but it compares diagnostic
*counts* rather than error codes, and sweeps neither `benchmarks/` nor
`stdlib/` uniformly. The first full-corpus differential sweep found a total
miscompile of Euclid's algorithm in `benchmarks/medium/M13_gcd` that both
guards had sailed past for the entire life of the benchmark suite.

The prompt for this work was Google's
[Scaling Memory Safety](https://bughunters.google.com/blog/scaling-memory-safety)
pilot, which validated an LLM-assisted C→Rust rewrite of giflib three ways:
mass-scale corpus replay (30M real GIFs), differential fuzzing, and adversarial
AI review. Their differential fuzzer ran 200M iterations over six days and found
**nothing**; the corpus replay and the adversarial review found the two real
bugs. That result ordering is why the tiers below are sequenced corpus-first.

## The three tiers

| Tier | Cadence | Cost | Blocking | What runs |
|---|---|---|---|---|
| 1 | every PR | seconds | yes | promoted fixtures in `tests/` |
| 2 | nightly | ~90 min, sharded | no | full-corpus sweep (`scripts/equivalence.py`) |
| 3 | monthly | credentialed, agent-driven | no | adversarial AI pair review |

**Tier 1** is where found bugs stay fixed. Every confirmed divergence is
delta-debugged to a minimal reproducer and committed as a `tests/run/` or
`tests/error/` fixture, so the existing suite regression-guards it forever. This
tier is cheap precisely because the expensive sweeps happen elsewhere.

**Tier 2** re-establishes equivalence against a moving codebase. It is
deterministic and credential-free, so it runs unattended.

**Tier 3** is the only tier that needs model credentials and real spend, which
is why it is an agent command rather than a workflow step. It reviews matched
module pairs (`lexer.rs` ↔ `lexer.vow`, and so on) for semantic divergence, and
publishes to `docs/equivalence/<YYYY-MM>/` alongside the monthly audit's own
reports.

## The rule for tier 3

**A claimed divergence is not a finding until a discriminating `.vow` input
exists and `scripts/equivalence.py` confirms the two compilers actually disagree
on it.** The model generates hypotheses; the runner is the judge.

This is not optional rigour. The 2026-06-12 verification-honesty pass found that
half of the preceding audit's severities were overstated. An unconfirmed finding
costs more reviewer time than it saves, so a hypothesis without a runner verdict
is published as a hypothesis and excluded from the summary counts.

## Reading a report

Every run states what it did **not** cover — shards skipped, budget exhausted,
pairs deferred, files skipped and why. A sweep that silently skipped most of its
corpus otherwise reads as "all clear" when it measured almost nothing; the skip
histogram and `--min-compared` floor exist to make that impossible to miss.

Two numbers matter more than the divergence count:

- **compared** — how many files actually reached a comparison.
- **the compiler digests** — which two binaries were compared. A stale
  `build/vowc` turns the whole run into a test of last week's compiler, so
  `results.json` records the sha256 of both.

Note which self-hosted binary a run used. `build/vowc` is the verified fixed
point. A stage-1 binary (Rust-compiled `compiler/main.vow`) is a valid target
too, but a divergence found against it may live either in the self-hosted source
or in the Rust compiler's lowering of that source; only a fixed-point run
separates those.

## The ledger

`ledger.json` makes recurring runs incremental and comparable. Without it, each
monthly review would re-review every module pair from scratch — wasteful, and
worse, non-comparable run to run.

It records, per module pair, the content hash last reviewed and the outcome, so
a run re-reviews only **changed** pairs and reports the rest as explicitly
skipped. For the corpus it records which files have ever diverged, so a
regression is distinguishable from a new finding.

Schema: see `ledger.schema.json`.

## Running it

```bash
# Tier 2 — full-corpus differential sweep (deterministic, no credentials)
python3 scripts/equivalence.py --rust target/release/vow --self build/vowc

# Tier 3 — adversarial pair review (needs ANTHROPIC_API_KEY or OPENAI_API_KEY)
python3 scripts/pair_review.py --model claude-sonnet-4-20250514
```

Bootstrap first (`cargo build --all --release && scripts/bootstrap.sh --skip-cargo`): both tiers must
compare the verified fixed point, for the attribution reason above.

The monthly cadence is `/equivalence-review` (see `.claude/commands/equivalence-review.md`), which runs
both tiers, triages what they find, and publishes `report.md` + `recap.md` here.

## Index

| Month | Verdict | Report | Recap |
|---|---|---|---|

<!-- Add a row per cycle, most recent first. -->
