# Plan — issue #1078: Codegen-stage failures emit no structured diagnostics

## 1. Problem restated

Every failure raised after the frontend hands the IR to the backend — the seven
`vow_codegen::CodegenError` variants in the Rust driver, and the `-1`/`-2`
returns from `clif_emit_module` in the self-hosted one — reaches the agent as a
`BuildStatus::CompileFailed { message }` with an **empty `diagnostics` array**.
The Rust driver stuffs a `Debug`-formatted string (`UnsupportedOpcode("…")`)
into `message`; the self-hosted driver emits `message` not at all. Either way
there is no `error_code`, no `severity`, no `span`, so an agent must regex free
text to learn that (say) it hit a backend limitation rather than a genuine
program error. The gate in `scripts/parity.py:196` requires *both* compilers to
emit ≥1 structured diagnostic for every `tests/error/*.vow` fixture, so a
codegen-stage rejection cannot be expressed as a black-box fixture at all —
three such fixtures were written during #1057 and withdrawn for exactly this
reason. The fix is to give codegen failures a real `vow-diag` `ErrorCode` and
push a `Diagnostic` alongside the existing `message`, in both compilers.

## 2. Files to touch

### New error codes (both compilers must agree)

| File | Change |
| --- | --- |
| `vow-diag/src/lib.rs` | Add `ErrorCode::{CodegenUnsupported, CodegenFailed, LinkFailed}` to the `#[non_exhaustive]` enum, with doc comments in the style of `VerificationSkipped` / `ArithOverflowReachable`. |
| `compiler/diag.vow` | Add `EC_CODEGEN_UNSUPPORTED() -> 35`, `EC_CODEGEN_FAILED() -> 36`, `EC_LINK_FAILED() -> 37`; add the three arms to `ec_name` (note the nested-`if` chain closes with a matching run of `}` — the count must be updated). |

### Rust compiler

| File | Change |
| --- | --- |
| `vow-codegen/src/lib.rs` | `impl CodegenError { pub fn error_code(&self) -> vow_diag::ErrorCode }`. `vow-codegen` already depends on `vow-diag`, so no `Cargo.toml` edit. |
| `vow/src/verify_outcome.rs` | New smart constructor `codegen_failed(err: &CodegenError, file: &str, diagnostics: Vec<Diagnostic>) -> BuildOutput` next to the existing `compile_failed`: builds the `Diagnostic` from `err.error_code()` + `err.to_string()`, appends it to `diagnostics`, and delegates to `compile_failed`. Single seam for every backend failure path. |
| `vow/src/main.rs` | `link_obj` returns `Result<PathBuf, CodegenError>` instead of `Result<PathBuf, String>` (`CodegenError::Link` for a link failure or a missing `libvow_runtime.a`). The three backend error sites (`backend.compile_module` at ~549, `compiled.write_to_file` at ~559 → `CodegenError::Io`, and the two `link_obj` sites at ~517 and ~585) route through `verify_outcome::codegen_failed`. Also emit the new diagnostic to the human emitter so JSON and text stay in parallel. |

### Self-hosted compiler

| File | Change |
| --- | --- |
| `vow-clif-shim/src/lib.rs` | Add `const CLIF_ERR_UNSUPPORTED: i64 = -3;` and return it (instead of `-1`) from the two `report_narrowed_wide_argument()` sites (~2567, ~2675). `__vow_clif_fn_end` already returns `compile_current_function`'s value verbatim, so it propagates with no other change. |
| `compiler/clif.vow` | `clif_emit_module`: propagate `-3` when `clif_compile_function` returns it (currently collapses every non-zero to `-1`). Contract becomes `0` = ok, `-1` = internal codegen failure, `-2` = link failure, `-3` = unsupported backend operation. |
| `compiler/main.vow` | `build_codegen_phase` gains a `path: String` parameter and maps `-1/-2/-3` to `EC_CODEGEN_FAILED` / `EC_LINK_FAILED` / `EC_CODEGEN_UNSUPPORTED`, pushing a `Diagnostic` into `dctx` (and `eprintln_str(diag_to_human(...))`) before `diag_emit_build_verify_json`. **Also fix the two other `clif_emit_module` call sites that test `result == -1` and would treat `-3` as success**: `run_legacy` (~2357) and `run_test` (~2163). |

### Spec (mandatory — `docs/spec/` is the spec)

| File | Change |
| --- | --- |
| `docs/spec/errors.md` | Three new `###` sections under *Compile-Time Errors*: `CodegenUnsupported` (Phase: Codegen), `CodegenFailed`, `LinkFailed`, each with meaning / example / fix, matching the existing entry shape. |
| `docs/spec/schemas/diagnostic.schema.json` | Append the three names to `properties.error_code.enum`. (`vow-diag`'s `diagnostic_schema_lists_*` tests read this file, so it is load-bearing, not documentation.) |
| `docs/spec/cli.md` | Extend the `CompileFailed` row in *Status Values* (§274) and the `message` row in *Fields Reference* (§360) to name codegen/backend rejection; extend the *Agent Decision Tree* (§582) with a `CodegenUnsupported` branch. |

After any `docs/spec/` edit, run `uv run python scripts/generate_help.py` — it
re-embeds `errors.md`, `cli.md`, and `diagnostic.schema.json` into the
`GENERATE:` blocks of **both** `vow/src/skill.rs` and `compiler/main.vow`. That
regeneration is a large mechanical diff; it must be its own commit inside the PR
so the hand-written changes stay reviewable.

### Tests / fixtures

| File | Change |
| --- | --- |
| `tests/error/<name>.vow` | New black-box fixture (see slice 6) — the artefact the issue exists to unblock. |
| `vow-codegen/src/lib.rs` (`mod tests`) | Table test: every `CodegenError` variant maps to the expected `ErrorCode`. |
| `vow/src/verify_outcome.rs` (`mod tests`) | `codegen_failed` produces exactly one extra diagnostic, error severity, correct code, `executable: None`. |
| `vow/tests/wide_literal_aggregates.rs` | Extend `wide_values_in_aggregates_fail_closed` to assert `diagnostics[0].error_code == "CodegenUnsupported"` — the end-to-end proof the array is no longer empty. |
| `vow-clif-shim/src/lib.rs` (`mod tests`) | Assert `CLIF_ERR_UNSUPPORTED` is distinct from the generic `-1` and is what a narrowed-wide-argument path returns. |

## 3. TDD slices

Each slice is red → green → commit. Slices 1–3 are Rust-only and independently
mergeable; 4–5 are self-hosted; 6 is the cross-compiler payoff.

1. **`CodegenError` → `ErrorCode` mapping.**
   *Test:* `vow-codegen/src/lib.rs` `mod tests`, new
   `codegen_error_maps_every_variant_to_an_error_code` — a table over all seven
   variants asserting `UnsupportedOpcode → CodegenUnsupported`;
   `IsaBuild | FunctionDeclare | FunctionDefine | Emit → CodegenFailed`;
   `Link → LinkFailed`; `Io → IoError`.
   *Production:* the three new `ErrorCode` variants in `vow-diag/src/lib.rs`
   plus `CodegenError::error_code`.

2. **Driver seam.**
   *Test:* `vow/src/verify_outcome.rs` `mod tests`,
   `codegen_failed_attaches_one_structured_diagnostic` — feeds a
   `CodegenError::UnsupportedOpcode` and one pre-existing frontend diagnostic,
   asserts two diagnostics out, the new one last, `Severity::Error`,
   `ErrorCode::CodegenUnsupported`, message equal to the `Display` text, span
   `{file: <source path>, byte_offset: 0, byte_len: 0}`, and
   `BuildStatus::CompileFailed` preserved.
   *Production:* `verify_outcome::codegen_failed`.

3. **Wire the Rust pipeline.**
   *Test:* `vow/tests/wide_literal_aggregates.rs`
   `wide_values_in_aggregates_fail_closed` grows an assertion that
   `json["diagnostics"]` is non-empty and its first entry has
   `error_code == "CodegenUnsupported"`.
   *Production:* `link_obj` returns `Result<PathBuf, CodegenError>`; the four
   backend failure sites in `run_pipeline_inner` call `codegen_failed`; the new
   diagnostic also goes to the human emitter.

4. **Shim carries the "unsupported" bit across FFI.**
   *Test:* `vow-clif-shim/src/lib.rs` `mod tests` —
   `narrowed_wide_argument_returns_the_unsupported_sentinel`, asserting the
   value is `-3` and distinct from the generic `-1`.
   *Production:* `CLIF_ERR_UNSUPPORTED` const + the two return-site edits.

5. **Self-hosted driver emits the diagnostic.**
   *Test:* the existing self-hosted harness has no unit-test seam, so this
   slice is verified by `tests/run_tests.sh --filter <fixture>` (Phase 5) once
   slice 6's fixture lands; write slice 6's fixture first if that ordering is
   more convenient, and keep it failing until this slice is done.
   *Production:* `clif.vow` propagates `-3`; `build_codegen_phase` takes `path`
   and maps kind → `EC_*`, pushes into `dctx`, prints the human form; the
   `run_legacy` and `run_test` `== -1` checks become a full kind dispatch.

6. **Black-box fixture (the deliverable the issue asks for).**
   *Test:* `tests/error/<name>.vow` with
   `// TEST: error-code CodegenUnsupported` and `// TEST: error-count 1`.
   Candidate source (from `wide_values_in_aggregates_fail_closed`):

   ```vow
   module I128VecElement
   fn main() -> i32 {
       let v: Vec<i128> = Vec::new();
       v.push(1);
       0
   }
   ```

   **The implementer must empirically confirm** that both compilers reach
   codegen on this program and reject it there with the same single code — the
   self-hosted frontend's `i128` support is the risk. Probe candidates with
   `target/release/vow build --no-verify` and `build/vowc build --no-verify`
   and compare `diagnostics[].error_code`. If `Vec<i128>` is rejected earlier by
   one frontend, fall back to another `report_narrowed_wide_argument()`-reaching
   shape (`Vec<Option<u128>>` assignment, `HashMap<i64, u128>` insert, struct
   field store of a 128-bit value — all exercised in
   `vow/tests/wide_literal_aggregates.rs:158–207`). If **no** program reaches the
   guard in both compilers, land slices 1–5 (which already close the stated gap:
   non-empty `diagnostics` on codegen failure) and file a follow-up for the
   fixture, saying so explicitly in the PR body.
   *Verification:* `tests/run_tests.sh` Phase 5 (self-hosted `error-code`
   assertion) **and** `scripts/full_test.sh` Section 7 (`compare_error`
   cross-compiler error-code multiset). Both must pass.

7. **(Droppable) `message` stops being `Debug`-formatted.**
   *Test:* new assertion in `vow/tests/wide_literal_aggregates.rs` that
   `message` does **not** start with `UnsupportedOpcode(`.
   *Production:* `format!("{e:?}")` → `format!("{e}")` at `vow/src/main.rs:549`.
   The issue says keeping `message` as-is is acceptable, so this is the last
   slice and can be dropped without weakening the fix. No existing test asserts
   on the `Debug` shape (`wide_literal_aggregates.rs:415` matches a substring of
   the inner string, which survives).

## 4. Verification surface

No change to contracts, the C model, or the ESBMC pipeline. Codegen failures
are reported by the driver *after* the IR is built and are orthogonal to the
verification thread — `run_pipeline_inner` already joins `verify_handle` before
returning on every codegen error path, and that stays.

No new fixture under `tests/run/` or `examples/` is needed: `tests/run/` holds
programs that must compile and execute, and this issue is about programs that
must not. `tests/verify/` and `tests/verify-fail/` are likewise untouched.

The only property the implementation must preserve is the existing fail-closed
one, already asserted by `wide_values_in_aggregates_fail_closed`: a refused
codegen leaves **no** executable behind.

## 5. Risk areas

- **Binary fixed point.** `compiler/main.vow` and `compiler/clif.vow` are
  compiler inputs *and* compiler sources. After editing them, the bootstrap
  triple test must still produce byte-identical stage-2/stage-3 binaries
  (`tests/run_tests.sh` Phase 0). The changes here add only string constants
  and integer comparisons — no map iteration, no `HashMap`-vs-`BTreeMap`
  choice, no stack-slot layout change in `vow-clif-shim` — so the risk is low,
  but the gate is non-negotiable. Note `build/vowc` does **not** exist in this
  workspace: the implementation stage must run `cargo build --release -p vow`
  then `scripts/bootstrap.sh --skip-cargo` before any self-hosted test.
- **`clif_emit_module`'s widened return contract.** Introducing `-3` silently
  breaks any caller that tests `result == -1`. There are exactly three
  (`build_codegen_phase`, `run_legacy`, `run_test`); missing one turns a hard
  codegen failure into a reported success. Grep for `clif_emit_module` and
  audit every call site, not just the one being edited.
- **`ec_name`'s nested-`if` tail.** `compiler/diag.vow:184` closes with a run of
  33 `}` characters that must grow by exactly three. An off-by-one is a parse
  error, not a silent bug, so it fails loudly — but budget for it.
- **Parity gate.** `scripts/parity.py:compare_error` compares the *error-code
  multiset* across compilers. If the two disagree on the new fixture the run
  fails; do not paper over it with a `docs/equivalence/ledger.json` entry —
  that registry is for tracked divergences, and a divergence introduced by this
  very PR is a bug in the PR.
- **`generate_help.py` regeneration.** Skipping it leaves `--help` / the
  embedded skill stale, and `scripts/check_help_coverage.py` (run inside
  `full_test.sh`) can catch drift. Run it, and commit the regenerated blocks
  separately.
- **Clippy.** CI runs `cargo clippy --all -- -D warnings` (no `--all-targets`,
  per `.claude` memory), so lints inside `#[cfg(test)]` modules are not gating —
  but the `link_obj` signature change touches non-test code in `vow/src/main.rs`
  and must be warning-clean.
- **`parse → print → parse` idempotency.** Unaffected: no grammar, AST, or
  printer change.

## 6. Out of scope

- **#1076 — the multi-kilobyte CLIF dump in `message`.** Truncating or
  restructuring the Cranelift verifier's raw output is a separate fix; this PR
  gives it a structured `error_code` (`CodegenFailed`) to sit beside, which is
  what #1078 asks for.
- **Per-instruction spans for codegen diagnostics.** `CodegenError` carries only
  a `String`; threading `vow_ir::Origin` through the backend so the diagnostic
  points at the offending expression is a real improvement and a real refactor.
  This PR emits a file-level span (`byte_offset: 0, byte_len: 0`), matching the
  existing `EsbmcNotFound` precedent in `vow/src/verify_outcome.rs:321`.
- **Splitting `CodegenError` into finer variants** or attaching hints per
  variant. The four-code mapping is the minimum that makes `UnsupportedOpcode`
  branchable.
- **Making the self-hosted driver emit a top-level `message` field.** It emits
  none today for `CompileFailed`; `compare_error` does not compare `message`,
  and adding it is a separate cross-compiler parity task.
- **Extending the `-3` sentinel to every unsupported condition in
  `vow-clif-shim`.** Only the `report_narrowed_wide_argument()` sites are
  reachable from a source program today; the remaining ~40 generic `-1` returns
  stay `CodegenFailed` and can be reclassified when a program can actually
  reach them.
- Formatting passes, unrelated cleanups in `vow/src/main.rs`, and any change to
  `build/` or the `symphonika/` submodule.
