# Plan: port frontend diagnostic I/O-failure handling (#944) to the self-hosted compiler

## Goal
`build/vowc` observes a failed stderr write while emitting frontend diagnostics, tolerates `BrokenPipe`
(frontend result unchanged), and turns any other failure into a structured `CompileFailed`
(`message` = `failed to emit frontend diagnostics: …`, exit 1), matching the Rust stage 0 after #944
(7b302b39: `emit_frontend_result`, `run_decl_command`, `run_complexity_command`).

Size: Large (cross-cutting: runtime + builtin registration in both compilers + driver rewrite + spec).
One PR, ordered commits; slice 1-2 (builtin plumbing) land before slice 4+ (callers) because stage 0
must know the builtin before `compiler/*.vow` calls it.

## Assumptions (decided unattended; reviewer may override)
- **New builtin `try_eprintln_str(s: String) -> i64 [io]`**, runtime symbol `__vow_try_eprintln_str`.
  Name mirrors #944's `try_emit`/`try_finish`. `eprintln_str` is left untouched (surgical; 70+ callers).
- **Status is a closed enum, not errno**: `0` ok, `1` broken pipe (`ErrorKind::BrokenPipe`),
  `2` any other failure. Same convention as `fs_read_status` (0/1/2/3). Rationale: one catalogue entry,
  no thread-local error-text accessor, no `String`-returning builtin (no `arena_routing`).
- **`EBADF` maps to `0`**, mirroring Rust std (`stderr` treats a closed fd 2 as success), so
  `2>&-` behaves identically in both compilers. Confirm against stage 0 during slice 0.
- **Detail text differs from Rust**: self-hosted message is `failed to emit frontend diagnostics: stderr write failed`;
  Rust appends the OS error text (`No space left on device (os error 28)`). `cli.md` documents `{io_error}`
  abstractly, so no cli.md edit. Tests assert the prefix, not the tail. A `last_io_error_message` accessor is a follow-up.
- **SIGPIPE must be shielded per call, not ignored globally.** `vowc`'s `main` is Cranelift-emitted
  (`vow-codegen/src/cranelift_backend.rs:2245-2260`), Rust std's `lang_start` never runs, so SIGPIPE is
  default and today a closed stderr pipe kills `vowc` with exit 141 (empirically probed). The runtime's
  existing `write_shielded` (`vow-runtime/src/piped.rs:106`) blocks SIGPIPE on the thread and drains it;
  its doc comment rejects a global ignore (`| head` must still kill the program). Reuse it by extracting
  the mask/drain guard into a generic helper.
- **Follow-on stderr lines on the six paths go through a best-effort wrapper** (`try_eprintln_str`,
  status discarded) so a tolerated BrokenPipe is not followed by a SIGPIPE kill from the very next bare
  `eprintln_str`. Mirrors Rust's `stderr_broken_pipe` flag. Residual: later bare `eprintln_str` calls
  (verify progress lines, `emit_ir_warnings`, clif errors) keep today's behaviour (kill by SIGPIPE, 141).
- **`message` is added only on the I/O-failure path.** The self-hosted `CompileFailed` JSON has never carried
  `message` (no emitter takes one; `scripts/parity.py compare_json` ignores it). Adding it to ordinary
  parse/type/module-load failures is a separate, pre-existing parity gap (see Out of scope).
- **No contracts on the new/changed `[io]` functions.** Effectful vowed functions are `Skipped` and fail
  closed under `vowc build`; none of `diag_ctx_print_all`'s siblings carry vows either.

## Per-call-site parity table (self-hosted ← Rust)
`status` = value returned by the rewritten `diag_ctx_print_all`. Stop at the first failed write (Rust's
`emit_frontend_diagnostics` short-circuits and skips `try_finish`).

| Site | Subcommand | status 0 / 1 (ok / broken pipe) | status 2 (other failure) |
|---|---|---|---|
| `compiler/main.vow:1874` `run_verify` | verify | unchanged flow (JSON `CompileFailed` if `n_errors>0`, else continue) | skip the `N items` line; emit CompileFailed JSON with `message`, `return 1` — **even when `n_errors == 0`** (Rust `emit_frontend_result` Ok-path) |
| `compiler/main.vow:2115` `run_build_cmd` | build | same | same |
| `compiler/main.vow:16294` `run_contracts` | contracts | same | same (`contracts.rs:248`: `emit_json` + `exit(1)`) |
| `compiler/main.vow:2365` `run_test` | test | unchanged | unchanged (entry is already `compile_error`; Rust also drops `message` there). Only bind/ignore the new return |
| `compiler/decl_main.vow:46` `run_decl` | decl | unchanged; later `decl_fail`/`wrote` lines via best-effort wrapper (this replaces Rust's `stderr_broken_pipe` flag) | `n_errors==0`: best-effort `vow decl: failed to emit frontend diagnostics: <detail>`, exit 1 before writing the stub. `n_errors>0`: `vow decl: <stage>; failed to emit frontend diagnostics: <detail>`, exit 1 |
| `compiler/complexity_main.vow:209` `run_complexity` | complexity | unchanged; later messages via wrapper | `n_errors==0`: `vow complexity: failed to emit frontend diagnostics: <detail>`, exit 1. `n_errors>0`: `vow complexity: compile errors; failed to emit frontend diagnostics: <detail>`, exit 1 |

## Key Files
| File | Role | Lines of Interest |
|---|---|---|
| `vow-runtime/src/lib.rs` | `__vow_eprintln_str` (4217) stays; add `__vow_try_eprintln_str` + pure status seam + unit tests (mod tests at 5487) | 4217-4226 |
| `vow-runtime/src/piped.rs` | extract SIGPIPE mask/drain guard out of `write_shielded` (106-130) | 106-130 |
| `docs/spec/operations.json` | add catalogue entry after `eprintln_str` (201-208): params `["ptr"]`, return `i64`, `[io]`, `verifier_model: unmodeled`, no `arena_routing` | 201-208 |
| `scripts/generate_operations.py` | run only; no edit (`ptr`/`i64` tokens exist). Rewrites 4 GENERATE blocks | `TARGET_FILES` 670-678 |
| `scripts/test_generate_operations.py` | `FS_STDIN_ARGS_STDERR_OPS` (add dict after 207-214); `KNOWN_OPS` is asserted equal (ordered) to the real catalogue (434) | 54-215, 422-434 |
| `vow-types/src/env.rs` | `def("try_eprintln_str", [Str], I64, [IO])` after 253; sorted golden snapshot `EXPECTED_BUILTIN_SIGNATURES` needs `try_eprintln_str(Str) -> I64 [IO]` in sorted position (between `time_unix_ms` at env.rs:1067 and `u128_to_i128_sat` at 1068) | 253, 883-1126 |
| `compiler/env.vow` | `env_define_fn(e, "try_eprintln_str", vec_of_1(str_tid), i64_tid, eff_io)` after 309 | 309 |
| `vow-ir/src/lower/mod.rs` | generated `catalogue_builtin_to_runtime` (56); hand table `builtins_lower_to_runtime_symbols_and_return_types` (~6386) | 56, 6386 |
| `vow-codegen/src/cranelift_backend.rs` | generated `catalogue_extern_sig` (2494); tests list `one_param_returns_i64` (3735-3749) | 2494, 3735 |
| `vow-clif-shim/src/lib.rs` | generated `catalogue_extern_sig` (3423); test list `one_param_returns_i64` (~4519-4533). **Unknown symbols silently fall back to no-arg/no-return (4173)** → generated arm is load-bearing | 3423, 4173, 4519 |
| `compiler/lower.vow` | generated `catalogue_builtin_to_extern` (1826) / `catalogue_builtin_ret_ty` (1876) | 1802-1914 |
| `compiler/diag.vow` | `diag_ctx_print_all` (581) → returns status; new pure `diag_emission_failure_message`, JSON helper, best-effort wrapper | 581-589, 710-833 |
| `compiler/main.vow` | callers 1874, 2115, 2365, 16294; embedded help/skill blocks regenerated | see table |
| `compiler/decl_main.vow` | caller 46; `decl_fail` 29-34; `wrote` line 73 | 29-73 |
| `compiler/complexity_main.vow` | caller 209-226 | 209-226 |
| `docs/spec/grammar.md` | Print / IO table (1326-1332) row + status-code paragraph | 1326-1332 |
| `skills/vow/reference/grammar.md`, `vow/src/skill.rs`, `compiler/main.vow` | **generated** by `scripts/generate_help.py` | — |
| `compiler/tests/test_lower_catalogue_fs_stdin_args_stderr.vow` | add `expect_extern`/`expect_ret_ty` (lines 42/68 are the `eprintln_str` rows; unique codes go past 40) | 25-70 |
| `compiler/tests/test_diag_emit_failure.vow` `[new]` | unit tests for the pure helpers | — |
| `tests/run/try_eprintln_str_status.vow` `[new]` | end-to-end builtin test on both compilers | — |
| `tests/diag-io/tests.sh` `[new]` | dual-compiler shell harness (template `tests/cli-flags/tests.sh`, `tests/decl/tests.sh`) | — |
| `scripts/full_test.sh` | add Section 4j beside 4h/4i (1516-1551) | 1516-1551 |

## Steps (TDD slices, in order)

### 0. Outer red: shell harness `tests/diag-io/tests.sh` [new]
- Drive `VOWC_BIN`/`VOWC_KIND` exactly like `tests/cli-flags/tests.sh`. Fixture: a tiny source with one parse error (reuse an existing `tests/error/*.vow`) plus a warning-only/valid fixture (`examples/hello.vow`) for the `n_errors == 0` case.
- Cases (assert exit code, parsed stdout JSON `status`, `diagnostics` non-empty; for the emission-failure cases assert `message` **prefix** `failed to emit frontend diagnostics` — Rust and self-hosted):
  - `2>/dev/full` (ENOSPC): `build --no-verify`, `verify`, `contracts` on the failing fixture and on the valid fixture → CompileFailed, exit 1 (valid fixture proves the `n_errors == 0` path). `decl` and `complexity`: exit 1, no `.vow.d` written / no JSON on stdout (stderr text is unobservable under `/dev/full`).
  - `2>&-` (EBADF): behaves as a normal run (no emission failure) in both compilers.
  - EPIPE: `python3 -I` helper that `os.pipe()`s, closes the read end, then `subprocess.run(..., stderr=write_end)` (child gets default SIGPIPE; `os.pipe`+close is deterministic, unlike `| head`) → JSON `CompileFailed` on stdout preserving the original diagnostics, exit 1, **not 141**.
- Expected at this point: Rust stage 0 green (confirms the EBADF assumption), self-hosted red. Do not wire into `full_test.sh` yet.

### 1. Runtime builtin (RED: runtime unit tests, then GREEN)
- `vow-runtime/src/lib.rs`: add private pure seam `stderr_write_status(w: &mut impl Write, bytes: &[u8]) -> i64` (appends `\n`, **one** `write_all`; `BrokenPipe`→1, `EBADF`→0, other→2, ok→0). Add `#[unsafe(no_mangle)] pub unsafe extern "C" fn __vow_try_eprintln_str(s: *const u8) -> i64` (null → 0, same `sanitize_on_read` + `VowVec` read as `__vow_eprintln_str`), writing to `std::io::stderr()` inside the SIGPIPE shield.
- `vow-runtime/src/piped.rs`: extract the block/drain/restore sequence from `write_shielded` into `pub(crate) fn with_sigpipe_blocked<T>(f: impl FnOnce() -> T) -> T`; `write_shielded` calls it. Behaviour-preserving.
- Tests (in `mod tests`): fake writers for `BrokenPipe` → 1, `PermissionDenied`/`StorageFull`/`WriteZero` → 2, `EBADF` raw OS error → 0, ok → 0 (and asserts exactly one trailing `\n`); one real-fd test on a pipe whose read end is closed with SIGPIPE temporarily set to `SIG_DFL` (precedent `piped.rs:490-492`) proving the process survives and the helper returns EPIPE and leaves no pending SIGPIPE.

### 2. Register in both compilers (RED: snapshot/KNOWN_OPS/shape tests, then GREEN)
- Reds first: add the dict to `FS_STDIN_ARGS_STDERR_OPS`, the snapshot line to `EXPECTED_BUILTIN_SIGNATURES`, `"__vow_try_eprintln_str"` to both `one_param_returns_i64` lists, the `("try_eprintln_str", "__vow_try_eprintln_str", Ty::I64)` row in `vow-ir/src/lower/mod.rs` test table, and `expect_extern`/`expect_ret_ty(ITY_I64())` rows in `compiler/tests/test_lower_catalogue_fs_stdin_args_stderr.vow`.
- Greens: `docs/spec/operations.json` entry; `python3 scripts/generate_operations.py` (regenerates `lower/mod.rs`, `cranelift_backend.rs`, `clif-shim/src/lib.rs`, `compiler/lower.vow`; `region.rs`/`ir.vow`/`vc_ops.vow` must stay unchanged — no arena route, `unmodeled`); hand-edit `vow-types/src/env.rs` and `compiler/env.vow`. Template commit: 19e3055c (`fs_remove_dir_all`, scalar `(ptr)->i64 [io]`); `2faf9df8` for the touched-file list.
- `generate_operations.py --check` also requires the grammar row and the `skill.rs`/`main.vow` help lines → do step 9's doc edit + `generate_help.py` in the same commit so `--check` is green.

### 3. End-to-end builtin test `tests/run/try_eprintln_str_status.vow` [new]
- `// TEST: stdout "0\n"`, `// TEST: stderr "<marker>"`: call `try_eprintln_str(String::from("<marker>"))`, `print_i64` the result. Template: `tests/run/fs_read_status.vow`, `tests/run/fs_remove_dir_semantics.vow`. Compiled by stage 0 and by the self-hosted compiler through `full_test.sh` Section 4.

### 4. diag.vow seam (RED: `compiler/tests/test_diag_emit_failure.vow` [new], then GREEN)
- `diag_ctx_print_all(ctx) [io] -> i64`: loop `try_eprintln_str(diag_to_human(ds[i], ctx))`; return `0` when all ok; remember a broken pipe (result 1) but **stop and return 2** at the first status 2 (a broken pipe on one write → stop writing, return 1, since every later write fails too).
- Named constants as functions in the repo style (`DIAG_EMIT_OK()`, `DIAG_EMIT_BROKEN_PIPE()`, `DIAG_EMIT_FAILED()`).
- Pure `diag_emission_failure_message() -> String` returning `failed to emit frontend diagnostics: stderr write failed`; pure `diag_compile_failed_json(dctx, message) -> String` reusing `diag_ctx_to_json` and `diag_json_escape_str` (shape: `{"status":"CompileFailed","executable":null,"diagnostics":…,"counterexamples":[],"message":"…"}` — schema `docs/spec/schemas/build-result.schema.json` requires `message` for CompileFailed); thin printer `diag_emit_compile_failed_json` doing `print_str` + `"\n"`.
- `diag_eprintln_best_effort(s) [io]`: `try_eprintln_str`, status discarded.
- Tests assert the exact message string, the JSON parses and carries `status`/`message`/`diagnostics`, and the message is escaped.

### 5. Wire build / verify / contracts
- `run_verify` (1874), `run_build_cmd` (2115), `run_contracts` (16294): capture the status; on `DIAG_EMIT_FAILED()` call `diag_emit_compile_failed_json(dctx, diag_emission_failure_message())` and `return 1` before the `N items` line; otherwise the `N items, M errors` line goes through `diag_eprintln_best_effort`. All other flow unchanged. Gate: step 0 harness verify/build/contracts cases turn green on the self-hosted binary.

### 6. Wire decl / complexity
- `run_decl` (decl_main.vow:46): per table; `decl_fail` and the `wrote` line use `diag_eprintln_best_effort`.
- `run_complexity` (complexity_main.vow:209-226): per table; its two later `eprintln_str` calls (212, 223) use the wrapper.

### 7. Test runner call site
- `run_test` (main.vow:2365): consume the new return value (no behaviour change; document why in one comment only if the language needs a discard binding).

### 8. Wire harness into `scripts/full_test.sh`
- Section 4j "Diagnostic emission I/O failures": run `tests/diag-io/tests.sh` for `$RUST` (`VOWC_KIND=rust`) and `$SELF` (`VOWC_KIND=self`), mirroring Sections 4h/4i, with `pass`/`fail` entries `diag-io/rust` and `diag-io/self-hosted`.

### 9. Spec + generated help
- `docs/spec/grammar.md`: add `| \`try_eprintln_str\` | \`fn(s: String) -> i64\` | \`[io]\` |` after the `eprintln_str` row, plus a short paragraph: writes `s` + `\n` to stderr; returns `0`/`1`/`2`; a closed stderr (`EBADF`) is `0`; SIGPIPE is shielded for the call so a closed pipe returns `1` instead of terminating the process.
- `uv run python scripts/generate_help.py` (regenerates `vow/src/skill.rs`, `compiler/main.vow` GENERATE blocks, `skills/vow/reference/grammar.md`), then `python3 scripts/generate_operations.py --check`.
- `docs/spec/cli.md`: no change (the `{io_error}` placeholder already covers it); re-read the `CompileFailed` rows to confirm.

## Testing / gates (background + poll; never the 2-min default timeout)
- `cargo test -p vow-runtime -p vow-types -p vow-ir -p vow-codegen -p vow-clif-shim` (use `-j 4`).
- `python3 -m unittest scripts/test_generate_operations.py`; `python3 scripts/generate_operations.py --check`; `scripts/check_help_coverage.py` (needs rebuilt `--help`).
- `cargo clippy --all --all-targets -- -D warnings`; `cargo fmt --all`.
- `scripts/bootstrap.sh --skip-cargo --no-cache` on the **final head SHA**, recorded in the PR checklist (CLAUDE.md "green locally" rule).
- `build/vowc test compiler/tests/test_diag_emit_failure.vow` and `build/vowc test compiler/tests/test_lower_catalogue_fs_stdin_args_stderr.vow`.
- `tests/diag-io/tests.sh` against both binaries; `scripts/full_test.sh` Sections 4, 4h-4j, 8, 9, 13.
- Known env flakes (memory): ~8 vow-crate run tests SKIP-panic without a linked runtime; `u64_marker_propagation`/`contracts_tmp_cleanup`/`concrete-block-region-parity` pre-exist on clean main — verify on `origin/main` before blaming this change.

## Verification surface
- Builtin is `verifier_model: unmodeled`; `compiler/vc_ops.vow` `catalogue_verifier_known` stays `return false;`. `c_emitter.{rs,vow}` untouched → byte-identical C parity (Section 2c) unaffected.
- No new vows: all changed functions are `[io]`. Do **not** add `requires`/`ensures` to fit the status range; a vowed effectful function is `Skipped` and fails `vowc build` closed.
- No `tests/verify*` fixture added (would regenerate `docs/verifier-eval.md` counts); the `tests/run` fixture is enough.

## Risks
- **Binary fixed point / bootstrap order**: `compiler/*.vow` will call a builtin an older `build/vowc` does not know. `scripts/bootstrap.sh` builds from stage 0 (built from this tree) so it works; a stale `build/vowc` cannot compile the new sources until re-bootstrapped. No pinned seed exists yet (`scripts/seed.toml` absent) — once it lands, the builtin must exist in the pin before callers use it.
- **Clif shim silent miscompile**: an unregistered symbol falls back to a no-arg/no-return signature with only a warning (`vow-clif-shim/src/lib.rs:4173`). The generated arm plus the `one_param_returns_i64` shape tests are the guard; do not skip regeneration.
- **Signal masking**: pthread-mask SIGPIPE only blocks the thread-directed signal; the drain (`sigpending`/`sigwait`) must run or the signal fires on unmask. Reuse, don't re-derive. macOS delivers SIGPIPE process-wide (see `suppress_sigpipe` comment) — the helper is per-thread and best-effort there; Linux is the CI target.
- **Stack-slot/`clif-shim` ABI**: an `i64` return from a `ptr` arg is already covered by existing shapes (`__vow_fs_rename`); no shim code change beyond the generated arm.
- **Parity of message detail**: Rust appends OS error text, self-hosted does not; harness asserts prefix only. A reviewer wanting full text needs the follow-up accessor.
- **Partial tolerance**: later bare `eprintln_str` (verify progress lines, `emit_ir_warnings` main.vow:1246, `emit_codegen_failure` 2080) still die on a closed pipe, same as today.
- **Process-wide stderr state**: `/dev/full` tests cannot observe the stderr text of decl/complexity; assert exit code + absent outputs only.
- **Clippy gate**: new Rust runtime code and tests are subject to `--all --all-targets -D warnings` (test lints gate). `noUncheckedIndexedAccess` is irrelevant (no TS).
- **parse → print → parse**: no grammar change; no canonical-printer impact.

## Out of scope (file as follow-ups, do not bundle)
- `message` on ordinary self-hosted `CompileFailed` JSON (`parse error`/`type error`/`module load error`) — pre-existing schema gap; `decl_failure_stage` (decl_main.vow:15-28) is the ready classifier.
- Backend/codegen diagnostic emission parity with #1163 (`failed to emit backend diagnostics`): `emit_codegen_failure` (main.vow:2080) and the test-runner codegen path (2513).
- `emit_ir_warnings` raw printing (main.vow:1246); all verify progress lines; `clif.vow`/`checker.vow`/`mutants_main.vow`/`runner_plan.vow` bare `eprintln_str`.
- Rust `complexity` message `vow complexity: <failure_message>` vs the self-hosted fixed `compile errors` string.
- A process-wide SIGPIPE policy for Vow programs, OS error-text accessor, changing `eprintln_str` itself, stdout write-failure handling.
- Refactors/formatting of `diag.vow`, `main.vow`, or the Rust driver.

## PR notes
- Squash-merge title (≤ 92 chars, lower-case subject): `fix(vowc): handle frontend diagnostic I/O failures like stage 0`.
- Commit order: `feat(runtime): add try_eprintln_str builtin` → `fix(vowc): …` → `test: …`/`docs(spec): …`. Implementation stage `git rm PLAN.md` before the PR.
