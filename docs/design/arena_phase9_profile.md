# Arena Phase 9 Profile Report

Date: 2026-04-29

Issue: #204, final arena-per-scope performance pass for epic #195.

## Scope

Measured the original Vow benchmark suite references: 36 non-Stretch E/M/H benchmarks from `benchmarks/manifest.toml`.

Baseline is a clean detached `main` worktree at `dc8db24`. The current measurement is the `issue204-arenas-phase-9-performance-pass` branch at the same base commit plus the Phase 9 working-tree changes.

Important caveat: current `main` already contains the arena cutover, so this is a pre-optimization vs post-optimization comparison, not a historical pre-arena comparison. The benchmark `main()` bodies are also very short; runtime differences are around process-startup noise. Treat runtime deltas as smoke signals and RSS deltas as coarse peak-memory checks.

## Method

- Built both worktrees with `cargo build --release --all`.
- For each benchmark reference:
  - compiled with `vow build --no-cache --no-verify`;
  - ran the produced executable 30 times and recorded median wall time using `time.perf_counter`;
  - recorded peak run RSS with `/usr/bin/time -f "%M"` during executable runs.
- Excluded HumanEval benchmarks and Stretch benchmarks to match the original suite named in the arena PRD.

Raw measurement artifact for this run: `/tmp/arena_phase9_profile/profile.json`.

## Runtime Regressions

Top 5 by median executable wall-time delta:

| Benchmark | Baseline | Current | Delta |
| --- | ---: | ---: | ---: |
| M13 gcd | 1.200 ms | 1.254 ms | +4.48% |
| H08 interval_overlap | 1.206 ms | 1.259 ms | +4.40% |
| M14 selection_min | 1.219 ms | 1.267 ms | +3.90% |
| E02 max_of_two | 1.228 ms | 1.270 ms | +3.46% |
| M06 vec_max | 1.218 ms | 1.257 ms | +3.27% |

Median runtime delta across the suite: -0.19%.

The largest observed runtime regression is about 0.054 ms absolute, below a meaningful threshold for these process-sized benchmark runs.

## Peak RSS Regressions

Top 5 by executable peak RSS delta:

| Benchmark | Baseline | Current | Delta |
| --- | ---: | ---: | ---: |
| H01 stack_push_pop | 2804 KB | 2996 KB | +192 KB (+6.85%) |
| M05 vec_count | 2808 KB | 2924 KB | +116 KB (+4.13%) |
| E03 min_of_two | 2740 KB | 2852 KB | +112 KB (+4.09%) |
| H03 bounded_queue | 2760 KB | 2868 KB | +108 KB (+3.91%) |
| E02 max_of_two | 2872 KB | 2976 KB | +104 KB (+3.62%) |

Median peak RSS delta across the suite: +0.63%.

The largest RSS increase is 192 KB. No benchmark showed a multi-megabyte regression.

## Applied Optimizations

- Block-local heap producers now route to `Block(defining_block)` when all direct uses stay in that block.
- `FreshInCaller` call results can route to a caller block when their uses are local.
- Marker insertion remains elided for blocks with no block-region heap producer.
- Self-hosted region inference now inserts block markers from inferred regions instead of retagging allocations independently.

## Correctness Hardening

While closing Phase 9, the self-hosted compiler had a remaining store-effect parity gap: it did not publish direct `Store` / `FieldSet` effects and therefore could not emit the Rust-side `RegionConflict` for alloc-into-param-via-callee. That gap is now covered by a source-level regression fixture.

Remaining known limitation: full region-conflict detection for cross-parameter, Phi-mixed, and complete block-tree LUB cases is still outside the current partial checker. The implemented checker covers the source-visible alloc-into-param-via-callee shape already documented in the Rust pass.
