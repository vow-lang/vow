# Architecture deepening backlog

Persisted candidate memory for the `pm-deepen` routine. Statuses: proposed | in-flight |
landed | dropped | rejected. Never delete rows — `landed`/`dropped`/`rejected` are the memory
that stops the next firing re-deriving them. See `.architecture/reviews/` for the scored reports.

## builtin-method-spec

- **Status**: in-flight
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (**actual: 1**). Diff estimate was ~250 removed / ~120 added plus ~60 test
  lines; **actual 430 added / 384 deleted in 1 file, net +46**. About 262 of that churn is the five
  kept arms re-indented one level into the new `else` block, counted on both sides; excluding it,
  real churn is ~552 against ~430 estimated — a 28% overshoot, inside the 2x bail-out threshold, and
  driven by the table growing from an estimated 18 rows to 20.
- **Modules**: `vow-ir/src/lower/mod.rs:3519-3902` — the `match (recv_struct, method)` in the
  `ExprKind::MethodCall` arm of `lower_expr` (`:1405`). Seam sited beside the landed `builtin_result_tag`
  (`:199-259`) and the `vow_static_builtin_to_runtime` name table (`:60-159`).
- **Summary**: table the 18 uniform builtin-method lowering arms into a pure
  `builtin_method_spec(recv, method) -> Option<MethodSpec>` carrying `{symbol, ret_ty, arity,
  consume_arg, missing_arg, result_struct_tag}`, with a generic applier keeping `ctx.emit` and the
  `inst_struct_type` insert at the call site. The 7 non-uniform arms (`String::substring`,
  `String::parse_i64`, `String::parse_u64`, `HashMap::insert`, `BTreeMap::insert`, `Vec::push`, the
  `unwrap` fallback) stay inline — each does real extra work. Rust-only; `compiler/lower.vow` (which
  mirrors the same arms at `:3810/3826/3949/3967`) is untouched because the change is
  behaviour-preserving, so no new drift is introduced — same precedent as `builtin-result-tag`.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **PR**: #1299
- **Reason**: **picked this firing** (2026-09-18). Fresh candidate; the arm had never been carded
  despite heavy `lower/mod.rs` coverage (prior firings carded `unwrap-payload-ty` and the landed
  `builtin-result-tag`, never the method-dispatch table). Leverage 4 by precedent with the landed
  `builtin_constructor_spec` and `builtin_result_tag` — same shape: one dispatch site, a name-keyed
  table, a pure spec output. Heat 5: `lower/mod.rs` was touched two days prior by PR #1290, which
  extracted the sibling `builtin_result_tag` seam from this same file. **The pick was close — three
  runners-up at 21**: `vec-reserve-next-capacity-seam`, `arena-variant-rule` and
  `esbmc-auto-timeout-policy`. Won on being the only candidate at heat 5, on being purely
  behaviour-preserving (the other two 21s change behaviour on at least one path), and on sitting in
  `vow-ir`, where IR-shape `#[cfg(test)]` assertions plus the 213-program `tests/run/` corpus check
  the change — unlike `vow-codegen`, which produces stage-0 `vowc1` with no IR-level diff to lean on.
  Three drifts the table makes visible: `String::contains` (`:3557`) uses `lower_expr` where all 17
  siblings use `lower_consumed_expr`; `Vec::truncate` (`:3868`) falls back to `ConstI64(0)` where the
  others use `ConstUnit`; `BTreeMap::get` (`:3710`) tags its result `"Option"` while `HashMap::get`
  (`:3772`) returns a bare `Ty::I64` and tags nothing. **No overlap with `builtin_result_tag`**: that
  seam is keyed on free-function builtin names and dispatches only at `:1815`; `BTreeMap::get`'s bare
  `"Option"` tag (no element type) is not expressible as `BuiltinResultTag::OptionOf(Ty)`.
  Adjudicated **design A** (minimal surface: one enum + a 4-tuple) over **C** (shape-variant enum,
  the runner-up design), **B** (uniform spec, 24 arms) and **D** (single adapter, spec private). A
  and C tied on depth, seam placement and test surface; A won on blast radius. A correction during
  adjudication changed the winner: the first pass ranked C above A on depth, believing only C could
  table `parse_i64`/`parse_u64` — A's result tag is the fourth tuple slot, independent of
  `MethodArg`, so A tables them too, reaching 20 of 25 arms.
  **Landed:** IR identity verified differentially against `HEAD~1` over `tests/run/` — 212 identical,
  0 different, 1 skipped for a type error pre-existing on both compilers. Gate: build, clippy
  `-D warnings`, 1676 tests / 0 failures, fmt — all green. `clippy::type_complexity` does **not**
  fire on the 4-tuple, settling design A's one open question empirically.

## builtin-result-tag

- **Status**: landed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (actual: 1; +79/-31 in `vow-ir/src/lower/mod.rs`)
- **Modules**: `vow-ir/src/lower/mod.rs` (`tag_builtin_result` L201-251, single caller L1808;
  seam sited beside the pure `narrow_intrinsic_target` L161-183 and the `vow_static_builtin_to_runtime`
  name table L60-159)
- **Summary**: extract the ~40-name builtin-result classification (String heap / Vec heap /
  `Option<elem_ty>`) into a pure `builtin_result_tag(name) -> Option<BuiltinResultTag>`, mirroring the
  landed `builtin_constructor_spec` seam; `tag_builtin_result` keeps only the `ctx.inst_struct_type`
  / `inst_option_elem_ty` inserts. Preserves the `_try`+`narrow_intrinsic_target` early-path ordering
  (the `i16_to_u8_try` / `i64_to_i32_try` fall-through traps). Rust-only; `compiler/lower.vow`
  untouched (behaviour-preserving, no new drift).
- **First seen**: 2026-09-16
- **Report**: `.architecture/reviews/2026-09-16-builtin-result-tag.md`
- **PR**: #1290 (merged 2026-09-16; reconciled 2026-09-18 via `gh pr view 1290` → MERGED)
- **Reason**: **picked the 2026-09-16 firing**. Fresh candidate — `tag_builtin_result` was hand-
  edited by #1288 two days prior (adding `proc_sample`), evidence the untested list already drifts.
  Scored leverage 4 by precedent with the landed `builtin_constructor_spec` (same shape: one dispatch
  site, name-keyed table, pure spec output). Tied `call-argument-coercion-action` at 22; won the
  deterministic tie-break on most-recently-touched file (`lower/mod.rs` #1288 > `cranelift_backend.rs`
  #1279). Runner-up *candidate* by score `coerce-context-argument-epilogue` (21, within 1 pt).

## question-operator-verdict

- **Status**: landed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (actual: 1)
- **Modules**: `vow-types/src/check.rs` (`ExprKind::Question` arm of `check_expr_inner`, ~L2776-2830;
  seam sited beside `cast_verdict` L668 / `same_operand_ty` L808)
- **Summary**: extract the inline 5-way `?`-operator payload policy (Option-ok → unwrap arg0;
  Option-without-Option-return → reject; Result → reject "not lowered"; `Never` → propagate; anything
  else → reject "needs Option/Result") into a pure
  `question_verdict(inner_ty, return_ty) -> Result<Ty, QuestionReject>`, mirroring the landed
  `same_operand_ty` Result-shaped seam; diagnostic emission and the `pattern_aggregates` bookkeeping
  stay at the call site.
- **First seen**: 2026-09-14
- **Report**: `.architecture/reviews/2026-09-14-question-operator-verdict.md`
- **PR**: #1281 (merged 2026-09-14; reconciled 2026-09-16 via `gh pr view 1281` → MERGED)
- **Reason**: **picked this firing** (2026-09-14). Fresh candidate surfaced by the step-1 scan; not
  previously carded despite heavy `check.rs` coverage (prior firings carded the cast/operator/method
  arms, never the `?` arm). Scored leverage 4, matching the landed single-arm pure-verdict seams
  `cast_verdict`/`builtin-constructor-spec`; above the leverage-3 error-reason-only extractions
  (`arm-pattern-support-classifier`, `builtin-receiver-kind`) because the success path returns a
  computed payload `Ty` used downstream, not just a rejection reason. Deterministic top at 22; pick was
  close — runner-up candidates `coerce-context-argument-epilogue` and `call-argument-coercion-action`
  at 21 (within 1 pt). Adjudicated
  design C (Result-shaped, mirrors `same_operand_ty`) over A (classification enum à la `cast_verdict`)
  and B (verdict owns the diagnostic strings). Adjudicated without an advisor (rate-limited this run).

## same-operand-type-verdict

- **Status**: landed
- **Score**: 23/25 (leverage 4, locality 5, blast radius 1, heat 5)
- **Files**: ~1 estimated (actual: 1)
- **Modules**: `vow-types/src/check.rs` (`check_same_numeric`, `check_same_integer`;
  seam `same_operand_ty` at L784, `OperandError` L763, `OperandClass`)
- **Summary**: collapse the two line-for-line-twin operand-type checks into one pure
  `same_operand_ty(lhs, rhs, class) -> Result<Ty, OperandError>` (Design C, adjudicated over the
  bespoke-enum Design A), mirroring the landed `cast_verdict` seam; each method keeps only its distinct
  `ErrorCode::TypeMismatch` message/hint at the call site.
- **First seen**: 2026-09-03
- **Report**: `.architecture/reviews/2026-09-03-same-operand-type-verdict.md`
- **PR**: #1234 (merged 2026-09-03)

## integer-literal-range-fit

- **Status**: landed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (actual: 1)
- **Modules**: `vow-types/src/check.rs` (`literal_out_of_range` seam beside `integer_type_range`;
  adapter `check_integer_value_range`; call sites L1522, L2003)
- **Summary**: extract the pure literal-fits-target decision + range-text into
  `literal_out_of_range(value, target) -> Option<String>` (Design A, adjudicated over the
  neutral-bounds newtype Design C on depth), leaving `emit_error_with_hints` at the call site; pins
  the `negative_max`/`i64::MIN` asymmetry.
- **First seen**: 2026-08-31
- **Report**: `.architecture/reviews/2026-09-04-integer-literal-range-fit.md`
- **PR**: #1241 (merged 2026-09-04)

## call-argument-coercion-action

- **Status**: proposed
- **Score**: 19/25 (leverage 3, locality 3, blast radius 1, heat 4)
- **Files**: ~1 estimated
- **Modules**: `vow-codegen/src/cranelift_backend.rs:425-459` (`coerce_call_argument`; re-anchored
  2026-09-18 from ~L398-432). 2 production call sites (L1523, L1630) + 6 test; shim twin at
  `vow-clif-shim/src/lib.rs` has 2 production + 4 test.
- **Summary**: extract the pure coercion decision into
  `call_argument_coercion(actual_bits, expected_bits, is_i128, signed) -> CoercionAction`, leaving
  `builder.ins()` emission at the call site. Codegen must stay a byte-identical bootstrap fixed point.
- **First seen**: 2026-08-31
- **Reason**: **rescored 21→19 on 2026-09-18** (leverage 4→3, locality 4→3). New evidence: there is no
  shared crate to hold a `CoercionAction` type — `vow-clif-shim` depends only on `vow-linker` +
  cranelift, `vow-codegen` on `vow-ir`. Extraction therefore yields *two* independently-testable
  copies that still must be hand-synced, buying none of the cross-crate agreement that is the only
  thing worth paying for here. Both sides are already tested
  (`abi_and_field_widening_preserve_logical_signedness`,
  `call_argument_coercion_refuses_only_wide_narrowing`, each with a shim twin). Also corrected: the
  earlier "byte-identical-bootstrap risk" framing was wrong — the fixed point is
  `sha256(vowc2) == sha256(vowc3)`, both produced by the self-hosted pipeline
  (`scripts/bootstrap.sh:224-225`); `vow-codegen` produces only stage-0 `vowc1`. The real risk is that
  a silent stage-0 miscompile has no IR-level diff to catch it.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## arm-pattern-support-classifier

- **Status**: proposed
- **Score**: 20/25 (leverage 3, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs:3167-3225` (`validate_arm_pattern`; classifying match
  3168-3212, emit tail 3214-3224). Re-anchored 2026-09-18 from ~L3045.
- **Summary**: extract the pure unsupported-match-arm reason into
  `unsupported_arm_pattern(pat, is_last) -> Option<(&str, &str)>`, leaving the emit/return-bool
  wrapper at the call site.
- **First seen**: 2026-08-31
- **Reason**: re-heated 19→20 as `check.rs` moved to heat 5. Re-verified 2026-09-18: the classifying
  match is already a value-returning expression with zero `&self` — the cleanest pure shape in the
  file and the lowest-risk extraction — but no defect sits behind it, which is what holds leverage
  at 3.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## ce-trace-reconstruction

- **Status**: proposed
- **Score**: 18/25 (leverage 3, locality 4, blast radius 1, heat 3)
- **Files**: ~1 estimated (26 + 25 lines)
- **Modules**: `vow/src/counterexample.rs:331-356` (execution path) and `:357-381` (branch decisions),
  inside `build_structured_counterexample_with_module` (`:188-399`, 212 lines). Re-anchored
  2026-09-18 (drift +4).
- **Summary**: extract the two pure block-visit → source-trace loops into
  `reconstruct_execution_path` / `reconstruct_branch_decisions`, leaving blame/name/call-site work in
  the builder.
- **First seen**: 2026-08-31
- **Reason**: **rescored 20→18 on 2026-09-18** (leverage 4→3): both loops are total pure functions of
  `(&[BasicBlock], &HashSet<u32>)`, but each has exactly one call site, so nothing "stops reaching
  past a seam". The payoff is test surface, and it is real: the only test covering both loops
  (`:1743-1840`, `execution_path_and_branch_decisions_from_block_visits`) spends **98 lines of
  scaffolding** — a full `Function`, 3 `BasicBlock`s, 4 `Inst`s, a `VowEntry`, a `Counterexample`, an
  empty `call_site_index` — to assert 8 facts. Heat 3 (`counterexample.rs` 7 commits/90d).
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## narrow-shift-findings

- **Status**: proposed
- **Score**: 20/25 (leverage 3, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs:2084-2115` (narrow `Shl`/`Shr` arm; re-anchored 2026-09-18
  from ~L1875-1906)
- **Summary**: extract the two independent shift-operand checks (count must be `u32`; const count in
  range) into a pure seam returning a **struct of two findings** (not a single-variant enum — both can
  fire on one expression via `Cast`-folded const counts).
- **First seen**: 2026-09-03
- **Reason**: re-verified 2026-09-18; best-pinned candidate in `check.rs` (8 existing tests at
  L4427/4436/4452/4492/4501/4517/4821/4383). The `.expect("narrow shift types have an integer width")`
  at L2092 re-derives a width the arm's own guard already decided — an `Option<u16>` predicate
  removes it.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## builtin-constructor-spec

- **Status**: landed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (actual: 1; +260/-131 in `vow-types/src/check.rs`)
- **PR**: #1268 (merged 2026-09-11)
- **Modules**: `vow-types/src/check.rs` (`EnumConstruct` builtin dispatch, ~L2867-3016)
- **Summary**: extract the inline `match (enum, variant)` builtin-constructor policy (arity, per-arg
  `ArgExpect`, result shape) into a pure `builtin_constructor_spec(enum, variant) -> Option<CtorSpec>`
  table, mirroring the landed `method_result_type`/`method_argument_expectations` seams; the generic
  argument-checking loop and payload-wrapping stay at the call site.
- **First seen**: 2026-09-04
- **Report**: `.architecture/reviews/2026-09-11-builtin-constructor-spec.md`
- **Reason**: picked 2026-09-11; **merged 2026-09-11** (reconciled 2026-09-14 via
  `gh pr view 1268` → MERGED). Adjudicated design C (mirror the landed-seam pattern) over A (minimal,
  deepens only 8/11) and B (uniform spec, widest interface).

## comparison-operand-verdict

- **Status**: proposed
- **Score**: 18/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (~52 lines)
- **Modules**: `vow-types/src/check.rs` (`Eq|Ne|Lt|Le|Gt|Ge` sub-arm, now ~L2028-2080)
- **Summary**: fold the comparison type-mismatch predicate + zero-comparison-tautology (with the
  `widened_to` split) into a pure `comparison_verdict(lhs, rhs, never_negative) -> ComparisonVerdict`;
  caller pre-computes `never_negative` from the existing `zero_comparison_verdict` +
  `never_negative_operand` helpers, message/hint strings stay at the call site.
- **First seen**: 2026-09-11
- **Reason**: **rescored 20→18** on 2026-09-16 (leverage 3→2): the step-1 explore pass found the
  tautology half is already extracted (`zero_comparison_verdict` L565, `never_negative_operand`
  L1927) and the composite verdict cannot be fully pure — `never_negative_operand` reads
  `self.nonneg_casts` — so the only un-extracted pure piece is a one-line mismatch predicate.
  Partially resolved by prior seams. Eligible but low; not vetoed. Re-verified 2026-09-18 at
  `check.rs:2028-2080` (line numbers unchanged): `self.nonneg_casts` is genuinely necessary — a
  chained `v as u32 as i64` carries its root source forward — so any seam must take the *lookup
  result* as an argument rather than `&self`. Unusually dense existing coverage (16 tests,
  `:4591`-`:4815`).
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## builtin-function-spec

- **Status**: proposed
- **Score**: 18/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (~70 lines)
- **Modules**: `vow-types/src/check.rs:2212-2240` (`pin_to_root`) and `:2241-2281`
  (`string_matches_literal_at`), ahead of the ordinary `env.lookup_fn` path at `:2282`.
  **Re-anchored 2026-09-18**: the carded ~L2082-2151 is now the bitwise/shift/logical arms.
- **Summary**: table the two free-function builtins into `builtin_function_spec(name) -> Option<..>`,
  mirroring `builtin-constructor-spec`, generic arg loop + static-literal emit stay at the call site.
- **First seen**: 2026-09-11
- **Reason**: **rescored 20→18** on 2026-09-16 (leverage 3→2): the step-1 explore pass confirmed the
  extraction "mostly moves" — only two bespoke builtins, each with its own per-arg checking that
  cannot leave the `&mut self` call site, so the pure spec mostly relabels. Thin. Re-verified
  2026-09-18: `env.rs:140-460 builtin_free_fn_signatures()` is already a 320-line table of builtin
  free functions, and these two are the *exceptions* that did not fit it — so any seam here should be
  shaped as "what extra constraint does this intrinsic impose beyond its signature", not as a second
  signature table. Neither has a named test today, so the red step is larger.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## oversized-chunk-path-predicate

- **Status**: proposed
- **Score**: 17/25 (leverage 2, locality 3, blast radius 1, heat 4)
- **Files**: ~1 estimated (2 sites)
- **Modules**: `vow-runtime/src/lib.rs:1011` (inside `__vow_arena_alloc`, `:983-1047`) and `:1237`
  (inside `arena_grow_backing`, `:1209-1243`). Re-anchored 2026-09-18 (drift −9).
- **Summary**: single-source the must-agree `bytes > OVERSIZED_THRESHOLD || bytes + (align-1) >
  CHUNK_PAYLOAD` predicate into `const fn takes_oversized_path(bytes, align) -> bool`, called from
  both sites; the L1246 comment documents that its correctness depends on the L1020 decision.
- **First seen**: 2026-09-11
- **Reason**: leverage 2 — one-line predicate; value is a drift-kill, not interface depth. Distinct
  from `vec-reserve-next-capacity-seam`. Re-verified 2026-09-18 with new evidence: the two sites have
  *different* overflow protection. `:1011` is preceded by an explicit guard at `:1016-1019`; `:1237`
  has none, and its safety rests on a **non-local argument** — `__vow_arena_alloc(arena, new_size,
  align)` at `:1221` runs first and guards `new_size + align`, and `old_size < new_size`. Nothing in
  `arena_grow_backing` states this. Both sites also underflow `align - 1` when `align == 0`, so the
  seam should use `saturating_add`/`saturating_sub` and be total.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## coerce-context-argument-epilogue

- **Status**: proposed
- **Score**: 18/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs` — **site inventory corrected 2026-09-18**: 5 loop-shaped sites
  (`:2254-2267`, `:2340-2353`, `:2919-2943`, `:3056-3072`, `:3116-3133`), 3 single-value sites
  (`:1525-1542`, `:1613-1629`, `:2797-2814`), 2 near-misses on a *different* predicate
  (`:2362-2382` via `ArgExpect::accepts`, `:2862-2874` via the strictly-weaker
  `can_assignment_coerce`), and 2 range-check-only sites with no coercion test (`:1651`, `:2538`).
  The earlier "~10 sites" list was stale.
- **Summary**: collapse the repeated `check_expr + check_contextual_integer_literal_ranges +
  can_context_coerce + emit-mismatch` epilogue into one `&mut self` helper taking a per-site message
  closure. `can_context_coerce` (L263) is already pure; this is sibling-epilogue dedup, not a pure
  seam. Not compiler drift — `compiler/checker.vow` inlines the same glue.
- **First seen**: 2026-09-04
- **Reason**: **rescored 21→18 on 2026-09-18** (leverage 3→2, locality 5→4). The policy is *already*
  pure (`can_context_coerce`, `:263-286`); what repeats is sequencing, and the diagnostic message
  differs at every site, so the honest extraction is a `&mut self` helper taking a message closure —
  a shallow 4-param wrapper of 3 statements, which fails the deletion test on the helper itself.
  Score it as a shallow-helper cleanup, not a deepening. The genuinely interesting finding nearby is
  an *inconsistency*, not the duplication: `:2538` range-checks an index against `I64` while
  `ArgExpect::AnyInteger` (`:318`) range-checks against `I64` but accepts any width, and `:2862` uses
  a strictly weaker predicate than the other nine. A `CoercionSite` enum naming which predicate
  applies where would be the honest deepening, and it is a larger job.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## recover-unknown-name-prologue

- **Status**: proposed
- **Score**: 18/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs` — **re-anchored 2026-09-18, 5 sites not 4**: `:1991-2005`
  (undefined variable, recovery `Ty::Unit`), `:2285-2304` (undefined function, recovery `Ty::Never`),
  `:2455-2470` (unknown method, + "available methods:" fallback), `:2503-2521` (unknown field, +
  "available fields:" fallback), `:2900-2916` (unknown struct, recovery `Ty::Unit`). All four carded
  line numbers were stale.
- **Summary**: collapse the five unresolved-name recovery epilogues (`suggest_similar` + emit + drain
  child exprs + fallback `Ty`) into one `&mut self` recovery helper. `suggest_similar` (L203) already
  pure; low leverage, divergent messages. The pure part worth extracting is
  `did_you_mean_hints(name, candidates, list_fallback) -> Vec<String>`; candidate *sourcing* and the
  recovery-type choice legitimately differ per site and stay put.
- **First seen**: 2026-09-04
- **Reason**: re-verified 2026-09-18; three hint variants exist (suggestion-only for var/fn/struct,
  suggestion-else-list for method/field) and the recovery type differs (`Unit` vs `Never`) for
  reasons documented only at `:2301-2303`. Pairs naturally with `hint-candidate-capping`.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## division-abort-spec

- **Status**: proposed
- **Score**: 16/25 (leverage 3, locality 4, blast radius 4, heat 4)
- **Files**: ~3 estimated (3 crates)
- **Modules**: `vow-codegen/src/cranelift_backend.rs` (`divisor_abort_condition`, ~L325);
  `vow-verify/src/c_emitter.rs` (`emit_checked_arith`, ~L2882); `vow-runtime/src/lib.rs`
  (`define_wide_div_rem!`, ~L2987)
- **Summary**: extract the triplicated "zero divisor aborts all div/rem; signed div also aborts on
  `MIN/-1`" policy into a shared pure `division_abort_spec(opcode, ty) -> {check_zero,
  check_min_neg_one}`; each backend keeps its own emission. `vow-runtime` L2987 documents the manual
  sync.
- **First seen**: 2026-09-04
- **Reason**: blast radius 4 (crosses codegen/verify/runtime) plus byte-identical bootstrap risk — a
  human-scheduled pick, not unattended. NB an unverified soundness asymmetry noticed nearby (codegen
  traps wrapping-div-by-zero, verifier may not model it) is recorded in the 2026-09-04 report for a
  human to triage; deliberately not filed. Re-checked 2026-09-18: still blast radius 4, still
  triplicated across `vow-codegen`/`vow-verify`/`vow-runtime`.

## vec-reserve-next-capacity-seam

- **Status**: proposed
- **Score**: 21/25 (leverage 4, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated (26 + 7 + 8 lines across 3 sites)
- **Modules**: `vow-runtime/src/lib.rs:1497-1522` (`vec_reserve_in_arena_no_null_check`),
  **`:4193-4199`** (`__vow_map_insert_in_arena`), **`:4362-4369`** (`__vow_btreemap_insert`).
  Two sites added 2026-09-18; `vec_reserve` re-anchored from ~L1473-1519.
- **Summary**: extract the capacity-doubling/overflow policy into a pure
  `next_capacity(old_cap, required, initial) -> Option<usize>` (`None` = caller must `oom_trap`),
  called from all three growth sites; `oom_trap` and the `arena_grow_backing` call stay at each site.
- **First seen**: 2026-08-31
- **Reason**: **rescored 18→21 on 2026-09-18** (leverage 3→4, locality 3→4). New evidence: there are
  *three* growth sites with divergent overflow policy, not one. `Vec::reserve` uses
  `checked_mul(2)` + a `< VOW_CAP_VALUE_MASK` guard + `checked_mul(elem_size)`, and its comment
  (`:1491-1495`) explains exactly why unchecked doubling is a bug — "doubling `new_cap` past
  usize::MAX wraps it to 0 so the `< required` loop never terminates". The two map paths then do
  precisely that: `m.cap * 2` and `new_cap * MAP_ENTRY_BYTES` are unchecked at `:4195-4196`, as are
  `m.keys_cap * 2` and `new_cap * BTREEMAP_ENTRY_BYTES` at `:4364-4365`. NB both maps are constructed
  with an initial cap of 8 (`:4160`, `:4306`), so the zero-cap doubling hazard is unreachable through
  today's constructors and the overflow needs a capacity above `usize::MAX / 2` — this is a latent
  divergence to close, not a live bug, and a PR landing it must say so. Runner-up **candidate** this
  firing (21, one point behind the pick).
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## negation-verdict

- **Status**: proposed
- **Score**: 20/25 (leverage 3, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (38 lines across 2 sites)
- **Modules**: `vow-types/src/check.rs:2161-2183` (`UnaryOp::Neg` arm of `check_expr_inner`) **and
  `:1696-1710`** (the `UnaryOp{Neg}` branch of `check_integer_literal_range`) — second site added
  2026-09-18
- **Summary**: extract the 3-way `{Unsigned, NonNumeric, Ok}` negation decision into a pure verdict,
  leaving the two emits and the literal-folding (L2138-2158) at the call site.
- **First seen**: 2026-09-03
- **Reason**: **rescored 18→20** on 2026-09-16 (leverage 2→3): the arm is a *total pure function of
  `operand_ty`*, testable with zero `&mut self`. **Justification strengthened 2026-09-18** (score
  unchanged): the unsigned-negation rule is encoded *twice* with byte-identical message text
  (`"unary negation is not allowed on unsigned type ..."` at `:1702-1706` and `:2165-2167`), and the
  two sites already disagree on recovery (`:1707` recurses into the operand, `:2170` returns
  `Ty::Unit`). The `Unsigned` branch of the `:2161` arm has no test — `unary_neg_numeric_ok` (:4259)
  and `unary_neg_non_numeric_error` (:4271) cover only the other two.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## builtin-receiver-kind

- **Status**: proposed
- **Score**: 17/25 (leverage 2, locality 3, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs:2383-2395` (5 `matches!` booleans) + `:2438-2454` (the
  display-name ladder). **Re-anchored 2026-09-18** from ~L2174-2245.
- **Summary**: extract the pure receiver-kind classification into
  `builtin_receiver(ty: &Ty) -> BuiltinReceiver` with a `display_name` method; marginal on its own,
  because its primary output is a display string the landed-seam pattern keeps at the call site.
- **First seen**: 2026-09-03
- **Reason**: re-verified 2026-09-18 and now demonstrably redundant — `:2446`/`:2449` re-derive
  Option/Result with fresh `matches!` instead of reusing `is_option_or_result` computed at `:2393`,
  and the same classification is spelled a third and fourth time inside `method_result_type`
  (`:392-420`) and `builtin_method_names` (`:441-451`). A shared discriminator would make adding a
  builtin container a one-place edit instead of four. Held at leverage 2: no caller simplifies.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## unwrap-payload-ty

- **Status**: proposed
- **Score**: 20/25 (leverage 3, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (9 + 21 + 9 lines across 3 sites)
- **Modules**: `vow-ir/src/lower/mod.rs:4206-4214` (`lower_unwrap`, itself now `:4145-4226`),
  **`:3264-3284`** (`ExprKind::Match` arm), **`:4022-4030`** (`ExprKind::Question`, the `?` desugar).
  Two sites added 2026-09-18; `lower_unwrap` re-anchored from ~L4186-4194.
- **Summary**: extract the payload-slot type selection into a total pure
  `payload_slot_ty(aggregate, declared_wide, recorded, slot_index) -> Ty`, with each site
  pre-computing its own `ctx` lookups; `ctx.emit` stays at the call site.
- **First seen**: 2026-09-03
- **Reason**: **rescored 17→20 on 2026-09-18** (leverage 2→3, heat 4→5). As carded — one 9-line
  selection with one call site — it was too thin. There are three sites sharing only the two-rung
  aggregate prefix (`is_linear → LinearPtr`, `is_some → Ptr`) and diverging after it: `:3264-3284`
  adds a `declared_wide_payload_ty` rung and an `if i == 0` slot rule, and **`:4022-4030` omits the
  `variant_payload_ty` consult the other two make**. The extraction must *preserve* that asymmetry,
  not silently fix it — the seam's job is to make it visible. Separately: the comment at `:4202-4205`
  says the omitted lookup is what stops a wide `Result` payload loading as a truncated limb, so the
  `?`-desugar site is either deliberately exempt or latently wrong. **Confirm before unifying; do not
  fold that question into the extraction.**
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## cast-legality-verdict

- **Status**: landed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (actual: 1)
- **Modules**: `vow-types/src/check.rs` (`ExprKind::Cast` arm of `check_expr_inner`)
- **Summary**: extract the inline 4-way `as`-cast legality policy into a pure
  `cast_verdict(src, tgt) -> CastVerdict`, mirroring the `method_result_type` seam, leaving
  diagnostics and the `nonneg_casts` mutation at the call site.
- **First seen**: 2026-08-31
- **Report**: `.architecture/reviews/2026-08-31-cast-legality-verdict.md`
- **PR**: #1161 (merged 2026-09-01)

## builtin-method-result-type-seam

- **Status**: landed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (actual: 1)
- **Modules**: `vow-types/src/check.rs` (`ExprKind::MethodCall` arm of `check_expr`, ~L1941-2122)
- **Summary**: extract the inline builtin-method result-type and known-methods resolution into pure
  free functions mirroring the existing `method_argument_expectations` seam, leaving diagnostics in
  `check_expr`.
- **First seen**: 2026-08-31
- **PR**: #1153 (merged 2026-08-31)
- **Report**: `.architecture/reviews/2026-08-31-builtin-method-result-type-seam.md`

## arena-variant-rule

- **Status**: proposed
- **Score**: 21/25 (leverage 4, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated (136 lines, 22 arms)
- **Modules**: `vow-codegen/src/cranelift_backend.rs:769-904`; 2 production call sites (`:1582`,
  `:3156`). Target shape already exists in `vow-clif-shim/src/lib.rs:625-819`
  (`routed_vec_extern(sym, inst_rgn, receiver_route) -> (&str, Option<i64>)`); third copy in
  `compiler/clif.vow:244-380`.
- **Summary**: table the arena-routed extern-symbol rule into
  `arena_variant_rule(sym) -> Option<ArenaVariantRule>` over
  `{InstRegion, Receiver, ReceiverWithCandidate}`, with a ~15-line applier keeping `inst.region` and
  `first_arg_route` at the call site. The 22 arms collapse to two shapes: 17 `match inst.region`
  (`Root` → passthrough, else `"<sym>_in_arena"`) and 5 `first_arg_route`
  (`Direct(Block|Caller)` → `_in_arena`; `ProjectionCandidate` → `_in_candidate_arena`, only for
  `string_push_str`/`string_push_byte`).
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: runner-up **candidate** at 21 (one point behind the pick). Signature-mirroring of a
  landed shape rather than invention. Held below `builtin-method-spec` on risk, not score:
  `vow-codegen` produces stage-0 `vowc1` directly and, unlike a `vow-ir` change, has no IR-level diff
  to catch a regression — only the bootstrap triple. Coverage is the 213-program `tests/run/` corpus
  (`string_substring.vow`, `dealloc_string.vow`, `region_string_trim_root_escape_span.vow`,
  `cmdloop.vow`).

## esbmc-auto-timeout-policy

- **Status**: proposed
- **Score**: 21/25 (leverage 4, locality 4, blast radius 1, heat 4)
- **Files**: ~2 estimated (43 Rust lines)
- **Modules**: `vow-verify/src/esbmc.rs:942-964` (`effective_multi_property_config`) and
  `vow-verify/src/solver_strategy.rs:227-246` (`bv_config_for`). Self-hosted mirror at
  `compiler/verifier.vow:504-517` (`effective_timeout_for`, 5 call sites) is documentation, not an
  edit target.
- **Summary**: extract `auto_timeout_secs(encoding, solver, user_timeout) -> Option<u32>` into
  `solver_strategy.rs`, reusing `SolverConfig::resolve()` for the BV-solver rule instead of the two
  local partial copies.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: runner-up **candidate** at 21. The shared rule — *user `--timeout` wins verbatim; else
  Bitwuzla is uncapped; else `DEFAULT_AUTO_TIMEOUT_SECS`* — is spelled three times and the doc comment
  at `esbmc.rs:936-938` admits it. **Not picked because the two Rust spellings already differ**, so
  unifying them is a behaviour change on at least one path, not a pure extraction:
  `effective_multi_property_config` gates on `config.encoding == Encoding::Auto` and `bv_config_for`
  does not, and both re-derive `resolve()`'s `Auto → Boolector` rule locally in a way that disagrees
  with `resolve()` (`solver_strategy.rs:80-84`) for `(Encoding::Ir, Solver::Auto)`, where it yields
  `Z3`. Deciding which spelling is correct is a fork the autonomy contract reserves for a human. The
  duplication has reached the tests too (`solver_strategy.rs:1091/1102/1115` vs `esbmc.rs:2001`
  assert the same four facts).

## builtin-generic-arity-spec

- **Status**: proposed
- **Score**: 20/25 (leverage 4, locality 4, blast radius 1, heat 3)
- **Files**: ~1 estimated (68 lines)
- **Modules**: `vow-types/src/env.rs:656-723` (`TypeEnv::resolve`, `AstType::Generic` arm); 14
  `resolve` call sites in `check.rs`.
- **Summary**: replace the five hand-written builtin-generic arms with
  `builtin_generic_spec(name) -> Option<BuiltinGeneric>` carrying `{base: Ty, arity: usize}`; one
  arity check, one message shape, then `resolve` mapped over the arguments.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: the five builtin generics use **two different arity policies**. `Option` (`:660`),
  `Result` (`:671`) and `BTreeMap` (`:704`) reject `args.len() != N`; `Vec` (`:682`) and `HashMap`
  (`:692-697`) use `args.first()`/`args.get(1)` with `ok_or_else`, so too *few* arguments error and
  too *many* are silently dropped — `Vec<i64, bool>` resolves to `Ty::Applied(Vec, [I64])`, and the
  surplus argument is never `resolve`d, so an unknown type name in that position is also never
  reported. `vow-syntax/src/parser/types.rs` does not constrain generic arity at parse time.
  Test-shaped gap: `env.rs:1213`/`:1272` have `resolve_{option,result}_generic_rejects_wrong_arity`
  and there is deliberately no `Vec`/`HashMap` equivalent. **Not picked on heat 3** (`env.rs` is 9
  commits/90d, last touched 2026-09-05) — and note it *changes behaviour*: `Vec<i64, bool>` would
  start erroring. That is a correctness fix worth making, but it should land as a fix, not as a
  deepening.

## hidden-region-store-targets

- **Status**: proposed
- **Score**: 19/25 (leverage 3, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated (46 lines)
- **Modules**: `vow-codegen/src/cranelift_backend.rs:214-259` — `hidden_region_store_targets`
  (`:214-226`), `hidden_region_param_count` (`:228-237`), `hidden_region_idx_for_store_target`
  (`:239-259`). 3 call sites.
- **Summary**: retype `hidden_region_store_targets` from `&IrFunction` to `&RegionSummary` so
  `hidden_region_idx_for_store_target` can share it instead of re-inlining the identical
  collect/sort/dedup at `:247-252`; `hidden_region_param_count` then becomes a total pure function of
  `(&RegionSummary, is_main)`.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: the tightest precedent match in the audit — `hidden_region_param_count` is the exact
  twin of the shim's landed `hidden_region_count` (`vow-clif-shim/src/lib.rs:355-365`), same
  `is_main → 0`, `FreshInCaller → +1`, `+ store_targets.len()` shape, and the code already carries a
  "Keep this slot order in sync with compiler/clif.vow's clif_hidden_store_targets" comment at
  `:215-216`. Held at leverage 3: only ~10 lines of real duplication. Coverage:
  `vow/tests/region_summary_equivalence.rs`.

## contracts-clause-status-precedence

- **Status**: proposed
- **Score**: 19/25 (leverage 4, locality 4, blast radius 1, heat 2)
- **Files**: ~1 estimated (89 lines)
- **Modules**: `vow/src/contracts.rs:98-186` (`update_contract_statuses`); 1 call site
  (`run_contracts_command`, `:234`), 5 near-identical mutation loops at `:110-114`, `:122-126`,
  `:138-143`, `:158-162`, `:172-176`.
- **Summary**: have the probe block produce a plain
  `FunctionProbe { modelable, esbmc_found, overall, verdicts, vacuous, trivially_satisfiable }` and
  extract `clause_status(entry, func_id, probe) -> (&'static str, bool)`, leaving `find_esbmc`,
  `emit_*_c_source` and `run_esbmc_*` at the call site.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: the *policy* is the precedence order — non-modelable ⇒ `skipped` and skip all probes;
  missing ESBMC ⇒ `error`; else per-clause `resolve_clause_status`; then a `Vacuous` reach verdict
  **overwrites every status including `proven`**; then body-replace sets an orthogonal boolean. Only
  the innermost step is tested (`resolve_clause_status`, `:466`/`:488`); **the vacuous-override
  precedence has zero unit coverage** — `:339` only tallies pre-set statuses. Today "a `proven`
  clause is demoted to `vacuous`" cannot be tested without installing ESBMC. Held down by heat 2
  (`contracts.rs` is 3 commits/90d, last touched 2026-08-24).

## ce-diagnostic-shaping

- **Status**: proposed
- **Score**: 18/25 (leverage 3, locality 4, blast radius 1, heat 3)
- **Files**: ~1 estimated (69 lines)
- **Modules**: `vow/src/verify_outcome.rs:221-289`, inside `to_output_with_warnings` (`:205-367`,
  163 lines); 1 call site (`:220`).
- **Summary**: extract `counterexample_diagnostic(&StructuredCounterexample) -> Diagnostic`, total and
  pure — it already calls only the extracted `blame_to_error_code` (`:177-183`) and
  `blame_to_diag_blame` (`:185-191`). The `match sce.blame.as_str()` hint selection (`:243-263`) and
  the `UNATTRIBUTED_VOW_ID` code/message split (`:264-280`) move with it.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: low-risk and mechanical; the win is altitude (the 163-line function drops to ~95) rather
  than new testability — coverage already exists and carries over unchanged (`:571`, `:588`, `:641`,
  `:659`, `:682`). Ranked accordingly.

## c-source-variant-spec

- **Status**: proposed
- **Score**: 18/25 (leverage 3, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated (89 lines across 3 siblings)
- **Modules**: `vow-verify/src/esbmc.rs:573-595` (`emit_verify_c_source`), `:602-625`
  (`emit_reach_c_source`), `:635-661` (`emit_bodyreplace_c_source`); 7 call sites across
  `vow/src/contracts.rs:130,153,171`, `vow/src/verification.rs:116,279`, `esbmc.rs:558`,
  `solver_strategy.rs:760`.
- **Summary**: a `CSourceVariant { Verify, Reach, BodyReplace }` enum plus a pure
  `variant_applies(variant, &Function) -> bool` composing the already-extracted
  `function_has_requires` (`:663`), `function_has_ensures` (`:671`), `returns_scalar` (`:680`) and
  `body_replaceable_result` (`:697`).
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: the three siblings share an identical 9-line body — `collect_modelable_callees` →
  `HashSet` → `emit_c_module_with_callees(.., bool, bool)` → `push_str(&emit_harness(func))` —
  differing only in a guard predicate and **two positional booleans** taking `(false,false)`,
  `(true,false)`, `(false,true)`. The fourth combination `(true,true)` is unrepresentable in intent
  but constructible in code, which is the shallowness worth removing.

## artifact-path-naming

- **Status**: proposed
- **Score**: 16/25 (leverage 2, locality 3, blast radius 1, heat 3)
- **Files**: ~2 estimated (32 lines across 3 policies)
- **Modules**: `vow/src/main.rs:501-505` (build output/object), `:771-783` (decl output, `.vow` →
  `.vow.d`), `vow/src/replay.rs:504-517` (replay harness/bin/object,
  `__vow_replay_{stem}_{vow_id}_{pid}`).
- **Summary**: three total pure functions of `(source, output)` — the replay one additionally taking
  `vow_id` and an **injected** pid rather than calling `std::process::id()` inline, which is its only
  impurity.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: no unit test exercises any of the three. The replay policy is the interesting one: the
  `cleanup` closure at `replay.rs:514-518` must enumerate exactly the paths the naming produced, and a
  divergence leaks temp files into the user's source directory. Held at leverage 2 — three unrelated
  one-call-site policies, no shared rule between them.

## hint-candidate-capping

- **Status**: proposed
- **Score**: 17/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs:2503-2509` (unknown-field hint) vs `vow-types/src/env.rs:66-96`
  (`sorted_capped_keys`); 4 capping sites (`env.rs:632`, `:636`, `:589`, `check.rs:2503`).
- **Summary**: lift `sorted_capped_keys` to take an iterator or `&[String]` instead of a
  `&HashMap<String, V>` and reuse it from all four hint sites, including `all_var_names`' per-scope
  heap. Total and pure: `(&[String], usize, usize) -> Vec<String>`.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: two disagreeing capping policies feed the same "did you mean" surface.
  `sorted_capped_keys` picks the lex-smallest `MAX_HINT_CANDIDATES` names via a bounded max-heap;
  `check.rs:2503-2509` filters by length then `.take(256)` in *declaration* order. For a struct with
  more than 256 fields the field hint is non-deterministic relative to the other four sites, and the
  "available fields:" list is unsorted while "available methods:" (`:2463`) follows the hard-coded
  `builtin_method_names` order. Pin the ordering with a >256-field struct test before unifying.
  Pairs naturally with `recover-unknown-name-prologue`.

## enum-payload-slot-index

- **Status**: proposed
- **Score**: 17/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs:1790-1794`, `:3327-3338`, `:3420-3437`, and implicitly `:416`
  (`method_result_type`'s `unwrap` → `args.first()`).
- **Summary**: `builtin_variant_payload_slots(enum_name, variant) -> Option<&'static [usize]>` —
  `Some([0])` for `Option::Some`/`Result::Ok`, `Some([1])` for `Result::Err`, `Some([])` for
  `Option::None`, `None` for a non-builtin.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: the smallest "one fact, four copies" item in the audit — the `Some/Ok → slot 0,
  Err → slot 1` mapping. `:416` is correct only *because* `Ok` is slot 0, and nothing states that.
  Sites 2 and 3 additionally read `self.env`, so only the mapping itself extracts.

## const-item-literal-spec

- **Status**: proposed
- **Score**: 17/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (77 lines)
- **Modules**: `vow-types/src/check.rs:1268-1344` (Pass 1b2 of `check_module`); 1 call site with three
  duplicated sub-branches.
- **Summary**: `const_item_spec(ty, value) -> ConstSpec` classifying int / bool / negated-int /
  not-a-literal, leaving `env.resolve`, emission, the range check and the
  `const_values`/`const_types` inserts at the call site.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: `ty != Ty::I64 && ty != Ty::I32` appears twice (`:1284`, `:1313`) with an identical
  message, and the "must be a literal" rejection appears twice (`:1328`, `:1336`). The arm interleaves
  four concerns, and the accepted-type set is *narrower than the language* — `const X: u64 = 1;` is
  rejected, a policy statement buried in a `&mut self` pass. **No existing tests**, so the red step is
  larger than for the other `check.rs` candidates; that is what holds it at 17.

## cast-plan

- **Status**: proposed
- **Score**: 17/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (62 lines)
- **Modules**: `vow-ir/src/lower/mod.rs:4044-4105` (`ExprKind::Cast`); 1 call site.
- **Summary**: `cast_plan(literal, src, tgt) -> CastPlan` over
  `{ConstU8, ConstU64, ConstI128, ConstU128, IntCast{from,to}, Passthrough}`. Total and pure.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: clean but thin — one call site and no cross-backend twin, which is what separates it
  from `builtin-method-spec` in the same file. Coverage:
  `integer_literals_lower_at_their_native_ir_width` (`:5847`),
  `explicit_wide_suffixes_lower_through_the_parser_to_native_constants` (`:5922`),
  `vow/tests/wide_literal_ranges.rs`.

## driver-command-epilogue

- **Status**: proposed
- **Score**: 16/25 (leverage 2, locality 3, blast radius 1, heat 3)
- **Files**: ~1 estimated (~40 lines)
- **Modules**: `vow/src/main.rs:726-735` (`run_build_command`) and `:813-823` (`run_verify_command`);
  separately `:539-570` vs `:600-637` inside `run_pipeline_from_frontend`.
- **Summary**: sibling-epilogue dedup — `finish_command(result, session, replay, emit) -> !` over the
  repeated `run_replay_cex` → `session.finish()` → `emit_json()` → `build_exit_code` →
  `process::exit` sequence.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: not a pure-verdict seam — the verdict half (`build_exit_code`, `:61-68`) is already
  extracted and tested (`:1121`). The only divergence between the two epilogues is
  `run_build_command`'s `if !dump_ir` guard on both the replay call and `emit_json`. Lowest-value item
  carded this firing; recorded so the next run does not re-derive it.

## literal-marker-propagation

- **Status**: proposed
- **Score**: 15/25 (leverage 3, locality 4, blast radius 3, heat 5)
- **Files**: ~1 estimated, but a restructure of two mutually-recursive `&mut self` walkers — the
  blast-radius band reflects the restructure, not the file count.
- **Modules**: `vow-types/src/check.rs:1674-1737` (`check_integer_literal_range`) and `:1739-1801`
  (`check_contextual_integer_literal_ranges`).
- **Summary**: a pure `literal_range_targets(expr, target) -> Vec<(&Expr, Ty)>` that *enumerates* the
  (sub-expression, effective-target) pairs, leaving one flat `&mut self` loop to run
  `check_integer_value_range` over them.
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: the two near-sibling recursive walkers **silently diverge**. The first descends blocks
  via `integer_marker_from_block` (`:1721`) and `if` via `integer_marker_from_block(then_branch)`
  (`:1730`); the second uses raw `block.trailing_expr` (`:1746`, `:1756`). The first handles
  `UnaryOp{Neg}` and `BinaryOp` (with the `Shl/Shr → U32` retarget at `:1713-1717`); the second
  handles `EnumConstruct` payloads (`:1784-1800`) and `Match` arms. So a literal inside
  `Some(if c { 1 } else { 2 })` takes a different path than one inside `if c { 1 } else { 2 }`.
  Well-pinned (`:5728`, `:5738`, `:5774`, `:5804`, `:5823`, `:6515`, `:6267`, `:6280`), but the
  refactor restructures two mutually-recursive `&mut self` walkers — high cost, and it needs a
  decision about which traversal is correct, which is a human's call.

## builtin-method-arity-check

- **Status**: proposed
- **Score**: n/a — **not a deepening candidate.** Recorded here so the finding survives; a future
  firing should not try to "deepen" it, and a human should decide whether to file it.
- **Files**: n/a (diagnosis only)
- **Modules**: `vow-types/src/check.rs` (`ExprKind::MethodCall` arm, builtin-method branch)
- **Summary**: the type checker does **not** enforce arity for builtin methods. `hay.contains()` and
  `v.truncate()` type-check with no argument and reach IR lowering with an empty argument list,
  where the lowerer synthesises a placeholder constant (`ConstUnit`, or `ConstI64(0)` for
  `truncate`). Reproduced directly against `./target/debug/vow build --dump-ir --no-verify` on
  2026-09-18:
  - `fn f(hay: String) -> bool { hay.contains() }` → `ConstUnit()` then
    `Call[extern:__vow_string_contains](%0, %1)`
  - `fn f(a: String, b: String) -> bool { a.eq() }` → same shape for `__vow_string_eq`
  - `v.truncate()` → `ConstI64[0]()` then `Call[extern:__vow_vec_truncate](%2, %5)`
- **First seen**: 2026-09-18
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`
- **Reason**: surfaced while closing a coverage gap on PR #1299, which needed to know whether the
  lowerer's missing-argument fallbacks were reachable. They are. The behaviour is pre-existing and
  was preserved exactly by #1299 — pinned there by
  `tabled_builtin_methods_synthesise_their_missing_arguments` — so the tests now *document* it. For
  `v.truncate()` the zero fallback is plausibly intended ("truncate to 0"); for `contains()` and
  `eq()` passing unit into a call that reads a `String` looks like a genuine checker gap. Deciding
  which is which is a human's call, which is why this is recorded rather than filed.

## clif-shim-region-parity

- **Status**: dropped
- **Score**: n/a (not scored — excluded before ranking)
- **Files**: ~20+ estimated
- **Modules**: `vow-clif-shim/src/lib.rs`, `vow-codegen/src/cranelift_backend.rs`
- **Summary**: give the Rust `vow-codegen` backend parity with the shim's pure, tested
  `hidden_region_count` / `hidden_region_for_store_target` seams.
- **First seen**: 2026-08-31
- **Reason**: Too large to automate — blast radius 4, crosses a crate/tier seam and touches codegen
  output. A human should schedule it. Re-checked 2026-09-03: still large. Re-checked 2026-09-11: still
  large. Re-checked 2026-09-16: still large. Re-checked 2026-09-18: still large — but the sibling
  `hidden-region-store-targets` (19/25) is the tractable slice of the same idea and is now carded
  separately, so a human scheduling this can start there.

## esbmc-ce-description-heuristic

- **Status**: proposed
- **Score**: 16/25 (leverage 2, locality 3, blast radius 1, heat 4)
- **Files**: ~1 estimated (6 lines)
- **Modules**: `vow-verify/src/esbmc.rs:170-175` (`parse_esbmc_output` description branch)
- **Summary**: extract the counterexample-description heuristic into a pure
  `counterexample_description(output: &str) -> String`, total over one `&str`, leaving the
  `Counterexample` construction at the call site.
- **First seen**: 2026-08-31
- **Reason**: **Un-dropped 2026-09-18 — the drop reason was false.** It was excluded on 2026-08-31 as
  "inert: the only caller destructures `Failed(_)` and discards the description", and that reason was
  re-affirmed without re-verification on 2026-09-03, 09-11 and 09-16. The description is live on three
  surfaces: `esbmc.rs:907` wraps it into `VerificationResult::Failed`; `vow/src/verification.rs:190`
  clones it into `VerifyOutcome::Failed`, which `vow/src/report.rs` serializes as the public
  `counterexample` JSON field (asserted at `vow/src/main.rs:2893`); and `vow/src/cache.rs:237`
  persists it into the on-disk verify-failure record (round-trip assertion at `cache.rs:476`), making
  it part of a cache file format. Zero direct test coverage: all ten tests calling
  `parse_esbmc_output` assert `vow_id`/`values`/`block_visits`/`arith_overflow`, never `description`.
  Eligible but low-scoring — 6 lines, one production call site.
- **Report**: `.architecture/reviews/2026-09-18-builtin-method-spec.md`

## solver-classify-function

- **Status**: dropped
- **Score**: n/a
- **Files**: n/a
- **Modules**: `vow-verify/src/solver_strategy.rs` (`classify_function`)
- **Summary**: solver-strategy classification for a function.
- **First seen**: 2026-08-31
- **Reason**: Already a pure, unit-tested seam (`test_classify_*`). No shallowness to remove.
  Re-checked 2026-09-03. Re-checked 2026-09-11. Re-checked 2026-09-16. Re-checked 2026-09-18.
