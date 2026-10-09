# Plan: preserve f32/f64 bits in the ESBMC aggregate payload C model (#1513)

## Goal
Make both C emitters (`vow-verify/src/c_emitter.rs`, `compiler/c_emitter.vow`) move `f32`/`f64`
values through the `int64_t` aggregate slots by IEEE-754 bit pattern instead of numeric
conversion, matching native codegen (`vow-codegen/src/cranelift_backend.rs:462-483` store,
`:1755-1770` load: f64 raw 64 bits, f32 low 32 bits zero-extended). The emitted C stays
byte-identical across the two compilers. Closes the "unsupported" caveat in `docs/spec/grammar.md`.

## Current behaviour (verified by reading)
- `Option::Some(2.5)` / `Result::Ok(x)` / struct fields lower to `RegionAlloc` + `FieldSet`/`FieldGet`
  over the per-function `int64_t __vow_heap[]`. Rust: `c_emitter.rs:2129-2177`; Vow:
  `c_emitter.vow:1985-2052`.
- `FieldSet` emits `__vow_heap[vB + i] = vV;` and `FieldGet` emits `vN = __vow_heap[vB + i];`:
  a C double<->int64 conversion. `2.5` is stored as `2`, so `ensures: result == 2.5` yields a
  spurious counterexample and a false `result == 2.0` is "proved". No gate refuses it
  (`field_access_is_wide` only refuses 128-bit).
- `__vow_option_t { int64_t tag; int64_t payload; }` (`c_emitter.rs:3267`, `c_emitter.vow:3673`) and
  `vals[]`/`data[]` of vec/hashmap/btreemap are also `int64_t`: same hazard for `Vec<f64>`, `HashMap<_, f64>`.
- ESBMC 8.3.0 is on this host (`~/.local/bin/esbmc`). Probed: union-based `double<->int64_t`
  and `float<->uint32_t` punning round-trips exactly and distinguishes 2.5 from 2.0, including through
  `static inline` helper functions and nondet doubles; `memcpy(&int64_slot, &double, 8)` gave a spurious
  FAILED. So use unions.

## Assumptions
- Union-based `static inline` helpers (not `memcpy`, not inline compound-literal unions): the only
  form probed to work in ESBMC 8.3.0 (best guess, empirically tested).
- Helpers are emitted only when a module uses them, via the existing `ModelHelpers`
  (`c_emitter.rs:2772-2785`) / `append_model_helpers` (`c_emitter.vow:3586`) mechanism, so the C for
  every existing non-float fixture is unchanged (no cache/parity churn).
- Vec/HashMap/BTreeMap float elements are included (slice 4). Reason: the Option projection
  (`FieldGet` on `option_vars`) is fed by map `get`; decoding there without encoding map inserts would be
  inconsistent, and `Vec<f64>` has the same bug. Slices are independently committable; slice 4 may be
  dropped without breaking 1-3.
- NaN: `ensures: result == x` is false for NaN `x` under IEEE; fixtures use the existing finite
  guard idiom `requires: x - x == x - x` (see `tests/verify/float_arithmetic.vow`). That is a semantic
  precondition, not an ESBMC bound.
- Native verifier (`compiler/vc_*.vow`) is exempt from parity (CLAUDE.md scoped exception) and is not touched.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `vow-verify/src/c_emitter.rs` | slot encode/decode helpers, `FieldSet`/`FieldGet`, vec/map sites, preamble, `ModelHelpers` | 2136-2177, 1577-1630, 1944/2018/2026, 2772-2785, 3246-3275 |
| `compiler/c_emitter.vow` | same, byte-identical | 1985-2052, 2632-2747, 3586-3673 |
| `docs/spec/grammar.md` | rewrite the "Built-in Enums" ESBMC caveat | 1010-1018 |
| `skills/vow/reference/grammar.md` | mirrored copy of the same paragraph | 1016 |
| `compiler/main.vow`, `vow/src/skill.rs` | generated embeds of that paragraph (via `scripts/generate_help.py`) | main.vow:4856,10894; skill.rs:2413,8429 |
| `tests/verify/float_enum_payload.vow` [new] | proves float payload round-trips | n/a |
| `tests/verify-fail/float_enum_payload_wrong.vow` [new] | false clause must be refuted | n/a |
| `compiler/tests/test_c_emitter.vow` | self-hosted emitter unit tests | n/a |

## Steps (TDD slices; each = red test, minimal green, commit)

### Slice 1 - heap path: struct fields and enum payloads (FieldSet/FieldGet), both compilers
1. Red (Rust): unit tests in `vow-verify/src/c_emitter.rs` `mod tests` next to `emit_float_arithmetic`
   (line ~5355), built with the same `inst(...)` builders: `RegionAlloc` + `ConstF64(2.5)` + `FieldSet`
   idx 1 + `FieldGet` idx 1 with `ty=F64`. Assert C contains `__vow_f64_to_slot(v` in the store,
   `v_n = __vow_f64_from_slot(__vow_heap[v_b + 1]);` in the load, and the two helper definitions in
   the module preamble. Repeat for F32 (`__vow_f32_to_slot`/`__vow_f32_from_slot`). Add a negative test:
   an i64 `FieldSet`/`FieldGet` module emits no `__vow_f*_slot` text (existing C unchanged).
2. Green (Rust): add `fn float_slot_kind(ty: Ty) -> Option<FloatSlot>` (F32/F64 only).
   - `FieldSet` (2141-2150): `let ty = operand_ty(val, inst_by_id)`; wrap value in
     `__vow_f{32,64}_to_slot(v{val})` for float types, else unchanged. Reuses `operand_ty` (`:292`).
   - `FieldGet` heap branch (2166-2171): wrap `__vow_heap[...]` in `__vow_f{32,64}_from_slot(...)` when
     `inst.ty` is float. Leave the `option_vars` branch (2158-2165) untouched in this slice.
   - Add `float_slots: FloatSlotNeeds {f32: bool, f64: bool}` to `ModelHelpers` (`:2775`), filled by a
     `scan_float_slot_needs(funcs)` walking `FieldSet` (value type via a per-function id->ty map) and
     `FieldGet` (inst.ty), skipping accesses with `InstData::WideSlot`.
   - `emit_c_preamble` (`:3246`): after the `__vow_option_t` typedef (3267) and before shift helpers,
     emit per used flavor, in fixed order f64 then f32:
     ```
     static inline int64_t __vow_f64_to_slot(double value) { union { double f; int64_t i; } u; u.f = value; return u.i; }
     static inline double __vow_f64_from_slot(int64_t slot) { union { double f; int64_t i; } u; u.i = slot; return u.f; }
     static inline int64_t __vow_f32_to_slot(float value) { union { float f; uint32_t i; } u; u.f = value; return (int64_t)u.i; }
     static inline float __vow_f32_from_slot(int64_t slot) { union { float f; uint32_t i; } u; u.i = (uint32_t)slot; return u.f; }
     ```
     Extend the "emit blank line after helper group" condition (3274) to include the float group.
3. Red (Vow): mirror tests in `compiler/tests/test_c_emitter.vow` (same builders, `ITY_F64()`/`ITY_F32()`).
4. Green (Vow): mirror in `compiler/c_emitter.vow`: float-slot helper-name selection beside
   `field_access_is_wide` (1306); `IOP_FIELD_GET` heap branch (2005-2013) and `IOP_FIELD_SET` (2045-2052)
   wrap with the helpers; scope scan + emission in `append_model_helpers` (3586-3626) /
   `cemit_preamble_with_limits` (3629) in the same canonical order and spelling as Rust. Reuse
   `vec_contains_i64`, `str3`/`str6`. Vow has no closures/generics: use a flat `fn` returning an `i64` kind.
5. Fixtures (`tests/verify/`, picked up by `full_test.sh` Section 2c parity glob automatically):
   - `float_enum_payload.vow` [new], header `// TEST: category model-drift`: `fn some_f64(x: f64) -> f64`
     with `requires: x - x == x - x`, `ensures: result == x`, body `match Option::Some(x) {...}`; a
     constant case `ensures: result == 2.5`; `Result<f64, f32>` Ok/Err extraction for both widths; a user
     struct with an `f64` field set then read. These fail today with a spurious CE and pass after.
   - `tests/verify-fail/float_enum_payload_wrong.vow` [new]: `ensures: result == 2.0` on a body that
     returns `Option::Some(2.5)`'s payload; must report a counterexample (copy the
     `counterexample-*` directive set from `tests/verify-fail/clamp_wrong_op.vow`). Today it wrongly proves.
6. Commit: `fix(verify): preserve float bits in aggregate slot model` (subject lower-case, <100 chars).

### Slice 2 - Option projection + containers (Rust + Vow together; optional/separable)
1. Red: unit tests that `Vec<f64>` push/get/set, `HashMap<i64, f64>`/`BTreeMap` insert, and the
   `FieldGet` on an `option_vars` source with `ty=F64` use `__vow_f64_to_slot`/`from_slot`; integer
   containers unchanged.
2. Green Rust: wrap stored value at `__vow_vec_push_val` (1585), `__vow_vec_set_val` (1629), map
   insert sites (1944, 2018, 2026) using `operand_ty` of the value operand; wrap loads at
   `__vow_vec_get_val` (1608) and the option projection (2163: `.payload` -> `from_slot` when
   `inst.ty` float). Extend `scan_float_slot_needs` to those call sites. Green Vow: mirror at
   `c_emitter.vow` 2632, 2687, 2733, 2747 and the vec cases around 183-190 / 279-312 dispatch.
3. Fixtures: add `Vec<f64>` push/index and `HashMap<i64, f64>` insert/get cases to
   `tests/verify/float_enum_payload.vow` (or a sibling `float_container_payload.vow`) plus a verify-fail twin.
4. Commit: `fix(verify): preserve float bits in vec and map slot model`.

### Slice 3 - docs and generated embeds
1. `docs/spec/grammar.md:1010-1018`: replace "still uses integer slots ... unsupported" with: the ESBMC
   model stores `f32`/`f64` payloads and struct fields by bit pattern (f64 raw 64 bits, f32 zero-extended
   32 bits), so contracts over float payloads verify; NaN follows IEEE equality. Make the same edit in
   `skills/vow/reference/grammar.md`.
2. Run `uv run python scripts/generate_help.py` to regenerate `compiler/main.vow` and `vow/src/skill.rs`
   embeds; run `scripts/check_help_coverage.py` (no drift expected).
3. Commit: `docs(spec): document faithful float payload verification`.

## Testing / verification commands
- `cargo test -p vow-verify` (new c_emitter tests), `cargo clippy --all --all-targets -- -D warnings`,
  `cargo fmt --all --check`.
- `build/vowc test compiler/tests/test_c_emitter.vow` (after `scripts/bootstrap.sh`).
- `cargo build --release -p vow && scripts/bootstrap.sh --skip-cargo` then re-run with `--no-cache` on
  the final head SHA; record the SHA.
- `python3 scripts/parity.py c target/release/vow build/vowc tests/verify/float_enum_payload.vow
  tests/verify-fail/float_enum_payload_wrong.vow` plus the full Section 2c set (`scripts/full_test.sh`;
  ~40 min, run in background and poll).
- Real ESBMC run: `build/vowc verify tests/verify/float_enum_payload.vow` must prove all functions;
  `tests/verify-fail/...` must produce the counterexample. Use `VOW_CACHE_DIR=$(mktemp -d)` (stale
  compile cache can mask codegen changes). Use `-j` caps for cargo (shared memory budget).

## Verification surface
ESBMC must prove: `result == x` after a `Some(x)`/`Ok(x)`/`Err(x)` + extraction for arbitrary finite `x`
(f64 and f32); `result == 2.5` for a constant payload; struct f64 field set/get; and refute
`result == 2.0`. Contracts stay semantic (only the finite-guard requires, no `--unwind`-style bounds).
Fixtures under `tests/verify/` and `tests/verify-fail/` grow; no `tests/run/` or `examples/` change
(`tests/run/issue1124_scalar_payloads.vow` already covers native behaviour).

## Risks
- Byte parity: Rust and Vow must emit identical helper text, order (f64 before f32; to before from),
  trailing blank-line rule and call spelling. Mitigation: mirror in one sitting; parity script over the new
  fixtures; unit tests assert exact substrings in both suites.
- Unchanged C for non-float programs: helpers are scan-gated; negative test pins it. Otherwise every
  verify-cache key and existing substring-asserting test moves.
- `operand_ty` defaults to `I64` for unknown ids (`c_emitter.rs:292`): an untracked float operand would
  silently stay lossy. Mitigation: scan and emit both use the same id->ty source; add a test with a
  `GetArg` float and a Phi-sourced float value.
- Mixed consistency between slices: do not decode the option projection without encoding map/vec
  stores (slice 2 lands atomically for those sites).
- Bootstrap fixed point: Vow emitter change must avoid HashMap iteration order; use ordered scans
  (existing code uses index loops) so `build/vowc` stays byte-identical.
- Clippy `--all-targets` gate covers new tests; keep new functions small (<40 lines each).
- Unrelated pre-existing failures to verify against clean `origin/main` before blaming this change:
  u64_marker_propagation / contracts_tmp_cleanup, concrete-block-region-parity, ~8 vow-crate run tests in sandbox.
- Coordination with epic #1398: ESBMC emitters are deleted at P6; this is a bounded fix that stays
  in stage 0 under the parity rule. No `vc_*.vow` change.

## Out of scope
- Float `%` (`RemF32/RemF64` stays "not modelled"), non-finite float constants, 128-bit payloads
  (`ModelIssue::Wide` unchanged), float comparison codegen (#1508), `?` on `Result`.
- Refactoring the emitter, splitting `c_emitter.rs`, formatting, or changing nondet ranges.
- Native verifier (`vc_*.vow`) float modelling, and any language/type-system change.
