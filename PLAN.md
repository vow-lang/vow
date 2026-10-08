# Plan: issue #1401 — docs(adr): native verifier CLI and status surface

## 1. Problem restated

Epic #1398 replaces ESBMC with a native in-Vow verifier (Bitwuzla via SMT-LIB). Decision D8 of the
epic already says, in one table row, which verification flags and statuses survive; D10 says
`Skipped` is fail-closed; D7/D13 name two reason strings. Nothing durable and enumerated exists
yet. Later children (`--backend native` in P1, the `docs(spec)` rewrite in P3, the flip in P5, the
removal in P6) each need one authoritative answer to "is flag X kept, removed or changed, and what
may a `Skipped` result say?". This issue produces that answer as an ADR and nothing else:
`docs/adr/YYYY-MM-DD-HHMM-<slug>.md`, a docs-only PR. It records the surface; it does not change
the surface (that is P1/P3/P5/P6 work).

## 2. Files to touch

Only one new file in the PR:

- `docs/adr/2026-10-08-HHMM-native-verifier-cli-and-status-surface.md` — HHMM is the UTC minute at
  authoring (`date -u +%Y-%m-%d-%H%M`), per `docs/adr/README.md`. Reference it as `ADR-2026-10-08-HHMM`.

Deliberately **not** touched (no Rust/`compiler/` change, so the dual-compiler rule is vacuous):

- `docs/spec/cli.md`, `errors.md`, `contracts.md`, schemas, `skills/vow/**`,
  `vow/src/skill.rs`, `compiler/main.vow` help text. The spec must change only when the behaviour
  changes; the epic's P3 `docs(spec)` child owns that and runs `generate_help.py`. An ADR that
  rewrote the spec early would make spec claim behaviour the compilers don't have, and
  `check_help_coverage.py` would not flag it. Therefore no `generate_help.py` run is needed here.
- `CLAUDE.md` and the D1 dual-compiler exception — owned by the sibling "D1 exception" ADR child.
- The sibling "verification semantics" ADR (k-induction, unwinding, arrays, IEEE floats): this ADR
  only links to it by name, not by path (it may not exist yet).

The PR implementation stage must `git rm PLAN.md` before opening the PR (per the stage contract).
PR title (<= ~92 chars, lower-case subject): `docs(adr): record native verifier cli and status surface`.
Body: `Closes #1401` (sub-issue of #1398; do not close #1398).

## 3. Slices (docs-only, so "tests" are mechanical checks run from the shell)

Each slice is one reviewable step; commit per slice if useful, but the PR is squash-merged.

1. **Skeleton + context.** Create the ADR with the repo's house shape (see
   `2026-10-04-0615-linear-discharge.md`): title, `**Status:** accepted (2026-10-08)`, `## Context`
   (D8/D10 of #1398, why one table row is not enough, link to #1398/#1335), `## Decision`,
   `## Consequences`, `## Alternatives considered`. Check: file name matches
   `^docs/adr/\d{4}-\d{2}-\d{2}-\d{4}-[a-z0-9-]+\.md$`; `typos` and `end-of-file-fixer` clean
   (`pre-commit run --files <adr>`).
2. **Flag classification table (acceptance #2).** Source of truth is the live option tables in
   `docs/spec/cli.md` for `build`, `verify`, `contracts`, `test`. Rows (command columns, then
   verdict). Proposed verdicts:

   | Flag / env | Commands | Verdict | Note |
   |---|---|---|---|
   | `--backend <esbmc\|native>` | build, verify, contracts `--verify`, test `--verify` | **new, transitional** | Default `esbmc` until P5; P5 flips default to `native` with `esbmc` as the opt-out; P6 deletes the flag (a leftover `--backend esbmc` is then a usage error naming the removal; `--backend native` is tolerated for one release as a no-op). |
   | `--no-verify` | build | kept, text changed | Wording "ESBMC" -> "static verification". Result stays `Unverified`. |
   | `--verify` | contracts, test | kept, text changed | Same wording change. |
   | `--no-cache` | build, verify, contracts | kept | Meaning unchanged: disables the verification-result cache (and the compile-object cache for `--no-verify`). Native cache is keyed on canonical sliced query + Bitwuzla pin (P4); perf gate measures with it off. |
   | `--timeout <N>` (seconds) | build, verify | **changed** | Remains the only user-facing resource knob. Default: single fixed per-function default of 300 s; the `30`-s `--encoding auto` special case disappears with `--encoding`. `--timeout 0` stays "immediate kill". Enforcement becomes process-group kill of the `verify-worker` (D11). Timeout yields `timeout` status. |
   | `--timeout <ms>` | test | kept, unrelated | Per-test *execution* timeout in ms; explicitly distinct from the seconds-based verification `--timeout`. ADR notes the unit clash and that `test --verify` gets no separate knob (non-goal; file a follow-up if wanted). |
   | `--verify-jobs <N>` | build, verify, contracts (no-op), test | kept | Caps concurrent `verify-worker` subprocesses (was ESBMC processes). Default `num_cpus/2` unchanged. |
   | `--replay-cex` | build, verify | kept | Semantics unchanged; becomes mandatory in the P5 oracle. "After ESBMC reports" -> "after the verifier reports". |
   | `--perfetto <path>` | build, verify | kept, spans changed | Per-function ESBMC proof spans -> per-claim worker/Bitwuzla spans; compiler->ESBMC handoff -> compiler->worker handoff; RSS series per worker. Still a pure side artifact. |
   | `--max-k-step <N>` | build, verify, contracts, test | **removed** | Bound becomes internal (D5); unwinding assertion makes "too deep" `unknown`, never `proven`. Removed at P6 (stays functional for `--backend esbmc` until then; native ignores/rejects it — ADR picks: native **rejects** it with a usage error so no run silently ignores a bound the user set). |
   | `--solver <...>` | build, verify, contracts | **removed** | Bitwuzla only (D2). Same P6 timing and same native-rejects rule. |
   | `--encoding <...>` | build, verify, contracts | **removed** | Fixed-width BV only (D2/D7). Same timing/rule. |
   | `VOW_VERIFY_DEBUG` (Rust only) | env | removed with `c_emitter.rs` | Native debugging uses retained `.smt2` under `VOW_CACHE_DIR`/perfetto; defer the exact knob to the P1 solver-driver child. |
   | `VOW_VERIFY_RUN_MEMLIMIT_RSS` | test-only env (`vow-verify/tests/memlimit.rs`) | removed with ESBMC | Not part of the CLI contract; listed for completeness. |
   | `verify-worker` subcommand, `--worker-entry` | internal | not CLI contract | Output and flags unstable, like the existing `--worker-entry`. |

   Check: a throwaway script (not committed) extracts every `` `--flag` `` from the four option
   tables in `cli.md` and `grep -F`s each in the ADR; the diff must be empty except flags that are
   clearly not verification-related (`-o`, `--mode`, `--dump-ir`, `--debug-trace`, `--filter`,
   `--module-root`, `--jobs`, ... — list these as "out of scope: not verification" in the ADR so the
   classification is provably exhaustive).
3. **Status surface (kept/changed/removed).** Enumerate: build `status` (`Verified`, `Unverified`,
   `Skipped`, `CompileFailed`, `VerifyFailed`) — kept; `verify_status` enum (`timeout`, `unknown`,
   `error`, `tool_not_found`, `panicked`) — kept, with `tool_not_found` now meaning Bitwuzla and
   OOM in a worker reported as `unknown` with a structured reason (D11); contracts per-clause
   `status` — kept except **`proven-ir` removed** (enum entry in
   `docs/spec/schemas/contracts-result.schema.json` and its copy under `skills/vow/schemas/`,
   `vow/src/contracts.rs`, `compiler/main.vow` embedded copies, `docs/verifier-discipline.md`
   ProvenIr row — all at P6, not now); `vacuous`, `skipped`, `timeout`, `failed`, `unknown`,
   `error`, `not_verified`, `proven` kept; JSON schema (`build-result`, `counterexample`,
   `diagnostic`) kept with additive-only changes; blame (`Caller`/`Callee`) and `vow_id` kept and
   stable across backends; `replay` values kept; diagnostics `VerificationSkipped`,
   `ArithOverflowReachable`, `ModelCapacityAssumed` kept (the last becomes vacuous once D6 drops
   capacity caps; ADR states that it stops being emitted by native and is retired at P6).
   Fail-closed rules kept verbatim: `Skipped`/`VerifyFailed` exit 1; a halt-class result stops
   scheduling new checks. Add the rule "native never reports `proven` after a resource-limited
   weakened retry" (from `docs/verifier-discipline.md`).
4. **Closed `Skipped` reason list (acceptance #3).** Reasons today are free text built in
   `vow-verify/src/c_emitter.rs::non_modelable_reason` / `first_unsupported_opcode` and
   `compiler/c_emitter.vow::non_modelable_reason` (messages like ``function `f` has effects ...``,
   ``contains unsupported opcode `Load` ``, `FieldGet at 128-bit width`, ``Call extern `x` ``, ...).
   The ADR defines a closed set of stable kebab-case **reason codes** plus a human sentence
   template and an optional free `detail` (function/opcode/builtin name — detail is never part of
   the enumerated set). Proposed set:

   | Code | When | Lifetime |
   |---|---|---|
   | `function-has-effects` | callee or subject has non-empty effect set (D10) | permanent |
   | `recursion-unsupported` | direct or mutual recursion in the inlined call graph (D10) | permanent (modular verification is a separate ADR) |
   | `float-rem-unsupported` | `RemF32`/`RemF64` (D7) | until codegen defines float `%` |
   | `ir-non-dominating-read` | dominance validator rejects the IR (D13) | until lowerer fix is proven complete; keep as defensive gate |
   | `unmodeled-builtin` | extern call whose `operations.json` `verifier_model` is `unmodeled` or absent (D10) | shrinks as ops are modelled |
   | `unsupported-opcode` | opcode absent from `compiler/vc_ops.vow` (`Load`/`Store`, `LinearBorrow`, ...) with the opcode name in `detail` (D10) | shrinks |
   | `non-modelable-callee` | an inlined callee is itself `Skipped` (carries the callee's code in `detail`) | permanent |
   | `reserved-verifier-symbol` | function name collides with a reserved checker symbol | permanent (rename of reserved set decided at P1) |
   | `wide-aggregate-field` | `FieldGet`/`FieldSet` at 128-bit width | transitional: removed when the G8 aggregate ticket lands (D12) |

   Explicitly **not** `Skipped` (stated in the ADR to keep the gate honest): timeouts -> `timeout`;
   unwinding-assertion failure / solver `unknown` / OOM -> `unknown` with structured reason;
   Bitwuzla missing -> `tool_not_found`; worker crash -> `panicked`; a counterexample -> `failed`.
   Also explicitly dropped relative to ESBMC (now modelled, per D6/D7, extra proofs allowed by the
   acceptance gate): 128-bit scalar consts/checked ops, `Call extern ... with non-scalar element`,
   `Call target with a collection argument`.
   Decision to record: the code is carried as an **additive optional** `reason_code` on
   `VerificationSkipped` diagnostics (and in `verify_message`-style detail for the contracts
   `skipped` status), so the schema stays backwards compatible; the existing message text stays
   human-readable and is not a contract. Alternative considered: encode the code in the message
   prefix (rejected: agents parse message text). Adding any code outside the list requires amending
   this ADR (closed list, fail-closed gate).
   Check: grep that every reason fragment in the two `non_modelable_reason` functions maps to a row
   (or to the "dropped" list); the implementer must re-read both before finalising since the tree
   moves.
5. **Compatibility and migration timeline.** Table mapping the epic phases to when each verdict
   takes effect (P1 add `--backend`; P5 flip; P6 remove). State the interim rule while both
   backends exist: flags marked removed keep working under `--backend esbmc`, are rejected under
   `--backend native`. State that stage 0 (Rust `vow`) delegates verification to the pinned seed
   (D1/Boot) and so inherits whatever surface the seed has; the Rust CLI keeps parsing the removed
   flags until P6.
6. **Design-criteria section.** Short, per `CLAUDE.md`: does not make verification harder
   (removing knobs shrinks the matrix); eliminates a class of agent bugs (agents tuning
   `--max-k-step`/`--solver` to turn a failing proof green; unbounded free-text skip reasons parsed
   by regex); makes agentic coding easier (one deterministic verdict surface, stable codes).
   Include the contract-authoring corollary: the internal bound is not a contract bound.
7. **Self-review pass and PR.** Re-run the slice-2 and slice-4 checks; `pre-commit run --files
   <adr>`; verify every cross-reference path exists (`git ls-files`); verify no claim contradicts
   epic D1-D13. Then `git rm PLAN.md`, commit with
   `docs(adr): record native verifier cli and status surface`, push, `gh pr create --base main
   --head sym/vow/1401-docs-adr-native-verifier-cli-and-status-surface --title ... --body ...`.
   Checklist lines in the PR body: the three acceptance criteria. The "ADR merged" criterion is
   satisfied by the orchestrator's squash-merge.

## 4. Verification surface

None: no contracts, codegen, IR or C-model change. No new fixtures under `tests/` or `examples/`.
The CI gates that can still fire: `Lint PR title` (commitlint), `typos`, whitespace/EOF hooks.
`scripts/full_test.sh`, bootstrap and cargo gates are unaffected; do not run them, and do not
tick any "bootstrap green" claim in the PR body (CLAUDE.md pinning rule only applies to seam PRs).

## 5. Risk areas

- **Fixed point / idempotency / clippy:** untouched (no source changes).
- **Spec drift by accident:** editing cli.md/schemas here would desync `--help`, the embedded
  skill and `skills/vow/**`; keep the PR to the single ADR file.
- **Reason list goes stale:** the code today produces free text; the ADR list is derived by
  reading both `non_modelable_reason` implementations and the epic. Mitigation: slice-4 check and
  explicitly marking transitional rows. The later `vc_ops` child owns reconciling the real opcode set.
- **Over-specifying:** the ADR should fix names and semantics only, not the SMT/driver
  internals owned by sibling ADRs (verification semantics) — link instead of restating.
- **Judgement calls to flag in the PR description** (autonomous run, no operator): native
  *rejects* (not ignores) removed flags; `--backend` lifecycle ending in deletion at P6; additive
  `reason_code` field; default `--timeout` stays 300 s. If the implementer finds the spec or tree
  contradicts any of these, choose the conservative option (reject/keep stricter) and post a
  `gh issue comment 1401` noting it.
- ADR timestamp collision is not a real risk (minute granularity, UTC).

## 6. Out of scope

Any code or spec/help/skill/schema edit; implementing `--backend`; removing flags or `proven-ir`;
the D1 exception ADR and CLAUDE.md amendment; the verification-semantics ADR and `contracts.md`
rewrite plan; renaming or fixing stale spec wording (e.g. cli.md's "one of the four status values"
when five exist); a separate timeout knob for `test --verify`; `vow test --timeout` unit change;
Bitwuzla pinning; cache design beyond "`--no-cache` keeps its meaning".
