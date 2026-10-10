# Vow

[![CI](https://github.com/vow-lang/vow/actions/workflows/ci.yml/badge.svg)](https://github.com/vow-lang/vow/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/vow-lang/vow/graph/badge.svg)](https://codecov.io/gh/vow-lang/vow)

An agent-first programming language with formal verification.

Vow programs carry machine-checked contracts (preconditions, postconditions, loop invariants) that are statically verified by [ESBMC](https://esbmc.org/) bounded model checking. The compiler emits structured JSON output designed for AI agents to consume, enabling a CEGIS (counterexample-guided inductive synthesis) workflow: write code → compile → verify → read counterexamples → fix → iterate.

For project design details, see [docs/vow_design.md](docs/vow_design.md).

## Quick Start

```bash
# Bootstrap (one-time)
cargo build --all --release
scripts/bootstrap.sh --skip-cargo

# Install the self-hosted toolchain to a user prefix (add --with-rust-compiler to also install vowr)
scripts/install-toolchain.sh --prefix "$HOME/.local"
export PATH="$HOME/.local/bin:$PATH"
vow build --no-verify examples/hello.vow -o /tmp/vow_hello

# Day-to-day usage
build/vowc build examples/divide.vow              # compile + verify
build/vowc verify examples/divide.vow              # verify contracts only
build/vowc build --mode debug examples/divide.vow  # runtime vow checks
```

### Verification prerequisites

The native verifier (epic #1398) shells out to `bitwuzla`, which must be on `PATH` at the pinned release alongside ESBMC. Today's `vowc verify` still runs on ESBMC alone, but CI already installs both. The release is pinned, with per-platform SHA-256 checksums, in `.github/actions/install-bitwuzla/action.yml`; CI installs it through that action. Prebuilt archives exist for Linux x86_64, Linux arm64 and macOS arm64 only. The binary links GMP and MPFR dynamically: on macOS run `brew install gmp mpfr` (it uses Homebrew's copies), and on Debian or Ubuntu run `apt install libgmp10 libmpfr6` if they are missing. Check the install with `bitwuzla --version`.

## Development checks

Install the pinned repository hooks once per checkout:

```bash
uv tool install pre-commit
pre-commit install
```

`pre-commit install` only wires up the hook types present in this repo's
config at the time you run it. If you installed hooks before the
`commit-msg` stage was added, rerun `pre-commit install` — otherwise your
checkout keeps linting file contents but silently skips the commit message
check.

Run every hook against the full repository when changing hook configuration or
tooling:

```bash
pre-commit run --all-files
```

## Standard Library

`stdlib/` holds reusable, contract-annotated Vow modules — `math` (arithmetic,
number theory, vector math), `heap` (min/max binary heaps), `stack`, `geometry`,
`bignum` (arbitrary-precision integers), and `gc` (mark-and-sweep). Each module is a
self-contained directory with a runnable `main.vow` demo.

Vow has no module search path yet, so you consume a module by running its demo in
place or copying its `.vow` file(s) into your project. Only `geometry` currently passes
`vow verify` — and that proves the vowed checks reachable from its demo, not the whole
API (`point_distance_sq` has no contract); the others carry precise contracts that are
enforced at runtime in `--mode debug` while static verifiability is improved. See
[stdlib/README.md](stdlib/README.md) (human map) and
[docs/spec/stdlib.md](docs/spec/stdlib.md) (full API + verification reference).

`examples/` keeps the language demos (contracts, blame, CEGIS, IO) and the larger
showcase (`sat/`). The chess engine showcase moved to
[vow-lang/chess](https://github.com/vow-lang/chess).

## Agent Setup (Claude Code Skill)

Vow ships a Claude Code skill embedded in the compiler binary. The skill is the
canonical reference an agent needs to author and fix Vow programs: grammar, CEGIS
workflow, contract authoring, CLI surface, error catalogue, and JSON schemas.

The skill is **the** source of truth for any harness writing Vow code. Because it
is generated from the same compiler version that builds your programs, it cannot
drift from the toolchain you are running.

### Inside Claude Code

The first time you run `vowc build` (or `vowc <source.vow>`) inside a project that
already has a `.claude/` directory, the compiler installs the skill at:

```
.claude/skills/vow/
```

Claude Code discovers it from the `.claude/skills/` directory and uses the
frontmatter description/`when_to_use` metadata to load it for `.vow` file work
as well as creation and verification-debugging prompts before a `.vow` file
exists. Auto-install is silent, runs at most once (it leaves any existing
`SKILL.md` untouched), and is skipped entirely when `.claude/` is absent — so
non-Claude-Code projects are never touched. Unlike explicit `--local`,
auto-install only requires `.claude/`; it does not require the directory to be a
git checkout.

To install the skill explicitly (for a fresh checkout, or when bringing a Vow
toolchain into a project that already has `.claude/`):

```bash
build/vowc skill install --local
```

`--local` requires the current directory to contain both `.git` and `.claude/`.
For a machine-wide install on Linux, use:

```bash
build/vowc skill install --global
```

The installed skill is split into `SKILL.md` plus `reference/`, `examples/`, and
`schemas/` support files. Commit the resulting `.claude/skills/vow/`
tree to your repository so collaborators (human and agent) get the same skill
version on checkout.

**Upgrading an existing install.** Auto-install never touches an existing
`.claude/skills/vow/SKILL.md`, so an older single-file skill keeps working but
does not gain the split-layout support files. Re-run
`build/vowc skill install --local` (project install) or `--global` (machine-wide
install), matching the install you are replacing, to migrate or refresh it; this
overwrites `SKILL.md` and the support files, so review the diff before
committing. In a project without `.git`, delete `.claude/skills/vow/SKILL.md` and
run a build instead. A leftover `.claude/skills/vow-toolchain/` directory from
before the rename is not touched and will sit next to the new `vow` skill —
delete it by hand.

### Outside Claude Code (raw API harnesses)

For any other harness — a custom agent loop, the bench runner, a one-off API call
— pipe the self-contained bundle into the system prompt at session start:

```bash
SYSTEM_PROMPT="$(build/vowc skill print --bundle)"
# ... feed $SYSTEM_PROMPT to your model along with the user task
```

The bundle is a single self-contained markdown document, so no further loading is
required for raw API harnesses. Loading once per session is enough; the skill
describes a workflow, not per-task state.

## Vericoding Benchmark Suite

The `benchmarks/` directory contains 40 formal verification benchmarks (15 Easy, 15 Medium, 10 Hard) for measuring how well AI agents can write verified code from natural-language specifications. This is Vow's implementation of the *vericoding* concept ([arXiv:2509.22908](https://arxiv.org/abs/2509.22908)).

The `bench/` directory contains a Python CLI tool that runs frontier LLMs against the suite:

```bash
cd bench
uv sync
uv run python run.py validate-references                             # verify all reference solutions
uv run python run.py run --model claude-sonnet-5-5 --benchmark E01  # single benchmark
uv run python run.py run --model claude-sonnet-5-5                  # full suite
uv run python run.py report                                          # generate comparison report
```

Results are compared against paper baselines: Dafny 82%, Verus/Rust 44%, Lean 27%.

## Self-Hosted Compiler

The `compiler/` directory contains a complete Vow implementation of the compiler (13 modules). `build/vowc` is the primary compiler for day-to-day development — a verified fixed-point binary with full feature parity: subcommands, flags, structured diagnostics, verification pipeline, and staged codegen then bounded-parallel verify. The Rust compiler (`./target/release/vow`) serves only as the stage 0 bootstrap.
