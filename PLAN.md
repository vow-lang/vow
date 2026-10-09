# Plan: lower f32/f64 comparisons to the float compare opcodes (issue #1508)

## Goal
`a == b`, `!=`, `<`, `<=`, `>`, `>=` on `f32`/`f64` operands must lower to the dedicated
`EqF32/EqF64 … GeF32/GeF64` opcodes (`IOP_EQ_F32 … IOP_GE_F64`) in BOTH compilers, so native
codegen emits `fcmp` instead of an invalid `icmp` on a float controlling type. Classified **Small**:
two production functions (one per compiler), tests, one spec paragraph. No backend/emitter change.

## Assumptions
- No change to Cranelift backends, shim, C emitters or the native verifier: float compare opcodes are
  already implemented/handled there (`vow-codegen/src/cranelift_backend.rs:1144-1176`,
  `vow-clif-shim/src/lib.rs:2210-2238`, `vow-verify/src/c_emitter.rs:1285-1310`,
  `compiler/c_emitter.vow:1713-1732`, `compiler/vc_ops.vow:70-80`). Best guess, confirmed by reading.
- Operand type is the existing `operand_ty` (= lhs IR type; float literals are always `ConstF64`),
  unchanged. Float values whose IR type is not `f32`/`f64` (e.g. loaded through an integer-typed
  slot) are out of scope and keep today's behaviour.
- NaN semantics follow IEEE-754 / the C model: `==` false, `!=` true, ordered comparisons false
  (`FloatCC::NotEqual` is "unordered or not-equal"; matches C `!=`). Documented, not changed.
- Do not touch `ConstF*`/`#1503` operand-type annotation work; `Ne/Lt/…` data field for float
  compares is `InstData::None`, mirroring float arithmetic (`bin_data_kind = IDATA_NONE` already at
  `compiler/lower.vow:3062-3067`).

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `vow-ir/src/lower/mod.rs` | Rust `binop_opcode`: add float compare arms | 5048-5092; callers 1933, 4963 |
| `compiler/lower.vow` | self-hosted `binop_opcode`: add float compare branch | 1449-1499; callers 2646, 3058 |
| `vow-ir/src/lower/mod.rs` (tests mod) | new unit test next to `float_arithmetic_uses_float_opcodes` | 7214-7260; helpers 6874-6890 |
| `compiler/tests/test_lower_float_binop.vow` | extend float binop opcode test | whole file |
| `tests/run/float_comparison.vow` [new] | end-to-end fixture, both compilers via `full_test.sh` §4 | — |
| `tests/run/float_literal_binding.vow` | stale header comment says float comparisons are a known gap | 1-8 |
| `docs/spec/grammar.md` | "Comparison Operators" section: add float semantics paragraph | 434-445 |

## Steps (TDD slices, ordered)

### 1. Red→green Rust: lowering emits float compare opcodes
- **Test first** (`vow-ir/src/lower/mod.rs` tests mod, modelled on `float_arithmetic_uses_float_opcodes`):
  `float_comparison_uses_float_opcodes` lowers 12 one-liner fns (`fn eq_f32(a: f32, b: f32) -> bool { a == b }`
  … for `== != < <= > >=` × `f32`/`f64`) and asserts the instruction has `(EqF32, Ty::Bool, InstData::None)`
  etc. Also assert an `i64` comparison still lowers to `Opcode::Lt` with `InstData::Integer(I64)` (regression).
  Run `cargo test -p vow-ir float_comparison_uses_float_opcodes` → fails.
- **Code**: in `binop_opcode` (`vow-ir/src/lower/mod.rs:5050`) extend the `float_opcode` match with the 12
  `(BinOp::Eq|Ne|Lt|Le|Gt|Ge, Ty::F32|Ty::F64)` arms. Because the existing early return yields
  `result_ty = *operand_ty`, change that return so comparison ops yield `Ty::Bool`:
  split into arithmetic (`result_ty = *operand_ty`) vs comparison (`Ty::Bool`) arms — e.g. have the match
  return `(Opcode, Ty)` pairs. Data stays `InstData::None`.
- **Reuses**: existing `Opcode::{EqF32..GeF64}` (`vow-ir/src/types.rs:127-144`).

### 2. Red→green self-hosted: `binop_opcode` / `binop_result_ty`
- **Test first** (`compiler/tests/test_lower_float_binop.vow`): add `check_float_compares(ty, eq, ne, lt, le, gt, ge)`
  asserting `binop_opcode(BINOP_EQ(), ty) == IOP_EQ_F64()` … for `ITY_F32`/`ITY_F64`, `binop_result_ty(BINOP_LT(), ITY_F64()) == ITY_BOOL()`,
  `binop_opcode(BINOP_LT(), ITY_I64()) == IOP_LT()` (regression), and extend `check_float_binop_data` with a
  `fn lt(a: f64, b: f64) -> bool { a < b }` module asserting `inst.op == IOP_LT_F64()`, `inst.ty == ITY_BOOL()`,
  `inst.dk == IDATA_NONE()`, `inst.dv == 0 && inst.dv2 == 0`. Run `build/vowc test compiler/tests/test_lower_float_binop.vow` → fails.
- **Code**: in `compiler/lower.vow::binop_opcode` (line 1453 `if ir_ty_is_float(operand_ty) {`) add the six
  `BINOP_EQ/NE/LT/LE/GT/GE` branches returning `IOP_*_F32` / `IOP_*_F64`, same shape as the arithmetic ones.
  `binop_result_ty` (1513) already returns `ITY_BOOL()` first for comparisons — no change. Data-kind
  override at 3062-3067 already yields `IDATA_NONE` for float operand types — no change.
- Contracts on `binop_opcode` (`requires: is_valid_binop(op)`, `ensures: result != -1`) stay as they are.

### 3. End-to-end fixture (both compilers)
- **File** [new] `tests/run/float_comparison.vow`: header `// TEST: stdout "…"` (print_i64 results; floats
  have no print). All stdout-checked runtime cases use **f64 only**: `== != < <= > >=` (params + `let`
  bindings), a comparison in an `if` condition and a `while` loop condition, and the issue's exact repro.
  f32 follows `tests/run/float_arithmetic.vow`: define uncalled `cmp_f32` functions (params only, no f32
  literals/`let a: f32 = 1.5`) as a codegen guard — float literals always lower to `ConstF64`
  (`vow-ir/src/lower/mod.rs:1689`), so a called f32 compare with a literal would hand `fcmp` mixed widths
  (the separate, out-of-scope f32-literal gap). Include an `a != a`/NaN
  line only if `0.0 / 0.0` compiles and runs without a runtime abort (check during implementation; drop it
  otherwise rather than weakening the other cases). `full_test.sh` §4 builds/runs it with both compilers.
- Also run it under `--mode debug` once by hand (a float comparison inside a `requires:` takes the same lowering path).
- Update the stale paragraph in `tests/run/float_literal_binding.vow:4-7` (comparisons "intentionally not
  exercised … known Cranelift gap") — delete the claim; leave the fixture body unchanged.

### 4. Spec
- `docs/spec/grammar.md`, after the Comparison Operators table (~line 444): one paragraph — for `f32`/`f64`
  operands the six operators lower to IEEE-754 ordered/unordered comparisons (`fcmp`); any comparison
  with a NaN operand is false except `!=`, which is true; operands must have the same type
  (checker rule, `vow-types/src/check.rs:2250-2262`: differing operand types are `TypeMismatch`). `scripts/generate_help.py` only extracts the table column, so
  `--help`/skill text should not change; still run `scripts/check_help_coverage.py` to confirm.

## Testing / Verification surface
- `cargo test -p vow-ir`, `cargo test -p vow-codegen` (backend fcmp tests at `cranelift_backend.rs:4950-5010` stay green),
  `cargo clippy --all --all-targets -- -D warnings`, `cargo fmt --all`.
- `scripts/bootstrap.sh --skip-cargo --no-cache` (fixed point; record head SHA), then
  `build/vowc test compiler/tests/test_lower_float_binop.vow` and `build/vowc test compiler/`.
- Use `VOW_CACHE_DIR=$(mktemp -d)` for every manual build/run (compile cache ignores compiler changes).
- `scripts/full_test.sh` (background, ~40 min; do not rely on the 2-min default timeout): relevant sections are
  §4 (run fixtures, both compilers), §2c (verifier C parity over `tests/verify*/`), IR dump parity (~line 622),
  and `tests/verify-native` skip fixtures.
- ESBMC: no new contracts or obligations. Existing `tests/verify/float_*.vow` contain float comparisons and now
  lower to `GeF64`/`LeF64`; C text is unchanged because both emitters render float compare opcodes
  identically to the integer ones (`emit_cmp`), so parity §2c must stay byte-identical — confirm, don't assume.

## Risks
- **C parity / verifier drift**: opcode switches from `Ge[Integer(I64)]` to `GeF64[None]`. Rust emitter has
  `integer_test_data`-style data defaults only in tests; emitters ignore `data` for compares. Mitigation: run
  `python3 scripts/parity.py c` over `tests/verify/float_arithmetic.vow` and `float_literal_contract.vow`.
- **Native verifier skip reasons**: `vc_op_unsupported` already lists `IOP_*_F32/F64`
  (`compiler/vc_ops.vow:70-80`), so float-compare functions still skip with `unsupported-opcode`; the detail
  string (`vc_unsupported_detail`, `vc_gate.vow:248`) is `<OpName>[<ty>]` — now names `GeF64[bool]`-style
  instead of `Ge[f64]` when a compare is the first unsupported instruction. Check `tests/verify-native/skip/float_skipped.vow`
  (`skip-reason unsupported-opcode` only — code-level match, should be fine) and `compiler/tests/test_vc_gate.vow`.
- **`vow/src/cex_eval.rs`** matches only integer `Opcode::Eq…Ge`; `*F64` fall to `_ => None`. Counterexample
  evaluation of float comparisons previously went through `eval_callee_i64` and returned `None` for float
  operands anyway; verify no behaviour change by running `cargo test -p vow cex_eval`.
- **Fixed point**: self-hosted `lower.vow` itself uses no float comparisons (grep before landing); the change
  only affects float-typed programs, but run the full bootstrap to prove the triple.
- **Mixed-type fallback**: an operand whose IR type is not float despite being `f64` at source still emits
  `icmp` (bit compare). Not introduced here; record as follow-up, do not widen this PR.
- **Embedded `binop_opcode` text in `compiler/main.vow`** (lines ~6860, 6875, 12902, 12917) is illustrative
  skill/docs prose (`r.push_str` of a pedagogical `fn binop_opcode` snippet with `// ... one arm per valid op ...`),
  not a mirror of `lower.vow`; do not edit it.
- **Other direct `Opcode::Eq` emitters** (`vow-ir/src/lower/mod.rs:3553, 4115, 4324`, `Opcode::Lt` at 2712) are
  I64 enum/Option/Result tag and index comparisons, never float. Literal patterns (`PatKind::Lit`) are only
  checked in `vow-types`, never lowered to a compare, so there is no float-pattern equality path. Out of scope.
- **Golden IR snapshots**: grep found no `Lt[f64]`-style expectations in `tests/`, `vow/tests/`, `vow-ir`,
  `compiler/`; re-grep after the change for `Eq[`/`Lt[` with float operand types in dump-ir fixtures.
- **Dual-compiler rule**: both halves must land in the same PR; do not split.
- `float_remainder` stays `CodegenUnsupported` — unrelated, untouched.

## Out of scope
- #1503 (Eq operand-type annotation parity / `Eq[i64]` vs `Eq[Bool]` in `--dump-ir`).
- Float remainder codegen, float printing/conversion, checked float ops, f32 literal typing.
- Any refactor of `binop_opcode` beyond the minimal arm additions, formatting, or cleanup.
- Native-verifier float modelling (floats stay unverifiable there) and ESBMC model changes.
- Changing integer comparison lowering or `InstData::Integer` annotations on integer compares.

## PR
Title (≤92 chars, lower-case subject): `fix(codegen): lower f32/f64 comparisons to fcmp opcodes in both compilers`.
Remove `PLAN.md` in the implementation commit (stage handoff artefact). Record head SHA + `scripts/seed.toml`
pin (if present) next to the green-bootstrap claim.
