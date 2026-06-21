# Tutorial

This is the human-first on-ramp to Vow. By the end you will be able to write a Vow
program, attach contracts to it, run the verifier, read a counterexample, and fix the
program until it is **proven correct** — the core loop Vow is built around.

## What you'll learn

1. **[Install & first program](getting-started.md)** — get the `build/vowc` compiler
   and compile a "hello, world".
2. **[Your first contract](first-contract.md)** — add a `requires` precondition and
   see blame semantics in action.
3. **[The CEGIS loop](cegis.md)** — let the verifier find a bug, read the structured
   counterexample, and fix it.
4. **[Loop invariants](loop-invariants.md)** — prove properties of code that loops.
5. **[Using the standard library](using-stdlib.md)** — pull a verified module into
   your own project.

## Prerequisites

- A Unix-like environment (Linux or macOS).
- A Rust toolchain (to build the bootstrap compiler the first time).
- [ESBMC](https://github.com/esbmc/esbmc) on your `PATH` for the verification steps.

!!! warning "Always cap memory"
    Every command below is prefixed with `ulimit -v 2000000`. The compiler and the
    binaries it produces can otherwise consume all system memory. Make it a habit.

Once you're comfortable, the **[Language reference](../reference/index.md)** has the
full grammar, contract semantics, and CLI details, and the
**[Standard library](../stdlib.md)** documents the reusable modules.
