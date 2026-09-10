# Plan: issue #1266 — self-hosted checker never validates `EXPR_IF` condition is `bool`

## 1. Problem restated

In `compiler/checker.vow`'s `check_expr_inner`, the `EXPR_IF()` case type-checks the
condition expression but throws away the result (`let _cond_tid: i64 = check_expr(e, m,
cond_eid);`), so `if 5 { ... }` type-checks under the self-hosted checker even though the
Rust checker (`vow-types/src/check.rs:2507-2518`) explicitly rejects any condition whose
type is not `Ty::Bool` or `Ty::Never` with `TypeMismatch: "if condition must be `bool`,
found `{cond_ty}`"`. This is a real Rust-vs-self-hosted divergence — same root-cause shape
as #644 (fixed for `EXPR_ASSIGN` in PR #1264 / commit `ed4c9b9e`): a value Rust validates
explicitly, self-hosted computes via `check_expr` and silently drops via a `let _foo`
binding. The fix mirrors #1264's pattern exactly: keep the computed type, guard it with
`is_opaque` (already covers `CTY_NEVER()`/`CTY_UNKNOWN()`), and emit `EC_TYPE_MISMATCH` via
`env_emit_error` when it isn't `bool` and isn't opaque.

## 2. Files to touch

- **`compiler/checker.vow`** — `EXPR_IF()` case in `check_expr_inner` (currently
  `compiler/checker.vow:2805-2812`, `let _cond_tid: i64 = check_expr(e, m, cond_eid);`).
  Rename `_cond_tid` → `cond_tid`, add the bool/opaque guard, emit `EC_TYPE_MISMATCH`.
- **`compiler/tests/test_checker_if_cond_bool.vow`** *(new)* — self-hosted unit test
  following the `compiler/tests/test_checker_float_literal.vow` pattern (`checked_error_count`
  helper over `check_module`). CI-gated via `vow test compiler/` in
  `.github/workflows/bootstrap.yml` and `scripts/full_test.sh`.
- **`tests/error/if_cond_not_bool.vow`** *(new)* — dual-compiler parity fixture, auto-picked
  up by `scripts/full_test.sh`'s Section 7 (`run_promoted_error_tests`, globs
  `tests/error/*.vow`) with no harness code changes needed. Naming follows the existing
  `contract_requires_not_bool.vow` / `contract_ensures_not_bool.vow` sibling convention.
- **`docs/spec/grammar.md`** — the `### If / Else` section (currently line 544) documents
  that both branches must agree in type but never states the condition must be `bool`. This
  is a genuine, pre-existing spec gap (Rust already enforces it; self-hosted didn't). Add one
  sentence. `scripts/check_help_coverage.py` only scans markdown *tables* in grammar.md
  (`extract_table_rows`), not prose, so this addition does not require regenerating
  `--help`/the embedded skill and cannot trip `help/coverage-*` in Section 8.
- **No Rust crate change.** `vow-types/src/check.rs:2507-2518` is already correct — this is
  a self-hosted-only parity fix, not a language-semantics change, so the "modify both
  compilers" rule doesn't apply here (there is nothing to change on the Rust side).

## 3. TDD slices

1. **Red — self-hosted unit test.** Add `compiler/tests/test_checker_if_cond_bool.vow` with
   three cases, each returning a distinct nonzero code on failure (mirroring
   `test_checker_float_literal.vow`'s style), driven through a `checked_error_count(source)`
   helper built from `parse_module_into` + `env_new` + `check_module`:
   - `check_non_bool_condition_rejected()`: source with `if 1 { 0 } else { 1 }` inside
     `fn main() -> i32` — asserts `checked_error_count(source) != 0`. **Fails today** (the bug
     — this is the red step).
   - `check_bool_literal_condition_accepted()`: `if true { 0 } else { 1 }` — asserts
     `checked_error_count(source) == 0` (already passes; locks in the non-regression case).
   - `check_bool_variable_condition_accepted()`: `let flag: bool = true; if flag { 0 } else
     { 1 }` — asserts `checked_error_count(source) == 0` (already passes).
   `main() -> i32 [io]` chains all three, returning the first nonzero code, matching the
   existing file's structure. At this point only case 1 fails.

2. **Green — production fix.** In `compiler/checker.vow`, in the `EXPR_IF()` branch of
   `check_expr_inner` (~line 2805), change:
   ```vow
   let _cond_tid: i64 = check_expr(e, m, cond_eid);
   ```
   to:
   ```vow
   let cond_tid: i64 = check_expr(e, m, cond_eid);
   if cond_tid != CTY_BOOL() && !is_opaque(cond_tid) {
       let msg: String = String::from("if condition must be `bool`, found `");
       msg.push_str(ty_value_display_name(e.ts, cond_tid));
       msg.push_str(String::from("`"));
       env_emit_error(e, msg, expr_span(a, cond_eid));
   }
   ```
   `env_emit_error` defaults to `EC_TYPE_MISMATCH` (see `compiler/env.vow:796-798`), matching
   the `EXPR_ASSIGN` fix in `ed4c9b9e`. `is_opaque` already covers `CTY_NEVER()` and
   `CTY_UNKNOWN()` (`compiler/checker.vow:370-374`), so it's a superset of the Rust guard
   (`cond_ty != Ty::Bool && cond_ty != Ty::Never`) — accepting `CTY_UNKNOWN()` too is correct
   because that type only arises from an already-reported upstream error, and this mirrors
   the exact same pattern already used one case below for vow-clause bool checks
   (`check_vow_clause`, `compiler/checker.vow:806`: `if clause_tid != CTY_BOOL() &&
   !is_opaque(clause_tid)`). Span is `expr_span(a, cond_eid)` — the condition sub-expression,
   not the whole if-expr — matching Rust's `condition.span` and the span-anchoring fix in
   `0762f002`. Re-run slice 1's unit test: all three cases now pass.

3. **Green — dual-compiler parity fixture.** Add `tests/error/if_cond_not_bool.vow`:
   ```vow
   // TEST: stderr "TypeMismatch"
   module IfCondNotBool

   fn main() -> i32 {
     if 5 { 0 } else { 1 }
   }
   ```
   Before slice 2, self-hosted's `build --no-verify` on this fixture exits 0 (bug) while Rust
   exits 1 with `TypeMismatch` — `scripts/full_test.sh` Section 7's `compare_error` /
   `scripts/parity.py error` mode would flag the error-code multiset mismatch. After slice 2,
   both compilers emit `TypeMismatch` and exit 1; parity holds. No changes to
   `scripts/full_test.sh` are needed — Section 7 globs `tests/error/*.vow` already.

4. **Docs.** Add one sentence to `docs/spec/grammar.md`'s `### If / Else` section stating the
   condition must have type `bool` (coercible via `Never`), right after the existing "both
   branches must have the same type" sentence.

5. **Full-corpus regression pass** (not a new test, a verification step): run the existing
   suites listed in "Verification surface" below to confirm no fixture under `tests/run/`,
   `examples/`, or `compiler/*.vow` itself relies on a non-bool if-condition that the fix
   would now reject.

## 4. Verification surface

No contracts, codegen, or C-model changes — this is a pure type-checker diagnostic addition,
so there is no new ESBMC proof obligation and no new `tests/run/` or `examples/` fixture is
required beyond the `tests/error/` parity fixture in slice 3. Run, in this order, after slices
1–4 land:

- `build/vowc test compiler/` (or `target/release/vow test compiler/` if `build/vowc` is
  stale) — exercises the new unit test plus the full existing `compiler/tests/*.vow` suite,
  catching any other self-hosted-source `if` with an accidentally non-bool condition.
- `scripts/bootstrap.sh --skip-cargo` — rebuilds `build/vowc` from the patched
  `compiler/checker.vow` and re-verifies the self-hosted compiler's own contracts; also the
  cheapest way to confirm the self-hosted compiler's ~13 modules contain no `if` with a
  non-bool condition that the new check would newly reject (compilation would fail here if
  so).
- `scripts/full_test.sh` — full suite including the new Section 7 parity fixture and the
  bootstrap triple test (Section covering `run_bootstrap_triple`), to confirm the binary
  fixed point (`compiler_b`/`compiler_c` sha256 match) is unaffected.
- `cargo test --all` — sanity check that the untouched Rust side still passes (no Rust files
  are modified by this plan, so this is a no-op regression guard, not expected to catch
  anything new).

## 5. Risk areas

- **Binary fixed point.** The fix only adds a conditional diagnostic emission on an
  already-error path; it does not change codegen, IR shape, `BTreeMap` iteration order, or
  `vow-clif-shim` stack-slot layout. Risk to `compiler_b`/`compiler_c` sha256 parity is
  negligible, but the bootstrap triple test in slice/step above is the concrete check, not an
  assumption.
- **Self-hosted compiler compiling itself.** The one real risk: if any of the 13
  `compiler/*.vow` modules currently has an `if` with a non-bool, non-never condition, the
  self-hosted compiler would now fail to compile itself during
  `scripts/bootstrap.sh`. This is checked directly (not assumed) by running bootstrap after
  the change — see Verification surface. No manual audit is substituted for this.
- **`parse → print → parse` idempotency.** Not affected — no AST or printer changes.
- **`cargo clippy --all -- -D warnings`.** Not affected — no Rust source changes in this plan.
- **False positives from `is_opaque`.** `is_opaque` treats `CTY_UNKNOWN()` as opaque
  (suppresses the new diagnostic), which is correct and intentional: a condition that already
  failed to type-check upstream must not also trigger a redundant/misleading "if condition
  must be bool" error. This matches the existing `check_vow_clause` guard one case below.
- **Message-text drift, not error-code drift.** `scripts/parity.py`'s `error` mode compares
  error-code multisets (`_error_codes`), not exact message text, so the self-hosted message
  need not byte-match Rust's `"if condition must be `bool`, found `{cond_ty}`"` for the
  parity test to pass — slice 2 mirrors it anyway for consistency, using the same
  `ty_value_display_name` helper the `EXPR_ASSIGN` fix used for its `{cond_ty}`-equivalent
  rendering.

## 6. Out of scope

- **`EXPR_WHILE`'s identical bug.** `compiler/checker.vow:2841` (`let _cond_tid: i64 =
  check_expr(e, m, cond_eid);` in the `EXPR_WHILE()` case) has the exact same discard-the-
  condition-type bug, and the Rust checker likely validates `while` conditions the same way
  `if` conditions are validated. This is the same root-cause shape but a distinct AST node
  and a distinct issue from #1266's title/scope ("EXPR_IF condition"). Per "many small
  changes beat one large change," this plan deliberately does not bundle a fix for it. The
  implementation stage should file a follow-up issue for `EXPR_WHILE` (and audit whether any
  other `_cond_tid`/`_foo`-discard sites in `checker.vow` have the same shape) rather than
  scope-creep this PR.
- **Refactoring the `EXPR_IF()`/`EXPR_WHILE()` duplication.** Both branches share the same
  `let cond_eid = expr_a(a, eid); ... check_expr(...)` shape; extracting a shared
  `check_condition_is_bool` helper is tempting but is a refactor riding on a bug fix — not
  bundled here even when the `EXPR_WHILE` follow-up lands.
- **Extending `docs/spec/errors.md`.** No new error code is introduced (`EC_TYPE_MISMATCH`
  already exists and is already documented), so no `errors.md` change is needed.
- **Regenerating `--help`/embedded skill.** Confirmed unnecessary (see §2) since the
  `grammar.md` addition is prose, not a table row `check_help_coverage.py` scans.
