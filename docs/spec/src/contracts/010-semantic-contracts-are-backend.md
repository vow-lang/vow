# Contract Authoring and Verification

Vow statically verifies contracts with one of two backends: the native verifier (the default of the self-hosted `vowc`, `--backend native`, which emits SMT-LIB for Bitwuzla) or ESBMC (a bounded model checker, `--backend esbmc`, the opt-out and the only backend of the Rust `vow`). Both report the same verdict surface (statuses, blame, `vow_id`). This document covers contract patterns, verification behavior, and common pitfalls.

## Semantic Contracts Are Backend-Independent

A contract states the function's real semantic domain and result. It does not
state the limits of the tool currently used to check it. Replacing the verifier
with a stronger one must not require editing Vow source contracts.

Lengths, capacities, indices, and struct fields may appear in contracts when
they express a genuine algorithmic constraint or representation invariant. For
example, an index may need to be within a vector's length, paired inputs may
need equal lengths, or a ring buffer may need to be non-full before a write.
They must not be capped merely because a verifier uses a finite collection
model or because a smaller struct state space is easier to solve. The internal
unwinding bound of the native verifier is such a limit too: it never belongs in
a `requires`/`ensures`/`invariant` clause, and an unwinding failure is
`unknown`, never `proven`.

The same distinction applies to numeric bounds. Exact overflow guards and real
problem-domain restrictions are semantic; unwind caps and arbitrary
solver-friendly ranges are not. If the current verifier cannot establish an
honest contract, report that limitation outside the contract.

## Verification Pipeline

Codegen (Cranelift) and linking finish before verification starts, so peak memory is the larger of the two phases rather than their sum. A codegen or link failure ends the build before any verifier process is started. Verification itself runs up to `--verify-jobs` verifier processes at a time.

| | Native backend (`--backend native`, default) | ESBMC backend (`--backend esbmc`) |
|---|---|---|
| Pipeline | `IR → symbolic executor → SMT-LIB → Bitwuzla`, one `verify-worker` subprocess per function | `IR → C emit → ESBMC` |
| Clauses | `requires` is an assumption for the obligations after it; `ensures`, `invariant` and every abort site are obligations | `requires` → `__ESBMC_assume()`, `ensures`/`invariant` → `__ESBMC_assert()` |
| Loops | Unrolled to an internal bound (2, 4, 8, 16, 32, 64) with a mandatory unwinding assertion; loops with an `invariant` are also proven inductively, and an open round tries a k-induction step case | Incremental BMC, `--max-k-step` (default 50); no inductive step |
| Tuning | `--timeout` is the only knob; `--max-k-step`, `--solver` and `--encoding` are usage errors | `--max-k-step`, `--solver`, `--encoding`, `--timeout` |
| Collections | Array plus symbolic length, no capacity | Fixed-capacity arrays (see below) |
| Bounded result | An unwinding failure, solver `unknown` or out-of-memory worker is `unknown`/`timeout`, never `proven` | `proven` describes the configured model, including its finite unwind and capacities |

The table is the whole difference: contracts, blame and `vow_id` are the same under both. A contract is `proven` only when the verifier decides every obligation; the native backend never reports `proven` after a resource-limited weakened retry.

### Native backend

The native verifier executes the IR symbolically and encodes integers as fixed-width bit-vectors, so wrapping, signedness and 128-bit values are exact. Each `ensures` clause, loop `invariant` and abort site (division or remainder by zero, `MIN / -1`, an over-wide shift, `unwrap()` on `None`, an out-of-bounds `Vec` index) is a proof obligation; a `requires` is an assumption for the obligations after it. The exact subset, loop strategy and flags are in `docs/spec/cli.md` → "Native backend". A function outside the subset is `Skipped` with a code from the closed list in [`VerificationSkipped`](errors.md#verificationskipped), never `Verified`.

**Known limits.** The `parse_<int>`, `string_trim`, `string_to_upper`, `string_to_lower`, `parse_f64_bits` and `format_f64_bits` builtins are opaque total functions of their operands (a property that holds for every result is proven, one that depends on a result is `unknown`), `hash_str`, `hex_encode`, `hex_decode` and the decimal formatters are exact, and `replace`, `split` and `join` are opaque total functions too (`split` keeps the facts every run has about its piece count and `join` of no parts is empty). `f32` and `f64` are IEEE-754 (SMT-LIB `FloatingPoint`, round to nearest, ties to even): comparisons with a NaN are false except `!=`, `-0.0 == 0.0`, and division by zero is an infinity or a NaN, not an abort. There is one canonical NaN, so NaN payload and sign bits are not distinguished (a property of them is neither provable nor refutable), float `%` is `Skipped` as `float-rem-unsupported` until codegen defines it, and `parse_f64_bits`/`format_f64_bits` are exact on their `u64` side and a deterministic function of their `String` side, with the text conversion itself opaque: a property that depends on the value `parse_f64_bits` returns or on the text `format_f64_bits` returns (or on the result of an opaque string builtin above) is `unknown` (`opaque builtin result`). Float `*` and `/` can be expensive to decide, so such a contract may end `unknown` or `timeout`. Callee inlining makes a caller's query as large as its whole call tree, so a deep call tree can end `unknown` with `inlining budget:`.

### ESBMC backend

- Verification strategy: **incremental BMC** (`--incremental-bmc`) — base case plus forward condition, **not** k-induction (there is no inductive step). A contract is `proven` only when ESBMC completes the configured model checks; otherwise the result is `unknown`. `proven` describes the configured verification model, including its finite unwind and collection capacities, not executions outside that model.
- Incremental BMC with `--max-k-step` (default: **50**) — loops are verified incrementally up to N iterations
- Architecture: 64-bit
- Array bounds / pointer checks disabled (Vow handles these in its own model)
