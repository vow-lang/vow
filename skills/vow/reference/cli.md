# Vow CLI Reference

## Commands

### `vow build` (default)

Compile source to native executable. Verifies contracts by default.

```
vow build [OPTIONS] <source.vow>
vow [OPTIONS] <source.vow>          # legacy (equivalent)
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `-o, --output`    | `build/<stem>` | Output executable path                  |
| `--mode <debug\|release\|profile\|sanitize>` | `release` | Build mode: debug inserts runtime vow checks, profile inserts call counters and prints report on normal exit, sanitize adds debug checks + Vec provenance tracking |
| `--no-verify`     | (off)       | Skip ESBMC static verification            |
| `--dump-ir`       | (off)       | Print IR text to stdout and exit (no JSON output, no codegen) |
| `--debug-trace <off\|calls\|full>` | `off` | Emit JSON trace lines to stderr at runtime |
| `--no-cache`    | (off)       | Disable verification result caching, and (for `--no-verify` builds) the compile-object cache. See "Compile-object cache behavior" below |
| `--max-k-step <N>` | `50`     | ESBMC incremental BMC max iterations          |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver; auto selects per-function via heuristic |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 |
| `--timeout <N>` | `300` (or `30` when `--encoding` is `auto`) | ESBMC per-function timeout in seconds. Under `--encoding auto`, a 30s default is applied so the BV-timeout fallback to `--encoding ir --solver z3` can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit `--timeout` overrides both. `--timeout 0` is honoured as an immediate watchdog kill |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent ESBMC verification jobs |

**Compile-object cache behavior.** The on-disk compile-object cache (`$VOW_CACHE_DIR` or `~/.cache/vow/`, where each entry is a `<key>.o` artifact keyed by a content hash of all dependencies, mode, and trace settings) is automatically disabled whenever ESBMC verification is active. This guarantees the linked binary always comes from the same codegen run whose IR was verified, closing the integrity gap where a stale or attacker-supplied `.o` could be linked against freshly-verified IR. Concretely the cache only activates on `vow build --no-verify` invocations; it is bypassed on the default `vow build` path. `--no-cache` additionally disables the cache for `--no-verify` builds.

### `vow verify`

Verify contracts only — no executable output. Emits the same JSON format as `vow build` but `executable` is always `null`.

```
vow verify [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `--no-cache`      | (off)       | Disable verification result caching        |
| `--max-k-step <N>` | `50`       | ESBMC incremental BMC max iterations       |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver; auto selects per-function via heuristic |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 |
| `--timeout <N>` | `300` (or `30` when `--encoding` is `auto`) | ESBMC per-function timeout in seconds. Under `--encoding auto`, a 30s default is applied so the BV-timeout fallback to `--encoding ir --solver z3` can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit `--timeout` overrides both. `--timeout 0` is honoured as an immediate watchdog kill |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent ESBMC verification jobs |

### `vow contracts`

List all contracts (requires, ensures, invariant) in a program. Runs frontend only by default (no codegen, no verification).

```
vow contracts [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `--verify`        | (off)       | Run ESBMC verification and report per-contract status |
| `--no-cache`      | (off)       | Disable verification result caching        |
| `--max-k-step <N>` | `50`       | ESBMC incremental BMC max iterations       |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver (with --verify)           |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode (with --verify); ir requires z3 |
| `--verify-jobs <N>` | `num_cpus/2` | Accepted for CLI parity with build/verify/test; currently a no-op (the contracts verifier is serial) |

**Exit code.** With `--verify`, `vow contracts` fails closed exactly like `vow build --verify` and `vow verify`: it exits **1** if any contract's `status` is not proven — i.e. any `failed`, `timeout`, `unknown`, `error`, `skipped`, or `vacuous` — and **0** only when every contract is `proven`/`proven-ir`. Without `--verify` every contract is `not_verified` and the command exits 0. (This is independent of the static `quality` classification, which never affects the exit code.)

When `--verify` is requested but ESBMC is not installed, the command still emits the full contracts-result JSON schema with every entry's `status` set to `error` and exits with code 1 (fail-closed). Install ESBMC, or omit `--verify`, to obtain proven/failed/unknown statuses.

### `vow skill`

Generate or install the Claude Code skill document for the current compiler version. The skill is embedded in the compiler binary, ensuring the documentation always matches the installed toolchain.

```
vow skill              # print skill document to stdout (default: print)
vow skill print        # print concise Claude Code SKILL.md entrypoint
vow skill print --bundle  # print self-contained bundle for raw API harnesses
vow skill install      # prompt for local or global install target
vow skill install --local   # install to ./.claude/skills/vow/
vow skill install --global  # install to $HOME/.claude/skills/vow/ on Linux
```

`print` writes the concise installed `SKILL.md` entrypoint (with YAML frontmatter) to stdout. `print --bundle` writes a complete self-contained skill document to stdout for non–Claude Code harnesses that cannot load supporting files.

`install` writes `SKILL.md` plus supporting files under `reference/`, `examples/`, and `schemas/`. Claude Code discovers the skill from the `.claude/skills/` directory and uses the frontmatter description/`when_to_use` metadata to load it for `.vow` file work as well as creation and verification-debugging prompts before a `.vow` file exists.

When no scope flag is provided, `install` prompts on stderr for local (`./.claude`) or global (`$HOME/.claude`) installation. Scripts and agents should pass `--local` or `--global` explicitly. `--local` requires the current directory to contain both `.git` and `.claude/`; otherwise it exits with an error and writes nothing. `--global` installs under `$HOME/.claude/skills/vow/` and fails if `$HOME` is unset or empty.

**Auto-install on build.** The first time `vow build` (or the legacy `vow <source.vow>` form) runs in a directory that already contains a `.claude/` subtree but no `.claude/skills/vow/SKILL.md`, the compiler installs the skill silently. This bootstraps Claude Code projects without requiring an explicit `vow skill install`. Unlike explicit `--local`, auto-install only requires `.claude/`; it does not require the directory to be a git checkout. Auto-install is skipped when `.claude/` does not exist (so it never pollutes non–Claude Code projects) and when the skill file is already present (so user edits are never overwritten). Auto-install never fails the build.

### `vow test`

Discover, compile, run, and report on Vow test files. Tests are normal `.vow` programs with `main() -> i32` — no test-specific syntax.

```
vow test [OPTIONS] [<path>]
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `<path>`          | `.`         | Directory to scan or single `.vow` file    |
| `--verify`        | (off)       | Run ESBMC verification on test files       |
| `--filter <pat>`  | (none)      | Only run tests whose file stem contains pat |
| `--module-root <path>` | (auto)  | Resolve `use` declarations against `<path>`. Defaults to the scan path when it's a directory, otherwise the entry file's parent directory. |
| `--mode debug`    | (default)   | Insert runtime vow checks                 |
| `--mode release`  | `debug`     | Omit all vow checks for performance       |
| `--timeout <ms>`  | `30000`     | Per-test execution timeout in milliseconds |
| `--max-k-step <N>` | `50`       | ESBMC incremental BMC max iterations (with --verify) |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent ESBMC verification jobs (with --verify) |

Test discovery: files matching `test_*.vow` or `*_test.vow` under the given directory **and its subdirectories**, sorted alphabetically. Each test must contain `main() -> i32` returning 0 on success.

**Module resolution for directory scans.** When `<path>` is a directory, every discovered test resolves its `use` declarations against `<path>` rather than the test file's own parent directory. This lets internal-unit tests live in a subdirectory like `compiler/tests/test_region.vow` and still `use region;` to import the module under test (which lives at `compiler/region.vow`). Single-file invocations (`vow test path/to/test_foo.vow`) keep the default behaviour of resolving `use` against the file's parent directory.

**Test Output JSON:**

```json
{
  "status": "TestsPassed",
  "total": 3,
  "passed": 3,
  "failed": 0,
  "skipped": 0,
  "tests": [
    {
      "file": "compiler/test_arith.vow",
      "name": "test_arith",
      "status": "passed",
      "exit_code": 0,
      "stdout": "7",
      "stderr": "",
      "duration_ms": 72,
      "diagnostics": [],
      "counterexamples": []
    }
  ],
  "contract_density": {
    "functions_total": 1,
    "functions_with_vows": 0,
    "density_pct": 0.0
  }
}
```

| Status Field   | Meaning                                           |
|----------------|---------------------------------------------------|
| `TestsPassed`  | All tests passed                                  |
| `TestsFailed`  | One or more tests failed                          |

Per-test status: `passed`, `failed`, `timeout`, `compile_error`, `verify_failed`, `contract_skipped`, `skipped`.

`contract_skipped` means ESBMC was never invoked because a vowed function is non-modelable (distinct from `verify_failed`, where ESBMC proved a violation). Both are fail-closed — a `contract_skipped` test counts toward `failed` and yields a `TestsFailed` overall status.

### `vow decl`

Emit declaration file output only.

```
vow decl [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `-o, --output`    | `<source>.vow.d` | Output declaration file path          |

### `vow mutants` (self-hosted only)

Run mutation testing on a Vow source tree. Implemented in the self-hosted compiler only; the Rust bootstrap compiler emits an error pointing the user to `build/vowc`. See `docs/mutants.md` for full details on output schema, mutation kinds, skip-list, and known limitations.

```
vowc mutants version
vowc mutants list  [--root DIR] [--shard X/Y]
vowc mutants run   [--root DIR] [--shard X/Y]
                   [--tier1-cmd 'cmd'] [--tier2-cmd 'cmd']
                   [--tier1-timeout-secs N] [--tier2-timeout-secs N]
                   [--tier2-budget-secs N]
                   [--workdir DIR] [--output-dir DIR] [--force-unlock]
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
| `--workdir` | `/tmp/vow-mutants-<ms>` | Path of the throwaway `git worktree` used for all mutations. |
| `--output-dir` | `mutants.out` | Directory for `mutants.json`, `outcomes.json`, status text files, `diff/`, `logs/`. |
| `--force-unlock` | off | Remove a stale `output_dir/.lock` before starting. |

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

**Exit code.** `0` always, unless a `--max-*` threshold is passed and exceeded, in which case nonzero. With no `--max-*` flag the command is pure reporting — no threshold gates by default (per the decouple-language-from-prover principle).

**Numeric convention.** The non-integer metrics (`halstead.volume`/`difficulty`/`effort` and `score_factors.*`) are emitted as fixed-3-decimal JSON numbers computed in **integer fixed-point** (scale 1000) — never native floats — so both compilers stay byte-identical. `complexity_score` is an integer in `[0, 100]`. The score's saturating anchor map uses a rational curve (`0.800` at the anchor, asymptoting to `1.000`), not an exponential, because the self-hosted compiler has no floating point.

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

`vow build` and `vow verify` both fail closed on `Skipped`: if ESBMC was asked to verify a vowed
function but the verifier could not model the function body, the contract was not statically
proved, so the run exits non-zero. Use `--no-verify` if you genuinely want to skip verification —
that path produces `Unverified` (exit 0).

## Build Output JSON

`vow build` and `vow verify` emit a single JSON object to stdout. Schema: [`schemas/build-result.schema.json`](schemas/build-result.schema.json).

**Note:** `--dump-ir` suppresses JSON output — only IR text is printed.

### Status Values

| Status          | Meaning                                     |
|-----------------|---------------------------------------------|
| `Verified`      | Compiled + every vowed function's contract was statically proved by ESBMC. |
| `Unverified`    | Compiled but ESBMC was not invoked (e.g. `--no-verify`, `--dump-ir`). Exit 0. |
| `Skipped`       | ESBMC was invoked but at least one vowed function could not be modelled (e.g. body uses `RegionAlloc`, `FieldSet`, `Linear*`, `Load`/`Store`, `RemF*`, or has effects). Each such function appears as a `VerificationSkipped` *Warning* in `diagnostics[]`. Their contracts are runtime-checked under `--mode debug` but were not statically proved; the run fails closed with exit 1. |
| `CompileFailed` | Parse error, type error, module load error, or link failure |
| `VerifyFailed`  | ESBMC produced a non-Verified outcome: a counterexample, timeout, `VERIFICATION UNKNOWN` (`verify_status: "unknown"`), tool error, the tool was not found, or the verifier worker thread crashed (`verify_status: "panicked"`). Inspect `counterexamples[]` (definitive failures) and `verify_status`/`verify_message` (soft failures) to distinguish. |

### Verified Example

```json
{
  "status": "Verified",
  "executable": "examples/divide",
  "diagnostics": [],
  "counterexamples": []
}
```

### CompileFailed Example

```json
{
  "status": "CompileFailed",
  "executable": null,
  "diagnostics": [
    {
      "error_code": "TypeMismatch",
      "message": "function body has type `bool` but declared return type is `i32`",
      "severity": "error",
      "span": {
        "file": "bad.vow",
        "offset": 25,
        "length": 8
      }
    }
  ],
  "message": "type error",
  "counterexamples": []
}
```

### VerifyFailed Example

```json
{
  "status": "VerifyFailed",
  "executable": "examples/cegis_broken",
  "diagnostics": [],
  "function": "safe_sub",
  "counterexample": "[Counterexample]",
  "counterexamples": [
    {
      "function": "safe_sub",
      "inputs": { "a": "-9223372036854775808", "b": "0" },
      "violation": "ensures result >= 0",
      "vow_id": 1,
      "source": {
        "file": "examples/cegis_broken.vow",
        "offset": 76,
        "length": 20
      }
    }
  ]
}
```

### Fields Reference

| Field              | Type                | When Present      | Description                               |
|--------------------|---------------------|-------------------|-------------------------------------------|
| `status`           | string              | Always            | One of the four status values             |
| `executable`       | string \| null      | Always            | Path to binary, null on compile failure or library module (no main) |
| `diagnostics`      | array               | Always            | Compiler diagnostics (see schema)         |
| `message`          | string              | CompileFailed     | Error category ("parse error", "type error", "module load error", or link error detail) |
| `function`         | string              | VerifyFailed      | Function where verification failed        |
| `counterexample`   | string              | VerifyFailed      | Legacy description string                 |
| `counterexamples`  | array               | Always            | Structured counterexamples (see schema)   |
| `verify_status`    | string              | On backend failure | `"timeout"`, `"unknown"`, `"error"`, `"tool_not_found"`, or `"panicked"` (verifier worker thread crashed — no counterexample available) |
| `verify_message`   | string              | On backend failure | ESBMC/backend error detail                |

## Contracts Output JSON

`vow contracts` emits a single JSON object to stdout. Schema: [`schemas/contracts-result.schema.json`](schemas/contracts-result.schema.json).

### Example (without --verify)

```json
{
  "contracts": [
    {
      "vow_id": 0,
      "function": "divide",
      "kind": "requires",
      "description": "requires y != 0",
      "blame": "Caller",
      "source": { "file": "divide.vow", "offset": 42 },
      "status": "not_verified",
      "quality": "substantive"
    }
  ],
  "summary": { "total": 1, "proven": 0, "failed": 0, "timeout": 0, "error": 0, "not_verified": 1, "skipped": 0, "vacuous": 0, "trivially_satisfiable": 0, "quality": { "weak": 0, "tautological": 0, "substantive": 1 } }
}
```

### Example (with --verify)

```json
{
  "contracts": [
    {
      "vow_id": 0,
      "function": "divide",
      "kind": "requires",
      "description": "requires y != 0",
      "blame": "Caller",
      "source": { "file": "divide.vow", "offset": 42 },
      "status": "proven",
      "quality": "substantive"
    }
  ],
  "summary": { "total": 1, "proven": 1, "failed": 0, "timeout": 0, "error": 0, "not_verified": 0, "skipped": 0, "vacuous": 0, "trivially_satisfiable": 0, "quality": { "weak": 0, "tautological": 0, "substantive": 1 } }
}
```

### Contract Fields

| Field         | Type    | Description                                              |
|---------------|---------|----------------------------------------------------------|
| `vow_id`      | integer | Unique contract identifier within the program            |
| `function`    | string  | Function containing this contract                        |
| `kind`        | string  | `"requires"`, `"ensures"`, or `"invariant"`              |
| `description` | string  | Full contract text                                       |
| `blame`       | string  | `"Caller"` (requires) or `"Callee"` (ensures/invariant)  |
| `source`      | object  | `{ "file": string, "offset": integer }`                  |
| `status`      | string  | `"proven"`, `"proven-ir"`, `"failed"`, `"unknown"`, `"timeout"`, `"error"`, `"not_verified"`, `"skipped"`, or `"vacuous"` |
| `quality`     | string  | Static clause-shape classification (no ESBMC): `"weak"`, `"tautological"`, or `"substantive"` |
| `trivially_satisfiable` | bool | `--verify` only: true when a trivial `return <default>` body still satisfies this `ensures` (verification-confirmed weakness). Always false for `requires`/`invariant` and without `--verify`. Informational — never affects the exit code. See `docs/spec/contracts-methodology.md`. |

### Status Values

| Status          | Meaning                                              |
|-----------------|------------------------------------------------------|
| `not_verified`  | Verification not requested (no `--verify` flag)      |
| `proven`        | ESBMC proved this contract holds for all inputs (bit-vector encoding, overflow modeled) |
| `proven-ir`     | ESBMC proved this contract under integer-arithmetic encoding after BV timed out; overflow is not modeled by IR, but the BV caller preconditions still guard against it |
| `failed`        | ESBMC found a counterexample violating this contract |
| `unknown`       | ESBMC could not conclude for this contract — either `VERIFICATION UNKNOWN` was reported for the containing function (k-induction's forward condition unable to prove or falsify), or the function's verification failed overall and ESBMC's per-clause `--multi-property` run returned no individual verdict for this clause |
| `timeout`       | ESBMC timed out on the containing function (BV and — when applicable — IR fallback both timed out) |
| `error`         | ESBMC error or tool not found                        |
| `skipped`       | The containing function's body uses opcodes the verifier cannot model (e.g. `RegionAlloc` from struct construction). Contract is documentary; runtime checks still apply under `--mode debug`. Surfaces as a `VerificationSkipped` Warning in the build JSON's `diagnostics[]` and lifts the overall build/verify status to `Skipped` (fail-closed, exit 1) — use `--no-verify` if you want a non-failing path that does not invoke ESBMC at all. |
| `vacuous`       | The containing function's `requires` clauses are contradictory, so every `ensures` is satisfied vacuously — ESBMC proved nothing of substance (antecedent failure). Detected by a second ESBMC run with `--error-label`: a `vow_reach` label planted after the `requires` assumes is unreachable. All of the function's clauses are reported `vacuous` (fail-closed, exit 1). See `docs/spec/contracts-methodology.md`. |

### Quality Values

`quality` is a static classification of each clause's *shape*, computed without ESBMC and independent of `status`. It surfaces the "proven but trivial" problem: a `weak` contract can be `proven` while constraining almost nothing. See `docs/spec/contracts-methodology.md` for the full taxonomy.

| Quality        | Meaning                                                                                      |
|----------------|----------------------------------------------------------------------------------------------|
| `weak`         | An `ensures` that only bounds `result` by an integer literal (e.g. `result >= 0`). Satisfied by almost any implementation. |
| `tautological` | A constant clause that references no program value (e.g. `true`, `0 >= 0`). Constrains nothing. |
| `substantive`  | Everything else — equality, relational, inverse/round-trip, dispatch-totality, or function-call shapes. The classifier is conservative: anything not provably weak/tautological is reported `substantive`. |

## Trace Output (stderr, --debug-trace)

When `--debug-trace=calls` or `--debug-trace=full` is used, the compiled binary emits JSON lines to stderr:

### calls mode
```json
{"event":"enter","fn":"main"}
{"event":"enter","fn":"divide"}
{"event":"exit","fn":"divide"}
{"event":"exit","fn":"main"}
```

### full mode (adds vow check results)
```json
{"event":"enter","fn":"divide"}
{"event":"vow","fn":"divide","vow_id":0,"passed":true}
{"event":"exit","fn":"divide"}
```

## Profile Output (stderr, profile mode)

When `--mode profile` is used, the compiled binary prints a call-count report to stderr on normal exit (via `atexit`). The report is not printed if the program is killed by a signal or calls `abort()`.

```
--- vow profile report ---
function                                        calls       %
-------------------------------------------------------------
infer                                         4812399   48.2%
is_def_eq_core                                3201882   32.1%
whnf                                           984201    9.9%
main                                                1    0.0%
-------------------------------------------------------------
total calls: 9998483, unique functions: 12
```

The report lists the top 20 most-called functions sorted by call count. No vow checks are emitted in profile mode.

## Runtime Error JSON (stderr, debug/sanitize mode)

When a compiled program runs in debug mode (`--mode debug`) or sanitize mode (`--mode sanitize`) and violates a vow at runtime, it emits JSON to stderr before aborting.

### VowViolation

```json
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"divide.vow","offset":42,"values":{"y":0}}
```

Schema: [`schemas/vow-violation.schema.json`](schemas/vow-violation.schema.json).

### ArithmeticOverflow

```json
{"error":"ArithmeticOverflow"}
```

Emitted when a checked arithmetic operator (`+!`, `-!`, etc.) overflows at runtime.

### UnwrapOnNone

```json
{"error":"UnwrapOnNone"}
```

Emitted when `.unwrap()` is called on `Option::None`.

### IndexOutOfBounds

```json
{"error":"IndexOutOfBounds"}
```

Emitted when a `Vec` index is out of bounds.

### UseAfterFree (sanitize mode only)

```json
{"error":"UseAfterFree","op":"push","vec":"0x55a1b2c3d4e0"}
```

Emitted when a Vec operation is attempted on a Vec that has already been freed.

### DoubleFree (sanitize mode only)

```json
{"error":"DoubleFree","vec":"0x55a1b2c3d4e0"}
```

Emitted when a Vec is freed twice.

### StaleIndex (sanitize mode only)

```json
{"error":"StaleIndex","index":5,"expected_gen":3,"actual_gen":7,"vec":"0x55a1b2c3d4e0"}
```

Emitted when `__vow_sanitize_check_generation` detects that a Vec slot's generation counter does not match the expected value, indicating the slot was overwritten since the index was recorded.

## Agent Decision Tree

```
Parse JSON from stdout
├── status == "Verified"       → Success. Binary at `executable`.
├── status == "Unverified"     → Compiled but unverified. ESBMC missing or --no-verify.
├── status == "CompileFailed"  → Read `diagnostics[]` for error details.
│   ├── error_code is parse error  → Fix syntax (see grammar.md)
│   └── error_code is type error   → Fix types (see errors.md)
└── status == "VerifyFailed"   → Read `counterexamples[]`.
    ├── Check `inputs` for the violating values
    ├── Check `violation` for which contract failed
    ├── Check `source` for the location
    └── Fix the contract or the implementation, then rebuild
```

Always check stderr for human-readable diagnostics alongside the JSON on stdout.
