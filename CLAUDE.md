# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Language Design Principles

Vow is a language for agents, not humans. New language features should only be added if they meet **all three** criteria:
1. **Does not make verification harder** — the ESBMC pipeline and C model must not become more complex or fragile.
2. **Eliminates a class of agent bugs** — the feature must make it harder for agents to generate incorrect code, not just shorter code.
3. **Makes agentic coding easier** — the feature must help agents produce correct, verified programs more reliably (not merely reduce verbosity).

Syntactic convenience for humans (string interpolation, pattern matching sugar, etc.) is not sufficient justification. Every feature added expands the compiler, C emitter, and verification surface area.

**Crisp rule:** add surface sugar only when it desugars to today's core semantics with near-zero verifier impact; reject anything that introduces a new type-system axis. This preserves Vow's identity.

## Production Quality

Vow is a serious, production-grade project. All implementation decisions must reflect this:

- **No shortcuts.** Do not assume programs will be small, short-lived, or low-allocation. Vow programs may be long-running services, compile large codebases, or process substantial data.
- **Memory management matters.** Every allocation must have a clear ownership story and a path to deallocation. "The process exits soon anyway" is never an acceptable justification for leaking memory.
- **Scalability is a requirement.** Data structures, algorithms, and compiler passes must be chosen for reasonable asymptotic behavior, not just correctness on small inputs.
- **No "experimental" excuses.** Do not defer correctness, robustness, or resource discipline with the rationale that the project is early-stage or experimental. Treat every change as if it will run in production.

## Development Discipline

These principles apply to all work in this repo — compiler code, self-hosted compiler, tooling, and benchmarks. They also apply to Vow programs written by agents (see `docs/spec/index.md`, which embeds the same guidance into the skill).

- **Deep modules.** Favor deep modules (lots of functionality, simple interface, hides complexity) over shallow ones (thin wrappers, complex interface, surface complexity). When a module's interface is nearly as wide as its implementation, collapse it or widen its responsibilities. Many exported symbols and pass-through functions are a shallow-module smell.
- **Surgical changes.** Many small changes beat one large change. They are easier to review, debug, `git bisect`, and revert. Do not bundle refactors, formatting, or unrelated cleanups into a bug fix. If a task grows, split it.
- **Small files, smaller functions.** Context is precious — every file read consumes budget. Keep files and functions short enough that an agent can load the relevant unit without displacing other context. Split by responsibility as soon as a unit stops fitting a single coherent idea.

## Contract Authoring

Contracts express **semantic correctness** — what is mathematically required for a function to be correct. They must never be weakened or artificially bounded to satisfy ESBMC's verification limits.

- **Write the true contract.** `gcd(a, b)` requires `a >= 0, b >= 0, a + b > 0`. It does not require `a <= 50` — that is a verifier limitation, not a property of Euclid's algorithm.
- **ESBMC bounds are not contracts.** Bounds like `n <= 10` (to fit within `--unwind 10`) or `a <= 100` (to help the SMT solver) are verification artifacts. They do not belong in `requires`/`ensures` clauses.
- **Postconditions should be tight.** `min(a, b)` must ensure `result == a || result == b`, not just `result <= a && result <= b`. A weak postcondition that admits incorrect implementations is a bad contract.
- **If ESBMC can't prove a correct contract, that's ESBMC's problem.** Mark the function as unverifiable or skip it in the verification pass. Do not distort the contract to accommodate the tool.
- **Only add bounds that reflect genuine semantic constraints.** Overflow guards (e.g., `requires: x > -9223372036854775807` for `abs`) are legitimate — they prevent undefined behavior in the implementation. Loop iteration caps for ESBMC are not.

## Pull Requests

Always merge PRs via a merge commit (`gh pr merge --merge`). Do not squash or rebase-merge — preserving the individual commit history is required.

## Vow Compiler

When implementing changes across Vow compilers, always modify BOTH the Rust compiler and the self-hosted compiler in the same session. Run the full test suite (`cargo test` and self-hosted tests) after changes to both.

## Bootstrap Commands (Rust Stage 0)

These Rust workspace commands build the stage 0 bootstrap compiler only. For day-to-day development, use `build/vowc` (see below).

```bash
cargo build --all          # build all crates
cargo test --all           # run all tests
cargo test -p vow-syntax   # run tests for a single crate
cargo test lexer::tests::lex_keywords  # run a single test by path
cargo clippy --all -- -D warnings      # lint (CI enforces zero warnings)
cargo fmt --all            # format all code
```

## Day-to-Day Usage (Self-Hosted Compiler)

`build/vowc` is the primary compiler for all Vow development. It is a self-hosted, verified fixed-point binary produced by `scripts/bootstrap.sh`.

```bash
build/vowc build examples/divide.vow                   # compile + verify (default)
build/vowc build --no-verify examples/divide.vow       # compile, skip verification
build/vowc build --mode debug examples/divide.vow      # compile with runtime vow checks
build/vowc verify examples/divide.vow                  # verify contracts only (no executable)

# Help
build/vowc --help                                      # JSON capability description (for agents)
build/vowc --help --human                              # human-readable capability description
```

Debug mode is required to see runtime `VowViolation` output. Release omits all vow checks.
`vowc build` verifies by default; use `--no-verify` to skip ESBMC verification.
`vowc verify` runs only the frontend + verification pipeline (no codegen, no executable output).

## Bootstrap (Rust Compiler)

To get `build/vowc` in the first place, bootstrap from the Rust compiler:

```bash
scripts/bootstrap.sh                      # full bootstrap with verification
scripts/bootstrap.sh --skip-cargo         # skip cargo build if already built
```

This builds `./target/release/vow` (stage 0), then uses it to compile and verify the self-hosted compiler, producing `build/vowc`. The Rust compiler (`./target/release/vow`) is only needed for this bootstrap step.

## Canonical Source of Truth

`docs/spec/` contains the authoritative specification for the Vow language and CLI:

- **`index.md`** — overview and reference table
- **`grammar.md`** — complete grammar: types, operators, control flow, structs, enums, match, modules, methods, effects, builtins
- **`cli.md`** — CLI commands, flags, output JSON schema, exit codes, trace/error formats
- **`contracts.md`** — vow blocks, requires/ensures/invariant, blame semantics
- **`errors.md`** — diagnostic error codes and their meanings
- **`examples.md`** — worked examples

**Any change to Vow syntax, semantics, types, builtins, operators, effects, or CLI flags MUST include a corresponding update to the relevant `docs/spec/*.md` file.** The spec files are the spec — compiler code implements them, not the other way around. If you add a type, update `grammar.md`. If you change a builtin signature, update `grammar.md`. If you add a CLI flag, update `cli.md`. If you change contract semantics, update `contracts.md`.

The Claude Code skill document is embedded in the compiler and generated on demand:
```bash
build/vowc skill print                            # print skill to stdout
build/vowc skill install                          # install to .claude/commands/vow-toolchain.md
```

After updating a spec file, regenerate `--help` / embedded skill and rebuild:
```bash
uv run python scripts/generate_help.py          # regenerate --help and skill in both compilers
cargo build --release -p vow                     # rebuild Rust compiler
scripts/bootstrap.sh --skip-cargo                 # rebuild build/vowc
```

The staleness detector `scripts/check_help_coverage.py` (run in `full_test.sh`) will catch drift between `grammar.md` and `--help`.

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
- `ir.vow`, `ir_printer.vow`, `lower.vow` — IR lowering and printing (Wave 4)
- `clif.vow` — Cranelift backend via FFI shims (`vow-clif-shim` crate)
- `main.vow` — driver with subcommands (`build`, `verify`), flags, structured `--help`

### Building and running

```bash
build/vowc build --no-verify compiler/main.vow -o /tmp/vow_main  # compile self-hosted compiler
/tmp/vow_main compiler/lexer.vow                             # type-check, print IR
/tmp/vow_main -o /tmp/lexer compiler/lexer.vow               # compile to native binary
```

The self-hosted compiler supports DFS module loading via `use` declarations.

### Bootstrap triple test

`scripts/concat_vow.sh` merges all compiler modules into a single file (stripping `module`/`use` headers), avoiding the need for module loading.

```bash
./scripts/concat_vow.sh clif > /tmp/compiler_clif.vow
./target/release/vow --no-verify /tmp/compiler_clif.vow -o /tmp/compiler_a  # Stage 0: Rust → Binary A
/tmp/compiler_a -o /tmp/compiler_b /tmp/compiler_clif.vow                   # Stage 1: A → B
/tmp/compiler_b -o /tmp/compiler_c /tmp/compiler_clif.vow                   # Stage 2: B → C
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

## Mutation Testing (`tools/vow-mutants/`)

`tools/vow-mutants/` is a `cargo-mutants`-style mutation testing tool, dogfooded as a self-hosted Vow program. It mutates `compiler/*.vow` (or any `--root` directory), runs a tiered oracle (`scripts/bootstrap.sh --skip-cargo` then `scripts/full_test.sh`), and writes structured JSON output to `mutants.out/` (`mutants.json`, `outcomes.json`, per-status `.txt` lists, plus `diff/<id>.diff` and `logs/<id>.log` per mutant). Stdout carries only a one-line summary.

```bash
build/vowc build --no-verify tools/vow-mutants/main.vow -o build/vow-mutants
build/vow-mutants list                                                # enumerate sites only
build/vow-mutants run --shard 0/8 --tier2-budget-secs 9000            # writes ./mutants.out/
build/vow-mutants run --shard 0/8 --output-dir my-results             # custom output directory
```

Mutation kinds: `op-flip` (binary operators), `const-flip` (`0`/`1`, `true`/`false`), `body-replace` (function bodies → default value for return type), `contract-weaken` (`requires`/`ensures`/`invariant` clauses → `true`).

Skip-list: `// GENERATE:<NAME>:START`/`:END` blocks and `extern "C" { ... }` blocks are excluded. `test_*.vow` files are filtered before scanning.

**Local-only.** Mutation testing is not wired into CI — a full Tier-2 sweep across `compiler/*.vow` is multi-hour wall-clock and would burn through GitHub Actions budget on every nightly. Run it on the developer machine on whatever cadence suits the project (e.g., before tagging a release, or after a substantial compiler change). To split the work, shard explicitly with `--shard 0/8` etc. and run shards sequentially over multiple sessions; the determinism guarantee means the union of `mutants.out/` across shards is well-defined.

When a `missed.txt` entry appears, the actionable response is to either (a) write a test that catches the mutation, or (b) file an issue documenting why the mutation is equivalent and out of scope. See `tools/vow-mutants/README.md` for the full output schema and known limitations.

## Vericoding Benchmark Suite

`benchmarks/` contains 40 verification benchmarks (15 Easy, 15 Medium, 10 Hard; 4 Hard are Stretch).
Each benchmark has: `spec.md`, `skeleton.vow`, `reference.vow`, `meta.toml`.
`benchmarks/manifest.toml` lists all benchmarks. 36 non-Stretch references pass `vow verify`.

### Benchmark Runner (`bench/`)

A Python CLI tool (managed by `uv`) that runs frontier LLMs against the benchmark suite.

```bash
# From repo root (requires uv):
uv run --project bench bench/run.py validate-references                             # verify all reference.vow files
uv run --project bench bench/run.py run --model claude-sonnet-4-20250514 --benchmark E01  # single benchmark
uv run --project bench bench/run.py run --model claude-sonnet-4-20250514                  # full suite
uv run --project bench bench/run.py run --model claude-sonnet-4-20250514 --resume         # resume partial run
uv run --project bench bench/run.py report --run-id <id>                            # generate comparison report
uv run --project bench bench/run.py report                                          # report on most recent run

# Or from bench/:
cd bench && uv run python run.py run --model claude-sonnet-4-20250514
```

**Architecture:** Direct API calls (Anthropic/OpenAI SDKs), not agent tool use. Each benchmark is a single conversation: system prompt (skill docs ~35KB) + spec + skeleton → LLM returns Vow code → `vow verify` → CEGIS loop if needed. Temperature 0.0 for reproducibility.

**Files:**
- `bench/run.py` — CLI entry point (`run`, `report`, `validate-references`)
- `bench/runner.py` — core CEGIS loop per benchmark
- `bench/llm.py` — LLM provider abstraction (Anthropic, OpenAI)
- `bench/verifier.py` — `vow verify` subprocess wrapper
- `bench/manifest.py` — manifest + meta.toml loader
- `bench/prompts.py` — system prompt (skill docs) + user prompt templates
- `bench/report.py` — results → markdown comparison report
- `bench/config.toml` — model configurations
- `bench/results/` — gitignored output directory

**Environment variables:** `ANTHROPIC_API_KEY` for Claude models, `OPENAI_API_KEY` for OpenAI models.
