# Architecture deepening backlog

Persisted candidate memory for the `pm-deepen` routine. Statuses: proposed | in-flight |
landed | dropped | rejected. Never delete rows — `landed`/`dropped`/`rejected` are the memory
that stops the next firing re-deriving them. See `.architecture/reviews/` for the scored reports.

## builtin-method-result-type-seam

- **Status**: in-flight
- **Score**: 22/25 (leverage 4, locality 4, blast radius 1, heat 5)
- **Files**: ~1 estimated (actual: 1)
- **Modules**: `vow-types/src/check.rs` (`ExprKind::MethodCall` arm of `check_expr`, ~L1941-2122)
- **Summary**: extract the inline builtin-method result-type and known-methods resolution into pure
  free functions mirroring the existing `method_argument_expectations` seam, leaving diagnostics in
  `check_expr`.
- **First seen**: 2026-08-31
- **PR**: #1153
- **Report**: `.architecture/reviews/2026-08-31-builtin-method-result-type-seam.md`

## vec-reserve-next-capacity-seam

- **Status**: proposed
- **Score**: 17/25 (leverage 3, locality 3, blast radius 1, heat 3)
- **Files**: ~1 estimated
- **Modules**: `vow-runtime/src/lib.rs` (`vec_reserve_in_arena_no_null_check`, ~L1498-1519)
- **Summary**: extract the capacity-doubling/overflow policy into a pure
  `next_capacity(old_cap, required) -> Option<usize>`, leaving `oom_trap` at the call site.
- **First seen**: 2026-08-31
- **Reason**: runner-up candidate this firing; the natural next pick.

## clif-shim-region-parity

- **Status**: dropped
- **Score**: n/a (not scored — excluded before ranking)
- **Files**: ~20+ estimated
- **Modules**: `vow-clif-shim/src/lib.rs`, `vow-codegen/src/cranelift_backend.rs`
- **Summary**: give the Rust `vow-codegen` backend parity with the shim's pure, tested
  `hidden_region_count` / `hidden_region_for_store_target` seams.
- **First seen**: 2026-08-31
- **Reason**: Too large to automate — blast radius 4, crosses a crate/tier seam and touches codegen
  output. A human should schedule it.

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
