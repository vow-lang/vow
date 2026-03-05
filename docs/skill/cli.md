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
| `-o, --output`    | source stem | Output executable path                     |
| `--mode debug`    | `release`   | Insert runtime vow checks                 |
| `--mode release`  | (default)   | Omit all vow checks for performance       |
| `--no-verify`     | (off)       | Skip ESBMC static verification            |
| `--dump-ir`       | (off)       | Print IR text to stdout and exit (no JSON output, no codegen) |

### `vow verify`

Verify contracts only — no executable output. Emits the same JSON format as `vow build` but `executable` is always `null`.

```
vow verify <source.vow>
```

### `vow test`

Not yet implemented.

### `vow --help`

```
vow --help          # JSON capability description (for agents)
vow --help --human  # human-readable text
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
| `Unverified`    | Compiled but ESBMC was skipped or not found  |
| `CompileFailed` | Parse error, type error, module load error, or link failure |
| `VerifyFailed`  | Compiled but ESBMC found a counterexample    |

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
| `executable`       | string \| null      | Always            | Path to binary, null on compile failure   |
| `diagnostics`      | array               | Always            | Compiler diagnostics (see schema)         |
| `message`          | string              | CompileFailed     | Error category ("parse error", "type error", "module load error", or link error detail) |
| `function`         | string              | VerifyFailed      | Function where verification failed        |
| `counterexample`   | string              | VerifyFailed      | Legacy description string                 |
| `counterexamples`  | array               | Always            | Structured counterexamples (see schema)   |
| `verify_status`    | string              | On timeout/error  | "timeout" or "error"                      |
| `verify_message`   | string              | On error          | ESBMC error message                       |

## Runtime Error JSON (stderr, debug mode only)

When a compiled program runs in debug mode (`--mode debug`) and violates a vow at runtime, it emits JSON to stderr before aborting.

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
