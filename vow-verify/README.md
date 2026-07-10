# vow-verify

ESBMC integration for the Vow compiler. Extracts verification conditions from
`vow-ir`, invokes ESBMC, and maps counterexamples back to source via `Origin`
metadata. Emits the structured verification statuses consumed by the `vowc`
driver and the `vow-diag` JSON schema.

## Layout

- `esbmc.rs` — harness generation, ESBMC invocation, output parsing, and the
  `VerificationResult` status enum (`Proven`, `ProvenIr`, `Failed`, `Timeout`,
  `Unknown { reason }`, `Skipped { reason }`, tool-error variants).
- `solver_strategy.rs` — per-function solver selection (`classify_function`) and
  the adaptive BV→IR fallback (`run_with_fallback`).
- `c_emitter.rs` — the IR → C model handed to ESBMC.

## Soundness discipline (read before touching the retry logic)

`run_with_fallback` is an *adaptive retry*: on a resource-limited bit-vector
result it retries under the weaker IR encoding. Any change to this path — or any
new retry strategy — must obey the safe/unsafe rules in
[`docs/verifier-discipline.md`](../docs/verifier-discipline.md). The short
version:

> A retry may change solver, switch encoding, split the obligation, or raise a
> budget. It must **never** turn a weakened check into `Verified`.

The discipline is enforced in code, not just documented:
`enforce_retry_never_launders_proof` (`solver_strategy.rs`) fails closed if a
resource-limited retry ever returns a bare `Proven`, and the unwind assertion in
the same branch forbids reducing `max_k_step` on retry. See the doc for the full
status taxonomy and the regression tests that lock it down.
