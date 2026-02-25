# Vow Language Design Sketch — v1 (25.02.2026)

## 1. Vision

**Vow** is a programming language for an agent-first development paradigm: AI agents write, verify, and debug code while humans specify intent and run the final executables. The name comes from the central language construct — a **vow block** — which is the contract between human and machine. The human understands it because it's in plain English with formal backing; the machine satisfies it because the verification backend (Lean/ESBMC) proves it so.

**Motto: "There's a single way to do it."**

This principle drives every design decision. Ambiguity is the enemy. Flexibility is a cost, not a feature. The language is rigid by design so that agents can reason about it mechanically and humans can trust the output.

## 2. Core Design Principles

- **Canonical syntax enforced by the compiler.** No formatters, no style debates. The compiler includes a canonicalizer — there is exactly one surface representation for any given AST. This gives diff-stability for free and eliminates "was this a meaningful change or just reformatting?" ambiguity.
- **No comments.** Intent is captured by contracts and specifications. API documentation is the type signature plus contracts plus the `--help`-as-skill system. If humans need to communicate meta-instructions to agents ("I want this to be fast, not just correct"), those are structured annotations, not free-text comments.
- **Explicit semantics over implicit conventions.** No implicit conversions, no context-dependent parsing, no operator overloading surprises. Every syntactic construct has exactly one semantic interpretation.
- **Compositional and modular.** Small, orthogonal constructs that combine predictably. Clear interfaces with explicit contracts. Local reasoning without global context requirements.
- **Verification as a first-class citizen.** Specifications are part of the syntax. Verification conditions are automatically generated. Proof obligations guide code generation.

## 3. The Vow Block

The central language construct. Every module declares its vows — preconditions, postconditions, invariants — in a structured block that serves as both human-readable documentation and machine-checkable specification.

```
module PathFilter {
  vow {
    requires: valid_utf8(input)       // caller's obligation
    ensures:  valid_path(output)      // module's obligation
    sound:    forall p in output, matches(p, pattern)
  }
}
```

Vows have two lives:
- **At build time:** translated to Lean proof obligations and formally verified.
- **In debug mode:** compiled to runtime assertions that catch translation bugs.

When a vow is violated at runtime, the diagnostic is structured and precise: which vow broke, at which program point, with which inputs, and which side of a module boundary is to blame.

## 4. Type System

### Base types: simple and decidable
- Algebraic data types (sum and product types)
- Generics / parametric polymorphism
- **No subtyping** (subtyping creates ambiguity that hurts agents)
- **No inheritance** (composition through traits/interfaces only)
- **No implicit arguments or typeclass resolution** — all trait implementations are explicit, no orphan instances

The base type system stays decidable and fast. The agent can always know whether code type-checks without running the verifier.

### Refinement types for specifications
On top of the base types, refinement predicates are checked by the verification backend (ESBMC/Lean), not the type checker:

```
fn divide(x: i64, y: i64 where y != 0) -> i64
type NonEmptyList<T> = { xs: List<T> | length(xs) > 0 }
type ValidPath = { p: Path | is_valid_utf8(p) && no_null_bytes(p) }
```

This separation is key: types are decidable and mechanical; refinements are expressive and handled by powerful external engines. Most of the expressiveness of dependent types without the complexity.

### Deliberately excluded
- **Full dependent types in surface syntax.** Too complex for agents to reason about. If the agent needs dependent types for a specific proof, it drops into Lean.
- **Null.** `Option<T>` instead.
- **Exceptions.** `Result<T, E>` everywhere with `?` propagation. Exceptions create invisible control flow.

## 5. Effect System

Every function declares its effects in its type signature. A function with no effect annotation is pure — the compiler enforces this.

```
fn pure_transform(data: Bytes) -> Bytes { ... }                    // pure
fn read_entry(path: Path) -> Result<FileEntry> [Read] { ... }     // Read effect
fn write_output(data: Bytes) -> Result<Unit> [Write] { ... }      // Write effect
```

Effects are auto-propagated — if you call a function with `[Read]`, your function must also declare `[Read]` (or the compiler adds it). This makes side effects visible at every call site without manual bookkeeping.

The effect system enables:
- **Complete effect traces at runtime** — because the language requires effect declarations, the runtime knows exactly what operations are permitted and can log them as structured data.
- **Total effect mediation** — every interaction with the outside world goes through the effect system. No escape hatches (or if there are, they're marked `unsafe` and excluded from verification).
- **Deterministic replay** — all non-determinism is captured in an effect journal.

## 6. Memory Management

**Region-based allocation with compiler-assisted placement.** No garbage collector, no Rust-style lifetime annotations.

- The language has explicit memory regions (arenas, pools, scopes) as first-class constructs.
- Every allocation belongs to a region. Regions have clear lifetimes tied to lexical scope or explicit management.
- The **agent** decides the allocation strategy — which region an object belongs to.
- The **compiler/verifier** checks that no reference outlives its region.

This is simpler than Rust's full lifetime system because regions are coarser-grained. The verification obligation is "does this reference outlive its region?" — a much simpler property for ESBMC to check.

### Linear types for resource management
A value of linear type must be used exactly once:

```
fn read_file(handle: &File) -> Result<Bytes> { ... }   // borrows, doesn't consume
fn close(handle: File) -> Unit { ... }                   // consumes, can't use after

fn process(f: File) -> Result<Unit> {
  let data = read_file(&f)?;    // borrow
  transform(data);
  close(f);                      // consume — f is gone
  ok(())
}
```

### Spectrum of control
- **Default:** compiler-chosen regions, correct but conservative ("just make it work").
- **Optimized:** agent-chosen regions with verified safety ("make it fast").
- The compiler can suggest region assignments: "these five allocations share a lifetime scope, grouping them into a single arena eliminates four individual deallocations."

## 7. Number Tower

Minimal at the language level. Everything maps to hardware.

### Language-level (fixed-width)
- Integers: `i8`, `i16`, `i32`, `i64`, `i128`, `u8`, `u16`, `u32`, `u64`, `u128`
- Floats: `f32`, `f64` (IEEE 754)

### Overflow semantics: explicit per operation
No undefined behavior. No mode-dependent semantics. The agent picks the right mode for each use case:
- **Wrapping arithmetic** — wraps on overflow
- **Checked arithmetic** — returns `Result` type on overflow
- **Saturating arithmetic** — clamps to min/max

### Standard library (not language-level)
- Bigints, rationals, decimals, complex numbers
- Each can be domain-optimized (crypto bigints ≠ arbitrary-precision bigints)

## 8. Toolchain Architecture

The toolchain is a **unified system**, not a loose collection of independent tools.

### Parallel compilation and verification
Compilation and formal verification run in parallel from a shared IR:

```
Vow Source → Parser → AST → Shared IR
                                ├── Native codegen
                                ├── ESBMC bounded model checking
                                └── Lean proof obligations
```

Every successful build is also a correctness proof. The compiler *is* the verifier, or at least they share enough infrastructure to run concurrently. Errors from either path surface in a unified diagnostic stream.

### Self-describing tools (`--help` as skill)
Every tool in the ecosystem returns structured, machine-readable skill documents via `--help`. An agent that has never seen the tool before reads the skill and knows how to use it — capabilities, expected inputs, effects, error modes. No training data required.

This extends to the language itself: every module exports a machine-readable interface description (types, contracts, effects, usage examples). The language is self-describing at every level.

### Structured diagnostics
Error messages are data, not strings. The compiler, runtime, and debugger all emit structured output alongside human-readable messages. Agents parse diagnostics programmatically.

## 9. Debugging as Automated Diagnosis

Traditional source-level debuggers are the wrong tool for agentic debugging. An agent doesn't benefit from stepping through code — it can reason about the entire function at once. Instead, Vow supports **automated diagnosis**.

### Execution traces over breakpoints
The runtime produces structured execution traces — not printf debugging, but a first-class facility where a failing execution yields a complete, machine-readable record of function calls, argument values, return values, state transitions, and effect logs.

Two compilation modes for traces:
- **Development build:** fully instrumented, every function boundary and contract check.
- **Production build:** traces compiled out entirely, zero overhead. Optionally, trace points as no-ops that can be **dynamically activated** (similar to Linux ftrace) for surgical observability without recompiling.

### Differential diagnosis via contracts
The agent can mechanically instrument every function boundary along the call path and ask: "which is the first function whose postcondition was violated while its precondition was satisfied?" This is automatic fault localization — no human intuition required.

### Blame tracking across modules
When modules compose, Vow instruments interfaces to track blame. When a contract is violated at a module boundary, the diagnostic identifies which side is at fault — the caller (violated `requires`) or the callee (violated `ensures`).

### The Lean model as ground-truth oracle
When a bug is reported, the first step is testing whether the Lean model also exhibits the bug:
- **Lean crashes too → spec bug.** The vows don't capture what the user wanted. Agent engages the human to refine specifications.
- **Lean handles it → translation bug.** The implementation diverges from the model. Agent regenerates code autonomously, guided by the Lean model's behavior.

Most translation bugs never reach the human. The agent detects the divergence, regenerates, re-verifies, and re-tests. The human only gets involved when the *spec* is wrong.

### Counterexample-guided repair
Once the failing function is identified, the agent has the counterexample and the contract. This becomes a bounded synthesis problem: find a modification that satisfies the contract for this input *and* all previously passing inputs. ESBMC becomes not just a verification tool but a *repair* tool.

## 10. C Interoperability

C interop exists but is **contained behind a verification boundary**.

- Every foreign call requires a mandatory contract specifying expected behavior.
- The wrapper is verified against that contract.
- The C code itself is opaque and untrusted.
- No implicit C header parsing, no automatic binding generation.
- The agent writes the contract, the human confirms it matches the C library's behavior.

Inside Vow, the language maintains its own type system and memory model. Translation happens at the boundary only.

## 11. Self-Hosting

The ultimate goal: **Vow compiled by Vow, written by agents, verified by the toolchain.**

### Bootstrap path
1. **Stage 0:** First compiler in C++ (or Rust). Bootstrap compiler, doesn't need to be elegant.
2. **Stage 1:** Agent rewrites the compiler in Vow, module by module, each verified against a specification of what that compiler phase should do. Parser spec = the grammar. Type checker spec = the type rules. Codegen spec = the operational semantics.
3. **Stage 2:** Vow compiler compiles itself. Output verified against stage 0 via triple-test (similar to GCC bootstrap verification).

Self-hosting proves that:
- The language is expressive enough for systems programming.
- The agent toolchain can handle a non-trivial codebase.
- The specification system works for the hardest use case: a compiler for itself.

## 12. What Gets Inverted

The agent-first lens inverts many conventional language design decisions:

| Traditional | Vow |
|---|---|
| Comments explain intent | Contracts capture intent; no comments |
| Formatters enforce style | Canonicalizer is a compiler pass |
| Rich FFI for ergonomic C interop | Thin verification boundary |
| Complex type systems for expressiveness | Simple types + refinements checked by verifier |
| GC or manual memory management | Region-based with verified safety |
| Large number tower in language | Minimal hardware types; rest in stdlib |
| Interactive debugger | Automated diagnosis via traces and contracts |
| `--help` prints human text | `--help` returns machine-readable skill |
| Multiple ways to express the same thing | Single canonical form |
| Flexible syntax for human preference | Rigid syntax for mechanical reasoning |

## 13. Open Questions

- **Spec completeness:** How does the human know the vow set is sufficient? The tool could satisfy all properties and still have a bug if the specification itself is wrong. Mitigation: spec agent proposes test vectors derived from the spec as a human-legible sanity check.
- **Zero training data problem:** A new language has no training data for LLMs. The `--help`-as-skill system and self-describing modules partially address this, but bootstrapping agent capability is a real challenge.
- **Lean proof generation capability:** Current LLMs are mediocre at Lean proofs. Practical starting point: agents generate Vow source + implementation, specialized proof search tools (LeanDojo/ReProver) fill in proof obligations, agents retry with different implementations if proof search fails.
- **Incremental spec evolution:** When requirements change, the entire vow set may need revision. The spec agent needs to understand diffs to vow sets, not just generate from scratch.
- **IR design:** The shared IR should facilitate both native codegen and verification. Pizlo-style instruction-value uniformity, explicit effect tracking, and SSA form with Phi/Upsilon nodes are promising directions. The IR should make mechanical reasoning straightforward.
- **Bidirectional syntax:** Should Vow support both a machine-optimized and human-readable concrete syntax with lossless transformation between them? Early designs explored this, but the "single way to do it" principle may argue for one canonical form with good tooling for human readability.

## 14. Influences and References

- **Filip Pizlo's B3/Air IR design** — instruction-value uniformity, explicit semantics, compositional structure, uniform representations
- **Armin Ronacher** — agent-optimized language features: context-visible types, explicit effect markers, diff-stable formatting, strong local reasoning
- **Lean 4** — target for formal proofs, Mathlib for structured mathematical definitions, powerful simplification engine
- **ESBMC** — bounded model checking for verification, counterexample-guided repair
- **MoonBit** — approaches to type systems and agent-friendly design
- **Rust** — ownership model inspiration (simplified via regions), `Result`/`Option` patterns, trait system (without implicit resolution)
- **Racket/Eiffel** — contract systems and blame tracking
- **FEX-emu** — practical compiler and DBT experience informing systems-level design decisions

---

*This document captures the current state of design thinking as of February 2026. It is a living sketch, not a specification.*

