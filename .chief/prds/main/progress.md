## Codebase Patterns
- Build JSON output is emitted via manual string formatting in `BuildOutput::emit_json()` (vow/src/main.rs), not via serde serialization
- Diagnostics flow: parse returns Vec<Diagnostic>, module loader returns Err(Vec<Diagnostic>), type checker uses DiagnosticEmitter trait
- `CollectingEmitter` in vow-diag wraps another emitter and collects diagnostics — use this to capture type checker output
- All `BuildOutput` constructors must include `diagnostics`, `counterexamples`, `verify_status`, `verify_message` fields
- Pre-existing formatting changes may exist in the working tree from `cargo fmt`; only commit files actually modified for the story
- `parse_module` and `parse_item_source` in vow-syntax now require a `file: &str` parameter — always pass the source file path
- Type checker (`Checker::new`) already accepted a `file` param; effects/linear/exhaustiveness checkers also take `file: &str`
- Parser struct has a `file: String` field; `push_error` uses `self.file.clone()` for SourceLocation
- IR `Function` struct has `param_names: Vec<String>` — populated during lowering from AST `fn_def.params[].name`; empty in test code (use `param_names: vec![]`)
- C emitter names params `p{cl_idx}` (skipping Unit) and instructions `v{id}`; `build_c_to_source_name_map()` maps both back to source names
- Unmappable ESBMC variables get `_esbmc_` prefix (e.g., `_esbmc_v3`)
- `VowEntry` has `file: String` and `offset: u32` — in test code use `file: String::new(), offset: 0`
- `lower_function` and `lower_module` take `file: &str` — pass `""` in tests, actual path in pipeline
- `__vow_violation` runtime takes 7 args: vow_id, blame, desc_ptr, bindings_ptr, binding_count, file_ptr, offset

---

## 2026-03-02 - US-001
- What was implemented: Added `diagnostics` array to build JSON output. Every diagnostic from parse, module loading, and type checking is now collected and included in the JSON build output as a `"diagnostics": [...]` array. Each entry has `error_code`, `message`, `severity`, and `span` (with `file`, `offset`, `length`). Empty array on success.
- Files changed:
  - `vow-diag/src/lib.rs` — Added `CollectingEmitter` struct that wraps a `DiagnosticEmitter` and collects diagnostics
  - `vow/src/main.rs` — Added `diagnostics: Vec<Diagnostic>` to `BuildOutput`, updated `emit_json()` with `format_diagnostics_json()`, updated `run_pipeline()` to collect from all stages, added 5 new tests, updated skill_json/skill_human docs
- **Learnings for future iterations:**
  - The type checker takes `&mut dyn DiagnosticEmitter` — to collect its diagnostics, wrap the stderr emitter in `CollectingEmitter` and call `into_diagnostics()` after checking
  - Parse diagnostics use `"<input>"` as file path (US-002 will fix this)
  - The working tree may have pre-existing `cargo fmt` changes in other files — be selective when staging
  - `Checker` borrows the emitter, so you need `drop(checker)` before calling `into_diagnostics()` on the `CollectingEmitter`
---

## 2026-03-02 - US-002
- What was implemented: Threaded file path into all diagnostic spans. The parser now accepts a `file: &str` parameter that flows into all diagnostics it emits. The main pipeline passes the actual source file path, and the module loader passes each dependency's file path.
- Files changed:
  - `vow-syntax/src/parser/mod.rs` — Added `file: String` field to `Parser` struct, updated `Parser::new` to accept file param, `push_error` uses `self.file.clone()`, `parse_module` and `parse_item_source` now take `file: &str`
  - `vow-syntax/src/parser/expr.rs` — Updated 4 test `Parser::new` calls to include file arg
  - `vow-syntax/src/parser/items.rs` — Updated test `parse_item_source` call to include file arg
  - `vow-syntax/src/parser/types.rs` — Updated 2 test `Parser::new` calls to include file arg
  - `vow-syntax/tests/integration.rs` — Updated `roundtrip` helper to pass file arg
  - `vow/src/main.rs` — Pass `source.to_string_lossy()` to `parse_module`, added 2 new tests asserting file path in diagnostics
  - `vow/src/module_loader.rs` — Pass `file_path.to_string_lossy()` to `parse_module` for dependencies, updated 3 test calls
- **Learnings for future iterations:**
  - The `Parser` struct is private to `vow-syntax::parser` — all access goes through `parse_module`/`parse_item_source` public APIs
  - Tests in `expr.rs` and `types.rs` also construct `Parser::new` directly — don't forget these when changing the constructor
  - The type checker, effects checker, linear checker, and exhaustiveness checker already had `file` parameters — only the parser was missing it
  - Module loader's `load_deps` already has `file_path: PathBuf` — convert with `to_string_lossy()` for the parser
---

## 2026-03-02 - US-003
- What was implemented: ESBMC text output is now parsed into structured JSON counterexamples. Build JSON includes `"counterexamples": [...]` array, `"verify_status"`, and `"verify_message"` fields. The ESBMC output parser extracts vow_id from assertion labels (`vow:N`), variable assignments from counterexample trace states, and maps violated vows back to VowEntry descriptions and source spans.
- Files changed:
  - `vow-verify/src/esbmc.rs` — Enhanced `Counterexample` struct with `vow_id`, `inputs`, `raw_output` fields. Added `parse_esbmc_output()`, `extract_vow_id()`, `extract_variable_assignments()`, `parse_assignment_line()` functions. 7 new parser tests.
  - `vow-verify/src/lib.rs` — Added `Counterexample` and `parse_esbmc_output` to public exports
  - `vow/src/main.rs` — Added `StructuredCounterexample`, `CeSource`, `VerifyOutcome` types. Updated `BuildOutput` with `counterexamples`, `verify_status`, `verify_message` fields. Added `build_structured_counterexample()`, `find_vow_span()`, `format_counterexamples_json()`. Updated verification thread to return `VerifyOutcome` enum. Updated all BuildOutput constructors. Added 8 new tests.
- **Learnings for future iterations:**
  - ESBMC output format: `[Counterexample]` header, `State N file ... line ... column ... function ... thread 0` + `----` + `  var = val (bits)`, then `Violated property:` section with `vow:N` assertion label
  - Variable assignments in ESBMC output have optional binary representation in parentheses: `v1 = 0 (00000000...)` — strip the parens part
  - VowEntry.description includes the keyword prefix (e.g., `"ensures result > 100"`, not just `"result > 100"`)
  - The verification thread needs the file path string for source location mapping — pass it via closure capture
  - Vow instruction spans can be found by searching IR blocks for VowEnsures/VowInvariant opcodes with matching VowId
  - Timeout and ToolNotFound were previously silently ignored; now surfaced via `verify_status` and proper `VerifyOutcome` variants
---

## 2026-03-02 - US-004
- What was implemented: ESBMC counterexample variables are now mapped back to Vow source parameter names. Added `param_names: Vec<String>` field to the IR `Function` struct, populated during lowering from AST parameter names. `build_c_to_source_name_map()` builds a mapping from C emitter variable names (`p0`, `p1`, `v0`, `v1`, etc.) to source names (`x`, `y`, etc.). Unmappable variables receive an `_esbmc_` prefix. `build_structured_counterexample()` applies this mapping to counterexample inputs.
- Files changed:
  - `vow-ir/src/types.rs` — Added `param_names: Vec<String>` field to `Function` struct; updated 2 test constructors
  - `vow-ir/src/lower/mod.rs` — Extract param_names from `fn_def.params`, pass to `LowerCtx::new`
  - `vow-ir/src/printer.rs` — Updated test helper `make_func` with `param_names: vec![]`
  - `vow-ir/src/validator.rs` — Updated test helper `make_func` with `param_names: vec![]`
  - `vow-verify/src/c_emitter.rs` — Updated 5 test `Function` constructors with `param_names: vec![]`
  - `vow-verify/src/esbmc.rs` — Updated 3 test `Function` constructors with `param_names: vec![]`
  - `vow-codegen/src/cranelift_backend.rs` — Updated 23 test `Function` constructors with `param_names: vec![]`
  - `vow-codegen/tests/e2e.rs` — Updated 5 test `Function` constructors with `param_names: vec![]`
  - `vow/src/main.rs` — Added `build_c_to_source_name_map()`, `map_counterexample_inputs()` functions; updated `build_structured_counterexample()` to use name mapping; added 5 new tests
- **Learnings for future iterations:**
  - Adding a field to the IR `Function` struct requires updating ~30+ construction sites across all crates — use `param_names: vec![]` as default for test code
  - The C emitter names parameters `p{idx}` (skipping Unit params) and instruction results `v{inst_id}` — GetArg instructions copy params to v-variables
  - `param_names` comes from AST `fn_def.params[i].name` during `lower_function` — the AST is not available later in the verification pipeline
  - Clippy enforces `collapsible_if` — use `if a && let b = c && let d = e { ... }` style for chained conditions
  - Pre-existing `cargo fmt` changes in cranelift_backend.rs and lower/mod.rs get mixed into the commit when adding new fields — unavoidable since formatting is needed for the new code
---

## 2026-03-02 - US-005
- What was implemented: Added source location (file path and byte offset) to runtime VowViolation JSON output. VowEntry in vow-ir now carries `file: String` and `offset: u32`. LowerCtx stores the file path and passes it through to each VowEntry. The `__vow_violation` runtime function takes two new parameters (`file_ptr`, `offset`) and emits them in both JSON and human-readable output. Codegen creates data sections for file path strings and passes them through to the runtime call.
- Files changed:
  - `vow-ir/src/types.rs` — Added `file: String` and `offset: u32` fields to `VowEntry`
  - `vow-ir/src/lower/mod.rs` — Added `file: String` to `LowerCtx`, updated `LowerCtx::new`, `alloc_vow`, `lower_function`, `lower_module` to accept/pass file path
  - `vow-ir/src/lower/vow.rs` — Updated `lower_requires`, `lower_ensures`, `lower_invariant` to pass `span.start` as offset to `alloc_vow`
  - `vow-runtime/src/lib.rs` — Added `file_ptr: *const i8` and `offset: u32` params to `__vow_violation`; updated JSON/human output format
  - `vow-codegen/src/cranelift_backend.rs` — Added `file_ptr` and `offset` to violation signature; created data sections for vow file paths; added `vow_file_global_values` to `LowerCtx`; updated `emit_vow_check` to pass file_ptr and offset
  - `vow-codegen/tests/e2e.rs` — Updated 2 test VowEntry constructors
  - `vow-verify/src/c_emitter.rs` — Updated 2 test VowEntry constructors
  - `vow-verify/src/esbmc.rs` — Updated 3 test VowEntry constructors
  - `vow/src/main.rs` — Updated `lower_module` call to pass file path
- **Learnings for future iterations:**
  - VowEntry fields propagate to ~15+ test construction sites across 5 crates — always add `field: default_value` to all of them
  - `LowerCtx` in vow-ir (IR lowering) and `LowerCtx` in cranelift_backend (codegen) are different structs with the same name — changes may be needed in both
  - The vow clause `span.start` gives the byte offset of the `requires:`/`ensures:`/`invariant:` keyword in the source
  - File path data sections in codegen follow the same pattern as description strings: create anonymous data → define → declare in func → get GlobalValue
  - `lower_module` and `lower_function` are public API of vow-ir — changing their signatures affects the main pipeline caller in `vow/src/main.rs`
---
