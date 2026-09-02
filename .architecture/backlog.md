# Architecture deepening backlog

Persisted candidate memory for the `pm-deepen` routine. Statuses: proposed | in-flight |
landed | dropped | rejected. Never delete rows — `landed`/`dropped`/`rejected` are the memory
that stops the next firing re-deriving them. See `.architecture/reviews/` for the scored reports.

## same-operand-type-verdict

- **Status**: proposed
- **Score**: 23/25 (leverage 4, locality 5, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs` (`check_same_numeric` ~L3016, `check_same_integer` ~L3124;
  call sites L1812/L1818/L1873/L1907)
- **Summary**: collapse the two line-for-line-twin operand-type checks into one pure
  `same_operand_verdict(lhs, rhs, class) -> SameOperandVerdict`, mirroring the landed `cast_verdict`
  seam; each method keeps only its distinct `ErrorCode::TypeMismatch` message/hint at the call site.
- **First seen**: 2026-09-03
- **Report**: `.architecture/reviews/2026-09-03-same-operand-type-verdict.md`
- **Reason**: picked this firing (top score); within 1 point of the runner-up
  `integer-literal-range-fit`.

## integer-literal-range-fit

- **Status**: proposed
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs` (`check_integer_value_range`, ~L1582)
- **Summary**: extract the pure literal-fits-target decision + range-text into
  `literal_out_of_range(value, target) -> Option<String>`, leaving `emit_error_with_hints` at the
  call site; pins the `negative_max`/`i64::MIN` asymmetry.
- **First seen**: 2026-08-31
- **Reason**: runner-up candidate; the natural next pick (re-heated 21→22 as `check.rs` moved to
  heat 5).

## call-argument-coercion-action

- **Status**: proposed
- **Score**: 21/25 (leverage 4, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated
- **Modules**: `vow-codegen/src/cranelift_backend.rs` (`coerce_call_argument`, ~L398-432)
- **Summary**: extract the pure coercion decision into
  `call_argument_coercion(actual_bits, expected_bits, is_i128, signed) -> CoercionAction`, leaving
  `builder.ins()` emission at the call site. Codegen must stay a byte-identical bootstrap fixed point.
- **First seen**: 2026-08-31
- **Reason**: byte-identical-bootstrap risk makes a `vow-types` seam the safer unattended pick; carries
  heat 4 (`cranelift_backend.rs` below `check.rs`).

## arm-pattern-support-classifier

- **Status**: proposed
- **Score**: 20/25 (leverage 3, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs` (`validate_arm_pattern`, ~L3045)
- **Summary**: extract the pure unsupported-match-arm reason into
  `unsupported_arm_pattern(pat, is_last) -> Option<(&str, &str)>`, leaving the emit/return-bool
  wrapper at the call site.
- **First seen**: 2026-08-31
- **Reason**: re-heated 19→20 as `check.rs` moved to heat 5.

## ce-trace-reconstruction

- **Status**: proposed
- **Score**: 20/25 (leverage 4, locality 4, blast radius 1, heat 3)
- **Files**: ~1 estimated
- **Modules**: `vow/src/counterexample.rs` (`build_structured_counterexample_with_module`, ~L327-376)
- **Summary**: extract the two pure block-visit → source-trace loops into
  `reconstruct_execution_path` / `reconstruct_branch_decisions`, leaving blame/name/call-site work in
  the builder.
- **First seen**: 2026-08-31
- **Reason**: re-heated 21→20 (`counterexample.rs` only 7 commits/90d → heat 3, was 4).

## narrow-shift-findings

- **Status**: proposed
- **Score**: 20/25 (leverage 3, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs` (narrow `Shl`/`Shr` arm, ~L1875-1906)
- **Summary**: extract the two independent shift-operand checks (count must be `u32`; const count in
  range) into a pure seam returning a **struct of two findings** (not a single-variant enum — both can
  fire on one expression via `Cast`-folded const counts).
- **First seen**: 2026-09-03

## vec-reserve-next-capacity-seam

- **Status**: proposed
- **Score**: 18/25 (leverage 3, locality 3, blast radius 1, heat 4)
- **Files**: ~1 estimated
- **Modules**: `vow-runtime/src/lib.rs` (`vec_reserve_in_arena_no_null_check`, ~L1473-1519)
- **Summary**: extract the capacity-doubling/overflow policy into a pure
  `next_capacity(old_cap, required) -> Option<usize>`, leaving `oom_trap` at the call site.
- **First seen**: 2026-08-31
- **Reason**: carried forward; module still present (re-heated 17→18).

## negation-verdict

- **Status**: proposed
- **Score**: 18/25 (leverage 2, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs` (`UnaryOp::Neg` arm, ~L1952-1974)
- **Summary**: extract the 3-way `{Unsigned, NonNumeric, Ok}` negation decision into a pure verdict,
  leaving the two emits at the call site.
- **First seen**: 2026-09-03

## builtin-receiver-kind

- **Status**: proposed
- **Score**: 17/25 (leverage 2, locality 3, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs` (MethodCall arm, ~L2174-2245)
- **Summary**: extract the pure receiver-kind classification; marginal, because its primary output is a
  display string the landed-seam pattern keeps at the call site.
- **First seen**: 2026-09-03

## unwrap-payload-ty

- **Status**: proposed
- **Score**: 17/25 (leverage 2, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated
- **Modules**: `vow-ir/src/lower/mod.rs` (`lower_unwrap`, ~L4186-4194)
- **Summary**: extract the pure `payload_ty` selection, pre-computing the two `ctx` lookups it reads
  and passing them in, leaving `ctx.emit` at the call site.
- **First seen**: 2026-09-03

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

## clif-shim-region-parity

- **Status**: dropped
- **Score**: n/a (not scored — excluded before ranking)
- **Files**: ~20+ estimated
- **Modules**: `vow-clif-shim/src/lib.rs`, `vow-codegen/src/cranelift_backend.rs`
- **Summary**: give the Rust `vow-codegen` backend parity with the shim's pure, tested
  `hidden_region_count` / `hidden_region_for_store_target` seams.
- **First seen**: 2026-08-31
- **Reason**: Too large to automate — blast radius 4, crosses a crate/tier seam and touches codegen
  output. A human should schedule it. Re-checked 2026-09-03: still large.

## esbmc-ce-description-heuristic

- **Status**: dropped
- **Score**: n/a (leverage 1 — fails the deletion test)
- **Files**: ~1 estimated
- **Modules**: `vow-verify/src/esbmc.rs` (`parse_esbmc_output` description branch)
- **Summary**: extract the multi-property counterexample-description heuristic into a pure seam.
- **First seen**: 2026-08-31
- **Reason**: Inert — the only caller destructures `Failed(_)` and discards the description, so
  extraction deepens nothing observable. Latent bugs exist but are unreachable. Re-checked 2026-09-03:
  caller unchanged. Do not re-surface.

## solver-classify-function

- **Status**: dropped
- **Score**: n/a
- **Files**: n/a
- **Modules**: `vow-verify/src/solver_strategy.rs` (`classify_function`)
- **Summary**: solver-strategy classification for a function.
- **First seen**: 2026-08-31
- **Reason**: Already a pure, unit-tested seam (`test_classify_*`). No shallowness to remove.
  Re-checked 2026-09-03.
