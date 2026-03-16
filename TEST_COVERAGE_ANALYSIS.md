# Test Coverage Analysis

**Date:** 2026-03-08 (updated 2026-03-16 against current `main`)
**Baseline:** 540 `#[test]` functions across 9 crates (~526 pass, 14 fail due to missing release binary)

## Summary

| Crate | Tests | Coverage | Priority |
|-------|------:|----------|----------|
| vow-syntax | 139 | Good | Low |
| vow-types | 137 | Good | Low |
| vow-ir | 51 | Moderate | **Medium** |
| vow-codegen | 56 | Good | Low |
| vow-verify | 69 | Good | Low |
| vow-diag | 5 | **Weak** | **Medium** |
| vow (CLI) | 83 | Good | Low |
| vow-runtime | 0 | **None** | **High** |
| vow-clif-shim | 0 | **None** | Low (FFI) |

## Failing Tests (14)

All 14 failures are in the `vow` crate's end-to-end tests that compile `.vow` programs into executables and run them. They fail because `target/release/vow` is not built in the test environment. These are **not code bugs** — running `cargo build --all --release` before testing fixes them. The tests should either:

1. Use `cargo_bin("vow")` from `assert_cmd` to find the debug binary, or
2. Build with `--release` in CI before running tests, or
3. Use `env!("CARGO_BIN_EXE_vow")` to locate the binary built by `cargo test`.

**Affected tests:** `hello_world_prints_and_exits_zero`, `vow_violation_blame_caller_exit_code_1`, `while_loop_countdown_prints_zero`, `bisect_with_loop_invariant_compiles_and_runs`, `struct_construction_and_field_access`, `enum_construction_and_match`, `option_some_none_compiles_and_runs`, `question_operator_short_circuits`, `vec_push_len_index`, `struct_and_vec_combined`, `string_from_len_eq`, `hashmap_insert_get_contains_remove`, `module_system_two_files`, `agent_capability_test_skill_json_is_parseable_and_complete`.

---

## Critical Gaps

### 1. vow-runtime — 0 tests (HIGH priority)

The runtime crate has **27 public `extern "C"` functions** with zero test coverage. These implement core runtime behavior that compiled Vow programs depend on.

**Recommended tests (grouped by subsystem):**

#### Vector runtime (`__vow_vec_*`)
- `vec_new_val_creates_empty_vec` — length 0 after creation
- `vec_push_val_increments_len` — push N items, verify len == N
- `vec_get_val_returns_pushed_value` — push then get at same index
- `vec_set_val_updates_value` — push, set, get roundtrip
- `vec_pop_decrements_len` — push N, pop, verify len == N-1
- `vec_get_val_out_of_bounds_panics` — index >= len triggers bounds check
- `vec_capacity_doubles_on_growth` — push past initial capacity

#### String runtime (`__vow_string_*`)
- `string_new_and_len` — create from bytes, verify len
- `string_from_cstr_null_terminated` — from C string
- `string_eq_same_content` — equal strings return 1
- `string_eq_different_content` — different strings return 0
- `string_contains_substring` — positive and negative cases
- `string_push_str_appends` — concatenation updates len
- `string_push_byte_appends` — single byte append
- `string_byte_at_returns_correct_byte` — indexing
- `string_from_i64_formats_correctly` — 0, positive, negative

#### HashMap runtime (`__vow_map_*`)
- `map_new_is_empty` — len == 0
- `map_insert_and_get` — insert then retrieve
- `map_insert_updates_existing_key` — insert same key twice
- `map_contains_key_positive_negative` — contains after insert, not before
- `map_remove_decrements_len` — remove existing key
- `map_remove_nonexistent_key` — no-op, len unchanged

#### Violation handler
- `vow_violation_exits_with_code_1` — verify exit code and JSON on stderr

**Testing approach:** These are `unsafe extern "C"` functions. Tests can call them directly with `unsafe` blocks. The vec/string/map functions are pure memory operations and straightforward to test. The violation/overflow handlers call `exit()` so they need process-based tests.

---

### 2. vow-diag — 5 tests (MEDIUM priority)

Only covers `JsonEmitter` and `HumanEmitter` basics. Missing:

- **ErrorCode variants:** Only `VowRequiresViolated` tested. Add tests for `ParseError`, `TypeError`, `EffectViolation`, `LinearityViolation`, `ExhaustivenessError`, `UndefinedVariable`, `UndefinedFunction`, `UndefinedType`, `ArityMismatch`, `VowEnsuresViolated`, `VowInvariantViolated`.
- **Blame variants:** Only `Caller` tested. Add `Callee` and `None`.
- **Severity variants:** Only `Error` tested. Add `Warning` and `Note`.
- **CollectingEmitter:** Not tested at all — used as a test helper in other crates but its own behavior (collecting, dedup, ordering) is unverified.
- **Edge cases:** Empty message, very long message, special characters in JSON output, multiple diagnostics in sequence, diagnostics with multiple hints.

**Recommended tests (10):**
- `error_code_display_all_variants` — each variant serializes correctly
- `blame_serialization_all_variants` — Caller/Callee/None
- `severity_display_variants` — Error/Warning/Note
- `collecting_emitter_collects_in_order` — emit 3, retrieve in order
- `collecting_emitter_into_diagnostics` — ownership transfer
- `json_emitter_multiple_diagnostics` — emit sequence, all valid JSON
- `json_emitter_special_chars_in_message` — quotes, newlines, backslashes
- `human_emitter_with_source_location` — file:line:col formatting
- `human_emitter_blame_callee` — blame annotation in output
- `diagnostic_with_multiple_hints` — all hints printed

---

### 3. vow-ir lowering — 7 tests (MEDIUM priority)

The IR lowerer handles 17+ expression types but only tests 7 (const, add, let, if-else, empty fn, ensures, while). Major untested paths:

**Expression lowering gaps:**
- `lower_unary_neg` — negation
- `lower_unary_not` — boolean not
- `lower_function_call` — direct call
- `lower_struct_literal` — struct construction with fields
- `lower_enum_construction` — variant construction
- `lower_match_expr` — pattern matching with branches
- `lower_field_access` — struct field read
- `lower_index_expr` — vec/array index
- `lower_method_call` — method dispatch
- `lower_question_operator` — `?` unwrap
- `lower_assignment` — `x = expr`
- `lower_return_expr` — explicit return
- `lower_block_expr` — block with trailing expression
- `lower_checked_arithmetic` — `+!`, `-!`, `*!`
- `lower_float_operations` — f32/f64 arithmetic
- `lower_bool_literal` — true/false constants
- `lower_string_literal` — string constant

**Vow block gaps (in lower/vow.rs, 7 tests exist):**
- `invariant_clause_lowering` — while loop invariants
- `ensures_with_result_binding` — `ensures(result): result > 0`
- `multiple_requires_clauses` — conjunction of preconditions
- `free_variable_capture_in_predicates` — binding list correctness

---

### 4. vow-ir validator — 8 tests (LOW-MEDIUM priority)

Missing error variant coverage:
- `UndefinedInstRef` — reference to non-existent instruction
- `TypeMismatch` — e.g., branch on non-Bool
- Multi-function module validation
- Complex Phi/Upsilon patterns (multiple sources)
- Block with only a terminal instruction (minimal valid block)

---

### 5. vow-codegen linker — 1 test (LOW priority)

Only tests `find_runtime_returns_some_in_dev_build`. Missing:
- `find_shim_lib` discovery
- `link()` error cases (missing cc, invalid paths)

Not high priority since linking is well-covered by e2e tests.

---

### 6. vow module_loader — 3 tests (LOW-MEDIUM priority)

Missing:
- **Circular dependency detection** — A imports B imports A
- **Transitive dependencies** — A imports B imports C
- **Parse errors in imported module** — graceful error reporting
- **Multi-component use paths** — `use foo::bar::baz`
- **Merge with conflicting names** — same function name in two modules

---

### 7. vow-clif-shim — 0 tests (LOW priority)

This is an FFI shim calling Cranelift. Unit testing individual shim functions is difficult because they require a full Cranelift context. The bootstrap triple test (binary fixed-point) provides strong end-to-end coverage. Adding unit tests here would have low ROI.

---

## Recommended Priorities

### Phase 1: Fix infrastructure (quick wins)
1. **Fix the 14 e2e test failures** — Make them use the debug binary or `CARGO_BIN_EXE_vow` instead of hardcoding `target/release/vow`.
2. **Add `#[ignore]` annotations** with a clear message if release binary is intentionally required.

### Phase 2: vow-runtime tests (highest impact)
Add ~25 tests covering vec, string, map, and violation handler. These are the most safety-critical untested functions — bugs here cause UB in every compiled Vow program.

### Phase 3: vow-diag tests (quick, high value)
Add ~10 tests. Small crate, fast to test, important for error reporting correctness.

### Phase 4: vow-ir lowering tests (medium effort, high value)
Add ~15 tests for untested expression types. Each test is small (parse a snippet, lower it, check IR instructions). This catches regressions in the most complex compiler stage.

### Phase 5: Remaining gaps
- IR validator: 5 more tests
- Module loader: 5 more tests
- Codegen linker: 2 more tests

## Test Count Targets

| Crate | Current | Target | Delta |
|-------|--------:|-------:|------:|
| vow-syntax | 139 | 139 | +0 |
| vow-types | 137 | 137 | +0 |
| vow-ir | 51 | 75 | +24 |
| vow-codegen | 56 | 58 | +2 |
| vow-verify | 69 | 69 | +0 |
| vow-diag | 5 | 15 | +10 |
| vow (CLI) | 83 | 96 | +13 |
| vow-runtime | 0 | 25 | +25 |
| vow-clif-shim | 0 | 0 | +0 |
| **Total** | **540** | **614** | **+74** |
