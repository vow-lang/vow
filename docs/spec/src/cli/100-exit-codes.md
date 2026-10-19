
### `vow decl`

Emit declaration file output only.

```
vow decl [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `-o, --output`    | `<source>.vow.d` | Output declaration file path          |

`decl` type-checks the source with its `use` dependencies, then writes one declaration stub for the merged module: dependency items first, the entry module's items last, with no `use` lines. Function bodies are dropped; signatures, effects, `vow` blocks, struct/enum/const/type-alias definitions and `extern "C"` blocks are kept. The path written is reported on stderr as `wrote <path>` and stdout stays empty. An existing output file is overwritten. Frontend diagnostics go to stderr, followed by `vow decl: <parse|module load|type> error`, and the exit code is 1 with no file written. `decl` accepts only `-o/--output`, `--help` and `--human`.

The self-hosted compiler cannot print an `enum` with a struct-like variant (`V { f: T }`): its AST drops the field names. `vowc decl` exits 1 with `vow decl: enum <Name> has a struct-like variant, which the self-hosted compiler cannot print` instead of writing a stub with a different meaning.

### `vow mutants` (self-hosted only)

Run mutation testing on a Vow source tree. Implemented in the self-hosted compiler only; the Rust bootstrap compiler emits an error pointing the user to `build/vowc`. See `docs/mutants.md` for full details on output schema, mutation kinds, skip-list, and known limitations.

```
vowc mutants version
vowc mutants list  [--root DIR] [--shard X/Y]
vowc mutants run   [--root DIR] [--shard X/Y]
                   [--tier1-cmd 'cmd'] [--tier15-cmd 'cmd'] [--tier2-cmd 'cmd']
                   [--tier1-timeout-secs N] [--tier15-timeout-secs N] [--tier2-timeout-secs N]
                   [--tier2-budget-secs N]
                   [--workdir DIR] [--output-dir DIR] [--force-unlock] [--skip-baseline]
```

| Flag | Default | Notes |
|---|---|---|
| `--root` | `compiler` | Directory whose `*.vow` files are mutated. `test_*.vow` files are excluded. |
| `--shard X/Y` | `0/1` | Round-robin split of the deterministic mutant ID space. Mutant `id` is selected iff `id % Y == X`. |
| `--tier1-cmd` | `scripts/bootstrap.sh --skip-cargo` | Fast oracle. Anything but exit 0 = caught at Tier 1. |
| `--tier15-cmd` | `VOW_FULL_TEST_SKIP_CARGO=1 VOW_FULL_TEST_TIER15_ONLY=1 scripts/full_test.sh` | Checkpoint oracle, run on Tier-1 survivors only. Fast, behavior-exercising prefix of `full_test.sh` (Sections 0-8c) — see `docs/mutants.md`. |
| `--tier2-cmd` | `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh` | Full oracle. Only run on Tier-1.5 survivors. The env var skips the redundant `cargo build --all --release` step — see `docs/mutants.md`. |
| `--tier1-timeout-secs` | `180` | Per-mutant Tier-1 wall-clock cap. |
| `--tier15-timeout-secs` | `1200` | Per-mutant Tier-1.5 wall-clock cap. |
| `--tier2-timeout-secs` | `3600` | Per-mutant Tier-2 wall-clock cap. |
| `--tier2-budget-secs` | `7200` | Per-shard total Tier-2 budget. Once exhausted, surviving Tier-1.5 mutants are emitted with `status:"unrun"`. |
| `--workdir` | `/tmp/vow-mutants-<ms>` | Path of the throwaway `git worktree` used for all mutations. |
| `--output-dir` | `mutants.out` | Directory for `mutants.json`, `outcomes.json`, status text files, `diff/`, `logs/`. |
| `--force-unlock` | off | Remove a stale `output_dir/.lock` before starting. |
| `--skip-baseline` | off | Skip the baseline run of Tier 1 and Tier 1.5 on the unmutated worktree. Without it, a failing baseline aborts the run with exit 1 — see `docs/mutants.md`. |

Output schemas: see `docs/spec/schemas/mutants-result.schema.json`.

### `vow complexity`

Report per-function complexity metrics as deterministic, **byte-identical** JSON (the Rust and self-hosted compilers produce identical output). Every structural metric sits next to its size; the single 0–100 `complexity_score` is a readability / refactor-priority gate — explicitly **not** a defect predictor. The component vector, not the scalar, is the source of truth; gating on the scalar alone is opt-in and discouraged as the sole signal.

```
vow complexity <source.vow>
               [--cog-anchor N] [--nloc-anchor N]
               [--max-score N] [--max-cognitive N] [--max-cyclomatic N]
```

| Flag | Default | Notes |
|---|---|---|
| `--cog-anchor <N>` | `15` | Cognitive-complexity value mapped to sub-score `0.800` (SonarQube's default flag line). |
| `--nloc-anchor <N>` | `60` | NLOC value mapped to sub-score `0.800` (~50–60 line guidance). |
| `--max-score <N>` | (unset) | CI gate: exit nonzero if any function's `complexity_score` exceeds N. The recommended line is 80, but gating is opt-in only. |
| `--max-cognitive <N>` | (unset) | CI gate: exit nonzero if any function's `cognitive` exceeds N. |
| `--max-cyclomatic <N>` | (unset) | CI gate: exit nonzero if any function's `cyclomatic` exceeds N. |

**Exit code.** Nonzero on frontend/read failures, malformed numeric flags, or when a `--max-*` threshold is passed and exceeded. With no `--max-*` flag the command is pure reporting once the input is readable and valid — no threshold gates by default (per the decouple-language-from-prover principle).

**Numeric convention.** The non-integer metrics (`halstead.volume`/`difficulty`/`effort` and `score_factors.*`) are emitted as fixed-3-decimal JSON numbers computed in **integer fixed-point** (scale 1000) — never native floats — so both compilers stay byte-identical. `complexity_score` is an integer in `[0, 100]`. The score's saturating anchor map uses a rational curve (`0.800` at the anchor, asymptoting to `1.000`), not an exponential, because the self-hosted compiler has no floating point.

**Contract identifier convention.** `vow.contract.free_vars` counts distinct value identifiers referenced by clause predicates. It excludes the `result` binding, function callee identifiers, and method names; receiver and argument expressions still count when they are values.

Output schema: see `docs/spec/schemas/complexity-result.schema.json`.

### `vow --help`

`vow --help` is agent-first. It emits versioned JSON capability data for the tool, command set,
language surface, result schemas, and implementation status. `--help --human` exists only as a
legacy compatibility mode and is not the canonical interface.

```
vow --help               # versioned JSON tool-help protocol
vow --help --human       # legacy compatibility text
vow build --help         # same JSON (works on all subcommands)
vow verify --help --human  # same legacy text (works on all subcommands)
```

## Exit Codes

| Code | Meaning                                                                            |
|------|------------------------------------------------------------------------------------|
| `0`  | Success (`Verified` or `Unverified`)                                               |
| `1`  | Failure (`CompileFailed`, `VerifyFailed`, or `Skipped`)                            |
| `2`  | Usage error: an unknown flag (`error: unexpected argument '<flag>' found`)         |

Both compilers reject a flag the selected subcommand does not implement before doing any work, with
exit `2` and `error: unexpected argument '<flag>' found` on stderr. This includes flags that were
retired prover-model flags and typos: they are never
silently ignored. The self-hosted `vowc` only accepts the `--flag value` form for value flags;
`--flag=value` is reported as an unknown argument with a hint. `vowc mutants` forwards its flags to
its own parser and is not validated by the driver.

`vow build` and `vow verify` both fail closed on `Skipped`: if the verifier was asked to verify a vowed
function but the verifier could not model the function body, the contract was not statically
proved, so the run exits non-zero. Use `--no-verify` if you genuinely want to skip verification —
that path produces `Unverified` (exit 0).

The table above is the exit status of the `vowc` **compiler**. A **compiled Vow program** exits
with whatever its `main` returns, with one reserved exception: any runtime abort — out-of-memory,
contract violation, arithmetic overflow, division or remainder by zero, unwrap-on-`None`,
index-out-of-bounds, region-literal mutation, stack overflow, or a sanitizer trap — terminates with
the reserved status **`134`**. By convention `134` is reserved for aborts: it is never produced
*spontaneously* by a normal `main` return, so a program that does not itself return or
`process_exit(134)` can treat any `134` as a runtime abort rather than an application result. The
reservation is a convention, not enforced — a program that deliberately exits `134` opts out. See
the *Exit status* note under Runtime Errors in [`errors.md`](errors.md) for the full list and
rationale.

## Build Output JSON

`vow build` and `vow verify` emit a single JSON object to stdout. Schema: [`schemas/build-result.schema.json`](schemas/build-result.schema.json).

**Note:** `--dump-ir` suppresses JSON output — only IR text is printed.
