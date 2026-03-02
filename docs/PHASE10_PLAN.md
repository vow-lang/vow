# Vow Roadmap: Phase 10 and Beyond

This plan was produced by a multi-agent analysis of the Vow codebase,
design sketch, ideas-improvement.md, and the 2025-2026 competitive landscape
for AI-first / agent-optimized programming languages.

---

## Where Vow Stands Today

Self-hosting is achieved. The bootstrap triple test passes (6276 lines across
13 modules). But the project is at an inflection point: the compiler works,
but agents can't yet write verified programs in Vow beyond toy examples.

**Current maturity: 6.5/10 for agent autonomy.**

Strengths:
- JSON build output with status codes (Verified, Unverified, CompileFailed, VerifyFailed)
- VowViolation JSON: vow_id, blame (Caller/Callee), description, variable values
- Dual output (JSON + human-readable) always on
- Self-hosting compiler is a verified fixed point

Critical gaps:
- Diagnostic array NOT in build JSON (only first error visible)
- ESBMC counterexamples are unstructured raw text
- Cannot verify heap-type contracts (String, Vec, HashMap)
- No LSP/MCP server
- Only 4 example programs, 2 contracts total
- Self-hosted compiler uses zero vow blocks

---

## Strategic Context

### Competitive Landscape (2025-2026)

**MoonBit** is the closest competitor — but went a fundamentally different
direction: constrained token sampling (prevent bad generation) vs. Vow's
post-hoc verification (prove correctness). MoonBit has no formal verification.
MoonBit Pilot (integrated AI agent) generated a TOML parser in 6 minutes.

**Bosque** (Microsoft Research) is the philosophical sibling — contracts,
canonical form, determinism. But purely research; no self-hosting compiler.

**"Vericoding"** is now a named concept (Sep 2025 paper, arxiv.org/abs/2509.22908):
LLM generation from formal specs, verified by formal methods. Success rates:
82% in Dafny, 44% in Verus/Rust, 27% in Lean. Vow is architecturally
positioned to be a reference language for vericoding.

**CEGIS** (counterexample-guided repair) is validated (AAAI 2025): generate
code -> verify -> get counterexample -> fix. This is exactly what Vow's
contract + ESBMC pipeline enables.

**Kleppmann** (Dec 2025): "AI will make formal verification go mainstream."

**LSP/MCP is table stakes** — Claude Code shipped native LSP support (900x
speedup). Every AI coding tool interfaces via LSP.

### Vow's Unique Differentiators

1. Formal verification integrated into the compile pipeline (nobody else has
   this in a systems language)
2. Blame-tracking contracts (requires blames caller, ensures blames callee)
3. CEGIS potential (contracts + ESBMC + counterexamples + source mapping)
4. Effect system for agent safety (static side-effect knowledge)
5. No-comments + canonical syntax (strongest "single representation" guarantee)
6. Self-hosting as credibility proof

### The Strategic Narrative

Vow is the language where AI agents prove their code correct. Not "AI writes
code and hopes it works" (every other language). Not "AI writes tests and hopes
they're sufficient" (current practice). Vow: write the contract, generate the
code, prove it correct, blame-track any failure, counterexample-guide the fix.

---

## Phase 10: Close the CEGIS Loop (~4 weeks)

The highest-leverage work. Make this workflow end-to-end:

    agent writes spec -> writes code -> compile -> ALL errors as JSON ->
    fix -> verify -> structured counterexample -> fix -> verify passes

### 10.1 All diagnostics in build JSON (hours)

The JsonEmitter already collects all diagnostics. The CLI driver just doesn't
expose them in the build output JSON. Add a `"diagnostics": [...]` array.

Impact: Agents can batch-fix all errors in one pass (50% faster feedback loop).

### 10.2 Structured ESBMC counterexamples as JSON (~1 week)

Parse ESBMC output (XML or text) into structured JSON:
`{"inputs": {"y": 0}, "violation": "y != 0", "source": {"file": "divide.vow", "offset": 42}}`

Impact: Verification failures become actionable. Agents can suggest specific fixes.

### 10.3 Source location in runtime VowViolation (days)

Codegen already emits vow_id and blame. Add file path and byte offset to the
VowEntry metadata, thread it through to `__vow_violation`.

Impact: Agents can jump directly to violation source.

### 10.4 Vec/String/HashMap ESBMC models (~2 weeks)

Extend vow-verify/src/c_emitter.rs to model collection operations:
- `v.len()`, `v.push()`, `v.get()` for Vec
- `s.len()`, `s.contains()` for String
- `m.contains_key()`, `m.len()` for HashMap

Impact: Unlocks contract verification for real programs (not just integer arithmetic).

### 10.5 `where` clause / refinement type syntax (days)

`fn divide(x: i64, y: i64 where y != 0) -> i64` desugars to `requires: y != 0`.

`type NonZero = { x: i64 | x > 0 }` as refinement type syntax.

Parser changes + desugaring to existing vow block infrastructure.

### 10.6 Verified example programs (~1 week)

Write 10-15 programs demonstrating the full contract-verify loop:
- `ensures` with `result` keyword
- Multiple contracts per function
- Vec/String predicates
- Multi-function call chains with blame tracking
- Loop invariants on collection iteration

---

## Phase 11: Module Loading + Build System (~2 weeks)

### 11.1 DFS module loading in self-hosted compiler (~300 lines)

Implement recursive `use` resolution in compiler/main.vow, mirroring the
Rust module_loader.rs logic. Parse `use foo.bar` -> resolve to `foo/bar.vow`
-> read -> parse -> merge all items.

Removes the concat_vow.sh crutch. Enables multi-file Vow projects.

### 11.2 Basic build commands

`vow build`, `vow verify`, `vow test` as top-level commands for Vow programs
(distinct from Cargo, which is for the Rust reference compiler).

---

## Phase 12: LSP Server + MCP Integration (~3-4 weeks)

### 12.1 Vow language server

Diagnostics, go-to-definition, type information, hover docs (contract display).
Makes Vow usable from Claude Code, Cursor, VS Code with AI assistants.

### 12.2 MCP server

Expose verification results, contract status, and counterexamples to AI tools
via Model Context Protocol.

---

## Phase 13: Integrated Agent / "Vow Pilot" (~2-4 weeks)

A dedicated agent that understands the Vow toolchain:
1. Compiles code, reads all diagnostics
2. Fixes type/effect/syntax errors in batch
3. Invokes ESBMC, reads structured counterexamples
4. Acts on counterexamples to fix contract violations
5. Iterates until verification passes

Consider: constrained decoding support (Vow's grammar is well-suited for
grammar-constrained LLM sampling, a la MoonBit's semantics-based sampler).

---

## Phase 14: Vericoding Showcase

Publish a benchmark: N formal specs in Vow, LLM generates implementations,
ESBMC verifies. Compare against Dafny/Verus results from the vericoding paper.

Positions Vow as the reference language for specification-driven AI coding.

---

## Phase 15: Advanced Language Features (Lower Priority)

These matter but only after the verification loop is closed:

- Linear type enforcement (`linear struct` keyword + full checker tracking)
- Region-based memory (arena-per-scope runtime, explicit `region` syntax)
- Effect system completion (all builtins annotated, `[Panic]` tracking)
- Incremental compilation / verification caching
- Mandatory contracts on `extern "C"` blocks
- FFI contract enforcement in type checker
- Error suggestions in diagnostics ("expected i64, got String; did you mean .len()?")
- `--debug-trace` flag for structured execution traces

---

## Success Metric

Can an agent write a non-trivial Vow program (not the compiler), specify
contracts, verify them with ESBMC, and fix any counterexamples — without
human intervention?

That's the bar for "agentic coding language." Phase 10 gets us there for
base-type contracts. Phase 10.4 extends it to collection types. Phase 13
automates the entire loop.

---

## References

- Vericoding Benchmark: arxiv.org/abs/2509.22908
- CEGIS Repair (AAAI 2025): arxiv.org/html/2502.07786v1
- MoonBit AI-Native Toolchain: moonbitlang.com/blog/moonbit-ai
- MoonBit Pilot: moonbitlang.com/blog/intro-moonbit-pilot
- Bosque Language: github.com/microsoft/BosqueLanguage
- Armin Ronacher, "A Language For Agents": lucumr.pocoo.org/2026/2/9/a-language-for-agents/
- Martin Kleppmann, "AI will make formal verification mainstream": martin.kleppmann.com/2025/12/08/ai-formal-verification.html
- SWE-AGI Benchmark (MoonBit): arxiv.org/html/2602.09447v2
