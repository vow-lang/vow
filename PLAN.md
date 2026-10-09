# Plan: route self-hosted bare `vowc <file.vow>` through the `build` path (issue #596)

## Goal
Make the self-hosted no-subcommand form behave exactly like `vowc build` (verify by default, `--no-verify` opt-out, build-result JSON, fail closed on VerifyFailed/Skipped), mirroring the Rust driver's `None` arm (`vow/src/main.rs:1042-1090`, the `None =>` arm; verified: flattened `Args` in `vow/src/cli.rs:140`, source-required message `vow: source file required (try --help or use a subcommand)`, `maybe_auto_install`, then the same `run_build_command(..., args.no_verify, args.dump_ir, ...)` as `Command::Build` at :841-870 — no `--emit-c`/`--verify` fields, no Rust change needed). Delete the divergent `run_legacy` body instead of patching it.

## Assumptions
- Line numbers in the issue are stale; current locations: `run_legacy` `compiler/main.vow:2427-2560`, `run_build` `:1919`, `main()` `:16284-16332`, `get_source_path` `:58`, `get_source_path_sub` `:81`, `frontend_path_from_argv` `:157`. (best guess, verified by reading)
- The legacy-only flags `--emit-c` and `--verify` are **removed**, not re-homed behind new flags. Rust's clap `BuildArgs` (`vow/src/cli.rs:199-239`) has neither; nothing in `scripts/`, `tests/`, `.github/`, `docs/spec/` or bench uses `--emit-c` (only `compiler/cli_flags.vow:42`, `compiler/tests/test_cli_flags.vow:126`, `main.vow:2441`). `--dump-ir` already exists in `run_build`. After the change both flags are rejected with the existing `unexpected argument` usage error (exit 2) like Rust. Alternative considered: keep `--emit-c` gated in the build path; rejected — dead surface, no Rust twin, widens drift. (best guess)
- No-source message for the bare form stays `vow: source file required (try --help or use a subcommand)` (asserted by `scripts/full_test.sh:2422-2431`, `tests/run_tests.sh`); `argv.len() <= 1` guard in `main()` already produces it.
- Compiler-bootstrap callers of the bare form must be moved to explicit `build --no-verify` (see Risks) — the bare form now verifies, which for the 40k-line concat compiler is unaffordable/undesired in the triple test.

## Key Files
| File | Role | Lines |
|------|------|-------|
| `compiler/main.vow` | `run_build` (parametrize source-path mode), delete `run_legacy`, `main()` fallthrough | 1919-1925, 2427-2560, 16330-16332 |
| `compiler/cli_flags.vow` | drop `legacy` param / `--emit-c`/`--verify` from `cf_kind_build`; `cli_flag_kind` CMD_NONE arm | 40-44, 95-97 |
| `compiler/tests/test_cli_flags.vow` | `check_legacy_table` flips: `--emit-c` and `--verify` now rejected | 121-135 |
| `scripts/full_test.sh` | triple test Stage 1/2 (bare `-o`) → `build --no-verify`; `verify_jobs_ce_before_soft` `legacy` mode (`run_self --verify ...`) → bare form with no `--verify`; add bare-form parity/behaviour section | 504, 517, 995-1030, ~2422 |
| `tests/run_tests.sh` | same `legacy` mode (`--verify`) at ~493-510 | 493-510 |
| `tests/cli-flags/tests.sh` | add `--emit-c` / `--verify` bare-form rejection cases (self kind and rust kind) | after line ~45 |
| `docs/spec/cli.md` | line 11 stays; line 52 ("legacy `vowc <file> --verify` form") rewrite; add sentence: bare form ≡ `build`, no legacy-only flags | 11, 52, 108 |
| `scripts/generate_help.py`, `compiler/main.vow` skill/help, `vow/src/skill.rs` | **no change expected** (`legacy_usage` text already says "equivalent to vow build"); run `generate_help.py --check`-equivalent only if cli.md edit touches help-covered text | — |

## Steps (ordered, TDD slices)

### Slice 1 — red: behavioural test that bare form verifies / fails closed
- **File**: `tests/cli-flags/tests.sh` (already wired into CI via `scripts/full_test.sh` Section 4h, l.1456-1464, run for both compilers; fast red-green loop, unlike the ~40 min `full_test.sh`). Needs ESBMC on PATH — skip the verify cases (with a printed note) when `esbmc` is absent. Optionally also a `legacy` parity block in `full_test.sh`.
- **Test**: for `tests/verify-fail/verify_jobs_ce_before_soft.vow` (known VerifyFailed fixture), run `run_self "$fixture" -o "$TMPDIR/x"` and assert exit != 0, `status == "VerifyFailed"`, no output binary; for a proven fixture (`tests/verify/*` pick an existing small one) assert exit 0, `status == "Verified"` (check build-result key names against `diag_emit_build_verify_json`) and binary exists; with `--no-verify` assert exit 0 and `Unverified`. Also assert Rust (`$RUST`) bare form gives the same JSON via the existing `compare_json` helper (run both, `scripts/parity.py json`).
- Fails today: legacy returns `Unverified` / prints IR.

### Slice 2 — green: reuse `run_build` for the bare form
- **File**: `compiler/main.vow`.
- **Change**: rename `run_build(argv)` body to `run_build_cmd(argv, sub: bool)`; first line becomes `frontend_path_from_argv(argv, sub, "build")`. The legacy no-source message is handled in `main()` instead (decision): before dispatching the bare form, `if get_source_path(argv).len() == 0 { eprintln_str("vow: source file required (try --help or use a subcommand)"); return 1; }`. `frontend_path_from_argv` and its four callers stay untouched. `run_build(argv) = run_build_cmd(argv, true)`. In `main()` replace `run_legacy(argv)` with `run_build_cmd(argv, false)` (keep `maybe_auto_install_skill()` — cli.md:108 documents the bare form auto-installs).
- Delete `run_legacy` (2427-2560) wholesale; confirm no other caller (`grep -n run_legacy compiler/`). Do **not** copy-paste its verify loop — it duplicates `run_build`'s and is the source of the drift.
- Default output: `run_build` uses `default_output(path)` when no `-o` (matches Rust `build/<stem>`); this is an intended behaviour change for bare-without-`-o` (previously IR dump) — document in cli.md/PR.
- **Reuses**: `run_build` (`main.vow:1919`), `get_source_path` (`:58`), `default_output` (`:105`).

### Slice 3 — red→green: flag table
- **File**: `compiler/tests/test_cli_flags.vow` `check_legacy_table` (l.121): `--emit-c f.vow` and `f.vow --verify` must now be non-empty errors (return codes adjusted), `--no-verify`/`-o` still accepted. Then `compiler/cli_flags.vow`: remove `legacy` param from `cf_kind_build` and its two callers in `cli_flag_kind`.
- Run `build/vowc test compiler/tests/test_cli_flags.vow`.
- `backend_flag_error` (main.vow:16258) unaffected: `--backend native` is already rejected for CMD_NONE.

### Slice 4 — update dependents of the old semantics
- `scripts/full_test.sh:504,517`: `run_self_bin compiler_a build --no-verify --no-cache ... -o compiler_b "$TMPDIR/compiler_clif.vow"` (and Stage 2). `tests/full_test_bootstrap/tests.sh` stubs binaries and only scans args for `-o` (l.80-83), so it needs no change but must be re-run; `tests/run_tests.sh:292-296` already uses `build --no-verify`. Stage 0 line (`$rust --no-verify ...`) stays — Rust bare form with `--no-verify` is already ≡ build.
- `scripts/full_test.sh:995-1010` + `tests/run_tests.sh:493-510`: `legacy)` mode drops `--verify` (bare form verifies by default); expected status still `VerifyFailed`.
- `tests/cli-flags/tests.sh`: add `unknown_flag --emit-c --emit-c "$FIXTURE"` and `unknown_flag --verify --verify "$FIXTURE"` (bare form) for both kinds.
- `scripts/full_test.sh:2422` legacy no-source test must still pass unchanged (guard Slice 2).
- `.github/workflows/ci.yml:124,182` already use `vow build --no-verify`; no change.

### Slice 5 — docs
- `docs/spec/cli.md`: line 52 replace "the legacy `vowc <file> --verify` form" with "the bare `vowc <file>` form (identical to `build`)"; add under `vow build`: bare form is exactly `vow build` — verifies by default, same flags, same JSON, fails closed; `--emit-c` and `--verify` are not accepted. Keep the line-11 / line-108 text.
- Run `uv run python scripts/generate_help.py` only if help-covered sections changed; `python3 scripts/check_help_coverage.py` must stay green.

### Slice 6 — docs outside spec
- `CLAUDE.md` (repo root): the self-hosted examples `/tmp/vow_main compiler/lexer.vow` (type-check, print IR), `/tmp/vow_main -o /tmp/lexer compiler/lexer.vow`, and the "Bootstrap triple test" block (`/tmp/compiler_a -o /tmp/compiler_b ...`) use the bare form; rewrite to `build --no-verify` / `--dump-ir`. README.md:87 mention of `vowc <source.vow>` auto-install stays true.

## Testing
- Per slice: `build/vowc test compiler/tests/test_cli_flags.vow`; `bash tests/cli-flags/tests.sh` (`VOWC_BIN=build/vowc`, and Rust with `VOWC_KIND=rust`).
- Before PR: `scripts/bootstrap.sh --skip-cargo --no-cache` on final head SHA (record SHA in checklist per CLAUDE.md), then `scripts/full_test.sh` (~40 min; run in background and poll with short bounded checks — no sleep loops).
- Rust side: no Rust source change expected (bare form already ≡ build). Dual-compiler rule is satisfied because the self-hosted compiler is being brought *to* Rust behaviour; state this in the PR. Run `cargo test --all` only if any Rust file is touched.

## Verification surface
- No contract, codegen, or C-model change; no new ESBMC properties. `run_build_cmd` reuses existing verified functions; `run_legacy` removal drops no contracts. Verifier C parity (Section 2c) unaffected, but the bootstrap `build/vowc` is itself verified by `bootstrap.sh` so the refactored `main.vow` functions must still verify (keep new functions small, no new contracts needed).
- New fixtures: none required — reuse `tests/verify-fail/verify_jobs_ce_before_soft.vow` and an existing `tests/verify/*.vow`.

## Risks
- **Test-worker argv (checked)**: `main.vow:1206-1215` `test_worker_common_flags` forwards `--verify` to `vowc test` workers (`run_tests_parallel(argv[0], ...)`, l.2152). That `--verify` belongs to `CMD_TEST` (`cf_kind_test` still accepts it), not the legacy table. Implementer must confirm the worker spawn argv starts with the `test` subcommand (read `run_tests_parallel`) so the bare-form change cannot affect workers; if it spawns bare, add `test` there.
- **Wider caller grep (done)**: no bare-form callers in README/docs/examples/install scripts; `scripts/cli_compat_test.sh:127` and `scripts/full_test.sh:88` use the *Rust* bare form with `--no-verify` (unchanged). `full_test.sh:1204-1212` always passes `verify`/`build`. Implementer re-runs `rg -n 'run_self(_bin)? +(-|"?\$)' scripts tests` once before landing.
- **Bootstrap triple test (biggest)**: Stage 1/2 in `full_test.sh` invoke the self-hosted binary bare with `-o`; after the change that would run ESBMC on the entire concat compiler. Mitigation: Slice 4 switches to `build --no-verify`. Grep once more for other bare-form uses (`rg -n 'compiler_[abc]|vowc2|vowc3' scripts tests`) before landing.
- **Binary fixed point**: `main.vow` shrinks; codegen is deterministic so b==c should hold, but any new `Vec`/map iteration must not introduce ordering nondeterminism. No `BTreeMap`/clif-shim changes.
- **Behaviour change for users**: bare `vowc file.vow` without `-o` now writes `build/<stem>` and requires ESBMC (fails with `VerifyFailed` JSON if missing, same as `build`). That is the documented contract.
- **Verifier-parity of error text**: `run_build` prints `... or use --no-verify` hint on missing ESBMC; legacy previously printed a different message. Tests matching the old text: grep `ESBMC not found; install ESBMC to run verification` in scripts/tests before landing.
- **`get_source_path` vs `_sub`**: bare form scans from argv[1]; `build` scans from argv[2]. Using the wrong one makes `build` token be treated as source or vice versa — Slice 2 passes the `sub` flag to preserve each.
- **Rust-side drift**: confirm Rust bare form rejects `--emit-c`/`--verify` (clap) so the two compilers agree; the new cli-flags cases run against both.
- clippy gate: no Rust changes, so unaffected.

## Out of scope
- Re-adding a C-emission flag or any IR/`--dump-ir` change.
- `vow decl` missing from self-hosted `main()` (separate audit finding in the same doc).
- Refactoring `run_build`/`run_verify` duplication, splitting `main.vow`, or any formatting.
- Changes to `.github/workflows/*`, the Rust driver, or the verifier.
