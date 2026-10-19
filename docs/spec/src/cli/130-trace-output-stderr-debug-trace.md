
### Contract Fields

| Field         | Type    | Description                                              |
|---------------|---------|----------------------------------------------------------|
| `vow_id`      | integer | Unique contract identifier within the program            |
| `function`    | string  | Function containing this contract                        |
| `kind`        | string  | `"requires"`, `"ensures"`, or `"invariant"`              |
| `description` | string  | Full contract text                                       |
| `blame`       | string  | `"Caller"` (requires) or `"Callee"` (ensures/invariant)  |
| `source`      | object  | `{ "file": string, "offset": integer }`                  |
| `status`      | string  | `"proven"`, `"proven-ir"`, `"failed"`, `"unknown"`, `"timeout"`, `"error"`, `"not_verified"`, `"skipped"`, or `"vacuous"` |
| `quality`     | string  | Static clause-shape classification (no verifier): `"weak"`, `"tautological"`, or `"substantive"` |
| `trivially_satisfiable` | bool | `--verify` only: true when a trivial `return <default>` body still satisfies this `ensures` (verification-confirmed weakness). Always false for `requires`/`invariant` and without `--verify`. Informational — never affects the exit code. See `docs/spec/contracts-methodology.md`. |

`description` is the clause kind followed by the predicate rendered by the canonical
printer, and the two compilers produce byte-identical text. The same text is carried
into `VowViolation.description` and the counterexample `violation` field. Integer
literals print in unsigned decimal at full 128-bit width (a `u64` literal above
`i64::MAX` such as `18446744073709551614` is never shown as `-2`), a negated literal is
`-` followed by its magnitude, a suffixed literal prints as the cast it denotes
(`5u64` is `5 as u64`), a cast prints its real target type, and a nested binary
operator is parenthesized exactly when its precedence requires it. Block, `if`/`else`,
`match` and loop expressions print multi-line (so `description` may contain newlines):
4-space indentation per nesting level for statements, match arms and loop `vow` clauses,
`let` statements with their pattern and type annotation, and every closing brace aligned
with the line its block opened on, so a block nested inside another block, arm or loop
indents by its depth exactly as the canonical printer renders it.

`source.offset` anchors differently depending on where the clause comes from: for a
clause inside a `vow { ... }` block (`requires`, `ensures`, `invariant`), it is the byte
offset of the clause keyword, not the predicate expression, in both compilers. For a
parameter's inline `where` refinement (also reported as `kind: "requires"`), both
compilers anchor on the byte offset of the parameter name instead.

### Status Values

| Status          | Meaning                                              |
|-----------------|------------------------------------------------------|
| `not_verified`  | Verification not requested (no `--verify` flag)      |
| `proven`        | The verifier established this contract. ESBMC: for the configured bounded verification model (bit-vector encoding, overflow modeled). Native: for every unwinding and collection length, never after a resource-limited weakened retry |
| `proven-ir`     | ESBMC backend only, never produced natively (and not emitted as a clause status by either compiler today). ESBMC established this contract for the configured bounded model under integer-arithmetic encoding after BV timed out; overflow is not modeled by IR, but the BV caller preconditions still guard against it |
| `failed`        | The verifier found a counterexample violating this contract |
| `unknown`       | The verifier could not conclude for this contract — under the native backend a solver `unknown`, an unwinding failure (`unwinding assertion:`), a counterexample that depends on an opaque builtin result (`opaque builtin result:`) or on the keys or length of a map the function did not build (`unconstrained map state:`), a counterexample found with a callee summary (`callee summary:`), a budget or out-of-memory worker, or a function that fails only on an abort; under ESBMC either `VERIFICATION UNKNOWN` was reported for the containing function (the incremental-BMC forward condition was unable to prove or falsify), or the function's verification failed overall and ESBMC's per-clause `--multi-property` run returned no individual verdict for this clause |
| `timeout`       | The verifier timed out on the containing function (ESBMC: BV and — when applicable — IR fallback both timed out) |
| `error`         | Verifier error, or the tool was not found (ESBMC, or `bitwuzla` under `--backend native`) |
| `skipped`       | The containing function is outside what the verifier models, with a reason from the closed list in [`errors.md`](errors.md#verificationskipped) (e.g. effects, recursion, an unmodelled builtin, `Load`/`Store`). A machine-readable reason code is not emitted yet. Contract is documentary; runtime checks still apply under `--mode debug`. Surfaces as a `VerificationSkipped` Warning in the build JSON's `diagnostics[]` and lifts the overall build/verify status to `Skipped` (fail-closed, exit 1) — use `--no-verify` if you want a non-failing path that does not invoke a verifier at all. |
| `vacuous`       | The containing function's `requires` clauses are contradictory, so every `ensures` is satisfied vacuously — the verifier proved nothing of substance (antecedent failure). Detected by a second run: under ESBMC a `vow_reach` label (`--error-label`) planted after the `requires` assumes is unreachable; under `--backend native` the function is cut right after its last `requires` and the cut point must be unreachable. All of the function's clauses are reported `vacuous` (fail-closed, exit 1). See `docs/spec/contracts-methodology.md`. |

The `proven` / `proven-ir` split and the rule that a resource-limited retry (e.g. the BV→IR fallback) may never report a weakened check as `proven` are the verifier's soundness discipline — the safe-vs-unsafe retry rules are specified in `docs/verifier-discipline.md`.

### Quality Values

`quality` is a static classification of each clause's *shape*, computed without running a verifier and independent of `status`. It surfaces the "proven but trivial" problem: a `weak` contract can be `proven` while constraining almost nothing. See `docs/spec/contracts-methodology.md` for the full taxonomy.

| Quality        | Meaning                                                                                      |
|----------------|----------------------------------------------------------------------------------------------|
| `weak`         | An `ensures` that only bounds `result` by an integer literal (e.g. `result >= 0`). Satisfied by almost any implementation. |
| `tautological` | A constant clause that references no program value (e.g. `true`, `0 >= 0`). Constrains nothing. |
| `substantive`  | Everything else — equality, relational, inverse/round-trip, dispatch-totality, or function-call shapes. The classifier is conservative: anything not provably weak/tautological is reported `substantive`. |

`as` casts are transparent to both verdicts. `result >= 0 as i64` is `weak` exactly as `result >= 0` is, `result as i64 >= 0` folds the same way, and `0 as i64 >= 0` is `tautological` — a cast never makes a constant bound substantive. This matters because an agent can still wrap a literal bound in a cast (`0 as i64`) and the clause would otherwise read as `substantive`; without the fold a `weak` ratchet stops meaning anything. Only a primitive numeric target is folded, so `x as Foo` is left alone, and a cast never turns a program value into a literal (`result >= x as i64` stays `substantive`).

An always-true or always-false comparison against `0` on an unsigned — or zero-extended unsigned — operand is not a quality verdict at all: it is the hard [`TautologicalComparison`](errors.md#tautologicalcomparison) type error, so such a clause never reaches `vow contracts`.

## Trace Output (stderr, --debug-trace)

When `--debug-trace=calls` or `--debug-trace=full` is used, the compiled binary emits JSON lines to stderr:

### calls mode
```json
{"event":"enter","fn":"main"}
{"event":"enter","fn":"divide"}
{"event":"exit","fn":"divide"}
{"event":"exit","fn":"main"}
```

### full mode (adds vow check results)
```json
{"event":"enter","fn":"divide"}
{"event":"vow","fn":"divide","vow_id":0,"passed":true}
{"event":"exit","fn":"divide"}
```

## Profile Output (stderr, profile mode)

When `--mode profile` is used, the compiled binary prints a call-count report to stderr on normal exit (via `atexit`). The report is not printed if the program is killed by a signal or calls `abort()`.

```
--- vow profile report ---
function                                        calls       %
-------------------------------------------------------------
infer                                         4812399   48.2%
is_def_eq_core                                3201882   32.1%
whnf                                           984201    9.9%
main                                                1    0.0%
-------------------------------------------------------------
total calls: 9998483, unique functions: 12
```

The report lists the top 20 most-called functions sorted by call count. No vow checks are emitted in profile mode.

## Runtime Error JSON (stderr, debug/sanitize mode)

When a compiled program runs in debug mode (`--mode debug`) or sanitize mode (`--mode sanitize`) and violates a vow at runtime, it emits JSON to stderr before aborting.

Two of the diagnostics below are **mode-independent** and are emitted in release
builds as well: `UnwrapOnNone` and `IndexOutOfBounds`. Both report a partial
operation failing on its own terms rather than a vow check, so they are never
compiled out. Each is marked below.

### VowViolation

```json
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"divide.vow","offset":42,"values":{"y":0}}
```

Schema: [`schemas/vow-violation.schema.json`](schemas/vow-violation.schema.json).

`values` entries are bare JSON number tokens and are not bounded by `i64`:
`u64`, `i128`, and `u128` bindings render at full width, so a consumer that
acts on the exact value needs an arbitrary-precision number parser. See
[`errors.md`](errors.md#vowviolation).

### ArithmeticOverflow

```json
{"error":"ArithmeticOverflow"}
```

Emitted when a checked arithmetic operator (`+!`, `-!`, etc.) overflows, or
when checked or unchecked division/remainder has no result because the divisor
is zero or signed division evaluates `MIN / -1`.

### UnwrapOnNone

```json
{"error":"UnwrapOnNone"}
```

Emitted when `.unwrap()` is called on `Option::None` or `Result::Err`. Emitted in
**every build mode**, release included — `.unwrap()` is a partial operation, not a
vow check.

### IndexOutOfBounds

```json
{"error":"IndexOutOfBounds"}
```

Emitted when a `Vec` index is out of bounds. Emitted in **every build mode**,
release included.

### UseAfterFree (sanitize mode only)

```json
{"error":"UseAfterFree","op":"push","vec":"0x55a1b2c3d4e0"}
```

Emitted when a Vec or String operation is attempted after the region that owns it
has been closed. Vow has no per-value free, so closing a region is the only event that
reclaims a Vec's descriptor and backing store; a later access is a use-after-free and
means region analysis placed the value too deep. `op` is one of `push`, `set`, `pop`,
`clear`, `truncate`, or `read`. Exit code 134.

## Agent Decision Tree

```
Parse JSON from stdout
├── status == "Verified"       → Success. Binary at `executable`.
├── status == "Unverified"     → Compiled but unverified (--no-verify or --dump-ir).
├── status == "CompileFailed"  → Read `diagnostics[]` for error details.
│   ├── error_code is parse error  → Fix syntax (see grammar.md)
│   ├── error_code is type error   → Fix types (see errors.md)
│   ├── error_code == "CodegenUnsupported" → Avoid the named backend limitation.
│   ├── error_code == "CodegenFailed" → Inspect backend context; report a compiler bug if supported source triggered it.
│   └── error_code == "LinkFailed" → Check the host linker and Vow runtime archive.
└── status == "VerifyFailed"   → Read `counterexamples[]`.
    ├── Check `inputs` for the violating values
    ├── Check `violation` for which contract failed
    ├── Check `source` for the location
    └── Fix the contract or the implementation, then rebuild
```

Always check stderr for human-readable diagnostics alongside the JSON on stdout.
