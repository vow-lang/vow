
### VerificationSkipped

**Phase:** Verification (Warning surfaced alongside `BuildStatus::Skipped`)
**Meaning:** The function carries a `vow {}` block but the verifier has no model for something its body does, so no proof is attempted. The contract becomes documentary: runtime checks still apply in `--mode debug`. A function is `Skipped` only for one of the reasons below; a timeout, a solver `unknown`, an unwinding failure and an out-of-memory worker are never `Skipped` (they are `timeout` / `unknown`, see `docs/spec/cli.md` → "Status Values").

The native backend names the reason with a code from a **closed** list, written after the function name in the message (``skipped verification of `f`: <code>: <detail>``). `detail` is free-form (a function, opcode or builtin name) and is not part of the enumerated set; the message text is not a contract. A machine-readable `reason_code` field is specified additively but **not emitted yet**, so do not parse for it.

| Code | When | Typical `detail` |
|---|---|---|
| `function-has-effects` | The function has an effect other than `panic` (`io`, `read`, `write`, `unsafe`). `panic`, the effect of `.unwrap()`, is modelled. | the effect |
| `recursion-unsupported` | The function is directly or mutually recursive in the inlined call graph, or only reaches a recursive function. | the cycle, `a -> b -> a` |
| `float-rem-unsupported` | The body uses float `%` (`RemF32`, `RemF64`). Lasts until codegen defines float `%`. | |
| `ir-non-dominating-read` | The dominance validator rejects the IR (defensive gate). | |
| `unmodeled-builtin` | An extern call whose runtime symbol has no `verifier_model: known` entry in `docs/spec/operations.json` (`Vec` `reserve`, `String::from_cstr`, an `_in_arena` variant). Shrinks as builtins are modelled. | the runtime symbol |
| `unsupported-opcode` | An opcode the symbolic executor does not encode (today `Load`/`Store`, `LinearBorrow`, a string literal the pool does not resolve), a shape it cannot unroll, or an aggregate use outside the modelled subset. Shrinks. | `Op[type]`, or `irreducible loop (...)`, `loop value used outside its loop`, `entry block is a loop header`, `loop nest deeper than 8`, `FieldSet on a value that is neither an allocation, a parameter nor a merge of them`, `a pointer field is read from a value that is not a parameter or a field of one`, `a pointer read out of a field is used as a collection, a string or a value`, `a pointer read out of a field in a function whose parameters may be one object`, `Upsilon type differs from its Phi`, `map key or value that is not a Bool, an integer up to 64 bits or unit`, `map used as a struct`, `map passed to a call`, `map passed to an unmodelled builtin`, `map merged by a Phi`, `a pointer element is read back`, `a Vec of pointer elements is copied`, `a pointer element is stored in a function that reads memory through a handle` |
| `non-modelable-callee` | An inlined callee is itself `Skipped`. | `<callee> (<code>)` |
| `inlining-too-large` | The function and everything it calls flatten to more than 16 384 instructions, above what a worker decides within its 4 GB memory cap (the answer would be `unknown` after minutes). A proven callee is summarised where that brings the call tree under the budget (see the `Calls` bullet of the native backend in `cli.md`); the skip stays for a callee that is not proven or has no summary. | `<n> instructions after inlining, above the 16384 ...` |
| `reserved-verifier-symbol` | The function name starts with `__ESBMC_` or `__VERIFIER_`, the prefixes of the checker's own intrinsics. | the function name |

The list is closed: a function that fits no row is a verifier bug, not a new `Skipped` reason. Under the ESBMC backend the reason is the older free-text `non_modelable_reason` of the C emitter instead, for example an opcode the C model cannot represent (`RegionAlloc` and `FieldSet` from a struct construction the model does not cover, `Load`/`Store`, `RemF*`, `LinearBorrow`), a 128-bit checked operation or aggregate field, or a call that passes a `Vec`, `String`, map or `Option` argument to a user function (`Call target with a collection argument`) or uses a user function's result as one (`Call target with a collection result`). Native verification models the 128-bit cases, so those are `Skipped` only under ESBMC.

Every non-modelable vowed function is reported, including when another function fails verification in the same run: the set of skipped functions depends only on the program, not on the verify cache or `--verify-jobs`.

**Wrong:**

```vow
fn len_of(v: Vec<i64>) -> u64 {
    v.len()
}

fn count(v: Vec<i64>) -> u64 vow {
    ensures: result == v.len()
} {
    len_of(v)
}
```

```json
{
  "error_code": "VerificationSkipped",
  "severity": "warning",
  "message": "skipped verification of `ir_inst_set_region`: function `ir_inst_set_region` is not modelable in the verifier (contains unsupported opcode `RegionAlloc`)",
  "hints": [
    "the contract is documentary; runtime checks still apply in --mode debug"
  ]
}
```

**Why the build fails closed.** Per `CLAUDE.md`'s "Contract Authoring" guidance, contracts express semantic correctness and must not be weakened to fit the verifier. When the verifier cannot represent a function's body, the function is skipped with a structured warning instead of tripping a defense-in-depth assertion (under ESBMC, `__ESBMC_assert(0, "vow:UNSUPPORTED_OP_VOW_ID")`, which historically broke the bootstrap on every vowed struct-builder). But a skipped contract is still an unproved contract, so the build lifts its overall status to `Skipped` (exit 1). Use `--no-verify` if you explicitly want a non-failing path that does not invoke a verifier at all (`Unverified`, exit 0).

**Fix:** Refactor the function so its body uses only modelable opcodes — typically by splitting allocation/initialisation away from the contract-bearing computation. Alternatively, run with `--no-verify` if the contract is intentionally documentary.

**Right:**

```vow
fn count(v: Vec<i64>) -> u64 vow {
    ensures: result == v.len()
} {
    v.len()
}
```

**Note form (uncontracted caller).** The same code also appears with `severity: "note"` for a function that has no `vow` block, cannot be modelled (effects such as `[io]`, unsupported operations, a collection passed to a user function), and calls contracted functions. The callee `requires` at those call sites are not statically checked, but no contract of its own went unproved, so the build status does **not** fail closed. This note form is produced by the ESBMC backend; under `--backend native` the uncontracted-caller selection is shared but a native timeout or `unknown` for such a caller is a hard `VerifyFailed`, not a note. The message ends with the reason, and the reasons need different fixes: `` `f` cannot be modelled `` (restructure `f`, or accept that it stays unverified), or `the verifier timed out for `f`` / `the verifier could not decide them for `f`` (the proof was attempted but not finished — constrain or simplify `f`, or raise the timeout):

```json
{
  "error_code": "VerificationSkipped",
  "severity": "note",
  "message": "calls from `main` to contracted `divide` were not verified: `main` cannot be modelled",
  "hints": [
    "the callee `requires` at these call sites are checked at runtime in --mode debug only"
  ]
}
```

### ArithOverflowReachable

**Phase:** Verification (Warning; the build status stays `Verified`)
**Meaning:** The verifier proved that a checked operator (`+!`, `-!`, `*!`, `/!`, `%!`) can reach its `ArithmeticOverflow` abort for some input the function's `requires` admits. The operand span in `primary` points at the operator itself.

**Wrong:**

```vow
fn twice(x: i64) -> i64 vow {
    ensures: result == x + x
} {
    x +! x
}
```

```json
{
  "error_code": "ArithOverflowReachable",
  "severity": "warning",
  "message": "checked arithmetic in `twice` can abort: addition overflows",
  "hints": [
    "the contract is proved for every execution that returns; this abort is a separate runtime outcome",
    "constrain the operands in `requires` to rule the abort out, or use the wrapping operator if wrapping is intended"
  ]
}
```

The `message` names the cause: `addition overflows`, `subtraction overflows`, `multiplication overflows`, `divisor is zero`, or `quotient is not representable (MIN / -1)`.

**Why this is a Warning and not a failure.** Aborting *is* the checked operator's specified behaviour ([`grammar.md`](grammar.md#checked-arithmetic)), so a reachable abort is not an unproved obligation — unlike [`VerificationSkipped`](#verificationskipped), nothing went unchecked, and the status does not fail closed. The contract is genuinely proved: the verifier models the abort by pruning the overflowing execution, which never returns and so can never witness an `ensures` or a loop `invariant`. The warning exists because an abort is rarely what the author intended and is invisible in the contract verdict — `Verified` alone would not tell you the program can die at that operator.

**Fix:** If the abort is unreachable in practice, say so in the contract — a `requires` that bounds the operands removes the warning, and the bound is then a genuine semantic constraint rather than a verifier appeasement. If wrapping is the intended behaviour, use the wrapping operator (`+`, `-`, `*`, `/`, `%`) and state the wrapped result in the contract. If the abort is a real possibility the caller must handle, the warning is informational.

**Right:**

```vow
fn twice(x: i64) -> i64 vow {
    requires: x >= -4611686018427387904 && x <= 4611686018427387903
    ensures: result == x + x
} {
    x +! x
}
```

**Not emitted when:** the contract itself fails. The verifier reports one violated property per run, so a contract counterexample takes precedence and no abort claim is made — the counterexample is the actionable finding.

### VerifierAssertionUnattributed

**Phase:** Verification
**Meaning:** The verifier found a failed property that Vow cannot attribute to a user-authored `requires`, `ensures`, or `invariant` clause. The build fails closed with `VerifyFailed`, and the corresponding counterexample uses `blame: "None"` plus a reserved non-contract `vow_id` so it cannot collide with the function's real vow 0. Known examples are division or remainder by zero, a dynamic shift count that exceeds the operand's bit width, signed `MIN / -1` (native backend only), `unwrap()` called on `None` or `Err` (native backend only) and an out-of-bounds `Vec` index (native backend only). Under `--backend native` the native `vow_id` is `4294967293` and blame is `None`.

**Wrong:**

```vow
fn remainder(n: u64, d: u64) -> u64 vow {
    ensures: result <= n
} {
    n % d
}
```

```json
{
  "error_code": "VerifierAssertionUnattributed",
  "severity": "error",
  "message": "verification failed in `remainder` on an unattributed property: division or remainder by zero",
  "span": { "file": "", "offset": 0, "length": 0 }
}
```

The structured counterexample's `violation` field carries the stable property description instead of raw verifier text. `source` may be `null` when the verifier property has not been mapped back to a Vow source span.

**Fix:** Inspect `counterexamples[0].violation` and the reported values. For division or remainder by zero, prevent a zero divisor with a real semantic precondition or a checked branch. For `unwrap() called on None`, match on the value or require its discriminant instead of unwrapping. For a dynamic shift, keep the count below the left operand's bit width. If the description names an unfamiliar internal assertion, report it as a compiler attribution bug rather than treating the reserved `vow_id` as a contract clause.

**Right:**

```vow
fn remainder(n: u64, d: u64) -> u64 vow {
    requires: d != 0
    ensures: result <= n
} {
    n % d
}
```
