# Architecture review — vow — 2026-09-11

**Scope**: `vow-types/src/check.rs` (the type checker) as the standing hot spot — 28 commits/90d,
the file every prior firing has deepened. A workspace-wide sub-agent sweep looked for fresh candidates
outside it (vow-ir, vow-codegen, vow-verify, vow-runtime, vow, vow-syntax, vow-diag); the strongest
fresh finds are recorded below but none outscored the carried-forward `check.rs` pick.
**Picked**: `builtin-constructor-spec` — see [PR #1268](https://github.com/vow-lang/vow/pull/1268) and `.architecture/backlog.md`
**Degradations**: none — `gh` authenticated, sub-agent available, advisor used at step 4.

Diagram convention (replaces the upstream HTML legend): **solid edges are the interface** a caller
sees; **dashed edges are inside the implementation**, hidden behind the seam.

## Candidates

### builtin-constructor-spec — fold the EnumConstruct builtin dispatch into a spec seam · Strong · score 22/25

- **Files** — `vow-types/src/check.rs:2879-3028` (the `match (enum_name, variant_name)` builtin arm
  of `ExprKind::EnumConstruct`), private free fn added near the sibling seams
  `method_result_type:380` / `method_argument_expectations:337` / `cast_verdict:668` /
  `same_operand_ty:808`. **Estimate: 1 file, ~150 lines moved.**
- **Score — 22/25**
  - Leverage **4** — a single pure table serves ~11 compiler-known constructors and removes their
    arity/argument/result policy from the `check_expr` giant; mirrors the landed `method_result_type`
    seam that did the same for methods. Not 5: the payload-typed variants (`Some`/`Ok`/`Err`) and the
    FFI-coercion variants (`from_raw_parts_copy`) keep bespoke wrapping at the call site, so the table
    is partial.
  - Locality **4** — adding or correcting a builtin constructor becomes a one-entry edit in one table
    instead of a new inline arm interleaved with diagnostics. Not 5 because payload wrapping still
    lives at the call site.
  - Blast radius **1** — one file, no published interface; the fn is private to the crate. (Band 1.)
  - Heat **5** — `check.rs` is the hottest file in the tree (28 commits/90d) and the EnumConstruct arm
    is squarely in the churn.
- **Problem** — the arm is **shallow-by-interleaving**: ~145 lines where the *policy* of each builtin
  (how many arguments, what each argument must be, what type it yields) is inseparable from the
  *diagnostics* (`emit_error_with_hints`, per-builtin message and hint text) and the *recovery*
  (draining `fields` through `check_expr` on the error path). Understanding "what does `Vec::new`
  yield?" or "how many args does `String::from_raw_parts_copy` take?" means reading past four
  diagnostic blocks. The mechanical part — a lookup table keyed on `(enum, variant)` — has no seam of
  its own, so it cannot be unit-tested without constructing a `Checker` and asserting on emitted
  diagnostics (which is exactly how the 11 existing tests at L6189-6351 are forced to work).
- **Deletion test** — delete the arm and the complexity **concentrates**: every caller of
  `EnumConstruct` would have to re-derive the builtin arity/argument/result rules. It does not merely
  move — the rules are genuine domain knowledge (Option/Result/Vec/String/HashMap/BTreeMap
  constructors), not glue. This is a real deepening candidate, not a wrapper.
- **Solution** — extract the compiler-known-constructor policy into a pure
  `builtin_constructor_spec(enum, variant) -> Option<CtorSpec>` returning arity + per-argument
  expectation + result shape, mirroring `method_result_type` / `method_argument_expectations`. The
  generic argument-checking loop, the payload-typed result wrapping (`Some`/`Ok`/`Err`), and every
  diagnostic string stay at the call site, keyed on the spec. Exact interface chosen at step 4.
- **Benefits** — **leverage**: the arity/argument/result table is unit-testable on `(enum, variant)`
  pairs with no `Checker`, the way `cast_verdict`/`same_operand_ty` are; the giant `check_expr` shrinks
  by ~120 lines. **Locality**: a new builtin constructor is a table row, not an inline arm. **Test
  surface**: the 11 diagnostic-driven tests keep passing unchanged, and new table-shape assertions
  need no emitter.
- **Before / After**

```mermaid
graph LR
  CE[check_expr: EnumConstruct arm] --> A1[String::from arity+type+emit]
  CE --> A2[from_raw_parts arity+i64+emit]
  CE --> A3[new/None fixed result]
  CE --> A4[Some/Ok/Err payload wrap]
  CE --> A5[user-enum variant check]
```

Above: every builtin's policy is inlined in the arm, interleaved with its own diagnostics.

```mermaid
graph LR
  CE[check_expr: EnumConstruct arm] --> S[builtin_constructor_spec]
  CE --> A4[Some/Ok/Err payload wrap]
  CE --> A5[user-enum variant check]
  S -.-> T1[arity]
  S -.-> T2[arg expectation]
  S -.-> T3[result shape]
```

Below: the mechanical arity/arg/result table sits behind one seam; only the genuinely
payload-dependent wrapping and user-enum path stay inline.

- **Recommendation strength** — Strong. It is the deterministic top of the ranking, mirrors four
  already-landed seams, and is fully pinned by existing tests. The one caveat — a ~150-line
  heterogeneous diff rather than a ~15-line verdict — is a behaviour-preservation *risk* guarded by the
  11 existing EnumConstruct tests plus the new unit tests, not a reason to skip. (The step-5
  file-count bail is separate: it fires only if the diff spreads past ~1 file, which this does not.)

### coerce-context-argument-epilogue — collapse the repeated coerce-and-emit epilogue · Worth exploring · score 21/25

- **Files** — `vow-types/src/check.rs` (~10 sites: L1377, L1464, L2115, L2201, L2663, L2802, L2917,
  L2999, L3063). Estimate ~1 file.
- **Score — 21/25** (leverage 3, locality 5, blast radius 1, heat 5). Leverage 3: the helper is a
  shallow `&mut self` wrapper of three statements (`check_expr` + `check_contextual_integer_literal_ranges`
  + `can_context_coerce`/emit), so tests gain nothing — still `&mut self`. Locality 5: a change to the
  coercion-error epilogue would become a one-place edit across ~10 sites.
- **Problem / deletion test** — sibling-epilogue duplication, not a pure-seam candidate. Deleting the
  (hypothetical) helper moves complexity back to ~10 call sites rather than concentrating new domain
  knowledge — hence leverage 3.
- **Recommendation strength** — Worth exploring. Runner-up **candidate** by the tie-break; the natural
  next firing if the pick lands.

### call-argument-coercion-action — extract the codegen coercion decision · Worth exploring · score 21/25

- **Files** — `vow-codegen/src/cranelift_backend.rs` (`coerce_call_argument`, ~L398-432). Estimate ~1 file.
- **Score — 21/25** (leverage 4, locality 4, blast radius 1, heat 4). Loses the runner-up tie-break to
  `coerce-context-argument-epilogue` on heat (4 vs 5).
- **Problem** — pure coercion decision (`actual_bits`, `expected_bits`, `is_i128`, `signed`) is
  interleaved with `builder.ins()` emission.
- **Recommendation strength** — Worth exploring, but carries **byte-identical-bootstrap risk**: codegen
  output must stay a byte-identical fixed point, so a `vow-types` seam is the safer unattended pick.

### comparison-operand-verdict — fold the comparison mismatch/tautology policy into a verdict · Worth exploring · score 20/25 (fresh)

- **Files** — `vow-types/src/check.rs:1898-1950` (the `Eq|Ne|Lt|Le|Gt|Ge` sub-arm). Estimate ~1 file,
  ~52 lines.
- **Score — 20/25** (leverage 3, locality 4, blast radius 1, heat 5). Leverage 3, not 4: the tautology
  half (`zero_comparison_verdict` + `never_negative_operand`) is *already* pure/testable, so the new
  testable surface is mostly the mismatch predicate and the mismatch-over-tautology priority; one call
  site.
- **Problem** — an inline `if / else if` folding the type-mismatch predicate and the
  zero-comparison-tautology decision (with a `widened_to` split) into emission — the same shallowness
  the landed `same_operand_ty`/`cast_verdict` seams removed, but one arm over.
- **Solution** — `comparison_verdict(lhs, rhs, never_negative) -> ComparisonVerdict ∈ {Mismatch,
  Tautology{always, unsigned_ty, widened_to}, Ok}`; caller pre-computes `never_negative` from the two
  existing helpers, message/hint strings stay at the call site.
- **Recommendation strength** — Worth exploring. Strongest fresh find; the natural firing after the two
  21-point runners-up.

### builtin-function-spec — table the free-function builtins · Speculative · score 20/25 (fresh)

- **Files** — `vow-types/src/check.rs:2082-2151` (`pin_to_root`, `string_matches_literal_at` branches
  of the `Call` arm). Estimate ~1 file, ~70 lines.
- **Score — 20/25** (leverage 3, locality 4, blast radius 1, heat 5). Leverage 3: only two builtins,
  and both are non-uniform (`pin_to_root` returns its argument type; `string_matches_literal_at`
  carries a static-string-literal constraint), so the spec is not a clean arity/type table — same
  behaviour-preservation profile as the picked constructor seam but with far less to gain.
- **Recommendation strength** — Speculative. The last builtin family without a spec, but thin.

### oversized-chunk-path-predicate — single-source a must-agree arena predicate · Worth exploring · score 17/25 (fresh)

- **Files** — `vow-runtime/src/lib.rs:1020` (authoritative, `__vow_arena_alloc`) and `:1246`
  (fast-skip, `arena_grow_backing`). Estimate ~1 file, 2 sites.
- **Score — 17/25** (leverage 2, locality 3, blast radius 1, heat 4). Leverage 2: the extracted
  `const fn takes_oversized_path(bytes, align) -> bool` is a one-line predicate that fails a strict
  deletion test on depth — its value is killing a documented drift hazard (the L1246 comment states its
  correctness *depends on* the L1020 placement), not deepening an interface.
- **Recommendation strength** — Worth exploring for the drift-kill; low leverage keeps it below the
  `check.rs` picks. Distinct from `vec-reserve-next-capacity-seam` (that is `next_capacity` doubling).

*Carried-forward lower-scored `proposed` candidates* (`arm-pattern-support-classifier` 20,
`ce-trace-reconstruction` 20, `narrow-shift-findings` 20, `negation-verdict` 18,
`vec-reserve-next-capacity-seam` 18, `recover-unknown-name-prologue` 18, `builtin-receiver-kind` 17,
`unwrap-payload-ty` 17) remain scored in `.architecture/backlog.md`; none reaches the pick.

## Dropped

| Candidate | Dropped because |
|---|---|
| `esbmc-ce-description-heuristic` | Leverage 1 — inert: the only caller destructures `Failed(_)` and discards the description, so extraction deepens nothing observable. Re-checked 2026-09-11: caller unchanged. |
| `solver-classify-function` | Already a pure, unit-tested seam (`test_classify_*`) — no shallowness to remove. |

## Too large to automate

| Candidate | Blast |
|---|---|
| `clif-shim-region-parity` (`vow-clif-shim` ↔ `vow-codegen`, ~20+ files, crosses a crate/tier seam and touches codegen output) | 4 — a human should schedule it |
| `division-abort-spec` (codegen/verify/runtime triplicated div-abort policy, 3 crates) | 4 — crosses a tier seam plus byte-identical-bootstrap risk; human-scheduled. The nearby unverified soundness asymmetry noted in the 2026-09-04 report is still for a human to triage. |

## Pick

**`builtin-constructor-spec`, 22/25.** Deterministic top of the ranking; the backlog's flagged
"natural next pick," now unblocked since its 22-point tie-mate `integer-literal-range-fit` (#1241)
merged 2026-09-04. Friction re-verified present and unchanged: commits #1263/#1264 touched vow-diag and
the self-hosted `checker.vow`, not `vow-types/src/check.rs`, so the EnumConstruct arm is byte-for-byte
what the 2026-09-04 firing scored.

**The pick was close** — the top two are within 1 point (22 vs 21). The runner-up **candidate** is
`coerce-context-argument-epilogue` (21/25), which beats the equally-scored `call-argument-coercion-action`
on the heat tie-break (5 vs 4) and avoids its byte-identical-bootstrap risk. Either is the natural next
firing.

## Design

Three interfaces were produced by parallel sub-agents (design-it-twice), each briefed for a
radically different philosophy. The sub-agents' own advisor was rate-limited; adjudication used the
routine advisor against the fixed criteria (depth → locality → seam placement → test surface → blast
radius).

### Design A — minimal surface

Two tiny pure fns: `nullary_builtin_result_ty(enum, variant) -> Option<Ty>` (the 5 fixed-result
builtins) and `payload_builtin_result_ty(enum, variant, &payload) -> Option<Ty>` (Some/Ok/Err wrap
shape). `String::from` and **both** `from_raw_parts_copy` blocks stay fully inline. Net line count ~flat.
Deepens only 8 of 11 builtins on the result-type axis; the ~110-line FFI coercion mass (duplicated
verbatim between `String` and `Vec`) is left inline and un-deduplicated. Weakness: walks away from the
arm's largest, gnarliest block — exactly the pain motivating the refactor.

### Design B — maximum expressiveness (uniform `CtorSpec`)

One uniform `CtorSpec { display, arity: Option<Arity>, args: ArgPolicy, result: ResultShape }` plus
`Arity`, `ArityRecovery`, `ArgPolicy` (4 variants), `ResultShape`, `Slot` enums, and a 3-method
`&mut self` interpreter. Fully declarative — a new builtin is one data literal *within the shape
family*. Sub-agent's own honest estimate: **+220–230 new lines, ~370 lines of diff churn** — by far the
largest diff of the three (the step-5 mid-flight bail is on *file count* > 2×, not line churn, and B is
still one file, so it would not trip the bail; it simply loses criterion 5). Weakness: forced
uniformity makes illegal states representable (`CapturePayload`+`arity:Some`, `WrapPayload` with two
payload slots, etc.), and the interface is nearly as wide as the implementation it hides — the
shallow-module smell this exercise exists to remove.

### Design C — mirror the landed-seam pattern (WINNER)

```rust
enum PayloadWrap { Some, Ok, Err }          // Some(p)->Option<p>, Ok(p)->Result<p,Unit>, Err(p)->Result<Never,p>
enum BuiltinConstructor {
    Fixed(Ty),                              // String::new->Str; {HashMap,BTreeMap,Vec}::new, Option::None -> Never
    Payload(PayloadWrap),                   // Some/Ok/Err — call site evaluates fields.first(), seam wraps
    StringFrom,                             // exact-Str, arity 1 — body stays verbatim at the call site
    RawParts { display, signature, result },// from_raw_parts_copy (String/Vec) — one arm dedups the two blocks
}
fn builtin_constructor(enum_name: &str, variant_name: &str) -> Option<BuiltinConstructor>
```

Pure, total, name-keyed → neutral value; `None` falls through to the unchanged `lookup_enum`
user-enum path. The call site is a flat 4-arm `match`, owning every `check_expr`, the
`check_contextual_integer_literal_ranges` side effect, and all diagnostics — exactly the contract of
the four landed seams (`method_result_type`, `method_argument_expectations`, `cast_verdict`,
`same_operand_ty`). **Refinement applied for the implementation** (safer than the sub-agent's
`ArgExpect`-reuse variant): the `StringFrom` and `RawParts` call-site arms keep the *original argument
predicates verbatim* (`arg_ty != Ty::Str && arg_ty != Ty::Never`; `!can_context_coerce(&arg_ty,
&Ty::I64)`) rather than routing through `ArgExpect::accepts`, so no coercion-equivalence lemma has to
be proved — behaviour is byte-identical by construction. `RawParts { display, signature, result }`
collapses the two near-verbatim `from_raw_parts_copy` blocks (they differ only in those three values)
into a single parameterized arm — the real locality win Design A leaves on the table. Net ~−45 lines.

### Adjudication

| Criterion (in order) | A (minimal) | B (uniform spec) | C (landed pattern) |
|---|---|---|---|
| 1. Depth | deepens 8/11; two narrow fns | 1 query hides 11 **but** interface ≈ impl width | **all 11 behind one enum + one fn** |
| 2. Locality | fixed/payload localized; checked builtins get no help | best *within envelope*, undermined by illegal states | all 11 in one table; honest |
| 3. Seam placement | two narrow seams | right place, over-built | **names→neutral value, matching 4 proven landed adapters** |
| 4. Test surface | 8/11 Checker-free | pure spec testable | **11 `assert_eq` + `PayloadWrap::apply`, mirrors `cast_verdict_*` unit tests** |
| 5. Blast radius | smallest (~flat) | largest (~370-line churn, still 1 file) | small (~−45 net) |

**Winner: C.** It resolves on criterion 1 (depth): C covers all 11 builtins behind one coherent,
narrow interface, where A deepens only 8/11 and abandons the gnarliest FFI block, and B's interface is
nearly as wide as the implementation it hides. C also matches the four already-landed seams' proven
seam placement — pattern consistency is itself leverage for an AI-navigable codebase (a maintainer
learns one pattern, not five) — and has the most-precedented test surface. B ranks last on criterion 1
(its 5-type interface is the shallow-module smell the exercise fights, and it makes illegal states
representable) and criterion 5 (largest diff of the three) — all three are single-file, so none trips
the step-5 file-count bail; B is simply the weakest, not disqualified by a rule. The criteria separate
the designs cleanly, so this is a decision, not a bail.

**Runner-up design: A (minimal surface).** Safe and small, but under-delivers on depth/locality — it
deepens 8/11 and leaves the duplicated FFI coercion mass inline, which is precisely the friction the
pick exists to remove.

**Carried to the PR body:** winner C; runner-up design A and why it lost (deepens only 8/11, leaves the
FFI dedup on the table).

**Out-of-scope note surfaced by the sub-agents:** `check_contextual_integer_literal_ranges`
(~L1660-1669) *also* hard-codes the `Some`/`Ok`/`Err` payload-index knowledge — a second site mirroring
constructor shapes. Unifying it is a separate seam, deliberately not in this diff.
