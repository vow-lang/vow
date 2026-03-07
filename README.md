# Vow

An agent-first programming language with formal verification.

Vow programs carry machine-checked contracts (preconditions, postconditions, loop invariants) that are statically verified by [ESBMC](https://esbmc.org/) bounded model checking. The compiler emits structured JSON output designed for AI agents to consume, enabling a CEGIS (counterexample-guided inductive synthesis) workflow: write code → compile → verify → read counterexamples → fix → iterate.

For design details, see [docs/vow_design_sketch.md](docs/vow_design_sketch.md).

## Quick Start

```bash
cargo build --all --release
./target/release/vow build examples/divide.vow        # compile + verify
./target/release/vow verify examples/divide.vow        # verify contracts only
./target/release/vow build --mode debug examples/divide.vow  # runtime vow checks
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

The `compiler/` directory contains a complete Vow implementation of the compiler (13 modules). The bootstrap triple test passes: the self-hosted compiler is a verified fixed point producing byte-identical binaries.

## Project Status

- Phases 1–14: Complete (lexer, parser, type checker, IR, codegen, verification, self-hosting, contracts, CEGIS)
- Phase 15.1: Complete (vericoding benchmark suite — 36/36 non-Stretch references verified)
- Phase 15.2: Complete (benchmark runner CLI for running agents against the suite)
