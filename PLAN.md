# Plan: issue #1169 — `ec_name` must not call an unmapped code `IoError`

## 1. Problem restated

`compiler/diag.vow`'s `ec_name(code: i64) -> String` (lines 188–302) is a long
`if`/`else if` chain over every `EC_*` error-code constant. `EC_IO_ERROR()`
(declared at line 25, value `11`) is never checked explicitly — `"IoError"` is
simply the string returned by the chain's terminal `else` arm (line 300), so
*any* code that fails to match every preceding arm — including a future
`EC_*` constant added without a matching `ec_name` arm — silently renders as
`"IoError"` in both the JSON (`diag_to_json`, line 355) and human
(`diag_to_human`, line 422) diagnostic output. Since `error_code` is the
field agents branch on, a mislabelled code is a silent-failure shape, not
just a cosmetic bug. The fix: add an explicit `EC_IO_ERROR()` arm and replace
the terminal `else` with a real, distinct default.

## 2. Files to touch

- `compiler/diag.vow` — production fix (only file changed in the self-hosted
  compiler).
- `compiler/tests/test_diag_ec_name.vow` — **new** test file (TDD, see §3).
- No Rust crate changes. `vow-diag/src/lib.rs`'s `ErrorCode` enum has no
  manual name-lookup match with a fallback arm — its only rendering is
  `#[derive(Debug)]`'s `{:?}` format, used directly at `vow-diag/src/lib.rs:199`
  and locked by a test at `vow-diag/src/lib.rs:349–351` asserting
  `format!("{:?}", ErrorCode::RegionConflict) == "RegionConflict"`. This is
  exhaustive by construction (the compiler enforces every variant is
  covered), so the audit the issue requests ("consider mirroring... on the
  Rust side") is **done, and negative**: there is nothing to change there.
- No `docs/spec/*.md` changes. This fix adds no new `EC_*` constant, and
  changes no syntax, semantics, builtin, operator, effect, or CLI flag — it
  only changes which string an existing internal helper returns for inputs
  that must never legitimately occur. See §6 for the pre-existing
  `docs/spec/schemas/diagnostic.schema.json` drift this surfaced, which is
  explicitly *not* being fixed here.

## 3. TDD slices

### Slice 1 (the only slice in this PR): explicit `EC_IO_ERROR()` arm + distinct terminal default

**Test** — new file `compiler/tests/test_diag_ec_name.vow`, mirroring the
existing `compiler/tests/test_smoke.vow` layout (`module TestDiagEcName`,
`use diag`, `fn main() -> i32` returning `0` on success and a distinct
nonzero code per failed assertion). Do not extend `test_smoke.vow` itself —
it is documented as a layout smoke test for the `compiler/tests/` scan
mechanism, not a place for feature-specific assertions.

Assertions (each maps to a distinct return code so a failure is
identifiable):
1. `ec_name(EC_IO_ERROR()) == String::from("IoError")` — the new explicit arm.
2. `ec_name(EC_UNTERMINATED_STRING()) == String::from("UnterminatedString")` —
   first arm in the chain, regression guard for the insertion.
3. `ec_name(EC_LINK_FAILED()) == String::from("LinkFailed")` — last arm
   before the terminal `else`, regression guard for the insertion point.
4. `ec_name(-1) == String::from("Unknown")` — **the red-then-green
   assertion**. Today this returns `"IoError"` (the bug); after the fix it
   must return the new distinct sentinel. `-1` is guaranteed to never match
   any `EC_*` constant (all are non-negative).

This test is RED before the production change (assertion 4 observes
`"IoError"` instead of `"Unknown"`) and GREEN after it. Assertions 1–3 are
regression guards that should already pass and must keep passing.

**Production code** — in `compiler/diag.vow`:
- Insert a new arm immediately after the existing `EC_LOWERING_WARNING()` arm
  (after line 236's `} else {`), matching the constants' declaration order
  (`EC_LOWERING_WARNING` at line 24, `EC_IO_ERROR` at line 25):
  ```vow
  if code == EC_IO_ERROR() {
      String::from("IoError")
  } else {
  ```
  (with one matching `}` added to the closing brace run at the end of line 301).
- Change the terminal `else` body at line 300 from `String::from("IoError")`
  to `String::from("Unknown")`.

No other arm in the chain moves or changes.

**Gate for this slice:**
```bash
scripts/bootstrap.sh --skip-cargo    # rebuild build/vowc from the modified diag.vow
build/vowc test compiler/
./target/release/vow test compiler/  # Rust-compiled self-hosted-source parity
```
`build/vowc` will not see the change until it is rebuilt — the test must not
be run against a stale binary. `compiler/tests/test_diag_ec_name.vow` is
auto-discovered by `discover_test_files_recursive`, so no wiring into
`scripts/full_test.sh` is needed for this slice; `vowc test compiler/` (part
of the existing test flow) picks it up automatically. Finish with the
concat/triple-test bootstrap check (see §5) to confirm the fixed point still
holds.

## 4. Verification surface

`compiler/diag.vow` currently has **zero** `vow { ... }` blocks (confirmed:
`grep -c "vow {" compiler/diag.vow` → 0, vs. 110 in `main.vow`, 61 in
`module_io.vow`). `ec_name` is not part of the ESBMC-verified surface today,
and Slice 1 does not change that — it is a pure string-literal edit inside an
un-vowed function. No new ESBMC properties, no new `tests/verify*` or
`tests/run/` fixtures are needed for this slice. (A contract-based approach
*would* touch verification — see §6, Follow-up 1, for why it's deferred
rather than folded in.)

## 5. Risk areas

- **Binary fixed point.** The change is a string-literal swap and one new
  `if`/`else` arm in self-hosted source, compiled identically by both the
  Rust stage-0 compiler and `build/vowc` from the same `compiler/diag.vow`
  text — no codegen-ordering, `BTreeMap`/`HashMap`, or stack-slot concern.
  Still run the standard triple-test gate before considering this done:
  ```bash
  ./scripts/concat_vow.sh clif > /tmp/compiler_clif.vow
  ./target/release/vow --no-verify /tmp/compiler_clif.vow -o /tmp/compiler_a
  /tmp/compiler_a -o /tmp/compiler_b /tmp/compiler_clif.vow
  /tmp/compiler_b -o /tmp/compiler_c /tmp/compiler_clif.vow
  sha256sum /tmp/compiler_b /tmp/compiler_c   # must match
  ```
- **`parse → print → parse` idempotency.** Unaffected — no grammar or AST
  change; the printer never sees `ec_name`'s output.
- **`cargo clippy --all -- -D warnings`.** No Rust files touched; the gate is
  not exercised by this change.
- **The one real operational risk:** forgetting to rebuild `build/vowc`
  (`scripts/bootstrap.sh --skip-cargo`) before running
  `build/vowc test compiler/` — a stale binary will not exercise the new
  arm and the new test would spuriously pass or fail against old code. Call
  this out explicitly during implementation/review.

## 6. Out of scope

Deliberately **not** bundled into this PR, each with a concrete reason:

1. **Contract-based "fail loudly in debug"** — `vow { requires: code ==
   EC_UNTERMINATED_STRING() || code == EC_INVALID_CHARACTER() || ... || code
   == EC_LINK_FAILED() }` on `ec_name`, so an unmapped code aborts
   (`VowViolation`, blame `Caller`, exit 134) under `--mode debug` instead of
   silently returning a placeholder string. Investigated and found
   *technically* cheap on the ESBMC side — no vowed function in the codebase
   transitively calls `ec_name` (its only two callers, `diag_to_json` and
   `diag_to_human`, are called from `[io]`-effectful, non-vowed functions in
   `main.vow`), so the contract would get its own isolated, assume-only
   ESBMC harness with nothing to assert — trivially proven, same shape as
   the existing `requires`-only precedent at `module_io.vow:330`
   (`region_encode`). The reason it's deferred is test infrastructure, not
   verification cost:
   - `vowc test` (`compiler/main.vow`'s `run_test`) has no "expected abort"
     semantics — it treats any nonzero/abnormal exit as `"failed"`
     (`compiler/main.vow` ~line 2230), so a `compiler/tests/*.vow` file that
     intentionally aborts cannot be added to the auto-discovered
     `compiler/tests/` directory without permanently reddening
     `vowc test compiler/`.
   - The only place this repo tests a debug-mode `requires` violation by
     exit code / stderr substring is hand-written bash blocks in
     `scripts/full_test.sh` "Section 5: Debug Mode" (e.g. the
     `examples/divide.vow` and `tests/debug/u8_requires_violation.vow`
     blocks), which build with `--mode debug --no-verify`, run the binary
     directly, and assert `exit == 134` plus stderr substrings — bypassing
     `vowc test` entirely.
   - Reaching `compiler/diag.vow`'s `ec_name` from a fixture living outside
     `compiler/tests/` (as it must, to avoid reddening `vowc test`) requires
     `--module-root compiler` on `vowc build`, per the module-root
     resolution comment at `compiler/main.vow` ~line 2020. This needs
     confirming on both `./target/release/vow build` and `build/vowc build`
     before it can be relied on.
   - Once the contract exists, Slice 1's `ec_name(-1) == "Unknown"`
     assertion would itself abort under `vowc test`'s default debug mode and
     would have to be deleted from `test_diag_ec_name.vow`, replaced by a
     *second* new block proving `"Unknown"` is still reachable in release
     mode (where vow checks are stripped).
   - Net: 4–5 new moving parts (new fixture location, `--module-root`
     confirmation, a new Section 5 bash block, a release-mode counterpart,
     and deleting part of Slice 1) versus Slice 1's 2. This is exactly the
     kind of scope split #1163 → #1169 already modeled once; do it again
     rather than bundling.
   - Worth stating plainly for whoever picks this up: `scripts/bootstrap.sh`
     and the triple test build with no `--mode` flag (release), so this
     contract would never run in the production `build/vowc` binary anyway —
     its value is confined to developer `--mode debug` builds. It is not a
     CI safety net for the shipped compiler.
2. **Same silent-default shape elsewhere.** `diag_blame_name`
   (`compiler/diag.vow:168`, defaults to `"None"`), `sev_name_lower`
   (`compiler/diag.vow:304`), `keyword_text` (`compiler/token.vow:140`), and
   `enum_name_from_tid` (`compiler/checker.vow:1618`) all end their
   `if`/`else` chains with an unguarded default. The issue is scoped to
   `ec_name`; list these as follow-up candidates rather than fixing them
   here.
3. **Orphaned debug-violation fixtures.** `tests/debug/caller_blame_debug.vow`,
   `tests/debug/callee_blame_debug.vow`, and `tests/debug/divide_violation.vow`
   exist but are not invoked by any script (confirmed: no match for their
   filenames anywhere in `scripts/` or `tests/`), unlike their siblings
   `u8_requires_violation.vow` / `i128_requires_violation.vow` /
   `cast_in_contract_violation.vow`, which are wired into
   `scripts/full_test.sh` Section 5. Pre-existing gap, unrelated to this
   issue — not fixed here.
4. **`docs/spec/schemas/diagnostic.schema.json` drift.** Its `error_code`
   enum lists 29 codes but the compiler now defines 37 `EC_*` constants —
   `BTreeMapKeyTypeMustBeI64`, `LiteralOutOfRange`, `ShiftCountOutOfRange`,
   `TautologicalComparison`, `VerificationSkipped`, `ArithOverflowReachable`,
   `RegionLiteralMutation`, and others are already missing. Because the enum
   is already non-exhaustive and not regenerated from `diag.vow` by
   `scripts/generate_help.py`, it is not treated as a closed contract here:
   the new `"Unknown"` sentinel is **deliberately not added** to it (it is a
   defensive fallback for a should-never-happen input, not a documented
   error code). Fixing the pre-existing drift is a separate, larger cleanup
   and is out of scope for this issue.
