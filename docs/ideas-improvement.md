# Vow Language & Tooling: Ideas for Improvement

This file records observations and ideas for improving Vow (the language and tooling)
gathered during self-hosting work and agent-assisted development sessions.
Each entry includes the source observation and a concrete suggestion.

---

## Language

### 1. ~~Chained field access on non-tagged types causes silent wrong-index bugs~~ ADDRESSED

**Status:** Fixed in Pre-Wave 4 hardening. The IR lowerer now builds a
`struct_field_type_names` map from AST struct definitions and auto-tags
FieldGet results with their declared type names. Chained field access like
`e.ts.strs[fsid]` now works without annotated `let` bindings.

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

### 3. ~~No distinction between "unknown type" and "diverging expression"~~ ADDRESSED

**Status:** Fixed in Pre-Wave 4 hardening. Added `CTY_UNKNOWN` (tid 21) for
unresolved cross-module types. `CTY_NEVER` is now reserved for diverging
expressions only. `is_coercible` treats both `CTY_NEVER` and `CTY_UNKNOWN`
as coercible to/from anything. `resolve_ast_ty` returns `CTY_UNKNOWN` for
types not found in local struct/enum definitions.

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

### 5. ~~Method chaining on unknown types produces `CTY_UNIT` silently~~ ADDRESSED

**Status:** Fixed in Pre-Wave 4 hardening. All receiver-based expression handlers
(EXPR_METHOD, EXPR_INDEX, EXPR_FIELD, EXPR_QUESTION) now use `is_opaque()` guard
to propagate CTY_NEVER/CTY_UNKNOWN instead of defaulting to CTY_UNIT.

---

## Tooling

### 6. ~~`inst_struct_type` silently defaults to field index 0~~ ADDRESSED

**Status:** Fixed in Pre-Wave 4 hardening. The lowerer now emits `eprintln!`
warnings when FieldGet/FieldSet receivers are not found in `inst_struct_type`,
and when StructLiteral field names are not found in the struct definition.
The `unwrap_or(0)` fallback remains for backwards compatibility, but the
warning makes the issue visible during compilation.

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

### 11. ~~Error messages lack source location~~ ADDRESSED

**Status:** Fixed in Pre-Wave 4 hardening. `CheckEnv` now tracks `cur_fn_name`
(set at `check_fn` entry). `env_emit_error` prepends the function name:
`"error in fn <name>: <message>"`. Expression-level spans remain unpopulated
(parser doesn't set them); function names are sufficient for most debugging.

---

### 12. ~~`CTY_NEVER` propagation must be implemented manually at each expression kind~~ ADDRESSED

**Status:** Fixed in Pre-Wave 4 hardening. Added `is_opaque(tid)` helper that
checks both `CTY_NEVER` and `CTY_UNKNOWN`. All receiver-based handlers
(EXPR_FIELD, EXPR_METHOD, EXPR_INDEX, EXPR_QUESTION) and `check_block` now use
`is_opaque()` as the standard guard pattern. If/match branch type selection
also uses `is_opaque()` instead of checking only CTY_NEVER.

---
