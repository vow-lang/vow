# Plan: desugar every integer-literal suffix to a typed cast (#627)

## Goal
Close the remaining half of #627: the lexer already rejects `usize`/`isize` (#1145) and the parser already desugars
`u64`/`i128`/`u128` suffixes to `Cast(Lit, T)`, but `i8 i16 i32 i64 u8 u16 u32` still collapse to an unsuffixed
`Lit::Int` in BOTH compilers, so `256u8` is accepted and `5u8` is retyped to a context-coercing marker. Make all 10
suffixes behave like the already-working three, in both compilers, with the spec (already claiming "all 10 widths") now true.

## Assumptions
- Option (b) of the issue (typed literals), implemented as parser desugaring `NNNuT` → `NNN as T`; option (a) is moot (grammar.md already documents all 10 suffixes, so rejecting them would be a language regression). Best guess; no new AST field → no new type-system axis, near-zero verifier impact (CLAUDE.md "crisp rule").
- Canonical printed form stays `NNN as T` (cli.md:539 already states "a suffixed literal prints as the cast it denotes"). `100u64` and `100 as u64` remain one AST — documented, not changed.
- Why desugar is sound: `check_expr` types a bare literal as `Ty::LitInt`; `cast_verdict(LitInt, int)` = `LiteralRange` (vow-types/src/check.rs:713-716), so `256 as u8` yields `LiteralOutOfRange`, never `NarrowingCastNotAllowed`.
- Pattern-position suffixes (`match x { 5u8 => .. }`) are OUT of scope: Rust parser accepts only i128/u128 suffix in patterns (vow-syntax/src/parser/types.rs:173-176), self-hosted accepts all and discards (compiler/parser.vow:1232-1234). Pre-existing divergence; file a follow-up issue.
- `-128i8` must stay valid: lexer yields magnitude 128, so `Neg(Cast(128, i8))` must not report out-of-range. Checker's existing carve-out is i128/u128-only and must be widened to all signed integer targets.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `vow-syntax/src/parser/expr.rs` | `parse_prefix` suffix desugar | 177-212 (`matches!(suffix, U64\|I128\|U128)`) |
| `compiler/parser.vow` | self-hosted twin | 966-980 |
| `vow-types/src/check.rs` | `UnaryOp` Neg-of-Cast carve-out (i128 only); `Cast` arm; `cast_verdict` | 2381-2402; 3241-3282; 713 |
| `compiler/checker.vow` | twin carve-out; `integer_literal_out_of_range` | 3096-3114; 1612-1636 |
| `vow-ir/src/lower/mod.rs` | Cast-of-literal lowering (Neg = WrappingSub, so `-(128 as i8)` = -128 OK) | 4151-4210; 1901-1928 |
| `compiler/lower.vow` | twin cast lowering / wide-literal helpers | ~2452, 5116 |
| `vow-syntax/src/printer.rs`, `compiler/contract_text.vow` | already print casts; verify only | printer.rs:689 |
| `docs/spec/grammar.md` | §Integer literals | 245-261 |
| `docs/spec/cli.md` | literal printing | 539 |
| `compiler/main.vow` | embedded help/skill text generated from spec | via `scripts/generate_help.py` |
| `vow-syntax/src/parser/mod.rs` | parser unit tests (existing suffix tests at ~836-853) | |
| `tests/run/*.vow` using `1u8`,`7u32`,`1i32`,`100000i64`,… | now strictly typed; must still pass | alias_non_linear_lowering, nested_option_payload_width, short_circuit:13 (`return 1i32`), vec_narrow_index_*, vec_index_narrow_arith, vec_oversized_release_on_grow, (un)signed_zero_comparisons |

## Steps (TDD slices, ordered)

### 1. RED: Rust parser unit tests
- **File**: `vow-syntax/src/parser/mod.rs` tests (next to the `InvalidIntSuffix` tests ~836).
- Add a table test over all 10 suffixes: `fn f() -> T { 42<suf> }` parses to `ExprKind::Cast{ Lit::Int(42), Named(T) }`; and parse→print→parse idempotency (`42u8` prints `42 as u8`).
- Fails today for the 7 narrow suffixes.

### 2. GREEN: Rust parser desugar
- **File**: `vow-syntax/src/parser/expr.rs:177-212`.
- Replace the `matches!(U64|I128|U128)` + `unreachable!` with a total `fn suffix_type_name(IntSuffix) -> Option<&'static str>` (all 10 → name; `Usize|Isize` → `None`, unreachable in practice since lexer rejects them) and always wrap in `Cast`. Single deep seam; delete the else branch. Reuse: `unrepresentable_suffix_name` in `vow-syntax/src/lexer.rs:14` is the complementary function — place the new mapper beside `IntSuffix` in `vow-syntax/src/token.rs:127` (`IntSuffix::type_name`) and let the lexer helper stay as-is.

### 3. RED→GREEN: type-check behaviour (Rust)
- **File**: `vow-types/tests/` new `suffixed_literal_types.rs` [new] (pattern: `vow-types/tests/const_integer_types.rs`).
- Cases: `256u8`, `128i8`, `32768i16`, `65536u16`, `2147483648i32`, `4294967296u32`, `9223372036854775808i64` → `LiteralOutOfRange`; boundaries `255u8`, `127i8`, `-128i8`, `-32768i16`, `-2147483648i32`, `-9223372036854775808i64`, `4294967295u32` accepted; `-1u32` / `-1u8` → unary-negation-on-unsigned `TypeMismatch` (issue's "-1 via u32"); `let x: i64 = 5u8;` → `TypeMismatch` (suffix now fixes the type); `let x: u8 = 5i64` mismatch; `let x: u64 = 5usize` still `InvalidIntSuffix`.
- **Code**: `vow-types/src/check.rs:2381-2402` — generalize the `UnOp::Neg` over `Cast(Lit, T)` arm from `name == "i128"` to every signed integer target (`i8 i16 i32 i64 i128`): call `check_integer_value_range(ConstIntValue{magnitude, negative:true}, &ty, operand.span)` and return that type; keep the `u128 && magnitude==0` arm and extend it to all unsigned types (`-0u8` is `0`; check what the u128 arm's intent is and mirror it consistently). Use `Ty::from_primitive_name(name)` rather than hard-coded `Ty::I128`.

### 4. RED→GREEN: self-hosted parser + checker
- **Tests**: `compiler/tests/test_lexer_dot_and_suffix.vow` (extend; existing line 62 lexes `5i8 5u8`) and a new parser/checker test `compiler/tests/test_parser_suffix_literals.vow` [new] mirroring step 1/3 assertions (`build/vowc test compiler/tests/<file>`; `--filter`).
- **Code**: `compiler/parser.vow:966-980` — replace the 3-way condition with a `suffix_type_name(suffix: i64) -> String` helper (empty string = none) covering the 10 suffixes; always emit `EXPR_CAST`. `compiler/checker.vow:3096-3114` — widen `is_signed_wide` (`target_tag == CTY_I128()`) to all signed CTY tags (reuse the existing signedness helper, e.g. `ty_is_unsigned` negated / `integer_tag_range(..).has_negative`) and the u128-zero case likewise. `integer_literal_out_of_range` (1612) already unwraps `Neg(Cast)`; verify.

### 5. End-to-end fixtures (both compilers)
- `tests/error/u8_suffix_out_of_range.vow`, `i8_suffix_out_of_range.vow`, `u32_suffix_negated.vow` [new] with `// TEST: stderr "LiteralOutOfRange"` / TypeMismatch (follow the layout of `tests/error/u8_literal_out_of_range.vow`; `TEST: stderr` on `tests/error` is only checked by `tests/run_tests.sh`, so also cover with the Rust/vow-types tests above).
- `tests/run/suffixed_literal_widths.vow` [new]: one value per suffix incl. extremes (`-128i8`, `255u8`, `-9223372036854775808i64`, `4294967295u32`), arithmetic mixing suffixed and unsuffixed (`x + 1u8` with `x: u8`), exit-code/stdout assertions. Runs through `full_test.sh` Section 4 on both compilers.
- Re-run the existing corpus fixtures listed in Key Files; fix any that relied on suffix-as-marker coercion (expected: none; all pass a value of the matching type — verify `short_circuit.vow:13` `return 1i32` against its function's return type; if the fn returns `i64` the fixture was wrong and must be corrected to `1`/`1i64`).

### 6. Spec + embedded help
- `docs/spec/grammar.md:245-261`: state that a suffix is sugar for `NNN as T` (canonical printed form is the cast), is range-checked (`256u8` → `LiteralOutOfRange`), fixes the type (no context coercion), `-128i8` is valid, `-1u32` is a negation-of-unsigned error; keep the usize/isize paragraph. Note pattern-position suffix handling is unchanged.
- `docs/spec/errors.md` `LiteralOutOfRange`: add a suffixed example if the section lists examples.
- Regenerate: `uv run python scripts/generate_help.py`, then `cargo build --release -p vow`, `scripts/bootstrap.sh --skip-cargo`; `scripts/check_help_coverage.py`.

### 7. Parity/regression gates
- `python3 scripts/parity.py c ./target/release/vow build/vowc <new tests/run fixture>`-style check: new fixture must produce byte-identical verifier C in both compilers (Section 2c iterates `tests/verify*/`; add a `tests/verify/suffixed_literals.vow` [new] so the gate covers it).
- Run order: `cargo test -p vow-syntax`, `cargo test -p vow-types`, `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all`, `cargo test --all`, `scripts/bootstrap.sh --skip-cargo --no-cache` (record head SHA), `build/vowc test compiler/`, `scripts/full_test.sh` (~40min; background + poll in the foreground-bounded way).

## Testing
Unit: step 1, 3, 4. Fixtures: step 5, 7. Gates: step 7. Parse→print→parse idempotency test in step 1.

## Verification surface
No contract changes. Surface: `x as u8` of a literal in verified functions lowers to `IntCast` from an i64 const (Rust lower/mod.rs:4194-4208; self-hosted equivalent) for i8/i16/i32/u16/u32 — previously suffixed forms produced a bare i64 marker, so new IR/C shapes reach `c_emitter.{rs,vow}` for narrow widths. ESBMC must prove nothing new; the concern is only that both emitters stay byte-identical (step 7) and that `IntCast` of an in-range constant raises no spurious overflow assertion (existing `5 as u8` tests prove it for u8; add i8/i16/i32/u16/u32 coverage in the `tests/verify` fixture). `-128i8` lowers as `0 - (-128 as i8)` via `WrappingSub` — no checked-neg overflow assertion; confirm in fixture.

## Risks
- Behaviour change: `5i64` etc. no longer context-coerce (`let x: u8 = 5i64` becomes an error; `x + 1i32` with `x: i64` becomes mismatch). Correct per issue; corpus fixtures audited in step 5. Compiler sources under `compiler/` contain no narrow-suffix literals outside `compiler/tests/test_lexer_dot_and_suffix.vow` (lexer-level only), so bootstrap fixed point is unaffected by source churn; still run triple-bootstrap.
- Negated min values (`-128i8`, `-9223372036854775808i64`): without step 3/4 carve-out they falsely report out-of-range. i64 min: confirm `ConstI64(*v as i64)` wrap in IntCast-to-i64 path (cast i64←LitInt lowers as `IntCast{from:i64,to:i64}` on a const holding the wrapped magnitude) yields correct value; add to run fixture.
- `Ty::from_primitive_name` / Vow `resolve_ast_ty` on the synthetic `Type::Named` span: spans are the literal span (as for u64 today) — diagnostics keep pointing at the literal.
- Binary fixed point / C parity: both parsers must emit structurally identical ASTs (Cast node, literal span) — contract_text/`vow contracts` offsets use spans; check `description` text for `requires: x == 5u8` equals in both compilers.
- Clippy: replacing `unreachable!()` branch with exhaustive match; keep `-D warnings` clean. Codecov patch gate (95%): all new parser lines are exercised by the table test.
- Squash-merge PR title must be lower-case conventional: `fix(parser): type every integer-literal suffix instead of dropping narrow ones`.

## Out of scope
- Pattern-position literal suffixes (Rust accepts only 128-bit, self-hosted accepts-and-discards) → follow-up issue.
- Adding a `suffix` field to `Lit::Int` or changing canonical printing (`100u64` vs `100 as u64`).
- Lexer changes (usize/isize already rejected; no lexer edits needed).
- Unrelated literal-overflow refactors, formatting, other audit findings.
