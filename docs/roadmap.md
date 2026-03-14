# Vow Roadmap — Revised (14.03.2026)

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
- Vericoding benchmark: **100% (36/36)** with claude-sonnet-4-6 vs Dafny 82%, Verus 44%, Lean 27%
- 82/82 tests passing, 40/40 CLI compatibility tests
- Toolchain Skill document, structured `--help`, `--debug-trace`, incremental compilation
- Bootstrap: `scripts/bootstrap.sh` produces `./vowc` from Rust stage 0

**Current maturity: 9/10 for agent autonomy on verified programs.**

Known limitations:
- Arena deallocation is a no-op (`__vow_arena_free` leaks; fine for short-lived programs)
- Expression-level source spans are unpopulated (function/statement spans work)
- 2/4 Stretch benchmarks hit ESBMC `--unwind` ceiling (H07, H10)
- `divide.vow` release build has UB (no runtime checks; debug mode works)
- Zero public visibility — benchmark results not yet published

---

## Competitive Landscape (March 2026)

| Project | Approach | Verification | Status |
|---------|----------|-------------|--------|
| **MoonBit** | Constrained token sampling | No formal verification | Approaching 1.0; AI Pilot agent |
| **Bosque** | Contracts, canonical form | Research-stage verifier | Solo maintainer, slow progress |
| **Dafny + LLMs** | LLM-generated proofs | DafnyPro: 86% (POPL 2026) | Very active research ecosystem |
| **Verus + LLMs** | AutoVerus, VeriStruct | OS verification (Asterinas) | Growing Rust verification niche |
| **Dana** | Intent-driven, agent orchestration | None | AI Alliance backed, different niche |
| **Vow** | ESBMC + blame tracking, CEGIS | **100% (36/36)** integrated | Self-hosted, zero visibility |

The Dafny ecosystem is publishing prolifically (ATLAS, DafnyPro, BRIDGE — all
at POPL 2026). Vow's 100% result is the best published number but nobody knows
about it. The competitive window is open but narrowing.

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

## Phase 21: Publish & Position

**Priority: CRITICAL — competitive window is closing.**

The 100% vericoding result is Vow's strongest asset but has zero visibility.
Meanwhile, DafnyPro reached 86% at POPL 2026 and is publishing actively.

### 21.1 Publish benchmark results

**Status: BLOCKED — comparison methodology invalid, spec expressiveness gap
identified.**

The original 36/36 (100%) result is on Vow's own benchmark suite. The
Vericoding paper (arxiv.org/abs/2509.22908) uses 12,504 specifications across
Dafny (3,029), Verus (2,334), and Lean (7,141) — the same problems translated
across languages. Their headline numbers (Dafny 82%, Verus 44%, Lean 27%) are
the **union across 9 models** with up to 5 repair iterations each, not a single
model's result. The best single model achieves 67.5% on Dafny (Opus 4.1).

Comparing Vow's 36/36 on custom benchmarks against those numbers is not valid.

**Pilot: 10 HumanEval problems translated to Vow (March 2026).**
Translated 10 problems from the Vericoding HumanEval-Dafny set to Vow
(`benchmarks/humaneval/HE*`). Result: 10/10 verified with claude-sonnet-4-6,
mean 1.3 CEGIS iterations. However:

- Only 2/10 have **exact** Dafny-equivalent contracts (HE041, HE060). The
  rest have weaker contracts because Vow ensures clauses cannot call
  user-defined functions or express quantifiers.
- Weaker contracts make verification easier — a trivial implementation can
  satisfy weak specs. This inflates Vow's pass rate relative to Dafny.
- Nested loops with Vec access fail ESBMC verification (had to drop 2SUM
  and 3SUM problems).
- `let mut` declarations inside loop bodies cause ESBMC errors — variables
  must be declared outside all enclosing loops.

**Key blocker: spec function calls in ensures clauses.** The C emitter
currently replaces non-constant function calls with `__VERIFIER_nondet()`,
making them meaningless. If the emitter instead emitted the actual function
body into the C model, ensures clauses could reference pure spec functions
(e.g., `ensures: result == is_prime_spec(n)`). ESBMC would then verify by
bounded model checking — no quantifiers needed, no new syntax, just a
verification pipeline fix. This would make Vow contracts as strong as Dafny's
for bounded inputs.

**Path to publishable comparison:**
1. Fix C emitter to emit spec function bodies (not nondet) for pure functions
   referenced in ensures clauses.
2. Re-translate the 10 HumanEval pilots with full-strength contracts.
3. Scale to the full 162 HumanEval-Dafny set from the Vericoding benchmark.
4. Run the same protocol (up to 5 CEGIS iterations) and compare against
   their published per-model results.

**Resources:**
- Vericoding benchmark: github.com/Beneficial-AI-Foundation/vericoding-benchmark
- Vericoding scripts: github.com/Beneficial-AI-Foundation/vericoding
- Pilot results: `bench/results/humaneval-pilot/`
- Pilot benchmarks: `benchmarks/humaneval/HE*`

Target: blog post + arxiv preprint (after spec function fix).

### 21.2 Expand example coverage

The `examples/` directory has significant feature gaps that weaken the skill
document. Add examples demonstrating:

- `match` expressions on enums (currently zero examples)
- `Option<T>` and `Result<T,E>` workflows
- `?` operator for error propagation
- Checked arithmetic operators (`+!`, `-!`, `*!`)
- Non-IO effects (`[Read]`, `[Write]`)
- `f32`/`f64` floating-point types

Each example should be a complete, runnable program that compiles and verifies.

---

## Phase 22: Language Ergonomics

**Priority: HIGH — these directly improve agent productivity and reduce
error rates. Ordered by implementation difficulty (easiest first).**

### 22.1 Named constants (`const` declarations) — GitHub #15

Top-level `const` declarations that fold at compile time:

```
const TWO_32: i64 = 4294967296
const HASHMAP_ENTRY_STRIDE: i64 = 16
```

Impact: eliminates magic numbers throughout the self-hosted compiler (e.g.,
`4294967296` in `item_pack`, `16` for HashMap stride, `24` for HashMap header).
Agents cannot accidentally use the wrong literal. Zero runtime cost.

Scope: lexer + parser + type checker + IR lowering (constant folding). No
verification changes needed.

### 22.2 Break and break-with-value — GitHub #13

`break` exits the current loop. `break <expr>` exits with a value (loop
becomes an expression):

```
let found: i64 = loop {
    if list_get(data, lid, i) == target { break i }
    i = i + 1
    if i >= n { break -1 }
}
```

Impact: eliminates the sentinel-flag antipattern from search and validation
code. The self-hosted compiler has many instances of `let found: i64 = -1`
followed by a while loop. Agents frequently fail to handle the `-1` sentinel
correctly in all callers.

Scope: lexer (`break` keyword) + parser + IR lowering (jump to loop exit
block) + verification (break as loop termination).

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

## Phase 23: Standard Library Expansion

**Priority: HIGH — prerequisite for the AI coding benchmark (Phase 24) and
generally useful for any real-world Vow program.**

These are new runtime builtins registered in the type checker and lowerer.

### 23.1 Filesystem builtins

7 new `[IO]` functions:
- `fs_mkdir(path: String) -> i64`
- `fs_exists(path: String) -> i64`
- `fs_listdir(path: String) -> Vec<String>`
- `fs_remove(path: String) -> i64`
- `fs_remove_dir(path: String) -> i64`
- `fs_is_dir(path: String) -> i64`
- `fs_rename(old: String, new: String) -> i64`

Scope: `vow-runtime` C implementations + type checker registration + IR
lowerer builtin dispatch.

### 23.2 String builtins

11 new string functions:
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

### 23.3 Bitwise operations

XOR operator (`^`) and hex conversion:
- `a ^ b` — bitwise XOR (new token kind + parser + IR opcode)
- `hex_encode(data: Vec<u8>) -> String`
- `hex_decode(s: String) -> Vec<u8>`

### 23.4 Vec sorting

- `vec_sort(v: Vec<i64>) -> Vec<i64>` — returns sorted copy

### 23.5 Time builtin

- `time_unix() -> i64` — current Unix timestamp in seconds

---

## Phase 24: AI Coding Language Benchmark

**Priority: HIGH — external validation and visibility.**

Participate in `ai-coding-lang-bench`, which measures how efficiently Claude
Code can implement a mini-git clone across programming languages.

### 24.1 Benchmark harness integration

Fork the benchmark repository. Add Vow as a target language with build
scripts, skill docs, and test infrastructure.

### 24.2 Pilot runs and iteration

Run Claude Code against the mini-git specification using `./vowc`. Identify
failure modes. Fix the toolchain, not the agent.

### 24.3 Full benchmark execution

20-trial suite for statistical significance. Target: ≥38/40 pass rate with
competitive execution time (<90s).

### 24.4 Verified variant

Run the benchmark with `vow build` (verification enabled). Demonstrate that
Vow can produce a verified mini-git implementation — something no other
benchmark language can do.

---

## Phase 25: Toolchain Improvements

**Priority: MEDIUM — these improve the development experience but don't
block current workflows.**

### 25.1 Verification caching

Cache ESBMC results by function content hash. If a function hasn't changed,
skip re-verification. This directly speeds up the CEGIS loop — currently
every `vow build` re-verifies all functions even if only one changed.

### 25.2 Expression-level source spans

Populate expression spans in the self-hosted compiler's parser. Currently
only function and statement spans are wired through. Expression spans would
give more precise error locations (e.g., "type mismatch in `a + b`" instead
of "type mismatch in fn foo").

### 25.3 Property-based tests for compiler pipeline (PR #27)

Add proptest-based roundtrip, determinism, and robustness tests across
vow-syntax, vow-types, and vow-codegen. Catches edge cases that handwritten
tests miss.

### 25.4 IO error diagnostic code (PR #26)

New `EC_IO_ERROR` error code. Consolidate error counting to `DiagCtx` for
consistent error tracking across the pipeline.

---

## Phase 26: Advanced Language Features

**Priority: DEMAND-DRIVEN — each is triggered when a concrete need surfaces.**

### 26.1 String comparison deallocation ✅ (partial)

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

### 26.2 Recursive type ESBMC bounds

**Trigger: proven — Stretch benchmarks H07 (ring_buffer) and H10
(expression_eval) hit `--unwind` ceiling.**

Options:
- Automatic `--unwind` bound selection based on type recursion depth
- User-specified per-function unwind annotations
- Direct goto-program emission (bypass C intermediate representation)

### 26.3 Effect system completion

**Trigger: agent needs to reason about which functions can panic.**

`[Panic]` effect exists in the grammar but no builtins are annotated with it.
Division by zero, array out-of-bounds, and `.unwrap()` are all silent panic
sources. Completing the effect system would let agents statically reason about
failure modes.

### 26.4 Linear type enforcement

**Trigger: agent writes resource-managing code that leaks handles.**

The `linear struct` syntax and checker infrastructure exist (`vow-types/src/
linear.rs`, 192 lines) but have never been exercised in practice. No examples
use `linear struct`. Activation requires resource types (file handles, network
connections) to exist in the language.

### 26.5 Direct goto-program emission

**Trigger: C model limitations accumulate (Ptr type mismatches, struct field
tracking, fixed-size collection models).**

Emit ESBMC goto programs directly from Vow IR instead of going through C.
Eliminates the C type system as an intermediate representation. Currently
Vec is modeled as `int64_t data[128]`, String as `int8_t data[256]`,
HashMap as 64-entry arrays — these fixed sizes are artificial constraints.

---

## Phase 27: Ecosystem

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

---

*This document captures the forward-looking roadmap as of March 2026. Phases
21–25 are prioritised by impact. Phase 26 is demand-driven. Phase 27 is
demand-triggered. If a phase isn't earning its keep, cut it.*
