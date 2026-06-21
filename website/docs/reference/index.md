# Language reference

This section is the authoritative reference for Vow's syntax, semantics, contracts,
and command-line interface. The pages are generated verbatim from the canonical
specification in [`docs/spec/`](https://github.com/vow-lang/vow/tree/main/docs/spec)
— the same source the compiler embeds into its agent skill — so they never drift from
the compiler.

| Page | Covers |
|------|--------|
| [Grammar & syntax](grammar.md) | Types, operators, control flow, structs, enums, `match`, modules, methods, effects, builtins |
| [Contracts & verification](contracts.md) | `vow` blocks, `requires`/`ensures`/`invariant`, blame semantics, verification patterns and anti-patterns |
| [Contract methodology](contracts-methodology.md) | Which properties to prove: the contract taxonomy and how to write tight specifications |
| [CLI reference](cli.md) | `vow build` / `vow verify`, flags, JSON output schemas, exit codes |
| [Diagnostics](errors.md) | Every diagnostic error code and how to fix it |
| [Worked examples](examples.md) | Full CEGIS cycles with complete JSON output |

New to Vow? Start with the **[Tutorial](../tutorial/index.md)** for a guided
introduction, then return here for the details.
