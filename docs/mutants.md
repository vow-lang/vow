# Mutation testing (`vowc mutants`)

A `cargo-mutants`-style mutation-testing tool integrated as a `vowc` subcommand. The default target is the self-hosted compiler at `compiler/*.vow`, with `scripts/full_test.sh` (run with `VOW_FULL_TEST_SKIP_CARGO=1`, see Caveats) as the catch-it-or-miss-it oracle.

Mutation testing is **local-only** — it is not wired into CI. A full sweep across `compiler/*.vow` is multi-hour wall-clock, and a nightly across 8 shards would burn through the GitHub Actions budget for findings only the developer ever consumes. Run it on the cadence the project warrants — before tagging a release, after a substantial compiler change, or whenever you want to audit test coverage.

## Subcommands

```text
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
| `--root` | `compiler` | Directory whose `*.vow` files are mutated. `test_*.vow` files are excluded. Path is interpreted relative to the worktree (see Worktree mode below). |
| `--shard X/Y` | `0/1` | Round-robin split of the deterministic mutant ID space. Mutant `id` is selected iff `id % Y == X`. |
| `--tier1-cmd` | `scripts/bootstrap.sh --skip-cargo` | Fast oracle. Anything but exit 0 = caught at Tier 1. |
| `--tier15-cmd` | `VOW_FULL_TEST_SKIP_CARGO=1 VOW_FULL_TEST_TIER15_ONLY=1 scripts/full_test.sh` | Checkpoint oracle, run on Tier-1 survivors only. `VOW_FULL_TEST_TIER15_ONLY=1` makes `full_test.sh` exit right after Section 8c, before the two expensive, largely redundant-for-mutation-purposes sections (Section 9's bootstrap triple test, which re-does what Tier 1 already verified, and Section 10b's `vowc test` run, ~564s alone). This fast prefix still exercises real program behavior (unlike Tier 1, which only checks that the self-hosted compiler builds to a fixed point) and historically catches most real mutant kills — see #1328 for the measurement. The default shells out to *this repo's own* `scripts/full_test.sh`, which is meaningless as an oracle when `--root` points at a different Vow source tree; pass `--tier15-cmd 'true'` to opt back out to the old two-tier behavior in that case (or in hermetic tests against `tests/fixtures/mutants`). |
| `--tier2-cmd` | `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh` | Full oracle. Only run on Tier-1.5 survivors. The env var skips `full_test.sh`'s `cargo build --all --release` step: every mutant comes from `*.vow` source, so the Rust bootstrap compiler never changes across a run, making that rebuild always redundant — and, under the symlinked-`target/` fast path below, actively destructive (see Caveats). A custom override must preserve this (or avoid `cargo build --all --release` some other way) to stay safe under that fast path. |
| `--tier1-timeout-secs` | `180` | Per-mutant Tier-1 wall-clock cap. |
| `--tier15-timeout-secs` | `1200` | Per-mutant Tier-1.5 wall-clock cap (20 min — roughly 2x the measured ~10 min cost of the Section 0-8c prefix, with headroom for slower machines). |
| `--tier2-timeout-secs` | `3600` | Per-mutant Tier-2 wall-clock cap. |
| `--tier2-budget-secs` | `7200` | Per-shard total Tier-2 budget. Once exhausted, surviving Tier-1.5 mutants are emitted with `status:"unrun"`. Tier-1.5 itself has no aggregate budget — it always runs for every Tier-1 survivor, bounded only per-mutant by `--tier15-timeout-secs`. |
| `--workdir` | `/tmp/vow-mutants-<ms>` | Path of the throwaway `git worktree` used for all mutations. Created at run start, removed at exit. |
| `--output-dir` | `mutants.out` | Directory where `mutants.json`, `outcomes.json`, status text files, `diff/`, `logs/` are written. |
| `--force-unlock` | off | Remove a stale `output_dir/.lock` before starting (recovery from a previous run that exited abnormally). |
| `--skip-baseline` | off | Skip the baseline check (see Baseline check). Needed only for oracles that are deliberately failing or meaningful only on mutated trees. |

### Baseline check

Before the first mutant, `run` executes the Tier-1 and Tier-1.5 oracles once on the unmutated worktree. If either does not exit 0 (including spawn failure and timeout), the run aborts with exit 1 and `baseline oracle failed; mutation results would be meaningless`, without writing `mutants.json` or `outcomes.json`; the worktree and lock are released. Output of the baseline is kept in `logs/baseline.log`. Without this check a globally broken oracle (for example a fresh worktree with no `./target/release/vow`) fails every mutant identically, and every mutant is scored `caught`.

Tier 2 is not baselined (a full suite run per shard is ~30-46 minutes); Tier 1.5 already runs the same `full_test.sh` prefix. The baseline reuses `--tier1-timeout-secs` and `--tier15-timeout-secs`, and costs one Tier-1 plus one Tier-1.5 run per shard. Pass `--skip-baseline` to bypass it.

### Infrastructure failures

Each oracle command runs as `cd <workdir> || exit 125; (...) > log`. If `cd` fails (the worktree vanished or became unreadable), the oracle never ran, so exit code 125 is reserved: `run` aborts with exit 1 and `oracle workdir unreachable (cd failed); aborting shard` instead of scoring the mutant `caught`; the mutated file is restored best-effort and the worktree and lock are released. An oracle command that itself exits 125 is indistinguishable and aborts the run the same way.

### Tier 1.5

The oracle is a three-tier pipeline: Tier 1 (build-to-fixed-point only) → Tier 1.5 (a fast,
behavior-exercising prefix of `full_test.sh`) → Tier 2 (the full suite). A mutant that survives
Tier 1 almost always gets a verdict from Tier 1.5 in ~10 minutes instead of paying for the full
~30-46 minute Tier-2 run; only mutants Tier 1.5 can't catch fall through to Tier 2. See #1328 for
the rationale and the measured section timings that motivated the Section 0-8c cutoff.

Tier 2's default command is the unmodified, full `scripts/full_test.sh` (Sections 0 through 13),
so a mutant that survives Tier 1.5 and falls through to Tier 2 re-runs the Section 0-8c prefix a
second time as part of that full run. This is a deliberate, minimal-diff trade-off (see #1328):
Tier 1.5's whole purpose is triage for the common case, and most mutants never reach Tier 2 at
all. Only mutants that *do* reach Tier 2 pay the Section 0-8c prefix twice, costing roughly
`--tier15-timeout-secs`'s measured ~10 minutes in addition to the full Tier-2 run.

## Worktree mode

`vowc mutants run` operates on a fresh `git worktree` (created via `git worktree add --detach`) instead of mutating the live source tree. This guarantees the original `compiler/` (or any `--root`) is byte-identical before and after the run, even on Ctrl-C or oracle crashes. The worktree is removed via `git worktree remove --force` at exit.

**Caveats**:
- The worktree's `target/` starts empty. The default Tier-1 oracle `scripts/bootstrap.sh --skip-cargo` requires `target/release/vow` to already exist, so it will fail in the worktree unless you (a) pass `--tier1-cmd 'scripts/bootstrap.sh'` to run the full bootstrap inside the worktree, or (b) symlink `target/` from the original tree before invoking. Local development typically uses (b) for speed; (a) is what you'd want when running from a fresh checkout. Under (b), `target/` is shared with the real repo, so anything that writes through it — like `cargo build --all --release` — writes into the real repo's artifacts from inside the throwaway worktree. The default `--tier2-cmd` above sets `VOW_FULL_TEST_SKIP_CARGO=1` specifically so Tier 2 never does this; a hand-rolled `--tier2-cmd` that drops that env var (or otherwise shells out to `cargo build`) will corrupt the shared `target/` the same way (issue #1296).
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
├── unrun.txt          # Tier-1.5 survivors not run because Tier-2 budget was exhausted
├── diff/<id>.diff     # per-mutant unified diff, captured from the worktree
└── logs/<id>.log      # per-mutant oracle stdout+stderr (Tier 1, then Tier 1.5, then Tier 2 if reached)
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
     "status": "caught", "tier": 1.5, "oracle_ms": 612000},
    {"id": 1, "name": "compiler/checker.vow:89:5: 0 → 1",
     "status": "missed", "tier": 2, "oracle_ms": 2731000},
    …
  ]
}
```

See `docs/spec/schemas/mutants-result.schema.json` for the formal schema.

`stdout` carries only the one-line summary record (so the per-shard verdict surfaces at a glance even when the full output is in `mutants.out/`).

## Determinism guarantee

For a fixed source tree and shard configuration, `vowc mutants list` produces byte-identical output across runs. `vowc mutants run` produces `mutants.json` byte-identically; `outcomes.json` differs only in the `oracle_ms` field per record. Mutant IDs are stable, so re-running a single shard's failing mutants is straightforward.

## How to run it

To split the work across multiple sessions, shard explicitly with `--shard 0/8` and run shards sequentially. The determinism guarantee above means the union of `mutants.out/` across shards is well-defined, so partial nightly progress accumulates over runs.

When a `missed.txt` entry appears, the actionable response is to either (a) write a test that catches the mutation, or (b) file an issue documenting why the mutation is equivalent and out of scope.

## Limitations (v1)

- **Wall-clock at scale**: a full Tier-2 sweep across `compiler/*.vow` takes hours. The `--tier2-budget-secs` cap (default 7200 = 2 h) ensures graceful degradation: surviving Tier-1.5 mutants beyond the budget are emitted with `status:"unrun"` so coverage gaps are explicit rather than silent. Multiple sessions across different shards reach full coverage; the determinism guarantee makes the union well-defined.
- **Equivalent mutants**: weakening a non-load-bearing `ensures` clause (e.g., a `result >= 0` clause on a constant function) yields a `missed` record even though the contract is functionally redundant. There is no equivalent-mutant detector in v1.
- **Lock TOCTOU race**: the `.lock` directory is created with `fs_mkdir` after an `fs_exists` probe; vow-runtime's `fs_mkdir` is `mkdir -p` semantics, not atomic. Two nearly-simultaneous invocations against the same `--output-dir` could both pass the existence check. In practice you only run one mutation pass at a time per output dir; if you parallelise, point each invocation at its own `--output-dir`.
- **JSON output is escaped** for `"`, `\`, and ASCII control bytes (newline / carriage-return / tab / backspace / form-feed → `\n` / `\r` / `\t` / `\b` / `\f`; other bytes < 0x20 → `?`). Non-ASCII bytes (UTF-8 continuation bytes for multi-byte codepoints) pass through unescaped, which is valid in modern JSON parsers but technically not pure-ASCII JSON.
- **Generic-type angle brackets are mutated.** The token-level scanner emits `< → >=` and `> → <=` op-flip sites for `<` and `>` everywhere they appear, including generic type positions (`Vec<i64>`, `Vec<String>`, etc.). Mutating these produces unparseable source, which the oracle classifies as `unviable`. The unviable count therefore inflates with the number of generic type uses in the target tree; this is correct but noisy.
- **Unary minus produces unviable records.** The scanner emits `- → +` for every `-` not immediately followed by `>` (the return-type arrow), including unary positions (e.g., `let x: i64 = -5;`). Vow has no unary `+` operator, so these mutations produce unparseable source and the oracle classifies them as `unviable` — same shape as the angle-bracket case above.
- **Op-flip coverage gap inside `vow { … }` blocks** when the function's return type is supported by `default_for_ty` (i64/bool/String/Vec/etc.). In that case `try_emit_body_replace` consumes the vow block via `scan_vow_block_contracts` (contract sites only) and the outer scanner skips ahead to the body, so operator mutations *inside* contract clauses (e.g. `b == 0` inside `requires: b != 0`) aren't enumerated. Functions with unsupported return types don't have this gap because the outer scan walks through the vow block naturally.
- **Build-vs-test failures both classify as `caught`.** When a Tier-1 oracle's exit code is nonzero, `vowc mutants` records the mutant as `caught` regardless of whether the failure was a real test detecting the mutation or a build failure (e.g., the mutated source is unparseable). cargo-mutants distinguishes these via parse-time checks; we don't currently. Practically, angle-bracket and unary-minus noise inflates the `caught` bucket; equivalent-mutant analysis would require deeper integration with the Vow parser. The baseline check guards only against a *globally* broken oracle; a mutant that merely fails to build is still `caught`.
- **Unsupported return types**: function bodies whose return type doesn't match the supported set produce no `body-replace` site (silent skip).
- **Sequential within a shard**: one mutant at a time. Parallel workers per shard would require multiple worktrees; deferred to a follow-up.
- **Quadratic line/column lookup**: each `Site` constructor calls `line_col_at(src, off)` independently, walking from byte 0 every time. For ~24 K LOC of `compiler/*.vow` this is observable in `list` wall-clock but small in absolute terms; a single-pass running accumulator threaded through the scanner would make site enumeration O(file_size) instead of O(file_size × site_count). Deferred.
