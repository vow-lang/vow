# Vow Language Reference

Vow is a systems programming language with built-in contracts (preconditions, postconditions, loop invariants) that are statically verified by ESBMC bounded model checking. Programs compile to native executables via Cranelift. The compiler emits structured JSON for machine consumption.

In all documentation below, `vow` refers to the `build/vowc` binary. Always use `ulimit -v 2000000` before invoking the compiler or any binary it produces — without this, the process can consume all system memory.

## What Vow Excludes

No block comments, no generics, no traits, no closures, no macros, no garbage collection. Line comments (`//`) are supported.

## CEGIS Workflow

The standard workflow for writing verified Vow programs:

1. **Write** — Create a `.vow` file with function contracts (`requires`, `ensures`, `invariant`)
2. **Build** — Run `ulimit -v 2000000; build/vowc build <file.vow>`
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

Build and run (`build/vowc` is the primary compiler binary, produced by `scripts/bootstrap.sh`):
```
$ ulimit -v 2000000; build/vowc build hello.vow
$ ulimit -v 2000000; ./hello
Hello, world!
```

## Development Discipline

Vow is written by agents under a finite context window. These principles apply to every task, not just language design.

**Deep modules, not shallow ones.** A deep module packs a lot of functionality behind a simple interface and hides complexity. A shallow module exposes a complex interface for not much functionality and surfaces complexity to every caller. Prefer deep. When you design or extend a module, ask: does the interface hide more complexity than it exposes? If not, collapse the module or move its logic behind a narrower boundary. Many exported symbols, thin wrappers, and "pass-through" functions are signs of a shallow module.

**Surgical changes.** Prefer many small changes over one large change. Small changes are easier to review, debug, bisect with `git bisect`, and revert. A bug fix fixes the bug — it does not also refactor the surrounding code, rename variables, or reformat the file. Unrelated improvements belong in their own commits or PRs. If a task grows, split it; do not bundle.

**Keep files small, functions smaller.** Context is precious. Every file an agent reads consumes budget that could go toward solving the problem. A 2000-line file forces whole-file reads and pushes other context out; a 200-line file does not. A 100-line function is harder to reason about than four 25-line functions with clear names. Split by responsibility as soon as a unit stops fitting a single coherent idea. This applies to both Vow source code and any tooling around it.

<!-- OMIT-FROM-SKILL-START -->
## Reference Files

| File                                              | Description                                        |
|---------------------------------------------------|----------------------------------------------------|
| [grammar.md](grammar.md)                          | Complete grammar: types, operators, effects, methods|
| [cli.md](cli.md)                                  | CLI commands, flags, JSON output schemas            |
| [errors.md](errors.md)                            | Error catalog: every error code with fix            |
| [contracts.md](contracts.md)                      | Contract patterns, verification, and anti-patterns  |
| [contracts-methodology.md](contracts-methodology.md) | Which properties to prove: contract taxonomy & strength |
| [examples.md](examples.md)                        | 3 worked CEGIS cycles with full JSON output         |
| [../dev/benchmarks.md](../dev/benchmarks.md)      | Developer benchmark harnesses outside the CLI spec  |
| [schemas/build-result.schema.json](schemas/build-result.schema.json)     | Build output JSON schema            |
| [schemas/diagnostic.schema.json](schemas/diagnostic.schema.json)         | Diagnostic JSON schema              |
| [schemas/counterexample.schema.json](schemas/counterexample.schema.json) | Counterexample JSON schema          |
| [schemas/vow-violation.schema.json](schemas/vow-violation.schema.json)   | Runtime violation JSON schema       |
| [schemas/mutants-result.schema.json](schemas/mutants-result.schema.json) | `vowc mutants` run/list output schema |
<!-- OMIT-FROM-SKILL-END -->
