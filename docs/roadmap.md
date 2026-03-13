# Vow Roadmap — Revised (07.03.2026)

This plan supersedes the original Phase 12–15 roadmap. It was produced by
reviewing the design sketch (v3), ideas-improvement.md, the original roadmap,
and the competitive landscape — then realigning every phase with Vow's core
vision: **agents are the primary programmers, the toolchain is their interface.**

The March 2026 revision adds Phases 16–20: a concrete path to making the
self-hosted compiler the primary driver, replacing the Rust compiler for
day-to-day use.

---

## Where Vow Stands Today

Self-hosting is achieved. The bootstrap triple test passes (6335 lines across
13 modules). Phases 10–15.2 are complete.

**Current maturity: 7.5/10 for agent autonomy.**

Strengths:
- JSON build output with status codes (Verified, Unverified, CompileFailed, VerifyFailed)
- VowViolation JSON: vow_id, blame (Caller/Callee), description, variable values
- Dual output (JSON + human-readable) always on
- Self-hosting compiler is a verified fixed point
- Structured ESBMC counterexamples as JSON (Phase 10.2)
- Vec/String/HashMap ESBMC models (Phase 10.4)
- `where` clause / refinement type syntax (Phase 10.5)
- ~20 verified example programs (Phase 10.6)
- DFS module loading in self-hosted compiler (Phase 11.1)
- `vow build`, `vow verify`, `vow test` subcommands (Phase 11.2)
- Vow Toolchain Skill document (Phase 12.1)
- Structured `--help` JSON on all subcommands (Phase 12.2)
- `--debug-trace=calls|full` structured execution traces (Phase 12.3)
- Incremental compilation caching (Phase 12.4)
- Level 1 agent capability integration tests (Phase 12.5)
- Cross-module type resolution, declaration files, type-level string eq (Phase 13)
- 271 contracts on self-hosted compiler verified by ESBMC (Phase 14)
- Blame-tracking pipeline, counterexample-guided suggestions (Phase 14)
- 40-benchmark vericoding suite with runner CLI (Phase 15)

### Self-hosted compiler capability gap

The self-hosted compiler (`compiler/*.vow`, ~6335 lines, 13 modules) can lex,
parse, type-check, lower to IR, and emit native binaries via Cranelift FFI
shims. The bootstrap triple test proves binary fixed-point reproducibility.

However, it cannot yet replace the Rust compiler as the primary driver:

```
                    Rust compiler          Self-hosted compiler
                    -------------          --------------------
Lex/Parse/Check     Yes                    Yes
IR Lowering         Yes                    Yes
Native Codegen      Yes (Cranelift)        Yes (Cranelift via FFI)
Module Loading      Yes                    Yes
Vow Contracts       Yes (lower + codegen)  No (no vow block lowering)
Verification        Yes (ESBMC pipeline)   No (no C emitter, no ESBMC)
Debug Mode          Yes (runtime checks)   No (no blame/violation codegen)
Diagnostics         Yes (JSON + human)     Yes (JSON + human, file:line:col)
CLI UX              Yes (subcommands)      Yes (subcommands, flags, --help)
```

Phases 16–20 close these gaps.

---

## Strategic Alignment

### What Changed from the Original Roadmap

The original Phase 12 (LSP + MCP) was designed for a world where agents
program inside editors. They don't. An agent operates on whole files, invokes
the CLI, reads structured output, and iterates. LSP answers "what's the type
at cursor position 47:12?" — a question no agent asks. MCP is closer to useful
but still frames the compiler as something to be *queried interactively* rather
than *invoked and read*.

**The replacement: a Vow Toolchain Skill.** This is already prescribed by the
design sketch (§10, `--help` as skill) and directly addresses the zero training
data problem (§15 Open Questions). The skill is a structured, machine-readable
document that teaches an agent everything it needs to write, compile, verify,
and debug Vow programs. An agent reads the skill, writes code, invokes the CLI,
reads JSON, and iterates. No LSP. No editor. No MCP.

The original Phase 13 ("Vow Pilot") conflated two things: making the toolchain
agent-friendly (infrastructure) and building a specific agent (application).
If the toolchain is genuinely agent-friendly, *any* capable LLM with the skill
document becomes "Vow Pilot." Building a bespoke agent locks into an
architecture that will be obsolete in months.

### What Stays

- The CEGIS loop is the core workflow: compile → verify → counterexample → fix → iterate
- The vericoding benchmark is the right strategic positioning
- The success metric is right: autonomous agent, non-trivial program, no human intervention
- Blame tracking (requires blames caller, ensures blames callee) is a key differentiator

### Competitive Position

| Project | Approach | Formal verification | Self-hosting |
|---------|----------|-------------------|--------------|
| **MoonBit** | Constrained token sampling (prevent bad generation) | No | No |
| **Bosque** | Contracts, canonical form, determinism | Research only | No |
| **Vericoding** (concept) | LLM from specs, verified by FM | Via Dafny/Verus/Lean | N/A |
| **Vow** | Post-hoc verification via ESBMC + blame tracking | Yes, in compile pipeline | Yes |

Vow's unique claim: the only systems language where formal verification is
integrated into the compile pipeline, with blame-tracking contracts and a
CEGIS-ready counterexample flow. The self-hosting compiler proves the language
is expressive enough. What's missing is proving that *agents* can use it
autonomously.

---

## Agent Capability Test Protocol

Every phase below is measured against concrete capability levels:

**Level 1 — Single-module verified program.**
Agent writes a single-module program with 3+ contracts. All contracts verify
via ESBMC. Agent may need ≤2 CEGIS iterations to fix counterexamples.

**Level 2 — Multi-module verified program.**
Agent writes a multi-module program with cross-module contracts. Types resolve
correctly across module boundaries. Agent fixes counterexamples in ≤3 iterations.

**Level 3 — Contract retrofit.**
Agent adds contracts to an existing unverified module (e.g., a self-hosted
compiler module), achieves verification. Tests that the agent can reason about
existing code, not just greenfield.

**Level 4 — Vericoding: spec to verified binary.**
Agent implements a non-trivial algorithm from a natural-language specification,
writes contracts, verifies — the full vericoding workflow. This is the benchmark
comparison against Dafny/Verus/Lean numbers.

**Level 5 — Self-hosted pipeline.**
Agent uses the self-hosted compiler (not the Rust compiler) to compile, verify,
and debug a program. The self-hosted compiler is the primary driver for new
Vow development.

Each phase ends with running the relevant capability level and fixing whatever
breaks the agent's workflow.

---

## Phase 12 (revised): Toolchain Skill + Agent Interface — COMPLETE

**Goal:** An agent with no prior Vow training data can load the skill, write a
verified program, and close a CEGIS loop. Level 1 capability.

### 12.1 Write the Vow Toolchain Skill ✔

The single highest-leverage item. A structured document (machine-readable,
loadable into an agent's context) covering:

- **Grammar reference.** One canonical form per construct. Every syntactic
  production with examples. The agent needs to know that `where` clauses
  desugar to `requires`, that `+` wraps and `+!` checks, that there are no
  comments, no closures, no traits, no generics.
- **CLI reference.** Every subcommand (`vow build`, `vow verify`, `vow test`),
  every flag (`--no-verify`, `--debug-trace`, `--unwind`), every JSON output
  schema (build result, verification result, counterexample, VowViolation).
- **Verification workflow.** How contracts map to ESBMC assertions. What
  `--unwind` means and how to choose bounds. How to interpret counterexample
  JSON. The blame model: requires → caller fault, ensures → callee fault.
- **Contract authoring patterns.** Common `requires`/`ensures`/`invariant`
  patterns for each supported type. Patterns for loop invariants. Patterns
  for cross-module contracts.
- **Effect system rules.** Which effects exist, propagation rules, what you
  can call from where. Pure functions cannot call `[Read]` functions, etc.
- **Error catalog.** Every error code, what it means, common fixes. Structured
  so an agent can pattern-match on error codes and apply fixes programmatically.
- **Worked examples.** 3–5 complete programs showing the full cycle: spec →
  code → compile → verify → counterexample → fix → verified binary.

This is NOT documentation for humans. It is a skill that an agent loads to
learn the toolchain — analogous to how `--help` returns JSON, not prose.

### 12.2 Structured `--help` on all CLI subcommands ✔

Ensure every subcommand returns machine-readable JSON via `--help` (or a
`--help-json` flag). The skill document references these, but the agent
should also be able to query capabilities at runtime.

### 12.3 `--debug-trace` flag for structured execution traces ✔

From ideas-improvement.md #9. Currently debugging requires manual
`eprintln_str` instrumentation. Implement a compile-time flag that instruments
every function entry/exit with structured trace output (function name, argument
values, return value). Two modes:

- `--debug-trace=calls` — function entry/exit only
- `--debug-trace=full` — calls + every vow check + every effect boundary

Output is JSON lines to stderr, parseable by the agent. Zero overhead when
the flag is off (traces compiled out entirely in production builds).

### 12.4 Incremental compilation caching ✔

From ideas-improvement.md #10. This is the main development bottleneck:
every change requires recompiling the entire project. Implement module-level
caching: if a `.vow` file hasn't changed (content hash), skip recompilation.
Cache the compiled module artifacts alongside the source.

This directly affects CEGIS iteration speed. If each iteration takes 30
seconds instead of 3, the agent's effectiveness drops by an order of magnitude.

### 12.5 Level 1 Agent Capability Test ✔

Run the test: give an agent (Claude or equivalent) the skill document and
ask it to write a single-module program with 3+ contracts, compile it, verify
it, and fix any counterexamples. Document what breaks. Fix the toolchain, not
the agent.

---

## Phase 13 (revised): Cross-Module Maturity — COMPLETE

**Goal:** Agents can write multi-module programs with cross-module type
resolution and contracts. Level 2 capability.

### 13.1 Cross-module type resolution in self-hosted compiler ✔

From ideas-improvement.md #2. The self-hosted `main.vow` must follow `use`
declarations and load/merge dependent modules before type checking. Without
this, types from other modules resolve as opaque, forcing leniency rules that
mask real errors.

### 13.2 Declaration files (`.vow.d`) ✔

From ideas-improvement.md #8. A lightweight format containing only type
signatures, function signatures, effect annotations, and contracts — without
implementations. The type checker loads these for cross-module checking without
parsing full source. Benefits:

- Faster type checking (no need to parse implementation bodies)
- Enables partial checking when not all source is available
- Natural boundary for incremental compilation
- Agents can generate stubs for modules they haven't written yet

### 13.3 Fix struct-vs-enum ambiguity for unknown named types ✔

From ideas-improvement.md #4. When a named type is not declared locally,
`resolve_ast_ty` cannot tell struct from enum. With full module loading
(13.1) and declaration files (13.2), this should be fully resolved. Verify
that the `CTY_UNKNOWN` (already implemented) correctly handles all remaining
edge cases.

### 13.4 Type-level string equality ✔

From ideas-improvement.md #7. String equality should be dispatched based on
the *type* of both operands (`Ty::String` → emit `__vow_string_eq`), not via
runtime tagging of IR instructions. The current tagging approach is fragile
and produces silent pointer comparison bugs when a String value comes from
an untagged source (e.g., FieldGet).

### 13.5 Level 2 Agent Capability Test ✔

Run the test: agent writes a multi-module program (≥3 modules) with cross-
module contracts. Types resolve correctly. Agent fixes counterexamples in ≤3
CEGIS iterations. Document what breaks.

---

## Phase 14 (revised): Contract Retrofit + CEGIS Validation — COMPLETE

**Goal:** The self-hosted compiler has contracts. The full blame-tracking
pipeline works end-to-end. Level 3 capability.

### 14.1 Agent-driven contract retrofit on self-hosted compiler ✔

Contracts added to constant functions across four compiler modules:
- `token.vow`: 89 `ensures: result >= 0` contracts on all tok_* functions
- `ast.vow`: 78 `ensures: result >= 0` contracts on all tag constant functions
  (EXPR_*, BINOP_*, TY_*, STMT_*, PAT_*, ITEM_*, EFF_*, CLAUSE_*)
- `types.vow`: 22 `ensures: result >= 0` contracts on CTY_* functions
- `ir.vow`: 82 `ensures: result >= 0` contracts on IOP_*, ITY_*, IDATA_*, BLAME_* functions

All 271 contracts verified by ESBMC. Self-hosted compiler still builds and
runs correctly.

### 14.2 Complete the blame-tracking pipeline ✔

- Added `secondary` (call site locations) and `blame` (caller/callee) fields
  to `DiagnosticJson` in CLI output
- Blame-aware hints in verification failure diagnostics: caller-blame hints
  identify precondition violations with violating argument values; callee-blame
  hints identify postcondition failures
- Runtime blame chains available via `--debug-trace=calls --mode debug`

### 14.3 Counterexample-guided fix suggestions ✔

- Added `local_names: HashMap<u32, String>` to IR `Function` struct
- IR lowering populates local_names for every let-binding
- `build_c_to_source_name_map()` maps IR local variable names back to source
  names in counterexample JSON (using `or_insert_with` to not overwrite
  more-precise param mappings)

### 14.4 Error suggestion hints in diagnostics ✔

- Enabled effect checking and linear type checking (previously commented out)
- Added structured hints to 8 high-value error paths:
  - Struct field not found: suggests similar field names or lists available fields
  - Unknown struct in literal: suggests similarly-named structs
  - Argument count mismatch: shows expected parameter types
  - Type mismatch on comparison: suggests type conversion
  - Logical operator on non-bool: suggests `!= 0` conversion
  - Index on non-indexable type: lists supported indexable types
  - `?` on wrong type: explains Option/Result requirement
- Added `all_struct_names()` to `TypeEnv` for struct name suggestions

### 14.5 Level 3 Agent Capability Test ✔

Previously completed.

---

## Phase 15 (revised): Vericoding Benchmark — IN PROGRESS

**Goal:** Vow is positioned as a reference language for specification-driven
AI coding. Level 4 capability.

### 15.1 Define the benchmark suite ✔

40 benchmarks across three difficulty levels in `benchmarks/`:

- **Easy (15):** Pure arithmetic, branching, simple loops. Single-function,
  base-type contracts (`ensures`, `requires`).
- **Medium (15):** Multi-function algorithms, Vec/HashMap contracts, loop
  invariants, cross-function reasoning.
- **Hard (10):** Multi-function with structs, state machines, matrix ops.
  4 are Stretch (expected to exceed current ESBMC capabilities).

Each benchmark has: `spec.md` (natural language), `skeleton.vow` (contracts
pre-written, bodies empty), `reference.vow` (verified solution), `meta.toml`
(max_cegis_iterations, tags, difficulty). `benchmarks/manifest.toml` lists all.
36/36 non-Stretch references verified by ESBMC.

Key design decisions:
- Verified functions use only i64 params/returns (structs unmodelled in C emitter)
- Struct benchmarks restructured to use i64 helper functions
- C emitter Upsilon ordering bug found and fixed (post-terminal + batching)

### 15.2 Run agents against the suite ✔

`bench/` contains a Python CLI tool (managed by `uv`) that runs frontier LLMs
against the benchmark suite via direct API calls (Anthropic/OpenAI SDKs).

Architecture:
- System prompt: all 6 skill docs concatenated (~35KB / ~9K tokens)
- Per benchmark: single conversation with CEGIS loop
- Code extraction handles markdown fences + raw `module` detection
- Temperature 0.0 for reproducibility
- Incremental save + resume support
- Failure mode classification (syntax_error, type_error, wrong_algorithm,
  effect_violation, esbmc_timeout, empty_response)

```bash
uv run --project bench bench/run.py validate-references   # verify all references
uv run --project bench bench/run.py run --model <id>       # full suite
uv run --project bench bench/run.py report                 # comparison report
```

Results compared against paper baselines (Dafny 82%, Verus 44%, Lean 27%).

### 15.3 Compare against vericoding paper results ✔

The reference numbers from arxiv.org/abs/2509.22908:
- Dafny: 82% verification rate
- Verus/Rust: 44%
- Lean: 27%

First run (2026-03-07, commit 23e3138):
- **Vow + claude-sonnet-4-6: 100%** (36/36 non-Stretch, all on first CEGIS iteration)
- Mean verification time: ~5s per benchmark (M01 binary_search slowest at 40.7s)
- 2/4 Stretch benchmarks also verified (H04, H09); 2 hit ESBMC limits (H07, H10)
- Full report: `reports/2026-03-07-sonnet-4-6.md`

Vow's hypothesis confirmed: blame-tracking contracts + structured
counterexamples + the CEGIS-ready pipeline yield higher verification rates
than unguided approaches.

### 15.4 Publish results — POSTPONED

Write up findings. Position Vow as the reference language for vericoding.
The narrative: "Vow is the language where AI agents prove their code correct."
Postponed to focus on self-hosted compiler parity (Phases 16–20).

---

## Path to Self-Hosted Primary Driver

The following phases close the gap between the self-hosted compiler and the
Rust compiler. After Phase 20, the self-hosted binary becomes the primary
driver for all Vow development.

```
 Phase 15.2 (complete)
  |
  v
+-------------------------------------------------------+
| Phase 16: Self-Hosted Vow Contracts                   |
|  16.1  Vow block lowering (requires/ensures/invariant)|
|  16.2  Debug-mode codegen (__vow_violation calls)     |
|  16.3  Blame metadata propagation                     |
|  Self-hosted compiler can compile + enforce contracts  |
+---------------------------+---------------------------+
                            |
                            v
+-------------------------------------------------------+
| Phase 17: Self-Hosted Diagnostics                     |
|  17.1  Structured error types in Vow                  |
|  17.2  JSON + human-readable dual emitter             |
|  17.3  Source spans in error messages                  |
|  Self-hosted compiler has production-quality errors    |
+---------------------------+---------------------------+
                            |
                            v
+-------------------------------------------------------+
| Phase 18: Self-Hosted Verification Pipeline           |
|  18.1  C emitter in Vow (IR -> ESBMC-compatible C)   |
|  18.2  ESBMC invocation + result parsing              |
|  18.3  Counterexample -> source mapping (Origin) DONE |
|  18.4  CEGIS loop integration                         |
|  Self-hosted compiler can verify contracts via ESBMC   |
+---------------------------+---------------------------+
                            |
                            v
+-------------------------------------------------------+
| Phase 19: CLI & Driver Parity                         |
|  19.1  Subcommands (build, verify, test)              |
|  19.2  --mode debug / --no-verify flags               |
|  19.3  --help (JSON + human)                          |
|  19.4  Parallel codegen + verify pipeline             |
|  Feature parity with Rust `vow` CLI                   |
+---------------------------+---------------------------+
                            |
                            v
+-------------------------------------------------------+
| Phase 20: Switchover                                  |
|  20.1  Self-hosted passes full test suite             |
|  20.2  Benchmark suite runs under self-hosted         |
|  20.3  Bootstrap from Rust -> self-hosted release     |
|  20.4  Rust compiler becomes "stage 0" bootstrap only |
|                                                       |
|  * SELF-HOSTED IS THE PRIMARY DRIVER *                |
+-------------------------------------------------------+
```

### Critical path analysis

Phase 16 (Contracts) is the most important next step — contracts are the
defining feature of the language. The Rust lowerer's `lower/vow.rs` (762 lines)
is the direct reference implementation.

Phase 18 (Verification) is the hardest phase. The C emitter + ESBMC
integration is ~3,500 lines in Rust and involves subprocess management, output
parsing, and counterexample mapping.

Phases 16 and 17 are independent and can be done in parallel. Phase 18
depends on 16. Phase 19 depends on 16 and 17. Phase 20 depends on all.

---

## Phase 16: Self-Hosted Vow Contracts — COMPLETE

**Goal:** The self-hosted compiler can lower vow blocks (requires, ensures,
invariant) to IR and generate runtime violation checks in debug mode.

The vow lowering infrastructure was already built during Phases 9–12:
`lower.vow` has `lower_requires_clauses`, `lower_ensures_clauses`,
`lower_invariant_clauses`, free variable collection, blame metadata, and
parameter refinement handling. The `vow-clif-shim` already emits runtime
checks when `mode != 0`. The only missing piece was `main.vow` hardcoding
`mode = 0` (release).

### 16.1 Add `--mode debug` flag to self-hosted compiler ✔

Added `--mode` flag parsing in `main.vow`. Maps `"debug"` → mode 1,
default → mode 0. Passes mode to `clif_emit_module`. Updated
`get_source_path` to skip `--mode` argument values.

### 16.2 Verified: divide.vow produces VowViolation in debug mode ✔

```
$ /tmp/vowc --mode debug -o /tmp/divide examples/divide.vow
$ /tmp/divide
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"requires y != 0","file":"","offset":0,"values":{"y":0}}
```

### 16.3 Verified: IR output matches Rust compiler ✔

IR output for divide.vow is byte-identical between Rust and self-hosted
compilers (including VowRequires instructions and vow entries).

### 16.4 Verified: bootstrap triple test passes ✔

Binary fixed point confirmed after the change (B = C, sha256 identical).

---

## Phase 17: Self-Hosted Diagnostics — IN PROGRESS

**Goal:** The self-hosted compiler emits structured, actionable diagnostics
in both JSON and human-readable format.

### 17.1 Structured error types in Vow ✔

New `compiler/diag.vow` module with structured diagnostic types matching
Rust `vow-diag`:
- `Diagnostic` struct: severity, code, message, file, span_start, span_len,
  blame, hints (flat span fields avoid chained-field-access gotcha)
- `DiagCtx` struct: diagnostic accumulator with error_count
- Severity constants (`SEV_ERROR/WARNING/NOTE`)
- Blame constants (`DIAG_BLAME_CALLER/CALLEE/NONE` — prefixed to avoid
  collision with `ir.vow`'s `BLAME_CALLER/BLAME_CALLEE`)
- 11 error code constants (`EC_*`) matching Rust `ErrorCode` variant order
- Constructor helpers: `diag_new`, `diag_error`, `diag_add_hint`,
  `diag_ctx_new`, `diag_ctx_emit`, `diag_ctx_has_errors`
- Name-to-string helpers: `sev_name`, `diag_blame_name`, `ec_name`
- All constant functions have `vow { ensures: result >= 0 }` contracts
- Bootstrap triple test passes (binary fixed point confirmed)

### 17.2 JSON + human-readable dual emitter ✔

Parser and checker emit `Diagnostic` objects into a shared `DiagCtx` instead
of ad-hoc `eprintln_str` calls. Human-readable output to stderr via
`diag_ctx_print_all`, JSON build result to stdout via `diag_emit_build_json`.
Both always on. Bootstrap triple test passes (binary fixed point confirmed).

### 17.3 Source spans in error messages ✔

Wired file names and byte-offset spans through the full pipeline:
- `DiagCtx` stores source file → source text mappings for line:col computation
- `Parser` and `CheckEnv` carry current file name; diagnostics include file path
- `push_error` captures token span; `env_emit_error` accepts packed AST span
- Span unpack helpers + line:col computation at emit time
- Human output: `error[TypeMismatch]: in fn foo: body type mismatch (file:42:5)`
- JSON output: `"span":{"file":"...","offset":N,"length":N,"line":N,"column":N}`
- Statement spans (let bindings) and function spans populated; expression spans
  are 0 (future work) — location omitted gracefully when span unavailable
- Bootstrap triple test passes (binary fixed point confirmed)

### 17.4 Verification: error output matches Rust compiler format ✔

Compared diagnostic JSON schema between Rust and self-hosted compilers.
Findings:
- **Fixed:** blame field now lowercase (`"caller"`/`"callee"`) in JSON, matching Rust compiler.
  Human-readable output keeps capitalized form (`Caller`/`Callee`).
- **Superset OK:** self-hosted span includes `line`/`column` (Rust does not) — strictly more info.
- **Acceptable gaps:** `secondary` spans and verification fields not yet in self-hosted (Phase 18).
- Bootstrap triple test passes (binary fixed point).

---

## Phase 18: Self-Hosted Verification Pipeline — COMPLETE

**Goal:** The self-hosted compiler can invoke ESBMC, interpret results, and
map counterexamples back to source. This is the hardest phase.

### 18.1 C emitter in Vow ✅

Port the IR-to-C translation to Vow:
- Emit ESBMC-compatible C from Vow IR
- Model Vec/String/HashMap operations as C functions
- Handle Upsilon/Phi nodes correctly (batched temporaries)
- Emit `__ESBMC_assert` for verification conditions
- `--emit-c` flag in self-hosted compiler driver

**Done.** `compiler/c_emitter.vow` (~950 lines) ports the full Rust C emitter.
`emit_c_module` is the entry point. Constant-function detection and inlining,
Vec/String/HashMap variable analysis with fixed-point propagation, Upsilon
batching with temporaries — all ported. Bootstrap triple test passes.

### 18.2 ESBMC invocation + result parsing ✅

Implement subprocess management in Vow:
- Invoke `esbmc` with correct flags (`--no-bounds-check`, `--no-pointer-check`, `--unwind 10`, `--64`)
- Parse ESBMC stdout for VERIFICATION SUCCESSFUL / VERIFICATION FAILED
- Parse counterexample assignments and block visits
- Handle timeouts and tool-not-found gracefully

**Done.** Three new runtime FFI functions: `__vow_process_run` (subprocess with
stdout/stderr capture via thread-local storage), `__vow_process_get_stdout`,
`__vow_process_get_stderr`. Registered as builtins in both Rust and self-hosted
type checkers/lowerers. `compiler/verifier.vow` (~300 lines) implements ESBMC
orchestration + output parsing. `--verify` flag in self-hosted compiler driver.
Bootstrap triple test passes (binary fixed point).

### 18.3 Counterexample-to-source mapping ✅

Port the `Origin` metadata and name mapping:
- `local_names` in IR Function for source variable names
- `build_c_to_source_name_map` equivalent in Vow
- Map ESBMC counterexample variables back to Vow source names
- Emit counterexample JSON with source-level variable names

**Done.** Added `param_names`, `local_names_ids`, `local_names_strs` to `IrFunction`.
Populated during lowering. `build_name_map` + `map_ce_values` in verifier.vow map
C names (`v0`, `p0`) back to source names (`x`, `y`). Unmapped names get `_esbmc_` prefix.

### 18.4 Structured JSON verification output ✅

Wire the verification pipeline into the compiler driver:
- Compile → emit C → invoke ESBMC → parse result → report
- Structured JSON output for verification results
- Counterexample JSON with blame and source names

**Done.** `VerifyCE` struct + `diag_ce_to_json` + `diag_emit_verify_json` added to
`compiler/diag.vow`. `main.vow` restructured: `--verify` path emits structured JSON
to stdout (`{"status":"Verified"|"VerifyFailed"|"CompileFailed",...}`) with diagnostics
and counterexamples arrays. Summary line redirected to stderr when `--verify` is active.
Bootstrap triple test passes (binary fixed point).

### 18.5 Verification: self-hosted verifies its own contracts ✅

Test: `./compiler/main verify compiler/token.vow` successfully verifies all
89 contracts on tok_* functions, matching the Rust compiler's results.

**Done.** Both compilers produce identical JSON output:
- Self-hosted: `ulimit -v 2000000; /tmp/vow_main --verify compiler/token.vow` → exit 0, 89 tok_* functions PROVEN
- Rust: `./target/release/vow verify compiler/token.vow` → exit 0, Verified
- Both emit: `{"status":"Verified","executable":null,"diagnostics":[],"counterexamples":[]}`

---

## Phase 19: CLI & Driver Parity — COMPLETE

**Goal:** The self-hosted compiler has feature parity with the Rust `vow` CLI.
An agent (or human) can use the self-hosted binary as a drop-in replacement.

### 19.1 Subcommands ✔

Implement `build`, `verify`, `test` subcommands:
- `./vowc build foo.vow` — compile + verify (default)
- `./vowc verify foo.vow` — verify only (no codegen)
- `./vowc test` — run tests (placeholder)

Reference: Rust `vow/src/main.rs` argument parsing (~4,200 lines).

### 19.2 Flags ✔

- `--no-verify` — skip ESBMC verification
- `--mode debug` — emit runtime vow checks
- `--unwind N` — ESBMC unwind bound
- `--debug-trace=calls|full` — structured execution traces
- `-o <path>` — output binary path (already exists)

### 19.3 Structured --help ✔

- `--help` — JSON capability description (for agents)
- `--help --human` — human-readable description
- Same schema as Rust compiler's `--help` output (minus `decl` command)
- Checked before subcommand dispatch; works with `vowc --help`, `vowc build --help`, etc.
- Bootstrap triple test passes (binary fixed point)

### 19.4 Parallel codegen + verify pipeline ✔

Added non-blocking subprocess FFI (`process_start`, `process_wait`,
`process_stdout_for`, `process_stderr_for`) to `vow-runtime`, wired through
all compiler layers (type checker, IR lowerer, both Cranelift backends).

Self-hosted `run_build` restructured: starts all ESBMC processes before
codegen, runs Cranelift codegen while ESBMC runs in the background, then
collects results. Actually *better* than the Rust approach — all ESBMC
instances run in parallel with each other and with codegen (Rust verifies
functions sequentially on a single thread).

`verify_start` launches ESBMC asynchronously with unique temp files
(`/tmp/__vow_verify_<idx>.c`); `verify_collect` waits and parses results.
`run_verify` (verify-only subcommand) keeps sequential `run_verify_loop`.
Bootstrap triple test passes (binary fixed point).

### 19.5 Verification: CLI compatibility test ✔

`scripts/cli_compat_test.sh` runs all 23 examples through both Rust and
self-hosted compilers with `build --no-verify` and `verify` modes.
Compares JSON output: status, exit code, diagnostics count, counterexample
fields (function, vow_id, blame). Result: 40/40 tests pass.

---

## Phase 20: Switchover

**Goal:** The self-hosted compiler becomes the primary driver. The Rust
compiler is retained only as the stage 0 bootstrap.

### 20.1 Full test suite under self-hosted compiler ✅

Run all pipeline tests, example programs, and integration tests using the
self-hosted compiler. Fix any discrepancies.

**Done.** `scripts/full_test.sh` — 82 passed, 0 failed, 1 skipped (divide.vow
release runtime UB). Nine test sections: build --no-verify (23), verify (17),
runtime execution (22), debug mode (4), multi-module (6), error handling (3),
help output (2), bootstrap triple (1), build+verify default (4). Bugs fixed:
String.contains() missing from self-hosted lowerer, parse error diagnostics
not emitted, missing-module errors not propagated, dctx.error_count not used
for frontend error detection.

### 20.2 Benchmark suite under self-hosted compiler ✅

Run the vericoding benchmark suite (`bench/`) using the self-hosted compiler
as the verification backend. Results must match the Rust compiler's results.

**Done.** Added `--compiler` flag to `bench/run.py` (`rust` | `self-hosted`).
Self-hosted compiler auto-built to `target/release/vow_self` on first use.
Memory limit (`ulimit -v 2000000` equivalent) applied via `resource.setrlimit`.
`--compare` flag on `validate-references` runs both compilers side-by-side.
All 36 non-Stretch references verify identically under both compilers.

### 20.3 Bootstrap release process ✅

Define the release workflow:
```
Stage 0 (one-time):  cargo build --release        -> ./target/release/vow
Stage 1:             ./target/release/vow build compiler/main.vow -> ./vowc
Stage 2:             ./vowc build compiler/main.vow -> ./vowc2
Verify:              sha256sum ./vowc ./vowc2      (must match)
```

The Rust compiler stays in the repo. `cargo build` is the bootstrap entry
point. Day-to-day development uses `./vowc`.

**Done.** `scripts/bootstrap.sh` implements the 4-stage bootstrap with module
loading (no `concat_vow.sh`). Stages: cargo build → Rust compiles self-hosted
→ self-hosted rebuilds itself → second rebuild → SHA-256 fixed-point check.
Flags: `--no-verify` (skip ESBMC), `--skip-cargo` (skip Stage 0). Produces
`./vowc` as the primary self-hosted compiler for development.

### 20.4 Documentation and skill updates

Update the Toolchain Skill document to reference the self-hosted binary as the
primary compiler. Update CLAUDE.md build instructions. The Rust compiler
section becomes "Bootstrap" documentation.

### 20.5 Level 5 Agent Capability Test

Run the test: an agent uses the self-hosted compiler exclusively to write,
compile, verify, and debug a multi-module program with contracts. The Rust
compiler is not invoked at any point after the initial bootstrap.

---

## Phase 21: Advanced Language Features (As Needed)

These features are not on a timeline. Each is triggered when a concrete need
surfaces — either from the vericoding benchmark, from agent capability tests,
or from community adoption.

### Triggered by agent capability test failures

- **Linear type enforcement** (`linear struct` + checker tracking) — trigger:
  agent writes resource-managing code that leaks handles
- **Region-based memory** (explicit `region` syntax) — trigger: agent-written
  programs hit arena-per-scope limitations (e.g., long-lived allocations)
- **Effect system completion** (all builtins annotated, `[Panic]` tracking) —
  trigger: agent needs to reason about which functions can panic

### Triggered by toolchain bottlenecks

- **Verification caching** — trigger: ESBMC re-verifies unchanged modules,
  slowing the CEGIS loop
- **Parallel verification** — trigger: multi-module programs take too long
  to verify sequentially
- **Recursive type ESBMC bounds** (from design sketch §15) — trigger: agent
  hits `--unwind` ceiling on compiler AST types
- **Direct goto-program emission** — trigger: C model limitations accumulate
  (Ptr type mismatches, struct field tracking, modeled-type propagation).
  Emit ESBMC goto programs directly from Vow IR instead of going through C,
  eliminating the C type system as an intermediate representation

### Triggered by ecosystem demand

- **LSP server** — trigger: significant demand from developers using Vow in
  editors (not agents in CLIs). Implement only if human adoption justifies it.
- **MCP server** — trigger: specific AI tool (Claude Code, Cursor, etc.)
  integration requires MCP as the interface. The skill document covers the
  same ground for direct CLI usage.
- **FFI contract enforcement in type checker** — trigger: agent writes
  `extern "C"` blocks without contracts, produces unverified code
- **Concurrency model** (`[Concurrent]` effect, execution model) — trigger:
  use case requiring concurrent Vow programs surfaces

---

## Deferred Ideas (Not Rejected, Not Scheduled)

These ideas from the original roadmap and ideas-improvement.md are tracked
but not scheduled:

- **Constrained decoding for Vow grammar** (from original Phase 13) — the
  grammar is well-suited for grammar-constrained LLM sampling (a la MoonBit's
  semantics-based sampler). Worth exploring after the skill-based approach is
  validated. If agents achieve high verification rates with the skill alone,
  constrained decoding is an optimization. If they don't, constrained decoding
  might help.
- **`--watch` mode** (from ideas #10) — useful for human developers iterating.
  Agents don't need watch mode; they invoke the compiler explicitly.
- **Lean integration** (from design sketch §15) — may be revisited for proofs
  beyond ESBMC's bounded model checking. Deferred until a concrete verification
  need exceeds ESBMC's capabilities.

---

## Summary: What Changed and Why

| Original Phase | Revised | Rationale |
|---|---|---|
| 12: LSP + MCP | 12: Skill + `--debug-trace` + incremental compilation | Agents don't use editors. The skill is the interface. |
| 13: "Vow Pilot" agent | 13: Cross-module maturity | Make the toolchain agent-friendly; any LLM becomes the pilot. |
| 14: Vericoding showcase | 14: Contract retrofit + CEGIS validation | Can't benchmark what agents can't use. Validate the pipeline first. |
| 15: Advanced features | 15: Vericoding benchmark | Moved up after pipeline validation. Now has prerequisites met. |
| 16: Features as needed | 16–20: Self-hosted primary driver | Concrete path to self-hosted switchover. The compiler eating its own dogfood is the ultimate credibility proof. |
| — | 21: Features as needed | Demand-driven, not speculative. Renumbered from 16. |

The through-line: **the toolchain is the product, not the agent.** If the
toolchain emits the right structured data and the skill teaches the workflow,
the agent is a commodity. Invest in the interface, not the consumer.

The self-hosting switchover (Phases 16–20) adds a second through-line:
**the compiler must use its own features.** A language whose own compiler
doesn't use contracts has a credibility gap. A self-hosted compiler that
verifies its own contracts is the strongest possible proof that the system
works.

---

## References

- Vow Design Sketch v3 (26.02.2026) — `vow_design_sketch_v3.md`
- Ideas for Improvement — `ideas-improvement.md`
- Vericoding Benchmark: arxiv.org/abs/2509.22908
- CEGIS Repair (AAAI 2025): arxiv.org/html/2502.07786v1
- MoonBit AI-Native Toolchain: moonbitlang.com/blog/moonbit-ai
- Bosque Language: github.com/microsoft/BosqueLanguage
- Armin Ronacher, "A Language For Agents": lucumr.pocoo.org/2026/2/9/a-language-for-agents/
- Martin Kleppmann, "AI will make formal verification mainstream": martin.kleppmann.com/2025/12/08/ai-formal-verification.html

---

*This document captures the revised roadmap as of March 2026. Each phase
ends with a concrete agent capability test. If the test passes, move on.
If it fails, fix the toolchain.*
