# Plan: `vowc test` Subcommand

## Overview

Implement a `test` subcommand for the self-hosted `vowc` compiler (and the Rust `vow` CLI) that discovers, compiles, runs, and reports on Vow test files — with structured JSON output including coverage data.

## Design Principles

Per CLAUDE.md, Vow is a language for agents. The test subcommand:
- **No new language syntax** — tests are normal `.vow` programs with `main() -> i32`. No `#[test]` attribute or test-specific grammar. This adds zero verifier complexity.
- **Convention-based discovery** — test files are `test_*.vow` or `*_test.vow` in a given directory.
- **Structured JSON output** — agents parse results programmatically, just like `build`/`verify`.
- **Coverage = contract coverage** — reports which functions have vow blocks vs. which don't, giving a "specification coverage" metric that matters for verified code.

## Test Semantics

A **test file** is a `.vow` file matching the naming convention that contains a `main() -> i32` function. The test **passes** if:
1. It compiles successfully.
2. (Optional) Its contracts verify via ESBMC.
3. The compiled binary exits with code `0`.

A non-zero exit code means failure. Compile errors and verification failures are also failures.

## JSON Output Schema

```json
{
  "status": "TestsPassed" | "TestsFailed" | "CompileFailed",
  "total": 5,
  "passed": 4,
  "failed": 1,
  "skipped": 0,
  "tests": [
    {
      "file": "compiler/test_arith.vow",
      "name": "test_arith",
      "status": "passed" | "failed" | "skipped" | "compile_error" | "verify_failed",
      "exit_code": 0,
      "duration_ms": 120,
      "diagnostics": [],
      "counterexamples": []
    }
  ],
  "coverage": {
    "functions_total": 42,
    "functions_with_vows": 15,
    "vow_coverage_pct": 35.7
  }
}
```

## CLI Interface

```
vowc test [OPTIONS] [<path>]
```

| Flag | Default | Description |
|------|---------|-------------|
| `<path>` | `.` | Directory to scan for test files (or single `.vow` file) |
| `--no-verify` | (off) | Skip ESBMC verification of test files |
| `--filter <pattern>` | (none) | Only run tests whose name contains this substring |
| `--mode debug\|release` | `debug` | Build mode (debug recommended — enables runtime vow checks) |
| `--unwind <N>` | `10` | ESBMC loop unwind bound |

Default mode is `debug` (unlike `build` which defaults to `release`) because tests should catch runtime vow violations.

## Implementation Steps

### Step 1: Rust CLI — Wire up `TestArgs` and test runner

**Files:** `vow/src/main.rs`

- Expand `TestArgs` struct with new flags: `path` (optional dir/file), `no_verify`, `filter`, `mode`, `unwind`.
- Implement `run_test_command()`:
  1. **Discovery**: Glob `test_*.vow` and `*_test.vow` in the given directory (non-recursive by default).
  2. **Filter**: If `--filter` is set, keep only matching names.
  3. **For each test file**:
     a. Run the frontend (lex, parse, type-check) — collect diagnostics.
     b. If `--no-verify` is not set, run verification — collect counterexamples.
     c. Compile to a temp binary.
     d. Execute the binary with `ulimit -v 2000000`, capture exit code and timing.
     e. Record pass/fail/compile_error/verify_failed status.
  4. **Coverage**: Walk all parsed ASTs across test files, count functions total and functions with `vow {}` blocks.
  5. **Output**: Emit the JSON result object to stdout.
  6. **Exit code**: 0 if all tests pass, 1 otherwise.
- Replace the "not yet implemented" stub with the real implementation.

### Step 2: Self-hosted CLI — Implement test runner in `main.vow`

**Files:** `compiler/main.vow`

- Add `run_test()` function following the same pattern as `run_build()` and `run_verify()`.
- Implement test discovery:
  - Use the existing file I/O to list directory contents (will need a new `__vow_readdir` FFI shim, or accept an explicit file list via repeated args).
  - **Pragmatic alternative**: Accept explicit test file paths as arguments (`vowc test file1.vow file2.vow`) and/or a directory path. For directory scanning, add a minimal `__vow_readdir` builtin to `vow-runtime` that returns a `Vec<String>` of filenames.
- For each test file:
  - Run frontend via `run_frontend()`.
  - Optionally verify via the existing verification pipeline.
  - Compile to a temp path via the existing codegen pipeline.
  - Execute via `__vow_system()` or a new `__vow_exec` FFI call, capture exit code.
- Compute coverage from the parsed ASTs (count functions, count functions with vow blocks).
- Emit structured JSON to stdout.
- Replace the `"test: not yet implemented"` stub.

### Step 3: Add `__vow_readdir` runtime FFI shim

**Files:** `vow-runtime/src/lib.rs` (or new file), `vow-clif-shim/src/lib.rs`

- Add `extern "C" fn __vow_readdir(path: *const u8, path_len: usize) -> VowVec` that returns filenames in a directory as a `Vec<String>` (flattened to VowVec of VowVec).
- **Simpler alternative**: Add `__vow_glob(pattern_ptr, pattern_len) -> VowVec<String>` that does glob matching and returns matching paths. This is more useful and handles the `test_*.vow` convention directly.
- Register in the clif shim's function table so the self-hosted compiler can call it.

### Step 4: Add test files for the compiler itself

**Files:** `compiler/test_*.vow` (already 3 exist), add more

- Ensure existing `test_arith.vow`, `test_assign.vow`, `test_while.vow` follow the convention (they already do — each has `main() -> i32` returning 0).
- Add a few more meaningful tests:
  - `test_string.vow` — string operations
  - `test_vec.vow` — vector operations
  - `test_struct.vow` — struct creation and field access
  - `test_vow.vow` — a test with vow contracts that should verify
- Each test prints nothing on success, returns 0 on pass, non-zero on fail.

### Step 5: Update `--help` and skill docs

**Files:** `docs/skill/cli.md`, `compiler/main.vow` (help strings), `vow/src/main.rs` (help strings), `scripts/generate_help.py`

- Add `vow test` section to `docs/skill/cli.md` with flags, JSON schema, and examples.
- Update `skill_json()` and `skill_human()` in both compilers.
- Run `uv run python scripts/generate_help.py` to regenerate.
- Run `scripts/check_help_coverage.py` to verify no drift.

### Step 6: Add `vowc test` to `full_test.sh`

**Files:** `scripts/full_test.sh`

- Add a new **Section 10: Test Subcommand** that:
  1. Runs `$RUST test compiler/` and `run_self test compiler/`.
  2. Compares JSON output (status, pass/fail counts).
  3. Verifies coverage data is present and sane.

### Step 7: Update `TestResult` JSON schema

**Files:** `docs/schemas/test-result.schema.json` (new)

- Add a formal JSON schema for the test output, parallel to `build-result.schema.json`.

## Ordering and Dependencies

```
Step 3 (FFI shim)  ──→  Step 2 (self-hosted)  ──→  Step 6 (full_test.sh)
                                                          ↑
Step 1 (Rust CLI)  ──────────────────────────────────────┘
Step 4 (test files) ─── can be done in parallel ──────────┘
Step 5 (docs)      ─── after Steps 1 & 2 ────────────────┘
Step 7 (schema)    ─── after Step 1 ──────────────────────┘
```

Steps 1 and 3 can be developed in parallel. Step 4 is independent.

## Scope and Non-Goals

**In scope:**
- Test discovery, compilation, execution, and reporting
- Contract coverage metrics (functions with vow blocks / total functions)
- Structured JSON output for agents
- Both Rust and self-hosted compiler implementations

**Not in scope (may be future work):**
- Line-level code coverage (would require instrumentation — significant complexity)
- Parallel test execution (sequential is fine for now)
- Test-specific language syntax (`#[test]`, `assert_eq!` macros, etc.)
- Watch mode / re-run on file change
- Recursive directory scanning (can be added later with a `--recursive` flag)

## Risk Assessment

- **FFI shim for directory listing (Step 3)**: Low risk — similar pattern to existing `__vow_string_*` and `__vow_vec_*` shims. If too complex, the self-hosted version can accept explicit file paths instead of directory scanning.
- **Coverage counting**: Low risk — just counting AST nodes, no new type-system or verification surface.
- **No new grammar**: Zero risk to verifier — tests are normal programs.
