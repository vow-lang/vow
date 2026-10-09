# Plan: collect effectful calls inside `break <expr>` in the self-hosted checker

## Goal
Make `collect_calls_in_expr` in `compiler/checker.vow` descend into the `break` value so a pure function cannot perform an effect via `loop { break io_call() }` without an `EffectViolation`. The Rust twin already does this (`vow-types/src/effects.rs:443-447`), so this closes the self-hosted drift.

## Assumptions
- Issue line numbers are stale. The function is now at `compiler/checker.vow:4397-4557` and the `EXPR_RETURN` arm to mirror is at `:4500-4506`. (best guess, verified by grep)
- Only the self-hosted compiler needs a code change. Rust is already correct and has codecov tests at `vow-types/src/effects.rs:~1606-1780` covering `Break`. The dual-compiler rule is satisfied by adding one shared fixture that both compilers run.
- No spec change: behaviour matches the documented rule ("calling an effectful function from a pure one is an `EffectViolation`", `docs/spec/errors.md:257`). `break value` grammar is unchanged.
- The other self-hosted traversals already handle `EXPR_BREAK` (`check_expr` `:3741`, `collect_may_write_sites_in_expr` `:4311`, `:1463`). Only this one is missing.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `compiler/checker.vow` | add `EXPR_BREAK` arm to `collect_calls_in_expr` | 4397; insert right after the `EXPR_RETURN` arm (4500-4506) |
| `vow-types/src/effects.rs` | reference behaviour, no change | 443-447 |
| `compiler/parser.vow` | confirms `break` value lives in `expr_a`, `-1` when absent | ~834-841 |
| `tests/error/break_value_effect_pure_fn.vow` [new] | red/green regression fixture | n/a |
| `tests/error/panic_effect_unwrap_pure_fn.vow` | template for `// TEST: error-code EffectViolation` | 1-12 |

## Steps (TDD slices)

### 1. RED: add the failing fixture
- **File**: `tests/error/break_value_effect_pure_fn.vow` [new], modelled on `panic_effect_unwrap_pure_fn.vow`.
- **Change**: header `// TEST: error-code EffectViolation`, `module break_value_effect_pure_fn`. A pure `fn f() -> i64 { loop { break g(); } }`, where `g() -> i64 [io]` is a user-defined effectful function (user-defined avoids depending on builtin effect tables). `fn main() -> i32 [io] { 0 }`.
- **Confirm red**: `build/vowc build --no-verify tests/error/break_value_effect_pure_fn.vow` currently compiles with no `EffectViolation`. The Rust compiler (`./target/release/vow`) should already report it, which proves the fixture's intent.
- Call `g` from `f` in no other position, so the break value is the only call site.
- Add a second case in the same file only if it needs no separate error-code assertion (each fixture asserts one code). Instead add fixture `tests/error/break_value_effect_nested.vow` [new]: `break` inside a `while`/`if` within the `loop`, value `g()` wrapped in a binop (`1 + g()`), to guard the recursion path.

### 2. GREEN: add the arm
- **File**: `compiler/checker.vow`, after the `EXPR_RETURN` arm (4500-4506).
- **Change**:
  ```
  if tag == EXPR_BREAK() {
      let val_eid: i64 = expr_a(a, eid);
      if val_eid != -1 {
          collect_calls_in_expr(e, m, val_eid, calls);
      }
      return;
  }
  ```
- **Reuses**: identical shape to the `EXPR_RETURN` arm and to `collect_may_write_sites_in_expr`'s `EXPR_BREAK` arm (`:4311`). No comment needed.
- Rebuild: `scripts/bootstrap.sh --skip-cargo` (verification runs on the compiler source; the new arm is a trivial recursion with no contract changes).

### 3. Positive guard (no false positives)
- **File**: `tests/run/break_value_effect_ok.vow` [new].
- **Change**: `main() -> i32 [io]` containing `loop { break g(); }` where `g` is `[io]`, and a pure function with `break 1 + 2` (no call). Expects exit 0 and stdout via `// TEST: exit` / `// TEST: stdout`. Confirms effects are allowed in an effectful caller and pure break values are still accepted.
- Skip if `tests/run/loop_break.vow` already covers the pure case; add only the effectful-caller assertion.

### 4. Run both compilers over fixtures
- Rust: `cargo test -p vow-types effects` (unchanged, regression check).
- Self-hosted fixture runs: `scripts/full_test.sh` Section 4 (`tests/run`) and the error-fixture section; locally `tests/run_tests.sh` for the new `tests/error` and `tests/run` files against `build/vowc` and `target/release/vow`.
- Use `VOW_CACHE_DIR=$(mktemp -d)` when validating after the compiler rebuild (compile cache ignores compiler changes).

## Testing
- New: `tests/error/break_value_effect_pure_fn.vow`, `tests/error/break_value_effect_nested.vow`, `tests/run/break_value_effect_ok.vow`.
- Gates: `scripts/bootstrap.sh --skip-cargo --no-cache`, `cargo test --all` (note sandbox e2e SKIP-panic flakes are environmental), `scripts/full_test.sh` in the background (about 40 min), `build/vowc test compiler/` for compiler unit tests.
- Record the final head SHA beside any "bootstrap green" claim in the PR.

## Verification surface
- No contracts, C emitter, or IR are touched, so no new ESBMC properties and no C-parity impact (`tests/verify*/` untouched).
- The only verified artefact is `compiler/checker.vow` itself, via the bootstrap's verification of the compiler; the new arm adds no `requires`/`ensures`.

## Risks
- Binary fixed point: a new `if` arm in a self-hosted function changes the compiler binary, but deterministically; bootstrap Stage 1/2 must still hash-match. Run the full bootstrap, not just a stage.
- Newly rejected programs: any existing in-tree `.vow` (compiler sources, `examples/`, `benchmarks/`, `tests/run`) that calls an effectful fn inside a `break` value from a pure fn will now fail. Mitigation: bootstrap plus `full_test.sh` surfaces these; fix by declaring the missing effect (correct per spec), not by weakening the check. Grep first: `grep -rn "break [a-z_]*(" compiler/ tests/ examples/ benchmarks/`.
- Recursion into the value could double-report calls for `break` nested in `return` etc.; `calls` is a flat list consumed by `check_effects_fn`, and the same pattern already exists for `RETURN`, so no new risk.
- `parse -> print -> parse` and clippy are unaffected (no Rust change).

## Out of scope
- Any refactor of the `collect_*` traversals (for example a shared expression walker), or merging the three self-hosted collectors.
- Auditing other missing arms in the self-hosted checker (`EXPR_CONTINUE` has no children, so nothing to collect).
- Rust-side changes, spec/`--help` regeneration, and contract-related effects (`collect_may_write_sites`).
