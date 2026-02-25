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
- **`vow-codegen`** — `Backend` trait + Cranelift backend. Debug builds compile vow blocks to traps; release builds omit them entirely.
- **`vow-verify`** — ESBMC integration. Extracts verification conditions from IR, invokes ESBMC, maps counterexamples back to source via `Origin` metadata.
- **`vow-runtime`** — arena allocator, trace facility, vow violation handler.
- **`vow`** — CLI driver (`vowc`). Orchestrates the parallel codegen + verification pipeline. Structured JSON build output.
- **`vow-diag`** — `Diagnostic`, `ErrorCode`, `Blame` (Caller/Callee), `JsonEmitter`, `HumanEmitter`.

### Key design invariants

- **No comments in Vow source.** The lexer does not handle `//` or `/*`. Intent lives in contracts.
- **Canonical form.** The printer is a compiler pass, not a formatter. `parse → print → parse` must be idempotent. Tests enforce this.
- **Effects are explicit.** Pure functions have empty effect sets. Calling an effectful function from a pure one is a type error. Effect sets are part of every function's type.
- **Vow blocks have blame.** `requires` violations blame the Caller; `ensures`/`invariant` violations blame the Callee. This is encoded in `vow-diag::Blame`.
- **Linear types.** `linear struct` values must be consumed exactly once. The type checker tracks this.
- **Checked arithmetic operators** (`+!`, `-!`, etc.) are distinct token kinds from wrapping arithmetic (`+`, `-`, etc.) — both are in the grammar, not library functions.

### Span

`Span { start: u32, len: u32 }` lives in `vow-syntax::span` and is the single source-location type. Every AST node and every token carries one.
