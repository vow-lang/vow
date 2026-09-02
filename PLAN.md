# Plan: #1170 — self-hosted compiler cascades TypeMismatch after missing-module IoError

## 1. Problem restated

When a `use` target cannot be read from disk, the Rust compiler's frontend
(`vow/src/frontend.rs::prepare_frontend_with_root`) returns immediately after
`module_loader::load_modules_with_root` reports the failure, so type-checking
never runs and `diagnostics[]` contains only `IoError`. The self-hosted
compiler's equivalent (`compiler/frontend.vow::frontend_prepare_path_traced`)
has no such gate: `load_frontend_deps` emits the `IoError` diagnostic into the
shared `DiagCtx` but the function falls straight through to `check_module`
regardless. `check_module` then type-checks the root file against an
incomplete symbol table (the missing module's items were never merged in),
and every reference to a symbol that would have come from that module
produces a spurious `TypeMismatch` (or similar) diagnostic layered on top of
the real `IoError`. The fix is a single missing early-exit in the self-hosted
frontend pipeline, mirroring behavior the Rust compiler already has correct.

## 2. Files to touch

**Self-hosted compiler (the actual fix):**
- `compiler/frontend.vow` — `frontend_prepare_path_traced`: skip the
  `check_module` call when `load_frontend_deps` already produced a
  diagnostic-level error.

**Rust compiler (no production-code change expected):**
- `vow/src/frontend.rs` already early-returns on module-load failure
  (`prepare_frontend_with_root`, the `Err(diags) => { ... return Err(...) }`
  arm around `module_loader::load_modules_with_root`). No change needed
  there. Confirm this during implementation before touching anything —
  do not "fix" code that is already correct.

**Regression fixtures (exercise both compilers differentially):**
- `tests/error/` — add one new fixture that reproduces the issue's exact
  repro shape (`use` of a missing module *and* a reference to a symbol that
  would have come from it), so the existing `scripts/full_test.sh` Section 7
  / `scripts/parity.py::compare_error` sweep catches the cascade. The
  existing `tests/error/missing_module.vow` and the inline
  `missing_module.vow` fixture in `scripts/full_test.sh` do **not** catch
  this bug today — neither references a symbol from the missing module, so
  `check_module` never has anything to cascade on. This gap is why the bug
  shipped; the new fixture closes it.

**Docs:**
- `docs/spec/errors.md` — `### IoError` section: add one sentence stating
  that a module-load `IoError` halts the pipeline before type-checking, so
  `diagnostics[]` never contains follow-on errors for code that would have
  resolved through the missing module. This documents an existing (now
  restored) contract, not a new one — no other spec file changes; no syntax,
  builtin, operator, effect, or CLI flag is changing.

**Not touched:** `vow-types`, `vow-ir`, `vow-codegen`, `vow-verify`,
`vow-diag` crates — this is a frontend pipeline sequencing bug, not a type
system or diagnostic-shape change. `compiler/module_io.vow` is unrelated
(that's the `.vmod` binary IR serialization format, not `use`-declaration
module loading — do not confuse the two "module" concepts in this codebase).

## 3. TDD slices

1. **Red: add the differential regression fixture.**
   - File: `tests/error/missing_module_symbol_reference.vow` (or similar
     descriptive name), modeled directly on the issue's repro:
     ```vow
     module MissingModuleSymbolReference

     use nonexistent

     fn main() -> i32 { f() }
     ```
   - Behavior under test: `scripts/full_test.sh` Section 7 sweeps
     `tests/error/*.vow` through both `target/release/vow` and `build/vowc`
     and diffs the sorted `error_code` multiset via
     `scripts/parity.py::compare_error`. Confirm this fixture currently
     fails parity (self-hosted reports `["IoError", "TypeMismatch"]` or
     similar, Rust reports `["IoError"]`) before touching any production
     code — run the section 7 loop (or `python3 scripts/equivalence.py
     tests/error/missing_module_symbol_reference.vow --no-ledger`) against
     both binaries and confirm the divergence is visible.
   - No production code changes in this slice.

2. **Green: gate `check_module` on module-load success.**
   - File: `compiler/frontend.vow`, function `frontend_prepare_path_traced`.
   - Change: after the `load_frontend_deps(root_dir, m, arena, all_items,
     item_files, visited, dctx)` call (and its `trace_span` line), check
     whether that call added any error-severity diagnostics to `dctx`, and
     only call `check_module(e, merged, item_files)` when it did not.
   - Implementation detail worth getting right: `DiagCtx.error_count` is a
     plain `i64` field. Reading `dctx.error_count` (or the existing but
     currently-unused `diag_ctx_has_errors(dctx)` helper in
     `compiler/diag.vow:164`) directly after `load_frontend_deps` returns,
     in the *same* function frame that owns `dctx`, is expected to work —
     this mirrors how `checked.n_errors: dctx.error_count` a few lines later
     (frontend.vow:197) already correctly picks up `check_module`'s own
     error emissions within that same frame. **Verify this empirically as
     the first thing in this slice** by adding a temporary
     print/assertion or just running the red fixture from slice 1. If
     `error_count` does *not* reflect the load-phase error (i.e. the guard
     doesn't trip), fall back to the pattern already proven correct
     elsewhere in this exact file (`frontend_lower_path_traced`,
     frontend.vow:242 and :280-297): snapshot `dctx.diags.len()` before
     `load_frontend_deps`, and after, walk from that index checking
     `severity == SEV_ERROR()`. Prefer the simpler `error_count` check;
     only reach for the diags-diff fallback if the simple check is
     demonstrated not to work.
   - When `check_module` is skipped, `e: CheckEnv` (built via `env_new`)
     keeps its all-empty defaults (`env.vow:197-236` — every `Vec` field
     starts empty). `FrontendPrep`'s pattern-tracking fields
     (`str_eids`, `pat_*`, `question_*`) will just be empty, which is safe:
     every caller of `frontend_lower_path_traced` already gates on
     `checked.n_errors > 0` before touching any of that data (see
     `frontend.vow:243-247`, mirroring the exact same "skip real work,
     return a placeholder" shape this fix now needs one stage earlier).
   - Re-run the slice-1 fixture: expect `build/vowc` to now report only
     `["IoError"]`, matching `target/release/vow`.

3. **Green: full differential sweep, including the issue's original
   13-file corpus-impact list.**
   - Behavior under test: the 13 `compiler/tests/*.vow` files listed in the
     issue are not part of `scripts/equivalence.py`'s default corpus roots
     (`tests/`, `examples/`, `stdlib/`, `benchmarks/`, `euler/` —
     `compiler/` is not in that list), which is why the issue calls this
     "the widened #1081 sweep." No default-corpus config change is needed
     to fix the bug (see "Out of scope" below); this slice is a manual
     verification pass, not a new automated fixture.
   - Run explicitly: `python3 scripts/equivalence.py compiler/tests
     --no-ledger` (after both binaries are rebuilt with the frontend.vow
     fix — `cargo build --release -p vow` and `scripts/bootstrap.sh
     --skip-cargo`). Confirm all 13 files now agree on `error_codes`
     between the two compilers (both `["IoError"]`-only, since none of
     these files carry `// TEST:` directives and none are expected to
     compile standalone).

4. **Refactor / cleanup check (no behavior change expected).**
   - Re-read the final `frontend_prepare_path_traced` for the minimal diff:
     the guard should be a single `if` around the existing `check_module`
     call, not a restructuring of the function. Confirm `checked.n_errors`
     (frontend.vow:197, `dctx.error_count`) is still correct in the
     skipped-check_module case — it should be exactly 1 (the `IoError`)
     for the slice-1 fixture, not 0.

5. **Docs slice.**
   - `docs/spec/errors.md`, `### IoError` section: add one sentence after
     the existing "Meaning" paragraph, e.g. "A module-load failure halts
     the pipeline before type-checking begins, so `diagnostics[]` will not
     contain follow-on errors for code that referenced the missing
     module's symbols." Small, additive, no other spec files affected.

6. **Full quality gate.**
   - `cargo build --all`, `cargo test --all`, `cargo clippy --all -- -D
     warnings`, `cargo fmt --all --check` (Rust side — expect no diffs
     since no Rust production code changed, but the new Rust-visible
     fixture must not break any existing Rust test).
   - `scripts/bootstrap.sh --skip-cargo` (rebuild `build/vowc` with the
     `compiler/frontend.vow` fix).
   - `scripts/full_test.sh` (exercises the new `tests/error/` fixture via
     Section 7, plus the rest of the differential and self-hosted suites).
   - `uv run python scripts/generate_help.py` only if `errors.md` changes
     are picked up by `--help`/skill generation for `IoError` — check
     `scripts/check_help_coverage.py` output; skip regeneration if the
     one-sentence addition isn't help-surfaced text (it documents
     `docs/spec/errors.md`'s prose, not a CLI-flag or schema change, so
     this is likely a no-op, but confirm rather than assume).

## 4. Verification surface

This change does not touch contracts, codegen, the C model, or ESBMC
integration — it is a pure frontend-pipeline sequencing fix (diagnostic
control flow only). No new `vow {}` contracts are introduced, no IR
opcodes change, and no `tests/run/` (execution-behavior) fixtures are
affected. The only "verification" in play is the existing differential
equivalence tooling (`scripts/equivalence.py`, `scripts/parity.py`,
`scripts/full_test.sh` Section 7), which is exactly the mechanism this plan
uses in slices 1–3. No `examples/` growth is needed; this is an error-path
fix, not a new language feature or example-worthy pattern.

## 5. Risk areas

- **Binary fixed point:** `compiler/frontend.vow` participates in the
  self-hosted compiler's own bootstrap triple. The fix is a pure
  control-flow guard (an `if` around one existing call) with no new
  nondeterminism (no new `HashMap`, no new iteration order, no new
  concurrent/parallel path) — low risk to the A/B/C fixed-point hash.
  Still run the full triple-build check
  (`scripts/concat_vow.sh` + stage 0/1/2 + `sha256sum`) as part of the
  quality gate before declaring done, since any self-hosted-compiler change
  is in scope for that check per `CLAUDE.md`.
- **`error_count` propagation assumption:** the plan's slice 2 flags this
  explicitly — Vow struct field-mutation-through-parameter semantics are
  subtle in this codebase (see the existing comment at
  `compiler/frontend.vow:280-287` about `DiagCtx` "value semantics" for a
  documented instance of this exact class of bug). Slice 2's empirical
  verify-first step and documented fallback exist specifically to de-risk
  this; do not skip that check.
- **Over-suppression:** the fix must skip `check_module` on *any*
  module-load error, matching Rust's all-or-nothing behavior exactly (Rust
  aborts before type-checking *anything* in the root file once *any*
  dependency fails to load, even if the root file's own code is otherwise
  fully self-contained and correct). Do not attempt a more "helpful"
  partial type-check that still validates code paths unrelated to the
  missing module — that would be a *new* behavior not present in the Rust
  compiler, reintroducing exactly the kind of parity gap this issue is
  about. Slice 3's full-corpus sweep is the check that this wasn't
  over- or under-applied.
- **`cargo clippy --all -- -D warnings`:** no Rust production code changes
  are planned, so this gate should be a no-op; only re-run it to confirm.
- **`parse → print → parse` idempotency:** unaffected — no AST or printer
  changes.

## 6. Out of scope

- **Expanding `scripts/equivalence.py`'s default corpus roots to include
  `compiler/`.** The issue's 13-file list came from a deliberately widened,
  one-off sweep (per #1081), not the standing nightly/CI corpus. Adding
  `compiler/tests/` (or `compiler/`) to the default roots is a CI-policy
  decision with its own blast radius (every file in `compiler/` would need
  to either compile standalone or carry a `// TEST:` directive), and
  belongs to whatever follow-up tracks the #1081 sweep, not this bug fix.
- **A general "cascading diagnostics after any error" audit.** While
  investigating this issue, note that `frontend_prepare_path_traced` also
  does not currently gate `load_frontend_deps`/`check_module` on a *parse*
  error in the root file itself (compare Rust's explicit `if parse_failed
  { return Err(...) }` before module loading even starts, `vow/src/
  frontend.rs:183-190`). This may or may not be a live bug — the self-hosted
  parser's error-recovery behavior on the root file needs its own
  investigation to confirm whether it actually cascades. That is a distinct
  potential root cause from this issue's missing-module case and should be
  filed as its own issue if confirmed, not folded into this fix.
- **Rust-side production code changes.** `vow/src/frontend.rs` is already
  correct; do not refactor or "clean up" it as part of this PR.
- **Refactoring `frontend_prepare_path_traced` beyond the minimal guard**
  (e.g. introducing a Result-like early-return idiom into the self-hosted
  compiler's frontend layer generally). The codebase's established
  convention here is "always build the result struct, gate on an error
  count downstream" (see every `if checked.n_errors > 0` call site in
  `compiler/main.vow`); this fix follows that convention rather than
  introducing a new control-flow idiom.
