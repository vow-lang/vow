# Plan: attribute co-emitted callee `ensures`/`invariant` failures to the callee's own contract (#609)

## Goal
When verifying target `f`, a failing assertion from a co-emitted callee `g`'s `ensures`/`invariant` must be reported against `g` (its `function`, `violation`, `blame: callee`, `source` span), not resolved against `f`'s same-numbered vow. Land identically in the Rust and self-hosted compilers with byte-identical verifier C.

## Current state (verified on this branch; issue line numbers are stale)
- Callee `requires` was already fixed: C emits `"vow:pre:<func_id>:<vow_id>"` when `requires_as_assert` is true (`vow-verify/src/c_emitter.rs:1360-1380`; `compiler/c_emitter.vow:1811-1822`), parsed into `CalleePrecondition` (`vow-verify/src/esbmc.rs:61-65,282-296`; `compiler/verifier.vow:796-823`) and resolved via the module (`vow/src/counterexample.rs:188-225`; `compiler/main.vow:505-570`).
- **Still broken**: `Opcode::VowEnsures | VowInvariant` emit bare `"vow:<local id>"` for target AND callees (`c_emitter.rs:1386-1396`; `c_emitter.vow:1824-1829`). The parse yields only `vow_id`, and `counterexample.rs:215-225` resolves it in `func.vows`, with `function: func.name` at the end of the builder.
- Side effect of the same collision: `parse_multi_property_verdicts`/`find_vow_claim_id` (`esbmc.rs:338-380`; `verifier.vow:1016-1070`) AND-combine a callee's `vow:0` PASS/FAIL into the target's clause 0 verdict (`vow contracts --verify`).
- `emit_c_module_with_callees` already passes `requires_as_assert=true` for callees only and `false` for the target (`c_emitter.rs:3340-3365`) — the exact discriminator needed. The self-hosted twin passes the same flag.

## Assumptions
- Design (best guess, matches existing precedent): new label `vow:post:<func_id>:<vow_id>` for callee-emitted `ensures`/`invariant`; the target's own clauses keep `vow:<id>`. Rejected: changing every label to `vow:<func>:<id>` (would churn every `vow:N` test/fixture, multi-property, vacuity, bodyreplace, and cache keys for no gain), and module-globally-unique VowIds (changes IR, runtime `vow_id` JSON, debug-mode violations).
- Public `vow_id` in the counterexample stays the callee-local id (it is what a debug-mode runtime `VowViolation` for `g` reports), paired with `function: "g"`.
- Blame comes from the callee's `VowEntry.blame` (Callee for ensures/invariant); no override.
- Out of scope design question (separate issue): whether callee `ensures` should be *assumed* (contract stub) rather than re-asserted when verifying the caller (audit finding at `docs/audit-20260610/vow-analysis.md:507`). This plan only fixes attribution.
- The outcome-level `VerifyOutcome::Failed.function` (`vow/src/verification.rs:167-176`) stays the verify target; only `counterexamples[].function` is the owner. Recorded in cli.md.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `vow-verify/src/c_emitter.rs` | emit `vow:post:` for callee ensures/invariant; `caller_preconditions_only_source` keep-list check | 1386-1396, 1360-1380, 2913-2955 |
| `vow-verify/src/esbmc.rs` | `CalleePostcondition` type, `VowLabel` variant, `parse_vow_label`, `Counterexample` field | 33-65, 145-185, 261-296 |
| `vow-verify/src/lib.rs` | re-export new type | 13 |
| `vow/src/counterexample.rs` | resolve owner function/entry; `function`, span, name map, blame | 188-345 |
| `vow/src/cache.rs` | persist new field (`#[serde(default)]`) | 140-245 |
| `vow-verify/src/solver_strategy.rs`, `vow/src/verification.rs` | add `callee_postcondition: None` to `Counterexample` literals | 1193,1256; 871,1055 |
| `compiler/c_emitter.vow` | mirror emitter change | 1824-1829 |
| `compiler/verifier.vow` | `ParsedVowLabel`/`VerifyResult` fields, `parse_vow_label_text`, all literals | 19-34, 788-844, ~1670-1950 |
| `compiler/main.vow` | `build_ce_from_result`: post branch, `function` = owner | 479-600 |
| `compiler/verify_report.vow` | diagnostic uses `ce.function` already — confirm | 205-235 |
| `docs/spec/cli.md`, `docs/spec/contracts.md` | document attribution rule | cli.md ~400-420; contracts.md ~440-475 |
| `tests/verify-fail/callee_ensures_wrong_function.vow` [new] | end-to-end regression | — |

## Steps (TDD slices, each red → green)

### 1. Emitter: label callee ensures/invariant (both compilers, one slice for parity)
- Red: `c_emitter.rs` unit test next to `emit_callee_requires_as_structured_precondition_assert` (~4504): module with callee `g` (func id 3, ensures vow 0) co-emitted with target; assert C contains `__ESBMC_assert(vN, "vow:post:3:0");` and target keeps `"vow:0"`. Same in `compiler/tests/test_c_emitter.vow` (existing ensures check at :184 stays for target).
- Green: in `Opcode::VowEnsures|VowInvariant` branch, when `requires_as_assert` (callee flag) emit `vow:post:{current_func_id}:{vow_id}`. Introduce `CALLEE_POSTCONDITION_LABEL = "vow:post:"` next to `CALLEE_PRECONDITION_LABEL` (c_emitter.rs:2918). Mirror in `compiler/c_emitter.vow` (use `f.id`, `inst.dv`). Consider renaming the flag doc to "callee mode" (comment only, no rename).
- `caller_preconditions_only_source` (c_emitter.rs:2933) demotes everything except `vow:pre:` and trap, so `vow:post:` asserts are demoted like today's callee `vow:N` — add a test pinning that; check the `.vow` mirror (`verifier.vow:224-227`) behaves identically.
- Parity: run `python3 scripts/parity.py c ./target/release/vow build/vowc tests/verify*/...` over a fixture with a contracted callee.

### 2. Parse: new label → structured field (Rust)
- Red: `esbmc.rs` tests beside `parse_callee_precondition_label_extracts_callee_contract` (~2311): `Violated property:\n  vow:post:3:0` → `callee_postcondition == Some{func_id:3,vow_id:0}`, `vow_id == Some(0)`; malformed (`vow:post:3`, `vow:post:3:0:1`, `vow:post:x:0`) → `None` and not misparsed as numeric.
- Green: add `pub struct CalleePostcondition {func_id, vow_id}` (same shape as `CalleePrecondition`; do not unify in this PR), `VowLabel::CalleePostcondition`, `post:` arm in `parse_vow_label`, `Counterexample.callee_postcondition`. Update literals (`solver_strategy.rs`, `verification.rs`, `counterexample.rs` tests, `cache.rs`).
- Multi-property test: `parse_multi_property_verdicts` on output with `vow:0` PASSED (target) and `vow:post:3:0` FAILED yields `{0: true}` — pins the latent collision fix; same for `find_vow_claim_id("vow:post:3:0") == None`. No code change expected (`vow:` + non-digit is skipped) — if the assertion fails, fix `find_vow_claim_id` in both compilers.

### 3. Cache round-trip
- Red: `cache.rs` test beside :401/:602 — failure with `callee_postcondition` survives write/read; a record JSON lacking the new keys deserializes with `None`.
- Green: add `callee_postcondition_func_id/vow_id: Option<u32>` to the record with `#[serde(default)]`. Cache keys hash the C source, which now differs for callee ensures, so stale entries cannot be hit; no version bump.

### 4. Rust structured CE resolves the owner
- Red: `counterexample.rs` tests (module already has CE builders, ~1000-1500): module with `f` (vow 0 = ensures "f clause") and `g` (vow 0 = ensures "g clause", distinct description/span/file); `Counterexample{vow_id:Some(0), callee_postcondition:Some{g.id,0}}` → `function == "g"`, `violation == "g clause"`, `blame == "callee"`, `source` = g's span/file, `vow_id == 0`, `values` mapped through g's name map. Also: unknown func id → falls back to existing generic path ("internal verifier assertion failed"/raw label), not f's vow 0. And the pre-existing `callee_precondition` tests stay green (`function == "f"`).
- Green: generalize `resolved_callee_precondition` into an `owner` resolution: `(owner_func, entry)` from either `callee_precondition` or `callee_postcondition`; `vow_func = owner_func`; `function: vow_func.name` only for the post case (keep `func.name` for `requires`, per `tests/verify-fail/caller_requires_violation.vow` `counterexample-fn "f"`). Without a module (`None`) never fall back to `func.vows` for a post label. Execution path/branch decisions: derived from `func` blocks keyed by `__blk_` ids of the whole model, so for a callee-owned failure use the owner's blocks only if ids are per-function; otherwise leave empty — decide by reading `block_visits` emission in c_emitter and document in a comment-free way via test (assert no panic and path empty or owner-scoped).

### 5. Self-hosted mirror
- Red: `compiler/tests/test_verifier.vow` (near :118 / `parse_vow_label` tests): `vow:post:3:0` parses to `callee_postcondition_func_id=3, vow_id=0`; malformed rejected; numeric unchanged. A test for `build_ce_from_result` if one exists in `compiler/tests/` (grep `build_ce_from_result`; if not, rely on step 6 fixture).
- Green: add `callee_postcondition_func_id/vow_id` to `VerifyResult` + `ParsedVowLabel` (update every literal: verifier.vow 788-823, ~1694, 1709, 1734, 1873-1948), `post:` arm in `parse_vow_label_text` (today `str_to_i64("post:…")` would yield garbage — must be explicit), and a branch in `build_ce_from_result` (main.vow:~505) after the precondition branch: look up callee by id, set `ce_violation=ve.description`, `ce_blame=ve.blame`, source via `ce_source_for_vow(callee, ve, path)`, and return `VerifyCE{function: callee.name, …}` with `var_names` unchanged. Reuse `find_ir_function_index_by_id`, `find_ir_vow_index`.
- Keep `vf_add_ce_diagnostics` (verify_report.vow:205) as is: it already uses `ce.function` and `ce.blame`; verify EC mapping gives `VowEnsuresViolated`.

### 6. End-to-end fixture + parity
- `tests/verify-fail/callee_ensures_wrong_function.vow` [new]: `g` with a false `ensures` and `f` with a *different, true* clause at vow id 0; directives `counterexample-fn "g"`, `counterexample-blame callee`, `counterexample-vow-id 0`, `counterexample-violation "<g clause text>"`. Verification is multi-threaded, so either `f` or `g` can be the failing target first; post-fix both report `function: g`, making the expectation deterministic. Confirm how `full_test.sh` Section 4c runs both compilers on it; ensure `f` is modelable and `g` is not itself excluded.
- Section 2c C-parity picks up `tests/verify*/` automatically; also run the new fixture explicitly.

### 7. Spec + help
- `docs/spec/cli.md` (~line 400-420): add paragraph — when a co-emitted callee's `ensures`/`invariant` fails during a caller's verification, `counterexamples[].function` names the callee, `violation`/`source`/`blame` are the callee's clause, `vow_id` is the callee-local id; top-level outcome `function` remains the verified target.
- `docs/spec/contracts.md` (~469): same under "Interpreting Counterexamples".
- If `docs/spec` edits touch text embedded in help/skill: `uv run python scripts/generate_help.py`, then `cargo build --release -p vow` and `scripts/bootstrap.sh --skip-cargo`; `python3 scripts/check_help_coverage.py` must pass (also regenerates `skills/vow/reference/*`, `vow/src/skill.rs`, `compiler/main.vow` embeds).

## Verification surface
- No change to what ESBMC proves; only the label text of callee-emitted `ensures`/`invariant` asserts changes. Target-function labels (`vow:N`), vacuity (`vow_reach`), body-replace, and multi-property verdict maps are untouched. No `tests/run/` fixtures or `examples/` need to grow. Benefit: `vow contracts --verify` clause statuses no longer polluted by callee ids.

## Testing / gates (run separately, background + poll; see memory on wall-clock)
- `cargo test -p vow-verify`, `cargo test -p vow`, `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all --check`.
- `build/vowc test compiler/tests/test_verifier.vow`, `test_c_emitter.vow` (after `scripts/bootstrap.sh --skip-cargo`).
- `scripts/bootstrap.sh --skip-cargo --no-cache` at the final head SHA (record SHA in PR); `scripts/full_test.sh` (Sections 2c parity, 4c verify-fail). Use `VOW_CACHE_DIR=$(mktemp -d)` when validating.
- Known pre-existing failures (verify against clean main before blaming): see memory notes (`u64_marker_propagation`, `contracts_tmp_cleanup`, `concrete-block-region-parity`, e2e SKIP-panics).

## Risks
- **C parity drift**: both emitters must change in the same commit; parity.py runs over `tests/verify*/` only if a fixture contains a contracted callee with ensures — the new fixture does.
- **Binary fixed point**: `.vow` changes are in verifier/main modules (no codegen-order change); `BTreeMap` not touched. Struct field additions to `VerifyResult` require updating *every* literal or the self-hosted compile fails (grep `callee_precondition_func_id:`).
- **Cache**: records written before this change lack the field → `serde(default)`; no stale-hit risk since C text differs.
- **`caller_preconditions_only_source`** string-matches labels; verify `vow:post:` lines are demoted (not kept) in both compilers — a regression there would make uncontracted callers fail on callee ensures.
- **Replay (`--replay-cex`)**: compares failing `vow_id`+blame to the runtime violation of the *target*; a callee-owned CE must not replay-confirm against `f`. Check `vow/src/replay.rs` and the `compiler/main.vow` replay section (~line 640+); if it keys on `ce.function`, it now replays `g` (acceptable); otherwise mark `skipped` with a reason. Add a test if behavior changes.
- **Non-determinism in the fixture** (parallel verification order): addressed by asserting post-fix output that is order-independent.
- Execution-path/branch data for a callee-owned CE is block-id-scoped to the target; do not present target blocks as the callee's path (step 4).
- No new type-system axis or language feature; contracts untouched.

## Out of scope
- Assuming callee `ensures` via contract stubs / not re-asserting callee bodies (audit §507); callee-body soundness work.
- Unifying `CalleePrecondition`/`CalleePostcondition` into one type or a generic `vow:<func>:<id>` scheme.
- Making vow ids module-globally unique; changing runtime `VowViolation` JSON.
- Attributing capacity/bounds/`UNATTRIBUTED` properties to callees (separate audit finding), arith overflow (already carries `func_id`).
- Refactors, formatting, unrelated cleanups.
