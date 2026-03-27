# Vow Design Document

This document defines the vision, design goals, language boundaries, and toolchain architecture for Vow. It is the authoritative design document for the project.

This document is not yet a complete language specification or formal semantics document. It defines what the project is trying to build, what constraints govern feature admission, and how the language and tools are intended to work together. When implementation artifacts diverge from this document, the document should be treated as the intended direction and the code should be brought back into alignment.

Implementation maturity is called out explicitly:

- `Implemented`: available in the current toolchain.
- `Partial`: present but incomplete, provisional, or not yet carried through the full pipeline.
- `Target`: part of the intended design, not yet implemented end to end.

## 1. Project Vision

Vow is an agent-first programming language and toolchain for building software that can be written, repaired, and checked by AI agents and trusted by humans through formal verification.

The project thesis is straightforward:

- Agents are good at generating and transforming code.
- Humans need a stronger basis for trust than style, tests, or review alone.
- Formal contracts and mechanical verification are the scalable trust mechanism for agent-produced software.

Vow therefore treats verification as a first-class design constraint rather than a later tool integration. The language is intentionally narrow. It excludes features that make the compiler harder to reason about, the verifier harder to scale, or the behavior harder for agents to model mechanically.

The validation target for the project is ambitious and concrete: a Vow compiler, written in Vow, largely produced and maintained by agents, and verified by the same contract system and toolchain that Vow exposes to its users.

## 2. Design Goals

Vow is designed to satisfy the following project-level goals.

### 2.1. Verification is the primary trust mechanism

The language exists to support code that can be verified, not merely tested. If a feature materially harms verification tractability without being necessary for the target domain, it should not be in the language.

### 2.2. The language must be easy for agents to operate mechanically

The important property is not human terseness. The important property is that an agent can:

- infer what the code means from local structure,
- predict compiler and verifier behavior,
- produce code in a single preferred form,
- repair failures using structured feedback.

Verbosity is acceptable when it reduces ambiguity.

### 2.3. The surface language and the tools must form one system

Vow is not just a syntax and a compiler. The language, verifier, diagnostics, debug mode, and agent-facing tool interfaces are designed together. Tool behavior is part of the programming model.

### 2.4. Self-hosting is the language sufficiency test

The language should be just expressive enough to implement a serious systems toolchain, especially a compiler, without relying on escape hatches such as traits, macros, or user-defined generics.

### 2.5. Canonical form is more valuable than stylistic flexibility

Vow prefers a single canonical representation over multiple equivalent idioms. This reduces diff noise, simplifies synthesis and repair, and makes the output of multiple agents easier to compare mechanically.

## 3. Non-Goals

Vow is deliberately not trying to optimize for the following:

- maximal human ergonomics,
- broad language generality,
- multiple styles for the same concept,
- extensibility via macros or custom type-system features,
- dynamic abstraction mechanisms that complicate verification,
- preserving familiar mainstream-language features for their own sake.

In particular, features that mainly help humans avoid repetitive code are not automatically desirable in Vow. Agent-generated duplication is often a cheaper cost than expanding the language and verifier surface.

## 4. Governing Principles

The following principles are used to evaluate both language features and tooling decisions.

### 4.1. Single canonical way

Vow prefers one clear representation over several equivalent ones. A feature that introduces multiple ways to express the same semantics must justify itself against this rule.

Examples:

- the compiler canonicalizes source form,
- comments are limited to `//` rationale comments,
- contracts live in vow blocks rather than in multiple assertion systems,
- explicit effects are preferred over implicit behavior.

### 4.2. Explicit semantics over convenience

No implicit conversions, no operator overloading, no context-dependent meaning, and no hidden dispatch. Each construct should have one semantic interpretation.

### 4.3. Verification tractability gates feature admission

The primary question for a feature is not "is it expressive?" but "can it be modeled, checked, and diagnosed reliably?" Features that introduce higher-order behavior, ambiguous dispatch, or proof escape hatches are presumptively excluded.

### 4.4. Local reasoning beats global indirection

The language should favor modules, contracts, and concrete control flow over abstractions that require global lookup or non-local inference.

### 4.5. Tooling is part of the language contract

Structured diagnostics, machine-readable `--help`, debug traces, and counterexample reporting are not optional extras. They are part of the intended agent workflow.

## 5. Language Design

### 5.1. Core programming model

Vow is a modular, statically typed systems language with explicit effects and built-in contract syntax.

The central construct is the vow block. Vow blocks express:

- `requires` preconditions,
- `ensures` postconditions,
- `invariant` loop or module invariants.

At build time, these clauses are lowered into verification obligations for ESBMC. In debug mode, they are compiled into runtime checks. They also carry blame metadata so the system can distinguish caller violations from callee violations.

Function-level vow blocks are the current core contract mechanism.

```vow
fn divide(x: i64, y: i64) -> i64 {
    vow {
        requires: y != 0
        ensures: result == x / y
    }
    x / y
}
```

Within `ensures`, `result` refers to the function result. `requires` violations blame the caller. `ensures` violations blame the callee.

Module-level vow blocks are part of the intended design but not yet implemented.

Status:

- Function-level vow blocks: `Implemented`
- Loop invariants for simple predicates: `Implemented`
- Module-level vow blocks: `Target`
- Quantifiers such as `forall` and `exists`: `Target`

### 5.2. Surface syntax and source form

Vow uses Rust-like syntax where that choice does not conflict with the project principles. This gives agents and humans a familiar baseline while preserving Vow-specific constraints where necessary.

Key source-form decisions:

- The compiler enforces a canonical source form.
- `//` comments are allowed for non-semantic rationale only.
- `/* */` comments are excluded.
- `if` is an expression.
- `match` is exhaustive.
- `while` and `loop` are the only loop forms.
- `break` and `break value` are supported.
- `for` loops are excluded because they imply iterator abstractions that Vow does not want in the core language.

The language intentionally does not rely on formatters, style guides, or human convention to recover meaning. The source form itself is part of the mechanical interface.

### 5.3. Type system

The type system is intentionally small, nominal, and decidable.

#### Base categories

- Primitive integers: `i8`, `i16`, `i32`, `i64`, `i128`, `u8`, `u16`, `u32`, `u64`, `u128`
- Primitive floats: `f32`, `f64`
- Other primitives: `bool`, `usize`
- Algebraic data types: structs and enums
- References and tuples
- A small closed set of compiler-known intrinsic built-ins

The type checker should answer type questions quickly and mechanically. Richer logical constraints belong in contracts and refinements, not in the core type relation.

#### User-defined generics are excluded

Vow deliberately excludes user-defined generics on functions and types.

Rationale:

- Generics mainly solve a human code-reuse problem.
- Agents can cheaply generate monomorphic specializations.
- ESBMC verifies concrete programs, not abstract type parameters.
- Generics increase compiler complexity and proof plumbing without improving the trust model.

This is a design decision, not a deferred feature.

#### Traits, interfaces, and ad-hoc polymorphism are excluded

Vow does not provide traits, interfaces, typeclasses, or dynamic dispatch.

Rationale:

- Trait resolution introduces non-local reasoning.
- Dynamic dispatch is a poor fit for first-order bounded model checking.
- The compiler's own architecture can be expressed with concrete functions on concrete types.

#### Closures and higher-order functions are excluded

Vow does not provide closures, lambdas, or higher-order collection APIs in the core language.

Rationale:

- closure capture introduces implicit state,
- mutable capture creates aliasing pressure,
- ESBMC is a poor fit for higher-order reasoning,
- explicit loops are easier for agents and verifiers to model.

#### Refinement properties live above the base type system

Vow supports refinement-like surface syntax, but the design keeps a strong separation:

- the base type system remains decidable and syntactic,
- logical predicates are carried through contracts and verifier obligations.

Parameter `where` clauses are treated as sugar for `requires`.

```vow
fn divide(x: i64, y: i64 where y != 0) -> i64 {
    x / y
}
```

Status:

- Parameter `where` clauses lowered to `requires`: `Implemented`
- Refinement type syntax parsed but not fully forwarded to verification: `Partial`

### 5.4. Intrinsic built-ins

Vow allows a small closed set of compiler-known built-ins. This is a narrow exception to the otherwise monomorphic design.

The current intrinsic built-in set is:

- `Option<T>`
- `Result<T, E>`
- `Vec<T>`
- `HashMap<K, V>`
- `String`

These are not a general-purpose user extension mechanism. They are admitted only because the language needs a few foundational abstractions with fixed semantics:

- `Option<T>` replaces `null`,
- `Result<T, E>` replaces exceptions,
- `Vec<T>` is the single blessed growable sequence abstraction,
- `String` is a core systems type,
- `HashMap<K, V>` is useful for compiler construction but is the shakiest member of the set and must continue justifying its verification cost.

This boundary is important. If the intrinsic built-in set grows substantially, the project will have recreated user generics by another name.

### 5.5. Effect system

Every function has an explicit effect signature. A function with no effect annotation is pure.

Current effect vocabulary:

- `Pure` (implicit, no annotation)
- `[Read]`
- `[Write]`
- `[IO]` as sugar for `[Read, Write]`
- `[Panic]`
- `[Unsafe]`

The compiler checks effect propagation: a function that calls a `[Read]` function must itself admit `[Read]`, and so on.

Expressions inside vow clauses must be pure. Contract checking must not itself perform I/O or hidden state changes.

Status:

- Effect parsing and checking for user-defined functions: `Implemented`
- Builtin effect coverage such as panic-producing builtins: `Partial`

### 5.6. Memory and resource model

Vow's intended memory model is region-based allocation with compiler assistance rather than garbage collection or Rust-style lifetime syntax.

The design target is arena-per-scope allocation:

- each function has an implicit allocation region,
- temporary allocations die with the function or scope,
- escaping values are placed in an appropriate caller-visible region,
- the compiler is responsible for placement and escape analysis.

The language also intends to support linear types for resources that must be consumed exactly once, such as file handles or other external capabilities.

Status:

- Arena-per-scope memory model: `Target`
- Linear types as a resource discipline: `Target`
- Current runtime representation relies on explicit allocation rather than the full intended region model: `Partial`

### 5.7. Arithmetic and numeric model

Vow keeps the language-level number model close to hardware.

Language-level numeric types:

- fixed-width integers,
- `f32` and `f64` with IEEE 754 semantics.

Integer arithmetic is intentionally split into two explicit operator families:

- `+`, `-`, `*` are wrapping,
- `+!`, `-!`, `*!` are checked and return `Option<T>`,
- `/` and `%` trap on zero divisor,
- `/!` and `%!` return `Option<T>` on zero divisor.

There is no undefined behavior and no mode-dependent arithmetic semantics. Saturating arithmetic is considered a library concern, not a language concern.

### 5.8. C interoperability

Vow supports FFI, but only behind a verification boundary.

The intended model is:

- foreign declarations are explicit,
- foreign behavior is described by vow contracts,
- wrappers are checked against those contracts,
- foreign code remains opaque and untrusted,
- there is no automatic header import or broad binding generation in the core design.

Status:

- `extern` declarations: `Implemented`
- Mandatory contracts on foreign declarations: `Target`

### 5.9. Deliberate exclusions

The following remain outside the intended language:

- user-defined generics,
- traits, interfaces, typeclasses, dynamic dispatch,
- closures and higher-order function APIs,
- macros,
- operator overloading,
- subtyping and inheritance,
- exceptions,
- `null`,
- statement-level `assert` and `assume`,
- `async`/`await` in the core language,
- multiple visibility gradations such as `pub(crate)`.

These exclusions are not accidental omissions. They are design boundaries in service of verification tractability and local reasoning.

## 6. Toolchain Design

Vow is designed as a unified language-and-tool system. The compiler, verifier, diagnostics, and debug pipeline are part of one programming model.

### 6.1. Architecture

The current project architecture is:

- Stage 0 bootstrap compiler in Rust
- Primary code generation backend: Cranelift
- Primary verification backend: ESBMC
- Self-hosted Vow compiler as the long-term fixed point

Today `build/vowc` is the primary compiler for day-to-day development. The Rust compiler remains the bootstrap compiler and recovery path.

Cranelift is preferred over LLVM because it avoids LLVM's undefined-behavior-centered optimization model, has a simpler API surface, and is feasible to wrap or replace in a self-hosting path. ESBMC is used because bounded model checking aligns with the contract style and counterexample-driven repair workflow that Vow is built around.

### 6.2. Pipeline

The intended compilation pipeline is:

```text
Vow Source
  -> Parse
  -> AST
  -> Type and Effect Check
  -> Vow IR
       -> Lower to Cranelift CLIF -> machine code
       -> Lower verification conditions -> ESBMC
```

Compilation and verification should operate over a shared intermediate representation and surface errors through a unified diagnostic interface.

### 6.3. Intermediate representation

The internal IR follows a simple, optimizer-friendly SSA-style design inspired by Filip Pizlo's work on B3/Air.

The intended properties are:

- instruction/value uniformity,
- simple array-backed storage,
- Phi/Upsilon style SSA handling,
- uniform effect representation,
- explicit contract obligations in IR rather than out-of-band metadata.

Representing contract obligations directly in IR matters because both code generation and verification need to consume the same semantic object.

### 6.4. Diagnostics, debugging, and repair

Traditional source-level interactive debugging is not the primary design target for Vow. The intended debugging model is machine-assisted diagnosis.

Key pieces:

- structured diagnostics rather than ad hoc error strings,
- blame information on contract failures,
- debug-mode trace output for function and contract boundaries,
- ESBMC counterexamples as first-class repair inputs,
- fault localization by checking where preconditions held and postconditions first failed.

This design is meant to support a CEGIS-style workflow:

1. agent writes code,
2. compiler and verifier produce structured failure information,
3. agent repairs code against the counterexample and contract,
4. verification reruns.

Status:

- Structured diagnostics in the toolchain: `Implemented`
- Debug mode with basic contract and boundary events: `Partial`
- Full effect and state trace recording: `Target`

### 6.5. Agent-facing tooling

The toolchain is intended to expose its capabilities in machine-readable form. The design goal is that tools describe themselves to agents without requiring hidden training-set knowledge.

This implies:

- machine-readable `--help`,
- structured diagnostics and outputs,
- deterministic, diff-stable code formatting via canonicalization,
- stable error codes and blame categories,
- explicit command boundaries for build, verify, debug, and contract inspection.

The language and the tools should therefore be understandable as an operational skill surface, not merely as prose documentation.

## 7. Self-Hosting and Project Validation

Self-hosting is not only an implementation milestone; it is a design validation criterion.

The self-hosting target answers three questions:

- Is the language expressive enough to build a real systems compiler?
- Can agents work effectively in a new language with low prior training exposure?
- Does the contract and verification model scale to the project's own implementation?

The intended bootstrap path is:

1. bootstrap the language with a Rust compiler,
2. port the compiler into Vow module by module,
3. verify the Vow implementation against contracts and differential behavior,
4. reach a fixed point where the Vow compiler builds itself.

What self-hosting is expected to prove:

- the exclusion of traits, closures, and user-defined generics does not prevent serious systems programming,
- the language/tooling interface is sufficient for agent-driven development,
- the verification story is strong enough for the hardest in-project workload.

## 8. Current Implementation Status

This section records the current maturity of major design areas without changing the intended direction.

| Area | Status | Note |
|------|--------|------|
| Function-level vow blocks | Implemented | Current core contract mechanism |
| Loop invariants for simple predicates | Implemented | Present in the current verification pipeline |
| Parameter `where` refinements | Implemented | Lowered to `requires` |
| Module-level vow blocks | Target | Intended but not parsed or represented end to end |
| Quantifiers in contracts | Target | `forall` / `exists` not yet fully supported |
| Refinement type predicates in verification | Partial | Syntax exists, full semantic forwarding is incomplete |
| Effect propagation for user-defined functions | Implemented | Core effect checking works |
| Builtin panic/unsafe effect coverage | Partial | Not all builtins are yet modeled precisely |
| Arena-per-scope memory model | Target | Intended design direction |
| Linear resource types | Target | Part of the intended resource story |
| Debug mode contract and boundary traces | Partial | Available but not yet a full execution trace facility |
| Mandatory contracts on extern declarations | Target | FFI design boundary not yet fully enforced |

The design document is normative for direction. Experimental or leftover implementation artifacts that conflict with it should be treated as transition debt, not as permanent language commitments.

The project already has a working self-hosted compiler. The remaining `Target` items in this document are therefore about aligning the language and tooling with the intended design, not about reaching first compile.

## 9. Feature Admission Policy

New language or tooling features should be admitted only if they satisfy all of the following:

1. They materially improve verification, trust, or essential expressiveness for the target domain.
2. They can be modeled clearly in the compiler, IR, and verifier.
3. They do not create multiple equivalent ways to express the same semantics.
4. They preserve local reasoning for both humans and agents.
5. They fit the self-hosting objective rather than expanding the language toward general-purpose feature accumulation.

Questions that should reject a feature by default:

- Does this mainly reduce human typing rather than improve trust?
- Does this require hidden dispatch, global inference, or higher-order reasoning?
- Does this expand the intrinsic built-in set without a strong verification case?
- Does this create a new escape hatch around the verifier?

## 10. Open Questions

The following remain open at the design level:

- How should humans validate that a vow set is complete enough for the intended behavior?
- How far can ESBMC scale on recursive compiler data structures and long call chains without additional summarization or proof tooling?
- How precise does region escape analysis need to be for the compiler workload?
- Does `HashMap<K, V>` continue to justify its place in the intrinsic built-in set?
- What is the right concurrency model, if any, for Vow's long-term design?
- When, if ever, should the project add proof infrastructure beyond bounded model checking?
- How should the project formalize the boundary between authoritative design intent and implementation status as the language stabilizes?

## Appendix A. Design Summary by Inversion

The table below summarizes the core inversion that Vow makes relative to conventional language design.

| Conventional default | Vow position |
|---|---|
| comments carry intent | contracts carry intent; `//` carries rationale |
| formatting is convention | canonicalization is a compiler responsibility |
| verification is external | verification is part of the programming model |
| rich abstraction features reduce duplication | agents can generate monomorphic concrete code |
| debuggers are interactive and human-oriented | diagnosis is structured, blame-aware, and machine-oriented |
| FFI is ergonomic first | FFI is verification-bounded |
| type systems optimize expressiveness | type systems optimize decidability and verifier compatibility |

## Appendix B. References and Influences

The current design draws on the following sources and influences:

- ESBMC and bounded model checking,
- Cranelift as a no-UB code generation backend,
- Filip Pizlo's B3/Air IR design ideas,
- Rust for surface syntax and systems-language grounding,
- Eiffel and contract-oriented programming,
- work on agent-oriented and verifiable programming workflows,
- practical compiler and systems implementation experience from the Vow codebase itself.
