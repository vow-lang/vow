---
name: vow-toolchain
description: >-
  Write, compile, debug, and verify Vow programs (.vow files). Covers the
  CEGIS workflow (counterexample-guided inductive synthesis), contract
  authoring (requires, ensures, invariant), fixing VerifyFailed
  counterexamples, resolving CompileFailed diagnostics, loop invariants,
  the Vow effect system, and running vow build / vow verify. Use when the
  user says "write a Vow program", "fix this counterexample", "add
  contracts", "why did verification fail", "ESBMC", or "vow build".
globs: "**/*.vow"
---

# Vow Language Skill Document

Vow is a systems programming language with built-in contracts (preconditions, postconditions, loop invariants) that are statically verified by ESBMC bounded model checking. Programs compile to native executables via Cranelift. The compiler emits structured JSON for machine consumption.

In all documentation below, `vow` refers to the `./vowc` binary in the repository root. Always use `ulimit -v 2000000` before invoking the compiler or any binary it produces — without this, the process can consume all system memory.

## What Vow Excludes

No comments, no generics, no traits, no closures, no macros, no garbage collection.

## CEGIS Workflow

The standard workflow for writing verified Vow programs:

1. **Write** — Create a `.vow` file with function contracts (`requires`, `ensures`, `invariant`)
2. **Build** — Run `ulimit -v 2000000; ./vowc build <file.vow>`
3. **Parse JSON** — Read the JSON object from stdout
4. **Handle status:**
   - `Verified` → Done. Binary is at `executable`.
   - `Unverified` → Compiled but ESBMC not available. Binary is at `executable`.
   - `CompileFailed` → Read `diagnostics[]` for errors. Fix and retry.
   - `VerifyFailed` → Read `counterexamples[]`. Fix contracts or implementation. Retry.
5. **Iterate** — Repeat steps 2–4 until `Verified`.

## Minimal Program

```vow
module Hello

fn main() -> i32 [io] {
    print_str("Hello, world!");
    0
}
```

Build and run (`./vowc` is the primary compiler binary, produced by `scripts/bootstrap.sh`):
```
$ ulimit -v 2000000; ./vowc build hello.vow
$ ulimit -v 2000000; ./hello
Hello, world!
```

## Reference Files

| File                                              | Description                                        |
|---------------------------------------------------|----------------------------------------------------|
| [grammar.md](grammar.md)                          | Complete grammar: types, operators, effects, methods|
| [cli.md](cli.md)                                  | CLI commands, flags, JSON output schemas            |
| [errors.md](errors.md)                            | Error catalog: every error code with fix            |
| [contracts.md](contracts.md)                      | Contract patterns, verification, and anti-patterns  |
| [examples.md](examples.md)                        | 3 worked CEGIS cycles with full JSON output         |
| [schemas/build-result.schema.json](schemas/build-result.schema.json)     | Build output JSON schema            |
| [schemas/diagnostic.schema.json](schemas/diagnostic.schema.json)         | Diagnostic JSON schema              |
| [schemas/counterexample.schema.json](schemas/counterexample.schema.json) | Counterexample JSON schema          |
| [schemas/vow-violation.schema.json](schemas/vow-violation.schema.json)   | Runtime violation JSON schema       |
