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
| `--timeout <N>` | build, verify | **changed** | Seconds, per function. Remains the only user-facing resource knob. The default is one fixed 300 s; the 30 s `--encoding auto` special case goes away with `--encoding`. `--timeout 0` stays "kill immediately". Enforced by killing the `verify-worker` process group (D11). Expiry yields `timeout`. |
| `--timeout <ms>` | test | kept, unrelated | Per-test execution timeout in milliseconds, unrelated to the verification `--timeout` despite the shared name. `test --verify` gets no separate verification-timeout knob. |
| `--verify-jobs <N>` | build, verify, contracts, test | kept | Caps concurrent `verify-worker` subprocesses (was ESBMC processes). Default `num_cpus/2`. Still a no-op for `contracts`. |
| `--replay-cex` | build, verify | kept | Semantics unchanged; "after ESBMC reports" becomes "after the verifier reports". The P5 oracle requires it. |
| `--perfetto <path>` | build, verify | kept, spans changed | Per-function ESBMC proof spans become per-claim worker and Bitwuzla spans; the compiler-to-ESBMC handoff becomes a compiler-to-worker handoff; RSS is sampled per worker. Still a pure side artifact. |
| `--max-k-step <N>` | build, verify, contracts, test | **removed** | The bound is internal (D5). |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | build, verify, contracts | **removed** | Bitwuzla only (D2). |
| `--encoding <bv\|ir\|auto>` | build, verify, contracts | **removed** | Fixed-width bit-vectors only (D2, D7). |
| `VOW_VERIFY_DEBUG` (Rust driver env) | env | removed with `c_emitter.rs` | The native replacement (retained `.smt2` under `VOW_CACHE_DIR`, perfetto) is decided in the P1 solver-driver child. |
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
raise `--max-k-step` expecting a proof that no longer depends on it. The flags
are deleted from the parser at P6, together with `--backend esbmc`.

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
  `contracts-result`. Changes are additive only (see section 4).
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
  `compiler/main.vow`, and from `docs/verifier-discipline.md`. Until then the
  native backend never emits it. Its exit-code rule ("every contract is `proven`
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
| `unmodeled-builtin` | An extern call whose `verifier_model` in `docs/spec/operations.json` is `unmodeled` or absent (D10). `detail` is the builtin. | Shrinks as builtins are modelled. |
| `unsupported-opcode` | An opcode absent from `compiler/vc_ops.vow` (today `Load`, `Store`, `LinearBorrow`). `detail` is the opcode (D10). | Shrinks. |
| `non-modelable-callee` | An inlined callee is itself `Skipped`. `detail` carries the callee's code. | Permanent. |
| `reserved-verifier-symbol` | The function name collides with a reserved checker symbol. | Permanent; the reserved set is fixed in P1. |
| `wide-aggregate-field` | `FieldGet`/`FieldSet` at 128-bit width. | Transitional: removed when the 128-bit aggregate work (D12) lands. |

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
the `VerificationSkipped` diagnostic and on the `skipped` contract-clause
status. Old consumers ignore it; the `message` text stays human-readable and is
**not** a contract. Encoding the code as a message prefix was rejected because
agents would then parse prose.

**The list is closed.** Adding a code requires amending this ADR. A function
that fits no row is a verifier bug, not a new `Skipped` reason. Until a row
exists the build must report the failure (`error`) instead of skipping, so the
gate cannot be widened silently.

### 5. Timeline

| Phase | Effect on this surface |
|---|---|
| P1 | `--backend` added (default `esbmc`). Native rejects the removed flags. `reason_code` introduced. |
| P3 | `docs/spec` and the skill are rewritten to this surface; `generate_help.py` is rerun. |
| P5 | Default flips to `native`. `--backend esbmc` is the opt-out. `--replay-cex` joins the oracle. |
| P6 | `--backend esbmc`, `--solver`, `--max-k-step`, `--encoding` and `proven-ir` are deleted; `ModelCapacityAssumed` is retired. |

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
- Tooling that reads the `proven-ir` status or the `ModelCapacityAssumed` note
  can drop that handling once P6 lands. The native backend never produces
  either.
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
