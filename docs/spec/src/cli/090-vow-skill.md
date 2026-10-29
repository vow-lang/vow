
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

**Atomic install.** `install` builds the whole tree in a staging directory next to the target (`.claude/skills/.vow-install-*`) and renames it into place, writing `SKILL.md` last, so a failed or interrupted install never leaves a `SKILL.md` that links to missing `reference/`, `examples/`, or `schemas/` files. When `skills/vow/` already exists, the old tree is renamed aside, the new one renamed in, and the old one restored if that rename fails; the directory is briefly absent between the two renames, never half-written. Files the install does not own (hand-added notes, stale files an older toolchain left in `reference/`) are copied from the old tree into the staged one before the swap, so a re-install rewrites owned files and leaves everything else alone; the install needs write access to `.claude/skills/` itself. A `skills/vow` symlink (dotfile managers) is kept and written through in place, support files first and `SKILL.md` last. A pre-existing partial tree is repaired by the next explicit `install`; auto-install never touches a tree that already holds `SKILL.md`.

When no scope flag is provided, `install` prompts on stderr for local (`./.claude`) or global (`$HOME/.claude`) installation. Scripts and agents should pass `--local` or `--global` explicitly. `--local` requires the current directory to contain both `.git` and `.claude/`; otherwise it exits with an error and writes nothing. `--global` installs under `$HOME/.claude/skills/vow/` and fails if `$HOME` is unset or empty.

**Auto-install on build.** The first time `vow build` (or the bare `vow <source.vow>` form) runs in a directory that already contains a `.claude/` subtree but no `.claude/skills/vow/SKILL.md`, the compiler installs the skill silently. This bootstraps Claude Code projects without requiring an explicit `vow skill install`. Unlike explicit `--local`, auto-install only requires `.claude/`; it does not require the directory to be a git checkout. Auto-install is skipped when `.claude/` does not exist (so it never pollutes non–Claude Code projects) and when the skill file is already present (so user edits are never overwritten). Auto-install never fails the build and stays silent on failure; a failed auto-install leaves no `SKILL.md`, so the next build retries it.

**Migrating an existing install.** An existing `.claude/skills/vow/SKILL.md` is never rewritten, and while it is present a build does not add or repair support files, so an older single-file skill keeps working but lacks `reference/`, `examples/`, and `schemas/`. To migrate, refresh after a compiler upgrade, or repair, run `vow skill install` with the scope being replaced: `--local` for `./.claude/skills/vow/` or `--global` for `$HOME/.claude/skills/vow/` (a global install does not refresh a project-local one). Without `.git`, `--local` is unavailable: delete `.claude/skills/vow/SKILL.md` and run a build instead. Explicit install rewrites `SKILL.md` and every support file it owns, overwriting local edits, and leaves other files alone; review the diff of a committed project install. A pre-rename `.claude/skills/vow-toolchain/` install is neither detected nor modified, so a build installs `.claude/skills/vow/` beside it: delete the old directory by hand.

### `vow test`

Discover, compile, run, and report on Vow test files. Tests are normal `.vow` programs with `main() -> i32` — no test-specific syntax.

```
vow test [OPTIONS] [<path>]
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `<path>`          | `.`         | Directory to scan or single `.vow` file    |
| `--verify`        | (off)       | Run verification (the native backend by default, or ESBMC with `--backend esbmc`) on test files |
| `--backend <esbmc\|native>` | `native` | Verification backend; an explicit `--backend native` requires `--verify` (see "Native backend: `contracts` and `test`") |
| `--filter <pat>`  | (none)      | Only run tests whose file stem contains pat |
| `--module-root <path>` | (auto)  | Resolve `use` declarations against `<path>`. Defaults to the scan path when it's a directory, otherwise the nearest ancestor of the entry file that resolves all of its `use` declarations. |
| `--jobs <N>`      | `min(num_cpus/2, 8)` (`1` with `--verify`) | Max test files compiled and run concurrently |
| `--mode debug`    | (default)   | Insert runtime vow checks                 |
| `--mode release`  | `debug`     | Omit all vow checks for performance       |
| `--timeout <ms>`  | `30000`     | Per-test execution timeout in milliseconds |
| `--max-k-step <N>` | `50`       | ESBMC backend only; a usage error unless `--backend esbmc` is given. ESBMC incremental BMC max iterations (with --verify) |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent verification jobs (with --verify) |

Test discovery: files matching `test_*.vow` or `*_test.vow` under the given directory **and its subdirectories**, sorted alphabetically. Each test must contain `main() -> i32` returning 0 on success.

**Module resolution for directory scans.** When `<path>` is a directory, every discovered test resolves its `use` declarations against `<path>` rather than the test file's own parent directory. This lets internal-unit tests live in a subdirectory like `compiler/tests/test_region.vow` and still `use region;` to import the module under test (which lives at `compiler/region.vow`). A single-file invocation without `--module-root` (`vow test compiler/tests/test_region.vow`) infers the module root the same way: starting at the file's own directory and walking up through its ancestors, it picks the nearest directory against which every `use` declaration of the file resolves (to `<path>.vow`, or `<path>.vow.d`). The walk stops after the first directory that contains `.git` (the repository root), at a `..` path component, or at `.` / `/`. If the file's own directory already resolves every `use`, if the file has no `use` declarations, or if no directory resolves them all, the file's parent directory is used and any unresolved module is reported as an ordinary `IoError`. When the file's own directory does not shadow a module of the tree, the inferred root is the one the directory form would use, so the test gives the same result in both forms; a module in the file's own directory takes precedence over one in an ancestor. This rule applies to `vow test` only; `vow build` and `vow verify` keep resolving `use` against the entry file's parent directory. This existence-only walk never inspects file contents, so it is unaffected by the `.vow.d` stub-vs-source contract fallback described under [Use Declarations](grammar.md#use-declarations): that check only runs once a concrete `use` is actually loaded for a build/verify/test run, not during root inference's probing of candidate directories.

**Concurrency.** Test files in a scan run concurrently, at most `--jobs` at a time, and the `tests` array is always in sorted-path order regardless of completion order. A test's `duration_ms` covers its own compile, verification, and execution. Beyond the first, a worker starts a new file only while the machine is not under memory or IO stall pressure (Linux PSI `/proc/pressure/memory` `some avg10` and `/proc/pressure/io` `full avg10`, both below 20), so a loaded machine degrades to fewer workers rather than thrashing; with no PSI available the limit is just `--jobs`. `--jobs 1` runs files strictly one after another. The self-hosted compiler runs each file of a concurrent scan in a worker subprocess of itself, using an internal `--worker-entry` flag whose output is not part of the CLI contract; a worker that dies without a result is reported as a `compile_error` entry so the suite stays fail-closed.

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

Each test binary is run directly, with no memory limit, and is killed when `--timeout` elapses (status `timeout`). A binary that exits non-zero, or is killed by a signal, is `failed`; a signal death or a `timeout` has no `exit_code` (`null`, or omitted by the Rust compiler).

A `timeout` counts toward `failed`, like `compile_error`, `verify_failed` and `contract_skipped`: a run containing one yields `TestsFailed` and exit code 1.

A `<path>` that does not exist is itself a failure: the result is `TestsFailed` with `total: 1` and `failed: 1`, holding one synthetic `failed` entry whose `file` is the path as given and whose `stderr` names it, and the exit code is 1. `total` equals the length of `tests` in every result, so the synthetic entry is counted.

`contract_skipped` means no proof was attempted because a vowed function is non-modelable (distinct from `verify_failed`, where the verifier found a violation or could not decide). Both are fail-closed — a `contract_skipped` test counts toward `failed` and yields a `TestsFailed` overall status.
