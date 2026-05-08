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
                  [--workdir DIR] [--output-dir DIR] [--force-unlock]
```

| Flag | Default | Notes |
|---|---|---|
| `--root` | `compiler` | Directory whose `*.vow` files are mutated. `test_*.vow` files are excluded. Path is interpreted relative to the worktree (see Worktree mode below). |
| `--shard X/Y` | `0/1` | Round-robin split of the deterministic mutant ID space. Mutant `id` is selected iff `id % Y == X`. |
| `--tier1-cmd` | `scripts/bootstrap.sh --skip-cargo` | Fast oracle. Anything but exit 0 = caught at Tier 1. |
| `--tier2-cmd` | `scripts/full_test.sh` | Full oracle. Only run on Tier-1 survivors. |
| `--tier1-timeout-secs` | `180` | Per-mutant Tier-1 wall-clock cap. |
| `--tier2-timeout-secs` | `3600` | Per-mutant Tier-2 wall-clock cap. |
| `--tier2-budget-secs` | `7200` | Per-shard total Tier-2 budget. Once exhausted, surviving Tier-1 mutants are emitted with `status:"unrun"`. |
| `--workdir` | `/tmp/vow-mutants-<ms>` | Path of the throwaway `git worktree` used for all mutations. Created at run start, removed at exit. |
| `--output-dir` | `mutants.out` | Directory where `mutants.json`, `outcomes.json`, status text files, `diff/`, `logs/` are written. |
| `--force-unlock` | off | Remove a stale `output_dir/.lock` before starting (recovery from a previous run that exited abnormally). |

## Worktree mode

`vow-mutants run` operates on a fresh `git worktree` (created via `git worktree add --detach`) instead of mutating the live source tree. This guarantees the original `compiler/` (or any `--root`) is byte-identical before and after the run, even on Ctrl-C or oracle crashes. The worktree is removed via `git worktree remove --force` at exit.

**Caveats**:
- The worktree's `target/` starts empty. The default Tier-1 oracle `scripts/bootstrap.sh --skip-cargo` requires `target/release/vow` to already exist, so it will fail in the worktree unless you (a) pass `--tier1-cmd 'scripts/bootstrap.sh'` to run the full bootstrap inside the worktree, or (b) symlink `target/` from the original tree before invoking. The bundled `.github/workflows/vow-mutants.yml` uses option (a).
- The repo must be a git working tree. A non-git checkout is not supported in v1.

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

## Output: `mutants.out/`

`run` populates a directory (default `mutants.out/`) with:

```text
mutants.out/
├── .lock              # presence indicates a run is in progress
├── mutants.json       # full catalog of this shard's mutants, written before testing
├── outcomes.json      # per-mutant verdicts + summary, written after testing
├── caught.txt         # newline-separated mutant names (cargo-mutants format)
├── missed.txt
├── timeout.txt
├── unviable.txt
├── unrun.txt          # Tier-1 survivors not run because Tier-2 budget was exhausted
├── diff/<id>.diff     # per-mutant unified diff, captured from the worktree
└── logs/<id>.log      # per-mutant oracle stdout+stderr (Tier 1 followed by Tier 2 if reached)
```

`mutants.json` schema (abbreviated):

```json
{
  "version": 1,
  "tool": "vow-mutants",
  "shard": "0/8",
  "mutants": [
    {"name": "compiler/lower.vow:1234:17: + → -",
     "file": "compiler/lower.vow", "line": 1234, "col": 17,
     "off": 12345, "len": 1,
     "kind": "op-flip", "from": "+", "to": "-",
     "label": "+ → -", "clause_index": 0},
    …
  ]
}
```

`outcomes.json` schema:

```json
{
  "version": 1,
  "summary": {"total": 34, "caught": 12, "missed": 2, "timeout": 0, "unviable": 0, "unrun": 20, "shard": "0/8"},
  "outcomes": [
    {"id": 0, "name": "compiler/lower.vow:1234:17: + → -",
     "status": "missed", "tier": 2, "oracle_ms": 2731000},
    …
  ]
}
```

See `docs/spec/schemas/mutants-result.schema.json` for the formal schema.

`stdout` carries only the one-line summary record (so CI logs surface the verdict at a glance).

## Determinism guarantee

For a fixed source tree and shard configuration, `vow-mutants list` produces byte-identical output across runs. `vow-mutants run` produces `mutants.json` byte-identically; `outcomes.json` differs only in the `oracle_ms` field per record. Mutant IDs are stable, so re-running a single shard's failing mutants is straightforward.

## CI

`.github/workflows/vow-mutants.yml` runs the full mutation pass nightly, sharded across 8 GitHub Actions runners with a 150-minute per-shard Tier-2 budget. Each shard uploads its `mutants.out/` directory as an artifact for offline review.

## Limitations (v1)

- **Wall-clock at this scale, not specific to vow-mutants**: any mutation-testing pass on a codebase this size exceeds a single CI run, regardless of oracle. cargo-mutants on this repo is in the same position — its 90-min nightly with 8 shards and `cargo test` per mutant only reaches a fraction of the total mutant count, and the rest are silently absent from `mutants.out`. vow-mutants makes the unrun set explicit (`status:"unrun"` records when `--tier2-budget-secs` is exhausted) so coverage gaps are visible per shard. Full Tier-2 coverage takes multiple nightlies; the determinism guarantee above means the union across runs is well-defined.
- **Equivalent mutants**: weakening a non-load-bearing `ensures` clause (e.g., a `result >= 0` clause on a constant function) yields a `missed` record even though the contract is functionally redundant. There is no equivalent-mutant detector in v1.
- **Lock TOCTOU race**: the `.lock` directory is created with `fs_mkdir` after an `fs_exists` probe; vow-runtime's `fs_mkdir` is `mkdir -p` semantics, not atomic. Two nearly-simultaneous invocations against the same `--output-dir` could both pass the existence check. In practice CI shards write to different output dirs, so the race window is acceptable.
- **JSON output is escaped** for `"`, `\`, and ASCII control bytes (newline / carriage-return / tab / backspace / form-feed → `\n` / `\r` / `\t` / `\b` / `\f`; other bytes < 0x20 → `?`). Non-ASCII bytes (UTF-8 continuation bytes for multi-byte codepoints) pass through unescaped, which is valid in modern JSON parsers but technically not pure-ASCII JSON.
- **Generic-type angle brackets are mutated.** The token-level scanner emits `< → >=` and `> → <=` op-flip sites for `<` and `>` everywhere they appear, including generic type positions (`Vec<i64>`, `Vec<String>`, etc.). Mutating these produces unparseable source, which the oracle classifies as `unviable`. The unviable count therefore inflates with the number of generic type uses in the target tree; this is correct but noisy.
- **Unary minus produces unviable records.** The scanner emits `- → +` for every `-` not immediately followed by `>` (the return-type arrow), including unary positions (e.g., `let x: i64 = -5;`). Vow has no unary `+` operator, so these mutations produce unparseable source and the oracle classifies them as `unviable` — same shape as the angle-bracket case above.
- **Op-flip coverage gap inside `vow { … }` blocks** when the function's return type is supported by `default_for_ty` (i64/bool/String/Vec/etc.). In that case `try_emit_body_replace` consumes the vow block via `scan_vow_block_contracts` (contract sites only) and the outer scanner skips ahead to the body, so operator mutations *inside* contract clauses (e.g. `b == 0` inside `requires: b != 0`) aren't enumerated. Functions with unsupported return types don't have this gap because the outer scan walks through the vow block naturally.
- **Build-vs-test failures both classify as `caught`.** When a Tier-1 oracle's exit code is nonzero, vow-mutants records the mutant as `caught` regardless of whether the failure was a real test detecting the mutation or a build failure (e.g., the mutated source is unparseable). cargo-mutants distinguishes these via parse-time checks; we don't currently. Practically, angle-bracket and unary-minus noise inflates the `caught` bucket; equivalent-mutant analysis would require deeper integration with the Vow parser.
- **Unsupported return types**: function bodies whose return type doesn't match the supported set produce no `body-replace` site (silent skip).
- **Sequential within a shard**: one mutant at a time. Parallel workers per shard would require multiple worktrees; deferred to a follow-up.
- **Quadratic line/column lookup**: each `Site` constructor calls `line_col_at(src, off)` independently, walking from byte 0 every time. For ~24 K LOC of `compiler/*.vow` this is observable in `list` wall-clock but small in absolute terms; a single-pass running accumulator threaded through the scanner would make site enumeration O(file_size) instead of O(file_size × site_count). Deferred.
