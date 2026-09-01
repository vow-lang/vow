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

`full_test.sh` covers more over a fixed corpus. It compares diagnostic
error-code/blame multisets and source-level counterexample values, but sweeps
neither `benchmarks/` nor `stdlib/` uniformly. The first full-corpus
differential sweep found a total miscompile of Euclid's algorithm in
`benchmarks/medium/M13_gcd` that both guards had sailed past for the entire
life of the benchmark suite.

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
| 1 | push to `main`, nightly, local on demand | minutes | yes where run | promoted fixtures plus `vow test compiler/` under both compilers |
| 2 | nightly | ~90 min, sharded | no | full-corpus sweep (`scripts/equivalence.py`) |
| 3 | monthly | credentialed, agent-driven | no | adversarial AI pair review |

**Tier 1** is where found bugs stay fixed. Every confirmed divergence is
delta-debugged to a minimal reproducer and committed as a `tests/run/` or
`tests/error/` fixture, so the existing suite regression-guards it forever. This
tier now runs automatically after every code-bearing push to `main` and
nightly: `full-test.yml` runs the complete `scripts/full_test.sh`
promoted-fixture parity harness, while the Linux `bootstrap.yml` job
independently runs the two-compiler `vow test compiler/` comparison. Both jobs
fail when their Tier-1 comparison fails, but neither is on the pull-request
path. The compiler comparison covers the whole documented `vow test` contract
— every field in
`docs/spec/schemas/test-result.schema.json` except the wall-clock `duration_ms`
and `tests[].diagnostics`, read back out of the schema so a field added there is
gated automatically. It also validates each document against that schema in
absolute terms, since parity alone cannot see a field both compilers dropped or
corrupted the same way. `diagnostics` is excluded because it is a live
divergence, not a tolerated one: the Rust runner attaches each entry's compile
diagnostics while the self-hosted runner emits none (#1183), and the schema for
them has drifted from both emitters (#1184). Both must close before the field
can be gated.

The self-hosted suite produced no JSON before a 45-minute bound in one fresh
concatenated run; the full Section 10b later completed both compilers plus its
interface checks in 17.8 minutes. Both measurements came from a contended
development host and are placement evidence, not clean benchmarks. #1171 tracks
the performance gap that prevents responsible per-PR placement. Until that gap
closes, Tier 1 catches a regression on the first `main` push that includes it
rather than adding roughly 40 minutes to every pull request.

**Tier 2** re-establishes equivalence against a moving codebase. It is
deterministic and credential-free, so it runs unattended.

**Tier 3** is the only tier that needs model credentials and real spend, which
is why it is an agent command rather than a workflow step. It reviews matched
module pairs (`lexer.rs` ↔ `lexer.vow`, and so on) for semantic divergence, and
publishes to `docs/equivalence/<YYYY-MM>/` alongside the monthly audit's own
reports. Sources are split into whole function items and packed into bounded
prompts; no source tail is truncated, and a matched Rust/self-hosted pair always
shares a chunk so the model always has both sides in front of it. An oversize
group remains whole and is reported explicitly, while an operator-imposed chunk
cap lowers the reported coverage and leaves the remaining chunks visibly
deferred. Rust `#[cfg(test)]` items are excluded — two thirds of the Rust units
in the declared pairs are tests with no self-hosted counterpart.

Everything that is not a function item — a file's `struct`, `enum` and `impl`
declarations — goes to that file's preamble, which is repeated in every chunk of
the pair. Cutting from one `fn` token to the next would instead file `struct
Checker` and `impl Checker` under whichever function happened to precede them,
and the packer would then put that function in a different chunk from the
methods those declarations govern.

Counterparts are matched on name, allowing for the two conventions the compilers
differ by: a receiver prefix the self-hosted side spells out (`lctx_merge_inst_ty`
↔ `merge_inst_ty`) and a trailing qualifier one side adds (`lower_requires` ↔
`lower_requires_clauses`). Exact names are claimed first, across every unit, so
an approximate match can never strand an exact one. What is left over is real
asymmetry between the two implementations, and the run reports it: alongside
`coverage` and `paired_coverage` (both chunk-level), `matched_coverage` is the
share of unit bytes that sat beside their own counterpart, and `unmatched_units`
names every function that did not. A chunk of 80 Rust units next to 3
self-hosted ones is fully `paired` and almost entirely unmatched — only the
third figure says so.

The same machinery has a separate `soundness` mode for the two C emitters. It
asks whether an emitted `__ESBMC_assume` narrows the verifier model below what
Vow permits, then confirms candidates with the verifier-vs-debug-runtime gate.
That is a model-vs-language question, not a Rust-vs-self-hosted equivalence
outcome, so soundness runs never update the pair ledger — nor read it, since a
pair an equivalence run stamped has not been asked the soundness question.

## The rule for tier 3

**A claimed divergence is not a finding until a discriminating `.vow` input
exists and `scripts/equivalence.py` confirms the two compilers actually disagree
on it.** The model generates hypotheses; the runner is the judge.

This is not optional rigour. The 2026-06-12 verification-honesty pass found that
half of the preceding audit's severities were overstated. An unconfirmed finding
costs more reviewer time than it saves, so a hypothesis without a runner verdict
is published as a hypothesis and excluded from the summary counts.

"Disagree" is read strictly. The runner files a panic or signal death against
each side independently, because a crash is a bug whatever the peer did — but a
program that crashes *both* compilers is agreement, and the pair review records
it as inconclusive rather than letting any crashing input clear the gate.

CONFIRMED is a claim about the program, not about the pair. It means the two
compilers disagree on the supplied input — a global divergence found *during*
that pair's review, filed under it because that is where it surfaced. Every
program traverses every stage, and every observable the runner compares is
end-to-end CLI behaviour, so nothing mechanical can attribute a divergence to
the lexer rather than the lowerer. CONFIRMED therefore validates neither the
stage nor the mechanism the model claimed, only that the disagreement is real.
Locating it is a triage step for a human, and `confirmed_issues` is where that
conclusion lands.

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

Pair rows are written atomically by `pair_review.py --update-ledger --date
<YYYY-MM-DD>` after a complete, error-free equivalence review. A deferred or
errored pair keeps its prior hash and date, forcing the next run to revisit it.
The harness preserves the corpus rows and confirmed issue numbers; operators
still add issue and promoted-fixture metadata during triage.

Schema: see `ledger.schema.json`.

### Who writes it

Tier 2 reads the committed ledger and emits `ledger.proposed.json` into every
shard artifact. The workflow deliberately retains `contents: read`, so this is
a reviewable proposal rather than an unattended commit. Each shard proposal
contains that shard's corpus findings while preserving all untouched corpus
entries and the entire module-pair block.

A shard that misses its `--min-compared` floor emits no proposal at all, and
says so in its summary. A run that measured almost nothing must not hand back a
file that looks applicable. The `updated` stamp comes from `--today` (default:
UTC today) rather than local time, so re-running a sweep reproduces its
proposal byte-for-byte.

Tier 3 runs an unsharded sweep, applies its `ledger.proposed.json`, adds the
issue/fixture context that requires judgement, and commits the result together
with the module-pair hashes only the adversarial review can compute. Applying a
nightly result instead requires merging the four shard proposals; no individual
shard claims coverage of the other three.

### Tier-1 parity suppressions

Known `diagnostics[].error_code` divergences in the compile-error comparator
(`compare_error`, the `tests/error/` suite) use the corpus entries in
`ledger.json`. An entry suppresses only the observable it names and only while
its status is `open` or `expected`; agreement becomes a hard failure that
requires marking the entry fixed, and a divergence on a fixed entry is a
regression. Active error-code entries also pin the exact sorted Rust and
self-hosted code multisets; a different mismatch on the same fixture is a new
failure, not an inherited exception. This lets the per-fixture parity harness
and the Tier-2 sweep share one registry. `compare_json` has no ledger path: a
fixture that reaches it must agree outright.

Known `counterexamples[].values` divergences instead use a fixture-local
`// TEST: known-cex-divergence <issue> "<why>" rust-name=<name>
self-name=<name>` directive. The Tier-2 runner does not emit a
counterexample-values observable, so adding those gaps to the ledger would make
`reconcile()` incorrectly report them as fixed on every nightly run. The
directive is scoped to one declared source-label rename: after applying that
rename, every source-level value map must agree exactly. A changed value or any
additional mismatch remains a failure. The directive reports as a loud skip
while that exact rename reproduces and becomes a hard failure once it agrees so
stale directives cannot accumulate.

Every `compare_json` path enforces the expected counterexample count: zero for
soft verification failures and equal between compilers otherwise. Because the
values directive is scoped to values only, it cannot mask a count divergence.
Known count divergences use the separate fixture-local
`// TEST: known-cex-count-divergence <issue> "<why>"` directive. It applies only
to a hard `VerifyFailed` count mismatch, cannot cover any per-counterexample
field mismatch, and becomes a hard failure once the counts agree.

Two suppressions are unconditional rather than per-fixture, so they are the
ones to revisit first when widening the comparison:

- **`compare_json` compares no diagnostics at all when `status` is
  `VerifyFailed`.** This predates the comparator extraction and currently masks
  #1138 — the self-hosted compiler emits an empty `diagnostics[]` for
  `VerifyFailed` while Rust emits one entry per counterexample. Because that
  gap covers the whole `tests/verify-fail/` suite, the `error_code`/`blame`
  multiset is exercised only outside `VerifyFailed`. Dropping the guard is
  blocked on #1138, the same ordering constraint #1136 records for spans.
- **Counterexample value names prefixed `$esbmc$` are dropped before
  comparison.** These are ESBMC's own temporaries, not the agent-facing CEGIS
  payload the two compilers owe each other; `vowc --help` documents the prefix
  as internal. Everything without the prefix is compared exactly.

## Running it

```bash
# Tier 2 — full-corpus differential sweep (deterministic, no credentials)
python3 scripts/equivalence.py --rust target/release/vow --self build/vowc \
  --emit-ledger-update

# Tier 3 — adversarial pair review (needs ANTHROPIC_API_KEY or OPENAI_API_KEY)
python3 scripts/pair_review.py --dry-run --all
python3 scripts/pair_review.py --model claude-sonnet-4-20250514 \
  --update-ledger --date <YYYY-MM-DD>

# Separate verifier-model soundness question (c_emitter pair only)
python3 scripts/pair_review.py --mode soundness --dry-run
python3 scripts/pair_review.py --mode soundness \
  --model claude-sonnet-4-20250514
```

Bootstrap first (`cargo build --all --release && scripts/bootstrap.sh --skip-cargo`): both tiers must
compare the verified fixed point, for the attribution reason above.

The monthly cadence is `/equivalence-review` (see `.claude/commands/equivalence-review.md`), which runs
both tiers, triages what they find, and publishes `report.md` + `recap.md` here.

## Index

| Month | Verdict | Report | Recap |
|---|---|---|---|

<!-- Add a row per cycle, most recent first. -->
