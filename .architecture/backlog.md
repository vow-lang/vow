# Architecture deepening backlog

Persisted candidate memory for the `pm-deepen` routine. Statuses: proposed | in-flight |
landed | dropped | rejected. Never delete rows — `landed`/`dropped`/`rejected` are the memory
that stops the next firing re-deriving them. See `.architecture/reviews/` for the scored reports.

## cast-legality-verdict

- **Status**: in-flight
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs` (`ExprKind::Cast` arm of `check_expr_inner`, ~L2708-2730)
- **Summary**: extract the inline 4-way `as`-cast legality policy into a pure
  `classify_as_cast(src, tgt) -> AsCastVerdict`, mirroring the `method_result_type` seam, leaving
  diagnostics and the `nonneg_casts` mutation at the call site.
- **First seen**: 2026-08-31
- **Report**: `.architecture/reviews/2026-08-31-cast-legality-verdict.md`
- **PR**: (pending)

## integer-literal-range-fit

- **Status**: proposed
- **Score**: 21/25 (leverage 4, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs` (`check_integer_value_range`, ~L1539-1566)
- **Summary**: extract the pure literal-fits-target decision + range-text into
  `literal_out_of_range(value, target) -> Option<String>`, leaving `emit_error_with_hints` at the
  call site; pins the `negative_max`/`i64::MIN` asymmetry.
- **First seen**: 2026-08-31
- **Reason**: runner-up candidate this firing; the natural next pick.

## ce-trace-reconstruction

- **Status**: proposed
- **Score**: 21/25 (leverage 4, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated
- **Modules**: `vow/src/counterexample.rs` (`build_structured_counterexample_with_module`, ~L327-376)
- **Summary**: extract the two pure block-visit → source-trace loops into
  `reconstruct_execution_path` / `reconstruct_branch_decisions`, leaving blame/name/call-site work in
  the builder.
- **First seen**: 2026-08-31

## call-argument-coercion-action

- **Status**: proposed
- **Score**: 21/25 (leverage 4, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated
- **Modules**: `vow-codegen/src/cranelift_backend.rs` (`coerce_call_argument`, ~L398-432)
- **Summary**: extract the pure coercion decision into
  `call_argument_coercion(actual_bits, expected_bits, is_i128, signed) -> CoercionAction`, leaving
  `builder.ins()` emission at the call site. Codegen must stay a byte-identical bootstrap fixed point.
- **First seen**: 2026-08-31

## vec-reserve-next-capacity-seam

- **Status**: proposed
- **Score**: 17/25 (leverage 3, locality 3, blast radius 1, heat 3)
- **Files**: ~1 estimated
- **Modules**: `vow-runtime/src/lib.rs` (`vec_reserve_in_arena_no_null_check`, ~L1473-1519)
- **Summary**: extract the capacity-doubling/overflow policy into a pure
  `next_capacity(old_cap, required) -> Option<usize>`, leaving `oom_trap` at the call site.
- **First seen**: 2026-08-31
- **Reason**: carried forward from the prior firing; module still present.

## arm-pattern-support-classifier

- **Status**: proposed
- **Score**: 19/25 (leverage 3, locality 4, blast radius 1, heat 4)
- **Files**: ~1 estimated
- **Modules**: `vow-types/src/check.rs` (`validate_arm_pattern`, ~L2982-3040)
- **Summary**: extract the pure unsupported-match-arm reason into
  `unsupported_arm_pattern(pat, is_last) -> Option<(&str, &str)>`, leaving the emit/return-bool
  wrapper at the call site.
- **First seen**: 2026-08-31

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
  output. A human should schedule it. Re-checked 2026-08-31: still large.

## esbmc-ce-description-heuristic

- **Status**: dropped
- **Score**: n/a (leverage 1 — fails the deletion test)
- **Files**: ~1 estimated
- **Modules**: `vow-verify/src/esbmc.rs` (`parse_esbmc_output` description branch)
- **Summary**: extract the multi-property counterexample-description heuristic into a pure seam.
- **First seen**: 2026-08-31
- **Reason**: Inert — the only caller destructures `Failed(_)` and discards the description, so
  extraction deepens nothing observable. Latent bugs exist but are unreachable. Do not re-surface.

## solver-classify-function

- **Status**: dropped
- **Score**: n/a
- **Files**: n/a
- **Modules**: `vow-verify/src/solver_strategy.rs` (`classify_function`)
- **Summary**: solver-strategy classification for a function.
- **First seen**: 2026-08-31
- **Reason**: Already a pure, unit-tested seam (`test_classify_*`). No shallowness to remove.
