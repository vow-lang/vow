# ADR-2026-10-15-1600: Native verify workers decode the parent's lowered module

Status: accepted (issue #1776, epic #1398)

Addendum to
[ADR-2026-10-13-2212](2026-10-13-2212-native-verify-worker-sessions.md) (workers
are sessions over a task queue). Scoped exception of
[ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md): the
native verifier has no Rust twin.

## Context

A session still lowered the whole program itself (module load, parse, check,
lower, region inference) before it decided its first task, although the parent
had lowered the same program moments earlier to plan the tasks. On
`compiler/main.vow` (about 51k lines) that second frontend is the larger part of
what the native backend spends over the ESBMC one: the per-function work after
the frontend is a few hundredths of a second for each of the 40 targets, and
the frontend is 13 s of CPU. The cost model of ADR-2026-10-13-2212 was
`workers x frontend + solver time`; the gate of #1432 compares that with
ESBMC's `1 x frontend + per-function process time`, so every program was
charged one frontend more than ESBMC, and every worker held a second resident
copy of the compiler's data structures (syntax tree, type environment, lowering
state) that only the lowering needed.

## Decision

1. **The parent writes the lowered module once and workers decode it.** The
   queue directory gains `ir`, the encoding of the module the task table
   indexes into (`compiler/vc_irwire.vow`), written when the pool is laid out
   (`vc_pool_share_module`). Every worker is started with `--worker-ir <path>`
   and decodes that file instead of running the frontend.
2. **The encoding carries every field of every record** (instructions with
   their origin and region words, blocks, functions with their vow entries,
   local names and region summaries, struct and enum layouts, the string pool,
   warnings). A decoded module is the module that was encoded, so a worker
   decides exactly what a worker that lowered the program would have decided;
   the task table check of ADR-2026-10-13-2212 still runs against it.
3. **A decode failure is not a verdict.** A missing, truncated or foreign file
   (the reader never traps and refuses anything that is not exactly one complete
   encoding) makes the worker lower the program itself, as before; the only
   cost of an unreadable file is the time the frontend takes.
4. **`contracts --verify` and `test --verify` share the module the same way**
   (`vc_driver_run`).

## Consequences

- The frontend runs once per `vowc verify --backend native`, in the parent.
  Retries and replacement sessions after a timeout decode the module instead of
  paying a frontend each.
- A session holds the lowered module and nothing the lowering needed to build
  it, so the resident size of a worker drops with the syntax tree and the type
  environment.
- The worker no longer depends on the sources being readable at the moment it
  starts: it decides the module the parent planned, which closes the window in
  which an edited file could make a worker lower a different program (the task
  table check made that an exit 3, never a wrong verdict).
- The one-function `--worker-index` form, used by the wiring tests, still
  lowers the program: it is not given a module.
- Not changed: the wire format of answers, the task kinds, the statuses and
  every report.

## Measurements

See the pull request.
