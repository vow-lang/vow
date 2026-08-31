# Plan — #1077: `VowViolation` truncates `i128`/`u128` binding values to 64 bits

## 1. Problem restated

A runtime `vow` violation on a function with an `i128`/`u128` free variable fires correctly and
blames correctly, but the captured value in the diagnostic's `values` map is wrong. The
binding-capture path is 64-bit end to end: both backends lay each binding out as a 24-byte record
`{name: ptr, tag: u8 + 7 pad, payload: u64}`, and `vow-runtime`'s `VowBinding` reads it back at
that width. Neither `tag_for_ir_ty` has an `I128`/`U128` arm (both fall through to `_ => 0`, the
`i32` tag), and — worth correcting against the issue text — neither payload `match` has a 128-bit
arm either, so the payload falls through to `_ => iconst(I64, 0)`. The reported value is therefore
**always literally `0`**, not the low limb; the issue's repro only looks like truncation because
`0xAB << 64` happens to have a zero low limb. `docs/spec/grammar.md:307-311` already documents this
as a known, not-yet-fail-closed 128-bit gap. The fix is to widen the binding record to carry two
limbs, give both backends explicit `I128`/`U128` tag and payload arms, and teach the runtime
renderer two new tags so `values` carries the full magnitude.

## 2. Files to touch

### Runtime (the reader side of the ABI)

- `vow-runtime/src/violation.rs` — tag constants (`TAG_I32`..`TAG_U32`, lines 12-22): add
  `TAG_I128 = 11`, `TAG_U128 = 12`. Widen `ValueBinding::payload` from `u64` to `u128` and
  `format_value(tag: u8, payload: u128)` accordingly; add the two 128-bit arms.
- `vow-runtime/src/lib.rs` — `VowBinding` (`:70-76`) gains `payload_hi: u64` (24 → 32 bytes,
  align stays 8). `__vow_violation` (`:79-119`) reassembles
  `u128::from(payload) | (u128::from(payload_hi) << 64)` when building `ValueBinding`. Add a
  `const _: () = assert!(size_of::<VowBinding>() == 32);` mirroring the `VowVec` precedent at
  `vow-runtime/src/lib.rs:1277`.

### Rust stage-0 backend (the writer side)

- `vow-codegen/src/cranelift_backend.rs`
  - `tag_for_ir_ty` (`:1843-1857`) — add `IrTy::I128 => 11`, `IrTy::U128 => 12`.
  - `emit_vow_violation_body` (`:1919-1997`) — stack slot `24 * n` → `32 * n`; offsets become
    `i*32` (name), `i*32 + 8` (tag), `i*32 + 16` (payload lo), `i*32 + 24` (payload hi). Add
    `IrTy::I128 | IrTy::U128` to the payload match, producing `(lo, hi)` via
    `builder.ins().isplit(*cl_val)`. **Every** binding must store a hi limb — `iconst(I64, 0)` for
    all non-128-bit tags — because the slot is uninitialised stack memory otherwise.

### Self-hosted backend (the writer side, via the FFI shim)

- `vow-clif-shim/src/lib.rs`
  - `tag_for_ir_ty` (`:3043-3057`) — add `ITY_I128 => 11`, `ITY_U128 => 12`.
  - `emit_vow_check` (`:3099-3145`) — the identical stride/offset/`isplit`/zero-hi change.

  **No `compiler/*.vow` change is needed for the fix itself.** `compiler/clif.vow:501` passes only
  `(ctx, vow_id, description, binding_inst_ids, binding_names)` through `__vow_clif_fn_vow`; the
  shim resolves each binding's `ir_ty` from its own `inst_ty_map`
  (`vow-clif-shim/src/lib.rs:2418-2438`) and owns the record layout. `vow-clif-shim` **is** the
  self-hosted compiler's codegen backend, so fixing it there satisfies the CLAUDE.md
  "both compilers in the same session" rule. Do not go hunting in `compiler/lower.vow`.

### Spec (mandatory — this changes documented behaviour)

- `docs/spec/grammar.md:307-311` — delete the "One 128-bit gap is not yet fail-closed …" paragraph
  and replace it with the new guarantee: a 128-bit `vow` binding reports its full magnitude in the
  `values` map. Add the interoperability caveat (see §5).
- Regenerated, never hand-edited: `vow/src/skill.rs`, `compiler/main.vow`,
  `skills/vow/reference/grammar.md`. Produce them with
  `uv run python scripts/generate_help.py`. `scripts/full_test.sh:1222` fails the build on drift.
- `docs/spec/schemas/vow-violation.schema.json` — **no type change** (see §5); optionally extend
  the `values` `description` with one sentence on 128-bit magnitudes. If touched, re-run the
  generator (the schema is embedded in `compiler/main.vow:8503+` and `vow/src/skill.rs`).

### Tests / fixtures

- `vow-runtime/src/violation.rs` (`mod tests`).
- `vow-codegen/tests/e2e.rs` (next to `vow_violation_reports_u8_variable_value`, `:495`).
- `vow-codegen/src/cranelift_backend.rs` (`mod tests`, next to
  `compile_vow_captures_all_phase3_integer_widths`, `:7034`).
- `vow-clif-shim/src/lib.rs` (`mod tests`, next to the streamed-FFI narrow-captures test, `:4894`).
- `tests/debug/i128_requires_violation.vow` — **new** fixture, run by `tests/run_tests.sh` Phase 4.
- `scripts/full_test.sh` — new Rust-vs-self-hosted parity block in Section 5 (after the
  `cast_in_contract_violation` block ending at `:968`).

## 3. TDD slices

Four commits, one PR. Slices 2 and 3 are deliberately separated so the tree is green at every
commit: the ABI stride change cannot be split across crates (a state where the shim writes 24-byte
records and the runtime reads 32-byte ones produces garbage in self-hosted debug binaries), but the
*plumbing* widening and the *128-bit semantics* can be.

### Slice 1 — renderer learns two 128-bit tags (pure, no ABI change)

- **Test** — `vow-runtime/src/violation.rs::tests`:
  - extend `format_value_renders_each_tag` with
    `format_value(TAG_I128, i128::MIN as u128) == "-170141183460469231731687303715884105728"`,
    `format_value(TAG_U128, u128::MAX) == "340282366920938463463374607431768211455"`, and the
    issue's own value `3154393236604333326336` (`0xAB << 64`) under `TAG_I128`.
  - new `render_violation_reports_full_128_bit_magnitude`: one `ValueBinding` with `TAG_U128` and
    `u128::MAX`; assert the JSON line contains `"x":340282366920938463463374607431768211455`, that
    the human line contains `x=340282366920938463463374607431768211455`, and that the line is
    single-line valid JSON via `serde_json::from_str`. **Do not** assert
    `json["values"]["x"] == u128::MAX` — `serde_json` without `arbitrary_precision` folds
    out-of-`u64`-range integers into `f64`, so assert on the raw string for the value and use the
    parse only as a validity check.
  - regression: every existing `format_value` / `render_violation` assertion must still pass
    unchanged.
- **Production** — add `TAG_I128 = 11` / `TAG_U128 = 12`; change `ValueBinding::payload` to `u128`
  and `format_value`'s parameter to `u128` (bind `let lo = payload as u64;` at the top so the
  existing narrow arms stay literally the same expression); add the two new arms; keep the
  unknown-tag fallback as `0x{payload:x}` (unchanged output for the existing `0xdead` case).
  In `vow-runtime/src/lib.rs`, `__vow_violation` builds `payload: u128::from(b.payload)` — the hi
  limb does not exist yet, so end-to-end behaviour is unchanged.

### Slice 2 — widen the binding record to 32 bytes (plumbing only, hi limb always 0)

- **Test** — `vow-runtime/src/lib.rs::tests` (or a new `#[test]` beside the existing layout
  asserts): `vow_binding_carries_a_high_limb` asserting
  `size_of::<VowBinding>() == 32`, `align_of::<VowBinding>() == 8`, and
  `offset_of!(VowBinding, payload) == 16`, `offset_of!(VowBinding, payload_hi) == 24`.
  The existing narrow-value end-to-end tests (`vow_violation_reports_u8_variable_value`,
  `vow_violation_reports_variable_values`, `tests/debug/narrow_requires_violation.vow`) are the
  regression gate: they must keep passing with the new stride.
- **Production** — add `payload_hi: u64` to `VowBinding` + the `size_of` const assert; reassemble
  the `u128` in `__vow_violation`; change the stride to 32 and add an explicit
  `stack_store(.., iconst(I64, 0), slot, i*32 + 24)` in **both** `vow-codegen` and `vow-clif-shim`.
  No tag or payload semantics change in this slice.

### Slice 3 — 128-bit tags and two-limb payload in both backends

- **Test (Rust backend)**
  - `vow-codegen/src/cranelift_backend.rs::tests` — `compile_vow_captures_128_bit_widths`, modelled
    on `compile_vow_captures_all_phase3_integer_widths` (`:7034`): params `[Ty::I128, Ty::U128]`,
    two `GetArg`s bound as vow bindings, `ConstBool(true)` predicate, `VowRequires`; assert
    `compile_module(.., BuildMode::Debug, TraceMode::Off).is_ok()`. This is compile-only and runs
    everywhere, including sandboxes with no linkable runtime.
  - `vow-codegen/tests/e2e.rs` — `vow_violation_reports_i128_variable_value`, modelled on
    `vow_violation_reports_u8_variable_value` (`:495`): `bad(x: i128) requires x < 0`, called with
    `ConstI128(3154393236604333326336)`; assert exit `134`, blame `Caller`, and
    `stderr.contains(r#""x":3154393236604333326336"#)`. Add a `u128` twin using `u128::MAX` if it
    costs nothing. Note: e2e tests self-`SKIP` when `vow-runtime` is not linkable, so this test is
    additional evidence, not the only gate — hence the compile-only unit test above.
- **Test (self-hosted backend)**
  - `vow-clif-shim/src/lib.rs::tests` — clone the narrow-captures streamed-FFI test (`:4860-4905`)
    with `ITY_I128`/`ITY_U128` binding instructions; assert `__vow_clif_fn_vow` and
    `__vow_clif_fn_end` both return `0`.
  - `tests/debug/i128_requires_violation.vow` — **new**, exactly the issue's repro:
    ```
    // TEST: exit 134
    // TEST: stderr "VowViolation"
    // TEST: stderr "\"x\":3154393236604333326336"
    ```
    with a `u128` binding in the same contract if it can share one violation (otherwise a second
    fixture `u128_requires_violation.vow` asserting `u128::MAX`). Built `--mode debug --no-verify`
    by `tests/run_tests.sh` Phase 4 (`:824-877`), so verifier 128-bit gaps do not interfere.
- **Production** — the `I128`/`U128` arms in both `tag_for_ir_ty`s (`11` / `12`, matching the
  runtime constants exactly), and the `isplit`-based two-limb payload in both capture loops.

### Slice 4 — spec, regeneration, and the parity gate

- **Test** — new Section 5 block in `scripts/full_test.sh`, copied from the
  `u8_requires_violation` block (`:919-941`): build `tests/debug/i128_requires_violation.vow` in
  debug mode with **both** `$RUST` and `run_self`, run both, and require exit `134` plus
  `grep -qF` on `VowViolation`, `Caller`, and `"x":3154393236604333326336` in each. This is the
  test that proves the two compilers agree, which is the point of the issue's "both backends
  behave the same way".
- **Production** — rewrite `docs/spec/grammar.md:307-311`; run
  `uv run python scripts/generate_help.py`; rebuild (`cargo build --release -p vow`, then
  `scripts/bootstrap.sh --skip-cargo`). Confirm `uv run python scripts/generate_help.py --check`
  and `scripts/check_help_coverage.py` are clean.

## 4. Verification surface

**ESBMC is not involved.** The binding-capture path is a debug-mode runtime artefact; it has no
representation in the verifier C model. `vow-verify/src/c_emitter.rs` and `compiler/c_emitter.vow`
carry no binding-capture code (confirmed: no `binding` symbol in `compiler/c_emitter.vow`), so no
new proof obligations arise and no existing proof changes. No contract in this repo is
strengthened, weakened, or bounded by this change.

Fixture growth is confined to `tests/debug/` (one, possibly two, new `.vow` files) and to the
Rust-vs-self-hosted parity block in `scripts/full_test.sh`. `tests/verify/` and `benchmarks/` are
untouched — a contracted function containing a 128-bit *constant* is still reported `Skipped`
(`unsupported opcode ConstI128`), which this PR does not change and does not need to.

## 5. Risk areas

**JSON representation — the one real design call.** A 128-bit value cannot be rendered as a JSON
number inside the IEEE-754 interoperable range that RFC 8259 §6 recommends. Two options:

- *(chosen)* **bare decimal integer**, e.g. `"x":340282366920938463463374607431768211455`. This is
  what `TAG_U64` already does for values above 2^53 (`format_value` renders `u64::MAX` bare today),
  so it is the consistent choice; it keeps the `values` map homogeneous, needs no schema change,
  and is exactly right for the primary consumer — an agent reading the raw digits. Cost: a strict
  `serde_json`/JS consumer silently rounds the value to `f64`.
- *(rejected)* **decimal string for 128-bit only**, matching the `counterexamples[].values`
  convention (`docs/spec/cli.md:330`). Lossless for every parser, but it makes the `values` map
  heterogeneous, requires widening the schema's `additionalProperties.type` union, and diverges
  from how the same envelope already renders `u64`.

Document the caveat in one sentence in `grammar.md` ("consumers needing exact 128-bit values must
use an arbitrary-precision JSON number parser"). If a reviewer prefers strings, it is a one-line
change in `format_value` plus a schema edit — record the decision in the PR body.

**Binary fixed point.** The change is confined to `--mode debug`/`--mode sanitize` codegen
(`ctx.mode == 1 || ctx.mode == 3` in the shim); the bootstrap compiles `build/vowc` without vow
checks, so `scripts/bootstrap.sh`'s byte-identical fixed point is not affected. No `HashMap`
introduced, no iteration order changed: the capture loop still walks the `captures` `Vec` in index
order, and `slot_map` stays a `BTreeMap`. Keep it that way — do not introduce a map keyed on tag or
type in either backend.

**Stack-slot layout.** The 32-byte stride must be applied atomically in all three places
(`vow-runtime`, `vow-codegen`, `vow-clif-shim`) — this is why slice 2 is one commit. The static
`size_of::<VowBinding>() == 32` assert is the cheap tripwire; add it. The slot's alignment shift
stays `3` (8-byte): storing two `I64` limbs rather than one `I128` avoids needing 16-byte alignment
and is endianness-explicit, so do **not** switch to a single `stack_store` of an `I128`.
Uninitialised stack is the other hazard — every binding, at every tag, must write both limbs.

**Tag-table drift.** The tag numbering now lives in three independent tables
(`vow-runtime/src/violation.rs`, `vow-codegen::tag_for_ir_ty`, `vow-clif-shim::tag_for_ir_ty`).
Adding `11`/`12` to only two of them produces a wrong-but-plausible rendering. The
`tests/debug/` fixture plus the `full_test.sh` parity block are the cross-check; make sure both
land. Unifying the three tables is a genuine deepening opportunity but is a refactor — see §6.

**Clippy (`cargo clippy --all -- -D warnings`).** The payload match now yields a `(lo, hi)` pair;
prefer returning a tuple from the existing `match` over adding a helper with a long parameter list
(`clippy::too_many_arguments` already needs an `#[allow]` on the shim's `emit_vow_check`). Note
per the repo memory that CI runs without `--all-targets`, so match CI exactly rather than chasing
test-module lints.

**`parse → print → parse` idempotency.** Untouched — no syntax, AST, or printer change.

**Help/skill drift gate.** Editing `docs/spec/grammar.md` without re-running
`scripts/generate_help.py` fails `scripts/full_test.sh:1222` and `:1225`. Regenerate, and commit
`vow/src/skill.rs`, `compiler/main.vow`, and `skills/vow/reference/grammar.md` together with the
spec edit.

## 6. Out of scope

- **Deduplicating the three `tag_for_ir_ty` / tag-constant tables** into one shared definition.
  Real, but a refactor bundled into a bug fix — file a follow-up.
- **128-bit values in aggregates** (`Vec<i128>`, struct fields, enum payloads, `Option<i128>`).
  Still fail-closed and still documented as such in `grammar.md`; untouched here.
- **Modelling `ConstI128`/`ConstU128` in the verifier** so contracted functions with 128-bit
  constants stop reporting `Skipped`. Separate concern, separate issue.
- **Rendering 128-bit values in ESBMC counterexamples** (`counterexamples[].values`). Different
  code path (`vow-verify`), different representation (already strings).
- **Re-encoding the whole `values` map as strings**, and any other change to how `i64`/`u64`/float
  values are rendered. Behaviour-preserving for every existing tag is a hard requirement.
- **Non-finite float and unknown-tag JSON validity** in `format_value` — pre-existing, already
  tracked under #436 in that file's own doc comment.
- Formatting-only churn, unrelated cleanups in `cranelift_backend.rs` / `lib.rs`, and any change
  to `symphony/` or `build/`.

`#526` is the epic tracker and **must not be closed** by this PR. Reference it; close only `#1077`.

## 7. PR shape

Squash-merge only; the PR title is the commit that lands on `main` and must satisfy
Conventional Commits (lower-case subject, no trailing period, header ≤ 100 chars including the
` (#N)` suffix). Suggested title:

```
fix(codegen): report full 128-bit magnitude in VowViolation bindings
```

Before pushing, run as separate commands (never `&&`-chained): `cargo fmt --all`,
`cargo clippy --all -- -D warnings`, `cargo test --all`, `scripts/bootstrap.sh --skip-cargo`,
`tests/run_tests.sh`, `scripts/full_test.sh`. Delete `PLAN.md` (`git rm PLAN.md`) before opening
the PR.
