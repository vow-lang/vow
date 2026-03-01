# Vow Language & Tooling: Ideas for Improvement

This file records observations and ideas for improving Vow (the language and tooling)
gathered during self-hosting work and agent-assisted development sessions.
Each entry includes the source observation and a concrete suggestion.

---

## Language

### 1. Chained field access on non-tagged types causes silent wrong-index bugs

**Observation (Phase 9 Wave 3):**
`e.ts.strs[fsid]` caused a segfault. The intermediate result `e.ts` (a FieldGet)
is not automatically tagged in `inst_struct_type`, so the second `.strs` lookup
defaults to field index 0 (`data`) instead of index 1 (`strs`).

The workaround is:
```vow
let ts_local: TyStore = e.ts;
let fn_str: String = ts_local.strs[fsid];
```

**Suggestions:**
- The IR lowerer should propagate struct type tags through FieldGet results
  automatically. When a FieldGet's result type is a known struct (from the
  struct definition), tag the result in `inst_struct_type`.
- Alternatively, make the language require explicit annotated `let` for all
  struct-typed intermediate values (enforce via the type checker error rather
  than silent miscompilation).

---

### 2. No cross-module type resolution in single-file self-hosted checker

**Observation (Phase 9 Wave 3):**
When running `./compiler/main compiler/parser.vow`, only `parser.vow`'s items
are parsed (no recursive `use` loading). Types from `ast.vow`, `token.vow`, etc.
are resolved as opaque (`CTY_ENUM("X")` fallback) rather than their true kinds.

This forces the type checker to be lenient:
- `EXPR_FIELD` on unknown struct → return `CTY_NEVER` (not `CTY_UNIT`)
- `EXPR_SLIT` for unknown struct name → return `CTY_NEVER`
- `EXPR_INDEX` / `EXPR_METHOD` on `CTY_NEVER` → propagate `CTY_NEVER`

**Suggestions:**
- The self-hosted main.vow should follow `use` declarations and load/merge
  dependent modules before type checking (module loading was implemented in
  the Rust compiler but not yet in the self-hosted binary).
- Until then, document the leniency rules so future checkers know why they exist.
- Or introduce a `?` type (unknown/top) that is distinct from `Never` for
  representing "type not yet resolved" to avoid semantic confusion.

---

### 3. No distinction between "unknown type" and "diverging expression"

**Observation (Phase 9 Wave 3):**
The self-hosted type checker uses `CTY_NEVER` both for:
- Expressions that truly diverge (`return`, `break`, `process_exit()`)
- Expressions whose types cannot be determined (cross-module calls, field
  access on unregistered structs)

This conflation causes the type checker to suppress legitimate errors
(since `CTY_NEVER` is coercible to any type).

**Suggestion:**
Introduce a `CTY_UNKNOWN` (top type / type hole) distinct from `CTY_NEVER`.
`CTY_UNKNOWN` would be coercible to anything AND anything would be coercible
to `CTY_UNKNOWN`, representing "type not yet known". `CTY_NEVER` would remain
strictly for diverging expressions.

---

### 4. Struct-vs-enum ambiguity for unknown named types

**Observation (Phase 9 Wave 3):**
When a named type (e.g., `Module`) is not declared in the current module,
`resolve_ast_ty` cannot tell if it's a struct or an enum. It falls back to
`ts_make_enum`, but struct literals (`EXPR_SLIT`) return `ts_make_struct`.
This creates an incompatibility:
- `fn foo() -> Module` registers `Module` as `CTY_ENUM`
- `Module { ... }` struct literal produces `CTY_STRUCT`
- `is_coercible(CTY_STRUCT("Module"), CTY_ENUM("Module"))` → false

**Suggestion:**
- For self-hosting: use `CTY_NEVER` for unknown types to suppress false errors.
- Long term: always load all modules before type checking, so every name is resolved.
- Or: make `tids_equal` treat `CTY_STRUCT(name) ↔ CTY_ENUM(name)` as compatible
  when the type was not declared locally.

---

### 5. Method chaining on unknown types produces `CTY_UNIT` silently

**Observation (Phase 9 Wave 3):**
`m.items.len()` where `m` is an unknown struct:
- `m.items` → `CTY_NEVER` (fixed in EXPR_FIELD)
- `(m.items).len()` → `CTY_UNIT` (EXPR_METHOD defaulted without propagating NEVER)

This caused "let binding type mismatch" for `let n: i64 = m.items.len();`.

**Suggestion:**
Any operator/method that would be applied to `CTY_NEVER` should propagate
`CTY_NEVER` rather than returning `CTY_UNIT`. This applies to:
- `EXPR_METHOD`
- `EXPR_INDEX`
- `EXPR_FIELD` (already fixed)

---

## Tooling

### 6. `inst_struct_type` silently defaults to field index 0

**Observation (Phase 9 Wave 3):**
When a FieldGet can't find the struct type in `inst_struct_type`, the code uses
`unwrap_or(0)` as the field index. This means a wrong struct type causes field
access to silently read the WRONG field — leading to a crash (reading an i64 as
a VowVec pointer) rather than an early error.

**Suggestion:**
- Emit a compile-time diagnostic (warning or error) when a FieldGet's receiver
  is not found in `struct_field_map`. This would surface the bug at compile time
  instead of causing a runtime segfault.
- Alternatively, emit a runtime trap (`unreachable!()`) if the field index cannot
  be resolved, making the failure visible and debuggable.

---

### 7. String equality requires special IR handling invisible to the developer

**Observation (Phase 9 Wave 3):**
`fn_str == fname` would silently use pointer comparison (`EqI64`) unless one of
the operands was tagged as "String" in `inst_struct_type`. Since `inst_struct_type`
tagging depends on where a value came from (let binding annotation, function param
type, etc.), the behavior was invisible and non-obvious.

A `String` value from a FieldGet (e.g., `e.ts.strs[fsid]`) was NOT automatically
tagged, so `==` used pointer comparison rather than byte-wise comparison.

**Suggestion:**
- String equality should be handled at the TYPE level (if both operands have
  `Ty::String`, emit `__vow_string_eq`), not via runtime tagging of IR instructions.
- Or: make the tagging automatic for any instruction that is transitively derived
  from a `String`-typed source (SSA-style type propagation through FieldGet, Index,
  method returns, etc.).

---

### 8. No way to express "opaque/unknown" struct for partial type checking

**Observation (Phase 9 Wave 3):**
The self-hosted type checker can only check code whose dependencies are either
(a) declared in the same file, or (b) in the runtime builtins. There's no mechanism
to provide "stub" type information for cross-module types without loading the full
module AST.

**Suggestion:**
- Allow a `.vow.d` (declaration file) format that contains only type signatures
  without implementations. The type checker can load these stubs for cross-module
  checking without parsing full source.
- Or: export a serialized type environment from a compiled module and allow the
  checker to import it.

---

### 9. Debugging self-hosted code requires manual `eprintln_str` instrumentation

**Observation (Phase 9 Wave 3):**
Debugging a segfault required inserting `eprintln_str(String::from("CB1"))` etc.
at every step. There is no structured trace/log or breakpoint mechanism.
Rebuilding the entire compiler for each debug iteration was slow.

**Suggestion:**
- A debug mode that prints function entry/exit (name + arg values) would
  dramatically reduce self-hosting debug cycles.
- A compile-time flag `--debug-trace` that instruments every function entry
  with an `eprintln` would be useful for agent-driven debugging.
- Or: provide a way to attach a post-mortem to a segfault that shows the Vow
  call stack (via frame pointers or a shadow stack in debug mode).

---

### 10. Incremental self-hosted binary rebuild is the main development bottleneck

**Observation (Phase 9 Wave 3):**
Each change to `compiler/*.vow` requires:
1. `cargo build --all --release` (fast if Rust is cached)
2. `./target/release/vow --no-verify compiler/main.vow` to compile the self-hosted binary
3. Test the result

Step 2 takes significant time. There is no incremental compilation at the Vow level.

**Suggestion:**
- Implement a module-level caching scheme for the Vow compiler: if a `.vow` file
  hasn't changed, skip recompiling it.
- Or: support a `--watch` mode in `vowc` that rebuilds on file change.

---

### 11. Error messages lack source location

**Observation (Phase 9 Wave 3):**
The self-hosted checker emits error strings like "let binding type mismatch" and
"function body type mismatch" without any information about WHERE in the source
the error occurred (which function, which line, which variable).

This forces iterative debugging: add prints, narrow down, repeat.

**Suggestion:**
- Pass span information through to `env_emit_error`. The checker already has
  access to the AST arena, which stores spans for expressions, statements,
  and patterns.
- Even just printing the function name where the error occurred would reduce
  debug time significantly.
- Format: `"error at fn <name>, stmt <sid>: <message>"`.

---

### 12. `CTY_NEVER` propagation must be implemented manually at each expression kind

**Observation (Phase 9 Wave 3):**
After making EXPR_FIELD return `CTY_NEVER` for unknown types, we had to separately
fix EXPR_INDEX, EXPR_METHOD, and check_block statement propagation to also handle
`CTY_NEVER` receivers/results. Each required a separate fix.

**Suggestion:**
Add a convention: every expression handler that takes a receiver should check
`if recv_tid == CTY_NEVER() { return CTY_NEVER(); }` at the top. Or better:
add a general wrapper in check_expr that propagates NEVER from sub-expressions
before dispatching to specific handlers.

---
