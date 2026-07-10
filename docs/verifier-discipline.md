# Verifier Discipline: Safe vs Unsafe Adaptive Retry

This document codifies the soundness discipline for `vow-verify`'s adaptive-retry
logic. It is the **verifier-side mirror** of the contract-authoring rule that
artificial bounds must not enter a proof obligation
([Verification-Driven Bounds (Anti-Pattern)](spec/contracts.md#verification-driven-bounds-anti-pattern)):
a contract must not be weakened to appease ESBMC, and — symmetrically — the
verifier must not weaken the *proof obligation* to manufacture a passing result.

It exists because the pattern is tempting. Bug-finding tools legitimately trade
completeness for speed (e.g. "halve the unwind bound on timeout"). Vow is a
*verifier*: a `Verified` result is a claim that the property holds for **all**
inputs within the model. Importing a bug-finder's budget-reduction trick would
silently turn that claim into "no bug found within a reduced budget," which is
not the same thing and must never be reported as `Verified`.

## The core rule

> A retry may change the solver, switch the encoding, split the obligation, or
> raise a budget. It may **downgrade** the reported status (a stronger attempt
> that fails falls back to a weaker-but-honest status). It must **never upgrade**
> a weakened check into `Verified` — i.e. a result obtained under a strictly
> weaker obligation than the original must be labeled as such, or reported as
> inconclusive, never as an unqualified proof.

## Safe retry strategies

These preserve or strengthen the obligation, or fail honestly. They are allowed:

- **Try another solver** — Boolector → Bitwuzla → Z3. Same obligation, different
  decision procedure.
- **Switch encoding BV → IR** — as `run_with_fallback` already does on a
  resource-limited BV result. The IR (integer/real) encoding does **not** model
  machine-integer overflow, so a proof under it is strictly weaker than a
  bit-vector proof and is reported under the distinct `ProvenIr` status — never
  as bare `Proven`. See "Status taxonomy" below.
- **Split verification** by function or by contract clause. Each sub-obligation
  is proved in full; nothing is dropped.
- **Increase `max_k_step`** (the incremental-BMC unwind bound) within a
  configured ceiling. A *larger* unwind is a *stronger* obligation.
- **Return a distinct weaker-encoding status** (`ProvenIr`) when overflow
  semantics differ between BV and IR.
- **Add `__ESBMC_assume` constraints only for true Vow representation
  invariants** — e.g. `Vec.len <= Vec.cap`. These describe facts that hold for
  every reachable program state; they never prune a real program behaviour.
  User-supplied bounds are **not** representation invariants and must never be
  assumed.

## Unsafe retry strategies

These weaken the obligation and then report success. They must **not** be added,
even on timeout:

- **Halve (or otherwise reduce) the unwind and report normal `Verified`.** A
  smaller unwind proves a strictly weaker property; reporting it as `Verified`
  is unsound.
- **Disable Vow-emitted `__ESBMC_assert` properties** to get a run to pass.
- **Add `__ESBMC_assume` constraints that prune real program behaviour** (any
  assumption that is not a representation invariant — especially user-derived
  numeric bounds).
- **Simplify source loops** (or any part of the program) and treat the result as
  a proof of the original.

## Status taxonomy

The status vocabulary already distinguishes a clean proof from every weaker or
inconclusive outcome, so a weakened result is never indistinguishable from a
proof. The names below are the ones actually emitted; the third column maps them
to the conceptual vocabulary used when this discipline was drafted.

| `VerificationResult` (`vow-verify/src/esbmc.rs`) | JSON `status` (`vow verify`) | Conceptual name | Meaning |
|---|---|---|---|
| `Proven`            | `proven`        | `Verified`                   | Proved under the bit-vector encoding (overflow modeled). |
| `ProvenIr`          | `proven-ir`     | `VerifiedWithEncoding(IR)`   | Proved under the weaker IR encoding after a resource-limited BV attempt; overflow **not** modeled. Distinct from `Proven` by design. |
| `Failed(cex)`       | `failed`        | —                            | Counterexample found. |
| `Timeout`           | `timeout`       | `Timeout`                    | Wall-clock cutoff; neither proved nor disproved. Never upgraded to a proof. |
| `Unknown { reason }`| `unknown`       | `Indeterminate(reason)`      | Finished cleanly but inconclusive, **or** the safe-retry ladder was exhausted. The `reason` string carries the distinction (e.g. ESBMC `VERIFICATION UNKNOWN` vs. a memory-limit hit). |
| `Skipped { reason }`| `skipped`       | —                            | Function not modelable; ESBMC not invoked. Fails closed. |
| `ToolNotFound` / `ToolError` | `error` / `tool_not_found` | — | Infrastructure failure. Fails closed. |

Notes on the taxonomy decisions:

- **No `Verified` from a budget-reduction retry.** There is no status that means
  "proved under a reduced budget but reported as a full proof." That
  combination is exactly what the core rule forbids.
- **Retry-exhaustion reuses `Unknown { reason }`** rather than adding a separate
  variant: an exhausted safe-retry ladder is inconclusive, and the structured
  `reason` distinguishes it from a raw ESBMC `VERIFICATION UNKNOWN` for tools
  that route on it.
- **Fail-closed everywhere.** `vow build` / `vow verify` / `vow contracts
  --verify` exit `1` unless every contract is `proven` or `proven-ir`. `Timeout`,
  `Unknown`, `Skipped`, and `error` all fail the run (see
  [`docs/spec/cli.md`](spec/cli.md)).

## How the existing fallback embodies the discipline

`run_with_fallback` in `vow-verify/src/solver_strategy.rs` is the only adaptive
retry today, and it is a *safe* one:

1. Run BV first.
2. If the BV result is resource-limited (`Timeout`, or a memory-limit
   `Unknown`), retry with **Z3 + IR** — a solver/encoding switch, **not** a
   budget reduction. The IR retry uses the **same** `max_k_step`.
3. An IR *proof* is relabeled `Proven → ProvenIr` (weaker-encoding status).
4. An IR *counterexample* is **discarded** (IR does not model overflow, so a
   CE found only under IR may be infeasible under BV); the original
   resource-limited result stands.
5. A non-memory `Unknown` from BV is an explicit ESBMC inconclusive verdict and
   is **never** retried into a proof.

## Enforcement

The discipline is enforced in code, not just documented, so a future edit cannot
silently regress it:

- **`enforce_retry_never_launders_proof`** (`solver_strategy.rs`) is an always-on
  invariant: the resource-limited retry may never return a bare `Proven`. A
  proof found under the weaker IR encoding must be `ProvenIr`. A violation
  panics; the CLI surfaces the panic as `verify_status: "panicked"` and the run
  fails closed — it never reports `Verified`.
- **The unwind assertion** in the same branch asserts the IR retry's
  `max_k_step` is never below the BV attempt's, so a future "halve unwind on
  timeout" edit trips immediately.
- **Regression tests** (`solver_strategy.rs`, `#[cfg(unix)]` fake-esbmc):
  - `fallback_forced_timeout_never_verified` — a forced timeout on both BV and
    IR yields `Timeout`, never a proof.
  - `fallback_forced_timeout_ir_proof_is_labeled_proven_ir` — a BV timeout with
    an IR proof yields `ProvenIr`, never bare `Proven`.
  - `fallback_auto_bv_unknown_does_not_retry_ir` — a non-memory `Unknown` is not
    laundered by the IR retry.

The self-hosted compiler mirrors the same discipline: its IR fallback
(`compiler/verifier.vow`) relabels an IR proof to `VERIFY_PROVEN_IR`, and its
retry sites (`compiler/main.vow`) accept only a proof from the retry and treat a
non-memory `UNKNOWN` as a failure.

## Adding a new retry strategy

Before adding any retry, verify it against the two lists above. If it changes the
obligation at all, the burden is on the change to prove it only *strengthens* the
obligation or *downgrades* the reported status. If in doubt, report
`Unknown { reason }` — an honest "don't know" is always sound; a false
`Verified` is never acceptable.
