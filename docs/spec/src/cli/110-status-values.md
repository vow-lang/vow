
### Status Values

| Status          | Meaning                                     |
|-----------------|---------------------------------------------|
| `Verified`      | Compiled + every vowed function's contract was statically proved by the verifier, and every call from a verifiable uncontracted function into a contracted function satisfies the callee's `requires` (see [Callers Without a `vow` Block](contracts.md#callers-without-a-vow-block)). "Verifiable" excludes uncontracted callers that cannot be modelled (e.g. `main() [io]`) or whose proof the verifier could not finish; each carries a `VerificationSkipped` *Note* instead, and the note does not change the status. May still carry `ArithOverflowReachable` *Warnings* in `diagnostics[]`: those report a checked operator (`+!`, `-!`, `*!`, `/!`, `%!`) whose `ArithmeticOverflow` abort is reachable. The abort is the operator's specified behaviour and the contract is proved for every returning execution, so the status stays `Verified` (exit 0). See [`errors.md`](errors.md#arithoverflowreachable). It may also carry one `ModelCapacityAssumed` *Note* per proved function whose proof is bounded by a verifier model capacity; the status and exit code are unchanged. See [`errors.md`](errors.md#modelcapacityassumed). |
| `Unverified`    | Compiled but no verifier was invoked (`--no-verify`, `--dump-ir`). Exit 0. A missing verifier is not `Unverified`: it is `VerifyFailed` (exit 1). |
| `Skipped`       | Verification was requested but at least one vowed function could not be modelled. The reason is one of the closed list in [`errors.md`](errors.md#verificationskipped) under the native backend, or free text such as `LinearBorrow`, `Load`/`Store`, `RemF*`, effects under ESBMC. **ESBMC backend:** struct construction (`RegionAlloc`) and field reads/writes (`FieldGet`/`FieldSet`) **are** modelled via the user-struct heap model, except at 128-bit width: that slot is 8 bytes, so a `FieldGet`/`FieldSet` carrying an `i128`/`u128` is reported `FieldGet at 128-bit width` / `FieldSet at 128-bit width` instead of being modelled (the native backend models it). Each skipped function appears as a `VerificationSkipped` *Warning* in `diagnostics[]`, also when another function fails verification (the status is then `VerifyFailed`, and the warnings remain). Their contracts are runtime-checked under `--mode debug` but were not statically proved; the run fails closed with exit 1. |
| `CompileFailed` | Parse error, type error, module load error, unsupported code generation (a named backend limitation, such as `%` on floats), backend failure, link failure, or a diagnostic-emission I/O failure (e.g. a broken stderr/stdout pipe other than the tolerated case, or a full disk). Inspect `diagnostics[]`; backend failures use `CodegenUnsupported`, `CodegenFailed`, `LinkFailed`, or `IoError`. |
| `VerifyFailed`  | The verifier produced a non-Verified outcome: a counterexample, timeout, `VERIFICATION UNKNOWN` (`verify_status: "unknown"`; under the native backend also an unwinding failure, `unwinding assertion: ...`, an exhausted inlining or size budget, a counterexample that depends on an opaque builtin result, `opaque builtin result: ...`, or on the keys or length of a map the function did not build, `unconstrained map state: ...`, a counterexample found with a callee summary, `callee summary: ...`, or an out-of-memory worker, `memory limit exceeded`), tool error, the tool was not found (`tool_not_found`: ESBMC, or `bitwuzla` under native), or the verifier worker crashed (`verify_status: "panicked"`). Inspect `counterexamples[]` (definitive failures) and `verify_status`/`verify_message` (soft failures) to distinguish. |

For a multi-function `verify` or verified `build`, a halt-class result (definitive
counterexample, timeout, unknown, tool error, missing tool, or worker panic) stops new function
checks from being launched. Checks already claimed by parallel workers finish, and the compiler
reports the halt from the lowest module-declaration-order function in that claimed set. Therefore
`counterexamples[]` contains at most one entry; a reported soft failure leaves it empty and uses
`verify_status` instead. Fix the reported failure and run verification again to surface a later
failure.

### Verified Example

```json
{
  "status": "Verified",
  "executable": "examples/divide",
  "diagnostics": [],
  "counterexamples": []
}
```

### CompileFailed Example

```json
{
  "status": "CompileFailed",
  "executable": null,
  "diagnostics": [
    {
      "error_code": "TypeMismatch",
      "message": "function body has type `bool` but declared return type is `i32`",
      "severity": "error",
      "span": {
        "file": "bad.vow",
        "offset": 25,
        "length": 8
      }
    }
  ],
  "message": "type error",
  "counterexamples": []
}
```

### VerifyFailed Example

```json
{
  "status": "VerifyFailed",
  "executable": "examples/cegis_broken",
  "diagnostics": [
    {
      "error_code": "VowEnsuresViolated",
      "message": "contract violation in `safe_sub`: ensures result >= 0",
      "severity": "error",
      "span": {
        "file": "examples/cegis_broken.vow",
        "offset": 76,
        "length": 20
      },
      "hints": ["function `safe_sub` failed to establish its postcondition"],
      "blame": "Callee"
    }
  ],
  "function": "safe_sub",
  "counterexample": "[Counterexample]",
  "counterexamples": [
    {
      "function": "safe_sub",
      "values": { "a": "-9223372036854775808", "b": "0" },
      "violation": "ensures result >= 0",
      "vow_id": 1,
      "source": {
        "file": "examples/cegis_broken.vow",
        "offset": 76,
        "length": 20
      },
      "blame": "Callee"
    }
  ]
}
```

Every counterexample also yields one `error` diagnostic, appended after any
warnings. Its code follows the counterexample's `blame` (`Caller` →
`VowRequiresViolated`, `Callee` → `VowEnsuresViolated`; a failed `invariant` is
callee-blamed, so it reports `VowEnsuresViolated`), or is
`VerifierAssertionUnattributed` when the failure is not attributed to a vow
clause. The `message` is ``contract violation in `<function>`: <violation>`` (or
``verification failed in `<function>` on an unattributed property: <violation>``),
`span` is the counterexample's `source` (an empty file and zero offset/length
when `source` is `null`), `secondary` lists the counterexample's `call_sites`,
and `hints` name the failing function and, for caller blame, each violating
argument. A `timeout`, `unknown`, `error` or `panicked` outcome has no
counterexample and adds no such diagnostic. Both compilers emit the same
`diagnostics[]`, `counterexample` and `counterexamples[]` for the same source;
the only fields that are not specified byte for byte are `verify_message`
(the verifier's free text) and, under ESBMC, `values` entries named `$esbmc$...` (solver temporaries).

A diagnostic's `span` is exactly `{file, offset, length}`: positions are byte
offsets, and neither compiler adds line or column fields.

For caller-blame failures where a verified function violates a callee's
`requires` clause, the counterexample reports the callee clause in `violation`
and `vow_id`, and includes the caller expression in `call_sites`. When the
callee precondition binds a parameter, `violating_args` names the callee
parameter, the counterexample value when available, and the caller argument span.
If `violating_args[].value` is `""`, Vow could not statically recover the
caller argument value; `arg_offset` and `arg_length` still identify the
argument expression.

When a callee verified alongside its caller fails its own `ensures` or
`invariant`, the counterexample is attributed to that callee: `function` names
the callee, `violation`, `source` and `blame` (`"Callee"`) are the callee's
clause, and `vow_id` is the callee-local id. The outcome's top-level `function`
remains the function being verified. Such a counterexample carries no
`execution_path` or `branch_decisions` (block ids are not attributable to the
callee), and `--replay-cex` reports it as `"skipped"` because its inputs are
the caller's.

`blame` uses one casing on every surface: `Caller` or `Callee` in diagnostics
(omitted when no party is at fault), `Caller`, `Callee` or `None` in
counterexamples, and the same `Caller`/`Callee` in `vow contracts` and the
runtime `VowViolation` line.

When `blame` is `"None"`, `violation` describes the failed verifier-model check
(such as division by zero, collection bounds or capacity, unwrap-on-None, or
shift count) rather than exposing raw verifier output.

### Fields Reference

| Field              | Type                | When Present      | Description                               |
|--------------------|---------------------|-------------------|-------------------------------------------|
| `status`           | string              | Always            | One of the four status values             |
| `executable`       | string \| null      | Always            | Path to binary, null on compile failure or library module (no main) |
| `diagnostics`      | array               | Always            | Compiler diagnostics (see schema)         |
| `message`          | string              | CompileFailed     | Compatibility error category/detail (for example "parse error", "type error", "module load error", backend/link detail, or "failed to emit frontend diagnostics: {io_error}"). Agents should branch on `diagnostics[].error_code`, not parse this free text. |
| `function`         | string              | VerifyFailed      | Function where verification failed        |
| `counterexample`   | string              | VerifyFailed      | Legacy description string: `"[Counterexample]"` when the verifier produced a counterexample, otherwise the soft-failure text (`verification timed out`, `verification result unknown: <reason>`, `esbmc error: <message>` (the prefix is kept whichever backend ran), ...) |
| `counterexamples`  | array               | Always            | Structured counterexamples (see schema); contains at most one entry per run under the multi-function stopping policy above |
| `verify_status`    | string              | On backend failure | `"timeout"`, `"unknown"`, `"error"`, `"tool_not_found"`, or `"panicked"` (verifier worker thread crashed — no counterexample available) |
| `verify_message`   | string              | On backend failure | Verifier/backend error detail             |
