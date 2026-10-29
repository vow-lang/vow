# Vow repository guidance

Paths and commands below are relative to the repository root.

## Test Harnesses (`scripts/full_test.sh` vs `tests/run_tests.sh`)

Two separate scripts walk `tests/run/*.vow` and other `tests/*` directories, and they check
**different subsets** of the `// TEST:` directives. Do not assume one implies the other.

- **`scripts/full_test.sh`** is the CI-gating harness (wired via `.github/workflows/full-test.yml`
  and `promoted-fixtures.yml`). Its `run_promoted_run_tests` (Section 4, over `tests/run/*.vow`)
  parses and checks `TEST: exit`, `TEST: stdout`, `TEST: stdin[-file]`, `TEST: skip`,
  `TEST: verify-only`, `TEST: known-divergence`, and `TEST: stderr` (every line is a substring
  that must occur in the stderr of both compilers' binaries). Section 5 does the same for
  `tests/debug/*.vow` (debug-mode build, `TEST: exit` defaulting to 134), and Section 4c checks
  `TEST: counterexample-violation` on `tests/verify-fail/*.vow` for both compilers. **It does not
  check `TEST: stderr` on `tests/error/*.vow`** — compile-error stderr is not captured there.
- **`tests/run_tests.sh`** is a separate, local-developer-only harness (not invoked by any
  `.github/workflows/*.yml`). Its Phase 1 (`tests/run/`), Phase 4 (`tests/debug/`) and Phase 5
  (`tests/error/`) loops parse and check `TEST: stderr` (substring match against captured
  stderr, last directive line only), in addition to the directives `full_test.sh` also checks.

So a `// TEST: stderr "..."` line on a `tests/run/*.vow` or `tests/debug/*.vow` fixture is checked
by CI; on a `tests/error/*.vow` fixture it is enforced only when a developer runs
`tests/run_tests.sh` locally.
The `tests/verify-native/` directives (`counterexample-*`, `unknown-reason`, `replay-skipped`,
`skip-reason`, `contract-*`, `warning`) and the weak-contract gate are checked at pull-request time
too, by `.github/workflows/native-fixtures.yml`, which runs `scripts/check_native_fixtures.sh` and
`scripts/check_contract_quality.sh` (and the `summary/` and `trust/` runners). `full_test.sh`
Sections 4g and 8c call those same two scripts (through `run_shared_check`) rather than keeping
copies, so there is one place to change a directive. Run
`scripts/check_native_fixtures.sh [--filter <path substring>] [--jobs N] build/vowc` before
pushing a fixture. The rest of `full_test.sh` is still post-merge only (`full-test.yml`).
`compiler/` changes also run `scripts/compiler_test_parity.sh` (`vow test compiler/` under both
compilers) at PR time via `compiler-tests.yml`; it needs `build/vowc` from `scripts/bootstrap.sh`.

`scripts/full_test.sh` is only the entry script (setup, the `VOW_FULL_TEST_*` mode gates, the
Tier 1.5 cut-off and the summary). Its helpers and sections live in `scripts/full_test/`, one
sourced file per section group (`lib_*.sh` for helpers, `section_NN_*.sh` for sections), listed in
run order in the entry script, so each file stays under the 20,000-byte budget. They share one
shell: a new section file is a `source` line in the entry script, and a helper goes in a `lib_*.sh`.
Before claiming a directive is "never parsed by any harness," grep `tests/*.sh` as well as
`scripts/*.sh` — the two harnesses live in different directories and cover different directives.

## Verifier C Parity

`vow-verify/src/c_emitter.rs` and `compiler/c_emitter.vow` must hand ESBMC **byte-identical C** for the same program. This rule stays in force until both emitters are deleted (epic #1398, P6). `python3 scripts/parity.py c RUST_BIN SELF_BIN FIXTURE...` proves it: it puts a fake `esbmc` first on `PATH`, runs `verify` with each compiler, and diffs the set of distinct C sources recorded per function (`scripts/parity_c.py`; no real solver needed). `scripts/full_test.sh` Section 2c runs it over every `tests/verify*/` fixture, so any change to either emitter, to the lowering feeding it, or to a model helper that is not mirrored in the other compiler fails CI. To see the C itself, call `parity_c.capture(binary, fixture, shim_dir, capture_dir)` (see `scripts/parity_c.py`), or set `VOW_VERIFY_DEBUG=1` on the Rust compiler (it writes `/tmp/vow-verify-debug/<fn>.c`).

### Memory bounds

`bench/memory/programs/*.vow` carry a `// BENCH: max-rss-kb N` bound on their first line.
`scripts/check_memory_bounds.py --compiler <vow|vowc>` builds and runs them against either compiler
and fails when a peak resident set exceeds its bound; `full_test.sh` Section 8a runs it for both
compilers (Linux only), so an allocation fix is pinned in the Rust and the self-hosted compiler alike.
`bench/memory/run.sh` is the developer harness (needs GNU `time`, Rust compiler only). Do not run
`run.sh --record` to add a program: it rewrites every bound; add the program, its annotation and its
`expected.toml` entry by hand (measured peak + 4096 KiB).

## Mutation Testing (`vowc mutants`)

Mutation testing is integrated into the self-hosted compiler as the `vowc mutants` subcommand. It mutates `compiler/*.vow` (or any `--root` directory), runs a tiered oracle (`scripts/bootstrap.sh --skip-cargo`, then a fast `VOW_FULL_TEST_SKIP_CARGO=1 VOW_FULL_TEST_TIER15_ONLY=1 scripts/full_test.sh` checkpoint, then the full `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh` suite), and writes structured JSON output to `mutants.out/` (`mutants.json`, `outcomes.json`, per-status `.txt` lists, plus `diff/<id>.diff` and `logs/<id>.log` per mutant). Stdout carries only a one-line summary.

```bash
build/vowc mutants list                                                # enumerate sites only
build/vowc mutants run --shard 0/8 --tier2-budget-secs 9000            # writes ./mutants.out/
build/vowc mutants run --shard 0/8 --output-dir my-results             # custom output directory
```

Mutation kinds: `op-flip` (binary operators), `const-flip` (`0`/`1`, `true`/`false`), `body-replace` (function bodies → default value for return type), `contract-weaken` (`requires`/`ensures`/`invariant` clauses → `true`).

Skip-list: `// GENERATE:<NAME>:START`/`:END` blocks and `extern "C" { ... }` blocks are excluded. `test_*.vow` files are filtered before scanning.

**Local-only.** Mutation testing is not wired into CI — a full Tier-2 sweep across `compiler/*.vow` is multi-hour wall-clock and would burn through GitHub Actions budget on every nightly. Run it on the developer machine on whatever cadence suits the project (e.g., before tagging a release, or after a substantial compiler change). To split the work, shard explicitly with `--shard 0/8` etc. and run shards sequentially over multiple sessions; the determinism guarantee means the union of `mutants.out/` across shards is well-defined.

**Track shard progress persistently.** Because shards are explicitly designed to run sequentially across multiple sessions, keep a checklist (e.g. `TASKS.md`) recording which shard indices are done vs. pending, alongside each shard's `mutants.out/` location. Update it as each shard completes so a new session can resume from the right shard instead of re-deriving progress from `mutants.out/` state or guessing.

When a `missed.txt` entry appears, the actionable response is to either (a) write a test that catches the mutation, or (b) file an issue documenting why the mutation is equivalent and out of scope. See `docs/mutants.md` for the full output schema and known limitations.

## Vericoding Benchmark Suite

`benchmarks/` contains 40 verification benchmarks (15 Easy, 15 Medium, 10 Hard; 4 Hard are Stretch).
Each benchmark has: `spec.md`, `skeleton.vow`, `reference.vow`, `meta.toml`.
`benchmarks/manifest.toml` lists all benchmarks. 36 non-Stretch references pass `vow verify`.

### Benchmark Runner (`bench/`)

A Python CLI tool (managed by `uv`) that runs frontier LLMs against the benchmark suite.

```bash
# From repo root (requires uv):
uv run --project bench bench/run.py validate-references                             # verify all reference.vow files
uv run --project bench bench/run.py run --model claude-sonnet-5-5 --benchmark E01  # single benchmark
uv run --project bench bench/run.py run --model claude-sonnet-5-5                  # full suite
uv run --project bench bench/run.py run --model claude-sonnet-5-5 --resume         # resume partial run
uv run --project bench bench/run.py report --run-id <id>                            # generate comparison report
uv run --project bench bench/run.py report                                          # report on most recent run

# Or from bench/:
cd bench && uv run python run.py run --model claude-sonnet-5-5
```

**Architecture:** Direct API calls (Anthropic/OpenAI SDKs), not agent tool use. Each benchmark is a single conversation: system prompt (skill docs ~35KB) + spec + skeleton → LLM returns Vow code → `vow verify` → CEGIS loop if needed. Temperature 0.0 for reproducibility.

**Cost confirmation required.** A full-suite `bench/run.py run` (no `--benchmark` filter) calls frontier LLM APIs (Anthropic/OpenAI) across all ~40 benchmarks, with CEGIS retries on top — real, non-trivial API spend. Confirm with the user before starting a full-suite run; a single `--benchmark` run does not need confirmation.

**Files:**
- `bench/run.py` — CLI entry point (`run`, `report`, `validate-references`)
- `bench/runner.py` — core CEGIS loop per benchmark
- `bench/llm.py` — LLM provider abstraction (Anthropic, OpenAI)
- `bench/verifier.py` — `vow verify` subprocess wrapper
- `bench/manifest.py` — manifest + meta.toml loader
- `bench/prompts.py` — system prompt (skill docs) + user prompt templates
- `bench/report.py` — results → markdown comparison report
- `bench/config.toml` — model configurations
- `bench/results/` — gitignored output directory

**Environment variables:** `ANTHROPIC_API_KEY` for Claude models, `OPENAI_API_KEY` for OpenAI models.
