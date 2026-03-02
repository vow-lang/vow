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

`compiler/` contains a Vow implementation of the compiler front-end:
- `span.vow`, `token.vow`, `lexer.vow` — lexer (Wave 1)
- `ast.vow`, `parser.vow` — parser (Wave 2)
- `types.vow`, `env.vow`, `checker.vow` — type checker (Wave 3)
- `main.vow` — driver: runs lexer → parser → type checker, prints error count

Compile and run the self-hosted binary:
```bash
./target/release/vow --no-verify compiler/main.vow   # compile → ./compiler/main
./compiler/main compiler/lexer.vow                   # 3932 tokens, 11 items, 0 errors
./compiler/main compiler/parser.vow                  # 6760 tokens, 38 items, 0 errors
./compiler/main compiler/checker.vow                 # self-checks with 0 errors
```

`./compiler/main` loads a single file only — no recursive `use` resolution. Cross-module
types appear as unknown (`CTY_NEVER`); errors from unresolved types are suppressed.

**Gotcha:** Chained field access on struct values requires annotated `let` bindings.
`e.ts.strs[i]` reads the wrong field index. Use:
```vow
let ts: TyStore = e.ts;
let s: String = ts.strs[i];
```

### Examples

The `examples/` directory contains runnable `.vow` programs:
- `divide.vow` — `requires: y != 0` (triggers Caller violation when run with `--mode debug`)
- `hello.vow` — basic IO
- `bisect.vow` — loop invariant
- `countdown.vow` — while loop
