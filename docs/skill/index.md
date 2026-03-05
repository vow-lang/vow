# Vow Language Skill Document

Vow is a systems programming language with built-in contracts (preconditions, postconditions, loop invariants) that are statically verified by ESBMC bounded model checking. Programs compile to native executables via Cranelift. The compiler emits structured JSON for machine consumption.

## What Vow Excludes

No comments, no generics, no traits, no closures, no macros, no garbage collection.

## CEGIS Workflow

The standard workflow for writing verified Vow programs:

1. **Write** — Create a `.vow` file with function contracts (`requires`, `ensures`, `invariant`)
2. **Build** — Run `vow build <file.vow>`
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

Build and run:
```
$ vow build hello.vow
$ ./hello
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
