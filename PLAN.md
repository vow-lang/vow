# Plan: #712 — Route exhaustiveness-pass errors through `error_count`

## 1. Problem restated

`Checker::check_expr`'s `ExprKind::Match` arm (`vow-types/src/check.rs:2766`) calls
`crate::exhaustiveness::check_exhaustive(..., self.emitter)` directly, so a `NonExhaustiveMatch`
diagnostic reaches `self.emitter` (and therefore `diagnostics[]` in the JSON output) but never
increments `self.error_count`. The build gate is `Checker::has_errors()` → `error_count > 0`, so
`vow build` on a non-exhaustive `match` over a user-defined enum reports the error yet exits 0
(`Unverified`) and ships a binary — the same fail-open class fixed for the effect/linear passes in
#705/#706, deferred here because exhaustiveness is a separate pass with its own test surface.
Investigation for this plan found the self-hosted compiler's gap is actually larger than "needs the
same wrapper": `compiler/checker.vow`'s `EXPR_MATCH` handling (~line 3343) has **no exhaustiveness
check at all** — it validates arm-pattern shapes and binds variables but never computes missing
variants or emits `EC_NON_EXHAUSTIVE_MATCH` (that error code already exists in `compiler/diag.vow`,
unused). This matches the `docs/audits/2026-07/report.md` finding ("self-hosted: Absent — accepts a
missing arm silently, 0 errors reported") and its recommendation to port exhaustiveness into the
self-hosted checker, not just re-wrap Rust's call. Because `scripts/full_test.sh`'s Section 7 runs
every `tests/error/*.vow` fixture through **both** compilers and diffs their error codes/hints/spans
(`scripts/parity.py::compare_error`), adding the one regression fixture this issue asks for is only
green once the self-hosted compiler can detect the same non-exhaustive match — so the self-hosted
port is required to close the issue, not optional scope creep.

## 2. Files to touch

Rust:
- `vow-types/src/check.rs` — wrap the `check_exhaustive` call (currently ~line 2766-2773) in the
  existing `ErrorCounter` (defined at `check.rs:1024`, already used at `check.rs:1567` and
  `check.rs:1677` for `where`-refinement purity and the effect/linear passes). Fold `counter.errors`
  into `self.error_count`, following the three-line edit shape at `check.rs:1567-1572`
  (`check_param_refinements`), not the `check_fn` block-with-return-value shape at `check.rs:1677-1686`
  — the smaller diff avoids re-indenting unrelated lines, which the codecov/patch gate would otherwise
  count as new-and-uncovered.
- `vow/tests/exhaustiveness_gating.rs` — new file, mirrors `vow/tests/effect_gating.rs` and
  `vow/tests/pattern_gating.rs` (same `build_no_verify`/`error_codes` helper pattern).

Self-hosted (`compiler/`):
- `compiler/checker.vow` — add an exhaustiveness check (new functions placed near
  `is_enum_match_tid`/`validate_arm_pattern`, ~line 2133-2142) and call it from the `EXPR_MATCH()`
  handling block (~line 3343-3409), in the same position as Rust: right after the
  `all_arms_supported && !scrutinee_supported` diagnostic (~line 3363-3365), before the arm-binding
  loop. No changes needed to `compiler/diag.vow` (`EC_NON_EXHAUSTIVE_MATCH()` and its `ec_name()` case
  already exist, unused) or `compiler/env.vow` (`env_lookup_enum`, `edef_variant_count`,
  `edef_vname_lids` already exist and are already used by `unhandled_variant_owns_linear_payload`).

Shared:
- `tests/error/match_non_exhaustive_enum.vow` — new fixture, picked up automatically by
  `scripts/full_test.sh`'s `run_promoted_error_tests` (Section 7, both compilers via
  `compare_error`) and by `tests/run_tests.sh`'s Phase 5 (`// TEST: error-code` directive check).

docs/spec:
- **No change required.** `docs/spec/errors.md:326-334` already documents `NonExhaustiveMatch` as a
  Type Checker-phase error; this PR fixes enforcement reliability, not semantics, the error code, or
  its meaning. (See §6 for a documentation-accuracy issue this investigation surfaced, which is a
  separate follow-up, not part of this change.)

## 3. TDD slices

**Prerequisite:** this is a fresh workspace with no `target/release/vow` or `build/vowc` yet. Before
slice 1, run `cargo build --release -p vow` (needed for `vow/tests/*_gating.rs`, which shell out to
`CARGO_BIN_EXE_vow`) and `scripts/bootstrap.sh` (needed to exercise `build/vowc` for slice 3/4's
manual checks and for the eventual `full_test.sh` run). Cap build parallelism per the run's memory
budget (e.g. `cargo build --release -p vow -j4`).

1. **Rust, red.** Add `vow/tests/exhaustiveness_gating.rs` with:
   - `non_exhaustive_enum_match_fails_build`: a 3-variant enum (e.g. `Color { Red, Green, Blue }`), a
     `match` covering only 2 variants, no wildcard. Assert exit code 1, `status == "CompileFailed"`,
     and `error_codes` contains `"NonExhaustiveMatch"`.
   - `wildcard_arm_suppresses_non_exhaustive_check`: same enum, 2 explicit arms plus a trailing `_`,
     must compile (pins the existing, intentional catch-all-bypasses-the-check behavior so this fix
     doesn't accidentally tighten it later).
   - `exhaustive_enum_match_compiles`: all 3 variants covered explicitly, no wildcard, must compile
     (no-false-positive guard).
   Run `cargo test -p vow --test exhaustiveness_gating`. `non_exhaustive_enum_match_fails_build` must
   fail (red: today it observes exit 0 because `error_count` never saw the diagnostic); the other two
   should already pass, since they don't touch the broken path.

2. **Rust, green.** Apply the `check.rs` edit described in §2. Re-run
   `cargo test -p vow --test exhaustiveness_gating` — all three green. Then run `cargo test -p vow-types`
   (exhaustiveness.rs's own unit tests are untouched and must stay green — this PR does not change
   `exhaustiveness.rs`'s logic, only its caller) and `cargo clippy --all -- -D warnings` (no
   `--all-targets`, matching the CI gate).

3. **Self-hosted, red.** Add `tests/error/match_non_exhaustive_enum.vow`:
   ```vow
   // TEST: error-code NonExhaustiveMatch
   module MatchNonExhaustiveEnum

   enum Color {
       Red,
       Green,
       Blue,
   }

   fn describe(c: Color) -> i64 {
       match c {
           Color::Red => 1,
           Color::Green => 2,
       }
   }

   fn main() -> i32 {
       0
   }
   ```
   This is the only diagnostic-triggering construct in the file (both arms type-check against
   `describe`'s `i64` return type), so a green run after slice 4 attributes success to the
   exhaustiveness check alone. Confirm red by hand first:
   `build/vowc build --no-verify tests/error/match_non_exhaustive_enum.vow -o /tmp/mne` must exit 0
   today (self-hosted reports nothing), while
   `target/release/vow build --no-verify tests/error/match_non_exhaustive_enum.vow -o /tmp/mne_r`
   exits 1 once slice 2 has landed — a live parity mismatch, which is the fixture's red state.

4. **Self-hosted, green.** In `compiler/checker.vow`, add (near `is_enum_match_tid`):
   - `check_exhaustive(e, m, scrutinee_tid, arms_lid, n_arms, span)`:
     - Scan all `n_arms` patterns; if any is `PAT_WILD()` or (`PAT_IDENT()` and not `mut`), return
       immediately — mirrors `exhaustiveness.rs`'s top-level `arms.iter().any(Wildcard | Ident)` bail,
       and is the only early-exit needed: `validate_arm_pattern` already rejects a *non-final*
       wildcard/ident as its own hard error, so by the time this function runs (gated on
       `all_arms_supported`), any such arm present is guaranteed last.
     - Skip (return) if `!is_enum_match_tid(e.ts, scrutinee_tid)` — this function is only reachable
       from the `all_arms_supported && scrutinee_supported` call site anyway, so this is defense in
       depth matching the Rust function's own `match scrutinee_ty { ... _ => {} }` fallthrough, not a
       new condition.
     - Resolve `enum_name_from_tid(e.ts, scrutinee_tid)`, then `env_lookup_enum(e, enum_name)`. If
       `-1` (unregistered — this is the path builtin `Option`/`Result` take, see §6), return without
       emitting. **Guard every `edef_*` index with `enum_idx != -1` before use**, matching the style
       `unhandled_variant_owns_linear_payload` already uses for the same tables.
     - Walk the `n_arms` patterns again; for each `PAT_ENUM()` pattern, take the **last** element of
       its path list (the variant name) via `arena_str` and add it to a `Vec<String>` of covered
       names (reuse `handled_variant_contains` to dedupe). **Do not gate this on
       `enum_pattern_matches_tid`** — Rust's `collect_enum_patterns` only checks the variant-name
       segment against the scrutinee enum's own variant set and ignores whether the pattern's
       qualifier actually matches the scrutinee enum. Gating on the qualifier would make self-hosted
       report `NonExhaustiveMatch` on some wrong-qualifier-without-wildcard shape where Rust reports
       only `TypeMismatch`, breaking parity. (The one existing wrong-qualifier fixture,
       `tests/error/linear_match_wrong_enum_qualifier.vow`, already has a trailing `_` arm, so it hits
       the first bail-out either way and isn't a live risk — but the general rule still matters for
       future fixtures.)
     - Walk the enum's declared variants via `e.edef_vname_lids[enum_idx]` /
       `e.edef_variant_count[enum_idx]` **in declaration order** (index `0..nv`, same order
       `env_define_enum` — called from the item-registration pass in file order — populated them, and
       the same order Rust's `EnumInfo.variants` is populated from the AST), collecting any name not
       in the covered set into a `missing: Vec<String>`.
     - If `missing.len() > 0`: build `msg = "non-exhaustive match: missing patterns: " +
       string_join(missing, String::from(", "))` (the `string_join` builtin, already available to
       self-hosted Vow code, gives the same `", "`-joined text as Rust's `.join(", ")`). Emit one
       diagnostic with **exactly two hints, in this order** (hint text and order are parity-compared
       verbatim since `NonExhaustiveMatch` is not in `scripts/parity.py`'s `HINT_UNCOMPARED_CODES`):
       `"add arms for: " + joined` and the literal `"or add a wildcard '_' arm to match remaining
       patterns"`. `env_emit_error_code_hint` only carries one hint, so build the diagnostic directly
       (`env_error_diag` + two `diag_add_hint` calls + `diag_ctx_emit`, all already used elsewhere in
       `env.vow`/`checker.vow`) rather than adding a new two-hint helper for a single call site.
   - Call site: inside `EXPR_MATCH()` handling, immediately after the existing
     `if all_arms_supported && !scrutinee_supported { ... }` block and before
     `let mut result_tid: i64 = CTY_NEVER();`, add
     `if all_arms_supported && scrutinee_supported { check_exhaustive(e, m, scrutinee_tid, arms_lid, n_arms, expr_span(a, eid)); }`
     — `expr_span(a, eid)` is the whole `match` expression's span, matching Rust's `expr.span` (not
     the scrutinee's span).
   - **No `ErrorCounter`-equivalent wrapper needed.** `env_emit_error_code_hint`/manual
     `diag_ctx_emit` already increments `e.dctx.error_count` for every `Severity::Error` diagnostic
     (`compiler/diag.vow:258-263`) — self-hosted has no separate raw-emitter-bypass architecture the
     way Rust's free-function effect/linear passes did, so plugging into the standard emit path is
     sufficient for the build gate.
   - Rebuild and verify: `scripts/bootstrap.sh --skip-cargo` (rebuilds `build/vowc` from the edited
     `compiler/*.vow`, running the self-hosted compiler's own ESBMC verification pass over the new
     functions in the process — see §4), then re-run the manual check from slice 3 and confirm
     `build/vowc` now also exits 1 with `NonExhaustiveMatch`.

5. **Both, full gate + corpus confirmation.** Run, as separate (non-`&&`-chained) commands:
   - `cargo test --all`
   - `cargo clippy --all -- -D warnings`
   - `cargo fmt --all -- --check`
   - `scripts/bootstrap.sh` (full, with verification, not `--skip-cargo`)
   - `scripts/full_test.sh`
   This investigation manually audited every existing fixture that declares a user enum and matches
   over it — `tests/run/{btreemap_enum_value,issue843_u64_match_expr_phi,match_enum_multi_payload,
   match_fieldless,match_mut,match_option_payload,match_supported_catchalls,match_enum_payload,
   vec_pop_enum}.vow` and `tests/verify-skip/wide_enum_payload_skipped.vow` — and confirmed every
   match is already exhaustive (arm count equals variant count, or ends in `_`/a catch-all). No
   `enum` declarations exist anywhere in `compiler/*.vow`, `compiler/tests/*.vow`,
   `benchmarks/*/{reference,skeleton}.vow`, or `examples/*.vow`. Zero fixtures are expected to change
   status; `full_test.sh` is the authority that confirms this, not a substitute for having checked.

## 4. Verification surface

This is a frontend/type-checker diagnostic-wiring change: no new `requires`/`ensures` contracts, no
IR, codegen, or C-emitter changes, and the new fixture is rejected before verification would run (a
`CompileFailed` build never reaches ESBMC for *that fixture*). However, the self-hosted compiler's
own source is itself verified during `scripts/bootstrap.sh` — the new `check_exhaustive` /
`check_enum_exhaustive`-equivalent functions in `compiler/checker.vow` go through the same ESBMC pass
as every other self-hosted checker function. This is low risk, not zero: the new code follows the
exact loop/accessor shape already pervasive in `compiler/checker.vow` (bounded `while i < n` loops
over AST-list lengths, guarded `edef_*` array indexing identical to
`unhandled_variant_owns_linear_payload`'s), so it is not introducing a new verification pattern — but
slice 4/5's full (non-`--skip-cargo`) `scripts/bootstrap.sh` run is the actual confirmation, not an
assumption.

`tests/error/match_non_exhaustive_enum.vow` is the only new fixture needed. No `tests/verify-fail/`
fixture is warranted — this is a compile-time frontend rejection, not an ESBMC counterexample.
`scripts/parity.py::compare_error` (exercised by `full_test.sh` Section 7) is the de facto
cross-compiler verification surface for this change: it requires both compilers to agree on exit
code, `status`, the `error_code` multiset, and — since `NonExhaustiveMatch` is not listed in
`HINT_UNCOMPARED_CODES` — the exact hint text and order too. Getting the hint wording and
missing-variant ordering byte-identical (§3 slice 4) is therefore a correctness requirement for CI,
not a nicety.

## 5. Risk areas

- **Bootstrap / binary fixed point.** Low risk: `compiler/*.vow` defines no Vow-level `enum` types of
  its own (AST/IR tags are plain `i64`-returning constant functions like `EXPR_MATCH()`, `PAT_WILD()`
  — confirmed by grep), so turning on enum-exhaustiveness checking cannot affect the self-hosted
  compiler's ability to compile itself. The new functions use only already-verified-pattern loops
  (see §4); still confirm with a full (non-skip-cargo) `scripts/bootstrap.sh`, not just
  `--skip-cargo`, at least once before considering this done.
- **Declaration-order / hint-text parity.** The self-hosted "missing variants" list and the
  `", "`-joined, two-hint message must match Rust's wording and ordering exactly (§3 slice 4,
  §4) — a plausible-looking but differently-ordered or differently-worded self-hosted message passes
  a casual read but fails `scripts/parity.py`'s hint comparison in CI.
- **Qualifier-blind variant coverage.** Mirror Rust's `collect_enum_patterns` exactly (coverage keyed
  only on the pattern's variant-name segment, not on whether its qualifier matches the scrutinee
  enum) — see the explicit warning in §3 slice 4. Getting this "more correct" than Rust is a parity
  regression, not an improvement, until Rust is fixed too (out of scope here).
- **`cargo clippy --all -D warnings`.** The Rust edit reuses the existing `ErrorCounter` type and
  idiom verbatim (no new types, no new lint surface). Use the minimal three-line diff shape (§2) over
  the block-with-return-value shape to avoid incidental re-indentation that the codecov/patch gate
  would count as new-and-uncovered lines.
- **`parse → print → parse` idempotency.** Untouched — no AST, token, or printer changes in this PR.
- **Existing-corpus regressions.** Addressed by the explicit manual audit in §3 slice 5; re-confirmed
  by the full `scripts/full_test.sh` run rather than taken on faith.

## 6. Out of scope

- **Builtin `Option`/`Result` exhaustiveness.** Neither compiler registers `Option`/`Result` as
  lookup-able enums (`vow-types/src/env.rs`'s `enum_defs` is only populated by `define_enum`, called
  only for user `enum` items at `check.rs:1137`/`check.rs:1221`; `env_lookup_enum` in self-hosted is
  symmetric). `check_exhaustive`/`check_enum_exhaustive` therefore silently skip any match over
  `Option<T>`/`Result<T,E>` today, before and after this fix — `docs/spec/errors.md:326-334`'s own
  `NonExhaustiveMatch` example uses `Option<i64>` with a missing `None` arm, which is misleading
  since that exact shape is **not** currently caught. Extending exhaustiveness to builtin enums (a
  pseudo-`EnumInfo` registration for `Option`/`Result`, in both compilers, for parity) is a
  meaningfully broader change than this issue's "wire the existing pass through `error_count`" scope
  and belongs in its own issue. **Action for the implementation stage:** file a follow-up issue via
  `gh issue create` documenting this gap and the `errors.md` example inaccuracy, and record the
  scoping decision with `gh issue comment 712` (per the issue's own "best-effort decision, document
  it" operating contract) — do not fix it in this PR.
- **`bool` scrutinee exhaustiveness.** `check_bool_exhaustive`/the self-hosted equivalent would only
  ever run on a `bool` match, but `validate_arm_pattern` (both compilers) unconditionally rejects
  `PatKind::Lit`/`PAT_LIT()` arms as `UnsupportedPattern`, and `is_enum_match_scrutinee`/
  `is_enum_match_tid` don't accept `bool` as a scrutinee type in the first place — so a `bool` match
  never reaches `check_exhaustive` through the real pipeline (confirmed by
  `vow/tests/pattern_gating.rs::literal_patterns_fail_closed`'s
  `match_bool_literal_pattern.vow` case, which expects `UnsupportedPattern`, not
  `NonExhaustiveMatch`). `check_bool_exhaustive` is exercised only by `exhaustiveness.rs`'s own unit
  tests. This plan does not port a bool-exhaustiveness check into self-hosted (nothing to mirror — it
  would be dead code there too) and does not change this pre-existing, unrelated dead-code situation
  in Rust.
- **`vow-types/src/exhaustiveness.rs`'s internal logic.** Its enum/bool detection, missing-variant
  computation, and message/hint wording are already correct and already unit-tested — only its call
  site's error-counting wiring is broken. No edits planned there.
- **Consolidating the three `ErrorCounter` call sites** (`where`-refinement purity, effect/linear
  passes, and now exhaustiveness) into a shared helper. Three small, independent three-line wraps is
  consistent with "surgical changes, many small PRs" and avoids bundling an unrelated refactor into
  this bug fix.
- **`docs/equivalence/ledger.json`'s `checker` pair `content_hash`.** This is maintained by the
  `equivalence-review` skill's own recurring process when it re-reviews the pair, not by feature
  PRs; this change will make that stored hash stale, which is expected and will be picked up on the
  next scheduled equivalence review, not something to hand-edit here.
- **A dedicated low-level `compiler/tests/test_checker_*.vow` unit test** for the self-hosted
  exhaustiveness check. The shared `tests/error/match_non_exhaustive_enum.vow` fixture already
  exercises it end-to-end under both the `full_test.sh` cross-compiler parity gate and
  `tests/run_tests.sh`'s directive check; a hand-built-AST unit test in the
  `compiler/tests/test_checker.vow` style would duplicate that coverage at much higher authoring cost
  (that file builds `Module`/arena values by hand, not from parsed source).
