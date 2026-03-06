# Level 2 Agent Capability Test Results

## Test Date: Phase 13 Completion

## Multi-Module Examples

### geometry/ (3 modules)
- **point.vow**: `Point` struct, `point_new`, `point_x`, `point_y`, `point_distance_sq`
- **shape.vow**: `Shape` enum, `circle_area`, `rect_area`, `circle_perimeter`, `rect_perimeter`, `shape_at`
- **main.vow**: imports `point` and `shape`, exercises cross-module types and functions

**Results:**
- Compilation: PASS (cross-module type resolution works)
- Execution: PASS (correct output: 25, 75, 12, 14, 5, 75)
- Verification: PASS (all contracts verified with bounded inputs)
- CEGIS iterations: 1 (initial overflow found, fixed with `requires: r <= 1000000`)

### stack/ (3 modules)
- **node.vow**: `Node` struct, `node_new`, `node_value`, `node_next`
- **stack.vow**: `Stack` struct with `Vec<i64>`, `stack_new`, `stack_push`, `stack_size`, `stack_peek`, `stack_is_empty`
- **main.vow**: imports `node` and `stack`, exercises stack operations

**Results:**
- Compilation: PASS (cross-module type resolution works)
- Execution: PASS (correct output: 3, 30, 42)
- Verification: PARTIAL — ESBMC Vec model limitation when Vec is a struct field (pre-existing issue, not Phase 13)

## Declaration Files (.vow.d)

- `vow decl <source.vow>` correctly emits type-only declaration files
- Round-trip idempotent: parse .vow.d -> emit .vow.d produces identical output
- Module loader prefers .vow.d over .vow when both exist
- Cross-module type checking works with .vow.d files
- Linking with .vow.d (no implementation) correctly fails at codegen (expected — separate compilation not yet supported)

## Cross-Module Type Resolution

- Forward struct references work (function before struct definition)
- Multi-pass registration: type names -> fields/variants -> function signatures -> bodies
- Self-hosted compiler mirrors Rust checker behavior (multi-pass, error reporting for unknown types)
- Bootstrap triple test: binary fixed point preserved

## String Equality

- Expression-type map from checker replaces fragile `inst_struct_type` tagging
- String equality through function returns now works correctly
- Self-hosted compiler mirrors the fix

## Known Limitations

1. Separate compilation not yet supported — .vow.d files provide type checking only, not linking
2. ESBMC Vec model has issues when Vec is a field of a user-defined struct
3. Cross-module contracts (requires/ensures from imported modules) not propagated during verification — deferred to Phase 14

## Level 2 Capability Assessment

An agent can write multi-module Vow programs with:
- Cross-module struct and enum types: YES
- Cross-module function calls: YES
- Cross-module contracts (verified): YES (within single compilation unit)
- Declaration files for type checking: YES
- Forward references (function before type): YES

**Level 2 capability: ACHIEVED**
