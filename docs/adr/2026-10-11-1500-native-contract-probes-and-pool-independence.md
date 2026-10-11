# ADR-2026-10-11-1500: Native weak-contract probes and an independent pool

Status: accepted (issue #1427, epic #1398)

## Context

`vowc contracts --verify` and `vowc test --verify` had no native backend: they
ran ESBMC through the C emitter. P4 of the epic needs both on the native pool
with the status vocabulary, schemas and exit codes unchanged. Two things in the
way: the pool stopped at the first function that was not proven (right for
`verify`, wrong for a clause report that spans functions), and the two
weak-contract probes (`vacuous`, `trivially_satisfiable`) were C-model rewrites
(`vow_reach` label, body-replace) with no native counterpart.

## Decisions

### 1. Probes are IR rewrites fed to the unchanged engine

`compiler/vc_probe.vow` holds two pure `IrFunction -> IrFunction` transforms.
`vc_verify_function_mode` (slicing, inlining, loops, induction, solver) then
decides them like any function, so the executor and the term simplifier (the
trusted base, ADR-2026-10-10-2041) gain no probe-specific claim kind.

- *Vacuity*: cut right after the last `requires` (found by reverse postorder, so
  a `requires` lowered to a join block by `&&` is covered), append a claim on
  `false`, end in an internal cut terminator, drop blocks the cut makes
  unreachable. `proven` means the `requires` are jointly unsatisfiable.
- *Trivial*: redirect every read of the single returned value to a fresh default
  constant placed right after its definition (after the leading Phis when it is
  one). The original computation stays, so its abort claims stay: this is what
  the C model's overwrite-after-emit does. `proven` means a body that ignores
  its inputs satisfies every `ensures`.

Only `proven` sets a flag: an undecided probe never claims a weakness (the same
one-sidedness as the ESBMC path).

### 2. Probes are separate pool tasks

A probe runs in its own worker with its own `--timeout` budget. Inside the main
worker a slow probe would turn a finished verdict into `timeout`, because the
parent kills the whole group at the budget. It costs one more front-end
lowering per probe; only functions that pass the cheap IR applicability test
get one.

### 3. `VcPool` gets `halt`, per-task `kinds` and `module_root`

`halt` (default on) keeps `verify`'s early stop; `contracts` and `test` clear
it, so a failure never cancels a later function (the "Pool early stop" item of
ADR-2026-10-11-1200 assigned to this issue). `kinds` selects verify, vacuity or
trivial per task. `module_root` is forwarded because `test` lowers with an
explicit module root and the worker must see the same module graph.

### 4. `test --verify --backend native` is fail-closed

Every function with a `vow` block must be `proven`. The ESBMC path treats an
`error` outcome as a pass; the native path does not, because nothing was
established. This is a deliberate, documented divergence. `--backend native`
without `--verify` is a usage error on `contracts` and `test`, so a report of
`not_verified` is never read as "the native backend ran".

## Consequences

- The ESBMC path's vacuity label is only planted in the entry block, so it
  reports `vacuous` for a satisfiable `requires: a > 0 && a < 100`. The native
  probe follows the semantic definition and disagrees; the ESBMC behaviour is a
  false positive and is documented as a verdict divergence in `docs/spec/cli.md`.
- Each task re-lowers the program: up to three lowerings per contracted
  function. A worker that runs several tasks is a follow-up if the #1420
  performance gate flags it.
- Trusted base unchanged: the rewrites only construct IR the existing gate
  accepts; a wrong rewrite can only mislabel a contract as weak or not weak,
  never make `verify` accept a failing function.
