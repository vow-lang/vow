# The native verifier keeps the verdict surface and drops the tuning knobs

**Status:** accepted (2026-10-08)

## Context

Epic #1398 replaces ESBMC with a native in-Vow verifier that emits SMT-LIB for
Bitwuzla (D2). Decision D8 of that epic settles, in one table row, which
verification flags and statuses survive; D10 makes the `Skipped` gate fail
closed; D7 and D13 each name one reason string. One row is not enough for the
later children to build against. P1 adds `--backend native`, P3 rewrites the
spec, P5 flips the default and P6 deletes ESBMC, and each needs a single answer
to two questions: is flag X kept, changed or removed, and what may a `Skipped`
result say?

This ADR is that answer. It records the user-visible verification surface. It
changes none of it: `docs/spec/cli.md`, `errors.md`, the JSON schemas, the
embedded `--help` and the skill stay as they are until the child that changes
the behaviour lands. Verification semantics (unwinding, k-induction, arrays,
IEEE floats) and the D1 dual-compiler exception are separate ADRs and are only
referred to here by name.

## Decision

### 1. `--backend <esbmc|native>` is a transitional opt-in

`--backend` is added to `build`, `verify`, `contracts --verify` and
`test --verify`.

| Phase | Behaviour |
|---|---|
| P1 | Flag added; default `esbmc`; `native` is opt-in. |
| P5 | Default flips to `native`; `--backend esbmc` is the opt-out. |
| P6 | ESBMC is deleted. `--backend esbmc` becomes a usage error that names the removal. `--backend native` is accepted as a no-op for one release, then rejected like any unknown value. |

The flag exists only to carry the migration. It is not a permanent choice of
verifier.

### 2. Flag classification

Every verification-related flag in `docs/spec/cli.md`, for `build`, `verify`,
`contracts` and `test`:

| Flag / env | Commands | Verdict | Notes |
|---|---|---|---|
| `--backend` | build, verify, contracts `--verify`, test `--verify` | **new, transitional** | See section 1. |
| `--no-verify` | build | kept, text changed | "Skip ESBMC static verification" becomes "skip static verification". Result is still `Unverified`. |
| `--verify` | contracts, test | kept, text changed | Same wording change. |
| `--no-cache` | build, verify, contracts | kept | Still disables the verification-result cache and, for `--no-verify` builds, the compile-object cache. The native cache key is the canonical sliced query plus the Bitwuzla pin (P4). |
| `--timeout <N>` | build, verify, contracts | **changed** | Seconds, per function. Remains the only user-facing resource knob. The default is one fixed 300 s; the 30 s `--encoding auto` special case goes away with `--encoding`. `--timeout 0` stays "kill immediately". Enforced by killing the `verify-worker` process group (D11). Expiry yields `timeout`. Both compilers already accept it on `contracts`, but `docs/spec/cli.md` does not list it there; P3 documents it. |
| `--timeout <ms>` | test | kept, unrelated | Per-test execution timeout in milliseconds, unrelated to the verification `--timeout` despite the shared name. `test --verify` gets no separate verification-timeout knob. |
| `--verify-jobs <N>` | build, verify, contracts, test | kept | Caps concurrent `verify-worker` subprocesses (was ESBMC processes). Default `num_cpus/2`. Still a no-op for `contracts`. |
| `--replay-cex` | build, verify | kept | Semantics unchanged; "after ESBMC reports" becomes "after the verifier reports". The P5 oracle requires it. |
| `--perfetto <path>` | build, verify | kept, spans changed | Per-function ESBMC proof spans become per-claim worker and Bitwuzla spans; the compiler-to-ESBMC handoff becomes a compiler-to-worker handoff; RSS is sampled per worker. Still a pure side artifact. |
| `--max-k-step <N>` | build, verify, contracts, test | **removed** | The bound is internal (D5). |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | build, verify, contracts | **removed** | Bitwuzla only (D2). |
| `--encoding <bv\|ir\|auto>` | build, verify, contracts | **removed** | Fixed-width bit-vectors only (D2, D7). |
| `VOW_VERIFY_DEBUG` (Rust driver env) | env | removed with the ESBMC driver (`vow-verify/src/esbmc.rs` reads it; the self-hosted driver never wired it) | The native replacement (retained `.smt2` under `VOW_CACHE_DIR`, perfetto) is decided in the P1 solver-driver child. |
| `VOW_VERIFY_RUN_MEMLIMIT_RSS` (test-only env) | env | removed with ESBMC | Not part of the CLI contract. |
| `verify-worker` subcommand, `--worker-entry` | internal | not CLI contract | Output and flags are unstable. |

Not verification-related, therefore out of scope for this classification:
`-o/--output`, `--mode`, `--dump-ir`, `--debug-trace`, `--filter`,
`--module-root`, `--jobs` and every flag of `fmt`, `mutants`, `complexity`,
`skill` and the declaration-file commands.

**Interim rule.** A flag marked removed keeps working under `--backend esbmc`
and is **rejected with a usage error under `--backend native`**. The error names
the flag and says the bound or choice is now internal. Silently ignoring a bound
the user set would let a run look tuned when it is not, and an agent could then
raise `--max-k-step` expecting a proof that no longer depends on it.

At P6 each removed flag is replaced by an explicit rejection that names it,
together with `--backend esbmc`; it is not simply deleted from the parser. The
self-hosted driver validates no flags: `get_source_path` and
`get_source_path_sub` in `compiler/main.vow` hard-code the value-taking flags,
so once `--max-k-step` leaves that list `vowc build f.vow --max-k-step 50` takes
`50` as the source path instead of reporting the flag. The same lists must gain
`--backend` in P1, or `vowc build f.vow --backend native` takes `native` as the
source path.

The Rust stage 0 `vow` delegates verification to the pinned seed `vowc` (D1), so
it inherits whatever surface the seed has and keeps parsing the removed flags
until P6.

### 3. Status surface

Kept without change:

- **Build status** (`status` in the build JSON): `Verified`, `Unverified`,
  `Skipped`, `CompileFailed`, `VerifyFailed`.
- **`verify_status`**: `timeout`, `unknown`, `error`, `tool_not_found`,
  `panicked`. `tool_not_found` now means Bitwuzla is not on `PATH` or does not
  match the pinned SHA-256 (D2). A worker that is OOM-killed is reported as
  `unknown` with a structured reason (D11), never `panicked` and never `proven`.
- **Contract-clause `status`** (`vow contracts --verify`): `proven`, `failed`,
  `unknown`, `timeout`, `error`, `not_verified`, `skipped`, `vacuous`.
- **JSON schemas**: `build-result`, `counterexample`, `diagnostic` and
  `contracts-result`. Changes are additive only (see section 4), apart from the
  P6 removals under **Removed** and **Retired** below, which delete an enum
  value (`proven-ir`) and a diagnostic code (`ModelCapacityAssumed`). The child
  that first emits `reason_code` (P1) declares it in the schemas in the same
  change, not at P3: `diagnostic.schema.json` and `contracts-result.schema.json`
  set `additionalProperties: false`, so a strict validator rejects an undeclared
  field.
- **Blame** (`Caller` for `requires`, `Callee` for `ensures`/`invariant`) and
  `vow_id`. Both are stable across backends: a given contract keeps the same
  `vow_id` and blame whichever verifier produced the verdict.
- **`replay`** values on counterexamples.
- **Diagnostics** `VerificationSkipped` and `ArithOverflowReachable`.
- **Fail-closed rules**, verbatim: `Skipped` and `VerifyFailed` exit 1; a
  halt-class result stops scheduling further checks.

Removed:

- **`proven-ir`** (contracts clause status, `ProvenIr` in
  `docs/verifier-discipline.md`). Int mode is dropped (D2), so there is no
  weaker-encoding proof to label. The enum value is deleted at P6 from
  `docs/spec/schemas/contracts-result.schema.json` and its copy under
  `skills/vow/schemas/`, from `vow/src/contracts.rs` and the embedded copies in
  `compiler/main.vow`, and from `docs/verifier-discipline.md`. No compiler emits
  it as a clause status today: the Rust `resolve_clause_status` reports a
  `ProvenIr` result as `proven`, and `compiler/main.vow` only counts the string
  in the summary. It survives as a declared value in the spec and schemas. Under
  `--backend esbmc` an IR-fallback proof is still reported as plain `proven`;
  this ADR does not change that. Its exit-code rule ("every contract is `proven`
  or `proven-ir`") reduces to "every contract is `proven`".

Retired:

- **`ModelCapacityAssumed`**. D6 removes the Vec, String and map capacity caps
  and the heap-slot cap, so the native backend never emits it. The diagnostic is
  deleted from the spec at P6.

New rule, stated here so it cannot be lost in the rewrite: the native backend
**never reports `proven` after a resource-limited weakened retry**. A solver
`unknown`, an unwinding-assertion failure or an OOM stays `unknown`/`timeout`.
This restates the weakened-retry rule of `docs/verifier-discipline.md`; the IR
relabelling it allowed has no native counterpart.

### 4. Closed list of `Skipped` reasons

A function is `Skipped` only for one of the reasons below. Today the reason is
free text assembled in `vow-verify/src/c_emitter.rs::non_modelable_reason` /
`first_unsupported_opcode` and `compiler/c_emitter.vow::non_modelable_reason`.
The native backend carries a stable **reason code** from this table. The human
sentence is built from the code plus an optional free-form `detail` (a function,
opcode or builtin name); `detail` is never part of the enumerated set.

| Code | When | Lifetime |
|---|---|---|
| `function-has-effects` | The function has a non-empty effect set (D10). | Permanent. |
| `recursion-unsupported` | The function is directly or mutually recursive in the inlined call graph (D10). | Permanent here; modular verification is a separate decision. |
| `float-rem-unsupported` | The body uses `RemF32` or `RemF64` (D7). | Until codegen defines float `%`. |
| `ir-non-dominating-read` | The dominance validator rejects the IR (D13). | Kept as a defensive gate after the lowerer fix. |
| `unmodeled-builtin` | An extern call whose runtime symbol has no entry with `verifier_model: known` in `docs/spec/operations.json`: the entry is absent or says `unmodeled` (D10). `detail` is the builtin. | Shrinks as builtins are modelled. |
| `unsupported-opcode` | An opcode absent from `compiler/vc_ops.vow`, a module the `vc_ops` child introduces (today `Load`, `Store`, `LinearBorrow`). `detail` is the opcode (D10). | Shrinks. |
| `non-modelable-callee` | An inlined callee is itself `Skipped`. `detail` carries the callee's code. | Permanent. |
| `reserved-verifier-symbol` | The function name collides with a reserved checker symbol. | Permanent; the reserved set is fixed in P1. |
| `wide-aggregate-field` | `FieldGet`/`FieldSet` at 128-bit width. | Transitional: removed when the 128-bit aggregate work (D12) lands. |

**Prerequisite for `unmodeled-builtin`.** Absent stays fail-closed, so the gate
can only be enabled once the catalogue says which builtins are modelled. Today
`docs/spec/operations.json` has 33 entries, none with `verifier_model`, keyed by
Vow builtin `name`. The gate sees runtime symbols, and the `__vow_vec_*`,
`__vow_string_*` and `__vow_map_*` symbols it accepts today (`is_known_builtin`)
are not in the catalogue at all. Before the gate is turned on, the P1 child must
look the call up by `runtime_symbol` and backfill `verifier_model: known` for
every pure builtin the native model covers. Without that backfill every function
that touches a collection would be `Skipped` and exit 1.

**Not `Skipped`.** Each of these has its own status, so the `Skipped` gate stays
meaningful:

| Situation | Reported as |
|---|---|
| Per-function time budget exhausted | `timeout` |
| Unwinding assertion fails, solver answers `unknown`, worker OOM | `unknown` with a structured reason |
| Bitwuzla missing or wrong hash | `tool_not_found` |
| Worker crash | `panicked` |
| A counterexample exists | `failed` |

**No longer `Skipped`.** The native model covers what ESBMC's C model could not,
so these reasons disappear and more functions reach a proof (permitted by the
P5 acceptance gate): 128-bit scalar constants and checked operations (D7), calls
with a non-scalar Vec element, and calls that pass a collection to a user
function (D6).

**Carrying the code.** The code is an additive optional field `reason_code` on
the Warning-severity `VerificationSkipped` diagnostic that lifts the build to
`Skipped`, and a sibling field on the contract entry object when its `status` is
`skipped` (`status` itself stays a plain string). Old consumers that do not
validate against the schemas ignore it (see section 3 for the schema change); the
`message` text stays human-readable and is **not** a contract. Encoding the code
as a message prefix was rejected because agents would then parse prose.

**Open gap: other `VerificationSkipped` emissions.** The same diagnostic code is
also emitted as a Note for calls from an uncontracted caller that went unchecked
(`UncheckedCallsNote` in `vow/src/verify_outcome.rs`): the caller cannot be
modelled, or the verifier timed out or could not decide for it. The self-hosted
`test --verify` also reports a `contract_skipped` entry with a fixed
`VerificationSkipped` message. Timeout and undecided are not `Skipped` reasons
above, so this ADR assigns no `reason_code` to those emissions. The P1 child must
either amend this list for them or leave the field absent on them.

**The list is closed.** Adding a code requires amending this ADR. A function
that fits no row is a verifier bug, not a new `Skipped` reason. Until a row
exists the build must report the failure as an `error` (`VerifyFailed` with
`verify_status: "error"`; clause status `error`) instead of skipping, so the
gate cannot be widened silently.

**Addendum (issue #1408, walking skeleton).** For `unsupported-opcode` the
`detail` may carry the operand type as `OpName[ty]` (`CheckedAdd[i64]`,
`WrappingAdd[u64]`, `GetArg[Bool]`; no suffix for `Void`), which keeps the table
above unchanged. The first native backend emits the code only inside the human
`VerificationSkipped` message (``skipped verification of `f`: <code>: <detail>``);
the `reason_code` field and its schema edits are deferred to the follow-up that
introduces the op-model table, so no schema changes in #1408.

**Addendum (issue #1409, op-model table).** `compiler/vc_ops.vow` is the single
op-model table and holds the closed code list (`vc_skip_code_valid`). Every opcode
of `compiler/ir.vow` is classified there, and `compiler/tests/test_vc_ops.vow`
fails when an opcode is added without a decision. The catalogue lookup is by
`runtime_symbol` (`catalogue_verifier_known`, generated from `docs/spec/operations.json`), and `generate_operations.py --check`
fails when a catalogued builtin has no `verifier_model`. All 45 catalogued
builtins are `unmodeled` for now: the symbolic executor encodes no call, so a
`known` entry alone would still be skipped as `unsupported-opcode`. The
collection runtime symbols are not catalogued, so they are absent, which is
`unmodeled-builtin` as well. When a function has several unsupported
instructions, a specific code (`float-rem-unsupported`, `unmodeled-builtin`,
`wide-aggregate-field`) is reported in preference to a generic
`unsupported-opcode` that happens to come earlier. The per-function gate emits
`function-has-effects`, `ir-non-dominating-read`, `float-rem-unsupported`,
`unmodeled-builtin`, `wide-aggregate-field` and `unsupported-opcode`. Three codes
need information the per-function gate does not have and are defined but not yet
emitted: `recursion-unsupported` and `non-modelable-callee` arrive with call
inlining (P2), where a call graph exists, and `reserved-verifier-symbol` waits for
the reserved set to be fixed (no native query contains a user function name).

### 5. Timeline

| Phase | Effect on this surface |
|---|---|
| P1 | `--backend` added (default `esbmc`). Native rejects the removed flags. `reason_code` introduced. |
| P3 | `docs/spec` and the skill are rewritten to this surface; `generate_help.py` is rerun. |
| P5 | Default flips to `native`. `--backend esbmc` is the opt-out. `--replay-cex` joins the oracle. |
| P6 | `--backend esbmc`, `--solver`, `--max-k-step` and `--encoding` become explicit rejections (section 2); `proven-ir` is deleted; `ModelCapacityAssumed` is retired. |

## Why this meets the language-design criteria

- **Does not make verification harder.** Each removed knob shrinks the matrix
  the verifier has to be correct across. Nothing is added to the pipeline.
- **Eliminates a class of agent bugs.** An agent can no longer turn a failing
  proof green by raising `--max-k-step` or switching `--solver`, and cannot
  mistake a weaker-encoding proof for a real one. A closed `Skipped` list
  replaces free text an agent would have to regex.
- **Makes agentic coding easier.** One deterministic verdict surface with stable
  codes, blame and `vow_id` across backends.
- **Contract authoring.** The internal unwinding bound is a verifier property,
  not a contract property: it must never appear in a `requires`/`ensures`
  clause, and an unwinding failure is `unknown`, never `proven`.

## Consequences

- Scripts and CI that pass `--max-k-step`, `--solver` or `--encoding` keep
  working under `--backend esbmc` and break on `--backend native`, loudly. They
  must drop the flags before the P5 flip.
- Tooling that handles the `proven-ir` status value or the `ModelCapacityAssumed`
  note can drop that handling once P6 lands. The native backend never produces
  either, and no compiler emits `proven-ir` as a clause status today.
- Agents get a machine-readable `reason_code` for `Skipped` and a finite set to
  handle.
- The reason table is derived from the two `non_modelable_reason`
  implementations and the epic. The `vc_ops` child reconciles it with the real
  opcode set and amends this ADR if it finds a case that fits no row.

## Alternatives considered

- **Ignore the removed flags under `--backend native`.** Rejected: a run would
  look tuned when it is not.
- **Keep `--backend` permanently.** Rejected: two verifiers is the cost the epic
  exists to remove.
- **Keep `--max-k-step` as an advisory hint.** Rejected: it invites using the
  bound as a contract substitute, which `CLAUDE.md` forbids.
- **Leave `Skipped` reasons as free text.** Rejected: the gate is fail-closed,
  and an open set of strings cannot be audited or tested for drift.
- **Encode the code in the message prefix.** Rejected: see section 4.
- **A separate verification timeout for `test --verify`.** Out of scope; file a
  follow-up if wanted.
