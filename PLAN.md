# Plan — issue #1076: 128-bit struct fields fail codegen with a raw Cranelift verifier dump

Branch: `sym/vow/1076-128-bit-struct-fields-fail-codegen-with-a-raw-cranelift-verifier-dump`
Proposed PR title: `fix(codegen): reject 128-bit struct fields and enum payloads instead of truncating`

## 1. Problem restated

Aggregate slots are 8 bytes wide everywhere in the pipeline (`RegionAlloc` sizes a struct at
`(n_fields + 1) * 8`, and every `FieldGet`/`FieldSet` computes its byte offset as `idx * 8`), but
`i128`/`u128` values are 16 bytes. The load side emits `load.i64` and then `ireduce.i128`, which is
not a reduction at all, so Cranelift's verifier rejects the function and the user gets a raw CLIF
dump; the store side emits a 16-byte `store` into an 8-byte slot, which silently overwrites the
neighbouring field. The same 8-byte assumption exists in the verifier's C model, whose
`int64_t __vow_heap[]` truncates a 128-bit field slot. The issue asks for either a real 16-byte
field layout or, as an interim step, a named `UnsupportedOpcode` refusal in the spirit of #1057.
This plan takes the interim step, because it is the smaller change *and* because it is what
`docs/spec/grammar.md` already promises today ("the compiler rejects every such program rather than
producing one that computes on a truncated value").

## 2. Evidence gathered while planning (all reproduced on this branch)

Built `./target/release/vow` from the branch HEAD and ran three probes. These matter because two of
them contradict the issue's severity assessment ("fails closed … not a soundness hole").

1. **The reported crash reproduces.** The repro from the issue body fails with
   `FunctionDefine("… Verifier errors …")`, containing `v21 = ireduce.i128 v20`. Confirmed.
2. **The store side silently corrupts a neighbouring field.** *Not* fail-closed:
   ```vow
   module Fld2
   struct Box { v: i128, w: i64 }
   fn main() -> () [io] {
       let b: Box = Box { w: 7, v: 3154393236604333326336 };
       print_i64(b.w);
   }
   ```
   compiles cleanly (`status: Unverified`, exit 0) and prints `171` instead of `7` — the 16-byte
   store of `v` at offset 0 clobbered slot 1.
3. **Enum / `Option` payloads silently truncate.** `enum E { V(i128) }` with
   `E::V(3154393236604333326336)` builds and runs, and the matched payload reads back with a zero
   high limb. `--dump-ir` shows why the load-side guard alone would not catch it: the match
   extraction emits `i64 %9 = FieldGet[field_1](%1)` — the *IR type is already i64*, so the
   truncation is baked in before codegen. The construction site, however, is
   `FieldSet[field_1](%1, %0)` with `%0 : i128`, so a guard on the **stored value's** type does
   reject the program. Since a Vow enum value can only come from a `FieldSet` somewhere in the
   module, guarding the store closes this hole at module granularity.
4. **The verifier model truncates too.** `fn roundtrip(x: i128) -> i128 vow { ensures: result == x }`
   whose body round-trips `x` through a `struct W { v: i128 }` field is reported `VerifyFailed`
   with a **false counterexample** (`x = -18446744073709551616`), because `__vow_heap[]` is
   `int64_t`. The model is a projection of the real state space rather than an over-approximation,
   so it can be wrong in either direction; the demonstrated instance is a false alarm.

So the defect surface is three-fold: field load (crashes), field store (silently wrong), verifier
model (silently wrong). All three must fail closed.

## 3. Decision and scope

**Fail closed everywhere; do not implement the 16-byte layout in this PR.**

Rationale:
- The real fix requires a variable-width slot layout. Codegen has *no* struct-layout information —
  `InstData::FieldIndex(idx)` is the only thing the backend sees, and `idx * 8` is the whole layout
  model. Making 128-bit fields occupy two slots means changing what a field index *means* in the IR
  and updating every consumer (`vow-ir/src/lower/mod.rs` sizes, `vow-ir/src/region.rs` field
  tracking, both C emitters' `__vow_heap[base + idx]` model, both IR printers/golden tests, the
  serializer, the shim). That is a multi-PR change and would bundle a layout refactor into a bug
  fix, against CLAUDE.md's "surgical changes" rule.
- The spec already states the intended *behaviour* is rejection. Making the rejection real and
  named is therefore spec conformance, not a narrowing of scope.
- ADR 0001 §Struct-layout and `grammar.md` already record the two-slot layout as the accepted
  target. The follow-up implements it; nothing in this PR contradicts it.

**Where the guard goes: the backends** (`vow-codegen` + `vow-clif-shim`), exactly as #1057 did for
`/`, `%`, `/!`, `%!`, `*!`, plus the two C emitters for the verifier side. Not the type checker
(that would turn a backend gap into a language rule, contradicting grammar.md and breaking
`tests/error/u128_struct_field_literal_below_range.vow`'s premise), and not IR lowering (the
self-hosted `lower_module_vow` has no `DiagCtx` parameter, so routing a diagnostic out of it means
widening an already 15-argument signature — disproportionate for an interim guard).

## 4. Files to touch

Production code:

| Path | Change |
|---|---|
| `vow-codegen/src/cranelift_backend.rs` | `extend_field_store_value` (line ~434) returns `Result<Value, CodegenError>` with an explicit `IrTy::I128 \| IrTy::U128` arm that errors instead of falling through `_ => value`; `Opcode::FieldGet` arm (line ~1779) errors when `inst.ty` is 128-bit; `Opcode::FieldSet` arm (line ~1800) propagates the helper's error. |
| `vow-clif-shim/src/lib.rs` | Mirror both guards: `extend_field_store_value` (line ~170) and the `IOP_FIELD_GET` / `IOP_FIELD_SET` arms (lines ~2745 / ~2769). Report through a shared `report_wide_aggregate_field()` next to the existing `report_narrowed_wide_argument()` and return `-1` from `__vow_clif_fn_end`. |
| `vow-verify/src/c_emitter.rs` | `is_modelable` (lines ~727-731): `FieldGet` is modelable only when `inst.ty` is not 128-bit; `FieldSet` only when its value argument is not 128-bit. `first_unsupported_opcode` (line ~795+): return `"FieldGet at 128-bit width"` / `"FieldSet at 128-bit width"`, mirroring the existing `CheckedAdd at 128-bit width` precedent. Add a `collect_wide_vars(func)` pass in the style of the existing `collect_option_vars`. |
| `compiler/c_emitter.vow` | Mirror of the above: `is_modelable` (line 289) and `first_unsupported_opcode_name` (line 430), plus a `collect_wide_vars` mirroring `collect_option_vars`. These two functions are explicitly documented as mirrors of the Rust ones and must stay in sync. |

Docs (spec is the source of truth; both statements below are currently inaccurate):

| Path | Change |
|---|---|
| `docs/spec/grammar.md` (~152-156) | The "Struct field layout" paragraph claims `i128`/`u128` fields "occupy two consecutive 8-byte slots (16 bytes)". No compiler implements this. Restate: every field occupies one 8-byte slot today; the two-slot layout is the accepted target (ADR 0001) but is not implemented, and 128-bit fields are refused until it lands. |
| `docs/spec/grammar.md` (~299-316) | Extend the "128-bit values are scalar-only" paragraph: the refusal now names itself rather than surfacing a Cranelift dump; enum and `Option`/`Result` payloads are refused at the construction site; a contracted function that reads or writes a 128-bit field is reported `Skipped` (fail-closed) rather than modelled on a truncated heap slot. |

Generated artefacts (must be regenerated, never hand-edited — `scripts/generate_help.py` reads
`docs/spec/grammar.md` and injects into both):

- `vow/src/skill.rs` (`GENERATE:SKILL_JSON` / `GENERATE:SKILL_HUMAN` blocks)
- `compiler/main.vow` (same blocks)
- `skills/vow/SKILL.md`

Tests:

| Path | Change |
|---|---|
| `vow/tests/wide_literal_aggregates.rs` | Two new end-to-end tests beside the existing `wide_values_in_aggregates_fail_closed`, using the same `status == "CompileFailed"` + `message.contains(...)` shape. |
| `vow-codegen/src/cranelift_backend.rs` (test module) | Unit tests for the load guard and the store guard, in the style of `wide_arguments_to_narrow_externs_are_rejected`. |
| `vow-clif-shim/src/lib.rs` (test module) | Mirror unit tests, in the style of `wide_constant_with_mismatched_metadata_is_rejected` (assert `__vow_clif_fn_end` returns `-1`). |
| `vow-verify/src/c_emitter.rs` (test module) | Unit test: a function containing a 128-bit `FieldGet`/`FieldSet` is not modelable and `non_modelable_reason` names it. |
| `tests/verify-skip/wide_struct_field_skipped.vow` | New fixture — parity-compared across both compilers by `scripts/full_test.sh` Section 4d, which asserts `status == "Skipped"`. |

## 5. TDD slices

Each slice is red → green → refactor and is independently reviewable. Run
`cargo test -p <crate>` for the slice's crate before moving on.

### Slice 1 — Rust backend refuses a 128-bit field **load**

- **Red.** `vow-codegen/src/cranelift_backend.rs`, test module: build a one-block IR function with
  `RegionAlloc` → `FieldGet[field_0]` typed `IrTy::I128`, compile it, assert
  `Err(CodegenError::UnsupportedOpcode(msg))` and that `msg` names the limitation. Fails today with
  a Cranelift verifier error instead.
- **Green.** In the `Opcode::FieldGet` arm, before emitting the load, return
  `Err(CodegenError::UnsupportedOpcode(WIDE_AGGREGATE_FIELD_MSG.to_string()))` when
  `matches!(inst.ty, IrTy::I128 | IrTy::U128)`.
- **Message (shared verbatim by all four guards).**
  `"128-bit struct fields and enum payloads are not supported yet (epic #526): an aggregate field slot is 8 bytes, so a 128-bit field would truncate or overwrite its neighbour"`

### Slice 2 — Rust backend refuses a 128-bit field **store**

- **Red.** Unit test: `FieldSet[field_0](alloc, const_i128)` must be refused. Fails today (compiles,
  emitting a 16-byte store into an 8-byte slot).
- **Green.** Change `extend_field_store_value` to
  `fn extend_field_store_value(..) -> Result<Value, CodegenError>` with an explicit
  `IrTy::I128 | IrTy::U128 => Err(...)` arm ahead of the `_ => Ok(value)` fallthrough (defect #2 in
  the issue is precisely that fallthrough), and `?` it at the call site. Update the two existing
  in-file callers at ~3426/3428.

### Slice 3 — End-to-end refusal, Rust driver

- **Red.** `vow/tests/wide_literal_aggregates.rs`: two tests modelled on
  `wide_values_in_aggregates_fail_closed` —
  (a) the issue's exact struct repro; (b) the enum-payload repro
  (`enum E { V(i128) }` + `E::V(3154393236604333326336)`) which today **builds and prints the wrong
  value**. Both assert exit 1, `status == "CompileFailed"`, `message` contains
  `"128-bit struct fields and enum payloads"`, and no executable is left behind.
- **Green.** Already satisfied by slices 1-2. If (b) still passes codegen, the store guard is not
  reaching the enum construction site — fix the guard, do not weaken the test.

### Slice 4 — `vow-clif-shim` mirrors both guards

- **Red.** Two unit tests in the shim's test module driving `__vow_clif_fn_begin` /
  `add_test_inst(IOP_FIELD_GET, ITY_I128, …)` and `IOP_FIELD_SET` with an `ITY_I128` value, each
  asserting `__vow_clif_fn_end(ctx) == -1`.
- **Green.** Add the same two guards, sharing one `report_wide_aggregate_field()` helper so the
  wording cannot drift from the Rust backend's.
- **Note.** This is the self-hosted compiler's codegen path — `compiler/clif.vow` only forwards
  opcodes over FFI, so no `.vow` change is needed for the codegen half of the fix.

### Slice 5 — Verifier fails closed on 128-bit field slots (Rust)

- **Red.** `vow-verify/src/c_emitter.rs` test module: a function with a 128-bit `FieldGet` (and one
  with a 128-bit `FieldSet`) must be non-modelable, and `non_modelable_reason` must name
  `FieldGet at 128-bit width` / `FieldSet at 128-bit width`.
- **Green.** Gate the two `=> true` arms in `is_modelable` and add the matching arms to
  `first_unsupported_opcode`, using a `collect_wide_vars` pre-pass for the `FieldSet` value type.
- **Why this belongs here.** Without it, `vow verify` on the probe in §2.4 keeps emitting a false
  counterexample derived from a truncated `int64_t __vow_heap[]` slot. Same root cause, same
  fail-closed remedy, and `vow build` verifies by default.

### Slice 6 — Self-hosted verifier mirror

- **Red.** `tests/verify-skip/wide_struct_field_skipped.vow` (new), following
  `tests/verify-skip/vec_of_vec_skipped.vow`:
  ```vow
  // TEST: category unverifiable
  module WideFieldSkipped

  struct Wide { value: i128 }

  fn roundtrip(x: i128) -> i128 vow {
    ensures: result == x
  } {
    let w: Wide = Wide { value: x };
    w.value
  }

  fn main() -> i32 [io] {
    print_i64(0);
    0
  }
  ```
  Take the value as a parameter, never as a literal: a 128-bit literal lowers to `ConstI128`, which
  is *already* non-modelable, and the fixture would then pass without exercising the new gate.
  `scripts/full_test.sh` Section 4d runs this through both compilers and asserts
  `status == "Skipped"` with `compare_json` parity. Today Rust reports `VerifyFailed`.
- **Green.** Mirror slice 5 in `compiler/c_emitter.vow` (`is_modelable` line 289,
  `first_unsupported_opcode_name` line 430). The Rust file's doc comments name these as mirrors that
  "must stay in sync".

### Slice 7 — Spec, regeneration, bootstrap

- Edit the two `docs/spec/grammar.md` paragraphs (§4).
- `uv run python scripts/generate_help.py`
- `cargo build --release -p vow`
- `scripts/bootstrap.sh --skip-cargo`
- Confirm `scripts/check_help_coverage.py` (run inside `scripts/full_test.sh`) reports no drift and
  that the bootstrap still reaches a byte-identical fixed point.

## 6. Verification surface

- **No contract is written, weakened, or bounded by this change.** Nothing in `benchmarks/`,
  `examples/`, `stdlib/`, or `tests/verify*/` uses a 128-bit aggregate field (checked: the only
  128-bit fixtures are `tests/run/{i,u}128_*.vow`, all scalar; `stdlib/bignum/bignum.vow` mentions
  i128 only in a comment). So no existing proof obligation changes.
- **What ESBMC must now prove: strictly less.** Functions touching a 128-bit field move from
  "modelled on a truncated heap slot" to `Skipped`, which is the project's documented fail-closed
  posture (`VerificationSkipped` warning, build fails closed with exit 1). This removes an unsound
  model rather than adding a proof obligation.
- **New fixture:** `tests/verify-skip/wide_struct_field_skipped.vow` only. No `tests/run/` or
  `examples/` growth — those directories are for programs that must build and run, and these
  programs must not build.
- **No `tests/error/` fixture.** `scripts/full_test.sh` line 1152 feeds every `tests/error/*.vow`
  to `scripts/parity.py::compare_error`, which requires `len(diagnostics) >= 1`. A backend
  `UnsupportedOpcode` surfaces as `CompileFailed` with an empty `diagnostics[]` (the message lives
  in `message` on the Rust side and on stderr on the self-hosted side), so such a fixture would fail
  parity for a reason unrelated to this bug. End-to-end coverage therefore lives in
  `vow/tests/wide_literal_aggregates.rs` (Rust) and the shim unit tests (self-hosted). Promoting
  backend refusals to structured diagnostics is listed as a follow-up.

## 7. Risk areas

- **Binary fixed point.** The guards only add early-return error paths; they emit no CLIF on the
  success path, introduce no new map iteration, and change no `BTreeMap`/`HashMap` choice or
  stack-slot allocation order in `vow-clif-shim`. The self-hosted compiler itself contains no
  128-bit aggregate field, so the bootstrap triple must stay byte-identical. Verify with
  `scripts/bootstrap.sh --skip-cargo` plus the Section 9 triple test in `scripts/full_test.sh`.
- **`parse → print → parse` idempotency.** Untouched — no syntax, AST, or printer change.
- **Clippy.** CI runs `cargo clippy --all -- -D warnings` (no `--all-targets`). Changing
  `extend_field_store_value` to return `Result` will produce `unused_result`-adjacent lints if a
  call site drops the value; propagate with `?` at every call site including the two in-file test
  helpers.
- **Over-rejection risk.** The `FieldSet` guard keys on the *stored value's* IR type. `Option`/
  `Result`/enum tags are `ConstI64`, so tag stores are unaffected; only a genuinely 128-bit payload
  trips it. Guard against regressing `vow/tests/wide_literal_aggregates.rs::aggregate_contexts_…`,
  which is `--dump-ir`-only and must keep passing (it asserts the IR still carries full-width
  literals — the frontend behaviour is correct and must not be touched).
- **Message drift between compilers.** Two independent copies of the wording (Rust backend, shim).
  Define it once per crate as a `const` and assert on a substring, not the whole string, in tests.
- **Self-hosted / Rust verifier mirror drift.** `is_modelable` and `first_unsupported_opcode` exist
  twice. Slice 6's fixture is parity-compared, which is the mechanical check that they agree.

## 8. Out of scope (deliberately not in this PR)

- **The real 16-byte field layout.** Two-slot allocation, byte-offset-carrying field indices, and
  the widened `__vow_heap` model. Follow-up issue, referencing #526 and this one.
- **Structured diagnostics for backend refusals.** Today a `CodegenError` becomes a bare
  `CompileFailed { message }` with no `ErrorCode` or span, and the shim's text reaches only stderr,
  so Rust and self-hosted `message` fields diverge for *every* backend refusal (pre-existing, also
  true of the `Vec<i128>` refusal). Fixing that needs an FFI channel to carry the shim's message
  back into `compiler/main.vow`. Worth a follow-up; it would also unblock a `tests/error/` fixture
  for this class of failure.
- **`parse_i128` / `parse_u128`.** `docs/spec/grammar.md` line ~1106 documents them, but the Rust
  compiler rejects `parse_i128` as an undefined function. Separate spec/implementation drift, noted
  here so it is not lost; not this PR.
- **The 128-bit `vow` capture gap** (grammar.md ~307: a 128-bit binding reports `0` in the runtime
  `VowViolation` `values` map). Already documented, separate defect.
- **Any refactor of `extend_field_store_value`'s float bitcast handling, the `FieldGet` result-cast
  match, or `region.rs` field tracking.** No formatting-only or drive-by cleanups.

## 9. Validation (run as separate commands, never `&&`-chained)

```bash
cargo fmt --all
cargo clippy --all -- -D warnings
cargo test --all
uv run python scripts/generate_help.py
cargo build --release -p vow
scripts/bootstrap.sh --skip-cargo
scripts/full_test.sh
tests/run_tests.sh --no-bootstrap
build/vowc test compiler/
```

## 10. Definition of done

1. The issue's repro fails with the named message, not a CLIF dump, on both
   `./target/release/vow build --no-verify --no-cache` and `build/vowc build --no-verify --no-cache`.
2. The two-field struct probe (§2.2) and the enum-payload probe (§2.3) no longer build — they are
   refused rather than producing a truncated or corrupted value.
3. `vow verify` on the §2.4 probe reports `Skipped`, not a false counterexample, on both compilers.
4. `docs/spec/grammar.md` no longer claims a layout no compiler implements, and the regenerated
   `--help` / skill artefacts match it.
5. Bootstrap fixed point unchanged; full test suite green.
