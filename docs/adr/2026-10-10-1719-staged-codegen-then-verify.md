# `build` finishes codegen and linking before verification starts

**Status:** accepted (2026-10-10)

Resolves #179.

## Context

`vow build` and `vowc build` used to start verification while codegen was
still running. The Rust driver spawned the verify thread before Cranelift
started and joined it after linking. The self-hosted driver launched ESBMC
processes in `build_verify_phase` and drained the in-flight ones after
codegen. Any overlap stacks the in-process Cranelift working set on top of up
to `--verify-jobs` ESBMC children, and the Rust driver also kept the backend
and compiled module alive through the whole verify phase.

## Decision

Both drivers run codegen, then link, then verification:

- Rust: `run_pipeline_from_frontend` builds the verify work as a closure and
  runs it on its dedicated thread (keeping the fail-closed `JoinError` path of
  #413) only after `link_obj`. Codegen lives in `codegen_to_object`, so the
  `CraneliftBackend` and compiled module are dropped before linking.
- Self-hosted: `run_build_cmd` calls `build_verify_phase` and
  `build_drain_verify_phase` after `build_codegen_phase`.

A codegen or link failure now ends the build before any ESBMC process starts.
The upfront "ESBMC not found" check stays before codegen.

## Alternatives

- **Bounded overlap** (allow N ESBMC jobs while codegen runs). Rejected: any
  overlap still stacks the two working sets, and the cap is a tuning knob plus
  a scheduler with no determinism benefit.
- **Codegen in a child process** or `malloc_trim` after `__vow_clif_finish`.
  Not bundled: staging cannot return pages the allocator retains, and whether
  that matters needs its own measurement.

## Consequences

- **Wall clock** is `codegen + verify` instead of `max(codegen, verify)`.
- **Diagnostics.** The self-hosted driver no longer emits verifier skip
  warnings in the `CompileFailed` JSON of a codegen failure, because
  verification never ran. This matches the Rust driver, which discards them.
  `status` and exit codes are unchanged.
- **`--perfetto`.** The `verification` and proof spans start after the
  `codegen` and `link` spans end. Names and tracks are unchanged.
- ESBMC input is unchanged: same IR, same emitter, same launch order within
  the verify phase.

## Measurement

`scripts/measure_build_tree_rss.py` samples the summed RSS of the build
process and its descendants (`time -v` and `ru_maxrss` only see the largest
single process, so they cannot show this). `build --no-cache --verify-jobs 2
compiler/main.vow`, one sample each, ESBMC 8.3.0, pre-change binaries built
from `b3da4bad`:

| compiler    | version | peak tree RSS | driver RSS | wall    |
|-------------|---------|---------------|------------|---------|
| Rust        | before  | 4.59 GB       | 150 MB     | 98.0 s  |
| Rust        | after   | 4.69 GB       | 148 MB     | 108.6 s |
| self-hosted | before  | 4.63 GB       | 382 MB     | 169.7 s |
| self-hosted | after   | 4.61 GB       | 360 MB     | 178.4 s |

The peak tree RSS does not move on this program. It is set by two concurrent
ESBMC children late in verification (at 85 s and 106 s of the Rust runs),
long after codegen finished under either ordering, and the driver's own RSS
is a few hundred MB. Staging therefore removes the possibility of
codegen-plus-ESBMC stacking rather than lowering the peak of this workload; it
costs roughly 9-10 s (about 9%) of wall clock here. Programs whose codegen
working set is large relative to their ESBMC children are where it pays off.
The self-hosted "before" driver already behaved almost sequentially at
`--verify-jobs 2`, because a full pool blocks in `build_verify_phase` before
codegen starts.

## Guard

`vow/tests/build_stage_order.rs` and `tests/build-staging/tests.sh` put a fake
`esbmc` first on `PATH` and assert that every launch sees the output
executable already linked, and that a codegen failure never launches it. There
is deliberately no RSS bound: the number depends on the machine and the ESBMC
version.
