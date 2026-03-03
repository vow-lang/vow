# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build --all          # build all crates
cargo test --all           # run all tests
cargo test -p vow-syntax   # run tests for a single crate
cargo test lexer::tests::lex_keywords  # run a single test by path
cargo clippy --all -- -D warnings      # lint (CI enforces zero warnings)
cargo fmt --all            # format all code
```

## vowc CLI

First build the release binary:
```bash
cargo build --all --release
```
Then use `./target/release/vow` (or `cargo install --path vow` to get `vowc` on PATH):

```bash
./target/release/vow examples/divide.vow                   # compile (release, no runtime vow checks)
./target/release/vow --mode debug examples/divide.vow      # compile with runtime vow violation checks
./target/release/vow --no-verify examples/divide.vow       # skip ESBMC static verification
./target/release/vow --help                                # JSON capability description (for agents)
./target/release/vow --help --human                        # human-readable capability description
```

Debug mode is required to see runtime `VowViolation` output. Release omits all vow checks.

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
- **`vow-verify`** — ESBMC integration. Extracts verification conditions from IR, invokes ESBMC, maps counterexamples back to source via `Origin` metadata.
- **`vow-clif-shim`** — `extern "C"` FFI shims wrapping Cranelift for the self-hosted compiler. The self-hosted `clif.vow` calls these shims to produce native object files directly. Uses stack slots (not SSA) to bypass Cranelift dominance requirements for cross-block references in the self-hosted IR.
- **`vow-runtime`** — vow violation handler (`__vow_violation`), print helpers (`__vow_print_str`, `__vow_print_i64`), arithmetic overflow handler.
- **`vow`** — CLI driver (`vowc`). Orchestrates the parallel codegen + verification pipeline. Structured JSON build output.
- **`vow-diag`** — `Diagnostic`, `ErrorCode`, `Blame` (Caller/Callee), `JsonEmitter`, `HumanEmitter`.

### Key design invariants

- **No comments in Vow source.** The lexer does not handle `//` or `/*`. Intent lives in contracts.
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

## Self-Hosted Compiler (Phase 9)

`compiler/` contains a complete Vow implementation of the compiler (13 modules). The bootstrap triple test passes: the self-hosted compiler is a verified fixed point producing byte-identical binaries.

### Modules

- `span.vow`, `token.vow`, `lexer.vow` — lexer (Wave 1)
- `ast.vow`, `parser.vow` — parser (Wave 2)
- `types.vow`, `env.vow`, `checker.vow` — type checker (Wave 3)
- `ir.vow`, `ir_printer.vow`, `lower.vow` — IR lowering and printing (Wave 4)
- `clif.vow` — Cranelift backend via FFI shims (`vow-clif-shim` crate)
- `main.vow` — driver with `-o <path>` for native compilation or IR text output

### Building and running

```bash
./target/release/vow --no-verify compiler/main.vow   # compile → ./compiler/main
./compiler/main compiler/lexer.vow                    # type-check, print IR
./compiler/main -o /tmp/lexer compiler/lexer.vow      # compile to native binary
```

The self-hosted compiler supports DFS module loading via `use` declarations.

### Bootstrap triple test

`scripts/concat_vow.sh` merges all compiler modules into a single file (stripping `module`/`use` headers), avoiding the need for module loading.

```bash
./scripts/concat_vow.sh clif > /tmp/compiler_clif.vow
./target/release/vow --no-verify /tmp/compiler_clif.vow -o /tmp/compiler_a  # Stage 0: Rust → Binary A
ulimit -v 2000000; /tmp/compiler_a -o /tmp/compiler_b /tmp/compiler_clif.vow  # Stage 1: A → B
ulimit -v 2000000; /tmp/compiler_b -o /tmp/compiler_c /tmp/compiler_clif.vow  # Stage 2: B → C
sha256sum /tmp/compiler_b /tmp/compiler_c              # must be identical (binary fixed point)
```

**Important:** Always use `ulimit -v 2000000` when running self-compiled binaries to cap memory.

### vow-clif-shim architecture

The shim exposes a medium-granularity FFI API: module-level operations are separate calls
(`__vow_clif_create`, `__vow_clif_add_string`, `__vow_clif_declare_function`, `__vow_clif_finish`,
`__vow_clif_link`), while per-function compilation is a single call (`__vow_clif_compile_function`)
that receives the function's IR as flattened parallel arrays (ids, ops, types, data kinds, values,
strings, args). This avoids Cranelift `FunctionBuilder` lifetime issues across FFI boundaries.

The shim uses stack slots instead of SSA values for all instruction results. This is necessary
because the self-hosted IR has cross-block references between sibling branches (e.g., an else
branch referencing a value from the then branch), which violates Cranelift's SSA dominance
requirements. Each instruction result is stored in a stack slot; reads load from the slot.
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
