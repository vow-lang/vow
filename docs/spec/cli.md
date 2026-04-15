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
| `--no-cache`    | (off)       | Disable compile and verify caching           |
| `--unwind <N>`  | `10`        | ESBMC loop unwind bound                      |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver; auto selects per-function via heuristic |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 |
| `--timeout <N>` | (none)      | ESBMC per-function timeout in seconds        |

### `vow verify`

Verify contracts only — no executable output. Emits the same JSON format as `vow build` but `executable` is always `null`.

```
vow verify [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `--no-cache`      | (off)       | Disable verification result caching        |
| `--unwind <N>`    | `10`        | ESBMC loop unwind bound                   |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver; auto selects per-function via heuristic |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 |
| `--timeout <N>` | (none)      | ESBMC per-function timeout in seconds        |

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
| `--unwind <N>`    | `10`        | ESBMC loop unwind bound                   |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver (with --verify)           |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode (with --verify); ir requires z3 |

### `vow skill`

Generate or install the Claude Code skill document for the current compiler version. The skill is embedded in the compiler binary, ensuring the documentation always matches the installed toolchain.

```
vow skill              # print skill document to stdout (default: print)
vow skill print        # same as above
vow skill install      # install to .claude/commands/vow-toolchain.md
```

`print` writes the complete skill markdown (with YAML frontmatter) to stdout. Pipe to a file or use `install` to place it directly.

`install` creates `.claude/commands/` in the current directory if needed and writes the skill document there. Claude Code discovers it automatically.

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
| `--filter <pat>`  | (none)      | Only run tests whose name contains pat     |
| `--mode debug`    | (default)   | Insert runtime vow checks                 |
| `--mode release`  | `debug`     | Omit all vow checks for performance       |
| `--timeout <ms>`  | `30000`     | Per-test execution timeout in milliseconds |
| `--unwind <N>`    | `10`        | ESBMC loop unwind bound (with --verify)    |

Test discovery: files matching `test_*.vow` or `*_test.vow` in the given directory, sorted alphabetically. Each test must contain `main() -> i32` returning 0 on success.

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

Per-test status: `passed`, `failed`, `timeout`, `compile_error`, `verify_failed`, `skipped`.

### `vow decl`

Emit declaration file output only.

```
vow decl [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `-o, --output`    | `<source>.vow.d` | Output declaration file path          |

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

| Code | Meaning                              |
|------|--------------------------------------|
| `0`  | Success (Verified or Unverified)     |
| `1`  | Failure (CompileFailed or VerifyFailed) |

## Build Output JSON

`vow build` and `vow verify` emit a single JSON object to stdout. Schema: [`schemas/build-result.schema.json`](schemas/build-result.schema.json).

**Note:** `--dump-ir` suppresses JSON output — only IR text is printed.

### Status Values

| Status          | Meaning                                     |
|-----------------|---------------------------------------------|
| `Verified`      | Compiled + all contracts proved by ESBMC     |
| `Unverified`    | Compiled with `--no-verify` (ESBMC skipped)  |
| `CompileFailed` | Parse error, type error, module load error, or link failure |
| `VerifyFailed`  | ESBMC found a counterexample, timed out, errored, or was not found |

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
| `verify_status`    | string              | On backend failure | `"timeout"`, `"error"`, or `"tool_not_found"` |
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
      "status": "not_verified"
    }
  ],
  "summary": { "total": 1, "proven": 0, "failed": 0, "timeout": 0, "error": 0, "not_verified": 1 }
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
      "status": "proven"
    }
  ],
  "summary": { "total": 1, "proven": 1, "failed": 0, "timeout": 0, "error": 0, "not_verified": 0 }
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
| `status`      | string  | `"proven"`, `"failed"`, `"unknown"`, `"timeout"`, `"error"`, or `"not_verified"` |

### Status Values

| Status          | Meaning                                              |
|-----------------|------------------------------------------------------|
| `not_verified`  | Verification not requested (no `--verify` flag)      |
| `proven`        | ESBMC proved this contract holds for all inputs      |
| `failed`        | ESBMC found a counterexample violating this contract |
| `unknown`       | Another contract in the same function failed; this one was not individually checked |
| `timeout`       | ESBMC timed out on the containing function           |
| `error`         | ESBMC error or tool not found                        |

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
