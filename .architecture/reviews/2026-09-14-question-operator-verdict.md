# Architecture review — vow — 2026-09-14

**Scope**: hot-spot-weighted scan of the Rust compiler crates, led by `vow-types/src/check.rs`
(heat 5 — 28 commits/90d, home of every landed deepening seam). A step-1 sub-agent walked
`check.rs`, `vow/src/counterexample.rs`, `vow-verify/`, `vow-runtime/`, `vow-codegen/`, and the
persisted `.architecture/backlog.md` was reconciled against `gh` first.

**Picked**: `question-operator-verdict` — see the PR and `.architecture/backlog.md`.

**Degradations**: advisor rate-limited this run — the step-4 design adjudication was done by this
skill against the written designs below, not by the stronger reviewer. Noted again in `## Design`.

**Diagram legend**: solid edges are the module interface; dashed edges are inside the implementation.

## Candidates

### question-operator-verdict — extract the `?`-operator payload policy into a pure verdict · Strong · score 22/25

- **Files**: `vow-types/src/check.rs:2776-2830` (the `ExprKind::Question` arm of `check_expr_inner`);
  the new seam sits beside `cast_verdict` (`check.rs:668`) and `same_operand_ty` (`check.rs:808`).
  File-count estimate: **1**.
- **Score**: **22/25**
  - **Leverage 4** — one call site, but the caller stops reaching past the seam: it no longer peels
    `Ty::Applied`/`Ty::Enum` shapes or cross-checks `current_return_ty` inline, and the 5-way policy
    becomes unit-testable without constructing a `Checker`. Matches the landed single-arm verdict seams
    `cast_verdict`/`builtin-constructor-spec`; not 5 because a single arm cannot pay back across many
    call sites.
  - **Locality 4** — a future change to `?` semantics (e.g. when Result propagation *is* lowered) becomes
    a one-place edit in the pure verdict plus its call-site emit, instead of surgery inside a 55-line arm.
  - **Blast radius 1** — one file, no published interface; `question_verdict` and its reject enum are
    crate-private, exactly like the existing seams. (6 − 1 = 5 into the total.)
  - **Heat 5** — `check.rs`, 28 commits in 90 days.
- **Problem**: the `?` arm is a **shallow** stretch of `check_expr_inner`: 55 lines in which type-shape
  inspection, three distinct rejection diagnostics, a payload-type computation, and `pattern_aggregates`
  bookkeeping are interleaved. To understand "what does `?` accept and what type does it yield" a reader
  must mentally run the whole arm; to test any single rejection they must build a checker with a function
  return context. The decision logic has no **seam** — the caller *is* the policy.
- **Deletion test**: delete the arm and the complexity does not move to callers (there is one) — it
  simply vanishes with the feature. Extracting the *decision* concentrates the five-way policy in one
  pure function; the emission and aggregate bookkeeping that genuinely need `&mut self` stay put. Passes:
  complexity concentrates, it does not merely relocate.
- **Solution**: add a pure `question_verdict(inner_ty: &Ty, return_ty: &Ty) -> Result<Ty, QuestionReject>`
  next to `cast_verdict`. `Ok(payload_ty)` carries the type the expression yields (Option-ok unwraps
  arg0; `Never` propagates as `Never`). `Err(QuestionReject::…)` names one of the three rejection kinds
  (`OptionNeedsOptionReturn`, `ResultNotLowered`, `NotTryable`). The call site matches: on `Ok` it runs
  the existing `pattern_aggregate_info` insert and returns the payload; on `Err` it emits the kind's
  message/hint (interpolating `inner_ty` for `NotTryable`) and returns `Ty::Unit`.
- **Benefits**: **leverage** — the five branches, including the subtle "Option `?` requires an Option
  return" cross-field rule, are now assertable in isolation (`question_verdict(&opt(i64), &plain) ==
  Err(OptionNeedsOptionReturn)`), a class of test that previously needed a full `Checker`. **Locality** —
  the policy lives in one pure function; diagnostics live at one call site. The test surface widens from
  "run the checker over a `.vow` snippet" to "call one total function", mirroring the payoff the landed
  seams already delivered.

**Before** — the caller *is* the policy:

```mermaid
graph LR
  Q["check_expr_inner: Question arm"] --> S1["peel Option/Result/Never shape"]
  Q --> S2["cross-check current_return_ty"]
  Q --> S3["emit 3 distinct rejects"]
  Q --> S4["compute payload Ty"]
  Q --> S5["pattern_aggregates insert"]
```

**After** — decision behind one seam; emission and bookkeeping stay at the call site:

```mermaid
graph LR
  Q["check_expr_inner: Question arm"] --> V["question_verdict"]
  Q --> S3["emit reject by kind"]
  Q --> S5["pattern_aggregates insert"]
  V -.-> S1["peel Option/Result/Never shape"]
  V -.-> S2["cross-check return_ty"]
  V -.-> S4["compute payload Ty"]
```

### coerce-context-argument-epilogue — collapse the repeated contextual-coercion epilogue · Worth exploring · score 21/25

- **Files**: `vow-types/src/check.rs` (~10 sites: L1377, L1464, L2115, L2201, L2663, L2802, L2917, L2999,
  L3063). Estimate: 1.
- **Score 21/25** — leverage 3, locality 5, blast radius 1, heat 5. Runner-up **candidate** (wins the
  21-point tie-break on heat 5 vs 4). Real sibling-epilogue dedup across ~10 sites, but the helper is
  `&mut self` (not a pure seam), so the test surface barely widens — hence leverage 3 and the borderline
  deletion-test note carried in the backlog.

### call-argument-coercion-action — extract the ABI width/sign coercion decision · Worth exploring · score 21/25

- **Files**: `vow-codegen/src/cranelift_backend.rs:425-459` (`coerce_call_argument`). Estimate: 1.
- **Score 21/25** — leverage 4, locality 4, blast radius 1, heat 4. A genuinely clean pure decision
  ({passthrough, ireduce, sextend, uextend, refuse-128-narrow}), but it lives in codegen, which must stay
  a byte-identical bootstrap fixed point — a `vow-types` pick is the safer unattended choice at equal
  score, and it loses the tie-break on heat (4 vs 5).

The remaining proposed candidates (`arm-pattern-support-classifier`, `narrow-shift-findings`,
`comparison-operand-verdict`, `builtin-function-spec`, `ce-trace-reconstruction`, and the rest) sit at
≤20/25; the step-1 scan re-confirmed the friction on `arm-pattern-support-classifier` and
`narrow-shift-findings` is still present. See `.architecture/backlog.md` for the full scored list.

## Dropped

No candidate was newly hard-filtered this run. The standing `dropped` rows were re-checked and still
apply:

| Candidate | Dropped because |
|---|---|
| `esbmc-ce-description-heuristic` | Leverage 1 — inert; the only caller discards the description. Caller unchanged. |
| `solver-classify-function` | Already a pure, unit-tested seam. No shallowness to remove. |
| `clif-shim-region-parity` | Blast radius 4 — see *Too large to automate*. |

## Too large to automate

- `clif-shim-region-parity` — blast radius 4, crosses the `vow-clif-shim`/`vow-codegen` crate seam and
  touches codegen output. Re-checked 2026-09-14: still large. A human should schedule it.
- `division-abort-spec` — blast radius 4, spans codegen/verify/runtime plus byte-identical bootstrap
  risk. Human-scheduled.

## Pick

`question-operator-verdict`, at **22/25**, is the deterministic top. It is a fresh candidate surfaced by
this run's step-1 scan — prior firings carded the cast, operator, method, and constructor arms of
`check.rs` but never the `?` arm — scored on the same rubric as the landed seams it mirrors. The pick was
**close**: the runner-up **candidate** is `coerce-context-argument-epilogue` at 21/25 (within 1 point),
which wins the 21-point tie-break over `call-argument-coercion-action` on heat. Both runner-ups are the
natural next firings. No `in-flight` backlog entry has an open PR (the previous pick, PR #1268, merged
2026-09-11), so implementing this run is unblocked.

## Design

Three interfaces were produced **inline** (the sub-agent path was skipped to stay within the shared
build-memory budget; the design space here is narrow and fully determined by the call site and the two
landed precedents). Adjudication was done by this skill against the written designs below — the
**advisor was rate-limited this run**, so the stronger reviewer did not weigh in.

All three are behavior-preserving: `pattern_aggregate_info(&Ty::Unit, _)` is `None`
(`aggregate_type_name(Unit)` is `None`), so the current arm's inconsistency — the Option-return-mismatch
branch early-returns and skips the `pattern_aggregates` insert, while the Result and non-tryable branches
fall through to it on `Ty::Unit` — is non-observable. Every design routes all rejections through a single
`Ty::Unit` fall-through, which the no-op insert leaves identical.

### Design A — classification enum (mirrors `cast_verdict`)

```rust
enum QuestionVerdict { Payload(Ty), OptionNeedsOptionReturn, ResultNotLowered, NotTryable }
fn question_verdict(inner_ty: &Ty, return_ty: &Ty) -> QuestionVerdict
```

Call site matches four variants; `Payload(ty)` → aggregate + return, the three reject variants each emit
their message/hint. **Hides**: Option/Result/Never shape-peeling and the return-type cross-check.
**Trade-off**: a flat four-variant enum conflates the one success with the three failures, so the caller
match cannot use the idiomatic `Ok`/`Err` split; wording correctly stays at the call site.

### Design B — verdict owns the diagnostics

```rust
enum QuestionVerdict { Payload(Ty), Reject { message: String, hints: Vec<String> } }
fn question_verdict(inner_ty: &Ty, return_ty: &Ty) -> QuestionVerdict
```

Smallest call site (two branches). **Trade-off**: the "pure" decision now owns diagnostic wording, so a
message reword edits the decision function — diagnostic **locality** regresses, and tests must assert on
strings or collapse all three rejects into one, losing the ability to pin *which* rejection fired. This
departs from the house pattern (`cast_verdict`, `same_operand_ty` keep wording at the call site).

### Design C — `Result<Ty, QuestionReject>` (mirrors `same_operand_ty`) — WINNER

```rust
enum QuestionReject { OptionNeedsOptionReturn, ResultNotLowered, NotTryable }
fn question_verdict(inner_ty: &Ty, return_ty: &Ty) -> Result<Ty, QuestionReject>
```

`Ok(payload_ty)` is the type the expression yields; `Err(kind)` names the rejection. The call site does
`match … { Ok(ty) => ty, Err(reject) => { emit by kind (interpolating the in-scope inner_ty for
`NotTryable`); Ty::Unit } }`, then runs the unchanged aggregate bookkeeping. `QuestionReject` carries no
data — the call site already holds `inner_ty` for the one interpolated message.

### Adjudication

Against the criteria, in order:

1. **Depth** — B and C give a two-branch (`Ok`/`Err`) call site; A gives four. A loses.
2. **Locality** — C keeps diagnostic wording at the call site; B pulls it into the decision function, so
   a reword edits the "pure" seam. C beats B.
3. **Seam placement** — C's `Result<Ty, QuestionReject>` names the future-varying axis (`ResultNotLowered`
   is exactly where "Result propagation gets lowered" will land); B's opaque `Reject { message }` hides
   the kinds. C wins.
4. **Test surface** — C is message-independent and mirrors the landed `same_operand_ty` tests
   (`question_verdict(&opt(i64), &plain) == Err(OptionNeedsOptionReturn)`); B's tests are brittle or
   lossy. C wins.
5. **Blast radius** — tie (~1 file).

**Winner: Design C.** It is the exact shape of the most recent landed seam, `same_operand_ty`
(`Result<Ty, OperandError>`): success carries the propagated `Ty`, failure names the rule that failed,
wording stays at the call site.

**Runner-up design: Design A.** It shares C's good diagnostic locality and mirrors `cast_verdict`, but
its flat four-variant enum yields a shallower call site than C's idiomatic `Ok`/`Err` split. **Design B**
loses hardest: coupling the pure decision to diagnostic wording defeats the locality the extraction
exists to gain.
