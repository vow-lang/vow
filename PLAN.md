# Plan: lower unary minus on f32/f64 to a float negate opcode in both compilers (#1647)

## Goal
`-x` on `f32`/`f64` (and negative float literals such as `-1.5`) compiles in both compilers as an IEEE `fneg`, via new IR opcodes `NegF32`/`NegF64`; the C emitters and the native verifier (`fp.neg`) follow, and `float_negation_skipped` becomes a pass fixture.

## Assumptions
- Two per-type opcodes `NegF32`/`NegF64` (not one generic `Neg`): every float op today is per-type (`AddF32`/`AddF64`, ...) and the verifier/emitter tables key on that. Opcode numbers 142 (`NegF32`) and 143 (`NegF64`), the next free ids after `ConstU128 = 141`, same value in `vow-ir/src/serialize.rs`, `compiler/ir.vow`, `vow-clif-shim/src/lib.rs`. (best guess)
- Bump `MODULE_VERSION` 5 -> 6 in both `vow-ir/src/serialize.rs:32` and `compiler/ir.vow:127`, and the pin at `serialize.rs:1027`: precedent #1063 (`1dbef4da`, the 140/141 opcode addition) bumped 4 -> 5, and an old reader would otherwise fail on unknown ids 142/143. The multi-file vmod tests use `MODULE_VERSION()` symbolically; `tests/multi/vmod_decode_bad_version` pins the bad byte 2, unaffected. (verified)
- Float literals are always `f64` (`vow-types/src/check.rs:2324`, `lower/mod.rs:1797`, `lower.vow:3013`): there is no f32 literal coercion, so no context-type propagation through `-` is needed (unlike the integer-marker machinery). `-1.5` is `ConstF64` + `NegF64`. f32 negation is only reachable through f32-typed operands (params, fields, results). (verified)
- `scripts/seed.toml` does not exist yet and `vow/src` has no seed delegation, so stage 0 verifies with its own lowering; no seed-pin note needed. (verified)
- No constant folding of `-<float literal>`: lower as `ConstF64` then `NegF64` (Cranelift folds it). Keeps the change surgical. (best guess)
- C model: `v = -v;` (IEEE sign-bit flip in C). ESBMC models C float negation exactly, so `-0.0` and NaN behave as in codegen. (best guess)
- Native verifier: a NaN sign flip is not distinguishable (documented single canonical NaN, `compiler/vc_fp.vow:3-12`); `fp.neg` of NaN is NaN. Note it in docs, no extra encoding.
- Out-of-scope latent items found, left alone: `const X: f64 = -1.5` handling in `vow-ir/src/lower/mod.rs:5810` is integer-only (const forms are int/bool per `grammar.md:51`); float `%` stays unsupported.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `vow-ir/src/lower/mod.rs` | Rust lowering of `UnOp::Neg`; add float arm | 2049-2074 |
| `vow-ir/src/types.rs` | `Opcode` enum: add `NegF32`, `NegF64` | 122-144 (float group) |
| `vow-ir/src/printer.rs` | opcode name match + printer test table | 86 (match), 579-597 (test table) |
| `vow-ir/src/serialize.rs` | `opcode_map!` ids, `MODULE_VERSION` 5->6 and its pin test | 32, 326-352, 1027 |
| `compiler/ir.vow` (also) | `MODULE_VERSION` 5->6 | 127 |
| `skills/vow/reference/{cli,errors,grammar}.md` | tracked copies of the spec; regenerate/sync with the spec edits (`scripts/generate_help.py`, `full_test.sh` check) | cli.md:63,75; errors.md:1411 |
| `vow-codegen/src/cranelift_backend.rs` | `fneg` codegen | 1120-1140 (float arith); test helper ~5044 |
| `vow-clif-shim/src/lib.rs` | `IOP_NEG_F32/F64` consts + `fneg` arm | 261-285 consts, 2180-2205 arm, tests ~6590 |
| `vow-verify/src/c_emitter.rs` | modelable-opcode list + emission | ~935 (list), 1296 (emit), test ~5548 |
| `compiler/ir.vow` | `IOP_NEG_F32/F64` ids | 96-97 |
| `compiler/ir_printer.vow` | op names | 110-118 |
| `compiler/lower.vow` | self-hosted `EXPR_UNOP` lowering | 3266-3293 |
| `compiler/c_emitter.vow` | modelable list, name table, emission | 515-535, 637, 1785-1800, 1834 |
| `compiler/vc_fp.vow` | float op tables + `vc_fp_inst_ok` | 24-105 |
| `compiler/vc_term.vow` | `VC_OP_FPNEG` id (46) | 82-91 |
| `compiler/vc_smt.vow` | `fp.neg` SMT symbol | 143-155 |
| `compiler/vc_exec.vow` | executor value function; unary path | 340-360 |
| `compiler/vc_gate.vow` | gate calls `vc_fp_inst_ok` | 287 |
| `tests/verify-native/skip/float_negation_skipped.vow` | becomes pass fixture | whole |
| `scripts/test_verify_perf.py` | names the skip fixture | 197, 212 |
| `docs/spec/cli.md`, `docs/spec/errors.md`, `docs/adr/2026-10-08-1422-native-verification-semantics.md` | stale "unary minus on a float is Skipped" prose | cli.md:63,75; errors.md:1411; ADR:565-568 |
| `compiler/main.vow`, `vow/src/skill.rs` | generated help/skill copies of the spec prose | main.vow:2967, 3213; skill.rs:6468, 13478 |

## Steps (ordered; see TDD slices in Testing)

### 1. Add the opcodes to the Rust IR
- `vow-ir/src/types.rs`: `NegF32`, `NegF64` after `GeF64` (enum order is not an ABI; ids live in `serialize.rs`).
- `vow-ir/src/serialize.rs`: `142 => NegF32, 143 => NegF64`; `MODULE_VERSION` 6 (and the `assert_eq!(MODULE_VERSION, 5)` pin).
- `vow-ir/src/printer.rs`: names `"NegF32"`/`"NegF64"` in the match and in the printer test table (line ~579).

### 2. Rust lowering
- `vow-ir/src/lower/mod.rs` `UnOp::Neg` arm: when `operand_ty` is `Ty::F32`/`Ty::F64`, emit `Opcode::NegF32/NegF64` with `ty = operand_ty`, `vec![val]`, `InstData::None`; otherwise keep the existing integer path untouched.

### 3. Rust codegen (Cranelift)
- `cranelift_backend.rs`: `Opcode::NegF32 | Opcode::NegF64 => builder.ins().fneg(arg!(0))` beside `SubF32|SubF64`.

### 4. Self-hosted IR/lowering
- `compiler/ir.vow`: `IOP_NEG_F32() = 142`, `IOP_NEG_F64() = 143`, `MODULE_VERSION() = 6`. `compiler/ir_printer.vow`: names. Check `compiler/module_io.vow` (writes/reads opcodes) needs no per-opcode table.
- `compiler/lower.vow` `EXPR_UNOP` (3266): after computing `operand_ty`, if `ITY_F32`/`ITY_F64` emit `IOP_NEG_F32()/IOP_NEG_F64()` with `un_args`, `IDATA_NONE()`, result type = operand type; return before the integer-zero path. Reuse `un_args` (already holds the operand).
- `vow-clif-shim/src/lib.rs`: consts 142/143 and `IOP_NEG_F32 | IOP_NEG_F64 => builder.ins().fneg(arg!(0))` next to the float arithmetic arm (2189). Check `compiler/clif.vow` needs no per-opcode table (grep `IOP_SUB_F64` found none outside shim).

### 5. C emitters (byte-identical)
- Rust `c_emitter.rs`: add both opcodes to the modelled list (~935) and emit `  v{id} = -v{a};\n` (model on the `Opcode::Not` arm, 1336).
- `compiler/c_emitter.vow`: add to the modelable-op chain (515-535) and emit the same text from the `IOP_NOT` pattern (1834, `str4`/`cemit_str2`). The unary text must match byte-for-byte; `scripts/parity.py c` over the new fixtures proves it.

### 6. Native verifier
- `vc_term.vow`: `VC_OP_FPNEG() = 46`. `vc_smt.vow` `vc_fp_op_symbol`: `"fp.neg"` (no rounding mode; unary).
- `vc_fp.vow`: add `IOP_NEG_F32/F64` to `vc_fp_op_ty` (so `vc_fp_is_op` holds), new `vc_fp_is_neg(op)`; `vc_fp_inst_ok`: `n == 1 && inst.ty == ft && t0 == ft`; update the header comment (NaN sign note).
- `vc_exec.vow`: handle neg **before** the `lhs`/`rhs` loads at 348-349 (they index `args[1]` and would be out of range for a one-operand op): `if vc_fp_is_neg(op) { return vc_tm_app1(ar, VC_OP_FPNEG(), w.var_of[inst.args[0] as u64]); }`. Check `vc_ops.vow:56` and any arity assumption in `vc_gate.vow` and `vc_term.vow` app1 simplifiers (no new rewrite needed).

### 7. Fixtures and docs
- `git mv tests/verify-native/skip/float_negation_skipped.vow tests/verify-native/pass/float_negation.vow`; rewrite with true contracts (no `result == result` placeholder): `negate(x)` with `requires: x > 0.0` `ensures: result < 0.0`; `negate_twice(x)` with `requires: x == x` `ensures: result == x`; an `f32` twin. Do not claim `result == 0.0 - x` (false at `x == 0.0`). Update `scripts/test_verify_perf.py:197,212` to the new name/`"Verified"`.
- `docs/spec/cli.md:63,75`, `docs/spec/errors.md:1411` (drop "unary minus on a float" from the `unsupported-opcode` examples and the Subset "Skipped" list; add float unary `-` to the supported ops), ADR `2026-10-08-1422-...md:565-568` (move from "Still skipped" to modelled, note `fp.neg`, single-NaN-sign limit). `docs/spec/grammar.md:529` Unary table: `-` also negates `f32`/`f64` (IEEE: flips sign of `0.0` and NaN).
- The skill/reference copies under `skills/vow/reference/` carry the same prose: update with the spec (confirm how `generate_help.py`/`full_test.sh` keep them in sync).
- Fixture directives: after the move change `// TEST: category unverifiable` to `// TEST: category float` and drop `// TEST: skip-reason unsupported-opcode`; check `tests/verify-native/tests.sh` for a closed category list.
- Regenerate: `uv run python scripts/generate_help.py` (updates `compiler/main.vow` and `vow/src/skill.rs`), then `cargo build --release -p vow`, `scripts/bootstrap.sh --skip-cargo`.

## Testing (TDD slices, red first)
1. **Rust lowering** (`vow-ir/src/lower/mod.rs` tests near 7248, `-x` on i128 test as the template): `fn n(x: f64) -> f64 { -x }` and the `f32` twin lower to `NegF64`/`NegF32` with one operand and the float type, not `WrappingSub`. Green: step 1+2.
2. **Rust codegen** (`vow-codegen/tests/e2e.rs`, mirror existing float e2e; plus the unit helper ~5044): `negate` compiles and links without `CodegenFailed`. Green: step 3. (vow e2e tests may SKIP-panic in the sandbox, see memory; compare against clean main.)
3. **Self-hosted lowering** (`compiler/tests/test_lower_float_binop.vow` pattern, new `compiler/tests/test_lower_float_neg.vow` [new]): lowering a `-x` unop on `f32`/`f64` yields `IOP_NEG_F32/F64`, arity 1, result type = operand type; integer `-x` still `IOP_WSUB`. Green: step 4. Run `build/vowc test compiler/ --filter test_lower_float_neg`.
3b. **Shim coverage** (`vow-clif-shim/src/lib.rs` tests, model on `float_remainder_is_refused_as_unsupported` ~6605): build a `NegF32`/`NegF64` function through `__vow_clif_fn_inst` and assert `__vow_clif_fn_end` returns 0 (the shim arm is otherwise only run by uninstrumented `build/vowc`, so codecov/patch would flag it). Also a `.vmod` serialize round-trip test for ids 142/143 and the bumped version in `serialize.rs` tests.
4. **Run fixture** `tests/run/float_unary_negation.vow` [new] (both compilers via `full_test.sh` section 4): f32 and f64 negate, negative literal `-1.5`, double negation, and f32 negation through f32 parameters/fields (no f32 literals exist), sign of zero observed without a float printer: `1.0 / negate(0.0) < 0.0` and `1.0 / negate(-0.0) > 0.0`; `// TEST: stdout` with `print_i64` flags; also a negate on an `f64` struct field/param in a loop to cover non-constant operands.
5. **C emitters**: unit test in `c_emitter.rs` (extend `emit_float_arithmetic` ~5548: assert `v{n} = -v{m};`), `compiler/tests/test_c_emitter.vow` equivalent; fixture in `tests/verify/float_negation.vow` [new] (ESBMC proves e.g. `requires: x > 0.0 ensures: result < 0.0`); `python3 scripts/parity.py c target/release/vow build/vowc tests/verify/float_negation.vow tests/verify-native/pass/float_negation.vow`.
6. **Native verifier** (`compiler/tests/test_vc_fp.vow`, add to `check_op_tables`/`check_gate`/render checks): `vc_fp_op_ty(IOP_NEG_F64()) == ITY_F64()`, `(assert (= ... (fp.neg x)))` renders, `vc_skip_reason` empty for a negate block and non-empty for wrong-typed/two-operand negate. Then step 7 pass fixture via `tests/verify-native/tests.sh`.
7. Spec drift: `python3 scripts/check_help_coverage.py`, `python3 scripts/test_verify_perf.py` (unit tests naming the fixture).
8. Gates: `cargo fmt --all`, `cargo clippy --all --all-targets -- -D warnings`, `cargo test --all`, `scripts/bootstrap.sh --skip-cargo --no-cache` (record head SHA in the PR checklist), `build/vowc test compiler/`, `scripts/full_test.sh` in the background with polling (~40 min, never the 2-min default).

## Verification surface
- ESBMC: new function-level properties `-x` result sign/value; C model emits `v = -v;`; ESBMC float semantics are exact. No contracts weakened; any NaN/-0.0 limit is documented, not bounded.
- Native verifier: `fp.neg`; fixtures `tests/verify-native/pass/float_negation.vow` (f64 + f32). `unsupported-opcode` still has other fixtures (`float_vec_skipped`), so the skip-code coverage gate (#1651) is unaffected.
- `tests/run/` and `tests/verify/` fixtures grow as listed above.

## Risks
- **Executor arity**: `vc_exec.vow:348-349` read `args[1]` unconditionally; the neg branch must come before them (step 6) or the verifier crashes/out-of-range.
- **Verifier C parity**: unary emission text must match exactly between `c_emitter.rs` and `c_emitter.vow`; `full_test.sh` Section 2c enforces it.
- **Fixed point**: new opcode ids touch lowering and codegen of the compiler itself only if the compiler source negates floats (it does not use floats), so the binary fixed point should hold; confirm via bootstrap.
- **Stale compile cache** after rebuilds: use `VOW_CACHE_DIR=$(mktemp -d)` when validating the codegen fix (memory note).
- **Idempotency/serialize**: `.vmod` round-trip test for the new ids; `MODULE_VERSION` unchanged by assumption.
- Cranelift `fneg` on f32/f64 verified, but clippy gate: the `opcode_map!` and exhaustive Rust matches (`printer.rs`, `c_emitter.rs`, `cranelift_backend.rs`) fail to compile until every arm is added, so land step 1-3 and 5 (Rust) in one commit.
- `codecov/patch` 95% gate: Rust unit tests must cover the new Rust lines (lowering, emitter, backend arms), since the .vow corpus runs uninstrumented.

## Out of scope
- Float `%` (`float-rem-unsupported`), `Vec` of floats, `parse/format_f64_bits` modelling.
- Constant folding of negative float literals; float `const` items.
- Distinguishing NaN sign in the verifier.
- Refactors of the opcode tables or generated-file mechanics; formatting changes; unrelated cleanups.
- Commit split suggestion (surgical): (a) Rust+self-hosted lowering/codegen + run fixture, (b) C emitters + verify fixture, (c) native verifier + fixture move + docs. Single PR titled `fix(lower): negate f32 and f64 with an IEEE float negate opcode` (squash subject lower-case, <= 92 chars).
