# Plan: Per-Function `source_file` for Multi-Module Region Diagnostics (#254)

## Goal
Thread the originating source-file path per-function through lowering so that
`RegionConflict` and `RegionLinear` diagnostics emitted by the region pass
label the correct file (and offset into that file) when imported modules
contribute to the merged AST. Today's pipeline labels every region diagnostic
with the *root* file path; the bug is latent because the conflict shape is
unreachable on the corpus, but lands as a user-visible regression once #204
exposes store-effect inference end-to-end.

## Key Files (verified)

| File | Role | Lines of Interest |
|------|------|-------------------|
| `vow-ir/src/types.rs` | `Function` struct gets a new field | 294-308 |
| `vow-ir/src/lower/mod.rs` | `lower_module` signature gains `item_files` | 3087 |
| `vow-ir/src/region.rs` | Drop `&str` parameter from inference; read per-function | 29 (module docstring), 54, 174, 180, 379, 1354-1424; test helper at 1648; **19** call sites `infer_regions(&mut m, "test.vow")` |
| `vow-ir/src/serialize.rs` | Bump `MODULE_VERSION`; encode/decode `source_file` | 27, 635-668, 670-737, 823 (encoder header — uses `MODULE_VERSION` constant directly), 855, 924, 1076, 1200 |
| `vow-ir/src/printer.rs` | Test fixture `make_func` | 313 |
| `vow-ir/src/validator.rs` | Test fixture `make_func` | 188 |
| `vow-ir/src/types.rs` | Inline test fixtures `simple_function` / `simple_function_with_instructions` | 446, 495 |
| `vow/tests/region_summary_equivalence.rs` | External test fixture + `infer_regions` call | helper at 186, `Function {` literal at 187, `infer_regions(&mut m, "test.vow")` at 258 |
| `vow/src/module_loader.rs` | `merge_modules` returns `(Module, Vec<String>)` | 88-106 |
| `vow/src/frontend.rs` | Wire `item_files` through; drop path arg from `infer_regions` | 147, 173, 182 |
| `compiler/ir.vow` | `IrFunction.source_file`; `ir_function_new` signature; **`MODULE_VERSION()` definition** | 164 (`MODULE_VERSION` const), 258, 356-357 (constructor body) |
| `compiler/lower.vow` | `lower_module_vow` accepts `item_files`; `lower_function_vow` threads it via `ctx.file` | 60 (placeholder, leave unchanged), 3128, 3139, 3259, 3475-3476 |
| `compiler/region.vow` | Drop `path` parameter; read `f.source_file`; preserve field on `IrFunction { ... }` literal | 27, 214, 811 |
| `compiler/frontend.vow` | `load_frontend_deps` tracks per-item paths; merged Module passes parallel Vec | 72-101, 103-143, 145-198 |
| `compiler/module_io.vow` | Encode/decode `source_file` (call sites of `MODULE_VERSION()` already pick up the bump) | 903, 965, 994, 1044, 1101 |
| `tests/multi/vmod_decode_bad_version/main.vow` | Pinned `bytes.push(2)` collides with new `MODULE_VERSION=2` | line 13 |
| `vow-ir/src/region.rs` (NEW test) | Cross-file diagnostic labeling proof | append after `region_conflict_alloc_into_param_via_callee_store_effect` (~2360) |

## Steps

### Phase A — Rust types and test fixtures

1. **Add field to `Function`.** `vow-ir/src/types.rs:295-308`. Append `pub source_file: String` after `summary`. Keep `Clone`, `PartialEq`, `Debug` derives.
2. **Update production-side test helper `function()`.** `vow-ir/src/region.rs:1648-1668`. Add `source_file: "test.vow".to_string()` so the existing test at line 2355 (`assert_eq!(c.primary.file, "test.vow")`) keeps passing without churning every caller.
3. **Update `make_func` test fixture.** `vow-ir/src/validator.rs:188` and `vow-ir/src/printer.rs:313` — add `source_file: String::new()`.
4. **Update inline `Function { ... }` literals** in `vow-ir/src/serialize.rs:1076`, `:1200`, `vow-ir/src/types.rs:446`, `:495`, and `vow/tests/region_summary_equivalence.rs:187`. Add `source_file: String::new()` (or `"test.vow".to_string()` for the latter — use whichever the file's surrounding tests expect).

### Phase B — Rust lowering and threading

5. **`LowerCtx::new` populates `source_file`.** `vow-ir/src/lower/mod.rs:224` — the *only* production-path `Function {...}` literal. `LowerCtx::new` already takes `file: String` at line 211; just add `source_file: file.clone()` to the literal. No signature change here.
6. **`lower_module` signature change.** `vow-ir/src/lower/mod.rs:3087`. Replace `file: &str` with `item_files: &[String]`. Inside the FnDef collection (`fn_items: Vec<&FnDef>` at 3088-3100), switch to `enumerate().filter_map(|(idx, item)| ...)` so each retained `&FnDef` carries its original index in `module.items`. At the `lower_function(fn_def, file, ...)` call site (line 2992 → currently invoked from somewhere in the lowering loop in `lower_module`; locate via `grep -n "lower_function(" vow-ir/src/lower/mod.rs`), pass `&item_files[orig_idx]` instead of the shared `file`. The threading then reaches `LowerCtx::new`'s `file` parameter, which step 5 forwards into the `Function` literal.
7. **`merge_modules` returns `(Module, Vec<String>)`.** `vow/src/module_loader.rs:88`. For each `(path, module)` in `graph.modules` push `path.to_string_lossy().into_owned()` once per item, in lockstep with the existing `all_items.extend(module.items.clone())` (replace the `extend` with a per-item loop). Update the docstring + the merge_modules unit tests in `vow/src/frontend.rs::tests` to consume the tuple.
8. **`prepare_frontend` rewires.** `vow/src/frontend.rs:147,173,182`:
   - `let (ast, item_files) = module_loader::merge_modules(graph);`
   - `vow_ir::lower_module(&ast, &item_files, &string_exprs)`
   - `vow_ir::infer_regions(&mut module)` — drop the second argument.

### Phase C — Rust region pass

9. **Drop `source_file: &str` from the region pass.** `vow-ir/src/region.rs`:
   - Public `infer_regions(module, source_file)` at line 54 → `infer_regions(module)`.
   - `check_linear_regions` (174), `check_function_linear_regions` (180), `emit_live_linear_errors` (379), and the inner-iteration helpers that thread `source_file` lose the parameter.
   - Inside `check_function_linear_regions` and `emit_live_linear_errors`, read `func.source_file.clone()` (already has `&Function`).
   - For `check_store_conflict` (line 1362): the call chain is `analyze_function` (854) → `handle_inst` → `check_store_conflict`. `analyze_function` already has `func: &Function`. Add `source_file: &str` as a parameter to `check_store_conflict` and pass `&func.source_file` from `analyze_function` (single read, no clone-per-iteration).
   - Update the **19** `infer_regions(&mut m, "test.vow")` call sites — mechanical sed-style edit (drop the `, "test.vow"`).
   - **Update the external test** `vow/tests/region_summary_equivalence.rs:258` — same mechanical drop.
   - Delete the multi-module limitation comment block at lines 1354-1361 (the bug it documents is fixed).
   - Update the module-level docstring at `vow-ir/src/region.rs:29` to drop the mention of the `source_file` flow (the field now lives on `Function`).
   - **Verification check:** `region_conflict_alloc_into_param_via_callee_store_effect` (line 2286) must still pass. Its assertions at lines 2355-2358 read `c.primary.file == "test.vow"`; the helper-default in step 2 keeps this true.

### Phase D — Rust serialization

10. **Bump `MODULE_VERSION`.** `vow-ir/src/serialize.rs:27` from `1` → `3` (per resolution in step 13). The encoder header at `:823` and the decode gate at `:855` automatically pick up the constant.
11. **Encode `source_file` in `write_function`.** `:635-668`. Append `write_string(out, &f.source_file);` after `write_region_summary(out, &f.summary);`.
12. **Decode in `read_function`.** `:670-737`. Read `let source_file = r.string()?;` after `read_region_summary`. Add `source_file` to the `Ok(Function { ... })` constructor.
13. **Update the version-mismatch test.** `:855` (decoder rejects mismatched version) and `:924` (`assert_eq!(MODULE_VERSION, 1)`) — flip to `3` (matching the bump in step 10). **Decision (resolved):** `tests/multi/vmod_decode_bad_version/main.vow:13` currently pushes `bytes.push(2)` as the "bad" version. Bumping `MODULE_VERSION` to `2` makes that byte *valid* and inverts the test. Pick **option A**: bump `MODULE_VERSION` to `3` instead of `2`, leaving the bad-version fixture untouched. This is one mechanical edit larger than option B (changing the fixture) but eliminates risk of decoding the fixture as a real version. Apply the same value to `compiler/ir.vow:164`.
14. **Add a serialize round-trip test.** Construct a `Function` with `source_file: "lib.vow".to_string()`, encode, decode, assert equality of the field.

### Phase E — Rust regression test (the proof)

15. **Add cross-file diagnostic test in `vow-ir/src/region.rs`.** New `#[test] fn region_conflict_uses_callee_function_source_file()` after line 2358. Build the same two-function pattern as `region_conflict_alloc_into_param_via_callee_store_effect` but:
    - Use a custom helper `function_with_source_file(...)` (or extend `function()` to accept an optional path).
    - Set `f1` (the analyzing function holding the alloc-→param-via-callee shape) to `source_file: "lib.vow".to_string()`.
    - Set `f0` (the callee) to `source_file: "main.vow".to_string()`.
    - After `infer_regions(&mut m)`, assert `c.primary.file == "lib.vow"` and every secondary span's `file == "lib.vow"` (the diagnostic is emitted at the call site in `f1`).
16. **(Deferred) End-to-end multi-module test.** Skip `tests/multi/region_conflict_*/` for now: the conflict shape is unreachable through the user pipeline until #204. Capture this in the commit message; the IR-level test in step 15 is the contract.

### Phase F — Rust gates

17. `cargo build --all`
18. `cargo test --all`
19. `cargo clippy --all -- -D warnings`
20. `cargo fmt --all --check`

Iterate until green.

### Phase G — Self-hosted mirror

21. **Add `source_file` to `IrFunction`.** `compiler/ir.vow:258`. Append `source_file: String,` (struct field order matters for the self-hosted layout — adding at the end is safest).
22. **Update `ir_function_new`.** `compiler/ir.vow:356-357`. New signature: `fn ir_function_new(id: i64, name: String, return_ty: i64, effects: i64, source_file: String) -> IrFunction`. Add `source_file: source_file` to the literal at `:357`.
23. **Update all `ir_function_new` callers (compile-error driven):**
    - `compiler/lower.vow:60` — `lctx_new` placeholder. Pass `String::from("")`. The IrFunction constructed here is overwritten inside `lower_function_vow` at line 3143 (`ctx.func = func`), so the placeholder value never reaches diagnostics. Resolved.
    - `compiler/lower.vow:3139` — `lower_function_vow` reads `ctx.file` (already set per-function by step 25) and passes it: `ir_function_new(0, name, ir_ret, eff, ctx.file)`.
    - `compiler/module_io.vow:994` — read source_file from the wire (step 28) and pass it.
24. **`lower_function_vow` reads `ctx.file`.** `compiler/lower.vow:3128`. No signature change needed — `ctx` is already the parameter, and `ctx.file` is set per-function by `lower_module_vow` (step 25). At line 3139 forward `ctx.file` to `ir_function_new` (per step 23).
25. **`lower_module_vow` signature.** `compiler/lower.vow:3259`. Replace `file_path: String` with `item_files: Vec<String>`. Today `ctx.file` is set once at line 3476 (`ctx.file = file_path;`); change to a per-iteration assignment inside the function-lowering loop (around `:3493 onward`): `ctx.file = item_files[func_idx];` before each `lower_function_vow(ctx, fid)` call. The single `ctx.file = ...` at line 3476 is removed.
26. **`compiler/region.vow` updates:**
    - `infer_regions_module(m, dctx)` (drop `path`) at `:27`.
    - `check_store_conflict(target_arg_id, source_arg_id, call_inst, id_to_inst, dctx, source_file)` at `:811` — receive `source_file: String` from the analyzing function. Replace the `path` argument with `f.source_file` at every call site (1 call site at `:778`).
    - The `IrFunction { ... }` literal at `:214` (constructed during summary publishing) must include `source_file: f.source_file` — preserve the input function's value.
27. **`compiler/frontend.vow` updates:**
    - `load_frontend_deps` (`:72`) maintains a parallel `item_files: Vec<String>` alongside `all_items`. For each item pushed at `:94`, push `dep_path` to `item_files`. Add `item_files: Vec<String>` to its parameter list.
    - `frontend_prepare_path` (`:103-143`) builds and stores the `item_files` parallel vector. Push the root path for each root item at `:118-121`. Expose via `FrontendPrep` (add field `item_files: Vec<String>`).
    - `frontend_lower_path` (`:145-198`) passes `prep.item_files` to `lower_module_vow` at `:164` and drops the path argument from `infer_regions_module` at `:165`.
28. **`compiler/module_io.vow` + `compiler/ir.vow` updates:**
    - **Bump the `MODULE_VERSION()` definition** in `compiler/ir.vow:164` from `{ 1 }` → `{ 3 }` (matching Rust per step 13). The call sites at `compiler/module_io.vow:1044` (encoder) and `:1101` (decoder gate) automatically pick this up — no separate edits there.
    - `write_function` at `compiler/module_io.vow:903` — append `write_string(out, f.source_file);` after `write_region_summary(out, f);`.
    - `read_function` at `compiler/module_io.vow:965` — read `let source_file: String = reader_read_string(r);` after `read_region_summary`. Pass to the new-form `ir_function_new` per step 22 (or set field directly on the constructed value).

### Phase H — Bootstrap and full test

29. `scripts/bootstrap.sh --skip-cargo` — rebuild the self-hosted compiler. Iterate until clean.
30. `ulimit -v 2000000; scripts/full_test.sh` — full suite (target: 195 passed / 0 failed / 3 skipped per memory).
31. **Bootstrap triple test (binary fixed-point):**
    ```bash
    ./scripts/concat_vow.sh clif > /tmp/compiler_clif.vow
    ulimit -v 2000000; ./target/release/vow --no-verify /tmp/compiler_clif.vow -o /tmp/compiler_a
    ulimit -v 2000000; /tmp/compiler_a -o /tmp/compiler_b /tmp/compiler_clif.vow
    ulimit -v 2000000; /tmp/compiler_b -o /tmp/compiler_c /tmp/compiler_clif.vow
    sha256sum /tmp/compiler_b /tmp/compiler_c   # must be identical
    ```
    The new String field must round-trip deterministically through `write_function` / `read_function` so the binary outputs match.
32. **vmod tests sweep.** Re-run `tests/multi/vmod_region_roundtrip/` (already exists per `tests/multi/`). The version bump may require updating any pinned wire byte fixtures (search `tests/multi/vmod_*/main.vow` for hex literals matching the old version byte).

### Phase I — Codex review and commit

33. Run `/codex:review` after Phases A–H verify clean. Address feedback; re-run Phase H gates after any changes.
34. Stage all touched files (Rust + self-hosted + tests). Commit:
    ```
    fix(region): label diagnostics with per-function source file (#254)
    ```
    Body: root cause (multi-module path discarded by `merge_modules`), fix (per-Function `source_file` threaded through lowering), tests (IR-level cross-file labeling assertion + serialize round-trip + bootstrap fixed-point preserved).
35. Push, comment on issue #254 with summary, close the issue.

## Testing

- **Unit (Rust):** `region_conflict_uses_callee_function_source_file` (new) — proof of fix.
- **Round-trip (Rust):** new `Function`-with-`source_file` round-trip test in `vow-ir/src/serialize.rs::tests`.
- **Self-hosted:** `tests/multi/vmod_region_roundtrip/` covers the wire-format change end-to-end after `MODULE_VERSION` bump.
- **Integration:** `scripts/full_test.sh` (195+ pass) + bootstrap triple (sha256 identical).

## Risks

- **`MODULE_VERSION` bump cascade (resolved).** `tests/multi/vmod_decode_bad_version/main.vow:13` pushes literal `bytes.push(2);`. To keep this fixture as a "bad" version we bump `MODULE_VERSION` to `3` (steps 10, 13, 28). Other `vmod_*` fixtures use the encoder + decoder symmetrically and re-derive the version from the constant — they need no edits. Spot-check during step 32 by greping `tests/multi/vmod_*/main.vow` for hard-coded literals 1, 2, 3.
- **Production-path `Function { ... }` literal site (resolved).** Verified: the *only* production-path `Function {...}` literal is `LowerCtx::new` at `vow-ir/src/lower/mod.rs:224`. All other `Function {...}` literals are test fixtures (enumerated in steps 1–4).
- **`lower.vow:60` site (resolved).** Verified: `lctx_new` is the LowerCtx root constructor invoked once at `compiler/lower.vow:3475`. Its inner `ir_function_new(0, name, return_ty, effects, ...)` IrFunction is overwritten by `lower_function_vow` at `:3143` (`ctx.func = func`). Pass `String::from("")` placeholder; never reaches diagnostics.
- **Threading `&Function` into `check_store_conflict`.** The cleanest fix is to pass `source_file: &str` (extracted at `analyze_function`'s top) rather than `&Function`, keeping the helper signature tight. Avoids deeper signature churn through `handle_inst` chain.
- **`PartialEq` on `Function`.** `Function` derives `PartialEq` (`vow-ir/src/types.rs:294`). Adding a `String` field changes equality. Existing tests compare specific fields (`m.functions[0].summary.return_region`), not whole Functions. Mitigation: grep `f0 == f1` and `assert_eq!(.*Function` before merge — none expected, but verify.
- **Bootstrap determinism (resolved).** Verified: Rust `module_loader::resolve_use` (`vow/src/module_loader.rs:78`) uses `Path::join` which on Linux emits `/`-separated paths. Self-hosted `resolve_use_path` (`compiler/frontend.vow:56`) manually concatenates with byte `47` (`/`). For ASCII paths on Linux, both produce byte-identical strings. The root path comes from `argv[1]` unchanged in both compilers. Bootstrap fixed-point preserved as long as the corpus stays ASCII-only on Linux (already the project's assumption).
- **`source_file` as opaque string.** No canonicalization (no `Path::canonicalize`, no relativization). Whatever string `merge_modules` records is what diagnostics emit. This matches the existing behavior of `prepare_frontend` (raw `source.to_string_lossy()`). Risk only if the bootstrap is invoked from different working directories — but the corpus uses repo-relative invocations consistently.
- **Latent bug, latent fix.** The conflict shape is unreachable end-to-end today, so the user-visible behavior of `build/vowc` cannot be regression-tested via `examples/` or `tests/run/`. Mitigation: the IR-level test in step 15 is the contract; document this in the commit body and PR description.
- **`RegionLinear` shares the bug.** `emit_live_linear_errors` uses the same root `source_file` parameter. The fix in step 8 covers it via the same threading change — no separate code path. Mention explicitly in the commit body.

## Notes

- **No spec change.** `docs/spec/errors.md` already says diagnostics carry file labels; the bug is implementation-level, not spec-level.
- **No CLI change.** `vowc build` / `vowc verify` surface unchanged.
- **No `--help` regen needed.** No new flag, builtin, or grammar rule.
- **Plan path:** This file is `plans/issue-254-region-source-file.md`.
- **Branch:** `vow/issue254` (clean as of plan time).
