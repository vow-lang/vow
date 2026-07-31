# HumanEval Removal and Semantic Contract Cleanup

**Status:** Approved for implementation

**Date:** 2026-07-30

## Context

The repository contains 67 generated HumanEval benchmark directories, two
HumanEval import scripts, HumanEval-specific benchmark-runner and reporting
branches, and active documentation claims based on their results. The imported
contracts were not authored from Vow's semantic-contract principles. In
particular, the generator added fixed numeric and collection-length
preconditions to keep ESBMC verification tractable.

Those preconditions conflict with Vow's established rule: a contract describes
the function's real domain and result, independently of the verifier used to
check it. A stronger verifier must be usable without editing Vow source
contracts.

HumanEval was not an intentional product dependency and does not justify the
contract noise or benchmark-reporting complexity it introduced. The live
HumanEval integration will therefore be removed rather than repaired.

## Goals

1. Remove the live HumanEval corpus and every HumanEval-specific execution,
   translation, manifest, and reporting path.
2. Preserve historical records while making it explicit that HumanEval was a
   retired experiment and is no longer part of the repository's supported
   benchmark surface.
3. State the semantic-contract rule consistently in authoritative repository
   guidance and remove active documentation that teaches the opposite rule.
4. Remove verifier-driven bounds from the remaining examples, tests, and
   original 40-benchmark suite without weakening their functional intent.
5. Rebaseline honest benchmark expectations when ESBMC cannot establish the
   corrected contract.
6. Publish the work as small commits in one draft pull request.

## Non-Goals

- Do not rename `Verified` or per-contract `proven` statuses.
- Do not add proof-scope metadata in this change. That remains the subject of
  issue #740.
- Do not change collection-capacity counterexample classification. That remains
  the subject of issue #647.
- Do not add a heuristic or allowlist-based contract-bound regression checker.
  Semantic intent cannot be inferred reliably from a numeric comparison.
- Do not delete or rewrite historical audit findings as if HumanEval never
  existed.
- Do not strengthen unrelated contracts or redesign the benchmark framework.

## Contract Rule

Contracts may refer to lengths, capacities, indices, and struct fields when the
predicate is part of the function's actual semantic domain or result.

Examples of valid functional constraints include:

- an index is within a collection's length;
- an input is nonempty when an algorithm such as `min` requires an element;
- two vectors have equal lengths for a pairwise operation;
- a data structure satisfies a real representation invariant;
- a ring buffer is not full before a write;
- arithmetic inputs are bounded at the exact overflow boundary.

The following are forbidden:

- a collection length is capped because the verifier models only a fixed number
  of elements;
- a struct's capacity or size is capped to reduce verifier state space;
- an input is capped to fit an unwind count;
- an arbitrary numeric range is added to make SMT solving faster;
- a tautological representation fact such as `Vec.len() >= 0` is presented as a
  functional precondition.

The deciding test is backend independence: replacing ESBMC with a stronger
verifier must not require a source-contract edit.

This rule will be recorded in `CLAUDE.md`, its `AGENTS.md` mirror,
`docs/spec/contracts.md`, and
`docs/design/verifier-model-bounds.md`. Active examples and generated skill
mirrors must agree with it.

## HumanEval Removal

The implementation will:

1. Delete `benchmarks/humaneval/`, including all 67 benchmark directories and
   `triage.toml`.
2. Delete `bench/triage_humaneval.py` and `bench/translate_dafny.py`.
3. Remove HumanEval entries from `benchmarks/manifest.toml` and restore its
   totals to the original 40-benchmark suite.
4. Remove `--suite humaneval` and HumanEval filtering and summary fields from
   `bench/run.py`. With only one supported suite, the `--suite` option itself is
   unnecessary and will be removed.
5. Remove the HumanEval fidelity table and HumanEval result handling from
   `bench/report.py`.
6. Update active benchmark, roadmap, comparison, and repository guidance so
   current claims describe only the supported 40-benchmark suite.

Historical material will be handled differently:

- Completed roadmap phases will remain as historical records but receive a
  clear note that the experiment and its repository artifacts were retired on
  2026-07-30.
- Audit reports will not have their original evidence rewritten. A brief
  disposition note will state that the live HumanEval integration was retired
  on 2026-07-30.
- Historical result files are already ignored and are not part of the tracked
  removal.

Issues #665 and #666 are resolved by removing the unsupported suite and its
combined reporting path. The pull request will close them.

## Remaining Contract Cleanup

The original 40 benchmarks, active examples, and verification fixtures will be
reviewed clause by clause.

For each numeric or structural bound:

1. Keep it when the benchmark or implementation is semantically undefined or
   incorrect without it, including exact overflow, indexing, nonempty-input,
   state-domain, and representation constraints.
2. Remove it when its only purpose is ESBMC unwind, collection-model capacity,
   or solver tractability.
3. Remove tautological validity clauses that express no callable-domain
   restriction.
4. Update the corresponding `spec.md`, `skeleton.vow`, and `reference.vow`
   together.

Removing a verifier-driven precondition may make a reference implementation
inconclusive under the current ESBMC strategy. In that case:

- keep the truthful contract;
- set `expected_status = "Stretch"` so the benchmark is outside the scored
  verified denominator using the framework's existing status mechanism;
- document the verifier limitation in benchmark metadata or prose, not in the
  Vow contract.

The `string_push_str_in_bounds` fixture will no longer use the verifier's
256-byte String model capacity as a source precondition. It will become a
zero-argument vowed helper that constructs concrete strings, concatenates them,
and specifies the resulting semantic length. This keeps coverage of
`String::push_str` without turning model capacity into the helper's domain.

The worked Vec-fill example will lose `requires: n <= 8` and the prose that
recommends it. Its recorded output will be re-measured rather than assumed.

## Generated Documentation

Changes to `docs/spec/` require regeneration through:

```bash
uv run python scripts/generate_help.py
```

Generated Rust help, self-hosted help, and `skills/vow/` mirrors will be
reviewed as generated outputs. No generated copy will be edited independently.

## Issue Handling

Before creating a GitHub issue, search open and closed issues for the same root
cause.

- Update #740 with the decision that `Verified` remains acceptable for now and
  that a later design may choose either `ModelVerified` or structured
  solver/bounds metadata.
- Leave #647 scoped to model-capacity counterexample classification.
- Close #665 and #666 through the removal pull request.
- Create a new issue only for a newly discovered problem that is outside this
  implementation scope and has no existing tracker.

GitHub issues are not a substitute for completing in-scope cleanup.

## Commit Structure

The implementation will use these reviewable slices:

1. Design specification.
2. Remove HumanEval corpus and integration.
3. Clarify authoritative semantic-contract guidance and regenerate mirrors.
4. Clean remaining verifier-driven contracts and rebaseline benchmark
   expectations.
5. Focused follow-up fixes discovered by validation, only when they are direct
   consequences of the preceding slices.

Unrelated cleanup will not be included.

## Validation

Validation proceeds from inexpensive structural checks to the full gate:

1. Confirm no live HumanEval integration remains outside explicitly historical
   documents.
2. Load `benchmarks/manifest.toml` and confirm it contains 40 entries with no
   `HE` identifiers or `humaneval/` paths.
3. Run the benchmark CLI help and manifest-loading paths through the `bench`
   environment.
4. Verify each changed benchmark reference individually and record its honest
   status.
5. Run focused verification fixtures affected by contract changes through both
   Rust and self-hosted compilers.
6. Run `uv run python scripts/generate_help.py --check`.
7. Run:

   ```bash
   cargo fmt --all
   cargo clippy --all -- -D warnings
   cargo test --all
   ```

8. Rebuild the self-hosted compiler:

   ```bash
   cargo build --release --all
   ulimit -v 2000000; scripts/bootstrap.sh --skip-cargo
   ```

9. Run the authoritative full gate:

   ```bash
   ulimit -v 2000000; scripts/full_test.sh
   ```

If the full gate fails for an unrelated baseline reason, compare the exact
failure set with `origin/main` and report the evidence rather than claiming a
clean result.

## Completion Criteria

The work is complete when:

- the repository has no live HumanEval corpus or dedicated integration;
- active documentation and benchmark claims describe the remaining 40
  benchmarks accurately;
- authoritative contract guidance contains no contradiction about
  verifier-driven bounds;
- remaining changed contracts express functional or exact safety constraints;
- verifier limitations appear in benchmark classification or prose, not source
  preconditions;
- generated documentation is synchronized;
- focused and full validation results are recorded honestly;
- existing relevant issues are updated without duplicates;
- the branch is pushed and one draft pull request is open against `main`.
