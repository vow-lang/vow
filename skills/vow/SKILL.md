---
name: vow
description: >-
  Write, compile, debug, and verify Vow programs (.vow files) with contracts,
  CEGIS, ESBMC counterexamples, diagnostics, and vow build / vow verify.
when_to_use: >-
  Use when the user edits or creates .vow files, says "write a Vow program",
  "fix this counterexample", "add contracts", "why did verification fail",
  "ESBMC", "vow build", or "vow verify".
argument-hint: "[file.vow]"
---

# Vow

Use this skill when writing, compiling, debugging, or verifying Vow programs.
Keep the workflow tight: run the compiler, read the structured JSON, fix the
program or contract, and repeat until the result is `Verified`.

## Do nots

- Do not weaken, cap, or distort a contract to make verification pass; fix the code or report that the proof is out of reach.
- Do not add verifier-motivated bounds (`n <= 10`, length or capacity caps) to `requires`; keep only real domain constraints and use checked operators (`+!`) for overflow.
- Do not use `--no-verify` to get past a real verification failure; it only skips the proof (status `Unverified`).
- Do not report a program as verified unless `status` is `Verified`; `Unverified` and `Skipped` are not success.
- Do not invent CLI flags, JSON fields, syntax, or builtins; check `--help` and the reference files first.
- Do not scrape terminal prose; parse the JSON object on stdout (`status`, `diagnostics`, `counterexamples`) and use `--human` only for people.

## Installed toolchain (live)

!`(command -v vow >/dev/null 2>&1 && vow --help 2>/dev/null | head -200) || (command -v build/vowc >/dev/null 2>&1 && build/vowc --help 2>/dev/null | head -200) || echo '(vow toolchain not found on PATH; run scripts/bootstrap.sh to build build/vowc)'`

## Core workflow

1. Write a `.vow` file with explicit contracts.
2. Run `build/vowc build <file.vow>`.
3. Parse stdout JSON and inspect `status`, `diagnostics`, and `counterexamples`.
4. Fix compile errors, verification failures, or weak contracts, then rerun.

## Minimal program

```vow
module Hello

fn main() -> i32 [io] {
    print_str("Hello, world!");
    0
}
```

## Reference files

- Grammar, types, effects, builtins: [reference/grammar.md](reference/grammar.md)
- CLI commands, flags, JSON output: [reference/cli.md](reference/cli.md)
- Contracts and CEGIS guidance: [reference/contracts.md](reference/contracts.md)
- Which contracts to write (taxonomy & strength): [reference/contracts-methodology.md](reference/contracts-methodology.md)
- Diagnostics and fixes: [reference/errors.md](reference/errors.md)
- Standard library (math, heap, stack, geometry, bignum, gc): [reference/stdlib.md](reference/stdlib.md)
- Worked examples: [examples/examples.md](examples/examples.md)
- JSON schemas: [schemas/](schemas/)

Outside Claude Code (raw API or custom agent harness that cannot load these files): run `build/vowc skill print --bundle` for one self-contained document with all of the above inlined.
