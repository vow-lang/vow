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

`compiler/` contains a complete Vow implementation of the compiler (6276 lines across 13 modules). The bootstrap triple test passes: the self-hosted compiler is a verified fixed point.

### Modules

- `span.vow`, `token.vow`, `lexer.vow` — lexer (Wave 1)
- `ast.vow`, `parser.vow` — parser (Wave 2)
- `types.vow`, `env.vow`, `checker.vow` — type checker (Wave 3)
- `ir.vow`, `ir_printer.vow`, `lower.vow` — IR lowering and printing (Wave 4)
- `cgen.vow` — C code generator (AST → C, pointer-based struct representation)
- `main.vow` — driver with `--cgen` flag for C output or IR output

### Building and running

```bash
./target/release/vow --no-verify compiler/main.vow   # compile → ./compiler/main
./compiler/main compiler/lexer.vow                    # type-check, print IR
./compiler/main --cgen compiler/lexer.vow             # generate lexer.vow.c
```

`./compiler/main` loads a single file only — no recursive `use` resolution. Cross-module
types appear as unknown (`CTY_NEVER`); errors from unresolved types are suppressed.

### Bootstrap triple test

`scripts/concat_vow.sh` merges all compiler modules into a single file (stripping `module`/`use` headers), avoiding the need for module loading.

```bash
./scripts/concat_vow.sh cgen > /tmp/compiler_cgen.vow          # concatenate all modules
./target/release/vow --no-verify /tmp/compiler_cgen.vow -o /tmp/compiler_a  # Stage 0: Rust → Binary A
ulimit -v 2000000; /tmp/compiler_a --cgen /tmp/compiler_cgen.vow            # Stage 1: A → stage1.c
gcc /tmp/compiler_cgen.vow.c -L target/release -lvow_runtime -lpthread -ldl -lm -o /tmp/compiler_b
ulimit -v 2000000; /tmp/compiler_b --cgen /tmp/compiler_cgen.vow            # Stage 2: B → stage2.c
diff /tmp/stage1.c /tmp/stage2.c                                            # must be empty
```

**Important:** Always use `ulimit -v 2000000` when running self-compiled binaries to cap memory.

### Gotchas

- Chained field access on struct values requires annotated `let` bindings.
  `e.ts.strs[i]` reads the wrong field index. Use:
  ```vow
  let ts: TyStore = e.ts;
  let s: String = ts.strs[i];
  ```
- cgen.vow uses pointer-based structs: `calloc(n_fields, sizeof(long))`, fields accessed via `((long*)ptr)[index]`. Variable types are tracked per-function in CGen for string equality dispatch (`__vow_string_eq` vs pointer `==`).
- `__vow_string_eq` returns `i64` (not `bool`) to avoid C ABI mismatch with Rust's 1-byte bool.

### Examples

The `examples/` directory contains runnable `.vow` programs:
- `divide.vow` — `requires: y != 0` (triggers Caller violation when run with `--mode debug`)
- `hello.vow` — basic IO
- `bisect.vow` — loop invariant
- `countdown.vow` — while loop
