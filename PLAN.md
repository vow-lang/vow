# Plan: tag if/else result and loop header/exit Phis with aggregate metadata (#403)

## Goal
`result.<field>` (and `h.<field>` on a loop-carried struct local) must lower to a real `FieldGet` when the value
comes out of an `if/else` tail or a `while`/`loop`/`for` header/exit Phi. Today those Phis carry no
`inst_struct_type` tag, so `ExprKind::FieldAccess` takes the "untagged instruction" ICE path and emits a sentinel
`ConstI64(0)` instead of a field read. Fix in BOTH compilers in one PR; no language/spec change.

## Root cause (verified by reading)
- Rust `vow-ir/src/lower/mod.rs`: tags live in `LowerCtx::inst_struct_type: HashMap<InstId,String>` (989), keyed by
  InstId. `ExprKind::FieldAccess` (~2990-3070) reads it; empty tag -> `ctx.warn("FieldGet on untagged instruction ...")`
  + sentinel `ConstI64(0)`.
- `match` lowering already calls `merge_compatible_aggregate_metadata(ctx, &arm_result_values, phi_id)` (mod.rs:3725,
  fn at 554; added in f59d2f88 / #1351; fixture `tests/run/match_aggregate_phi_metadata.vow`). **`ExprKind::If`
  (2040-2240) never does.** If-result Phis: 2177 (else arm terminated -> only then live), 2183 (then arm terminated
  -> only else live), 2232 (both live). (2160 is the if-*mutation* Phi loop - NOT the result Phi.)
  While/loop header+exit Phis: 2418, 2448 (while); 2603, 2636, 2788, 2809 (loop/for).
- `lower_ensures(ctx, vow_block, trailing)` (`vow-ir/src/lower/vow.rs:248`, called at mod.rs:5354) binds `result` to
  the tail value -> an untagged Phi for if/else tails. `expand_ptr_bindings` (vow.rs:129-143) silently `continue`s
  on untagged values, so violation `values` also drop `result.<field>`.
- Self-hosted twin: `compiler/lower.vow` `lctx_merge_compatible_aggregate_metadata` (746) is only called for match
  (4861). If-result Phis: 3312 / 3319 (one arm terminated), 3351 (both live). While header/exit: 3398 / 3419;
  loop/for: 3556 / 3580 / 3736 / 3759. Untagged warning at 4017.

## Assumptions
- The issue's three warnings (`size`, `size`, `data`) match the two `ensures` clauses
  (`result.size == h.size - 1`, `result.size == result.data.len()`), so `%44` is very likely the if-result Phi.
  The segfault is hypothesised to be callee-side: `result.data` lowers to sentinel `ConstI64(0)` and `.len()`
  derefs null inside the callee's `ensures` (debug) - NOT caller corruption. Slice 0 must confirm via IR dump in
  debug and release; if a different untagged source appears, the same helper covers it.
- Tags are static per variable, so a loop header/exit Phi takes metadata from the pre-loop value
  (`merge_compatible_aggregate_metadata(ctx, &[pre_val], phi)`); if-result Phis merge the live (non-terminated) arms.
  Conflicting/missing source tags stay untagged (conservative; existing helper semantics) - no new inference.
- Scope split (minimal slice closing the issue): if-result Phi + while/loop/for header/exit Phis. If/match
  *mutation* Phis and the loop break-value Phi are follow-ups, not in this PR.

## Files to touch
| File | Role | Lines |
|------|------|-------|
| `vow-ir/src/lower/mod.rs` | tag If result Phis; tag while/loop/for header+exit Phis; unit tests in `mod tests` | 554, 2177, 2183, 2232, 2418, 2448, 2603, 2636, 2788, 2809 |
| `compiler/lower.vow` | same sites via `lctx_merge_compatible_aggregate_metadata` | 3312, 3319, 3351, 3398, 3419, 3556, 3580, 3736, 3759 |
| `compiler/tests/test_lower_aggregate_metadata.vow` | add if/else + loop-phi tag cases (use `compiler/tests/builders.vow` to build AST); split into a new `test_lower_phi_aggregate_metadata.vow` [new] if it grows | whole |
| `tests/run/if_else_struct_result_field.vow` [new] | end-to-end run fixture (stdout) | - |
| `tests/debug/if_else_struct_ensures_true.vow` [new] | debug-mode, TRUE ensures `result.size == h.size - 1`, `result.size == result.data.len()`, `// TEST: exit 0`; red before (134 or 139), green after | - |
| `tests/verify/if_else_struct_result_contract.vow` [new] | issue-shaped contract, parity-checked automatically | - |
| `vow-ir/src/lower/vow.rs` | consumers (`expand_ptr_bindings`, `lower_ensures`); expected unchanged | 129-175, 248 |

No `docs/spec/*` change (no syntax/semantic/CLI change).

## TDD slices (each = one red/green commit; Rust and Vow halves land in the same slice)
0. **Reproduce.** Create `tests/debug/if_else_struct_ensures_true.vow`: `struct Heap {data: Vec<i64>, size: i64}`,
   `pop(h) -> Heap` with the issue's requires/ensures (true contract, `h.size > 1` path exercised) and an if/else
   tail; `// TEST: exit 0`. Run with `./target/release/vow` and `build/vowc` (`VOW_CACHE_DIR=$(mktemp -d)`) in debug
   and release; dump IR; confirm which Phi is untagged and where the crash is. This is the red end-to-end test.
1. **If-result Phi.** Rust unit test (hand-built AST pattern near mod.rs:10770; `lower_function` returns warnings): struct-returning fn,
   tail `if c {S{..}} else {S{..}}`, `ensures: result.f == ..`; assert `warnings.is_empty()` and a `FieldGet` with the
   right `FieldIndex`. Extra cases: `if … else if … else` chain (inner Phi feeds outer merge); one arm ending in
   `return` (one-sided Phi path 2177/2183). Green: call `merge_compatible_aggregate_metadata(ctx, &[live arm vals],
   phi_id)` after each result Phi. Mirror in `lower.vow` (3312/3319/3351) with a self-hosted unit test following the
   #1351 two-compiler test shape. Add `tests/run/if_else_struct_result_field.vow`.
2. **Loop-carried locals (issue's related observation).** Test: `while ... invariant: h.size >= 0 { h = S{..}; }` with
   `h` a mutable struct local, plus `loop` / `for`; assert no warnings and a real `FieldGet` in the invariant. Green:
   tag header + exit Phis from the pre-loop value (2418, 2448, 2603, 2636, 2788, 2809); mirror
   3398/3419/3556/3580/3736/3759.
3. **Verifier fixture.** `tests/verify/if_else_struct_result_contract.vow` with the true contract; expect PROVEN, or
   if ESBMC cannot prove (e.g. Vec `len` unmodelled) mark the function unverifiable - never weaken the contract.
4. **Readers audit.** Before landing, grep every reader of `inst_struct_type` / `lctx_get_tag` in both compilers and
   confirm none misbehaves for newly tagged Phis (see Risks). No code change unless a reader breaks.

## Verification surface
- ESBMC: `ensures`/`invariant` on struct-returning if/else and loop-carried structs now lower to real `FieldGet`
  instead of `ConstI64(0)`; C model sees genuine field reads. Slice 3 fixture must prove
  `result.size == h.size - 1` and `result.size == result.data.len()`.
- C parity: lowering feeds `vow-verify/src/c_emitter.rs` and `compiler/c_emitter.vow`; the new `tests/verify/`
  fixture is run by `scripts/full_test.sh` Section 2c (`scripts/parity.py c`). Both lowerers must tag identically.
- Fixtures grow: `tests/run/`, `tests/debug/`, `tests/verify/` (paths above), plus `compiler/tests`.

## Risks
- **Other readers of the tags change behaviour**, not just FieldGet: method-receiver dispatch (mod.rs ~321), FieldSet
  (2322), match (3740), 4382, 1968/1983, `expand_ptr_bindings` (vow.rs:143), and Phi ownership/dealloc logic.
  Grep all readers in both compilers; run `tests/run/dealloc_phi_struct.vow` and
  `tests/run/region_phi_widen_root_parity.vow` explicitly.
- **Violation JSON changes**: `values` gains `result.<field>` entries now that untagged bindings are not skipped.
  Grep `tests/debug/` and `tests/verify-fail*/` for fixtures pinning `TEST: stderr`/`values` on such functions and
  update only where the new output is the correct one.
- **Binary fixed point**: compiler sources with if/else struct tails or loop-carried struct locals now lower
  differently. Run `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA; record the SHA. No HashMap
  iteration added (keyed lookups only).
- Conservative merge: arms with different/missing tags stay untagged (warning persists; not worse than today).
- Previously-passing programs may now see genuine ensures violations (formerly evaluated against sentinel 0) -
  intentional; check `examples/` and `tests/verify*/` expectations.
- Dual-compiler rule: every Rust change has its `lower.vow` twin in the same PR. Gates as separate commands:
  `cargo test -p vow-ir`, `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all --check`,
  `build/vowc test compiler/tests/test_lower_aggregate_metadata.vow`, `scripts/bootstrap.sh --skip-cargo --no-cache`,
  `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh` (slow: bootstrap ~5 min, full_test ~40 min - foreground with
  explicit timeouts). Pre-existing failures to verify on clean main first: sandbox SKIP-panics,
  `u64_marker_propagation`, `contracts_tmp_cleanup`, `concrete-block-region-parity`.
- Parse/print idempotency: untouched (no syntax change). PR title: lowercase conventional, e.g.
  `fix(lower): tag if/else and loop phis with aggregate metadata`.
- The issue's workaround lives in `lib/heap` (PR #402), absent from this checkout; nothing to revert here.

## Out of scope (file as follow-ups)
- If/match *mutation* Phis (mod.rs 2160 loop, 3672; lower.vow 3298, 4785) and the loop break-value Phi (2865 / 3656).
- Promoting `LoweringWarning` ICE sentinels (FieldGet/FieldSet untagged) to hard errors, or a type-checker-driven
  tag fallback.
- Any dedupe/refactor of the Phi sites (the helper is already a single call), formatting, spec/doc changes.
