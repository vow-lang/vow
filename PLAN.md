# Plan: #1151 — unlabelled ESBMC properties reported as fabricated contract violations on vow_id 0

## 1. Problem restated

When ESBMC reports a violated property that carries no `vow:N` label — either
its own built-in **division/remainder-by-zero** check (fires on the plain `/`
and `%` operators, which the C emitter lowers to bare C `/`/`%`) or Vow's own
`"integer shift count"` assert — the verifier pipeline cannot resolve a real
vow id, and today it silently fabricates one: `Counterexample.vow_id` is
`None`, but `vow/src/counterexample.rs` maps `None` to the numeric literal
`0` (`ce.vow_id.unwrap_or(0)`), which collides with the function's *real*
`vow:0` if it has one. Downstream, `vow/src/verify_outcome.rs`'s
`blame_to_error_code` has a documented fallback that maps any non-caller/
non-callee blame (including this "none" case) to `VowRequiresViolated` — a
*Caller*-contract error code — even though the JSON's own `blame` field
correctly says `"none"`. The result, confirmed by direct local reproduction
against the current tree, is:

```json
{"error_code":"VowRequiresViolated","message":"contract violation in `r`: internal verifier assertion failed", ...}
```
```json
{"function":"r","values":{"b":"0"},"violation":"internal verifier assertion failed","vow_id":0,"source":null,"blame":"none"}
```

An agent consuming this JSON attributes the failure to the function's first
contract clause, which is not what failed. **Two of the five defects the
issue lists are already fixed** by #1204 (landed just before this issue was
filed): `violation`/`message` no longer show the literal `"[Counterexample]"`
string, and `blame` already correctly reports `"none"`. `extract_assert_label`
(Rust) / `describe_assert_label` (self-hosted) already recognize
`"integer shift count"` and map it to a real description. **Confirmed still
broken** (verified locally with `esbmc 8.3.0` and the current tree): (a)
`vow_id: 0` still collides with a real `vow:0`, (b) `error_code` is still
`VowRequiresViolated` for a non-attributable failure, and (c) ESBMC's own
`"division by zero"` property text (confirmed verbatim via a local ESBMC run)
is not yet recognized by `extract_assert_label`/`describe_assert_label` at
all, so it falls through to the generic `"internal verifier assertion
failed"` text instead of a specific description.

## 2. Files to touch

Follows the exact pattern #1150 established for `arith:` obligations
(its own property class, its own diagnostic, no borrowed contract code) —
but lighter weight, since no new C-emitter label is needed: ESBMC's
`"division by zero"` text is stable and already unlabelled by design (it is
not something Vow emits), and `"integer shift count"` is already Vow-labelled
and already recognized. The only genuinely new piece of state is a **third
reserved sentinel vow_id**, alongside the two that already exist
(`UNSUPPORTED_OP_VOW_ID = u32::MAX`, `CALLER_PRECONDITION_VOW_ID = u32::MAX - 1`),
plus a new `ErrorCode` and the wiring to use it. `StructuredCounterexample.vow_id`
**stays `u32`** — no `Option<u32>` threading, matching the sentinel idiom the
schema already documents as required+non-negative and the two existing
sentinels already establish. This keeps the change surgical.

**Rust (`vow-verify`, `vow`, `vow-diag`):**
- `vow-verify/src/c_emitter.rs` — add `pub const UNATTRIBUTED_VOW_ID: u32 = u32::MAX - 2;` next to `UNSUPPORTED_OP_VOW_ID`/`CALLER_PRECONDITION_VOW_ID`.
- `vow-verify/src/esbmc.rs` — add a `"division by zero" => "division or remainder by zero"` arm to `extract_assert_label`.
- `vow-verify/src/lib.rs` — re-export `UNATTRIBUTED_VOW_ID`.
- `vow/src/counterexample.rs` — change `let vid = ce.vow_id.unwrap_or(0);` to `.unwrap_or(vow_verify::UNATTRIBUTED_VOW_ID)`.
- `vow-diag/src/lib.rs` — add `ErrorCode::VerifierAssertionUnattributed` with a doc comment mirroring `ArithOverflowReachable`'s.
- `vow/src/verify_outcome.rs` — in `to_output_with_warnings`'s per-`sce` loop, branch on `sce.vow_id == vow_verify::UNATTRIBUTED_VOW_ID` *before* calling `blame_to_error_code`: use `ErrorCode::VerifierAssertionUnattributed` and a message that does not say "contract violation" (e.g. `"verification failed in `{fn}` on an unattributed property: {violation}"`). Leave `blame_to_error_code`'s existing caller/callee/fallback behaviour untouched — it stays reachable only for `UNSUPPORTED_OP_VOW_ID`, which is out of scope (see §6).
- `vow/src/replay.rs` — add `UNATTRIBUTED_VOW_ID` to the synthetic-id skip check at line ~536 (alongside `CALLER_PRECONDITION_VOW_ID`/`UNSUPPORTED_OP_VOW_ID`), since there is no runtime `VowViolation` a replay could ever match against it.

**Self-hosted (`compiler/`) — same session, mirrors the Rust change 1:1:**
- `compiler/verifier_ids.vow` — add `fn verifier_unattributed_vow_id() -> i64 { 4294967293 }` (single source of truth, mirroring the other two sentinels already there).
- `compiler/verifier.vow` — add a `"division by zero"` branch to `describe_assert_label`, returning the identical string used on the Rust side (`"division or remainder by zero"`) — parity scripts diff the two compilers' output verbatim.
- `compiler/main.vow` — in `build_ce_from_result` (~line 535-542), change the `public_vow_id` computation from the `if vow_id < 0 { 0 } else { vow_id }` fallback to `verifier_unattributed_vow_id()`, and rewrite the stale comment at lines 537-539 (it currently documents "keep `-1` internal, expose public `0`" as intentional — that sentence is the bug). In `replay_one_ce` (~line 792), add `|| ce.vow_id == verifier_unattributed_vow_id()` to the skip condition.
- No self-hosted equivalent of `blame_to_error_code`/`ErrorCode::VerifierAssertionUnattributed` is needed: `compiler/diag.vow`'s `dctx`/`diagnostics[]` path never emits a `VowRequiresViolated`-class entry for a `VerifyCE` at all today (confirmed by inspection — only `VerificationSkipped`, `LoweringWarning`, and `ArithOverflowReachable` are pushed into `dctx` from `main.vow`). The self-hosted counterexample failure surfaces solely through `counterexamples[]`, which this plan already fixes via the `vow_id` sentinel. This asymmetry with Rust's `diagnostics[]` entry is pre-existing (see §6) and not introduced or worsened by this change.

**Docs (`docs/spec/`) — required by CLAUDE.md's spec-is-source-of-truth rule:**
- `docs/spec/errors.md` — new `### VerifierAssertionUnattributed` section (Phase: Verification; meaning; example JSON; fix guidance), inserted after `### ArithOverflowReachable` and before `## Runtime Errors`, following that section's exact format.
- `docs/spec/schemas/diagnostic.schema.json` — add `"VerifierAssertionUnattributed"` to the `error_code` enum array (this schema is validated by `vow-diag`'s `assert_schema_lists` test helper — see Slice 4).
- `docs/spec/schemas/counterexample.schema.json` — tighten the `vow_id` field description to mention it may carry a reserved sentinel for a non-attributable failure (generic wording covering all three sentinels, not enumerating each).
- `docs/spec/cli.md` — in the "When `blame` is `"none"`" paragraph (~line 353-355), add "division by zero" to the parenthetical list of example causes (currently: "collection bounds or capacity, unwrap-on-None, or shift count").
- Regenerate mirrors: `uv run python scripts/generate_help.py` (updates `skills/vow/reference/errors.md`, `skills/vow/schemas/*.json`, and the embedded skill text baked into `vow/src/skill.rs` and `compiler/main.vow`), then `cargo build --release -p vow` and `scripts/bootstrap.sh --skip-cargo`.

**Test fixtures:**
- `tests/verify-fail/division_by_zero_unattributed.vow` — new, unguarded `%` (or `/`) with no `requires` ruling out zero.
- `tests/verify-fail/shift_count_unattributed.vow` — new, unguarded shift count (mirrors the issue's `s` example); confirms the already-fixed description text now also gets the correct `vow_id`/`error_code`.

## 3. TDD slices

1. **Recognize ESBMC's `"division by zero"` property (Rust).**
   RED: `vow-verify/src/esbmc.rs`, new test `extract_assert_label_maps_division_by_zero`, asserting `extract_assert_label(<fixture built from the real ESBMC block>) == Some("division or remainder by zero")`. Use the exact block shape confirmed by a local `esbmc --multi-property --z3` run: `Violated property:` / `  file ... line ... column ... function ...` / `  division by zero` / `  CWE: CWE-369` / `  b != 0`. GREEN: add the match arm in `extract_assert_label`.

2. **Introduce `UNATTRIBUTED_VOW_ID` and stop fabricating `vow_id: 0` (Rust).**
   RED: `vow/src/counterexample.rs` test module — add a new test asserting `build_structured_counterexample(...).vow_id == vow_verify::UNATTRIBUTED_VOW_ID` for an unlabelled-property counterexample, and extend the three existing `structured_unattributed_counterexample`-based tests (`..._vec_bounds_...`, `..._string_capacity_...`, `..._unrecognized_label_...`) with the same `vow_id` assertion (they currently only check `violation`/`blame`, so this is additive). GREEN: add `UNATTRIBUTED_VOW_ID` to `c_emitter.rs`, re-export from `lib.rs`, change the `unwrap_or(0)` in `counterexample.rs`.

3. **End-to-end confirmation the two slices compose (Rust).**
   Add a `structured_unattributed_counterexample("division by zero")` case (reusing the test helper already in `vow/src/counterexample.rs`) asserting `violation == "division or remainder by zero"`, `blame == "none"`, `vow_id == UNATTRIBUTED_VOW_ID`. Should pass with no new production code — this is a regression lock for the composition of slices 1+2, not a new behavior.

4. **New `ErrorCode` variant, schema-checked.**
   RED: `vow-diag/src/lib.rs`, new test `diagnostic_schema_lists_verifier_assertion_unattributed` using the existing `assert_schema_lists([ErrorCode::VerifierAssertionUnattributed])` helper — fails to compile until the variant exists, then fails the assertion until the schema is updated. GREEN: add the `ErrorCode` variant (with doc comment) and add it to `docs/spec/schemas/diagnostic.schema.json`'s enum.

5. **Wire the sentinel to the new error code in `verify_outcome.rs`.**
   RED: new test `failed_unattributed_counterexample_maps_to_verifier_assertion_unattributed` — build a `StructuredCounterexample` with `vow_id: vow_verify::UNATTRIBUTED_VOW_ID`, `blame: "none".to_string()`, `violation: "division or remainder by zero".to_string()`; call `to_output`; assert `diagnostics[0].code == ErrorCode::VerifierAssertionUnattributed`, `diagnostics[0].blame == Blame::None`, and the message does not contain `"contract violation"`. GREEN: add the branch ahead of the `blame_to_error_code(&sce.blame)` call. Confirm the pre-existing tests (`blame_to_error_code_maps_caller_callee_and_fallback`, `failed_caller_blame_...`, `failed_callee_blame_emits_single_hint`) are untouched and still pass — they all use `vow_id: 1` via the `ce()` helper, so the new branch never triggers for them. Also update `blame_to_error_code`'s doc comment (currently: "an unrecognised blame maps to `VowRequiresViolated` ... This is preserved behaviour, not a fix") — it becomes misleading once the new branch intercepts ahead of it; note that the fallback is now reachable only via `UNSUPPORTED_OP_VOW_ID` (see §6).

6. **Replay must skip the new sentinel (Rust).**
   RED: `vow/src/replay.rs`, new test mirroring the existing `CALLER_PRECONDITION_VOW_ID`/`UNSUPPORTED_OP_VOW_ID` skip test, using `UNATTRIBUTED_VOW_ID`, asserting a `"skipped"` outcome. GREEN: extend the `if` condition at line ~536.

7. **Self-hosted mirror (same session, per CLAUDE.md).**
   RED: `compiler/tests/test_verifier.vow` — add `check_parse_assert_label_maps_division_by_zero`, identical shape to the existing `check_parse_assert_label_maps_known_labels` cases, asserting the same string as slice 1. GREEN: `compiler/verifier_ids.vow` (new sentinel fn), `compiler/verifier.vow` (`describe_assert_label` arm), `compiler/main.vow` (`build_ce_from_result` fallback + `replay_one_ce` skip condition + the two stale comments). Run via `vowc test compiler/` (Rust-hosted `cargo run -p vow -- test compiler/` and self-hosted `build/vowc test compiler/`) so both interpreters of the test file agree.

8. **End-to-end fixtures, both compilers.**
   Add `tests/verify-fail/division_by_zero_unattributed.vow` and `tests/verify-fail/shift_count_unattributed.vow` with `// TEST: counterexample-vow-id 4294967293`, `// TEST: counterexample-blame none`, `// TEST: counterexample-violation "..."`, `// TEST: counterexample-fn "..."` directives (mirroring `tests/verify-fail/checked_arith_contract_false.vow`'s style). Deliberately **no** `// TEST: error-code ...` directive here: `error-code` checks `diagnostics[]` filtered to `severity == "error"`, and `compiler/diag.vow` never pushes a `VowRequiresViolated`-class entry into `dctx` for a `VerifyCE` at all (§2) — self-hosted's `diagnostics[]` stays empty on `VerifyFailed`, so an `error-code` directive would fail on the self-hosted run even though the fix is correct. `counterexample-*` fields are emitted by both compilers and are what actually locks the fix; `error_code` is already locked by slice 5's Rust-only unit test. Run `tests/run_tests.sh` (exercises both `$RUST` and the self-hosted binary against the same fixture) to confirm parity.

9. **Docs regeneration and final gate.**
   `docs/spec/errors.md`, `docs/spec/schemas/*.json`, `docs/spec/cli.md` edits (§2), then `uv run python scripts/generate_help.py`, `cargo build --release -p vow`, `scripts/bootstrap.sh --skip-cargo`. Full gate: `cargo test --all`, `cargo clippy --all -- -D warnings`, `cargo fmt --all -- --check`, `scripts/full_test.sh` (covers `tests/verify*/`, `tests/error/`, the help-coverage staleness detector, and cross-compiler parity).

## 4. Verification surface

No contracts, IR, or C-model changes. This is pure diagnostic-plumbing: no
new `__ESBMC_assert` is introduced (unlike #1150's `arith:` labels), no
existing C emission changes, and no verification property gains or loses
provability. The only "verification surface" is the two new `tests/verify-fail/`
fixtures, which must actually reach `VerifyFailed` via the *unattributed*
path (unguarded `%`/`/` and unguarded shift) rather than accidentally hitting
a different property first — confirmed locally that plain `a % b` with no
`requires` reaches ESBMC's `"division by zero"` property before anything
else fires (verified empirically against the current tree in planning).

## 5. Risk areas

- **Sentinel collision:** `UNATTRIBUTED_VOW_ID = u32::MAX - 2` must stay
  distinct from `UNSUPPORTED_OP_VOW_ID` (`u32::MAX`) and
  `CALLER_PRECONDITION_VOW_ID` (`u32::MAX - 1`), and from the self-hosted
  `4294967293` literal — a typo in either the Rust constant or the
  `verifier_ids.vow` literal would silently misattribute. Slice 7's test
  catches this only indirectly (via the description text, not the numeric
  value); consider a direct equality assertion between the two sentinel
  literals as an extra guard if the self-hosted test harness makes that easy.
- **Binary fixed point:** `compiler/verifier_ids.vow` and `compiler/main.vow`
  changes must go through the full bootstrap triple (`scripts/bootstrap.sh`)
  before landing, since `build/vowc` is itself compiled with the version of
  `compiler/` being edited. A stale `build/vowc` used to *test* the new
  self-hosted code would validate against the old sentinel logic.
- **Cross-compiler parity:** `scripts/full_test.sh`/`scripts/test_parity.py`
  diff Rust vs. self-hosted JSON for shared fixtures. The `"division or
  remainder by zero"` string must be byte-identical in both
  `extract_assert_label` (Rust) and `describe_assert_label` (self-hosted) —
  a wording mismatch fails parity, not compilation, so it will only surface
  at test time, not at build time.
- **Verify cache:** `vow/src/cache.rs` caches ESBMC failures keyed on C
  source; a cached `Counterexample` from *before* this fix could still carry
  the old shape if the cache isn't invalidated. This should be a non-issue
  since the cache stores raw ESBMC output (re-parsed on every read via
  `parse_esbmc_output`), not the derived `vow_id`/`error_code` — worth a
  one-line confirmation in implementation, not a design change.
- **`blame_to_error_code`'s existing fallback:** deliberately left
  unchanged (still maps unrecognised blame → `VowRequiresViolated`) so the
  `UNSUPPORTED_OP_VOW_ID` case's current (also arguably wrong, but
  unlabelled-property-issue-adjacent) behavior doesn't shift as a side
  effect of this fix. Flagged explicitly in §6 rather than silently patched,
  per "many small changes."
- **clippy/fmt:** new `match`/`if` arms and one new enum variant; no
  structural risk, but run `cargo clippy --all -- -D warnings` and
  `cargo fmt --all` as always before considering the slice done.

## 6. Out of scope

- **`span` for unattributed properties.** The issue's evidence table also
  flags `span: empty → should be the operator's span`; this plan does not
  fix it. `source`/`span` stays `null`/empty for the unattributed case. The
  issue's own suggested fix frames the shift assert's span as "free" because
  it's Vow-emitted — but making it free requires a span-carrying label
  (e.g. `shift:<func_id>:<start>:<len>`, mirroring `arith:<cause>:<span>`),
  which means C-emitter changes in **both** compilers, a new parser arm, and
  plumbing the resolved span into `StructuredCounterexample.source` — plus
  it changes the emitted C source, so every cached verification result for a
  function containing a shift invalidates. ESBMC's built-in division-by-zero
  property has no Vow-emitted assert to label at all, so it cannot get a
  span this way regardless (ESBMC's own property location line does carry a
  file/line/column, but wiring that into `source` is a different, larger
  parsing task). Worth a dedicated follow-up issue; not bundled here.
- **`UNSUPPORTED_OP_VOW_ID`'s error-code mismatch.** It also has `blame:
  "none"` and also currently maps to `VowRequiresViolated` via
  `blame_to_error_code`'s fallback — but it already carries a real,
  non-colliding sentinel `vow_id` and a correct, specific `violation`
  message ("function uses side-effecting operations not supported for
  verification"). Its `error_code` is arguably also wrong, but it is not an
  *unlabelled* property (`vow:{UNSUPPORTED_OP_VOW_ID}` is an explicit label)
  and therefore not what issue #1151 is about. Worth its own follow-up issue.
- **`diagnostics[]` containing a per-counterexample entry at all.** The
  documented `VerifyFailed` example in `docs/spec/cli.md` (line 324) shows
  `"diagnostics": []`, and the CLI's own agent-decision-tree (line 597) tells
  agents to read `counterexamples[]`, not `diagnostics[]`, for `VerifyFailed`.
  The current Rust implementation pushes a `Diagnostic` into `diagnostics[]`
  for every counterexample anyway (confirmed empirically) — a pre-existing
  doc/code mismatch, unrelated to vow_id/error_code correctness, and a much
  larger change (removing an established, tested field) than this issue
  asks for. Not touched here; flagged for a separate doc-or-code
  reconciliation issue.
- **#1148 (unchecked `/`/`%` abort with no diagnostic) and #599
  (`/!` `MIN/-1` uncontrolled hardware trap).** Explicitly named in the
  issue as runtime/codegen siblings, not verifier-side mislabelling. No
  codegen or runtime changes in this plan.
- **Making the unattributed case a suppressed re-verify (like #1150's
  `arith:` warning treatment).** That pattern exists because a checked
  operator's abort is *specified behaviour* that shouldn't mask the
  contract verdict. An unguarded unchecked division-by-zero or an
  unattributed internal assert is a genuine, still-unproved hazard — it
  must keep failing the build (`VerifyFailed`), just with honest fields.
  No re-run/suppression logic is added.
- **Backfilling the stale `docs/spec/schemas/diagnostic.schema.json` enum**
  for other already-emitted-but-missing codes (`ArithOverflowReachable`,
  `ShiftCountOutOfRange`, `VerificationSkipped`,
  `BTreeMapKeyTypeMustBeI64`/`BTreeMapValueMustBeNonLinear`,
  `NarrowingCastNotAllowed`, `LiteralOutOfRange`, `TautologicalComparison`
  are all missing from that enum today). Only the new
  `VerifierAssertionUnattributed` code is added; the pre-existing gaps are
  a separate cleanup.
- **Formatting/refactor of the surrounding functions** (`extract_assert_label`,
  `build_structured_counterexample_with_module`, `to_output_with_warnings`,
  `build_ce_from_result`) beyond the minimal new arms/branches described
  above.
