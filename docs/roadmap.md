# Vow Roadmap — Revised (03.03.2026)

This plan supersedes the original Phase 12–15 roadmap. It was produced by
reviewing the design sketch (v3), ideas-improvement.md, the original roadmap,
and the competitive landscape — then realigning every phase with Vow's core
vision: **agents are the primary programmers, the toolchain is their interface.**

---

## Where Vow Stands Today

Self-hosting is achieved. The bootstrap triple test passes (6276 lines across
13 modules). Phase 10 (CEGIS loop closure) and Phase 11 (module loading + build
system) are complete.

**Current maturity: 6.5/10 for agent autonomy.**

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

Critical gaps:
- Self-hosted compiler uses zero vow blocks (credibility gap)
- No cross-module type resolution in self-hosted checker (ideas #2, #4, #8)
- String equality still depends on IR tagging rather than type-level dispatch (ideas #7)
- Counterexample → fix suggestion pipeline lacks full blame chain

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

## Phase 13 (revised): Cross-Module Maturity (~2 weeks)

**Goal:** Agents can write multi-module programs with cross-module type
resolution and contracts. Level 2 capability.

### 13.1 Cross-module type resolution in self-hosted compiler

From ideas-improvement.md #2. The self-hosted `main.vow` must follow `use`
declarations and load/merge dependent modules before type checking. Without
this, types from other modules resolve as opaque, forcing leniency rules that
mask real errors.

### 13.2 Declaration files (`.vow.d`)

From ideas-improvement.md #8. A lightweight format containing only type
signatures, function signatures, effect annotations, and contracts — without
implementations. The type checker loads these for cross-module checking without
parsing full source. Benefits:

- Faster type checking (no need to parse implementation bodies)
- Enables partial checking when not all source is available
- Natural boundary for incremental compilation
- Agents can generate stubs for modules they haven't written yet

### 13.3 Fix struct-vs-enum ambiguity for unknown named types

From ideas-improvement.md #4. When a named type is not declared locally,
`resolve_ast_ty` cannot tell struct from enum. With full module loading
(13.1) and declaration files (13.2), this should be fully resolved. Verify
that the `CTY_UNKNOWN` (already implemented) correctly handles all remaining
edge cases.

### 13.4 Type-level string equality

From ideas-improvement.md #7. String equality should be dispatched based on
the *type* of both operands (`Ty::String` → emit `__vow_string_eq`), not via
runtime tagging of IR instructions. The current tagging approach is fragile
and produces silent pointer comparison bugs when a String value comes from
an untagged source (e.g., FieldGet).

### 13.5 Level 2 Agent Capability Test

Run the test: agent writes a multi-module program (≥3 modules) with cross-
module contracts. Types resolve correctly. Agent fixes counterexamples in ≤3
CEGIS iterations. Document what breaks.

---

## Phase 14 (revised): Contract Retrofit + CEGIS Validation (~3 weeks)

**Goal:** The self-hosted compiler has contracts. The full blame-tracking
pipeline works end-to-end. Level 3 capability.

### 14.1 Agent-driven contract retrofit on self-hosted compiler

The self-hosted compiler uses zero vow blocks. This is both a credibility gap
and a missed validation opportunity. An agent adds contracts to compiler
modules, starting where specs are most natural:

1. **Lexer** — every token produced is valid, token stream is well-formed,
   no tokens with empty spans
2. **Parser** — AST is well-formed, every node has a valid kind, parentheses
   are balanced
3. **Type checker** — type environment is consistent, every resolved type is
   in the type store, no dangling type IDs

This serves three purposes:
- Validates that the CEGIS loop works at scale (real code, not toy examples)
- Gives the compiler actual verified properties (credibility)
- Tests whether the skill document is sufficient for contract authoring on
  existing code

### 14.2 Complete the blame-tracking pipeline

The counterexample JSON (Phase 10.2) includes variable values and vow_id.
Verify that it also includes:

- The call site that violated a `requires` (caller blame) — file, line, column
- The function body that violated an `ensures` (callee blame) — same
- The full blame chain for multi-function paths (A calls B calls C, C's
  requires was violated by B's argument, which came from A's computation)

If any of this is missing, add it. The agent needs the complete blame chain
to fix the right function, not just the function where the violation was
detected.

### 14.3 Counterexample-guided fix suggestions

Go beyond raw counterexample reporting. When ESBMC produces a counterexample:

- Map variable values back to source-level names (not just IR temporaries)
- Identify which branch of the code was taken to reach the violation
- If the violation is a `requires`, identify the call site and the expression
  that produced the violating value
- If the violation is an `ensures`, identify which execution path through the
  function body fails to establish the postcondition

This structured information goes into the counterexample JSON. The agent uses
it to decide what to change. The compiler doesn't suggest fixes — that's the
agent's job — but it gives the agent enough information to reason about fixes.

### 14.4 Error suggestion hints in diagnostics

From ideas-improvement.md implicit in #7 and the original roadmap Phase 15.
Add structured fix hints to common error patterns:

- Type mismatch: "expected i64, got String" → hint: "did you mean `.len()`?"
- Effect violation: "calling [Read] from pure function" → hint: "add [Read] to function signature"
- Missing match arm: "non-exhaustive match on Shape" → hint: "missing arm: Shape::Circle"
- Unused linear value: "FileHandle must be consumed" → hint: "call `close(handle)`"

Hints are in the JSON diagnostic, not in prose. The agent reads them as
structured suggestions.

### 14.5 Level 3 Agent Capability Test

Run the test: agent adds contracts to the lexer module of the self-hosted
compiler. Achieves verification. Document what breaks in the CEGIS loop at
this scale.

---

## Phase 15 (revised): Vericoding Benchmark (~2 weeks)

**Goal:** Vow is positioned as a reference language for specification-driven
AI coding. Level 4 capability.

### 15.1 Define the benchmark suite

Design N formal specifications (target: 30–50) across difficulty levels:

- **Easy (10–15):** Pure arithmetic, simple data structures, sorting,
  searching. Single-function, base-type contracts.
- **Medium (10–15):** Multi-function algorithms, collection-type contracts,
  cross-function invariants. Binary search trees, graph algorithms, parsers.
- **Hard (5–10):** Multi-module programs, stateful algorithms, compiler
  phases. Contracts involving multiple interacting invariants.

Each spec includes: natural language description, formal contracts in Vow
syntax, reference implementation (for differential testing), expected ESBMC
unwind bounds.

### 15.2 Run agents against the suite

Test with multiple frontier models (Claude, GPT, Gemini — whatever is
available). Measure:

- **Verification rate:** % of specs where the agent produces verified code
- **CEGIS iterations:** how many counterexample-fix cycles per spec
- **Time to verified binary:** wall-clock including all iterations
- **Failure modes:** categorize why the agent failed (wrong algorithm, wrong
  contract, ESBMC timeout, type error, etc.)

### 15.3 Compare against vericoding paper results

The reference numbers from arxiv.org/abs/2509.22908:
- Dafny: 82% verification rate
- Verus/Rust: 44%
- Lean: 27%

Vow's hypothesis: blame-tracking contracts + structured counterexamples +
the CEGIS-ready pipeline yield higher verification rates than unguided
approaches. Test this hypothesis.

### 15.4 Publish results

Write up findings. Position Vow as the reference language for vericoding.
The narrative: "Vow is the language where AI agents prove their code correct."

---

## Phase 16: Advanced Language Features (As Needed)

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
  grammar is well-suited for grammar-constrained LLM sampling (à la MoonBit's
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
| — | 16: Features as needed | Demand-driven, not speculative. |

The through-line: **the toolchain is the product, not the agent.** If the
toolchain emits the right structured data and the skill teaches the workflow,
the agent is a commodity. Invest in the interface, not the consumer.

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

