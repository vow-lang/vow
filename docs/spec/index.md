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

<!-- OMIT-FROM-SKILL-START -->
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
<!-- OMIT-FROM-SKILL-END -->
