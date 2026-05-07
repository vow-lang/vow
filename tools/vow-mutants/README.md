# vow-mutants

A `cargo-mutants`-style mutation-testing tool for Vow programs. Self-hosted: `vow-mutants` is itself a Vow program in `tools/vow-mutants/`. The default target is the self-hosted compiler at `compiler/*.vow`, with `scripts/full_test.sh` as the catch-it-or-miss-it oracle.

Closes #306.

## Build

```bash
build/vowc build --no-verify tools/vow-mutants/main.vow -o build/vow-mutants
```

## Subcommands

```text
vow-mutants version
vow-mutants list  [--root DIR] [--shard X/Y]
vow-mutants run   [--root DIR] [--shard X/Y]
                  [--tier1-cmd 'cmd'] [--tier2-cmd 'cmd']
                  [--tier1-timeout-secs N] [--tier2-timeout-secs N]
                  [--tier2-budget-secs N]
```

| Flag | Default | Notes |
|---|---|---|
| `--root` | `compiler` | Directory whose `*.vow` files are mutated. `test_*.vow` files are excluded. |
| `--shard X/Y` | `0/1` | Round-robin split of the deterministic mutant ID space. Mutant `id` is selected iff `id % Y == X`. |
| `--tier1-cmd` | `scripts/bootstrap.sh --skip-cargo` | Fast oracle. Anything but exit 0 = caught at Tier 1. |
| `--tier2-cmd` | `scripts/full_test.sh` | Full oracle. Only run on Tier-1 survivors. |
| `--tier1-timeout-secs` | `180` | Per-mutant Tier-1 wall-clock cap. |
| `--tier2-timeout-secs` | `3600` | Per-mutant Tier-2 wall-clock cap. |
| `--tier2-budget-secs` | `7200` | Per-shard total Tier-2 budget. Once exhausted, surviving Tier-1 mutants are emitted with `status:"unrun"`. |

## Mutation kinds

| Kind | Trigger | Replacement |
|---|---|---|
| `op-flip` | Binary operators `+ - * / % == != < <= > >= && \|\|` | Canonical inverse (e.g., `+`→`-`, `==`→`!=`, `<`→`>=`). Checked-arith forms `+! -! *!` are skipped. |
| `const-flip` | Integer literals `0`/`1`, boolean keywords `true`/`false` | The other value. |
| `body-replace` | Function bodies whose return type is in {`i64`,`u64`,`i32`,`u32`,`i8`,`u8`,`i16`,`u16`,`bool`,`()`,`String`,`Vec<…>`} | The default value for that type. |
| `contract-weaken` | `requires:` / `ensures:` / `invariant:` clauses inside `vow { … }` blocks | Replaced with `true`. Sibling clauses on one function get distinct `clause_index` (0, 1, 2, …). |

## Skip-list

Sites whose byte range falls inside any of the following ranges are dropped before sharding:

- `// GENERATE:<NAME>:START` … `// GENERATE:<NAME>:END` line pairs (matched by name).
- `extern "C" { … }` blocks (brace-balanced; comment- and string-aware).
- Files matching `test_*.vow` are filtered before scanning.

## Output schema

`run` emits one JSON object per line, plus a final summary line. See `docs/spec/schemas/mutants-result.schema.json` for the full schema. Example record:

```json
{"file":"compiler/lower.vow","off":1234,"len":1,"kind":"op-flip","from":"+","to":"-","label":"+ → -","clause_index":0,"status":"missed","tier":2,"oracle_ms":2731000}
```

Final line:

```json
{"total":34,"caught":12,"missed":2,"timeout":0,"unviable":0,"unrun":20,"shard":"0/8"}
```

`oracle_ms` (per-mutant timing) is the only field that varies across runs; everything else is deterministic given a fixed compiler tree and shard configuration.

## Determinism guarantee

For a fixed source tree and shard configuration, `vow-mutants list` produces byte-identical output across runs. `vow-mutants run` produces output that differs only in the `oracle_ms` field. Mutant IDs are stable, so re-running a single shard's failing mutants is straightforward.

## CI

`.github/workflows/vow-mutants.yml` runs the full mutation pass nightly, sharded across 8 GitHub Actions runners, with a 150-minute per-shard Tier-2 budget. Artifacts (`mutants.out` per shard) are uploaded for offline review.

## Limitations (v1)

- **Wall-clock**: full Tier-2 coverage of `compiler/*.vow` does not fit in a single nightly run. Many shards will exit with a non-trivial number of `unrun` records. See the plan at `.ultraplan/vow-mutants.md` Risks section for the math.
- **Equivalent mutants**: weakening a non-load-bearing `ensures` clause (e.g., a `result >= 0` clause on a constant function) yields a `missed` record even though the contract is functionally redundant. There is no equivalent-mutant detector in v1.
- **Concurrency**: `run` mutates files in-place. Do not run locally with uncommitted changes in `--root`. The runner refuses to start if `--root` has uncommitted changes (use `--force` to override; not yet implemented).
- **No JSON escaping** of `from`/`label` fields. The fields originate from compiler source, which currently uses ASCII-printable byte ranges; non-ASCII content in identifiers or comments would produce malformed JSON.
- **Unsupported return types**: function bodies whose return type doesn't match the supported set produce no `body-replace` site (silent skip).
