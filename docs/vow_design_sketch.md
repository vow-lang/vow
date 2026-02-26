# Vow Language Design Sketch — v3 (26.02.2026)

## 1. Vision

**Vow** is a programming language for an agent-first development paradigm: AI agents write, verify, and debug code while humans specify intent and run the final executables. The name comes from the central language construct — a **vow block** — which is the contract between human and machine. The human understands it because it's in plain English with formal backing; the machine satisfies it because the verification backend (ESBMC) proves it so.

**Motto: "There's a single way to do it."**

This principle drives every design decision. Ambiguity is the enemy. Flexibility is a cost, not a feature. The language is rigid by design so that agents can reason about it mechanically and humans can trust the output.

**Guiding constraint: every feature must be verifiable by ESBMC.** If a language feature makes bounded model checking harder without making the language more expressive for its target use case (self-hosting a compiler), the feature is excluded. Language complexity is not free — it is paid for in verification tractability, toolchain complexity, and agent reasoning difficulty.

## 2. Core Design Principles

- **Canonical syntax enforced by the compiler.** No formatters, no style debates. The compiler includes a canonicalizer — there is exactly one surface representation for any given AST. This gives diff-stability for free and eliminates "was this a meaningful change or just reformatting?" ambiguity.
- **No comments.** Intent is captured by contracts and specifications. API documentation is the type signature plus contracts plus the `--help`-as-skill system. If humans need to communicate meta-instructions to agents ("I want this to be fast, not just correct"), those are structured annotations, not free-text comments.
- **Explicit semantics over implicit conventions.** No implicit conversions, no context-dependent parsing, no operator overloading. Every syntactic construct has exactly one semantic interpretation.
- **Compositional and modular.** Small, orthogonal constructs that combine predictably. Clear interfaces with explicit contracts. Local reasoning without global context requirements.
- **Verification as a first-class citizen.** Specifications are part of the syntax. Verification conditions are automatically generated. Proof obligations guide code generation.
- **Rust-like syntax for grounding.** The surface syntax borrows from Rust wherever Vow's principles don't force a divergence. This gives agents existing training data to leverage and gives human developers (especially C++ and Rust programmers) a familiar reading experience.

## 3. The Vow Block

The central language construct. Every module declares its vows — preconditions, postconditions, invariants — in a structured block that serves as both human-readable documentation and machine-checkable specification.

```
module PathFilter

vow {
    requires: valid_utf8(input)
    ensures:  valid_path(output)
    invariant: forall p in output, matches(p, pattern)
}
```

Vows have two lives:
- **At build time:** translated to ESBMC verification conditions and formally verified via bounded model checking.
- **In debug mode:** compiled to runtime assertions that catch bugs.

When a vow is violated at runtime, the diagnostic is structured and precise: which vow broke, at which program point, with which inputs, and which side of a module boundary is to blame.

### Vow blocks on functions

```
fn divide(x: i64, y: i64) -> i64 {
    vow {
        requires: y != 0
        ensures:  result == x / y
    }
    x / y
}
```

`result` is a keyword inside `ensures` that refers to the return value. `requires` violations blame the caller. `ensures` violations blame the callee.

### Refinement types as sugar

```
fn divide(x: i64, y: i64 where y != 0) -> i64 {
    x / y
}
```

`where` clauses on parameters desugar to `requires` in the canonical AST. The canonicalizer emits `where` for parameter-level refinements and `vow {}` for anything involving `ensures` or `invariant`. Both forms exist in surface syntax but there is one canonical AST representation.

### Loop invariants

```
while low < high {
    vow { invariant: low <= high && high <= len(xs) }
    ...
}
```

Invariants are checked at loop entry and preservation (back-edge). ESBMC verifies them as inductive invariants.

## 4. Type System

### Base types: simple and decidable

- **Primitive types:** `i8`, `i16`, `i32`, `i64`, `i128`, `u8`, `u16`, `u32`, `u64`, `u128`, `f32`, `f64`, `bool`, `usize`
- **Algebraic data types:** structs (product types) and enums (sum types)
- **Built-in parameterized types:** `Option<T>`, `Result<T, E>`, `Vec<T>`, `HashMap<K, V>`, `String` — compiler primitives with known, fixed semantics
- **No user-defined generics** (see §4.1)
- **No subtyping** (subtyping creates ambiguity that hurts agents)
- **No inheritance** (no object hierarchy of any kind)
- **No traits** (see §4.2)

The base type system stays decidable and fast. The agent can always know whether code type-checks without running the verifier.

### 4.1. Monomorphic by design — no generics

Vow deliberately excludes user-defined generics (parametric polymorphism on functions and types). This is not a deferred feature — it is a design decision that follows directly from the agent-first paradigm.

**Generics solve a human problem.** Parametric polymorphism exists so that human programmers don't have to write the same algorithm N times for N types. Agents have no such constraint. An agent can stamp out `binary_search_i64`, `binary_search_u32`, and `binary_search_string` in milliseconds. DRY is a human ergonomic principle, not a machine one.

**Monomorphic code is what the verification backends want.** ESBMC does bounded model checking on concrete types with concrete inputs. You cannot bounded-model-check `search<T: Ord>` — you check `search_i64` with specific values. Generics would be an indirection introduced only to be stripped away during lowering.

**Generics add substantial toolchain complexity for zero payoff.** A generics system requires unification, substitution, trait bound checking, instantiation logic, monomorphization, and generic-aware error diagnostics. Every one of these is a source of compiler bugs. None of them improve the final executable, which is monomorphic either way.

**The spec layer can still be abstract.** A module-level vow can express "this module provides sorted search for any ordered type" as a human-facing contract. The ESBMC model backing the spec can use parametric reasoning for some properties. The *implementations* are concrete per type, each verified against the spec.

**Built-in parameterized types are not generics.** `Option<i64>`, `Result<Bytes, IoError>`, `Vec<String>`, and `HashMap<String, i64>` are compiler-provided type constructors with known, fixed semantics. The agent writes concrete instantiations of these; it does not define new parameterized types.

### 4.2. No traits — concrete dispatch only

Vow deliberately excludes traits (interfaces, abstract base classes, typeclasses). This is a verification-driven decision.

**Traits introduce dispatch ambiguity.** Even with static dispatch, trait method resolution requires the compiler to determine which `impl` applies at each call site. This is a source of compiler complexity and a source of ambiguity for agents.

**Dynamic dispatch is a verification hazard.** `dyn Trait` means the verifier cannot know which function is called. ESBMC would have to reason about all possible implementations — a combinatorial explosion. Even static dispatch through traits requires vtable layout, method lookup tables, and coherence checking.

**The compiler doesn't need traits.** Every place a Rust compiler uses a trait (Backend, Emitter, Iterator, Hash, Eq, Ord), the Vow compiler uses concrete types and explicit functions:

```
// Instead of: impl Backend for CraneliftBackend { fn compile(...) }
// Just:
fn cranelift_compile(backend: &CraneliftBackend, module: &Module) -> Result<CompiledObject, CodegenError> [IO] {
    ...
}
```

The driver calls the right function based on configuration. An `if` statement, not a dispatch table.

**Hash, Eq, Ord are compiler-known operations on known types.** The compiler knows how to hash `i32`, `i64`, `usize`, and `String`. No trait resolution needed — just a built-in function per type.

### 4.3. No closures

Closures capture variables from their environment, which means the verifier must reason about captured state. A closure that captures a mutable reference creates aliasing — the hardest thing for any verifier. ESBMC is a first-order bounded model checker; it does not handle higher-order functions well.

**While loops replace closures everywhere.** Instead of `vec.iter().map(|x| x + 1)`:

```
let mut i: usize = 0;
let mut result = Vec::new();
while i < vec.len() {
    result.push(vec[i] + 1);
    i = i + 1;
}
```

This is verifiable. ESBMC can reason about the loop with an invariant. An agent doesn't care about the verbosity.

### Refinement types for specifications

On top of the base types, refinement predicates are checked by ESBMC, not the type checker:

```
fn divide(x: i64, y: i64 where y != 0) -> i64
type NonZero = { x: i64 | x != 0 }
type ValidPort = { p: u16 | p >= 1 && p <= 65535 }
```

This separation is key: types are decidable and mechanical; refinements are expressive and handled by ESBMC. Most of the expressiveness of dependent types without the complexity.

### Deliberately excluded

- **Generics / parametric polymorphism.** Agents generate monomorphic specializations (see §4.1).
- **Traits / interfaces / typeclasses.** Concrete functions on concrete types (see §4.2).
- **Closures / lambdas / higher-order functions.** While loops with explicit state (see §4.3).
- **Full dependent types in surface syntax.** Too complex for agents and verifiers.
- **Null.** `Option<T>` instead.
- **Exceptions.** `Result<T, E>` everywhere with `?` propagation.
- **Operator overloading.** Operators have fixed meaning per type.
- **Macros.** Macros destroy local reasoning. Agents generate code directly.
- **Async/await.** Concurrency modeled through the effect system, not language syntax (deferred).

## 5. Syntax Overview

### Rust as baseline, diverging only where principles demand it

| Feature | Rust | Vow | Reason for divergence |
|---------|------|-----|----------------------|
| Comments | `//` and `/* */` | None | Intent lives in vow blocks |
| Formatting | rustfmt (convention) | Canonicalizer (compiler pass) | Single representation |
| Error handling | `Result<T,E>` + `?` | Same | No divergence |
| Null | `Option<T>` | Same | No divergence |
| Traits | `impl Trait for Type` | None | Verification hazard (§4.2) |
| Closures | `\|x\| expr` | None | Verification hazard (§4.3) |
| Lifetimes | `'a`, `'static` | Regions (lexical) | Simpler for verification |
| Mutability | `let mut` | `let mut` | No divergence |
| Effects | Not in language | `[Read, Write]` on fn sig | First-class effect tracking |
| Contracts | Not in language | `vow {}` blocks | Central construct |
| Integer `+` | Panics in debug, wraps in release | Wrapping (hardware) | Unambiguous |
| Integer `+!` | N/A | Checked → `Option<T>` | Unambiguous |
| Macros | `macro_rules!`, proc macros | None | Destroys local reasoning |
| Generics | `<T: Bound>` | None | §4.1 |
| Operator overloading | Via traits | None | Ambiguity |

### Integer arithmetic: two operators, two meanings

```
a + b       // wrapping (hardware two's complement)
a +! b      // checked → Option<T>, forces caller to handle overflow
a - b       // wrapping
a -! b      // checked → Option<T>
a * b       // wrapping
a *! b      // checked → Option<T>
a / b       // traps on division by zero (hardware behavior)
a /! b      // checked → Option<T> (None on zero divisor)
a % b       // traps on zero
a %! b      // checked → Option<T>
```

The `!` reads as "watch out, this checks." Floats use standard operators (`+`, `-`, `*`, `/`) with IEEE 754 semantics — unambiguous by specification.

**Saturating arithmetic is not in the language.** It is a domain-specific behavior available as a standard library function call. The language provides what the hardware provides (wrapping) plus a safe alternative (checked). Everything else is a library concern.

### Comparison and boolean operators

```
a == b      a != b      a < b       a > b       a <= b      a >= b
a && b      a || b      !a
```

These have fixed semantics. No overloading.

### Struct and enum syntax

```
struct Point {
    x: f64,
    y: f64,
}

enum Shape {
    Circle { radius: f64 },
    Rect { width: f64, height: f64 },
}
```

### Pattern matching (exhaustive)

```
fn area(shape: Shape) -> f64 {
    match shape {
        Shape::Circle { radius } => 3.14159 * radius * radius,
        Shape::Rect { width, height } => width * height,
    }
}
```

The compiler rejects non-exhaustive matches. `::` for enum variant paths (Rust convention, maximizes agent training data compatibility).

### Control flow

```
let x = if cond { 1 } else { 2 };          // if/else is an expression

while cond { body; }                         // while loop

let result = loop {                          // loop with break-value
    if done { break value; }
};
```

No `for` loops. Use `while` with an index. Iterators require traits, which Vow excludes.

### Functions, effects, and vow blocks

```
fn pure_add(x: i32, y: i32) -> i32 {       // pure (no effects)
    x + y
}

fn read_config(path: &str) -> Result<Config, IoError> [Read] {
    vow {
        requires: path.len() > 0
        ensures: result.is_ok() || result.is_err()
    }
    let bytes = fs::read(path)?;
    Config::parse(bytes)
}
```

A function with no effect annotation is pure. The compiler enforces: calling a `[Read]` function from a pure function is a type error.

### Annotations (replacing comments)

```
@[optimize(speed)]
fn hot_loop(data: &[u8]) -> u64 { ... }

@[deprecated(since = "0.2.0", use_instead = "new_api")]
pub fn old_api() -> () { ... }
```

Annotations are structured data. The compiler knows every valid annotation. No arbitrary strings.

### Visibility

- `pub` — visible outside the module
- No annotation — module-private

Two levels. No `pub(crate)`, no `pub(super)`.

## 6. Effect System

Every function declares its effects in its type signature. A function with no effect annotation is pure — the compiler enforces this.

```
fn pure_transform(data: Bytes) -> Bytes { ... }                    // pure
fn read_entry(path: &str) -> Result<FileEntry, IoError> [Read] { ... }
fn write_output(data: &[u8]) -> Result<(), IoError> [Write] { ... }
fn copy_file(src: &str, dst: &str) -> Result<(), IoError> [Read, Write] { ... }
```

### Effect set

- `Pure` (no annotation) — no side effects, deterministic
- `[Read]` — reads from filesystem, network, etc.
- `[Write]` — writes to filesystem, network, etc.
- `[IO]` — sugar for `[Read, Write]`
- `[Panic]` — can panic (functions that call `.unwrap()` etc.)
- `[Unsafe]` — contains unsafe operations, excluded from verification

### Effect propagation

The compiler checks: if you call a `[Read]` function, your function must declare `[Read]`. This is a type error, not a warning.

### Vow block predicates must be pure

Expressions inside `requires`, `ensures`, and `invariant` clauses cannot have effects. This ensures that checking a contract never causes side effects.

The effect system enables:
- **Complete effect traces at runtime** — the runtime knows exactly what operations are permitted.
- **Total effect mediation** — every interaction with the outside world goes through the effect system.
- **Deterministic replay** — all non-determinism is captured in an effect journal.

## 7. Memory Management

**Region-based allocation with compiler-assisted placement.** No garbage collector, no Rust-style lifetime annotations.

### Arena-per-scope (MVP)

The initial memory model is simple: every function has an implicit arena. Allocations go to this arena. On function return, the arena is freed (except values that escape via the return value, which are allocated in the caller's arena).

```
fn process() -> Vec<u8> [Read] {
    let temp = Vec::new();              // allocated in this function's arena
    temp.push(1);
    temp.push(2);
    let result = transform(&temp);      // result allocated in caller's arena
    result
}                                        // temp's arena freed here
```

### Future: explicit regions

The language supports named regions as a future extension:

```
fn process() -> Vec<u8> [Read] {
    region scratch {
        let temp = Vec::new_in(scratch);
        ...
    }   // scratch freed here
}
```

### Linear types for resource management

A value of linear type must be used exactly once:

```
linear struct FileHandle {
    fd: i32,
}

fn open(path: &str) -> Result<FileHandle, IoError> [Read] { ... }
fn read(handle: &FileHandle) -> Result<Vec<u8>, IoError> [Read] { ... }
fn close(handle: FileHandle) -> Result<(), IoError> [IO] { ... }
```

`close` consumes the `FileHandle`. After `close(handle)`, using `handle` is a compile error. `read` borrows it (`&FileHandle`). Forgetting to close is also a compile error — a linear value must be consumed.

### Spectrum of control

- **Default:** compiler-chosen arena placement ("just make it work").
- **Optimized:** agent-chosen regions with verified safety ("make it fast").

## 8. Number Tower

Minimal at the language level. Everything maps to hardware.

### Language-level (fixed-width)

- Integers: `i8`, `i16`, `i32`, `i64`, `i128`, `u8`, `u16`, `u32`, `u64`, `u128`
- Floats: `f32`, `f64` (IEEE 754)

### Overflow semantics: two modes, operator-encoded

- `+` `-` `*` — wrapping (hardware two's complement). Silent, deterministic.
- `+!` `-!` `*!` — checked, returns `Option<T>`. Overflow produces `None`.
- `/` `%` — traps on zero divisor (hardware behavior).
- `/!` `%!` — checked, returns `Option<T>`. Zero divisor produces `None`.

No undefined behavior. No mode-dependent semantics. No saturating arithmetic in the language (available in stdlib).

### Standard library (not language-level)

- Bigints, rationals, decimals, complex numbers, saturating arithmetic
- Each can be domain-optimized (crypto bigints ≠ arbitrary-precision bigints)

## 9. Built-in Parameterized Types

These are compiler primitives — not user-defined generics. The compiler knows their layout, their methods, and how to lower them to IR.

### Option\<T\>

```
enum Option<T> { Some(T), None }
```

Compiler knows: `?` operator desugaring, `.unwrap()` semantics, exhaustive matching.

### Result\<T, E\>

```
enum Result<T, E> { Ok(T), Err(E) }
```

Compiler knows: `?` operator desugaring, `.unwrap()` semantics, exhaustive matching.

### Vec\<T\>

Growable array backed by arena allocation.

```
let mut v = Vec::new();
v.push(42);
let x = v[0];              // bounds-checked, panics on OOB
let y = v.get(0);          // returns Option<&T>
```

Layout: `{ ptr: *T, len: usize, capacity: usize, region: RegionId }`. Iteration via while loop with index — no iterators, no closures.

### HashMap\<K, V\>

Hash map backed by arena allocation. Open addressing with linear probing.

```
let mut m = HashMap::new();
m.insert("key", 42);
let v = m.get("key");      // returns Option<&V>
```

Key types: the compiler knows how to hash `i32`, `i64`, `usize`, `String`. No `Hash` trait — just built-in hash functions for known types.

### String

UTF-8 string backed by `Vec<u8>`.

```
let s = String::from("hello");
let len = s.len();
let slice: &str = s.slice(0, 3);
```

`&str` is a borrowed view: `{ ptr: *u8, len: usize }`.

### Boundary

The set of built-in parameterized types is: `Option`, `Result`, `Vec`, `HashMap`, `String`. If this set grows substantially, the design has failed — it would be generics by another name. New collection types go in the standard library as concrete types (e.g., `TreeMapStringI64`).

## 10. Toolchain Architecture

The toolchain is a **unified system**, not a loose collection of independent tools.

### Implementation

- **Stage 0 compiler:** Written in Rust.
- **Code generation backend:** Cranelift (not LLVM). Cranelift is pure Rust (no C++ dependency), has no `poison`/`undef` semantics, and wrapping arithmetic is the default — matching Vow's semantics. Faster compile times than LLVM, simpler API, good enough codegen for x86-64 and AArch64.
- **Verification backend:** ESBMC for bounded model checking. Lean integration deferred — may be revisited or reimplemented in Vow later.

### Why Cranelift over LLVM

- No undefined behavior in Cranelift's IR (CLIF). LLVM's optimization model relies on `poison`, `undef`, and UB-based folding — fundamentally at odds with Vow's "no UB" principle.
- Pure Rust dependency. No CMake-driven LLVM builds, no llvm-sys linking, no version pinning.
- Simpler API. The lowering from Vow IR to CLIF is more straightforward.
- Self-hosting path: Cranelift's logic can eventually be ported to Vow or replaced with a simpler Vow-native backend. LLVM can never be rewritten in Vow.
- Cranelift's AArch64 and x86-64 backends are production-quality (used in Wasmtime).

### IR Design: Pizlo-style SSA

The internal IR follows Filip Pizlo's approach (from B3/JavaScriptCore):

**Instruction-value uniformity.** Every instruction is a value, every value is an instruction. Constants, function arguments, and Phi nodes are all just instructions with different opcodes. This eliminates special cases throughout the compiler.

**Phi/Upsilon form.** SSA Phi nodes are split into two instructions: Upsilon (writes to a shadow variable) and Phi (reads from it). Both are regular instructions that can appear anywhere in a block. This decouples the CFG from SSA form, making control flow transforms trivial.

**Arrays, not linked lists.** Basic blocks are `Vec<Inst>`, functions are `Vec<BasicBlock>`. Cache-friendly, simple to implement. The InsertionSet pattern (batch insertions during a forward pass, execute all at once) provides efficient O(n+k) transforms.

**Uniform effect representation.** Every instruction reports its effects as reads/writes to abstract heaps (Memory, SSAState, VowState, IO). Passes that reason about ordering never special-case opcodes — they compare effect sets.

**Vow obligations as IR instructions.** `VowRequires`, `VowEnsures`, and `VowInvariant` are opcodes like any other. They read the `VowState` abstract heap (preventing the optimizer from eliminating them), carry source location and blame metadata, and are consumed by both codegen (debug-mode traps) and verification (ESBMC assertions/assumptions).

### Pipeline

```
Vow Source → Parser → AST → Type/Effect Check → Vow IR
                                                    ├── Lowering → Cranelift CLIF → machine code
                                                    └── VC generation → ESBMC verification
```

Both paths share the same IR (read-only after lowering). Compilation and verification run in parallel. Errors from either path surface in a unified diagnostic stream.

### Self-describing tools (`--help` as skill)

Every tool in the ecosystem returns structured, machine-readable JSON via `--help`. An agent reads the skill document and knows how to use the tool — capabilities, expected inputs, effects, error modes. No training data required.

### Structured diagnostics

Error messages are structured data, not strings. Every diagnostic carries: severity, error code, primary span, secondary spans, blame target (caller/callee), and machine-parseable context. JSON output for agents and colored terminal output for humans are produced in parallel.

## 11. Debugging as Automated Diagnosis

Traditional source-level debuggers are the wrong tool for agentic debugging. An agent doesn't benefit from stepping through code — it can reason about the entire function at once.

### Execution traces over breakpoints

The runtime produces structured execution traces — a first-class facility where a failing execution yields a complete, machine-readable record of function calls, argument values, return values, state transitions, and effect logs.

Two compilation modes:
- **Development build:** fully instrumented, every function boundary and contract check.
- **Production build:** traces compiled out entirely, zero overhead.

### Differential diagnosis via contracts

The agent instruments every function boundary along the call path and asks: "which is the first function whose postcondition was violated while its precondition was satisfied?" Automatic fault localization — no human intuition required.

### Blame tracking across modules

When a contract is violated at a module boundary, the diagnostic identifies which side is at fault — the caller (violated `requires`) or the callee (violated `ensures`). This is carried in the IR as metadata on VowRequires/VowEnsures instructions.

### Counterexample-guided repair

Once the failing function is identified, the agent has the counterexample from ESBMC and the contract. This becomes a bounded synthesis problem: find a modification that satisfies the contract for this input *and* all previously passing inputs.

## 12. C Interoperability

C interop exists but is **contained behind a verification boundary**.

```
extern "C" {
    vow {
        requires: ptr != null && len > 0
        ensures:  result >= 0
    }
    fn write(fd: i32, ptr: *const u8, len: u64) -> i64 [Write];
}
```

- Every foreign call requires a mandatory contract specifying expected behavior.
- The wrapper is verified against that contract.
- The C code itself is opaque and untrusted.
- No implicit C header parsing, no automatic binding generation.

For self-hosting, the Cranelift API is wrapped in C-compatible shim functions (Rust `#[no_mangle] extern "C"` functions) and called through `extern` blocks with vow contracts. ESBMC is invoked as a subprocess.

## 13. Self-Hosting

The validation target: **Vow compiled by Vow, written by agents, verified by ESBMC.**

### What the compiler needs (the complete language)

The entire language feature set is driven by what's needed to express a compiler:

- **Structs and enums** — AST nodes, IR instructions, tokens, types
- **Option\<T\> and Result\<T, E\>** — error handling on every other line
- **Vec\<T\>** — token lists, AST children, instruction arrays
- **String and &str** — source code, identifiers, error messages
- **HashMap\<K, V\>** — symbol tables, type environments, scope maps
- **Arena-per-scope allocation** — heap types need a memory model
- **Pattern matching** — the compiler's primary control mechanism
- **While loops** — iteration without closures or iterators
- **Concrete functions** — no traits, no dynamic dispatch
- **File I/O through effects** — reading source, writing objects
- **Module system** — the compiler is multiple modules
- **C FFI** — calling Cranelift and invoking ESBMC
- **Vow blocks** — specifying what each compiler phase does

That's the language. Nothing more is needed.

### Bootstrap path

1. **Stage 0:** Compiler written in Rust, using Cranelift for codegen and ESBMC for verification.
2. **Stage 1:** Agent rewrites the compiler in Vow, module by module. Each phase is verified against the Rust reference implementation via differential testing and ESBMC verification of vow blocks. Port order: lexer → parser → type checker → IR/lowering → codegen wrapper.
3. **Stage 2:** Vow compiler compiles itself. Triple test: Stage 0 compiles Vow source → Binary A. Binary A compiles Vow source → Binary B. Binary B compiles Vow source → Binary C. Assert B == C (bit-for-bit).

### What self-hosting proves

- The language is expressive enough for systems programming — without closures, traits, or generics.
- Agents can write a verified compiler in a new language with no training data, guided by vow blocks and `--help` skills.
- The specification system works for the hardest case: a compiler for itself.

## 14. What Gets Inverted

| Traditional | Vow |
|---|---|
| Comments explain intent | Contracts capture intent; no comments |
| Formatters enforce style | Canonicalizer is a compiler pass |
| Rich FFI for ergonomic C interop | Thin verification boundary |
| Complex type systems for expressiveness | Simple types + refinements checked by ESBMC |
| Generics eliminate code duplication | Agent generates monomorphic specializations |
| Traits/interfaces for polymorphism | Concrete functions on concrete types |
| Closures for abstraction | While loops with explicit state |
| GC or manual memory management | Region-based with verified safety |
| Large number tower in language | Minimal hardware types; rest in stdlib |
| `+` with implicit overflow mode | `+` wraps, `+!` checks — operator encodes semantics |
| Interactive debugger | Automated diagnosis via traces and contracts |
| `--help` prints human text | `--help` returns machine-readable skill |
| Multiple ways to express the same thing | Single canonical form |
| Flexible syntax for human preference | Rigid syntax for mechanical reasoning |
| LLVM for code generation | Cranelift (no UB, pure Rust, self-hostable) |

## 15. Open Questions

- **Spec completeness:** How does the human know the vow set is sufficient? Mitigation: spec agent proposes test vectors derived from the spec as a human-legible sanity check.
- **Zero training data problem:** A new language has no training data for LLMs. The `--help`-as-skill system and self-describing modules partially address this. The agent capability test (Phase 6 task) will provide empirical data.
- **Incremental spec evolution:** When requirements change, the entire vow set may need revision. The spec agent needs to understand diffs to vow sets, not just generate from scratch.
- **Built-in parameterized type boundary:** The set is `Option`, `Result`, `Vec`, `HashMap`, `String`. If this grows, it's generics by another name. Resist expansion.
- **Region escape analysis:** When a value escapes a function via return, it must be allocated in the caller's arena. The compiler needs escape analysis to determine this. How precise does this need to be?
- **Recursive types and ESBMC bounds:** The compiler's AST types are recursive (expressions contain sub-expressions). ESBMC needs `--unwind` bounds. How do we choose good bounds automatically?
- **Lean integration (deferred):** Lean was part of the original vision for formal proofs beyond ESBMC's bounded model checking. This may be revisited, or Vow may eventually implement its own proof checker.
- **Concurrency model:** Not addressed yet. The effect system provides the foundation (`[Concurrent]` effect?), but the execution model is undefined.

## 16. Influences and References

- **Filip Pizlo's B3/Air IR design** — instruction-value uniformity, Phi/Upsilon form, array-based IR, uniform effect representation, InsertionSet pattern
- **Armin Ronacher** — agent-optimized language features: context-visible types, explicit effect markers, diff-stable formatting, strong local reasoning
- **ESBMC** — bounded model checking for verification, counterexample-guided repair
- **Cranelift** — code generation backend, no-UB IR semantics, pure Rust
- **MoonBit** — approaches to type systems and agent-friendly design
- **Rust** — surface syntax baseline, `Result`/`Option` patterns, ownership inspiration (simplified via regions)
- **Racket/Eiffel** — contract systems and blame tracking
- **FEX-emu** — practical compiler and DBT experience informing systems-level design decisions

## Appendix A. Design Decisions Log

### A.1. No generics (v2, 26.02.2026)

**Decision:** Vow does not support user-defined generics. Functions and types are monomorphic. Specifications may be parametric; implementations are concrete.

**Rationale:** Generics exist to prevent humans from writing repetitive code — a problem agents don't have. The verification backends operate on concrete types, so generics would be an indirection introduced only to be removed during lowering. Toolchain complexity of unification, substitution, and monomorphization is pure cost with no payoff.

**What this replaces:** v1 listed "Generics / parametric polymorphism" as a type system feature.

### A.2. Cranelift over LLVM (v3, 26.02.2026)

**Decision:** Use Cranelift as the code generation backend, not LLVM.

**Rationale:** LLVM's IR has undefined behavior semantics (`poison`, `undef`, `nsw`/`nuw`) that fundamentally conflict with Vow's "no UB" principle. Cranelift has no such concepts — wrapping arithmetic is the default, matching Vow's `+` operator semantics. Cranelift is pure Rust (no C++ dependency), simplifying the build. The self-hosting path is viable: Cranelift can be ported to Vow or replaced with a simpler Vow-native backend. LLVM can never be rewritten in Vow.

**What this replaces:** v2 mentioned LLVM as a potential backend. The architecture now uses Cranelift exclusively, with the LLVM option permanently closed.

### A.3. No traits (v3, 26.02.2026)

**Decision:** Vow does not support traits, interfaces, typeclasses, or any form of ad-hoc polymorphism. All dispatch is concrete.

**Rationale:** Traits introduce dispatch ambiguity, require method resolution logic, and (for dynamic dispatch) create a verification hazard — ESBMC cannot determine which function is called through a vtable. The self-hosting compiler needs no polymorphism. Every use of a Rust trait in the compiler maps to concrete functions on concrete types. The `Backend` "trait" is a struct with functions; the `Emitter` "trait" is two concrete functions (`emit_json`, `emit_human`). This eliminates trait definitions, impl blocks, method resolution, vtable layout, and coherence checking from the compiler — substantial complexity removed.

**What this replaces:** v2 mentioned "composition through traits/interfaces only" and "all trait implementations are explicit." Both are removed.

### A.4. No closures (v3, 26.02.2026)

**Decision:** Vow does not support closures, lambdas, or anonymous functions of any kind.

**Rationale:** Closures capture environment variables, creating implicit aliasing that is hard for ESBMC to reason about. ESBMC is a first-order bounded model checker — it does not handle higher-order functions well. Every use of closures in the compiler (iteration, mapping, filtering) maps to a while loop with an index variable. This is verbose for humans but trivial for agents, and fully verifiable.

**What this replaces:** v2 did not explicitly address closures. This makes the exclusion explicit.

### A.5. Arithmetic operators: `+` wrapping, `+!` checked (v3, 26.02.2026)

**Decision:** Integer `+` `-` `*` are wrapping (hardware two's complement). `+!` `-!` `*!` are checked, returning `Option<T>`. No saturating arithmetic in the language.

**Rationale:** Two modes, two operators, zero ambiguity. `+` does what the hardware does — silent wrap. `+!` forces the caller to handle overflow via `Option`. The `!` reads as "watch out, this checks." Saturating arithmetic is domain-specific (audio, pixel math) and lives in the standard library. Division `/` traps on zero (hardware behavior); `/!` returns `Option<T>`.

**What this replaces:** v2 listed three modes (wrapping, checked, saturating) without specifying operators. The operator syntax and the exclusion of saturating from the language are new.

---

*This document captures the current state of design thinking as of February 2026. It is a living sketch, not a specification.*
