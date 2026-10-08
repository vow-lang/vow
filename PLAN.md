# Plan: stdin pipe and incremental stdout read for child processes (#1406)

## Goal
Add five catalogued builtins so a Vow program can start a child with piped stdin, write to it, and read its stdout one line at a time (with a timeout), enough to drive an SMT solver interactively. Both compilers, every registration site, spec docs, and a line-by-line interactive test. Part of #1398 (native verifier) — nothing in `compiler/` uses the new builtins yet, so the bootstrap fixed point is only exposed through the new registrations.

Classification: **Large** (cross-cutting registration, ~25 files, two compilers, runtime concurrency). Template followed: commit `2129324a` (hash builtins) for the registration fan-out, `fs_open/fs_read_line/fs_status` for the line-read + status API shape.

## Assumptions
- **New builtins, `process_start` unchanged** (best guess): `process_start` keeps inheriting stdin and capturing stdout/stderr; the verifier (`compiler/verifier.vow:393,600,643`), `mutants_oracle`, `runner_plan` depend on it. Alternatives (flag arg / always pipe stdin) rejected: they alter an existing signature at every site and change child stdin for existing callers.
- **API (5 ops, all `[io]`, all in `docs/spec/operations.json`)**:
  | name | signature | notes |
  |---|---|---|
  | `process_start_piped` | `fn(cmd: String, args: Vec<String>) -> i64` | handle >0, `-1` on error. stdin/stdout/stderr piped. Handle works with existing `process_wait`, `process_wait_timeout`, `process_poll_wait`, `process_kill`, `process_stdout_for`, `process_stderr_for`. |
  | `process_write_stdin` | `fn(pid: i64, data: String) -> i64` | writes all bytes + flush; `0` ok, `-1` unknown handle / not piped / stdin closed / broken pipe. |
  | `process_close_stdin` | `fn(pid: i64) -> i64` | drops the child's stdin (EOF); `0` ok (idempotent), `-1` unknown/not piped. |
  | `process_read_line` | `fn(pid: i64, timeout_ms: i64) -> String` | next stdout line **including trailing `\n`** (same convention as `fs_read_line`); `""` on timeout/EOF/error; `timeout_ms < 0` blocks, `0` polls. `arena_routing: heap_fresh`. |
  | `process_read_status` | `fn(pid: i64) -> i64` | status of the last `process_read_line` on that handle: `0` line read, `1` EOF (stdout closed, nothing left), `2` timed out (child alive), `-1` unknown handle / not piped. Mirrors `fs_status`. |
  Final unterminated line at EOF is returned without `\n` with status `0`; the next call returns `""` with status `1`.
- **Bounded memory** (CLAUDE.md production bar): per-piped-child stdout reader thread feeds a `sync_channel` of bounded chunks (16 × ≤64 KiB), so a child cannot make the runtime buffer unboundedly; backpressure matches a raw pipe. Consequence (documented): a caller that writes a huge payload without ever reading can deadlock against a chatty child — same as any raw pipe. stderr is drained by a second thread into a `Vec` (matches existing `process_start` semantics, which already buffers all stderr).
- **SIGPIPE** (verified: `vow-runtime/src/lib.rs` has no SIGPIPE handling; compiled Vow binaries enter via a Cranelift `main`, so the default disposition kills the process, while `cargo test` ignores SIGPIPE and would hide it): `process_write_stdin` to an exited child must return `-1`, not kill the program. Strategy: install `libc::signal(SIGPIPE, SIG_IGN)` once (`std::sync::Once`) on the first `__vow_process_start_piped`, then map `EPIPE` to `-1`. Only programs that use piped children change disposition. Covered by a real-binary fixture case.
- **Seed**: `scripts/seed.toml` does not exist yet, so no pinned-seed verification concern for this PR; recheck at implementation time (if it has landed, the seed will not know the new names and `compiler/` must keep not using them).
- **Arena variants**: `<name>_in_arena` signatures are derived generically (target arena first, then base params; `vow-clif-shim/src/lib.rs` ~3446), so the 3-arg `process_read_line_in_arena` needs no extra registration site; the e2e fixture proves it.
- **No verifier work**: all five are `[io]`, so `is_modelable`'s effect-emptiness gate already excludes callers from the C model (see `vow-verify/src/c_emitter.rs:3513` test comment). No `c_emitter.{rs,vow}` change, no `verifier_model` field, C parity (Section 2c) unaffected. A `tests/verify-skip/` fixture is not needed; extend the existing `process_builtins_are_not_known_to_the_verifier` symbol list instead.
- **No ADR**: this adds builtins, resolves no architecture decision. (If reviewers want one, ADR dir is `docs/adr/`.)
- **`compiler/` source must not call the new builtins in this PR** (Rust stage 0 must know them first; also keeps the fixed point trivially intact). Follow-up under #1398 adopts them.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `vow-runtime/src/lib.rs` | runtime impl: `ProcessState` (21), `PROCESS_MAP`/`POLL_READERS` (31-45), `__vow_process_start` (4056), `_wait` (4097), `_wait_timeout` (4172), `_poll_wait` (4267), `_kill` (4349), `read_file_line`/`fs_status` (3645-3697) as shape template, `alloc_bytes_string` (3671), `with_root_arena`, `_in_arena` variants (4030-4165) | new code goes after 4375 |
| `docs/spec/operations.json` | single source for symbol/ABI/return/doc; `process_*` entries 180-270, `arena_routes` list ~342 | add 5 entries + `__vow_process_read_line` to `arena_routes` |
| `scripts/generate_operations.py` | generator; run `python3 scripts/generate_operations.py` to splice projections | `RETURN_TOKENS` (36) — `ptr`/`i64` already present, no change expected |
| `scripts/test_generate_operations.py` | `PROCESS_OPS` fixture (~230-305), name-set test (665-690) | extend |
| `vow-types/src/env.rs` | `builtin_free_fn_signatures()` process defs (255-281); sorted signature listing test (~990-1004) | add 5 `def(..)` + 5 listing lines (alphabetical) |
| `vow-ir/src/lower/mod.rs` | generated `catalogue_builtin_to_runtime` (~44-66), `catalogue_builtin_result_tag` (73-82); unit tests: result-tag list (~5940-5960), runtime-symbol table (~6314-6324) | generated block + test rows |
| `vow-ir/src/region.rs` | generated `FRESH_ARENA_VARIANTS` (~2005-2030) | generated; add `__vow_process_read_line` |
| `vow-codegen/src/cranelift_backend.rs` | generated `catalogue_extern_sig` (~2462-2500); tests 3625-3665 (`process_extern_sigs_come_from_the_operation_catalogue`) | generated + test rows |
| `vow-clif-shim/src/lib.rs` | generated `catalogue_extern_sig` (~3393-3440); arena-variant list (~3538); parity test (~4325-4360) | generated + test rows |
| `compiler/env.vow` | `env_define_fn` for process ops (~528-540) | add 5 lines |
| `compiler/lower.vow` | generated `catalogue_builtin_to_extern` (~1808-1840), `catalogue_builtin_ret_ty` (~1847-1875), `catalogue_builtin_result_tag`; `builtin_result_tag` (1619) | generated |
| `compiler/ir.vow` | generated `fresh_arena_base_extern` decision tree (~435-455, keyed on length+char) | generated |
| `compiler/tests/test_op_catalogue.vow` | `check_process_externs`/`_ret_tys`/`_result_tags` | extend with new ops (numeric failure codes unique) |
| `compiler/tests/test_fresh_builtin_routing.vow`, `compiler/tests/test_lower_builtin_result_tag.vow` | fresh-arena + StringHeap tag pins | add `process_read_line` rows |
| `docs/design/arena_memory.md` | lists fresh-routed builtins (~572) | add `process_read_line` |
| `docs/spec/grammar.md` | builtin signature table (~1460-1469) + semantics paragraphs (~1479-1491) | add rows + `**Interactive child processes:**` paragraph |
| `skills/vow/reference/grammar.md`, `vow/src/skill.rs`, `compiler/main.vow` | embedded/generated docs + `--help` JSON (`skill.rs` ~951, `main.vow` ~3211, ~5098, ~10972) | regenerate via `uv run python scripts/generate_help.py` — never hand-edit |
| `vow-verify/src/c_emitter.rs` | `process_builtins_are_not_known_to_the_verifier` (3513-3525) | add new syms to the test list only |
| `tests/run/process_interactive_lines.vow` [new] | e2e line-by-line driver | see Testing |

## Steps (ordered; map 1:1 to TDD slices below)

### 1. Runtime: piped-child state + `__vow_process_start_piped` (`vow-runtime/src/lib.rs`)
- Add `struct PipedChild { stdin: Option<ChildStdin>, lines: Receiver<Vec<u8>> chunks, pending: Vec<u8>, eof: bool, status: i64, stdout_thread: Option<JoinHandle<()>>, stderr_thread: Option<JoinHandle<Vec<u8>>> }` and `static PIPED_MAP: Mutex<Option<HashMap<i64, Arc<Mutex<PipedChild>>>>>` next to `POLL_READERS` (init helper mirroring `process_map_init`). HashMap is fine at runtime (keyed lookup only, no iteration order observable; matches existing maps).
- `__vow_process_start_piped(cmd_ptr, args_ptr)`: factor the cmd/args decoding shared with `__vow_process_start` (4056-4082) into one private helper (avoids a copy-paste of 25 lines; touch `__vow_process_start` only to call it — behavior-preserving). Spawn with all three stdio piped, take stdin/stdout/stderr, start the two threads, insert `ProcessState::Running(child)` into `PROCESS_MAP` and the entry into `PIPED_MAP` under one fresh `NEXT_PROCESS_HANDLE`.

### 2. Runtime: write/close stdin
- `__vow_process_write_stdin(handle, data_ptr)`: `sanitize_on_read`, copy bytes out of the `VowVec`; clone the `Arc` out of `PIPED_MAP`, **drop the map lock**, lock the entry, `write_all` + `flush`; `0` / `-1`.
- `__vow_process_close_stdin(handle)`: `entry.stdin.take()`; `0`, idempotent; `-1` unknown/not-piped.

### 3. Runtime: incremental read
- `timeout_ms` is a **total deadline** (computed once with `Instant`) across all chunk receives, not per-`recv`, so a trickling child cannot hold the call open. On timeout, any partial data stays in `pending` for the next call. Keep `stdin` in its own `Mutex<Option<ChildStdin>>` separate from the reader state so a blocked reader cannot stall `write_stdin`/`close_stdin`/`wait`.
- Helper `piped_read_line(handle, timeout_ms) -> Vec<u8>`: loop — if `pending` contains `\n`, split and return (status 0); else `recv_timeout`/`recv` next chunk and append; `Disconnected` ⇒ `eof=true`: return remaining `pending` (status 0 if non-empty else 1); `Timeout` ⇒ status 2, return empty. Unknown handle ⇒ empty, and `__vow_process_read_status` returns `-1`. Never hold `PROCESS_MAP`/`PIPED_MAP` while blocking (only the per-entry lock).
- `__vow_process_read_line_in_arena(arena, handle, timeout_ms)` and `__vow_process_read_line(handle, timeout_ms)` exactly like `__vow_fs_read_line[_in_arena]` (3676-3685) using `alloc_bytes_string` / `with_root_arena`. `__vow_process_read_status(handle)` like `__vow_fs_status` (3688).

### 4. Runtime: lifecycle integration with existing process ops
- **Drain-before-wait (deadlock guard, the #784 hazard for piped children)**: the bounded stdout channel means an unread chatty child blocks in `write`, so it never exits. Therefore: `process_wait` first closes stdin, then drains the channel into `pending` until `Disconnected` **before** `child.wait()`; `poll_wait`/`wait_timeout` `try_recv`-drain into `pending` on every poll tick; release always drains-then-joins. Unread output accumulated this way is bounded only by what the child actually emits (same as existing `process_start` capture), which is acceptable because the caller asked to wait.
- `piped_release(handle) -> Option<(Vec<u8>, Vec<u8>)>`: removes the entry, drops stdin, drains the channel then joins reader/stderr threads (the child has already exited or been killed, so they terminate), returns (unread stdout = `pending` + drained channel, stderr).
- `__vow_process_wait`: for a piped handle close stdin **before** `wait()` (otherwise a child waiting for EOF hangs); use `child.wait()` instead of `wait_with_output` (stdout/stderr already taken), then fill `ProcessState::Completed` from `piped_release`.
- `__vow_process_wait_timeout` / `__vow_process_poll_wait` completion paths and `__vow_process_kill`: same release, so `process_stdout_for`/`process_stderr_for` return the unread stdout and full stderr after completion; `process_kill` must kill the child first, then release (reader `recv` then sees `Disconnected`, so a thread blocked in `process_read_line` unblocks). No leaked threads/fds (CLAUDE.md memory discipline).

### 5. Catalogue entries + regenerate projections
- `docs/spec/operations.json`: add the five entries (params/return per Assumptions table, `effects: "[io]"`, `process_read_line` with `"arena_routing": "heap_fresh"`) and `"__vow_process_read_line"` to `arena_routes`.
- `python3 scripts/generate_operations.py` splices `vow-ir/src/lower/mod.rs`, `vow-ir/src/region.rs`, `vow-codegen/src/cranelift_backend.rs`, `vow-clif-shim/src/lib.rs`, `compiler/lower.vow`, `compiler/ir.vow`. Then `--check`.

### 6. Hand-registered signature tables (the part generators do not cover)
- `vow-types/src/env.rs`: five `def(..)` next to the process defs (`process_start_piped`: `[Ty::Str, vec_ty(Ty::Str)] -> I64`; `write_stdin`: `[I64, Str] -> I64`; `close_stdin`/`read_status`: `[I64] -> I64`; `read_line`: `[I64, I64] -> Str`; all `&[Effect::IO]`) and five alphabetical lines in the sorted-listing test.
- `compiler/env.vow`: five `env_define_fn(..., eff_io)` mirroring them (Str param ids `str_tid`, `vec_str_tid`, `i64_tid`).
- `vow-ir/src/lower/mod.rs` tests: add rows to the StringHeap-tag test (`process_read_line`) and the runtime-symbol table test.

### 7. Docs
- `docs/spec/grammar.md`: five table rows + `**Interactive child processes:**` paragraph (start/write/close/read semantics, trailing `\n` included, status codes 0/1/2/-1, `timeout_ms` meaning, close-stdin-before-wait requirement, bounded-buffer/deadlock note, unread stdout still available via `process_stdout_for` after wait).
- `uv run python scripts/generate_help.py` (regenerates `skills/vow/reference/grammar.md`, `vow/src/skill.rs`, `compiler/main.vow`), then `python3 scripts/check_help_coverage.py`.

### 8. E2E fixture + wiring
- `tests/run/process_interactive_lines.vow` [new] (details below), directives `// TEST: stdout "..."`, exit 0. Picked up automatically by `scripts/full_test.sh` Section 4 for both compilers.

## TDD slices (red → green, each independently committable)
1. **Start/write/read happy path** — RED: `vow-runtime/src/lib.rs` `#[cfg(test)]` test `process_piped_echoes_line_by_line`: start `sh -c 'while IFS= read -r l; do echo "got:$l"; done'`, write `"a\n"`, `read_line(h, 2000)` == `"got:a\n"`, status 0; write `"b\n"`, read again (proves the child answered *before* stdin closed = genuinely interactive); close stdin; read → `""`, status 1; `process_wait` == 0. GREEN: steps 1-3.
2. **Timeout and unterminated tail** — RED: partial-then-complete: child prints `par`, sleeps ~300 ms, prints `tial\n`; `read_line(h, 50)` → `""` status 2, then `read_line(h, 3000)` → `"partial\n"` status 0 (deadline is total, partial retained). Also: `cat >/dev/null`-style child, `read_line(h, 50)` → `""`, status 2, child still alive (`poll_wait` returns `VOW_PROC_STILL_RUNNING`); `printf 'x'` child → `"x"` status 0 then `""` status 1. GREEN: step 3 refinements.
3. **Error paths** — RED: write to an exited child returns `-1` (not SIGPIPE-killed; unit test covers EPIPE mapping, the e2e fixture covers the real-binary signal disposition); unknown handle, handle from plain `process_start`, write after close, `-1` everywhere; `read_status(-1)` == -1; invalid cmd ⇒ `-1` handle. GREEN: guards in 2/3.
4. **Lifecycle** — RED: (a) `process_kill` of a piped child while another thread blocks in `read_line(h, -1)` returns promptly and the reader unblocks; (b) after `wait`, unread stdout is in `process_stdout_for`, stderr in `process_stderr_for`; (c) wait without explicit close_stdin on a `cat` child terminates (auto-close); (d) wait_timeout/poll_wait completion path. GREEN: step 4. (e) **no-read deadlock guard**: child emits ~2 MB (`head -c 2000000 /dev/zero`) and exits, caller does zero reads, then `process_wait` returns 0 and `process_stdout_for` has the full 2000000 bytes; repeat via `process_poll_wait` loop and `process_wait_timeout`. Also assert `PIPED_MAP` is empty after release (no leak).
5. **Fresh-arena variant** — RED: extend `fresh_process_builtins_allocate_in_the_requested_arena` (lib.rs ~6039) with `__vow_process_read_line_in_arena`. GREEN: step 3.
6. **Catalogue + ABI registration (Rust)** — RED: `scripts/test_generate_operations.py` (`PROCESS_OPS`, name-set test, `test_real_catalogue_projections_are_up_to_date`), cranelift_backend/clif-shim signature tests, `vow-types` sorted listing test, `vow-ir` result-tag/runtime-table tests. GREEN: steps 5-6 (`generate_operations.py` + `env.rs`).
7. **Catalogue registration (self-hosted)** — RED: `compiler/tests/test_op_catalogue.vow`, `test_fresh_builtin_routing.vow`, `test_lower_builtin_result_tag.vow` (`build/vowc test compiler/tests/test_op_catalogue.vow` etc.). GREEN: generated `lower.vow`/`ir.vow` + `env.vow`.
8. **E2E in both compilers** — RED: `tests/run/process_interactive_lines.vow` (Vow: `process_start_piped("sh", ["-c", loop script])`; for each of 3 lines: `process_write_stdin`, `process_read_line(pid, 5000)`, assert `process_read_status == 0`, `print_str`; `process_close_stdin`; `process_read_line` → len 0 and status 1; `process_wait` == 0; plus a timeout probe `process_read_line(pid2, 50)` status 2 then `process_kill`; plus a SIGPIPE case: start `sh -c 'exit 0'`, read until status 1, `process_wait`, then another piped `sh -c 'exit 0'` child: after EOF `process_write_stdin` must return `-1` and `main` must still return 0). Fails to type-check before step 6. GREEN: whole stack. Validate with both `./target/release/vow` and `build/vowc` (full_test.sh Section 4 runs both).
9. **Docs gate** — `grammar.md` rows + `generate_help.py` + `check_help_coverage.py` + `generate_operations.py --check` clean.

## Testing / verification commands (run separately, not `&&`-chained)
- `cargo test -p vow-runtime process_`; `cargo test -p vow-ir`; `cargo test -p vow-codegen`; `cargo test -p vow-clif-shim`; `cargo test -p vow-types`
- `cargo clippy --all --all-targets -- -D warnings`; `cargo fmt --all`
- `python3 scripts/generate_operations.py --check`; `python3 -m unittest scripts.test_generate_operations` (or the repo's invocation); `uv run python scripts/generate_help.py`; `python3 scripts/check_help_coverage.py`
- `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA (record SHA in PR checklist per CLAUDE.md); then `build/vowc test compiler/` (or the three touched test files) and `scripts/full_test.sh` (~40 min; run in background with polling bounded to the turn, per run-context rules).
- Run the new fixture with `VOW_CACHE_DIR=$(mktemp -d)` (stale compile cache after compiler changes).
- Known environmental failures (verify against clean `origin/main` before blaming): ~8 vow-crate run tests that SKIP-panic without a linked runtime; `u64_marker_propagation`/`contracts_tmp_cleanup`; `concrete-block-region-parity`.

## Verification surface (ESBMC)
- No contracts added or changed; all new ops are `[io]` so callers are excluded from the C model; nothing for ESBMC to prove. No `tests/verify*/` fixture growth, no change to `c_emitter.{rs,vow}`, Section 2c parity untouched. Only extend the `is_known_builtin` negative test with the new symbols.
- `examples/`: optional; not required (skip).

## Risks
- **Binary fixed point**: `compiler/` itself must not call the new builtins (stage 0 and seed `vowc` both must accept every identifier in `compiler/`). The only fixed-point exposure is the new tables: `env.vow`/`lower.vow`/`ir.vow` additions are straight-line `if name == ...` chains / decision tree — deterministic, no `HashMap` iteration. Re-run `bootstrap.sh --skip-cargo --no-cache` and compare `sha256sum` of stage 1/2 outputs.
- **`fresh_arena_base_extern` decision tree** (`compiler/ir.vow`) is keyed on symbol length and a character position; `__vow_process_read_line` (23 chars) must not collide with an existing branch — rely on the generator and `test_fresh_builtin_routing.vow`; check `region.rs` consistency tests.
- **Concurrency/deadlock**: never hold `PROCESS_MAP` while blocking (existing `wait` already does — pre-existing — so interactive use must not call `process_wait` on one handle while another thread reads another; document, don't redesign). Per-entry `Arc<Mutex<>>` lock held during a blocking read serializes same-handle ops only. `process_kill` ordering (kill child → release) is what unblocks readers; test slice 4a pins it.
- **Existing `wait`/`wait_timeout`/`poll_wait` use `wait_with_output` or take `child.stdout`**: for a piped child stdout/stderr are already taken ⇒ would silently return empty Completed output. Step 4 patches each completion path; slice 4b/4d guard against regression. Plain `process_start` children must be byte-for-byte unchanged (existing tests at lib.rs 5546-5610 and `tests/run/process_catalogue_smoke.vow` must stay green).
- **Zombie/fd leak** if a Vow program never waits/kills a piped handle: same as existing `process_start` (documented contract: call `process_wait` or `process_kill`). Not worsened beyond two extra threads per handle, released at wait/kill.
- **Flaky timing in tests**: use generous read timeouts (≥2 s) for positive reads; only the "timeout" assertions use short timeouts and run against a child that provably never writes (`cat`/`sleep`). Avoid `sleep`-based sequencing.
- **Generator coverage**: `generate_operations.py --check` doesn't check `vow-types/src/env.rs` or `compiler/env.vow` — those hand tables are the most likely omission; slices 6/7/8 fail loudly if missed (type-check error on the e2e fixture in each compiler).
- **Windows/platform**: runtime is already unix-assuming for `sh`; fixture uses `sh`. Same as `process_catalogue_smoke.vow`.
- **clippy gate**: `--all-targets`; new test helpers must be warning-free; `unsafe extern "C"` fns follow the existing `#[unsafe(no_mangle)]` style.
- **Codecov patch gate (95%)**: the `.vow` corpus runs an uninstrumented binary, so runtime branches (timeout, EOF tail, errors, release paths) need direct `vow-runtime` unit tests — slices 1-5 provide them; do not re-indent existing lines.
- **Commit hygiene**: PR title `feat(runtime): stdin pipe and incremental read for child processes` (lower-case subject, ≤92 chars). Suggested commit split: runtime+tests → catalogue/regeneration → hand tables (both compilers) → docs/help → e2e fixture. Implementation stage must `git rm PLAN.md` before opening the PR.

## Out of scope
- Using the new builtins from `compiler/` (verifier SMT driver, epic #1398 follow-ups); a Vow-level `Process` wrapper type or linear handle type (new type-system axis — rejected by CLAUDE.md language-design rules).
- Changing `process_start`/`process_run` semantics, merging stderr into stdout, non-line (byte-count) reads, reading stderr incrementally, a `select`-style multiplexing API, Windows support.
- Refactoring the existing `wait`/`kill` functions beyond the minimal lifecycle hooks and the shared cmd/args decode helper; formatting or unrelated cleanups; ADR; verifier/C-emitter changes; mutation-testing, bench or memory-bound additions.
