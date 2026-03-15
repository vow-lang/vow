# Vow Roadmap — Revised (15.03.2026)

This revision replaces the March 7 roadmap. Phases 10–20 are complete. The
self-hosted compiler (`./vowc`) is the primary driver. This document focuses
exclusively on what comes next, ordered by priority and impact.

---

## Where Vow Stands Today

**All foundational phases (10–20) are complete.** The self-hosted compiler is a
verified fixed point with full feature parity to the Rust bootstrap compiler.

Achieved:
- Self-hosted compiler: 13 modules, ~9000 lines of Vow, binary fixed point
- 271 contracts on compiler modules verified by ESBMC
- Full verification pipeline: compile → emit C → ESBMC → counterexample → blame
- Parallel codegen + verification (all ESBMC instances concurrent with codegen)
- Structured JSON diagnostics with line:col source spans
- `build`, `verify` subcommands; `--mode debug`, `--no-verify`, `--unwind N` flags
- Vericoding benchmark: **100% (36/36)** with claude-sonnet-4-6 on Vow's own suite
- 89/89 tests passing, 40/40 CLI compatibility tests
- Toolchain Skill document, structured `--help`, `--debug-trace`, incremental compilation
- Bootstrap: `scripts/bootstrap.sh` produces `./vowc` from Rust stage 0

**Current maturity: 9/10 for agent autonomy on verified programs.**

**Strongest current claim:** Vow is an unusually strong agent-first,
bounded-verification language with an integrated compile/verify/CEGIS loop,
blame tracking, structured counterexamples, and a self-hosted verified compiler.
What it does **not** yet have is a fair apples-to-apples comparison on spec
strength against Dafny/Verus/Lean. The 36/36 result is on Vow's own benchmark
suite; the Vericoding paper's numbers are on cross-language translated tasks
with stronger specs.

Known limitations:
- Arena deallocation is a no-op (`__vow_arena_free` leaks; fine for short-lived programs)
- Expression-level source spans are unpopulated (function/statement spans work)
- 2/4 Stretch benchmarks hit ESBMC `--unwind` ceiling (H07, H10)
- `divide.vow` release build has UB (no runtime checks; debug mode works)
- Zero public visibility — benchmark results not yet published
- Spec expressiveness gap: ensures clauses cannot call user-defined functions or express quantifiers

---

## Competitive Landscape (March 2026)

### Direct comparators (benchmark peers for Phase 21)

| | **Vow** | **Dafny** | **Verus** | **Lean** |
|---|---|---|---|---|
| Primary goal | Agent correctness | General verification | Rust verification | Theorem proving |
| Verification style | Bounded MC (ESBMC) | SMT (Z3) | SMT (Z3) | Interactive proofs |
| Spec expressiveness | Bounded, no quantifiers* | Full first-order logic | Rust-embedded specs | Dependent types |
| Automation burden | Lowest (CEGIS loop, no ghost code) | Moderate (ghost code, lemmas) | Moderate (proof hints, triggers) | Highest (tactic proofs) |
| Agent ergonomics | Agent-first design | Not agent-targeted | Not agent-targeted | Not agent-targeted |
| Counterexample quality | Structured JSON + blame | Textual counterexamples | Limited | N/A (proof-based) |
| Self-hosted pipeline | Yes (verified fixed point) | No | No | Yes (partial) |

*Spec expressiveness improves after Phase 21.1 (spec function calls in ensures).

### Adjacent comparators (positioning context)

| | **MoonBit** | **Bosque** | **Dana** |
|---|---|---|---|
| Primary goal | AI-native tooling | Regularized reasoning | Agent orchestration |
| Verification | None (constrained sampling) | Research-stage verifier | None |
| Niche | IDE + constrained decoding | Canonical forms | Intent-driven agents |

The Dafny ecosystem is publishing prolifically (ATLAS, DafnyPro, BRIDGE — all
at POPL 2026). Vow's 100% result is on its own suite; fair comparison requires
the work in Phase 21.

Vow's unique differentiators that no competitor has:
- Blame tracking (caller vs callee) in verification failures
- Integrated verification pipeline (not a separate framework)
- Self-hosted compiler that verifies its own contracts
- Structured counterexample JSON with source-level variable names

---

## Agent Capability Test Protocol

Every phase is measured against concrete capability levels:

**Level 1 — Single-module verified program.** ✅ Passed (Phase 12.5)
**Level 2 — Multi-module verified program.** ✅ Passed (Phase 13.5)
**Level 3 — Contract retrofit.** ✅ Passed (Phase 14.5)
**Level 4 — Vericoding: spec to verified binary.** ✅ Passed (Phase 15.3)
**Level 5 — Self-hosted pipeline.** ✅ Passed (Phase 20.5)

**Level 6 — Real-world application.**
Agent uses Vow to implement a non-trivial application (not a compiler or
algorithm benchmark) from a specification. The application uses filesystem I/O,
string manipulation, and data structures. Contracts verify correctness
properties. This is the `ai-coding-lang-bench` target.

---

## Phase 21: Publishable Comparison

**Priority: CRITICAL — competitive window is closing.**

Phase 21 has two tracks that can publish independently:

- **Critical path (21.1 → 21.4 → 21.7):** fix the verification pipeline, run
  the Vericoding comparison with contract fidelity, publish the direct
  comparison. This is the minimum viable publication.
- **Parallel track (21.3 → 21.6 → 21.8):** build the standard library, run
  ai-coding-lang-bench, publish the dual-track update.

Verification caching (21.2) and example coverage (21.5) accelerate this work
but are not on either critical path.

### 21.1 Verification pipeline prerequisites

**Status: BLOCKED — these must land before retranslating benchmarks.**

Three pipeline limitations prevent fair comparison:

**Spec function calls in ensures clauses.** The C emitter currently replaces
non-constant function calls with `__VERIFIER_nondet()`, making them
meaningless. If the emitter instead emitted the actual function body into the
C model, ensures clauses could reference pure spec functions (e.g.,
`ensures: result == is_prime_spec(n)`). ESBMC would then verify by bounded
model checking — no quantifiers needed, no new syntax, just a verification
pipeline fix. This makes Vow contracts as strong as Dafny's for bounded
inputs.

**Nested Vec loops.** Nested loops with Vec access fail ESBMC verification
(had to drop 2SUM and 3SUM from the pilot). Requires investigation of the
C model generated for nested indexed access patterns.

**`let mut` inside loop bodies.** Variables declared inside loop bodies cause
ESBMC errors — currently variables must be declared outside all enclosing
loops. The C emitter needs to handle scoped declarations within loop bodies.

### 21.2 Verification caching (accelerator, not prerequisite)

Cache ESBMC results by function content hash. If a function hasn't changed,
skip re-verification. This directly speeds up the CEGIS loop — currently
every `vow build` re-verifies all functions even if only one changed.
Valuable for comparison runs at scale (162 tasks × multiple iterations), but
not required for correctness of the comparison itself. Can land at any point.

### 21.3 Standard library core subset

**Prerequisite for the real-world comparison track (21.6).**

These are new runtime builtins registered in the type checker and lowerer.
Scope for each: `vow-runtime` C implementations + type checker registration +
IR lowerer builtin dispatch.

**Filesystem builtins** (7 new `[IO]` functions):
- `fs_mkdir(path: String) -> i64`
- `fs_exists(path: String) -> i64`
- `fs_listdir(path: String) -> Vec<String>`
- `fs_remove(path: String) -> i64`
- `fs_remove_dir(path: String) -> i64`
- `fs_is_dir(path: String) -> i64`
- `fs_rename(old: String, new: String) -> i64`

**String builtins** (11 new functions):
- `string_substr(s: String, start: i64, len: i64) -> String`
- `string_split(s: String, sep: String) -> Vec<String>`
- `string_starts_with(s: String, prefix: String) -> i64`
- `string_ends_with(s: String, suffix: String) -> i64`
- `string_trim(s: String) -> String`
- `string_to_upper(s: String) -> String`
- `string_to_lower(s: String) -> String`
- `string_replace(s: String, old: String, new: String) -> String`
- `string_join(parts: Vec<String>, sep: String) -> String`
- `parse_i64(s: String) -> i64`
- `i64_to_string(n: i64) -> String`

**Bitwise operations:**
- `a ^ b` — bitwise XOR (new token kind + parser + IR opcode)
- `hex_encode(data: Vec<u8>) -> String`
- `hex_decode(s: String) -> Vec<u8>`

**Vec sorting:**
- `vec_sort(v: Vec<i64>) -> Vec<i64>` — returns sorted copy

**Time:**
- `time_unix() -> i64` — current Unix timestamp in seconds

### 21.4 Vericoding comparison with contract fidelity

**Status: BLOCKED on 21.1.**

The 10-task HumanEval pilot showed 10/10 verified with claude-sonnet-4-6
(mean 1.3 CEGIS iterations), but only 2/10 have Dafny-equivalent contracts.

**Contract fidelity classification.** Every translated task must be tagged:

- **Exact:** Vow contract matches the reference Dafny spec (same properties
  verified).
- **Partial:** same intent, weaker encoding (e.g., missing a quantified
  property).
- **Weak:** only bounds/shape/basic safety (trivial implementation could pass).

Publish pass rates separately for **Exact only** and **All translated tasks**.
This avoids inflated comparisons from tasks with weak contracts.

**Path to publishable comparison:**
1. Land 21.1 (spec functions, Vec loops, let mut fixes).
2. Re-translate the 10 HumanEval pilots with full-strength contracts.
3. Scale to the full 162 HumanEval-Dafny set from the Vericoding benchmark.
4. Run the same protocol (up to 5 CEGIS iterations) and compare against
   their published per-model results.

**Resources:**
- Vericoding benchmark: github.com/Beneficial-AI-Foundation/vericoding-benchmark
- Vericoding scripts: github.com/Beneficial-AI-Foundation/vericoding
- Pilot results: `bench/results/humaneval-pilot/`
- Pilot benchmarks: `benchmarks/humaneval/HE*`

### 21.5 Expand example coverage (not on critical path)

Improves skill document quality and adoption, but does not block either
comparison track. Can land at any point.

The `examples/` directory has significant feature gaps. Add examples
demonstrating:

- `match` expressions on enums (currently zero examples)
- `Option<T>` and `Result<T,E>` workflows
- `?` operator for error propagation
- Checked arithmetic operators (`+!`, `-!`, `*!`)
- Non-IO effects (`[Read]`, `[Write]`)
- `f32`/`f64` floating-point types

Each example should be a complete, runnable program that compiles and verifies.

### 21.6 Real-world comparison track (ai-coding-lang-bench)

**Status: BLOCKED on 21.3 (standard library).**

Participate in `ai-coding-lang-bench`, which measures how efficiently Claude
Code can implement a mini-git clone across programming languages. This provides
a second comparison axis beyond Vericoding: real-world application development,
not just algorithm verification.

**21.6a Benchmark harness integration.**
Fork the benchmark repository. Add Vow as a target language with build
scripts, skill docs, and test infrastructure.

**21.6b Pilot runs and iteration.**
Run Claude Code against the mini-git specification using `./vowc`. Identify
failure modes. Fix the toolchain, not the agent.

**21.6c Full benchmark execution.**
20-trial suite for statistical significance. Target: ≥38/40 pass rate with
competitive execution time (<90s).

**21.6d Verified variant.**
Run the benchmark with `vow build` (verification enabled). Demonstrate that
Vow can produce a verified mini-git implementation — something no other
benchmark language can do.

### 21.7 Publish direct comparison

**Status: BLOCKED on 21.1 + 21.4 only.**

Publish the Vericoding comparison as soon as the direct track is complete:

1. **Vericoding results** — pass rates with contract fidelity breakdown (Exact
   vs All), compared against Dafny/Verus/Lean per-model results from the
   Vericoding paper.
2. **Comparison matrix** — the table from the Competitive Landscape section,
   with empirical data from the Vericoding track.

This is the minimum viable publication. It does not require the standard
library, ai-coding-lang-bench, or any other parallel work.

Target: blog post + arxiv preprint.

### 21.8 Publish dual-track comparison update

**Status: BLOCKED on 21.6 (real-world track).**

Follow-up publication adding the real-world track:

1. **ai-coding-lang-bench results** — pass rate and execution time, with
   verified variant as a differentiator.
2. **Updated comparison matrix** — empirical data from both tracks.

This can be a second blog post or an updated preprint. It strengthens the
story but does not gate the initial publication.

---

## Phase 22: Language Ergonomics

**Priority: HIGH — these directly improve agent productivity and reduce
error rates. Ordered by implementation difficulty (easiest first).**

### 22.1 Named constants (`const` declarations) — GitHub #15 ✅

Top-level `const` declarations that fold at compile time. Both Rust and
self-hosted compilers support `const NAME: TYPE = LITERAL;`. The checker
validates literal values; the lowerer folds to `ConstI64`. Zero verifier
impact — verification of contracts using consts works unchanged.

### 22.2 Break and break-with-value — GitHub #13 ✅

`break` exits the current loop via `Jump` to the loop exit block. Both Rust
and self-hosted compilers support `break` inside `while` loops. The Rust
compiler also supports `break` inside `loop` expressions and `ExprKind::Loop`
IR lowering. The type checker validates break-outside-loop errors. Back-edge
emission is guarded by `is_terminated()` to handle break in loop bodies.

### 22.3 Iterator protocol / for-each loop — GitHub #10

`for`-each loop over `Vec<T>`:

```
for item in vec { ... }
for i, item in vec.enumerate() { ... }
```

Impact: eliminates 50+ manual while-loop index patterns across the 9000-line
self-hosted compiler. Removes an entire class of off-by-one errors and
forgotten `i = i + 1` infinite loops that agents produce at elevated rates.

Scope: lexer (`for`/`in` keywords) + parser + IR lowering (desugar to while
loop with bounds check) + verification (loop invariant synthesis for for-loops).

Note: the design sketch (§4.3) says "no `for` loops" because iterators require
traits. This `for`-each is a syntactic desugar to `while` with index — no
traits, no closures, no iterators. The IR is identical to the manual pattern.

---

## Phase 23: Toolchain Improvements

**Priority: MEDIUM — these improve the development experience but don't
block current workflows.**

### 23.1 Expression-level source spans

Populate expression spans in the self-hosted compiler's parser. Currently
only function and statement spans are wired through. Expression spans would
give more precise error locations (e.g., "type mismatch in `a + b`" instead
of "type mismatch in fn foo").

### 23.2 Property-based tests for compiler pipeline (PR #27)

Add proptest-based roundtrip, determinism, and robustness tests across
vow-syntax, vow-types, and vow-codegen. Catches edge cases that handwritten
tests miss.

### 23.3 IO error diagnostic code (PR #26)

New `EC_IO_ERROR` error code. Consolidate error counting to `DiagCtx` for
consistent error tracking across the pipeline.

---

## Phase 24: Advanced Language Features

**Priority: DEMAND-DRIVEN — each is triggered when a concrete need surfaces.**

### 24.1 String comparison deallocation ✅ (partial)

Typed free functions (`__vow_string_free`, `__vow_vec_free_val`,
`__vow_map_free`) implemented in `vow-runtime`. Both lowerers emit inline
`__vow_string_free` calls immediately after string equality/contains
comparisons when one operand is a string literal. This eliminates the
dominant leak pattern (keyword matching in loops — ~180K allocations per
compiler invocation). Stress test: 100K iterations × 4 comparisons uses
constant 2.7 MB RSS.

**Remaining work (future):**
- Scope-exit deallocation for `let`-bound strings (requires escape analysis)
- Vec/Map/Struct deallocation
- Arena header-based `__vow_arena_free` for struct allocations
- `drop()` language builtin for manual control

### 24.2 Recursive type ESBMC bounds

**Trigger: proven — Stretch benchmarks H07 (ring_buffer) and H10
(expression_eval) hit `--unwind` ceiling.**

Options:
- Automatic `--unwind` bound selection based on type recursion depth
- User-specified per-function unwind annotations
- Direct goto-program emission (bypass C intermediate representation)

### 24.3 Effect system completion

**Trigger: agent needs to reason about which functions can panic.**

`[Panic]` effect exists in the grammar but no builtins are annotated with it.
Division by zero, array out-of-bounds, and `.unwrap()` are all silent panic
sources. Completing the effect system would let agents statically reason about
failure modes.

### 24.4 Linear type enforcement

**Trigger: agent writes resource-managing code that leaks handles.**

The `linear struct` syntax and checker infrastructure exist (`vow-types/src/
linear.rs`, 192 lines) but have never been exercised in practice. No examples
use `linear struct`. Activation requires resource types (file handles, network
connections) to exist in the language.

### 24.5 Direct goto-program emission

**Trigger: C model limitations accumulate (Ptr type mismatches, struct field
tracking, fixed-size collection models).**

Emit ESBMC goto programs directly from Vow IR instead of going through C.
Eliminates the C type system as an intermediate representation. Currently
Vec is modeled as `int64_t data[128]`, String as `int8_t data[256]`,
HashMap as 64-entry arrays — these fixed sizes are artificial constraints.

---

## Phase 25: Ecosystem

**Priority: DEMAND-TRIGGERED — implement only when external demand justifies.**

### LSP server

Trigger: significant demand from human developers using Vow in editors.
Agents use the CLI, not editors. Implement only if human adoption justifies.

### MCP server

Trigger: specific AI tool integration requires MCP as the interface. The
skill document + JSON CLI output covers the same ground for direct usage.

### FFI contract enforcement

Trigger: agent writes `extern "C"` blocks without contracts, producing
unverified code. The type checker should require vow blocks on all extern
declarations.

### Concurrency model

Trigger: use case requiring concurrent Vow programs surfaces. The effect
system provides the foundation (`[Concurrent]` effect), but the execution
model is undefined.

### Constrained decoding for Vow grammar

The grammar is well-suited for grammar-constrained LLM sampling (a la
MoonBit's semantics-based sampler). Worth exploring if the skill-based
approach plateaus. Recent research (ICML 2025) shows 17x faster preprocessing
for grammar-constrained decoding. If agents achieve high verification rates
with the skill alone, constrained decoding is an optimization.

### Lean integration

May be revisited for proofs beyond ESBMC's bounded model checking. Deferred
until a concrete verification need exceeds ESBMC's capabilities.

---

## References

- Vow Design Sketch v3 (26.02.2026) — `vow_design_sketch_v3.md`
- Ideas for Improvement — `ideas-improvement.md`
- Vericoding Benchmark: arxiv.org/abs/2509.22908
- CEGIS Repair (AAAI 2025): arxiv.org/html/2502.07786v1
- MoonBit AI-Native Toolchain: moonbitlang.com/blog/moonbit-ai
- Bosque Language: github.com/BosqueLanguage/BosqueCore
- Armin Ronacher, "A Language For Agents": lucumr.pocoo.org/2026/2/9/a-language-for-agents/
- Martin Kleppmann, "AI will make formal verification mainstream": martin.kleppmann.com/2025/12/08/ai-formal-verification.html
- DafnyPro (POPL 2026): popl26.sigplan.org
- ATLAS (2025): arxiv.org/abs/2512.10173
- BRIDGE (2025): arxiv.org/abs/2511.21104
- ESBMC + Rust Foundation: rustfoundation.org/media/expanding-the-rust-formal-verification-ecosystem-welcoming-esbmc/
- ai-coding-lang-bench: github.com/mame/ai-coding-lang-bench

---

*This document captures the forward-looking roadmap as of 15 March 2026.
Phase 21 critical path: 21.1 → 21.4 → 21.7 (publish direct comparison).
Parallel: 21.3 → 21.6 → 21.8 (publish dual-track update). Phase 22 improves
agent ergonomics. Phase 23 is toolchain polish. Phases 24–25 are
demand-driven. If a phase isn't earning its keep, cut it.*
