# Plan: split TIMEOUT vs UNKNOWN into sibling arms in verify_collect_and_report (#388)

## Goal
Behaviour-preserving readability refactor of `verify_collect_and_report` (`compiler/main.vow`): the merged
`if st == VERIFY_TIMEOUT() || st == VERIFY_UNKNOWN()` block (inner `bv_is_unknown` flag, three derived
label/message locals, a compound retry guard) becomes top-level sibling `if … else if … else if …` arms in the
shape of the `run_test` drains. No observable change: stderr lines, soft-fail `(status, message, function)`
triple, return value, and number/order of ESBMC invocations stay identical. Single production file.

## Assumptions
- Issue line numbers (`main.vow:422–477`, `run_test` ~914/~949) are stale. Current: merged arm is
  `compiler/main.vow:1647-1704` inside `verify_collect_and_report` (1564-1716); the `run_test` shape is at
  `compiler/main.vow:2462-2474` and `2509-2522`.
- The issue's "inner `if !bv_is_unknown && …` guard" is actually `(!bv_is_unknown || bv_memory_limit) &&
  auto_enc && bv_solver != "bitwuzla"` (`main.vow:1663`). A plain (non-memlimit) UNKNOWN skips the IR retry and
  lands in the shared epilogue (1701-1703); a memory-limit UNKNOWN retries like TIMEOUT. The refactor keeps this.
- No Rust twin: `BV inconclusive` / `BV and IR` occur only in `compiler/main.vow`. The Rust driver already splits
  the same policy into pure seams (`retry_plan`, `combine_retry`, `vow-verify/src/solver_strategy.rs:263,307`).
  So this is a self-hosted-driver-only change with nothing to mirror, needs no `docs/spec` change (no
  syntax/semantics/CLI change), and does not touch the C emitters (verifier C parity unaffected).
- The resource-limited predicate stays local to `main.vow` (private helper). The identical inline copy in
  `compiler/verifier.vow:1826-1827` (`verify_function_contracts_only`) and the two `run_test` copies
  (`main.vow:2464`, `2511`) are left alone — surgical-change rule; listed as follow-up.
- No unit test for the private helper: `main.vow` is a driver (`main`), not importable by `compiler/tests/*`.
  The end-to-end characterization test below is the safety net.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `compiler/main.vow` | the refactor | `verify_collect_and_report` 1564-1716; merged arm 1647-1704; `record_soft_fail` 1373; reference shape 2462-2474 |
| `tests/verify-bv-fallback/tests.sh` [new] | end-to-end characterization with a scripted fake `esbmc` | pattern: `tests/esbmc-path-cache/tests.sh` (fake `esbmc` on `PATH`, `VOWC_BIN`) |
| `scripts/full_test.sh` | wire the new script | next to `verifier/esbmc-path-cache`, ~850-854 |

Read-only references: `compiler/verifier.vow` `start_esbmc` 626-650, `append_solver_args` 531-538,
`verify_function_ir_fallback` 1735-1810, `parse_verify_status_tainted` 692-720, `parse_unknown_reason` 756.

## Steps

### 1. Add private helpers above `verify_collect_and_report` [compiler/main.vow]
- `fn bv_resource_limited(st: i64, raw_output: String) -> bool` =
  `st == VERIFY_TIMEOUT() || (st == VERIFY_UNKNOWN() && output_has_memory_limit(raw_output))`
  (Rust `retry_plan`'s `resource_limited`).
- `fn report_bv_timeout(f: IrFunction, soft_meta: Vec<String>) -> i64 [io]`: `diag_eprintln_best_effort("  <f.name>: TIMEOUT")`;
  `record_soft_fail(soft_meta, "timeout", "", f.name)`; returns 0. Empty message matches Rust `None`.
- `fn report_bv_unknown(f: IrFunction, raw_output: String, soft_meta: Vec<String>) -> i64 [io]`: prints `: UNKNOWN`;
  `record_soft_fail(soft_meta, "unknown", parse_unknown_reason(raw_output), f.name)`; returns 0. For a memlimit
  UNKNOWN `parse_unknown_reason` already yields `memory_limit_reason()` (`verifier.vow:756-759`), so the message
  is unchanged.
- `fn report_ir_retry_outcome(esbmc_bin, f, ir_mod, max_k_step, use_cache, timeout_secs, vec_max, string_max, hashmap_max, btreemap_max, st: i64, bv_raw: String, soft_meta) -> i64 [io, write]`:
  body = current `1664-1699` verbatim (print `BV inconclusive, retrying with --z3 --ir...`, call
  `verify_function_ir_fallback`, then the `st2` ladder: PROVEN_IR → 1; FAILED → demote; TIMEOUT →
  `TIMEOUT (BV and IR)` with empty message; UNKNOWN → `UNKNOWN (BV and IR)` with `parse_unknown_reason(vr2.raw_output)`;
  NOT_FOUND; generic ERROR). The FAILED-demotion branch reports the *original* BV verdict:
  `if st == VERIFY_TIMEOUT() { return report_bv_timeout(f, soft_meta); } return report_bv_unknown(f, bv_raw, soft_meta);`
  replacing the `bv_label` / `bv_label_upper` / `bv_msg_initial` locals, which disappear.

### 2. Flatten the merged arm [compiler/main.vow:1647-1704]
Replace the whole `if st == TIMEOUT || st == UNKNOWN { … }` block with top-level sibling arms:
```
let try_ir_retry: bool = bv_resource_limited(st, vr.raw_output)
    && is_auto_encoding(encoding)
    && resolve_auto_bv_solver(solver) != String::from("bitwuzla");
if try_ir_retry {
    return report_ir_retry_outcome(...);
} else if st == VERIFY_TIMEOUT() {
    return report_bv_timeout(f, soft_meta);
} else if st == VERIFY_UNKNOWN() {
    return report_bv_unknown(f, vr.raw_output, soft_meta);
}
```
- Carry the comment over: BV timeout / memlimit may retry with IR (Phase D); other UNKNOWN outcomes are explicit
  inconclusive ESBMC results and an IR proof must not hide them; mirrors `run_with_fallback` in vow-verify.
- Everything after (`VERIFY_NOT_FOUND`, generic ERROR epilogue 1705-1715) and the `has_arith` block (1593-1628)
  are untouched. Evaluation is pure (no I/O reordered); the `has_arith` block above is not affected.

## TDD slices
1. **Characterization, green on the unmodified compiler first.** Create `tests/verify-bv-fallback/tests.sh`
   and wire it into `scripts/full_test.sh`. Commit `test(verify): pin bv to ir fallback outcomes`.
   - Fixture: one function with a **single** `ensures` clause (avoids the multi-property path) plus `main`.
   - Run `"$VOWC_BIN" verify --no-cache --verify-jobs 1 <fixture>` with the fake `esbmc` first on `PATH`.
   - Fake `esbmc`: find the `.c` file argument (as Section 2b's fake does: arg is a file with extension `c`);
     append one line per invocation that **has a `.c` argument** to a log (ignores any version/probe call) and
     record whether argv contains `--ir`. Argv facts (read from `start_esbmc`/`append_solver_args`): the BV
     attempt under auto is `<tmp>.c --no-bounds-check --no-pointer-check --incremental-bmc --max-k-step N --64
     --timeout 30s --memlimit …` with no `--ir`/`--z3`; the IR retry adds `--z3 --ir`. Output text per scenario:
     `Timed out` → TIMEOUT, `Out of memory: memory limit exceeded` → memlimit UNKNOWN,
     `VERIFICATION UNKNOWN` → plain UNKNOWN, `VERIFICATION SUCCESSFUL`, `VERIFICATION FAILED`.
   - Scenarios (assert stderr line, JSON soft-fail status/message, invocation count):
     a. BV `Timed out`, IR `VERIFICATION SUCCESSFUL` → `PROVEN (IR)`, 2 invocations.
     b. BV `Timed out`, IR `VERIFICATION FAILED` → `: TIMEOUT`, soft-fail `timeout`, empty message, 2 invocations.
     c. BV `Timed out`, IR `Timed out` → `TIMEOUT (BV and IR)`, soft-fail `timeout`.
     d. BV memlimit, IR success → `PROVEN (IR)`.
     d2. BV memlimit, IR `VERIFICATION FAILED` → `: UNKNOWN`, soft-fail `unknown`, message `memory limit exceeded`
         (pins the demotion label through the helper).
     e. BV `VERIFICATION UNKNOWN`, IR would succeed → `: UNKNOWN`, soft-fail `unknown`, **1 invocation** (#387 invariant).
     f. BV `Timed out` with `--encoding ir`, and separately `--solver bitwuzla` → `: TIMEOUT`, 1 invocation.
   - Before relying on a scenario, run it once against the unmodified `build/vowc` and read the actual output;
     assert exactly what the old code emits.
2. **Refactor under the net.** Steps 1-2, then re-run the slice-1 script unchanged: identical results. Commit
   `refactor(verify): split bv timeout and unknown into sibling arms` (lower-case, ≤100 chars incl. ` (#N)`).
3. **Gate.** `scripts/bootstrap.sh --skip-cargo --no-cache` (record head SHA and seed pin in the PR checklist per
   CLAUDE.md), then `scripts/full_test.sh` verifier sections.

## Verification surface
- No contract, IR, codegen, or C-model change; helpers carry no `vow` blocks. `verify_collect_and_report` is
  `[io, write]` and not part of any proof obligation change.
- No new `tests/run/` or `examples/` fixtures; the new `tests/verify-bv-fallback/` directory is the only fixture.
- `cargo test` / clippy / `docs/spec` / help regeneration: not affected (no Rust or spec change).

## Risks
- **Fixed point:** driver source changes, so stage 2 differs from the previous `build/vowc` by design; the check is
  stage2 == stage3. Helper ordering in `main.vow` is deterministic (no map iteration).
- **Plain-UNKNOWN regression (#387):** the only way to break behaviour is `try_ir_retry` becoming true for a
  non-memlimit UNKNOWN. Scenario e pins it with the invocation count.
- **Demotion label:** FAILED-after-retry must surface the original BV verdict (TIMEOUT vs memlimit-UNKNOWN);
  scenarios b and d2 pin both.
- **Language gotchas:** `soft_meta: Vec<String>` is mutated through a shared handle — pass it, never copy. Chained
  field access on struct values needs annotated `let` bindings if a helper reads fields off `vr`/`f`.
- **Stale compile cache:** rebuilt compiler at the same source rev can serve stale objects; validate with
  `VOW_CACHE_DIR=$(mktemp -d)`. Wait for long gates with a done-marker file, never `pgrep -f`.
- **Fake-esbmc robustness:** discriminate IR by argv `--ir` and count by `.c` argument, not raw exec count; if the
  fake needs to know the function name it must not rely on multi-property output.

## Out of scope
- Replacing the inline predicate in `verify_function_contracts_only` (`compiler/verifier.vow:1826`) or in the two
  `run_test` drains (`main.vow:2464`, `2511`) with a shared helper — follow-up.
- Splitting the `has_arith` block (1593-1628) or reducing the 19-parameter signature of `verify_collect_and_report`.
- Any Rust-driver change, `docs/spec/*` edit, `--help`/skill regeneration, or change to the retry policy.
