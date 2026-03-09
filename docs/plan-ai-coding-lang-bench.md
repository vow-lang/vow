# Plan: Vow in the AI Coding Language Benchmark

## Background

The [ai-coding-lang-bench](https://github.com/mame/ai-coding-lang-bench) by @mame
measures how efficiently Claude Code can implement a **mini-git** (simplified Git
clone) across 13+ programming languages. It tracks wall-clock time, API cost,
lines of code, and pass rate across 20 independent trials per language, each with
two phases (v1: core commands, v2: extended commands).

### Current leaderboard (top 5)

| Rank | Language   | Time (v1+v2) | Cost  | LOC | Pass Rate |
|------|------------|--------------|-------|-----|-----------|
| 1    | Ruby       | 73.1s ± 4.2  | $0.36 | 219 | 40/40     |
| 2    | Python     | 74.6s ± 4.5  | $0.38 | 235 | 40/40     |
| 3    | JavaScript | 81.1s ± 5.0  | $0.39 | 248 | 40/40     |
| 4    | Go         | 101.6s ± 37  | $0.50 | 324 | 40/40     |
| 5    | Rust       | 113.7s ± 55  | $0.54 | 303 | 38/40     |

Dynamic languages win on speed/cost. Rust and Haskell are the only languages with
failures (38/40 and 39/40 respectively). Static type systems add overhead in both
time and cost.

### Why Vow should excel

Vow is **designed for agentic development**. Its key advantages:

1. **Dual-output diagnostics** — JSON (for agents) + human-readable (for humans),
   always emitted in parallel. Claude Code can parse structured JSON errors
   directly, not guess from text.
2. **Blame tracking** — `requires` violations blame the Caller, `ensures` blame
   the Callee. The agent gets precise blame attribution, not just "assertion
   failed."
3. **Contracts as first-class semantics** — preconditions, postconditions, and
   loop invariants are language primitives, not comments or annotations. The
   compiler enforces them. This should reduce logic bugs that waste regen cycles.
4. **Explicit effects** — `[io]`, `[read]`, `[write]` effect annotations
   prevent accidental side effects. The type checker catches effect violations
   before runtime.
5. **Concise syntax** — no comments in source (intent lives in contracts),
   minimal boilerplate, struct-based data modeling.
6. **Formal verification available** — `vow verify` can prove correctness of
   pure functions, potentially catching bugs that tests miss.

The hypothesis: Vow's agent-friendly diagnostics and contract system should yield
**higher pass rates** and **fewer wasted iterations**, even if raw token count is
slightly higher than Ruby/Python due to type annotations and contracts.

---

## What the Benchmark Requires

### Task: mini-git

A simplified Git implementation with filesystem operations, string processing,
custom hashing, and structured output.

**Phase v1 — Core commands (from empty directory):**
- `minigit init` — create `.minigit/` directory structure
- `minigit add <file>` — stage a file
- `minigit commit -m "<message>"` — create commit with custom hash
- `minigit log` — display commit history

**Phase v2 — Extended commands (modify existing code):**
- `minigit status` — show staged/modified/untracked files
- `minigit diff <commit1> <commit2>` — diff two commits
- `minigit checkout <commit_hash>` — restore working tree to a commit
- `minigit reset <commit_hash>` — reset HEAD to a commit
- `minigit rm <file>` — remove a file from staging
- `minigit show <commit_hash>` — show commit details

**Key constraints:**
- No external libraries (custom FNV-like hash, not SHA-256)
- Deterministic output (sorted filenames, exact formatting)
- Exit code 0 on success, 1 on error
- Tests are shell scripts (`test-v1.sh`, `test-v2.sh`)

### Required language capabilities

| Capability                      | Status in Vow | Gap?                            |
|---------------------------------|---------------|---------------------------------|
| Read/write files                | `fs_read` / `fs_write` | Partial — see gaps below |
| Create directories              | **Missing**   | Need `fs_mkdir`                 |
| List directory contents         | **Missing**   | Need `fs_listdir`               |
| Check file/dir existence        | **Missing**   | Need `fs_exists` / `path_exists`|
| Delete files                    | **Missing**   | Need `fs_remove`                |
| Get file metadata (timestamps)  | **Missing**   | Need for status/diff            |
| Command-line arguments          | `args()` ✓    | Works                           |
| String manipulation             | Partial ✓     | Have: eq, contains, push_str, byte_at, from_i64 |
| String splitting/substring      | **Missing**   | Need `string_split`, `string_substr`, `string_starts_with` |
| String to integer parsing       | **Missing**   | Need `parse_i64`                |
| Integer bitwise XOR             | **Missing**   | Need `^` operator or `bxor`     |
| Integer multiplication (wrapping u64) | Have `*` | Need unsigned 64-bit wrapping semantics for hash |
| Process exit with code          | `process_exit` ✓ | Works                        |
| Print to stdout/stderr          | `print_str`, `eprintln_str` ✓ | Works             |
| HashMap                         | `HashMap<String, String>` | Need string-keyed maps (currently i64-keyed) |
| Time (unix epoch)               | **Missing**   | Need `time_unix` or similar     |
| Executable entry point          | `fn main() -> i32 [io]` ✓ | Works                |
| Multi-module projects           | `module`/`use` ✓ | Works                          |

---

## Implementation Plan

### Phase 0: Language gaps (prerequisite runtime/compiler work)

These are the features Vow needs before it can participate in the benchmark.
Each is a concrete work item.

#### 0.1 Filesystem builtins

Add to `vow-runtime`, `vow-ir`, `vow-types`, `vow-codegen`, `vow-clif-shim`:

| Builtin            | Signature                          | Effect    | Runtime impl |
|--------------------|------------------------------------|-----------|--------------|
| `fs_mkdir`         | `(path: String) -> i64`            | `[write]` | `std::fs::create_dir_all` |
| `fs_exists`        | `(path: String) -> bool`           | `[read]`  | `std::path::Path::exists` |
| `fs_listdir`       | `(path: String) -> Vec<String>`    | `[read]`  | `std::fs::read_dir` |
| `fs_remove`        | `(path: String) -> i64`            | `[write]` | `std::fs::remove_file` |
| `fs_remove_dir`    | `(path: String) -> i64`            | `[write]` | `std::fs::remove_dir_all` |
| `fs_is_dir`        | `(path: String) -> bool`           | `[read]`  | `Path::is_dir` |
| `fs_rename`        | `(from: String, to: String) -> i64`| `[write]` | `std::fs::rename` |

Each requires changes in ~5 files following the existing `fs_read`/`fs_write` pattern:
1. `vow-runtime/src/lib.rs` — extern "C" function
2. `vow-types/src/env.rs` — type signature registration
3. `vow-ir/src/lower/mod.rs` — name → symbol mapping
4. `vow-codegen/src/cranelift_backend.rs` — Cranelift call lowering
5. `vow-clif-shim/src/lib.rs` — self-hosted compiler FFI

#### 0.2 String builtins

| Builtin                | Signature                                     | Effect | Runtime impl |
|------------------------|-----------------------------------------------|--------|--------------|
| `string_substr`        | `(s: String, start: i64, len: i64) -> String` | pure   | slice + copy |
| `string_split`         | `(s: String, delim: String) -> Vec<String>`   | pure   | `str::split`  |
| `string_starts_with`   | `(s: String, prefix: String) -> bool`         | pure   | `str::starts_with` |
| `string_ends_with`     | `(s: String, suffix: String) -> bool`         | pure   | `str::ends_with` |
| `string_trim`          | `(s: String) -> String`                       | pure   | `str::trim`   |
| `string_replace`       | `(s: String, from: String, to: String) -> String` | pure | `str::replace` |
| `string_char_at`       | `(s: String, idx: i64) -> String`             | pure   | single-char string |
| `string_index_of`      | `(s: String, needle: String) -> i64`          | pure   | `str::find`, -1 if not found |
| `parse_i64`            | `(s: String) -> i64`                          | pure   | `str::parse`  |
| `string_to_lower`      | `(s: String) -> String`                       | pure   | `str::to_lowercase` |
| `string_join`          | `(v: Vec<String>, sep: String) -> String`     | pure   | `join`        |

#### 0.3 Integer bitwise operations

The mini-git hash requires XOR and wrapping multiplication:

| Feature          | Syntax/Builtin          | Notes |
|------------------|-------------------------|-------|
| Bitwise XOR      | `bxor(a, b)` or `a ^ b` | New operator or builtin |
| Wrapping mul     | `*` already wraps       | Verify i64 wrapping semantics match u64 mod 2^64 |
| Integer to hex   | `i64_to_hex(n) -> String` | For hash output (16-char lowercase hex) |

#### 0.4 String-keyed HashMap

Currently Vow's HashMap is `HashMap<i64, i64>` (keys and values are i64).
Mini-git needs `HashMap<String, String>` for index tracking, commit metadata, etc.

Options:
- **A) Generalize HashMap** to support String keys/values (significant type system work)
- **B) Add a separate `StringMap`** type with dedicated builtins (`smap_new`, `smap_insert`, `smap_get`, `smap_contains`, `smap_remove`, `smap_keys`)
- **C) Use Vec<String> pairs** (simpler but O(n) lookup)

Recommendation: **Option B** — fastest to implement, avoids type system changes, directly parallels existing `map_*` builtins.

#### 0.5 Time builtin

| Builtin      | Signature        | Effect   | Runtime impl |
|--------------|------------------|----------|--------------|
| `time_unix`  | `() -> i64`      | `[read]` | `SystemTime::now().duration_since(UNIX_EPOCH)` |

#### 0.6 Vec sorting

| Builtin       | Signature                    | Effect | Notes |
|---------------|------------------------------|--------|-------|
| `vec_sort`    | `(v: Vec<String>) -> Vec<String>` | pure | Sort strings alphabetically |
| `vec_sort_i64`| `(v: Vec<i64>) -> Vec<i64>`  | pure   | Sort integers |

---

### Phase 1: Benchmark harness integration

#### 1.1 Fork and extend the benchmark repo

Fork `mame/ai-coding-lang-bench` and add Vow as a new language configuration:

```ruby
# In benchmark.rb, add language config:
{
  name: "Vow",
  ext: ["vow"],
  version_cmd: "./target/release/vow --version",
  extra_prompt: <<~PROMPT
    Write the implementation in Vow. The executable entry point is `fn main() -> i32 [io]`.
    Use `module MiniGit` at the top. Compile with: vow build --no-verify minigit.vow -o minigit
    Include a build.sh that runs this command. The binary must be named 'minigit'.
    Vow has: fs_read, fs_write, fs_mkdir, fs_exists, fs_listdir, fs_remove,
    fs_is_dir, args(), print_str, eprintln_str, process_exit, string_*, bxor,
    i64_to_hex, parse_i64, time_unix, StringMap (smap_*), Vec<String>, Vec<i64>.
    Effects must be annotated: [io] for print, [read] for fs_read/args, [write] for fs_write/fs_mkdir.
    Use contracts (vow { requires: ..., ensures: ... }) where appropriate.
  PROMPT
}
```

#### 1.2 Vow skill docs for Claude Code

Create a concise Vow language reference that goes into the CLAUDE.md of the
benchmark working directory (or as a separate file loaded by the prompt). This
should cover:
- All builtin functions with signatures and effects
- Struct syntax, Vec/HashMap usage
- Module system (`module`, `use`)
- Contract syntax
- Common patterns (string processing, file I/O, error handling)

This is critical — the benchmark's article notes that **training data volume**
is a major factor. Vow has zero training data in Claude's pretraining. We must
compensate with excellent in-context documentation.

#### 1.3 Build script template

Since Vow is compiled, we need a `build.sh` or `Makefile` pattern:

```bash
#!/bin/bash
vow build --no-verify *.vow -o minigit
```

The `--no-verify` flag skips ESBMC (verification is slow and unnecessary for
the benchmark's functional tests). We want raw compile speed.

---

### Phase 2: Benchmark execution

#### 2.1 Pilot runs (5 trials)

Run 5 trials to identify:
- Common failure modes (missing builtins, string edge cases)
- Average time and cost
- Whether Claude Code can figure out the Vow build workflow
- Whether the skill docs are sufficient

#### 2.2 Iterate on gaps

Based on pilot failures:
- Add any missing builtins discovered during runs
- Refine the skill docs / extra prompt
- Fix any compiler bugs exposed by the mini-git workload

#### 2.3 Full benchmark (20 trials)

Run the full 20-trial suite matching the original methodology:
- Claude Code (Opus 4.6) with `--dangerously-skip-permissions`
- JSON output parsing for metrics
- Both v1 and v2 phases

#### 2.4 Optional: Verified variant

Run a separate "Vow/verified" configuration that uses `vow build` (with
verification enabled) instead of `--no-verify`. This demonstrates Vow's unique
value: **the same code that passes tests is also formally verified**. Even if
it's slower, it's a capability no other language in the benchmark offers.

Metrics to highlight:
- How many contracts does Claude add spontaneously?
- Do contracts catch bugs before tests do?
- What's the verification overhead vs. the base compile time?

---

### Phase 3: Reporting and comparison

#### 3.1 Metrics to collect

| Metric              | How                                          |
|---------------------|----------------------------------------------|
| Wall-clock time     | v1 + v2 total, mean ± stddev over 20 trials  |
| API cost            | From Claude Code JSON output                 |
| Lines of code       | Count `.vow` files, exclude build scripts    |
| Pass rate           | X/40 (20 trials × 2 phases)                  |
| Contracts written   | Count `vow { ... }` blocks in generated code |
| Verification time   | For the verified variant only                 |
| Failure categories  | Type error, effect violation, runtime crash, wrong output |

#### 3.2 Expected positioning

**Optimistic scenario (Vow beats Ruby/Python):**
- Agent-friendly JSON diagnostics eliminate wasted regeneration cycles
- Contracts prevent logic bugs that cause test failures
- Explicit effects catch side-effect bugs at compile time
- Net result: fewer iterations → faster wall-clock, lower cost

**Realistic scenario (Vow lands mid-pack, near Go/Java):**
- Zero pretraining data means Claude relies entirely on in-context docs
- Compilation step adds overhead vs. interpreted languages
- Type annotations + effect annotations add LOC
- But: 40/40 pass rate (contracts + types catch bugs before tests)

**Pessimistic scenario (Vow lands near Haskell/C):**
- Insufficient in-context documentation
- Missing builtins force workarounds
- Compile errors from unfamiliar syntax waste iterations

**Key differentiator regardless of ranking:**
Vow is the only language that can offer **verified** implementations. Even if
Vow/basic is mid-pack, Vow/verified is a category of one.

---

## Work Breakdown and Effort Estimates

### Critical path (must complete before any benchmark runs)

| Item | Description | Complexity |
|------|-------------|------------|
| 0.1  | Filesystem builtins (7 functions × 5 files) | Medium — follows existing pattern |
| 0.2  | String builtins (11 functions × 5 files) | Medium — follows existing pattern |
| 0.3  | Bitwise XOR + hex conversion | Small — 2 builtins |
| 0.4  | StringMap type | Medium — new type, ~6 builtins |
| 0.5  | Time builtin | Small — 1 function |
| 0.6  | Vec sorting | Small — 2 builtins |
| 1.2  | Skill docs for Claude Code | Medium — must be comprehensive |

### Benchmark harness work

| Item | Description | Complexity |
|------|-------------|------------|
| 1.1  | Fork + add Vow config | Small |
| 1.3  | Build script template | Small |
| 2.1  | Pilot runs (5 trials) | Small — but may expose gaps |
| 2.2  | Iterate on gaps | Unknown — depends on pilot |
| 2.3  | Full benchmark (20 trials) | Small — just run it |
| 2.4  | Verified variant runs | Small — config change |
| 3.1  | Reporting | Small |

### Stretch goals

| Item | Description | Why |
|------|-------------|-----|
| S1   | Multi-file module support in benchmark | Claude might want to split minigit into modules |
| S2   | Contract-aware test generation | Generate Vow tests from contracts |
| S3   | Comparison with Vericoding benchmark | Cross-reference verification benchmark results |

---

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Zero training data for Vow | Claude may struggle with syntax | Comprehensive skill docs in prompt (~5KB) |
| Missing builtins discovered mid-run | Benchmark stalls | Pilot runs (Phase 2.1) to find gaps early |
| HashMap generics too complex | Blocks string-keyed maps | Use StringMap workaround (Option B) |
| Compile time slower than interpreted | Higher wall-clock | Use `--no-verify`, optimize Cranelift pipeline |
| Vow binary not on PATH | Build script complexity | Ship pre-built `vow` binary in benchmark dir |
| ESBMC timeout on verified variant | Verified runs too slow | Use `--no-verify` for base, verified is separate |
| Test scripts expect specific output format | String formatting bugs | Add `string_format` or ensure `string_from_i64` + concat suffice |

---

## Success Criteria

1. **Minimum viable**: Vow achieves ≥ 38/40 pass rate (matching Rust)
2. **Good**: Vow achieves 40/40 pass rate with time < 120s (beating Go, matching Java)
3. **Excellent**: Vow achieves 40/40 with time < 90s (near Ruby/Python tier)
4. **Unique win**: Vow/verified achieves 40/40 with formal proofs on key functions

The real story isn't just speed — it's that Vow is the only language where the
benchmark output is **both tested and verified**. That's the narrative advantage
even if raw speed is mid-pack.
