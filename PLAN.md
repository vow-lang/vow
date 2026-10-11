# Plan: reject struct literals that omit declared fields (#1571)

## Goal
Both checkers report `TypeMismatch` ``struct `S` literal is missing field `f` `` (one error per missing field, declaration order, span = the literal) instead of silently zero-initialising. Rust (`vow-types`) and self-hosted (`compiler/`) land together; `tests/error/` fixtures are compared byte-for-byte across both compilers by `scripts/full_test.sh` (`compare_error`, line ~450), so message, code and span must be identical.

## Assumptions
- Error code: reuse `TypeMismatch` (like the sibling "unknown struct" / "has no field" diagnostics of #1267) rather than adding a code: best guess; no new error-code table / `--help` / skill drift.
- One diagnostic per missing field (issue text: "report each missing field"), in declaration order, anchored at the whole literal span (`expr.span` / `expr_span(a, eid)`): best guess; keeps both compilers trivially identical.
- Presence check is set-based on declared fields (iterate declared fields, look each up in the literal's names). Duplicate fields (`S { a: 1, a: 2 }`) are a separate bug and out of scope; the set-based check still reports a genuinely missing field in that case.
- Emitted only when the struct is known (`Some(info)` / `sidx != -1`); the unknown-struct path is unchanged, so no cascade.
- Unknown-field errors and missing-field errors may both fire for one literal (`Foo { y: 1 }` with `Foo { x }`): acceptable, each is true.
- No ADR needed: this tightens an unstated rule; document the rule in the spec.
- Existing in-tree literals that omit fields are real latent bugs and are fixed by naming the field with its old zero value in the same PR. A static scan of `compiler/ tests/ examples/ benchmarks/ bench/` plus manual comparison against declarations found **one confirmed offender**: `compiler/tests/test_vc_cex.vow:13` (`claims_with` builds `VcClaims` without `step` and `hyp_now`, both `bool`, so add `step: false, hyp_now: false`; the `VcClaims` literal at `compiler/vc_exec.vow:505` is complete). Every other scan hit was a false positive (`->` fn signatures, `<`/`>` inside expressions or strings, match patterns such as `tests/error/match_parsed_unsupported_patterns.vow:22`; `VerifyCE` and `IrVowEntry` literals in `compiler/tests/test_diag_verify_json.vow` / `test_verify_report.vow` are complete). The scanner is crude, so Step 6's empirical sweep stays authoritative.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `vow-types/src/check.rs` | Rust checker; `ExprKind::StructLiteral` arm | 3209-3261 (`Some(info)` branch 3231-3259; add missing-field loop before `Ty::Struct(name.clone())` at 3258) |
| `vow-types/src/check.rs` | Rust unit tests for StructLiteral | 7055-7120 (`struct_literal_ok`, `..._wrong_field_type_error`); fixture at 6595-6612 (`Token` literal, complete) |
| `vow-types/src/env.rs` | `StructInfo.fields: Vec<(String, Ty)>` in declaration order | 37-42, `lookup_struct` 692 |
| `compiler/checker.vow` | Self-hosted checker; `EXPR_SLIT` arm | 3833-3892 (field loop 3846-3887; add missing-field loop before `ts_make_struct` at 3891); `emit_no_such_field` 1131 (model for a new `emit_missing_field`) |
| `compiler/checker.vow` | other `EXPR_SLIT` sites (divergence, etc.), no change | 1540, 4311, 4558 |
| `vow-types/src/check.rs` | second `StructLiteral` arm is `expr_diverges` only, no change | 2052 |
| `compiler/tests/test_vc_cex.vow` | confirmed incomplete `VcClaims` literal to fix (`step`, `hyp_now`) | 13-31 |
| `tests/error/struct_literal_unknown_field.vow`, `struct_literal_field_type_mismatch.vow` | fixture template (`// TEST: error-code TypeMismatch`, `// TEST: error-count N`) | whole files |
| `docs/spec/grammar.md` | Struct Literals section | ~950-975 (add the completeness rule) |
| `docs/spec/errors.md` | `### TypeMismatch` section | 157-230 (add missing-field output line) |
| `vow-ir/src/lower/mod.rs` | lowering tolerates missing fields (`field_names.len().max(...)`, 3271-3330); unchanged, now unreachable for checked programs | 3271-3330 |

## Steps (TDD slices, each red then green, Rust then self-hosted)

### 1. Red: Rust unit test for the missing field
- **File**: `vow-types/src/check.rs` tests, next to `struct_literal_wrong_field_type_error` (~7090)
- **Change**: add `struct_literal_missing_field_error`: `Point { x: i32, y: i32 }`, literal `Point { x: 1 }`; assert `has_errors()` and the single diagnostic message is ``struct `Point` literal is missing field `y` `` with code `TypeMismatch`. Add `struct_literal_missing_fields_report_each_in_declaration_order` (literal `Point3 {}`-style with two missing fields, assert two errors in declaration order) and keep `struct_literal_ok` as the green control. Use `TestEmitter` already in scope.
- **Reuses**: `TestEmitter`, `make_expr`, `int_lit` (existing helpers in that module).

### 2. Green: Rust checker
- **File**: `vow-types/src/check.rs` (`Some(info)` arm, after the `for (field_name, field_expr) in fields` loop, before `Ty::Struct(name.clone())`)
- **Change**: `for (declared, _) in &info.fields { if !fields.iter().any(|(n, _)| n == declared) { self.emit_error(ErrorCode::TypeMismatch, format!("struct `{name}` literal is missing field `{declared}`"), expr.span); } }`. Keep the returned type `Ty::Struct(name)` so downstream checking does not cascade.
- **Reuses**: `self.emit_error` (same call shape as the neighbouring "has no field" error at 3251).

### 3. Red: shared error fixtures
- **Files** `[new]`: `tests/error/struct_literal_missing_field.vow` (issue reproducer shape: `Pair { a, b, c }`, literal omits `c`; `// TEST: error-code TypeMismatch`, `// TEST: error-count 1`) and `tests/error/struct_literal_missing_fields.vow` (omits two fields, `error-count 2`; also nests the incomplete literal as another literal's field value to prove each is checked).
- **Run**: neither `target/release/vow` nor `build/vowc` exists in this workspace, so first build the unmodified baseline (`cargo build --release -p vow -j2`, then `scripts/bootstrap.sh --skip-cargo` in the background with a done-marker file). With it, `build/vowc build --no-verify <fixture>` exits 0 (red) before Step 4; Rust goes green after Step 2. To avoid a stale compile cache use `VOW_CACHE_DIR=$(mktemp -d)`.

### 4. Green: self-hosted checker
- **File**: `compiler/checker.vow` (`EXPR_SLIT` arm, just before the `if sidx == -1 { return CTY_UNKNOWN(); }` at 3888, inside a `if sidx != -1` guard)
- **Change**: add `fn emit_missing_struct_field(e: CheckEnv, sname: String, fname: String, span: i64)` beside `emit_no_such_field` (1131), building ``struct `S` literal is missing field `f` `` with `EC_TYPE_MISMATCH()`. In the arm, loop `field_index` over `e.sdef_field_count[sidx as u64]`, read the declared name via `ts_arg_get(e.ts, fname_lid, field_index)` / `ts_local.strs[declared_sid as u64]` (exact idiom of 3857-3860, needs the annotated `let ts_local: TyStore = e.ts;` gotcha), and for each declared name not equal to any literal field name (inner loop over `fields_lid` pairs, `arena_str` compare) call the emitter with `expr_span(a, eid)`. Declaration order matches Rust.
- **Reuses**: `env_emit_error_code`, `emit_unknown_struct`/`emit_no_such_field` patterns, `list_get`/`arena_str`.
- **Size**: if the arm grows past a screenful, extract the loop to a helper `check_struct_literal_complete(e, a, eid, sidx, sname)` (small functions rule); the helper is not a refactor of existing code.

### 5. Spec
- **Files**: `docs/spec/grammar.md` Struct Literals (state: "a struct literal must initialise every declared field; an omitted field is a `TypeMismatch`, never zero-initialised"), `docs/spec/errors.md` TypeMismatch (add the **Output** line ``struct `Pair` literal is missing field `c` `` with a Wrong/Right pair).
- **Then**: `python3 scripts/generate_help.py` only if `scripts/check_help_coverage.py` reports drift (grammar.md change may touch the embedded skill); if it regenerates `vow/src/skill.rs`/`compiler/main.vow`, include them (they must stay in sync in both compilers).

### 6. Fix the confirmed offender, then sweep the corpus for newly rejected literals
- First edit `compiler/tests/test_vc_cex.vow:13` to add `step: false,` and `hyp_now: false,` (declaration order, after `pre_only`), so `build/vowc test compiler/` keeps passing once Step 4 lands.
- Build: `cargo build --release -p vow` (use `-j2`), `scripts/bootstrap.sh --skip-cargo --no-cache` (background + done-marker file, ~5 min).
- Run `cargo test --all` (watch `vow-types` and `vow` crate tests with inline Vow source), `build/vowc test compiler/`, and `scripts/full_test.sh` (~40 min, background) to surface every `.vow` file (compiler/*.vow, compiler/tests/, tests/run, tests/verify*, tests/multi, examples/, benchmarks/, bench/ prompts/skill snippets, docs examples) whose literal is now rejected.
- Fix each by naming the omitted field with the value the old zero-init produced (`0`, `false`, `String::from("")`, `Vec::new()`) so behaviour is unchanged; one line per site, no unrelated edits. If a fixture's intent is "partial literal", it belongs in `tests/error/`.
- Also check `docs/` and the skill text for Vow snippets that omit fields (e.g. `grep -rn` on `docs/spec/*.md` code fences) and fix to keep examples compilable.

### 7. Quality gate
- `cargo fmt --all`, `cargo clippy --all --all-targets -- -D warnings`, `scripts/bootstrap.sh --skip-cargo --no-cache` against the final head SHA (record the SHA), `scripts/full_test.sh`.

## Testing
- Rust: new `vow-types` unit tests (Step 1); `cargo test -p vow-types`.
- Shared: `tests/error/struct_literal_missing_field{,s}.vow`, compared across both compilers by `full_test.sh` (`compare_error`); locally `tests/run_tests.sh` Phase 5 honours `error-code`/`error-count`.
- Self-hosted: `build/vowc test compiler/` stays green (any incomplete test literal is fixed in Step 6). Optional: a `compiler/tests` unit is unnecessary, the `tests/error` fixture is the coverage.
- Regression control: `struct_literal_ok`, `struct_literal_unknown_field.vow`, `struct_literal_field_type_mismatch.vow` still pass with the same counts.

## Verification surface
- No contract, ESBMC model or C-emitter change. A rejected program never reaches IR, so verifier C parity (`c_emitter.{rs,vow}`) is unaffected; `full_test.sh` Section 2c over `tests/verify*/` will still flag any fixture that had an incomplete literal (fixed in Step 6, which changes fixture C text identically in both compilers).
- No new `tests/run/` fixture; error fixtures suffice.

## Risks
- **Latent incomplete literals in the compiler itself** (the very bug class, e.g. #1410): if one exists in `compiler/*.vow`, the new self-hosted check makes stage-1 compile fail at bootstrap. Mitigation: build the Rust compiler with the Rust half first and run it with `--no-verify` over `compiler/` to enumerate offenders before touching `checker.vow`; fix them in the same PR. Bootstrap fixed point (binary identity stage 1 vs 2) is preserved because only checker behaviour changes, not codegen ordering.
- **Rust-vs-self-hosted diagnostic divergence** (`compare_error` fails on any text/span/ordering difference): both iterate declared order, both use the literal's span, identical message text. Verify with one fixture that omits two fields.
- **Generic/other struct-like constructs**: `StructInfo`/`sdef_*` hold only user structs; builtins (`Vec`, `Option`, ...) use `EnumConstruct`/calls, not `StructLiteral`. Struct update/shorthand syntax does not exist (`grammar.md`), so no exemption needed; confirm with `grep -n "\.\." docs/spec/grammar.md` during Step 5.
- **Error-count assertions in existing fixtures**: a fixture that relies on `error-count N` with a literal typo (e.g. `Foo { y: 2 }` replacing `x`) would now get an extra error. The sweep (Step 6) finds these; update the count only if the new diagnostic is legitimate.
- **Codecov patch gate (95%)**: the new Rust branch is covered by the unit tests; the self-hosted arm is covered by the uninstrumented corpus only, so keep the Rust code minimal.
- **Clippy `--all-targets`**: iterator closures in test code must be warning-free; use `any` rather than index loops.
- **Commit/PR**: subject lower-case, e.g. `fix(checker): reject struct literals missing declared fields`; delete `PLAN.md` (`git rm`) before opening the PR.

## Out of scope
- Duplicate-field detection in literals; default field values / `..` update syntax; a new `ErrorCode` variant.
- Hardening lowering (`vow-ir/src/lower/mod.rs`, `compiler/lower.vow`) to ICE on missing fields; it stays tolerant.
- Refactors of the `StructLiteral` arm, formatting, or unrelated test-literal cleanups beyond what the new check forces.
- Mutation-testing shards or benchmark runs.
