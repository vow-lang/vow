# The native verifier is checked against the runtime and against Z3 in CI only

**Status:** accepted (2026-10-11)

Part of epic #1398, issue #1431. Builds on the verifier-versus-runtime
differential (#1084, `scripts/verifier_runtime.py`) and on
[ADR-2026-10-11-1200](2026-10-11-1200-query-slicing-and-per-clause-verdicts.md)
(slice, then confirm in full).

## Context

The native verifier trusts one solver. A Bitwuzla bug, or an encoding that only
Bitwuzla happens to accept, becomes a false proof, and the product has no second
opinion. The C model had the same exposure to ESBMC; the differential against the
concrete runtime (#1084) was the check on it, and it has to follow the verifier
to the native backend. Z3 is a different solver family and an independent
implementation of QF_BV/QF_ABV, which is what a cross-check needs.

## Decision

1. **Z3 is a CI dependency, never a product one.** Nothing under `compiler/` or
   `vow-*` shells out to it, no release job installs it, and the install action
   (`.github/actions/install-z3`) is used by `native-oracle.yml` only
   (`scripts/test_install_z3_action.py` pins that).
2. **Queries are captured with a `bitwuzla` shim on `PATH`, not with a product
   knob.** The native driver finds the solver by `path_lookup("bitwuzla")`. The
   shim copies the `.smt2` file it is handed and then `exec`s the real solver
   with the same argv, so the product's watchdog, output and exit code are the
   real solver's and no orphan process is left. The comparison itself re-runs
   each captured file under the pinned Bitwuzla and under Z3 offline. This is
   the technique of `scripts/parity_c.py`'s fake `esbmc`, and it keeps the
   cross-check out of the compiler: no dual-compiler change, no new flag, no
   fixed-point exposure.
3. **Standalone queries only.** Runs set `VOW_VERIFY_NO_INCREMENTAL=1`, which is
   already shown verdict-neutral, `--no-cache`, and a fresh `VOW_CACHE_DIR`, so
   every claim is one self-contained file that is actually sent to the solver.
   A solver invocation with no file argument (an incremental session) is logged
   by the shim and makes the run a harness failure rather than a silent gap.
   Incremental session streams are not Z3-checked; their equivalence to
   standalone queries rests on the slicing/incremental parity tests.
4. **Disagreement policy.** Bitwuzla `unsat` with Z3 `sat` is `SOUNDNESS` (a
   false proof); Bitwuzla `sat` with Z3 `unsat` is `PRECISION` (a spurious
   counterexample). Either fails the run (exit 1). `unknown` and a timeout on
   either side are `inconclusive`: counted, never a disagreement. A Z3 error is
   the harness failing (exit 2), never a skip, and `--min-agreed` keeps a sweep
   that decided nothing from reading clean. A disagreement is triaged as an
   encoder bug (guard the operation); it is never allow-listed. The saved query
   is the repro.
5. **The runtime differential gains `--backend native`, held to cli.md's
   contract.** `verify` (not `build`, which rejects the flag) gets
   `--backend native`. Under it a replay that `aborted`, and any `VerifierBug`
   diagnostic, is a `PRECISION` finding: the native model is meant to be checked
   against the runtime, not trusted. A `skipped` replay is a `REPLAY-GAP` finding
   unless the counterexample is a verifier obligation with no runtime abort
   (a shift count, an unreached obligation, a co-emitted callee's inputs) or the
   fixture documents the limit with `// TEST: replay-skipped <why>`, the
   directive `full_test.sh` already enforces for `tests/verify-native/`. An
   unrecognised reason fails closed.
6. **A `Verified` program that violates the `requires` of a callee the
   verifier disclaimed is not a false proof.** Calls from a function the checker
   cannot model (`main`) to a contracted callee carry an explicit
   `VerificationSkipped` note naming the callee, and the callee's `requires` is
   a debug-mode runtime check by design. Under `--backend native` a Caller
   violation whose `vow_id` (mapped to its function with `vowc contracts`)
   belongs to a callee named in such a note is reported as
   `unverified-call-site` and counted. A violation of any other function, a
   Callee violation, a `vow_id` that cannot be mapped, or a Caller violation
   with no such note remains `SOUNDNESS`.
7. **Placement.** One scheduled/push-to-`main` workflow (`native-oracle.yml`),
   sharded four ways, builds `build/vowc` with `bootstrap.sh --no-verify`
   (neither oracle needs ESBMC) and sweeps the fixture corpora plus a
   `gen_corpus.py` corpus. The seed is pinned on pushes and date-derived on the
   nightly, and is echoed. It is off the pull-request path for the same reason
   `bootstrap.yml` is.

## Consequences

- A false proof or a spurious counterexample on the corpus fails CI with the
  offending query attached as an artifact.
- The `gen_corpus.py` equivalence axis supplies query volume but little
  soundness depth: its `main` never calls the generated functions. The runtime
  soundness direction gets its depth from the verifier axis of #1473
  (`gen_corpus.py --axis verifier`, see the Addendum), swept as one more
  `--extra-root`.
- Verdict equivalence of incremental sessions is not independently checked.
  Following the session stdin into Z3 is the natural next step.
- Z3 and Bitwuzla can legitimately differ on operations the SMT-LIB standard
  leaves unspecified (for example `fp.to_ubv` out of range, once the FP encoder
  lands). Such a disagreement is a signal to guard the operation in the encoder,
  not to teach the harness about it.

## Addendum (2026-10-13, #1473): the generated verifier corpus

`gen_corpus.py --axis verifier` generates the corpus the differential needs. Its
design choices:

- **Two classes, with the control as a gate.** `exceeds-bound` programs have a
  contract that is false only past a boundary a bounded checker might assume;
  `control` programs are the same shapes with the true contract and must stay
  `Verified`. Without the control, a verifier that answers `unknown` to
  everything satisfies the first class trivially.
- **The witness is checked against the runtime, not the verifier.**
  `check_witness` runs every generated program in debug mode whatever the
  verifier said, and `WITNESS-BROKEN` fails when the runtime does not match the
  class. A generator regression that stops reaching the false contract then
  fails loudly instead of reading as "no false proofs".
- **Inputs satisfy the `requires`.** `main` is not verified (`VerificationSkipped`),
  so an input that broke a `requires` would fire the disclaimed-obligation path
  of decision 6 and measure nothing.
- **Known disagreements are keyed by shape, class and regime and carry an issue
  number.** A tracked gap that stops reproducing fails the run
  (`STALE-KNOWN-GAP`), so the ledger cannot rot. The regime depends on the native
  unrolling bound, mirrored in the generator and pinned against
  `compiler/vc_native.vow` by `test_gen_verifier_corpus.py`.
- **Seeds.** Pinned on pushes to `main` (verifier axis `1473`), the UTC date on
  the nightly, an input on a manual run; always echoed.
