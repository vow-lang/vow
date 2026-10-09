# Plan: refresh narrowing help text and make the NarrowingCastNotAllowed hint truthful (#1516)

## Goal
Close the two items of #1516 that are still open on current `main` (7d42c57e): the "same-width signedness changes use `as`; they are bit reinterpretations, not narrowing" sentence, and the `NarrowingCastNotAllowed` hint that names the nonexistent `{i128,u128}_to_{i64,u64}_try` intrinsics. Both compilers move together; generated help/skill mirrors are regenerated.

## Findings from reconnaissance (state of the tree, not the issue text)
- Issue items 2 and 3 are **already fixed**: `docs/spec/grammar.md:618-637` (introduced by #1552 / 34a6b25f) lists `i128`/`u128` in the `i32` source list and in the `i8`/`i16`/`u16`/`u32` table, and `compiler/main.vow:4455-4474`, `vow/src/skill.rs`, `skills/vow/reference/grammar.md` match it. `python3 scripts/generate_help.py` on a clean tree is a no-op (verified: no diff). Do not re-edit those tables.
- Item 1 is still open: `docs/spec/grammar.md:636-637` says same-width changes "use `as`; they are bit reinterpretations, not narrowing", and the very next paragraph (638-644) says `i128_to_u128_wrap`/`_sat` and `u128_to_i128_wrap`/`_sat` exist. The sentence omits the second mechanism. Verified semantics: `_wrap` = two's-complement reinterpretation (`vow-runtime/src/lib.rs:3186`), `_sat` clamps (`i128_to_u128_sat(-1) == 0`, `u128_to_i128_sat(> i128::MAX) == i128::MAX`, lib.rs:3181/3191).
- Hint bug is real and wider than i128→i64: `cast_verdict` (`vow-types/src/check.rs:708`) returns `Narrowing` for any `tgt_width < src_width`, and the hint is formatted unconditionally (`check.rs:3258-3262`, `compiler/checker.vow:4057-4064`). The only (src,tgt) narrowing pairs with no registered family are `{i128,u128}` → `{i64,u64}` (registered targets are u8,i8,i16,u16,i32,u32 — `vow-types/src/env.rs:403-520`, `compiler/env.vow:440-486`). `Ty::is_integer` covers exactly i8..i128/u8..u128, so no other gaps.

## Assumptions
- Take the issue's second branch ("make the hint name only registered families"), not "land i64/u64 target families" (best guess): landing four new families (i128/u128 → i64/u64, ×3 modes) is a new-feature seam across env, IR lowering, runtime, Cranelift shim, C emitter, catalogue and both compilers; CLAUDE.md "surgical changes" says split it. Record in the PR body that it is deliberately deferred; the grammar.md paragraph at 641-644 already documents the absence.
- Registration is queried from the checker's own function table (`Env::lookup_fn` / `env_lookup_fn`) on the `_try` name rather than a hardcoded (src,tgt) list, so the hint can never drift from the registry again (best guess; a user fn named `i128_to_i64_try` would legitimately be named by the hint).
- Missing-family hint text (identical bytes in both compilers, so `tests/error` JSON parity in `scripts/full_test.sh` stays green):
  `no narrowing intrinsic from {src} to {tgt} exists; narrow to i32 or smaller with `{src}_to_i32_try`, or keep the value as {src}`
  (points at a family that exists: every i128/u128 source has an `i32` family, `env.rs:433-453`). Existing hint for registered pairs is unchanged byte-for-byte.
- The two judgement calls above (hint-only vs landing families; items 2/3 already done) are also posted as an issue comment on #1516.
- Same-width sentence replacement (grammar.md is the source; both `GENERATE:SKILL_*` blocks and `skill.rs` are projections):
  "Every listed source/target pair provides `_try`, `_wrap`, and `_sat`. Same-width signedness changes use `as`, which reinterprets the bits and is not narrowing; `i128`/`u128` additionally have the explicit same-width intrinsics described below."

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `vow-types/src/check.rs` | Rust checker: hint for `CastVerdict::Narrowing` | 3248-3263 (hint at 3258-3262); `cast_verdict` 708; `Env::lookup_fn` use at 1583/2070 |
| `compiler/checker.vow` | Self-hosted twin | 4044-4066 (`EXPR_CAST`), `env_lookup_fn` in `compiler/env.vow:796` |
| `vow-types/tests/narrowing_cast_hint.rs` [new] | Rust integration test of the hint | model on `vow-types/tests/const_integer_types.rs` (CollectingEmitter harness, `Checker::check_module`) |
| `tests/error/i128_narrowing_cast_to_i64.vow` [new], `tests/error/u128_narrowing_cast_to_u64.vow` [new] | Dual-compiler fixtures (error JSON compared by `scripts/full_test.sh` ~L450-460) | model on `tests/error/i32_narrowing_cast.vow` |
| `docs/spec/grammar.md` | Source of truth: reword sentence | 633-637 |
| `docs/spec/errors.md` | `NarrowingCastNotAllowed` section: note i128/u128→i64/u64 has no intrinsic | 184-203 |
| `compiler/main.vow`, `vow/src/skill.rs`, `skills/vow/**` | Generated — only via `scripts/generate_help.py` | main.vow ~4476-4477 and ~10514-10515 |

## Steps (TDD slices, each independently committable)

### 1. Red: Rust hint test
- **File**: `vow-types/tests/narrowing_cast_hint.rs` [new]
- **Change**: helper `hints_for(src)` that checks a module containing `fn f(x: <S>) -> <T> { x as <T> }` and returns the single diagnostic's `(code, hints)`. Cases: `i128→i64`, `u128→u64`, `i128→u64`, `u128→i64` expect `NarrowingCastNotAllowed` and a hint containing `no narrowing intrinsic from` and **not** containing `_to_i64_try`/`_to_u64_try`; regression cases `i64→i32` (hint contains `i64_to_i32_try`), `u128→u32` (`u128_to_u32_try`), `i128→u8` (`i128_to_u8_try`), `u64→u8` keep the old hint. Run `cargo test -p vow-types --test narrowing_cast_hint` — new-family cases fail.

### 2. Green: Rust checker
- **File**: `vow-types/src/check.rs` (3248-3263)
- **Change**: in `CastVerdict::Narrowing`, build `let try_name = format!("{src_ty}_to_{tgt_ty}_try");` and select hint: `self.env.lookup_fn(&try_name).is_some()` → existing text; else the missing-family text. Keep emission via `emit_error_with_hints`. No signature changes.

### 3. Red→green: self-hosted twin + fixtures
- **Files**: `tests/error/i128_narrowing_cast_to_i64.vow` [new], `tests/error/u128_narrowing_cast_to_u64.vow` [new], `compiler/checker.vow` (4057-4065)
- **Change**: fixtures with `// TEST: error-code NarrowingCastNotAllowed` and `// TEST: stderr "no narrowing intrinsic from i128 to i64"` (mirrors `i32_narrowing_cast.vow`; needs a `main`). In `checker.vow`, build `try_name` (`ty_tag_name(src)` + `_to_` + `ty_tag_name(tgt)` + `_try`), call `env_lookup_fn(e, try_name) >= 0` to choose hint; byte-identical strings to Rust. Because `String` is move/consumed in Vow, use `clone_string` where the name is reused (pattern: `env_define_narrowing_family`, `compiler/env.vow:114-127`).
- **Self-hosted unit test**: add `check_narrowing_cast_hint_*` cases to `compiler/tests/test_checker_diag_text.vow` (reuse `checked_diags`/`first_hints`, lines 8-52): `i128 as i64` hint contains `no narrowing intrinsic`; `i64 as i32` hint is still `use a `i64_to_i32_try`...`. Run `build/vowc test compiler/tests/test_checker_diag_text.vow`.
- **Verify**: `build/vowc build --no-verify tests/error/i128_narrowing_cast_to_i64.vow` after rebuild shows the new hint; Rust and self-hosted JSON identical.

### 4. Docs + regenerate
- **Files**: `docs/spec/grammar.md` (reword at 636-637), `docs/spec/errors.md` (add a sentence/line under NarrowingCastNotAllowed **Fix**: `i128`/`u128` → `i64`/`u64` has no narrowing intrinsic yet; see grammar.md §Type Cast)
- **Change**: apply the wording from Assumptions. Then `uv run python scripts/generate_help.py` (rewrites `compiler/main.vow`, `vow/src/skill.rs`, `skills/vow/`), then `uv run python scripts/generate_help.py --check` and `uv run python scripts/check_help_coverage.py docs/spec/grammar.md <help>`.
- **Do not** hand-edit generated blocks.

### 5. Rebuild and gate
- `cargo build --release -p vow`; `scripts/bootstrap.sh --skip-cargo --no-cache` (record head SHA); `cargo clippy --all --all-targets -- -D warnings`; `cargo fmt --all`; `cargo test -p vow-types`; `scripts/full_test.sh` (~40 min: run in the foreground with an explicit `timeout`, no async wakeup exists in this run; it covers `help/skills-dir-drift`, `help` coverage, `tests/error` Rust-vs-self-hosted parity).

## Testing
- New Rust integration test (step 1) and two `tests/error` fixtures (dual-compiler; error JSON parity is enforced by `full_test.sh`).
- Existing `tests/error/{i8,i16,i32,u8,u16,u32}_narrowing_cast.vow` keep asserting `cannot cast X to Y via as` — they must still pass untouched.
- No change to `tests/run/narrowing_matrix_128.vow`.

## Verification surface
No contracts, IR, codegen, or C model change; diagnostics only. ESBMC properties unchanged. Verifier C parity (`c_emitter.rs`/`c_emitter.vow`) untouched. Adding checker.vow code changes the compiled self-hosted binary but not its codegen policy — bootstrap triple still must reach the fixed point.

## Risks
- **Rust/self-hosted hint byte drift** → `scripts/parity.py` compares `hints` on every `tests/error` fixture except codes in `HINT_UNCOMPARED_CODES` (`parity.py:69-90`; `NarrowingCastNotAllowed` is not exempt), so `full_test.sh` fails on drift. Mitigate: copy the exact string; the new fixtures cover the new path.
- **Surfaces not touched**: no parser/printer (`parse → print → parse` unaffected), no `c_emitter.{rs,vow}` or lowering (verifier C parity unaffected), no `vow-clif-shim`/`BTreeMap` ordering. New test file is covered by the `--all-targets` clippy gate.
- **Judgement calls** (hint-only vs landing i64/u64 families; stale issue items 2/3) are recorded under Assumptions.
- **`Ty` Display vs `ty_tag_name`** must both yield `i128`, `u64` lowercase (existing hint already relies on it).
- **Compile cache** serving stale objects after rebuilding the compiler: use `VOW_CACHE_DIR=$(mktemp -d)` when validating.
- **`TEST: stderr` on `tests/error` is only enforced by `tests/run_tests.sh`** (local), not `full_test.sh`; the Rust integration test and JSON parity are the CI-level guards.
- **Pre-existing failures** (concrete-block-region-parity, u64_marker_propagation, contracts_tmp_cleanup, vow e2e SKIP-panics) reproduce on clean main — verify before blaming this change.
- **Snapshot staleness**: bootstrap checklist claims must name the final head SHA (CLAUDE.md).

## Out of scope
- Landing `i128`/`u128` → `i64`/`u64` narrowing families (file/leave a follow-up; grammar.md already states they don't exist).
- Re-editing the i32/u8/table passages (already correct).
- Changing `errors.md`'s quoted "Output:" format for NarrowingCastNotAllowed (pre-existing mismatch with real diagnostics), ADR 0003 wording, `cast_verdict`'s policy, any `LiteralOutOfRange` hints (`check.rs:1974`, `checker.vow:1708`).
