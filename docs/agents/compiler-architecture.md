# Vow repository guidance

Paths and commands below are relative to the repository root.

## Architecture

Vow is a Rust workspace where each crate maps to one compiler pipeline stage. The planned pipeline is:

```
Source → vow-syntax → vow-types → vow-ir → vow-codegen → executable
                                       └──→ vow-verify  → proof / counterexample
```

All diagnostic output flows through **`vow-diag`**, which every other crate uses. It always emits both JSON (for agents) and human-readable text in parallel — this is by design, not a flag.

### Crate responsibilities

- **`vow-syntax`** — lexer, token types, AST, canonical printer, `Span` type. `Span` is the shared source-location primitive used across all crates.
- **`vow-types`** — type checker and effect checker. Decidable base types only; refinement predicates go to `vow-verify`.
- **`vow-ir`** — Pizlo-style SSA IR with instruction-value uniformity. Every `Inst` is a value. Phi/Upsilon nodes (not traditional SSA Phi). `InsertionSet` is the standard IR mutation primitive.
- **`vow-codegen`** — `Backend` trait + Cranelift backend. Debug builds emit runtime vow checks via `__vow_violation`; release builds omit them entirely.
- **`vow-linker`** — shared platform-specific native library policy used by both linker paths.
- **`vow-verify`** — ESBMC integration. Extracts verification conditions from IR, invokes ESBMC, maps counterexamples back to source via `Origin` metadata.
- **`vow-perf`** — fixed complexity-class canonicalization, least-squares classification, and cloned-IR operation-count instrumentation for a separate performance-test artifact. Parser, generator, and CLI integration remain tracked by the performance-guarantees roadmap.
- **`vow-clif-shim`** — `extern "C"` FFI shims wrapping Cranelift for the self-hosted compiler. The self-hosted `clif.vow` calls these shims to produce native object files directly. Uses stack slots (not SSA) to bypass Cranelift dominance requirements for cross-block references in the self-hosted IR.
- **`vow-runtime`** — vow violation handler (`__vow_violation`), print helpers (`__vow_string_print`, `__vow_print_i64`), arithmetic overflow handler.
- **`vow`** — CLI driver (`vowc`). Orchestrates the parallel codegen + verification pipeline. Structured JSON build output.
- **`vow-diag`** — `Diagnostic`, `ErrorCode`, `Blame` (Caller/Callee), `JsonEmitter`, `HumanEmitter`.

### Key design invariants

- **Line comments only.** `//` comments are stripped at lex time (like whitespace). No block comments (`/* */`). Machine-relevant intent belongs in contracts; comments are for non-semantic rationale.
- **Canonical form.** The printer is a compiler pass, not a formatter. `parse → print → parse` must be idempotent. Tests enforce this.
- **Effects are explicit.** Pure functions have empty effect sets. Calling an effectful function from a pure one is a type error. Effect sets are part of every function's type.
- **Vow blocks have blame.** `requires` violations blame the Caller; `ensures`/`invariant` violations blame the Callee. This is encoded in `vow-diag::Blame`.
- **Linear types.** `linear struct` values must be consumed exactly once. The type checker tracks this.
- **Checked arithmetic operators** (`+!`, `-!`, etc.) are distinct token kinds from wrapping arithmetic (`+`, `-`, etc.) — both are in the grammar, not library functions.
- **VowViolation diagnostic shape.** JSON emitted on violation:
  `{"error":"VowViolation","vow_id":N,"blame":"Caller"|"Callee","description":"...","values":{"var":val,...}}`
  The `values` field contains runtime values of all free variables in the predicate. `VowEntry.bindings`
  in `vow-ir` stores the `(name, InstId)` pairs that drive this capture.

### Span

`Span { start: u32, len: u32 }` lives in `vow-syntax::span` and is the single source-location type. Every AST node and every token carries one.

## Self-Hosted Compiler

`compiler/` contains a complete Vow implementation of the compiler (13 modules). `build/vowc` is the primary compiler binary — a verified fixed point producing byte-identical binaries. The self-hosted compiler has full feature parity with the Rust compiler: subcommands, flags, structured diagnostics, verification pipeline, and parallel codegen+verify.

### Modules

- `span.vow`, `token.vow`, `lexer.vow` — lexer (Wave 1)
- `ast.vow`, `parser.vow` — parser (Wave 2)
- `types.vow`, `env.vow`, `checker.vow` — type checker (Wave 3)
- `ir.vow`, `ir_printer.vow`, `contract_text.vow`, `lower.vow` — IR lowering and printing (Wave 4); `contract_text.vow` renders contract predicate text identically to `vow-syntax/src/printer.rs`
- `clif.vow` — Cranelift backend via FFI shims (`vow-clif-shim` crate)
- `verify_report.vow` — failed-verification report pieces (violated-clause source, call-site argument values, one diagnostic per counterexample) that keep the `vow verify` JSON identical to the Rust driver's
- `main.vow` — driver entry point: loads the driver modules and dispatches the subcommands
  (`build`, `verify`, `test`, `contracts`, ...). The driver bodies live in `main_*.vow` modules,
  one per responsibility (`main_args`, `main_replay`, `main_verify_*`, `main_build`,
  `main_test*`, `main_skill`, `main_contracts`, ...)
- `help_text_gen.vow`, `skill_text_gen.vow`, `skill_bundle_NN_gen.vow`, `skill_docs_NN_gen.vow` —
  whole generated modules holding the embedded `--help` JSON, human help and skill text.
  `scripts/generate_help.py` writes them from `docs/spec/`; never edit them by hand. A file named
  `*_gen.vow` is exempt from the authored-file budget (`.gitattributes`), so a generated block never
  belongs inside a hand-written module. The Rust twin is `vow/src/skill_gen/*.rs`, pulled into
  `vow/src/skill.rs` with `include!` (same rules; its tests live in `vow/src/skill/tests_*.rs`)

### Building and running

```bash
build/vowc build --no-verify compiler/main.vow -o /tmp/vow_main  # compile self-hosted compiler
/tmp/vow_main build --no-verify --dump-ir compiler/lexer.vow               # type-check, print IR
/tmp/vow_main build --no-verify -o /tmp/lexer compiler/lexer.vow            # compile to native binary
```

The self-hosted compiler supports DFS module loading via `use` declarations.

Compiler unit tests live in `compiler/tests/` and run through the test runner. Run one with
`build/vowc test compiler/tests/test_region.vow` (the module root is inferred: the nearest ancestor
of the file that resolves all its `use` declarations, here `compiler/`), or all of them with
`build/vowc test compiler/`, which runs files concurrently (`--jobs N`, default `min(cpus/2, 8)`,
backing off under memory/IO pressure) and reports them in sorted order. Use `--filter <stem>` to
select by file stem.

### Bootstrap triple test

`scripts/concat_vow.sh` merges all compiler modules into a single file (stripping `module`/`use` headers), avoiding the need for module loading.

```bash
./scripts/concat_vow.sh clif > /tmp/compiler_clif.vow
./target/release/vow --no-verify /tmp/compiler_clif.vow -o /tmp/compiler_a  # Stage 0: Rust → Binary A
/tmp/compiler_a build --no-verify -o /tmp/compiler_b /tmp/compiler_clif.vow  # Stage 1: A → B
/tmp/compiler_b build --no-verify -o /tmp/compiler_c /tmp/compiler_clif.vow  # Stage 2: B → C
sha256sum /tmp/compiler_b /tmp/compiler_c              # must be identical (binary fixed point)
```


### vow-clif-shim architecture

The shim exposes a medium-granularity FFI API: module-level operations are separate calls
(`__vow_clif_create`, `__vow_clif_add_string`, `__vow_clif_declare_function`, `__vow_clif_finish`,
`__vow_clif_link`). Per-function compilation is streamed incrementally —
`__vow_clif_fn_begin` opens a function, `__vow_clif_fn_block` / `__vow_clif_fn_inst` /
`__vow_clif_fn_vow` append to it, and `__vow_clif_fn_end` drives Cranelift codegen. The
shim owns the parallel arrays (ids, ops, types, data kinds, values, strings, args) as
scratch buffers on `ModuleContext` and reuses them across functions via
`Vec::clear()` (preserves capacity), eliminating the per-function alloc/realloc churn that
the previous batched `__vow_clif_compile_function` required. Cranelift `FunctionBuilder`
still operates on all of this at once, inside `fn_end`.

The shim uses stack slots instead of SSA values for all instruction results. This is necessary
because the self-hosted IR has cross-block references between sibling branches (e.g., an else
branch referencing a value from the then branch), which violates Cranelift's SSA dominance
requirements. Each instruction result is stored in a stack slot; reads load from the slot.
Each `Phi` also owns a shadow slot: `Upsilon` writes the shadow, and the `Phi` copies it into
the value slot on block entry so back-edge `Upsilon` groups retain parallel-copy semantics.
`BTreeMap` (not `HashMap`) is used for `slot_map` to ensure deterministic codegen for binary
fixed-point reproducibility.

VowVec layout: `{ ptr: *mut u8, len: usize, cap: usize }` = 24 bytes. The shim reads Vec and
String values directly through this layout. All FFI parameters are `i64` (opaque pointers or values).

### Gotchas

- Chained field access on struct values requires annotated `let` bindings.
  `e.ts.strs[i]` reads the wrong field index. Use:
  ```vow
  let ts: TyStore = e.ts;
  let s: String = ts.strs[i];
  ```
- `__vow_string_eq` returns `i64` (not `bool`) to avoid C ABI mismatch with Rust's 1-byte bool.
- `lctx_assign` in `lower.vow` does in-place mutation of `scope_vals[i]`. `lctx_restore` only
  pops entries by vector length — it cannot undo in-place mutations. When lowering if-else,
  mutation variable values must be explicitly saved before and restored after each branch.
- Vec indexed assignment (`v[i] = val`) in Vow emits `__vow_vec_set_val(v, i, val)`. Both the
  Rust IR lowerer and self-hosted `lower.vow` handle EXPR_INDEX on the LHS of assignments.

### Examples

The `examples/` directory contains runnable `.vow` programs:
- `divide.vow` — `requires: y != 0` (triggers Caller violation when run with `--mode debug`)
- `hello.vow` — basic IO
- `bisect.vow` — loop invariant
- `countdown.vow` — while loop
