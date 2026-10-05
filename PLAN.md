# Plan: #1495 — extend exhaustiveness checking to builtin Option/Result enums

## 1. Problem restated

Neither compiler's exhaustiveness pass fires on a `match` over `Option<T>`/`Result<T, E>`. Both
`check_exhaustive`/`check_enum_exhaustive` (Rust: `vow-types/src/exhaustiveness.rs`) and
`check_exhaustive` (self-hosted: `compiler/checker.vow`) resolve the scrutinee's enum name and then
look it up — `env.lookup_enum(name)` / `env_lookup_enum(e, name)` — in a table (`enum_defs` /
`e.edef_*`) that is populated only by `define_enum`/`env_define_enum`, which only runs for
user-written `enum` items. `Option`/`Result` are structurally `Ty::Enum("Option"|"Result")` wrapped in
`Ty::Applied` (confirmed in both compilers: `enum_match_name`/`is_enum_match_scrutinee` in
`check.rs:3512-3529` and `is_enum_match_tid`/`enum_name_from_tid` in `checker.vow:2167-2329` already
accept them as valid match scrutinees), but since the lookup table has no entry for either name, both
`check_enum_exhaustive` (Rust, `exhaustiveness.rs:84-87`) and `check_exhaustive` (self-hosted,
`checker.vow:2344-2347`) hit their `None`/`-1` early-return and silently skip the check. A `match`
covering only `Option::Some` compiles cleanly in both compilers today, contradicting
`docs/spec/errors.md`'s own `NonExhaustiveMatch` example, which uses exactly that shape and claims it
is rejected.

This is a narrower, purely-additive follow-up to #712 (landed as #1499), which fixed the same
`NonExhaustiveMatch` diagnostic's fail-open-at-the-call-site bug for user enums and explicitly scoped
builtin `Option`/`Result` out (see `git show ced563fc:PLAN.md` §6 "Out of scope", still retrievable
from history even though `PLAN.md` itself was removed before merge). The pass's internal logic
(missing-variant computation, message/hint wording, `error_count` wiring) is already correct and
already fires correctly for user enums; this issue only needs the two `check_enum_exhaustive`-family
functions to recognize `Option`/`Result` as enums with known, fixed variant sets.

## 2. Files to touch

Rust:
- `vow-types/src/exhaustiveness.rs` — `check_enum_exhaustive` (or a new helper it delegates to) must
  resolve variant names for `"Option"` (`["Some", "None"]`) and `"Result"` (`["Ok", "Err"]`) directly,
  independent of `env.lookup_enum`, before falling through to the existing `env.lookup_enum(name)`
  path for user enums. **Do not** add `Option`/`Result` to `TypeEnv::enum_defs` via `define_enum` —
  `EnumInfo`/`VariantInfo`/`VariantKind` carry concrete, monomorphic payload types
  (`VariantKind::Tuple(Vec<Ty>)`), and `Option`/`Result` are generic; registering a pseudo-`EnumInfo`
  would require inventing a type-parameter placeholder `EnumInfo` doesn't support, and would change
  the return value of `env.lookup_enum("Option"|"Result")` from `None` to `Some`, which several other
  call sites (`check.rs:3595-3623`'s `variant_tys` `.or_else`, `check.rs:3690-3723`'s
  `unhandled_variant_owns_linear_payload`, `linear.rs:115`) already use as a deliberate "is this a
  *user* enum" signal alongside an explicit `name == "Option" || name == "Result"` check. A direct
  name-based special case in exhaustiveness checking only (mirroring
  `unhandled_variant_owns_linear_payload`'s own `match enum_name { "Option" => ..., "Result" => ...,
  _ => ... }` shape at `check.rs:3705-3723`, which already solves the identical "builtin enum, no
  `EnumInfo`, need a fixed variant-name list" problem for linear-payload checking) is the smaller,
  lower-risk change and does not touch any of those other call sites.
- No change needed to `vow-types/src/env.rs`, `vow-types/src/check.rs`, or `vow-types/src/linear.rs`.

Self-hosted (`compiler/`):
- `compiler/checker.vow` — `check_exhaustive` (`checker.vow:2331-2389`) must take the same
  name-based branch for `"Option"`/`"Result"` before its `env_lookup_enum(e, enum_name) == -1` early
  return (`checker.vow:2344-2347`), producing the same two variant-name lists. Mirror the existing
  self-hosted precedent at `checker.vow:2781-2790` (the twin of
  `unhandled_variant_owns_linear_payload`), which already special-cases `"Option"`/`"Result"` by name
  without touching `env_define_enum`/`env_lookup_enum`.
- No change needed to `compiler/env.vow` (`env_define_enum`/`env_lookup_enum` stay exactly as they
  are) or `compiler/diag.vow` (`EC_NON_EXHAUSTIVE_MATCH` already exists and is already wired through
  `error_count` by #1499).

Shared:
- `tests/error/match_non_exhaustive_option.vow` — new fixture, non-exhaustive `Option<T>` match
  (missing `None`), `// TEST: error-code NonExhaustiveMatch`. Picked up automatically by both
  `scripts/full_test.sh`'s Section 7 (`run_promoted_error_tests`, cross-compiler via
  `scripts/parity.py::compare_error`) and `tests/run_tests.sh`'s Phase 5.
- `tests/error/match_non_exhaustive_result.vow` — same shape for `Result<T, E>` (missing `Err`).
- `vow/tests/exhaustiveness_gating.rs` — extend with `non_exhaustive_option_match_fails_build` and
  `non_exhaustive_result_match_fails_build` (plus matching `*_wildcard_suppresses_check` /
  `*_exhaustive_match_compiles` no-false-positive guards), following the existing
  `non_exhaustive_enum_match_fails_build` pattern in that same file.
- `compiler/tests/test_checker_exhaustive.vow` — extend with
  `check_non_exhaustive_option_match_reports_missing_none` and the `Result` equivalent, following the
  existing `check_non_exhaustive_match_reports_missing_variant` pattern (parse real source via
  `check_module`, not hand-built AST — consistent with how this file already tests the user-enum
  case).

docs/spec:
- **`docs/spec/errors.md`** — **no wording change needed.** Its existing `NonExhaustiveMatch` example
  (`errors.md:336-342`, `Option<i64>` match missing `None`) already describes the *post-fix* behavior
  correctly; this PR makes the claim true rather than needing to pick a different example. Confirm
  this by hand once the fix lands: `build/vowc build --no-verify` and
  `target/release/vow build --no-verify` on that exact snippet must both now exit 1 with
  `NonExhaustiveMatch`.
- **`docs/spec/grammar.md`** — **no change needed.** The Pattern Matching section
  (`grammar.md:980-992`) already states "The scrutinee must have an enum type, including an applied
  built-in enum such as `Option<T>` or `Result<T, E>`... Patterns must be exhaustive" — this is
  already accurate as a semantic claim; today's bug is an enforcement gap, not a documented-vs-actual
  mismatch requiring a doc edit.

## 3. TDD slices

**Prerequisite:** confirm `target/release/vow` and `build/vowc` are buildable in this workspace
(`cargo build --release -p vow -j<N>` sized to the run's memory budget; `scripts/bootstrap.sh
--skip-cargo` if `build/vowc` is stale or absent).

1. **Rust, red.** Add the two new fixtures (`tests/error/match_non_exhaustive_option.vow`,
   `tests/error/match_non_exhaustive_result.vow`) and the two new `vow/tests/exhaustiveness_gating.rs`
   `*_fails_build` tests (`non_exhaustive_option_match_fails_build`,
   `non_exhaustive_result_match_fails_build`), modeled directly on
   `non_exhaustive_enum_match_fails_build`. Run `cargo test -p vow --test exhaustiveness_gating`;
   both new tests must fail red (today: exit 0, no `NonExhaustiveMatch`).

2. **Rust, green.** In `vow-types/src/exhaustiveness.rs`, change `check_enum_exhaustive` (or extract a
   small `builtin_enum_variant_names(name: &str) -> Option<Vec<&'static str>>` helper it checks first)
   so that for `name == "Option"` the variant-name list is `["Some", "None"]` and for `name ==
   "Result"` it is `["Ok", "Err"]`, falling back to `env.lookup_enum(name)` otherwise (unchanged
   behavior for user enums). Re-run `cargo test -p vow --test exhaustiveness_gating` (both new tests
   green, the three existing ones unaffected) and `cargo test -p vow-types` (exercise
   `exhaustiveness.rs`'s own unit tests — extend them with an `Option`/`Result`-scrutinee case
   mirroring `enum_missing_variant`, using `Ty::Applied(Box::new(Ty::Enum("Option".into())),
   vec![Ty::I64])` as the scrutinee type, consistent with how `check.rs`'s own tests already construct
   this type at e.g. `check.rs:6034`). Then `cargo clippy --all --all-targets -- -D warnings`.

3. **Self-hosted, red.** Confirm by hand that `build/vowc build --no-verify
   tests/error/match_non_exhaustive_option.vow -o /tmp/mno` still exits 0 (self-hosted reports
   nothing) while the Rust compiler (now fixed by slice 2) exits 1 — a live parity mismatch, which is
   this fixture's red state for the self-hosted side. Add the two new
   `compiler/tests/test_checker_exhaustive.vow` cases in the same red state (assert `error_count ==
   1` and `d.code == EC_NON_EXHAUSTIVE_MATCH()`; both currently fail because `error_count` stays 0).

4. **Self-hosted, green.** In `compiler/checker.vow`'s `check_exhaustive` (`checker.vow:2331-2389`),
   before the `env_lookup_enum(e, enum_name) == -1` early return, add the same name-based branch:
   build the `covered`/`missing` lists against a hardcoded `["Some", "None"]` for `enum_name ==
   String::from("Option")` or `["Ok", "Err"]` for `enum_name == String::from("Result")`, reusing the
   existing `handled_variant_contains` dedup helper and the existing
   `env_error_diag`/`diag_add_hint`×2/`diag_ctx_emit` emission shape already in this function
   (`checker.vow:2377-2388`) — do not introduce a second emission path. Rebuild:
   `scripts/bootstrap.sh --skip-cargo`, then re-run the slice-3 manual check (`build/vowc` now also
   exits 1) and `build/vowc test compiler/tests/test_checker_exhaustive.vow`.

5. **Both, corpus-wide confirmation.** This is the one risk area #712 didn't have to face (its
   user-enum fix touched a corpus with zero existing `match`es over user enums at all): roughly 45
   existing fixtures under `tests/run/`, `tests/error/`, `tests/verify-fail/`, `tests/verify-skip/`,
   and `examples/` already `match` over `Option`/`Result`. A manual spot-check during planning (
   `match_option_payload.vow`, `option_literal_context_coercion.vow`,
   `result_literal_context_coercion.vow`, `indirect_option_i32_flow.vow`,
   `narrow_option_annotated_local_overflow.vow`, every `tests/error/*option*`/`*result*` fixture, and
   every `linear_*option*`/`linear_*result*` fixture) found every match arm set already exhaustive
   (`Some`+`None` pair, `Ok`+`Err` pair, or a trailing wildcard/identifier catch-all) — but this was a
   sample, not the full ~45-file set, so treat it as a hypothesis, not a guarantee. Run the real gate
   as the actual confirmation, as separate (non-`&&`-chained) commands:
   - `cargo test --all`
   - `cargo clippy --all --all-targets -- -D warnings`
   - `cargo fmt --all -- --check`
   - `scripts/bootstrap.sh` (full, with verification — not `--skip-cargo`, at least once)
   - `scripts/full_test.sh`
   Any fixture that regresses is a real non-exhaustive `Option`/`Result` match that was previously
   silently accepted; fix it by adding the missing arm (not by weakening the new check), since that is
   exactly the class of bug this issue exists to catch.

## 4. Verification surface

No new `requires`/`ensures` contracts, no IR/codegen/C-emitter changes. This is a frontend
diagnostic-recognition change exactly like #712: a non-exhaustive `match` is rejected before lowering,
so it never reaches ESBMC for that fixture. The self-hosted compiler's own source (the edited
`check_exhaustive` in `compiler/checker.vow`) is itself verified during `scripts/bootstrap.sh`'s
self-hosting pass, same as every other self-hosted checker function — the new code is a straight-line
extension of an existing bounded-loop/array-index shape already proven verifiable
(`unhandled_variant_owns_linear_payload`'s twin at `checker.vow:2781-2790` uses the identical
`enum_name == String::from(...)` dispatch), so no new verification pattern is introduced. No
`tests/verify-fail/` fixture is needed — this is a compile-time rejection, not an ESBMC
counterexample. `scripts/parity.py::compare_error` (run by `full_test.sh` Section 7) requires
byte-identical hint text and order between compilers for `NonExhaustiveMatch` (it is not in
`scripts/parity.py`'s `HINT_UNCOMPARED_CODES` set, confirmed at `scripts/parity.py:73-88`) — reusing
the exact existing hint-building code path in both functions (rather than writing new hint text) is
what keeps this correct by construction instead of by careful wording-matching.

## 5. Risk areas

- **Existing-corpus regressions.** The real, non-trivial risk for this issue (see §3 slice 5) — the
  ~45-fixture `Option`/`Result`-match corpus was previously accepted unconditionally. Addressed by the
  full `scripts/full_test.sh` run, not assumed from the spot-check sample.
- **Hint-text / variant-order parity.** Both compilers must report missing variants in the same order
  (`Some`-then-`None` for `Option`, `Ok`-then-`Err` for `Result`) and identical hint wording, since
  `NonExhaustiveMatch` is hint-compared by `scripts/parity.py`. Hardcode both lists in a fixed,
  matching order in both compilers' new branches.
- **Qualifier-blind variant coverage.** Per #712's own documented risk (still applicable): coverage
  must be keyed only on the pattern's variant-name segment (`Some`/`None`/`Ok`/`Err`), not on whether
  the pattern's qualifier matches the scrutinee's enum name — do not add a stricter check here than
  the existing `collect_enum_patterns`/`check_exhaustive` loops already perform for user enums.
- **Binary fixed point / bootstrap.** Low risk: the self-hosted change is a pure `if`/`else`-style
  name dispatch over already-verified array/loop shapes, adds no new `enum` type, no new struct
  layout, and no change to `BTreeMap`/`HashMap` usage or `vow-clif-shim` stack-slot layout. Confirmed
  by a full (non-`--skip-cargo`) `scripts/bootstrap.sh` run, not assumed.
- **`parse → print → parse` idempotency.** Untouched — no AST, token, or printer changes.
- **`cargo clippy --all --all-targets -- -D warnings`.** The Rust change is additive logic inside an
  existing function (or one small new helper); no new public API surface, no new lint-prone patterns.

## 6. Out of scope

- **`bool` scrutinee exhaustiveness in self-hosted.** Still N/A per #712's own finding: `bool` never
  reaches `check_exhaustive` through the real pipeline in either compiler (rejected earlier as
  `UnsupportedPattern`/not an enum-match scrutinee). Not touched here.
- **Registering `Option`/`Result` as full `EnumInfo` entries in `enum_defs`/`e.edef_*`.** Deliberately
  rejected in §2 above — would require generic-type-parameter support `EnumInfo` doesn't have, and
  would change the `lookup_enum(name).is_some()` signal several unrelated call sites rely on to mean
  "this is a user enum." A direct name-based special case in the exhaustiveness functions only is the
  smaller, lower-blast-radius change, consistent with the existing
  `unhandled_variant_owns_linear_payload` precedent in both compilers.
- **Generalizing the `"Option"`/`"Result"` special-casing** that is already scattered across
  `check.rs`/`checker.vow` (construction, method resolution, linear-payload checks, pattern binding)
  into one shared helper or a unified builtin-enum registry. That is a cross-cutting refactor
  independent of this bug fix; bundling it here would violate "many small changes beat one large
  change." Worth a future issue if the duplication becomes a maintenance problem, not this one.
- **Any change to `docs/spec/errors.md` or `docs/spec/grammar.md` wording.** Per §2, both already
  describe the correct (post-fix) semantics; this PR is enforcement-only.
- **`docs/equivalence/ledger.json`'s `checker` pair `content_hash`.** As with #712, this is maintained
  by the `equivalence-review` skill's own recurring process, not by feature PRs.
