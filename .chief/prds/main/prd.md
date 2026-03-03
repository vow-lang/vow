# PRD: Phase 10 — Close the CEGIS Verification Loop

## Introduction

Vow is a programming language designed for AI agents to write formally verified code. The compiler pipeline already produces JSON build output, blame-tracking contracts, and integrates ESBMC for static verification. However, agents currently cannot autonomously complete the full verify-fix cycle because: (1) only the first diagnostic is visible in build JSON, (2) ESBMC counterexamples are unstructured text, (3) runtime violations lack source locations, (4) contracts cannot express properties over collections (Vec, String, HashMap), and (5) there are too few example programs to demonstrate or test the workflow.

Phase 10 closes the CEGIS (counterexample-guided inductive synthesis) loop: an AI agent writes a Vow program with contracts, compiles it, receives **all** errors as structured JSON, fixes them, verifies with ESBMC, receives **structured** counterexamples, fixes those, and iterates until verification passes — all without human intervention.

## Goals

- Expose all compiler diagnostics in a single JSON array in build output, enabling agents to batch-fix errors in one pass
- Structure ESBMC counterexamples as machine-readable JSON with input values, violated predicate, and source location
- Add source file path and byte offset to runtime VowViolation output so agents can jump directly to the failing contract
- Model Vec, String, and HashMap operations in the ESBMC C emitter so contracts over collection types can be verified
- Add `where` clause syntax on function parameters as sugar for `requires` blocks
- Produce 10-15 verified example programs demonstrating every contract pattern the language supports

## User Stories

### US-001: Add diagnostic array to build JSON
**Priority:** 1
**Description:** As an AI agent parsing `vowc` output, I want all diagnostics in a JSON array so that I can fix every error in a single repair pass instead of recompiling after each fix.

**Acceptance Criteria:**
- [ ] Build JSON output includes a `"diagnostics": [...]` array containing every diagnostic emitted during compilation
- [ ] Each diagnostic object includes: `error_code` (string), `message` (string), `severity` ("error" | "warning"), `span` object with `file` (string), `offset` (u32), `length` (u32)
- [ ] When compilation succeeds with no diagnostics, the array is empty (`[]`), not absent
- [ ] The existing top-level `"status"` field ("Verified", "CompileFailed", etc.) is preserved unchanged
- [ ] Human-readable output is unchanged (dual output invariant maintained)
- [ ] `cargo test --all` passes
- [ ] `cargo clippy --all -- -D warnings` passes

### US-002: Thread file path into diagnostic spans
**Priority:** 1
**Description:** As an AI agent, I need each diagnostic span to include the source file path so I can locate the error in multi-file projects.

**Acceptance Criteria:**
- [ ] `Span` or diagnostic metadata carries a file path (or file ID resolvable to a path)
- [ ] The `"file"` field in each diagnostic JSON object contains the path passed to the compiler
- [ ] Single-file and multi-file (module-loading) compilations both populate the file field correctly
- [ ] `cargo test --all` passes

### US-003: Parse ESBMC text output into structured JSON
**Priority:** 2
**Description:** As an AI agent, I want ESBMC verification failures returned as structured JSON so I can understand which inputs cause the violation and suggest targeted fixes.

**Acceptance Criteria:**
- [ ] When ESBMC finds a counterexample, the build JSON includes a `"counterexamples": [...]` array
- [ ] Each counterexample object includes: `inputs` (object mapping parameter names to concrete values), `violation` (string — the predicate text that failed), `vow_id` (integer), `source` object with `file`, `offset`, `length`
- [ ] When ESBMC proves all assertions, `"counterexamples"` is an empty array
- [ ] When ESBMC times out or errors, the build JSON includes `"verify_status": "timeout"` or `"verify_status": "error"` with a `"verify_message"` string
- [ ] The parser handles ESBMC's XML output format (preferred) or falls back to text parsing
- [ ] Existing `examples/divide.vow` produces a structured counterexample when verified without the `requires` precondition
- [ ] `cargo test --all` passes

### US-004: Map ESBMC counterexample variables back to source names
**Priority:** 2
**Description:** As an AI agent, I need counterexample variables reported using Vow source names (not ESBMC internal names) so I can correlate them with the original program.

**Acceptance Criteria:**
- [ ] The `inputs` object in counterexample JSON uses Vow parameter names (e.g., `"y": 0`), not C/ESBMC mangled names
- [ ] The mapping is derived from `Origin` metadata already present in vow-verify
- [ ] If a variable cannot be mapped back to a source name, it is included with a prefixed name like `"_esbmc_var_3": 42`
- [ ] `cargo test --all` passes

### US-005: Add source location to runtime VowViolation
**Priority:** 3
**Description:** As an AI agent observing a runtime vow violation (in debug mode), I want the JSON output to include the source file and byte offset so I can navigate directly to the contract that failed.

**Acceptance Criteria:**
- [ ] `VowEntry` metadata in vow-ir includes file path and byte offset of the originating vow block
- [ ] Codegen threads file path and offset through to the `__vow_violation` runtime call as additional arguments
- [ ] Runtime VowViolation JSON output includes `"file"` (string) and `"offset"` (integer) fields
- [ ] The existing fields (`vow_id`, `blame`, `description`, `values`) are preserved unchanged
- [ ] `examples/divide.vow` compiled with `--mode debug` and called with y=0 produces a VowViolation with source location
- [ ] `__vow_violation` C signature in vow-runtime updated; existing callers updated
- [ ] `cargo test --all` passes

### US-006: Model Vec operations in ESBMC C emitter
**Priority:** 4
**Description:** As a Vow programmer, I want contracts involving `Vec` operations (`len`, `push`, `get`) to be verifiable by ESBMC so I can write verified programs that use dynamic arrays.

**Acceptance Criteria:**
- [ ] vow-verify C emitter models `Vec<T>` as a C struct with `len` and abstract capacity
- [ ] `v.len()` in a contract maps to the modeled length field
- [ ] `v.push(x)` increments the modeled length by 1
- [ ] `v.get(i)` / `v[i]` in a contract maps to an abstract array access with bounds assertion
- [ ] A test program with `ensures: result.len() == n` on a function that pushes `n` elements verifies successfully
- [ ] A test program with an intentionally violated Vec contract produces a structured counterexample
- [ ] `cargo test --all` passes

### US-007: Model String operations in ESBMC C emitter
**Priority:** 4
**Description:** As a Vow programmer, I want contracts involving `String` operations (`len`, `contains`) to be verifiable by ESBMC.

**Acceptance Criteria:**
- [ ] vow-verify C emitter models `String` with an abstract length
- [ ] `s.len()` in a contract maps to the modeled length
- [ ] `s.contains(sub)` is modeled as a nondeterministic boolean (conservative but sound) or as a concrete check when both operands are known
- [ ] A test program with `ensures: result.len() > 0` on a string-returning function verifies
- [ ] `cargo test --all` passes

### US-008: Model HashMap operations in ESBMC C emitter
**Priority:** 4
**Description:** As a Vow programmer, I want contracts involving `HashMap` operations (`len`, `contains_key`) to be verifiable by ESBMC.

**Acceptance Criteria:**
- [ ] vow-verify C emitter models `HashMap<K,V>` with an abstract length and key set
- [ ] `m.len()` maps to the modeled length
- [ ] `m.contains_key(k)` maps to membership in the abstract key set
- [ ] After `m.insert(k, v)`, `m.contains_key(k)` is provably true and `m.len()` increases by 1 (if key was new)
- [ ] A test program verifying HashMap invariants produces correct verification results
- [ ] `cargo test --all` passes

### US-009: Add `where` clause syntax for parameters
**Priority:** 5
**Description:** As a Vow programmer, I want to write `fn divide(x: i64, y: i64 where y != 0) -> i64` as shorthand for a `requires` block, making contracts more readable inline.

**Acceptance Criteria:**
- [ ] The parser accepts `where <expr>` after a parameter's type annotation
- [ ] Multiple parameters can each have their own `where` clause
- [ ] The `where` clause desugars to a `requires` entry in the function's vow block during parsing (or immediately after)
- [ ] A function with `where` clauses and an explicit `requires` block merges both sets of preconditions
- [ ] The canonical printer outputs the `where` form (round-trip: `parse -> print -> parse` is idempotent)
- [ ] Type checking, IR lowering, codegen, and verification all work identically to an equivalent `requires` block
- [ ] Error messages for `where` clause violations reference the parameter and predicate clearly
- [ ] `cargo test --all` passes
- [ ] `cargo clippy --all -- -D warnings` passes

### US-010: Write verified example programs — base type contracts
**Priority:** 6
**Description:** As a user or agent learning Vow, I want example programs demonstrating `requires`, `ensures`, `invariant`, and blame tracking with base types so I can understand the contract system.

**Acceptance Criteria:**
- [ ] Example: `ensures` with `result` keyword (e.g., `fn abs(x: i64) -> i64 { ensures: result >= 0 }`)
- [ ] Example: multiple contracts per function (requires + ensures together)
- [ ] Example: multi-function call chain where a `requires` violation correctly blames the Caller
- [ ] Example: multi-function call chain where an `ensures` violation correctly blames the Callee
- [ ] Example: loop invariant with `invariant` block on a while loop
- [ ] Each example compiles, runs (in debug mode), and verifies (with `--no-verify` removed) without errors
- [ ] Each example includes a comment-free header line (e.g., `fn name(...) -> ... { requires: ... }`) that serves as its own documentation
- [ ] All examples are placed in `examples/` directory

### US-011: Write verified example programs — collection contracts
**Priority:** 7
**Description:** As a user or agent, I want example programs demonstrating contracts over Vec, String, and HashMap so I can write verified programs with heap types.

**Acceptance Criteria:**
- [ ] Example: Vec with length contract (`ensures: result.len() == n`)
- [ ] Example: Vec with bounds-checking contract (accessing within `requires: i < v.len()`)
- [ ] Example: String length contract
- [ ] Example: HashMap key-presence contract (`ensures: m.contains_key(k)` after insert)
- [ ] Example: loop that builds a collection with an invariant tracking its length
- [ ] Each example compiles, runs, and verifies successfully
- [ ] At least one example per collection type demonstrates a violation (counterexample is produced)
- [ ] All examples placed in `examples/` directory

### US-012: Write verified example programs — where clause and combined patterns
**Priority:** 7
**Description:** As a user or agent, I want examples using the `where` clause syntax and combining multiple contract features.

**Acceptance Criteria:**
- [ ] Example: function using `where` clause on parameter(s)
- [ ] Example: function combining `where` clause with explicit `ensures` block
- [ ] Example: program that triggers a CEGIS-style repair cycle (contract fails, counterexample points to fix, fixed version verifies)
- [ ] Each example compiles, runs, and verifies
- [ ] All examples placed in `examples/` directory

### US-013: End-to-end CEGIS loop integration test
**Priority:** 8
**Description:** As a developer, I want an automated test that exercises the full CEGIS loop (compile → all diagnostics → verify → structured counterexample → source mapping) to ensure the pipeline works end-to-end.

**Acceptance Criteria:**
- [ ] Test compiles a program with an intentional contract violation
- [ ] Test asserts the build JSON contains the `"diagnostics"` array (may be empty if no compile errors)
- [ ] Test asserts the build JSON contains a `"counterexamples"` array with at least one entry
- [ ] Test asserts the counterexample contains `inputs` with source-level variable names, `violation` predicate text, and `source` location
- [ ] Test compiles a corrected version of the same program and asserts verification passes (empty counterexamples)
- [ ] Test runs as part of `cargo test --all`

## Functional Requirements

- FR-1: The build JSON output object must include a `"diagnostics"` field containing an array of all diagnostics emitted during compilation. Each diagnostic must have `error_code`, `message`, `severity`, and `span` (with `file`, `offset`, `length`).
- FR-2: The build JSON output object must include a `"counterexamples"` field when verification is run. Each counterexample must have `inputs` (source-level variable names to values), `violation` (predicate text), `vow_id`, and `source` location.
- FR-3: The build JSON must include `"verify_status"` as one of: `"proven"`, `"counterexample"`, `"timeout"`, `"error"`, `"skipped"`.
- FR-4: Runtime `__vow_violation` must emit `file` and `offset` fields in its JSON output.
- FR-5: The ESBMC C emitter must model `Vec` with `len()`, `push()`, and indexed access, maintaining a symbolic length counter.
- FR-6: The ESBMC C emitter must model `String` with `len()` and `contains()` (conservative for `contains`).
- FR-7: The ESBMC C emitter must model `HashMap` with `len()`, `contains_key()`, and `insert()`, maintaining a symbolic length and key set.
- FR-8: The parser must accept `where <expr>` after a parameter type annotation, desugaring it to a `requires` entry in the function's vow block.
- FR-9: `where` clauses must compose with explicit `requires` blocks (all preconditions are merged).
- FR-10: The canonical printer must round-trip `where` clauses: `parse -> print -> parse` must be idempotent.
- FR-11: At least 10 verified example programs must be shipped in `examples/`, covering: `requires`, `ensures` with `result`, `invariant`, blame tracking (Caller and Callee), `where` clause, Vec contracts, String contracts, HashMap contracts, and multi-function call chains.
- FR-12: All JSON schema changes must be backward-compatible: new fields are added, no existing fields are removed or renamed.

## Non-Goals

- No LSP server implementation (Phase 12)
- No MCP server implementation (Phase 12)
- No automated CEGIS agent that iterates the loop autonomously (Phase 13 — "Vow Pilot")
- No refinement type syntax (`type NonZero = { x: i64 | x > 0 }`) — only `where` on parameters
- No `where` clause on return types (only on parameters)
- No changes to the self-hosted compiler (compiler/ directory) — Phase 10 targets the Rust reference compiler only
- No new data types or language features beyond `where` clause syntax
- No incremental compilation or verification caching
- No changes to the human-readable output format (dual output invariant preserved)
- No constrained decoding / grammar-guided sampling support

## Technical Considerations

### Build JSON schema

The current build JSON has a top-level structure like `{"status": "Verified", ...}`. The new fields are additive:

```json
{
  "status": "VerifyFailed",
  "diagnostics": [
    {
      "error_code": "TypeMismatch",
      "message": "expected i64, got String",
      "severity": "error",
      "span": { "file": "examples/foo.vow", "offset": 142, "length": 5 }
    }
  ],
  "verify_status": "counterexample",
  "counterexamples": [
    {
      "vow_id": 1,
      "violation": "y != 0",
      "inputs": { "y": 0 },
      "source": { "file": "examples/divide.vow", "offset": 42, "length": 6 }
    }
  ]
}
```

### ESBMC output parsing

ESBMC can output XML (`--xml-ui`) or text. The XML format is more structured and preferred. The parser should handle both, falling back to text regex parsing if XML is unavailable. Key data to extract: variable assignments in the counterexample trace, assertion failure location, property description.

### Collection modeling in C emitter

The C emitter (vow-verify/src/c_emitter.rs) currently models IR instructions as C code for ESBMC. Collection types need abstract models:

- **Vec**: struct with `int len; int cap;` fields. `push` increments `len`. `get(i)` asserts `i >= 0 && i < len` and returns nondet. `len()` returns the `len` field.
- **String**: struct with `int len;`. `len()` returns `len`. `contains()` returns nondet bool (sound over-approximation).
- **HashMap**: struct with `int len; int keys[MAX_KEYS];`. `insert(k,v)` adds to key array, increments len. `contains_key(k)` checks key array. `len()` returns `len`.

MAX_KEYS should be a configurable bound (default 16) for bounded model checking.

### `__vow_violation` ABI change

Adding `file` and `offset` parameters changes the C calling convention. This is a breaking change to the runtime ABI. All codegen call sites (Cranelift backend) must be updated simultaneously. The self-hosted compiler's cgen.vow does NOT need updating (non-goal).

### Dependency order

US-001 and US-002 are foundational — everything else depends on diagnostics being in the JSON. US-003 and US-004 depend on US-002 (file paths in spans). US-006/007/008 are independent of US-003 but US-011 (collection examples) depends on them. US-009 (where clause) is independent. US-013 (integration test) depends on all pipeline stories.

## Success Metrics

- An AI agent (Claude Code) can compile a Vow program with errors, parse the build JSON, identify all errors from the diagnostics array, and fix them in a single pass — without recompiling after each fix
- An AI agent can invoke ESBMC verification, parse a structured counterexample, identify the violating input values and predicate, and suggest a targeted code fix
- All 10-15 example programs compile, run (debug mode), and verify without errors
- At least 3 example programs demonstrate the full counterexample → fix → verify-pass cycle
- The `where` clause syntax round-trips through `parse -> print -> parse` identically
- `cargo test --all` and `cargo clippy --all -- -D warnings` pass with zero warnings
- Build JSON schema is documented (in this PRD) and stable for agent consumption

## Open Questions

1. **ESBMC XML vs text parsing**: Should we require ESBMC XML output (`--xml-ui`) or support both? XML is cleaner but may not be available in all ESBMC versions.
2. **HashMap bounded model size**: What should the default `MAX_KEYS` bound be for HashMap verification? 16? 32? Should it be configurable via a CLI flag?
3. **`where` clause error messages**: Should a `where y != 0` violation say "requires: y != 0 (from where clause)" or just "requires: y != 0"? Does blame tracking need to distinguish inline `where` from explicit `requires`?
4. **Counterexample for collection types**: When ESBMC finds a counterexample involving a Vec, what does the `inputs` field look like? `{"v.len()": 0}` or `{"v": {"len": 0}}`?
5. **Example program naming convention**: Should examples use a flat naming scheme (`examples/vec_contract.vow`) or subdirectories (`examples/contracts/vec.vow`, `examples/blame/caller.vow`)?
