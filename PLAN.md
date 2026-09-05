# Plan: fix #1244 — self-hosted checker rejects f64 let-bindings/comparisons against bare float literals

## 1. Problem restated

The self-hosted compiler never actually implements float-literal lexing: `compiler/lexer.vow`'s
digit-scanning branch (the `is_digit(b)` arm inside `lex()`, ~lines 269–300) only recognizes an
integer digit run (optionally followed by an integer suffix like `u64`/`i128`); it has no check for
a trailing `.` followed by more digits. So the two-character-class shape `1.5` lexes as **three**
tokens — `tok_lit_int(1)`, `tok_dot`, `tok_lit_int(5)` — never as a single float-literal token. The
`EXPR_LIT_FLOAT()` AST tag (`compiler/ast.vow:6`) is fully wired up downstream (`checker.vow`'s
`EXPR_LIT_FLOAT()` arm correctly returns `CTY_F64()`; `lower.vow`'s arm correctly reads `bits` from
`expr_a` and emits `IOP_CONST_F64`) but is **never constructed**, because `compiler/parser.vow`'s
`parse_primary` has no arm for a float token (there isn't one to have an arm for).

The actual failure mode traces further: in `parser.vow`'s postfix-chain loop (~lines 793–808), after
parsing the leading `1` as `EXPR_LIT_INT`, the loop sees `tok_dot()`, unconditionally advances past
it, then checks `at(p, tok_ident())` for a field/method name — false, since the next token is the
orphaned `tok_lit_int(5)`, not an identifier — so it sets `cont = false` and returns, having already
consumed the `.` but stopped before the `5`. The `let`-statement parser is left with just
`EXPR_LIT_INT(1)` as the initializer. `checker.vow`'s `STMT_LET` arm (line 1573) then correctly
rejects `is_coercible(CTY_LIT_INT(), CTY_F64())` — literal-int-to-float is not a coercion Vow
supports — producing exactly the reported `TypeMismatch`/"let binding type mismatch". **`checker.vow`
has no bug and needs no change; the entire defect is the lexer never producing a float token in the
first place**, with the parser's dot-then-non-ident bailout as the visible secondary symptom.

Fixing the lexer surfaces a second question and a second, previously-dormant bug:

- **How does a pure-Vow lexer get an IEEE-754 bit pattern from decimal text?** Vow has *no*
  int↔float conversion at all (`docs/spec/grammar.md`'s Type Cast section only covers integer
  widening and same-width int/uint bit-reinterpretation). This must go through a small new runtime
  FFI intrinsic, following the exact precedent already used for `__vow_clif_create`,
  `__vow_string_eq`, `memory_root_arena_bytes`, etc.: a `pub extern "C"` function in
  `vow-runtime`, registered as a bodyless builtin in both compilers' checker environments
  (`vow-types/src/env.rs` and `compiler/env.vow`), which the existing generic "registered-but-no-body
  → extern call by that name" lowering fallback (`compiler/lower.vow:2637`, already exercised by
  every `__vow_clif_*` call) picks up with zero additional lowering code.
- **`compiler/c_emitter.vow`'s `IOP_CONST_F64`/`IOP_CONST_F32` handling is itself buggy** (~lines
  1333–1342): it emits `v<id> = <dv as decimal integer>;`, i.e. it prints the raw i64 bit pattern as
  if it were the literal value, instead of reinterpreting it back into an actual double. Contrast
  `vow-clif-shim/src/lib.rs:2064`, which correctly does `f64::from_bits(dv as u64)` before emitting a
  Cranelift `f64const`, and the Rust `vow-verify/src/c_emitter.rs:991-995`, which never has this
  problem because its `InstData::ConstF64` already stores the real `f64` value, not bits. This bug is
  currently **unreachable** — no self-hosted-lexed program has ever produced an `EXPR_LIT_FLOAT`
  node — but becomes live the moment the lexer fix lands, for any `vowc verify`/default `vowc build`
  (verify defaults to on) over a program with a float literal. The issue's own repro uses
  `--no-verify` (Cranelift path only, already correct), so this half of the bug is invisible in the
  reported repro but must be fixed for the feature to actually work end-to-end. The identical bug
  exists in `compiler/ir_printer.vow`'s `IDATA_CONST_F32`/`IDATA_CONST_F64` text (~lines 207–208),
  used for human-readable `--dump-ir` output. (`scripts/full_test.sh`'s `--dump-ir` parity section
  only dumps `compiler/main.vow`, which has no float literals, so it won't happen to exercise this —
  see §3 slice 6 for the actual regression coverage.)

This is a **self-hosted-only parity bug**: the Rust compiler (`vow-syntax/src/lexer.rs:286-307`)
already lexes `digit+ '.' digit+` correctly, and its `vow-verify/src/c_emitter.rs` already formats
float constants correctly. No `docs/spec/*.md` semantic change is needed (float literals are not a
new language feature — they already exist per the Rust compiler's behavior), **except** that float
literal syntax was never actually documented in `docs/spec/grammar.md` at all; since this PR makes
the feature real, robust, and tested in both compilers, it should also close that pre-existing
documentation gap with a small addition.

## 2. Files to touch

**Rust side (`vow-runtime`, `vow-types`, `vow-codegen`) — new runtime intrinsic, registered so both
compilers can typecheck *and codegen* the self-hosted compiler's own use of it:**
- `vow-runtime/src/lib.rs` — add `__vow_parse_f64_bits(s: *const u8) -> u64` (VowVec/String-descriptor
  ABI, same pattern as `__vow_string_eq`/`__vow_string_contains`: slice the bytes, `std::str::from_utf8`,
  `.parse::<f64>()`, `.to_bits()`) and `__vow_format_f64_bits(bits: u64) -> *mut u8` returning a new
  Vow `String` (same construction pattern as other String-returning runtime fns already in this file)
  built from `f64::from_bits(bits).to_string()`. These are the *only* two new runtime symbols needed;
  everything else is a consumer of one or the other.
- `vow-types/src/env.rs` — register both signatures in the builtin table (near the existing
  `__vow_clif_*`/`memory_*`/`hex_encode` registrations, ~lines 240-340): `__vow_parse_f64_bits(Str) ->
  U64` and `__vow_format_f64_bits(U64) -> Str`. **Register both with an empty effect set (`&[]`), not
  `&[Effect::IO]`.** Verified: `compiler/lexer.vow`'s `fn lex(src: String) -> Vec<Token>` and
  `compiler/c_emitter.vow`'s `fn emit_inst(...)` (the callers) carry no `[io]`/effect annotation —
  they are pure functions — so an effectful callee here would be a hard type error ("calling an
  effectful function from a pure one"), not a warning. `hex_encode`'s existing registration
  (`&[]`) is the precedent to copy, not `__vow_clif_create`'s (`&[Effect::IO]`).
- `vow-codegen/src/cranelift_backend.rs` — **this file was missed in the first pass of this
  investigation and is required, not optional.** Unlike `compiler/lower.vow`'s self-hosted fallback
  (generic, name-keyed, needs no new entry), `vow-codegen`'s Cranelift backend builds each extern
  call's `Signature` from a hardcoded `match` on the symbol string (confirmed: `"__vow_clif_create"`,
  `"__vow_hex_encode"`, etc. each have their own arm, ~lines 2800-2975; there is no generic fallback
  here). Add two new arms — `"__vow_parse_f64_bits"` (one `I64` param for the string ptr, one `I64`
  return for the bits) and `"__vow_format_f64_bits"` (one `I64` param for the bits, one `I64` return
  for the string ptr) — matching the exact `AbiParam`/comment style of the neighboring
  `"__vow_hex_encode"`/`"__vow_hex_decode"` arms. Without this, `cargo build --release -p vow`
  cannot bootstrap-compile a `lexer.vow`/`c_emitter.vow` that calls either new intrinsic — it is not
  a self-hosted-only file list. (Confirmed `vow-ir` has no equivalent per-name list — only
  `vow-types` and `vow-codegen` need entries on the Rust side.)

**Self-hosted compiler (`compiler/`) — the actual bug fix:**
- `compiler/token.vow` — add `fn tok_lit_float() -> i64 { 82 }` (next free tag after
  `tok_invalid_int() = 81`). No `Token` struct field changes: reuse the existing `int_lo: u64` field
  (already used for wide-integer literals) to carry the f64 bit pattern, via the existing
  `make_wide_int_token(tag, lo, hi, suffix, start, len)` constructor with `hi = 0u64`, `suffix = 0`.
- `compiler/lexer.vow` — in the `is_digit(b)` branch of `lex()` (~line 269), after the existing
  integer digit-scanning loop and *before* the existing suffix/overflow handling, add the float
  check mirroring `vow-syntax/src/lexer.rs:291-307` exactly: if the current byte is `.` (46) and the
  *next* byte is a digit, consume `.` then a further digit run, slice `src[start..pos]` as the
  literal's text, call `__vow_parse_f64_bits(text)` for the bits, and push a `tok_lit_float()` token.
  Otherwise fall through to the existing integer-literal logic unchanged. This ordering guarantees
  `wide_overflow`/suffix tracking from the leading digit run is simply discarded for the float case
  (matching Rust: it never consults the int accumulator once it commits to the float branch). Guard
  the lookahead as `pos + 1 < src_len && src.byte_at(pos) == 46 && is_digit(src.byte_at(pos + 1))`
  before consuming — do not read `byte_at(pos + 1)` unguarded (mirrors Rust's
  `peek_byte(1).is_some_and(...)`, and matches this file's existing `pos < src_len` guard used before
  `try_suffix`).
- `compiler/parser.vow` — in `parse_primary` (~line 856, alongside the `tok_lit_int()` /
  `tok_lit_int_suffixed()` arms), add an arm for `tok_lit_float()` that advances past the token and
  emits `arena_add_expr(p.arena, EXPR_LIT_FLOAT(), t.int_lo as i64, 0, 0, <span, matching the sibling
  literal arms' convention>)`.
- `compiler/checker.vow` — **no change**. The `EXPR_LIT_FLOAT()` arm (line 2259) is already correct;
  confirmed by direct reading.
- `compiler/lower.vow` — **no change**. The `EXPR_LIT_FLOAT()` arm (line 2188) is already correct;
  confirmed by direct reading.
- `compiler/env.vow` — register `__vow_parse_f64_bits` (used by `lexer.vow`) as a builtin function
  (mirroring the `__vow_clif_*` registration block, ~lines 485-557, same `env_define_fn` idiom), with
  effect `0` (pure) — the `env_define_fn(e, name, params, ret, eff)` calls in that block use
  `eff_io` because every `__vow_clif_*` callsite is inside `[io]`-annotated code in `clif.vow`; this
  intrinsic's callers (`lex()`, `emit_inst()`) are pure, so `eff_io` here would break the build.
  `__vow_format_f64_bits` is registered here too (also effect `0`) since `c_emitter.vow`/
  `ir_printer.vow` call it.
- `compiler/c_emitter.vow` — fix the `IOP_CONST_F64` arm (~line 1338) to call
  `__vow_format_f64_bits(inst.dv as u64)` and emit that text directly as the C double-literal RHS,
  instead of `i64_to_string(inst.dv)`. (`IOP_CONST_F32` at ~line 1333 has the identical bug but is
  currently unreachable — no F32 literal syntax exists to trigger it — see §6 Out of scope.)
- `compiler/ir_printer.vow` — same fix for the `IDATA_CONST_F64` arm (~line 208): call
  `__vow_format_f64_bits` instead of `i64_to_string(inst.dv)`, for correct `--dump-ir` text and the
  Rust/self-hosted `--dump-ir` parity check in `scripts/full_test.sh` (~line 510).

**Docs:**
- `docs/spec/grammar.md` — add a short "Float Literals" note (near the existing `f32`/`f64` type
  table entries, ~line 136) documenting `digit+ '.' digit+` syntax (no exponent, no leading/trailing
  bare dot, no `f32`/`f64` suffix — matches exactly what the Rust lexer already accepts and what this
  fix makes the self-hosted lexer accept).

**Tests (new files):**
- `compiler/tests/test_lexer_float_literal.vow` — lexer-level unit test (same style as the existing
  `compiler/tests/test_lexer_dot_and_suffix.vow` and `test_wide_literal_lexer.vow`): asserts
  `lex("1.5")` produces exactly one `tok_lit_float()` token with the right span and the right
  `int_lo` bits (compare against a known bit pattern, e.g. `1.5f64.to_bits()` computed once and
  hard-coded, or round-trip it through `__vow_format_f64_bits` and compare strings); asserts
  `lex("1.abs()")`-shaped input (`digit '.' ident`) still lexes as `INT DOT IDENT LPAREN RPAREN`
  (i.e. the fix must not touch the "dot not followed by digit" case at all — this is the regression
  guard for the existing postfix/method-call/tuple-adjacent-dot behavior;
  `test_lexer_dot_and_suffix.vow`'s `check_dot_never_pairs` must keep passing unmodified).
- `compiler/tests/test_checker_float_literal.vow` — end-to-end unit test per the issue's own suggested
  name: `lex` → `parse_module_into` → `lower_module_vow` (or `check_expr`/`CheckEnv` directly, matching
  the style of `test_lower_float_binop.vow`) on the issue's exact repro (`let z: f64 = 1.5; a < z`,
  plus the `a + 0.0` arithmetic variant called out in the issue body), asserting zero diagnostics and
  the initializer's type is `CTY_F64()`. Also directly call the `ir_printer.vow`/`c_emitter.vow`
  formatting helper introduced in this PR (whatever it ends up named, e.g. a function wrapping
  `__vow_format_f64_bits`) on the lowered `IOP_CONST_F64` instruction's `dv` and assert the resulting
  text is `"1.5"`, not the raw bit pattern. This is the *only* planned regression coverage for the
  `ir_printer.vow`/`c_emitter.vow` fix's textual output — see §3 slice 6 for why the
  `scripts/full_test.sh` `--dump-ir` section does not exercise it.

**Tests (parity/integration, exercised by `scripts/full_test.sh` and CI):**
- `tests/run/float_literal_binding.vow` — a small program matching the issue's `MiniFloatLit` repro
  (comparison + arithmetic against a bare float literal), added to the `tests/run/*.vow` set so
  `full_test.sh`'s Section "Build --no-verify" compares Rust vs self-hosted JSON output for it
  automatically (this is the issue's literal repro, now regression-protected).
- `tests/verify/float_literal_contract.vow` — a small function using a float-literal comparison,
  added to `tests/verify/*.vow` so the default (verifying) `vow verify`/`vow build` path is exercised
  through `c_emitter.vow` on both compilers and compared for parity — this is what actually exercises
  and protects the `c_emitter.vow` fix (§1's second bug); without a fixture here, that half of the fix
  has no regression coverage at all.

## 3. TDD slices

Each slice is a small, independently-reviewable red→green step. **Bootstrap ordering matters and
`--skip-cargo` is only valid starting at slice 3**: slices 1-2 change `vow-runtime` and
`vow-types`/`vow-codegen` (Rust crates), so the *first* rebuild after them must be a full
`cargo build --release -p vow` (or `scripts/bootstrap.sh` without `--skip-cargo`) — reusing a stale
`./target/release/vow` would either fail to typecheck the new call ("undefined function", stale
`env.rs`) or fail to link (stale `vow-runtime`, missing symbol). Only once that full rebuild has
happened does `scripts/bootstrap.sh --skip-cargo` become valid for the remaining self-hosted-only
slices. Use `./target/release/vow test compiler/` / `build/vowc test compiler/` to run every
`compiler/tests/test_*.vow` (this is the actual runner — confirmed via `vow test <dir>`'s recursive
`test_*.vow` discovery, invoked by `scripts/full_test.sh`'s "Test Subcommand" section; there is no
other harness for these files).

1. **Runtime intrinsic, red→green via a throwaway probe.** Add `__vow_parse_f64_bits` and
   `__vow_format_f64_bits` to `vow-runtime/src/lib.rs`. Red: `cargo test -p vow-runtime` has no test
   yet exercising them (there's nothing to be red against) — instead, write a small `#[cfg(test)]`
   unit test in the same file asserting `__vow_parse_f64_bits` / `__vow_format_f64_bits` round-trip a
   couple of known values (`"1.5"` ↔ `1.5f64.to_bits()`, `"0.0"` ↔ `0.0f64.to_bits()`) — this is the
   only new Rust-side behavior and deserves a direct Rust test, not just downstream Vow coverage.
   Green: `cargo test -p vow-runtime`.
2. **Register the intrinsic everywhere the Rust side needs it.** Add the `vow-types/src/env.rs`
   registration (pure/`&[]`, per §2), the `vow-codegen/src/cranelift_backend.rs` ABI match arms, and
   the mirrored `compiler/env.vow` registration (pure/`0`) in the same commit as slice 1 — none of the
   three is independently testable; together they are what makes `cargo build --release -p vow`
   (a full, non-`--skip-cargo` build) succeed at all once slice 3's lexer starts calling the new
   function. No behavioral test yet; this slice only removes "undefined function"/"unresolved
   symbol"/"effectful call from pure function" build failures ahead of slice 3.
3. **Lexer: red test first.** Write `compiler/tests/test_lexer_float_literal.vow` against the
   *current* (unfixed) `lexer.vow` — it fails (wrong token count/tags). Then implement the
   `lexer.vow`/`token.vow` changes from §2. Rebuild with a full `cargo build --release -p vow` followed
   by `scripts/bootstrap.sh --skip-cargo` (per the ordering note above — this is the first slice where
   the new intrinsic is actually called, so the full rebuild is mandatory here, not optional). Green:
   `build/vowc test compiler/` passes `test_lexer_float_literal.vow`, and
   `test_lexer_dot_and_suffix.vow` / `test_wide_literal_lexer.vow` still pass unmodified (regression
   guard for the non-float digit/dot/suffix paths).
4. **Parser + end-to-end checker: red test first.** Write `compiler/tests/test_checker_float_literal.vow`
   against the lexer-fixed-but-parser-unfixed compiler — still fails (parser has no `tok_lit_float()`
   arm, so `parse_primary` falls through to whatever its default/error arm is). Implement the
   `parser.vow` arm from §2. Green: `test_checker_float_literal.vow` passes, and manually confirm the
   issue's exact repro now succeeds: `build/vowc build --no-verify <repro.vow> -o /tmp/out` (this is
   the issue's literal acceptance criterion).
5. **Parity fixture, red→green across both compilers.** Add `tests/run/float_literal_binding.vow`.
   Confirm `scripts/full_test.sh`'s relevant section was failing before slice 4 (self-hosted side
   errors, Rust side succeeds — the exact dual-compiler divergence from the issue) and passes after.
6. **c_emitter/ir_printer fix: red test first.** Add `tests/verify/float_literal_contract.vow`. Before
   the `c_emitter.vow`/`ir_printer.vow` fix, confirm this fixture's self-hosted `vow verify` output
   diverges from the Rust compiler's (wrong constant value reaching ESBMC — likely manifests as a
   spurious verification failure/counterexample, or a wrong-but-"proven" result, depending on how the
   mis-emitted huge integer interacts with the property being checked). Implement the
   `c_emitter.vow`/`ir_printer.vow` one-line-each fix from §2. Green: `scripts/full_test.sh`'s Section
   4 (`tests/verify/*.vow`) passes for this fixture. Note: `full_test.sh`'s `--dump-ir` parity section
   (~line 510) only dumps `compiler/main.vow`, which contains no float literals, so it does **not**
   exercise the `ir_printer.vow` fix — that fix's only regression coverage is the direct string
   assertion added to `test_checker_float_literal.vow` in slice 4 (see §2). Do not claim `--dump-ir`
   parity as coverage for `ir_printer.vow` in the PR description.
7. **Docs.** Add the `docs/spec/grammar.md` float-literal note. Run
   `uv run python scripts/generate_help.py` + `cargo build --release -p vow` +
   `scripts/bootstrap.sh --skip-cargo` per CLAUDE.md's spec-change workflow, and confirm
   `scripts/check_help_coverage.py` (part of `full_test.sh`) has nothing new to flag (this change adds
   prose, not a new `--help`-visible flag/type, so it should be a no-op for that checker — verify
   rather than assume).

## 4. Verification surface

- **No contract semantics change.** This fix does not add `requires`/`ensures`/`invariant` surface,
  so there is nothing to weaken or newly bound for ESBMC.
- **The `tests/verify/float_literal_contract.vow` fixture (slice 6) is the verification surface that
  matters**: it's what proves the `c_emitter.vow` fix actually produces a C model ESBMC evaluates
  correctly (a real double constant, not a billion-scale integer masquerading as one). Keep the
  fixture's property simple and load-bearing — e.g. `requires: x < 10.0` / `ensures: result` on a
  trivial comparison — so a wrong constant would flip the proof outcome, not just cosmetically differ
  in dumped IR text. A property that's true regardless of whether the constant is `1.5` or
  `4609434218613702656` would not actually catch a regression here.
- No new ESBMC-facing opcode, no new IR shape, no change to `vow-clif-shim` stack-slot/codegen
  layout — the Cranelift path (`clif.vow` → `vow-clif-shim`) already handles `IOP_CONST_F64` bits
  correctly today; this fix only touches what feeds that path (lexer/parser) and the separate
  human/ESBMC-text-emission paths (`c_emitter.vow`, `ir_printer.vow`).

## 5. Risk areas

- **Binary fixed point (`scripts/concat_vow.sh` triple test).** The new runtime calls
  (`__vow_parse_f64_bits`/`__vow_format_f64_bits`) and the lexer/parser changes must produce
  byte-identical `compiler_b`/`compiler_c` binaries. Since neither compiler source
  (`compiler/*.vow`) currently contains any float literal, this fix does not change what the
  bootstrap triple test *compiles*; the risk is confined to whether the new lexer branch is
  deterministic (it is — `str::parse::<f64>()`/`to_string()` are pure functions of their input bytes,
  no iteration-order-dependent map involved) and whether the new `env.vow`/`token.vow` additions
  preserve existing tag numbering (verified: `tok_lit_float() = 82` is the next free value after
  `tok_invalid_int() = 81`; do not renumber any existing tag).
- **`c_emitter.vow`/`ir_printer.vow` preamble determinism.** No new conditionally-emitted C preamble
  helper is needed (rejected in favor of the `__vow_format_f64_bits` text-substitution approach — see
  §1), which sidesteps any risk of the `need_shl_i64`-style conditional-injection bookkeeping getting
  out of sync between the two compilers.
- **`parse → print → parse` idempotency does not apply.** The self-hosted compiler has no canonical
  AST→source printer (only `ir_printer.vow`, an IR-text dumper used for debugging/parity, not
  round-tripped through the parser); that invariant is a Rust-`vow-syntax`-only property
  (`vow-syntax/tests/proptest_roundtrip.rs`), and the Rust lexer/printer already handle float literals
  correctly today, so this fix does not touch that invariant at all.
- **`cargo clippy --all -- -D warnings`.** Only the two new `vow-runtime` functions are new Rust code;
  keep them clippy-clean (in particular: `unsafe extern "C" fn` for the pointer-taking
  `__vow_parse_f64_bits`, matching `__vow_string_eq`'s existing `unsafe` annotation, and a normal safe
  `extern "C" fn` for `__vow_format_f64_bits` if it only takes a `u64` and allocates via existing safe
  String-construction helpers already in this file — check what `__vow_string_new`-equivalent helper
  those use and reuse it rather than hand-rolling allocation).
- **Do not widen the fix to "any digit run followed by a dot."** The lexer change must require a
  digit immediately after the dot to commit to the float branch (mirroring
  `vow-syntax/src/lexer.rs:292`'s `peek_byte(1).is_some_and(|b| b.is_ascii_digit())` exactly).
  Getting this wrong would break tuple-index-shaped and method-call-shaped `digit '.' ident`/`digit
  '.' EOF` sequences that must keep lexing as separate tokens — `test_lexer_dot_and_suffix.vow`'s
  `check_dot_never_pairs` is the existing regression guard for adjacent-dot behavior and must keep
  passing unmodified.

## 6. Out of scope

- **`IOP_CONST_F32`'s identical bug in `c_emitter.vow`/`ir_printer.vow`.** Same one-line-fix shape as
  the F64 case, but there is currently no F32 literal syntax anywhere in Vow to ever produce an
  `IOP_CONST_F32` with a literal-driven `dv`, so it stays dormant regardless of this PR. Fixing it
  here would be an unrelated, untestable change bundled into a bug fix — leave it for whenever F32
  literal syntax (if ever) is added, at which point it gets the same red-test treatment as this PR's
  F64 slices.
- **Float literal suffixes (`1.5f32`), scientific notation (`1.5e10`), hex floats.** None of these
  exist in the Rust compiler either (confirmed: `vow-syntax/src/lexer.rs:286-307` has no exponent or
  suffix handling for floats). Not part of dual-compiler parity, not part of this issue.
- **A general `String → f64` / `f64 → String` user-facing API.** `__vow_parse_f64_bits` /
  `__vow_format_f64_bits` are internal compiler-implementation-detail intrinsics, not registered as
  ordinary Vow stdlib functions and not documented in `docs/spec/grammar.md`'s builtin function list.
  Exposing float parsing/formatting to user Vow programs (e.g. a `String::parse_f64_opt()` mirroring
  the existing `__vow_string_parse_i64_opt`/`_u64_opt`) is a separate, much larger feature decision
  (error handling for malformed input, `Option` semantics, spec/grammar additions) that does not
  belong in a lexer bug fix.
- **Negative float literals, `NaN`/`Infinity` literal spellings.** Not mentioned in the issue, not
  present in the Rust compiler's grammar; `-1.5` continues to parse as `EXPR_UNOP(NEG, EXPR_LIT_FLOAT(1.5))`
  exactly as `-1` already does for integers today — no change needed for that composition to keep
  working once `EXPR_LIT_FLOAT` exists.
- **Refactoring the wide-integer digit-scanning loop** in `lexer.vow` to share code with the new float
  path beyond "run the same `is_digit` loop, then branch." No behavior-preserving cleanup bundled in.
