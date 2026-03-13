# Vow

An agent-first programming language with formal verification.

Vow programs carry machine-checked contracts (preconditions, postconditions, loop invariants) that are statically verified by [ESBMC](https://esbmc.org/) bounded model checking. The compiler emits structured JSON output designed for AI agents to consume, enabling a CEGIS (counterexample-guided inductive synthesis) workflow: write code → compile → verify → read counterexamples → fix → iterate.

For design details, see [docs/vow_design_sketch.md](docs/vow_design_sketch.md).

## Quick Start

```bash
# Bootstrap (one-time)
cargo build --all --release
scripts/bootstrap.sh --no-verify

# Day-to-day usage
ulimit -v 2000000; ./vowc build examples/divide.vow              # compile + verify
ulimit -v 2000000; ./vowc verify examples/divide.vow              # verify contracts only
ulimit -v 2000000; ./vowc build --mode debug examples/divide.vow  # runtime vow checks
```

## Vericoding Benchmark Suite

The `benchmarks/` directory contains 40 formal verification benchmarks (15 Easy, 15 Medium, 10 Hard) for measuring how well AI agents can write verified code from natural-language specifications. This is Vow's implementation of the *vericoding* concept ([arXiv:2509.22908](https://arxiv.org/abs/2509.22908)).

The `bench/` directory contains a Python CLI tool that runs frontier LLMs against the suite:

```bash
cd bench
uv sync
uv run python run.py validate-references                             # verify all reference solutions
uv run python run.py run --model claude-sonnet-4-20250514 --benchmark E01  # single benchmark
uv run python run.py run --model claude-sonnet-4-20250514                  # full suite
uv run python run.py report                                          # generate comparison report
```

Results are compared against paper baselines: Dafny 82%, Verus/Rust 44%, Lean 27%.

## Self-Hosted Compiler

The `compiler/` directory contains a complete Vow implementation of the compiler (13 modules). `./vowc` is the primary compiler for day-to-day development — a verified fixed-point binary with full feature parity: subcommands, flags, structured diagnostics, verification pipeline, and parallel codegen+verify. The Rust compiler (`./target/release/vow`) serves only as the stage 0 bootstrap.

## Project Status

- Phases 1–14: Complete (lexer, parser, type checker, IR, codegen, verification, self-hosting, contracts, CEGIS)
- Phase 15: Complete (vericoding benchmark suite — 36/36 non-Stretch verified; benchmark runner CLI; 100% with Claude Sonnet)
- Phase 16: Complete (self-hosted vow contracts — debug mode, blame, VowViolation JSON)
- Phase 17: Complete (self-hosted diagnostics — structured types, JSON + human dual emitter, source spans)
- Phase 18: Complete (self-hosted verification pipeline — C emitter, ESBMC invocation, counterexample mapping)
- Phase 19: Complete (CLI parity — subcommands, flags, structured --help, parallel codegen+verify; 40/40 compat tests)
- Phase 20: Complete (switchover — full test suite 82/82, benchmark suite under self-hosted, bootstrap release, documentation)
