# Plan: self-hosted `vowc` rejects unknown flags (clap parity)

## Goal
Make `build/vowc` fail with a clap-style usage error (exit 2) on any flag that the selected subcommand does not implement, so a stale `--vec-max 10000` (or a typo, or `--mode=debug`) is rejected instead of silently ignored. Option (b) from the issue: one general validator, no special-casing of the four retired capacity flags.

## Assumptions
- Size: Medium. One self-hosted module + hook in `main()`, one shell harness, docs. Explored inline (no subagents); adversarial review via advisor.
- Rust twin: `vow/src/cli.rs` (clap) already rejects unknown flags. Verified: `vow/src/main.rs:827` is a plain `Args::parse()` (no `try_parse`/custom handling), so clap's own error path applies: `error: unexpected argument '--vec-max' found`, exit 2. The only `--vec-max` mentions in `vow/src/main.rs:1298-1314` are a help-surface test (`prover_coupled_flags_are_not_advertised`), not a special-case. (No `target/release/vow` in this workspace, so the exit code was not run; slice 10's characterization test and step-1 run against a built Rust binary confirm it before the docs claim it.) No Rust production change. The Rust half of the dual-compiler rule is satisfied by a parity test that runs the same invocations against both binaries (plus one pinning test in `vow/tests/cli_dispatch.rs`). Recorded here so the implementer does not invent a Rust change.
- Exit code **2** and stderr text `error: unexpected argument '<flag>' found` (clap's wording/code) so a harness can assert both compilers identically. Other self-hosted usage errors keep exit 1 (not touched).
- `--flag=value` form: clap accepts it, self-hosted never did (it was silently ignored, i.e. `--mode=debug` built in release). It is now rejected as unknown with a hint `(use '--flag value')`. Supporting `=` is out of scope.
- `--help` short-circuits at the top of `main()` before validation, so `vowc verify --vec-max 1 --help` prints help (clap would error). Intentional divergence.
- `mutants` is NOT validated: Rust clap forwards everything verbatim (`allow_hyphen_values`), and `mutants_main.vow` owns its own flag set. Follow-up.
- Extra positionals (`vowc build a.vow b.vow`) are not rejected (clap would). Follow-up.
- `decl` subcommand is absent from the self-hosted compiler; unrelated gap, not touched.
- `--output` is documented in `cli.md` and advertised in `--help` JSON (`compiler/main.vow:2604-2607`) but `run_build` only reads `-o` (`main.vow:1935`). Under strict validation this must not become a rejected-documented-flag, so it is treated as an alias of `-o` (tiny, in-scope fix; slice 3).
- `--worker-entry <tag>` is an internal flag (`compiler/runner_plan.vow:255`), absent from Rust; accepted for `test` only.
- Validation runs after `backend_flag_error` in `main()` so the existing `--backend` usage errors/exit 1 (`tests/verify-native/tests.sh:259-269`) are unchanged.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `scripts/concat_vow.sh` | register `cli_flags` in both `FILES=` lists (lines 21, 23) | 21-23 |
| `compiler/cli_flags.vow` [new] | Deep module: per-command flag tables + `cli_flag_error` + `cli_flag_takes_value` | — |
| `compiler/main.vow` | hook validator into `main()`; replace the two duplicated `prev == ...` chains; `--output` alias | 47-91 (`get_source_path{,_sub}`), 117-153 (CMD_*), 1933-1939 (`-o` in `run_build`), 2425-2430 (`run_legacy`), 16257-16300 (`main`) |
| `compiler/tests/test_cli_flags.vow` [new] | unit tests for the pure validator | model on `compiler/tests/test_complexity_io.vow` |
| `tests/cli-flags/tests.sh` [new] | black-box harness, runs both binaries | model on `tests/verify-native/tests.sh:259-269` (`usage_error`) |
| `scripts/full_test.sh` | wire the harness (new section after 4g, ~line 1455) for `$RUST` and `$SELF` | `RUST=` line 15, `SELF=` 89, Section 4g 1378-1455 |
| `vow/tests/cli_dispatch.rs` | pin clap rejection (exit 2, message) for `build --vec-max 1 x.vow` | spawn pattern at 41-58 |
| `docs/spec/cli.md` | document unknown-flag behaviour + exit 2 | Exit Codes 264-285, Options tables 5-34 |
| `scripts/generate_help.py`, `compiler/main.vow` (GENERATE blocks), `vow/src/skill.rs` | regenerate if cli.md text is embedded (`exit_codes` at generate_help.py:569) | — |

## Steps (ordered; each = one reviewable commit)

### 1. Audit callers before tightening (read-only, no commit)
- `grep -rnE 'vowc|build/vowc|\$SELF|\$VOWC' scripts tests .github bench Makefile* docs 2>/dev/null` and collect every flag passed to the self-hosted binary (the recon grep found `--no-verify`, `-o`, `--verify-jobs`, `--verify`, `--output-dir`, `--mode`, `--backend`, `--help`; confirm with a full grep incl. `bench/verifier.py`, `scripts/bootstrap.sh`, `scripts/parity*.py`, `compiler/runner_plan.vow`, `compiler/main.vow:837-842` self-spawn of build).
- Any flag found that is not in the tables below is either added to the right command or the caller is fixed. Record findings in the PR body.

### 2. Create `compiler/cli_flags.vow` [new] — the validator
- **Move `CMD_NONE..CMD_COMPLEXITY` (`main.vow:117-124`) into `cli_flags.vow`; `main.vow` gains `use cli_flags`.** Decided: the unit test needs the ids and cannot `use main` (it defines `fn main`); no other non-`VC_CMD_*` user exists (grep showed only `main.vow`). Constants `CF_UNKNOWN()=0, CF_BOOL()=1, CF_VALUE()=2` (Vow has no tables; follow the `CMD_*` function-constant idiom at `main.vow:117`). `module CliFlags`, `use` nothing but builtins; commands passed as `i64` using the same numbering as `CMD_*` .
- **Table rule (invariant):** for each command, `accepted(self-hosted) ⊇ flags of the matching clap struct` (`BuildArgs`, `VerifyArgs`, `TestArgs`, `ContractsArgs`, `ComplexityArgs`, `SkillArgs`, top-level `Args` for legacy; `vow/src/cli.rs`) `∪ self-hosted-only extras` (`--backend`, `--worker-entry`, legacy `--emit-c`/`--verify`). Never reject something Rust accepts. Flags Rust accepts but vowc ignores (e.g. `contracts --verify-jobs`) stay accepted. Derive from the clap structs first, then add extras found via the `has_flag`/`get_flag_arg` reads in the matching `run_*`; implementer re-diffs both directions.
- `fn cli_flag_kind(cmd: i64, flag: String) -> i64` — decision chain per command (tables below are the starting point):
  - build: bool `--no-verify --dump-ir --no-cache --replay-cex --help --human`; value `-o --output --mode --debug-trace --max-k-step --solver --encoding --timeout --verify-jobs --perfetto --backend`
  - verify: bool `--no-cache --replay-cex --help --human`; value `--max-k-step --solver --encoding --timeout --verify-jobs --perfetto --backend`
  - test: bool `--verify --help --human`; value `--filter --module-root --mode --timeout --max-k-step --verify-jobs --jobs --worker-entry --backend`
  - contracts: bool `--verify --no-cache --help --human`; value `--max-k-step --solver --encoding --timeout --verify-jobs --backend`
  - complexity: bool `--help --human`; value `--cog-anchor --nloc-anchor --max-score --max-cognitive --max-cyclomatic`
  - skill: bool `--bundle --local --global --help --human`
  - legacy (`CMD_NONE`): the top-level clap `Args` = the build set (bool `--no-verify --dump-ir --no-cache --replay-cex --help --human`; value `-o --output --mode --debug-trace --max-k-step --solver --encoding --timeout --verify-jobs --perfetto --backend`) plus self-hosted extras bool `--emit-c --verify` (read by `run_legacy`, `main.vow:2425+`). `vowc --no-verify f.vow` must keep working.
  - mutants: not validated (`cli_flag_error` returns "").
- `fn cli_flag_error(argv: Vec<String>, cmd: i64) -> String` — pure; scan from index 2 (1 for legacy). For each token starting with `-` (byte 45, as at `main.vow:52`): kind UNKNOWN → `error: unexpected argument '<tok>' found` (append `\n  tip: use '<flag> <value>', not '<flag>=<value>'` when it contains `=` and the prefix is a known value flag); VALUE → consume the next token unconditionally (mirrors `get_flag_arg`, so `--filter -x` works); a VALUE flag as last token → `error: a value is required for '<flag>' but none was supplied`. Returns `""` when clean. Skill's `argv[2]` action word is a non-dash token and is skipped naturally.
- `fn cli_flag_takes_value(flag: String) -> bool` — command-agnostic union of all VALUE flags above; used by step 4.
- Keep every function short (project rule); no contracts needed beyond trivial ones — do not add artificial bounds. `cli_flag_error` is `[]`-pure so it is verifiable; if ESBMC cannot model string-chain functions, mark unverifiable per repo convention, never weaken.

### 3. TDD slices (red → green), each with its commit
1. **Red**: `compiler/tests/test_cli_flags.vow` — `cli_flag_error(["vowc","verify","--vec-max","10000","f.vow"], CMD_VERIFY())` returns text containing `unexpected argument '--vec-max'`. **Green**: minimal `cli_flag_kind` + scan.
2. Known flags per command accepted: one parametrised assertion per command over its full table (`""` result); `verify --mode debug` and `verify -o x` rejected (Rust `VerifyArgs` has neither); `build --output x` accepted.
3. Value consumption: `build --mode debug f.vow` clean; `test --filter -x f.vow` clean; `build f.vow --mode` → "a value is required"; `build --mode=debug f.vow` → unexpected + tip.
4. All four retired flags (`--vec-max --string-max --hashmap-max --btreemap-max`) rejected for build/verify/test/contracts/legacy with no flag-specific code (assert the source of `cli_flags.vow` has no such literal via the test's own table, i.e. test just feeds them).
5. mutants returns `""` for arbitrary flags; skill: `--bundle/--local/--global` ok, `--foo` rejected.
6. **Wire into `main()`** (`compiler/main.vow:16257`): after the `backend_flag_error` block, `let flag_error = cli_flag_error(argv, cmd); if len>0 { eprintln_str(flag_error); return 2; }`. Black-box red first: `tests/cli-flags/tests.sh` [new] runs `"$BIN" verify --vec-max 10000 "$FIXTURE"` etc. asserting rc 2 and stderr substring `unexpected argument '--vec-max'`, plus a positive control (`verify --no-cache fixture` rc 0/1 by verdict, not 2), loops over the 4 retired flags × {build,verify,test,contracts}, and `skill --bogus`, `complexity --verify`. Harness takes `BIN` so it runs for both compilers; for Rust, clap's message is `error: unexpected argument '--vec-max' found` (rc 2) — assert the shared substring only.
7. **`--output` alias** (`main.vow:1933-1939`): red = harness builds `build --no-verify --output $TMP/out fixture` and expects `$TMP/out` to exist; green = read `--output` when `-o` absent (small helper `output_flag_value(argv)` next to `has_flag`, used in `run_build` and `run_legacy` line 2509 if applicable).
8. **Dedupe path scanner**: replace the two `prev == "-o" || ...` chains (`main.vow:56`, `:79`) with `cli_flag_takes_value(prev)`. Behaviour-preserving for valid invocations; add `--output` (and for non-sub variant `--jobs/--worker-entry` harmlessly). Unit test: for each VALUE flag `F`, `get_source_path_sub(["vowc","build",F,"v","src.vow"]) == "src.vow"` — locks the single-source invariant. Not scope creep: if the validator and the path scanner disagree on which flags take a value, `get_source_path` picks the wrong token as the source; one predicate removes that failure class. Own commit (`refactor(cli): …`), separable if review asks.
9. Wire `tests/cli-flags/tests.sh` into `scripts/full_test.sh` as a new section after 4g, run with `BIN="$RUST"` and `BIN="$SELF"`; Rust-only skip of `mutants`/`--backend` cases.
10. `vow/tests/cli_dispatch.rs`: `build --vec-max 1 x.vow` → code 2, stderr contains `unexpected argument '--vec-max'` (pins the Rust half of the parity claim; passes today — characterization test).

### 4. Docs
- `docs/spec/cli.md`: in "Exit Codes" add row `2` = usage error — unknown or unexpected flag (both compilers, clap wording); add a short "Unknown flags" paragraph under `vow build` options: unknown flags are rejected, `--flag=value` form unsupported by `vowc`, `mutants` forwards its flags. Note retired `--vec-max/--string-max/--hashmap-max/--btreemap-max` now error.
- Run `uv run python scripts/generate_help.py`; if it rewrites the `exit_codes` block or skill text, commit `compiler/main.vow` GENERATE regions and `vow/src/skill.rs`; then `python3 scripts/check_help_coverage.py`.

## Testing / Verification surface
- Unit: `build/vowc test compiler/tests/test_cli_flags.vow` (run via background + poll; ~3 min/file budget).
- Black-box: `VOWC_BIN=build/vowc bash tests/cli-flags/tests.sh` and with `./target/release/vow`.
- Gates (separate commands, never `&&`): `cargo fmt --all --check`, `cargo clippy --all --all-targets -- -D warnings`, `cargo test -p vow --test cli_dispatch`, `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA (record SHA), then `scripts/full_test.sh` (~40 min; background + poll). Python lint untouched.
- ESBMC / C-model: no contract or codegen change → no new verification obligations; the new module's functions are verified by `bootstrap.sh` as part of the compiler (build/vowc verifies itself). No `tests/run/` or `examples/` growth needed; `tests/cli-flags/` is the new fixture dir. C parity (`c_emitter.{rs,vow}`) unaffected.

## Risks
- **Self-bootstrap breakage**: the compiler spawns itself (`main.vow:837-842` build args; `runner_plan.vow:255` test workers) and `bootstrap.sh`/`full_test.sh` invoke `vowc` with flags. A flag missing from a table bricks the pipeline. Mitigation: step 1 audit, then bootstrap + full_test before PR; keep `--worker-entry` in the `test` table.
- **Binary fixed point**: new module adds code → Stage 1/2 still must be byte-identical; no `HashMap`/ordering hazard (pure string chains). `scripts/concat_vow.sh:21,23` enumerate modules explicitly (two `FILES=(...)` lists): add `cli_flags` before `main` in BOTH or the bootstrap triple test fails to find the symbols. Also check any other module list (`bootstrap.sh`, `full_test.sh`, mutants skip lists) with `grep -rn complexity_main scripts`.
- **`CMD_*` move**: `main.vow` is the only user (re-grep `CMD_` excluding `VC_CMD_` before editing); no fallback to raw ints.
- **Exit 2 vs existing expectations**: nothing relied on unknown flags before; but `tests/verify-native/tests.sh` expects exit 1 for `--backend` misuse → guaranteed by ordering after `backend_flag_error`. Verify `usage_error ... "$ONE_CLAIM" --backend native` (legacy path with `--backend`) still hits `backend_flag_error` first.
- **Legacy table drift**: `run_legacy` flag reads must be re-derived by grep; undercounting would reject valid legacy calls.
- **Divergences left on purpose** (documented): `--flag=value`, extra positionals, mutants passthrough, `--help` precedence.
- **codecov/patch**: Rust change is test-only; the `.vow` corpus runs uninstrumented, so no patch-coverage exposure from the new `.vow` code.
- Commit/PR title must be lower-case Conventional Commits, e.g. `feat(cli): reject unknown flags in self-hosted vowc` (≤ ~92 chars). Remove `PLAN.md` before opening the PR.

## Out of scope
- Rejecting extra positional arguments; `--flag=value` support; validating `mutants` flags; adding `decl` to the self-hosted compiler; changing other self-hosted usage-error exit codes from 1; any change to the Rust CLI surface; removing/renaming existing flags; `--help` content beyond regeneration forced by the `cli.md` edit; broader `main.vow` splitting/formatting.
