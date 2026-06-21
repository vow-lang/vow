# Vow

**Vow is a systems programming language for AI agents.** Its defining feature is
built-in contracts — preconditions (`requires`), postconditions (`ensures`), and loop
invariants (`invariant`) — that are **statically verified at compile time** by ESBMC
bounded model checking. Programs compile to native executables through Cranelift.

Agents generate code, and generated code has bugs. Vow closes the loop: the compiler
either *proves* the correctness properties you declare, or returns a structured
counterexample the agent uses to fix the code — automatically. This is the **CEGIS**
workflow (counterexample-guided inductive synthesis): write, verify, fix, repeat,
until the program is proven correct.

```vow
module Divide

fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
} {
    x / y
}

fn main() -> i32 [io] {
    print_i64(divide(10, 2));   // 5
    0
}
```

```console
$ ulimit -v 2000000; build/vowc build divide.vow
{"status":"Verified","executable":"divide", ...}
```

## What makes Vow different

- **Contracts are first-class.** Every function can carry `requires`/`ensures`; every
  loop can carry `invariant`. They are fed to a bounded model checker, not treated as
  comments or runtime assertions.
- **Structured output everywhere.** The compiler emits JSON diagnostics,
  counterexamples, and build results — designed to be parsed, not just read.
- **Blame semantics.** When a contract fails, the diagnostic says whether the *caller*
  or the *callee* is at fault (`requires` → caller, `ensures` → callee).
- **Effects are explicit.** Pure functions have empty effect sets; calling an
  effectful function from a pure context is a type error.
- **Linear types.** `linear struct` values must be consumed exactly once.
- **No hidden complexity.** No generics, traits, closures, macros, or garbage
  collection. The language is intentionally small to keep the verification surface
  tractable.

## Where to go next

<div class="grid cards" markdown>

- :material-rocket-launch: **[Tutorial](tutorial/index.md)**
  Install the toolchain, write your first verified program, and learn the CEGIS loop.

- :material-book-open-variant: **[Language reference](reference/index.md)**
  Grammar, types, effects, contracts, the CLI, and the diagnostic catalog.

- :material-package-variant: **[Standard library](stdlib.md)**
  Reusable, contract-annotated modules: math, heaps, stack, geometry, bignum, gc.

</div>

!!! note "Documentation source of truth"
    The Language and Standard Library sections are rendered directly from the
    canonical specification in [`docs/spec/`](https://github.com/vow-lang/vow/tree/main/docs/spec)
    — the same files the compiler embeds into its agent skill. They are written
    *agent-first* (precise and table-dense); the Tutorial is the human-first
    on-ramp.
