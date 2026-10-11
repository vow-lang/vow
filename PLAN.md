# Plan: incremental solver sessions for the native verifier (#1428)

## Goal
Answer the claims of one verification round through a single interactive Bitwuzla process
(shared prefix asserted once, per-claim `push`/`pop`) instead of one spawn + one full
re-rendered query per claim, with verdicts and counterexample reports byte-identical to
one-shot mode and a clean fallback to one-shot when the solver misbehaves or exits.

## Assumptions
- Scope is `compiler/` only. The native verifier is the scoped exception in CLAUDE.md (ADR-2026-10-08-1421): no Rust twin, no C-parity obligation. `vow-verify/` and ESBMC are untouched. (best guess, from the ADR)
- "Across k steps" is delivered as: one session per unwinding round (every round, and the inductive attempt, goes through `vc_run_round`). Reuse **between** rounds is not possible today: `vc_unroll` numbers instances in DFS order with radix `bound+1` (`compiler/vc_unroll.vow:88-136`), so inst ids, `v<id>` names and term order differ for every bound, and each round builds a fresh `VcArena` (`vc_claims_mode`, `compiler/vc_exec.vow:479-502`). A prefix-stable unrolling is a separate, larger change; filed as follow-up, not bundled. (best guess)
- Only decisive-and-model-free answers are taken from the session: `unsat` (any claim) and `sat` on a soft claim (checked-arith / unwinding: `vc_note_soft_sat` never reads a model). Routing of everything else:
  - hard `sat` goes straight to a one-shot **full** query, which alone decides and supplies the model (the slice is a relaxation, so asking it first is wasted);
  - `unknown`, session timeout, EOF, desync or a dead session goes through the existing `vc_check_claim` path unchanged (sliced one-shot, then full), so a claim with an easy cone and a hard full prefix is still proven as in one-shot mode.
  This keeps verdicts *and* counterexample models byte-identical to one-shot mode — an incremental solver's model for the same query is not guaranteed equal to a fresh one's. Same discipline as ADR-2026-10-11-1200 ("slice, then confirm"). (best guess)
- Per-claim session wait is capped at half of the remaining function budget; on a cap hit the session is killed and marked dead and the claim goes to the one-shot path with the other half. Without the cap a hard full prefix would burn the whole budget before the (possibly easy) sliced query is tried. The one residual place verdicts can differ from one-shot mode is a claim whose full query needs more than the capped wait *and* whose slice also fails in the other half; recorded in the ADR. (best guess)
- Push/pop chosen over `check-sat-assuming`: both work on the local 0.9.1, but a non-literal assumption term is outside SMT-LIB's standard (literals only) and may be rejected by another build, silently disabling the speedup. Verified experimentally on 0.9.1 over stdin: a malformed command, an undefined symbol inside a `push` scope, an unbalanced `pop` and a use of a declaration after its scope was popped each make Bitwuzla print `[error] <stdin>:L:C: ...` to stderr and **exit 1 with empty stdout** — it never continues past an error, so an ignored `pop` cannot leave a previous negated goal asserted. The classifier still treats any reply that is not exactly `<verdict>\n"vow-<k>"\n` (in particular any `(error` line) as a desync, and an open-time probe (`(push 1)(pop 1)(echo "vow-open")`) rejects a solver without scope support before any claim is asked.
- Session asks the **full** prefix (no per-claim slice). Slicing stays as the one-shot path and fallback. Per-claim slice + session would need guard literals + `check-sat-assuming`, which bloats the base CNF and gains little; rejected. Whether full-prefix beats sliced one-shot is a measured question (slice 6); the policy gate `vc_session_wanted` is the knob. (best guess)
- Session is used only when the round has >= 2 claims that need the solver (a single claim cannot reuse anything). (best guess)
- Test-only switch `VOW_VERIFY_NO_INCREMENTAL=1`, mirroring `VOW_VERIFY_NO_SLICE` (`compiler/vc_native.vow:82`); not documented in `docs/spec/`, only in the ADR. (best guess)
- Protocol framing: `(echo "vow-<k>")` sentinel after `(check-sat)`. Verified against the local Bitwuzla 0.9.1: `push`/`pop` work in the default CLI mode (no `:incremental` option needed), stdout is flushed per answer when reading stdin, `(echo ...)` prints `"vow-<k>"` (quoted). The session never sends `get-value`. Declarations and `define-fun`s are emitted at base level only (a declaration inside a scope is lost on `pop`).
- Evidence for "measured speedup" is an A/B table in the ADR (default vs `VOW_VERIFY_NO_INCREMENTAL=1`, median of 5, wall-clock and peak RSS), not a new CI gate or harness script. The existing replacement gate `scripts/verify_perf.py` must stay green. (best guess; keeps the PR surgical)

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `compiler/vc_session.vow` [new] | Session driver: open / ask claim / close, framing, death handling | — |
| `compiler/vc_native.vow` | `vc_check_claim` (118) takes the session fast path then falls through; `vc_run_round` (218) owns session lifetime; policy gate; env switch (82) | 82, 118-134, 218-278 |
| `compiler/vc_smt.vow` | add `vc_smt_render_cmds(ar, s, from, to)` + assertion-term render helper next to `vc_smt_render` (283) / `vc_command_text` (241) | 241-292 |
| `compiler/vc_exec.vow` | `VcClaims` (79): append-only `header` + `assumed`, monotone `header_lens`/`assumed_lens` — the invariant the session relies on; `vc_claim_script*` (162-180) is the one-shot reference | 79-98, 162-180 |
| `compiler/vc_solver.vow` | `vc_solver_check` (232) = one-shot fallback/reference, `vc_solver_args` (218), `vc_solver_locate` (197), verdict constants (10-16) | 197-263 |
| `compiler/vc_slice.vow` | unchanged; stays on the one-shot path (`vc_check_claim` 118-130) | 183-250 |
| `compiler/env.vow` / `compiler/lower.vow` | process builtins already registered (`process_start_piped`, `process_write_stdin`, `process_close_stdin`, `process_read_line`, `process_read_status`: env.vow:562-572, lower.vow:1897-1967) — no change expected | — |
| `scripts/concat_vow.sh` | `clif` FILES list: insert `vc_session` between `vc_solver` and `vc_fmt`/before `vc_native` (line 23) | 23 |
| `compiler/tests/test_vc_session.vow` [new] | pure framing/reply-classification tests | — |
| `compiler/tests/test_vc_smt.vow` | tests for the new range renderer | — |
| `tests/verify-native/tests.sh` | fake `bitwuzla` (20-90) gains stdin session modes; helpers `run_native`/`queries`/`expect`/`no_leftovers` (268-290); new section | 20-90, 268-290 |
| `tests/verify-native/pass/loop_symbolic_body_bound_16.vow` [new] | loop fixture whose claims need the solver every iteration (existing literal-bound loops fold to constants and never reach it) | — |
| `scripts/full_test.sh` | Section 4g (1448-1571): add incremental-vs-one-shot byte-identity run next to the slicing one (1563) | 1560-1570 |
| `tests/verify-native/fail/loop_symbolic_body_bound_16_wrong_post.vow` [new] | wrong-postcondition twin: hard `sat` → one-shot model path with the real solver | — |
| `docs/adr/2026-10-08-1430-native-verifier-cli-and-status-surface.md` | env table: add a `VOW_VERIFY_NO_INCREMENTAL` (test-only) row beside the `VOW_VERIFY_NO_SLICE` one added in #1615 | env table |
| `docs/spec/cli.md` | **Solver** bullet (line 76) describes "one self-contained `.smt2` per obligation": update | 76 |
| `docs/adr/2026-10-11-1500-incremental-solver-sessions.md` [new] | decision record + measured numbers | — |
| generated: `skills/vow/reference/cli.md`, `skills/vow/SKILL.md`, `compiler/main.vow`, `vow/src/skill.rs` | regenerated by `scripts/generate_help.py` from cli.md, not hand-edited | — |

## Steps

### 1. Range rendering in `vc_smt.vow`
- **File**: `compiler/vc_smt.vow`
- **Change**: `vc_smt_render_cmds(ar, s, from, to) -> String` (commands `[from,to)`, one per line, same text as `vc_smt_render`), and `vc_smt_assert_text(ar, term) -> String` reusing `vc_term_text` for assumption terms (which are term ids, not script commands). `vc_smt_render(ar, s)` becomes `vc_smt_render_cmds(ar, s, 0, len)` so there is one rendering path.
- **Reuses**: `vc_command_text` (vc_smt.vow:241), `vc_term_text` (200).

### 2. Pure framing + reply classification in `vc_session.vow` [new]
- **File**: `compiler/vc_session.vow`
- **Change**: `struct VcSession { bin, handle, sent_hdr, sent_assumed, dead, asked }`. Pure helpers: `vc_session_delta(ar, cs, ss_sent_hdr, ss_sent_assumed, k) -> String` (header cmds `[sent_hdr, header_lens[k])` then `(assert ..)` for `assumed[sent_assumed, assumed_lens[k])`), `vc_session_frame(ar, cs, k) -> String` (`(push 1)`, `(assert <negated_goals[k]>)`, `(check-sat)`, `(echo "vow-<k>")`, `(pop 1)`), `vc_session_sentinel(k) -> String` (`"vow-<k>"`), `vc_session_classify(lines: Vec<String>, k) -> i64` returning `VC_ANS_SAT/UNSAT/UNKNOWN` only when the reply is exactly `<verdict>\n"vow-<k>"`, else `VC_ANS_ERROR` (desync). Verdict parsing via `vc_parse_check_sat` (vc_solver.vow:46).
- **Reuses**: `VC_ANS_*` (vc_solver.vow:10-16), `vc_claim_script_finish` shape (vc_exec.vow:162) as the one-shot reference.

### 3. Session driver (I/O)
- **File**: `compiler/vc_session.vow`
- **Change**: `vc_session_open(bin, cs)` — `process_start_piped(bin, vc_solver_args_stdin())` (same `--bv-output-format 2`, no file arg; add `vc_solver_stdin_args()` beside `vc_solver_args`, vc_solver.vow:218); a negative handle returns a dead session. `vc_session_ask(ss, cs, k, budget_ms) -> i64` writes delta+frame in one `process_write_stdin`, reads lines with `process_read_line(handle, remaining_ms)` until the sentinel, checks `process_read_status`; any failure (write -1, EOF, status 2 timeout or the half-budget cap, unexpected line, any `(error` text) kills the child, sets `dead`, and returns a `VC_ANS_NONE` sentinel (new constant) — never an answer. Open sends header delta `[0, header_lens[first claim])` after the scope-support probe. `vc_session_close(ss)` = `process_close_stdin` + `process_kill`, idempotent, evicts the process-table entry like `vc_solver_check` does (vc_solver.vow:259).
- **Effects**: `[io, read, write]`-compatible with callers; no temp files, so no scratch dir interaction.

### 4. Wire into the round loop
- **File**: `compiler/vc_native.vow`
- **Change**: `vc_run_round` (218): after `cs` is built, count claims needing the solver (`!vc_claim_is_static_sat`); if `vc_session_wanted(count, enabled)` open the session lazily at the first such claim (after `vc_solver_locate`, line 241), close it on **every** return path (wrap: inner function returns `VcRound`, outer closes — there are 5 returns at 246, 256, 268, 275, 277). `vc_check_claim` (118) gains a `VcSession` param: if live, `vc_session_ask`; take `unsat`, or `sat` when `vc_claim_is_soft`, as final; hard `sat` ⇒ one-shot **full** query directly (`vc_claim_script`, remaining budget); `VC_ANS_NONE`/`unknown` ⇒ the existing sliced-then-full body of `vc_check_claim` with the remaining budget (`vc_solver_check` already returns TIMEOUT for `<= 0`, vc_solver.vow:233). Dead/absent session ⇒ that same existing path, unchanged. Split the existing body into `vc_check_claim_oneshot` so the session wrapper is a thin caller (surgical diff). `vc_slicing_enabled`-style `vc_incremental_enabled()` reads `VOW_VERIFY_NO_INCREMENTAL`.
- **Reuses**: `vc_claim_is_soft` (183), `vc_claim_script` (vc_exec.vow:170), `time_micros()` pattern (127).

### 5. Fixtures
- **Files**: `tests/verify-native/pass/loop_symbolic_body_bound_16.vow` [new], `tests/verify-native/fail/loop_symbolic_body_bound_16_wrong_post.vow` [new]
- **Change**: see Testing.

### 6. Measure, set the policy threshold, document
- **Files**: `docs/adr/2026-10-11-1500-incremental-solver-sessions.md` [new], `docs/adr/2026-10-08-1430-native-verifier-cli-and-status-surface.md`, `docs/spec/cli.md`
- **Change**: time `build/vowc verify --backend native --no-cache <fixture>` with and without `VOW_VERIFY_NO_INCREMENTAL=1` (median of 5; wall-clock and peak RSS via `/usr/bin/time -v`) over `tests/verify-native/**/loop_*`, `inline_call_in_loop`, the new fixture, and `tests/verify-stress`; record the table in the ADR. If full-prefix sessions lose on some shape (large independent cones), tighten `vc_session_wanted` (claim count / prefix-size heuristic) and record the rule — never remove the one-shot path. cli.md Solver bullet: claims of a round may share one solver process; only `unsat` (and a reachable-abort `sat`) is taken from it; everything else is re-decided by a standalone query; solver death falls back transparently. Then `uv run python scripts/generate_help.py`, `scripts/check_help_coverage.py`, `cargo build --release -p vow` (regenerated `vow/src/skill.rs`), `scripts/bootstrap.sh --skip-cargo`.

## Testing (TDD slices, red → green, in order)
1. **Range render** — `compiler/tests/test_vc_smt.vow`: `vc_smt_render_cmds(ar, s, 0, n) == vc_smt_render(ar, s)`; `[2,4)` yields exactly commands 2-3; empty range is `""`. Make green with step 1. Run: `build/vowc test compiler/tests/test_vc_smt.vow`.
2. **Delta/frame text** — `compiler/tests/test_vc_session.vow` [new]: build a `VcClaims` via `vc_claims_mode` on a small IR (pattern in `test_vc_slice.vow`/`test_vc_exec.vow`); assert exact command order: the concatenation of the deltas for claims `0..k` is the header prefix `[0, header_lens[k])` followed by assumptions `[0, assumed_lens[k])` interleaved only at claim boundaries (every `define-fun` precedes its first use), and frame `k` equals `push`, the negated goal, `check-sat`, sentinel, `pop`; compare against `vc_claim_script(cs, k)` with its `get-value` line removed (same header commands, same assumption set, same negated goal). `sent_*` counters advance monotonically; a repeated ask for the same `k` emits an empty delta. Green with step 2.
3. **Reply classification** — same file: `unsat\n"vow-3"\n` ⇒ UNSAT; `sat\n"vow-3"\n` ⇒ SAT; `unknown`; sentinel for a different `k` ⇒ ERROR; missing sentinel, extra line, `(error ...)`, empty ⇒ ERROR (never UNSAT). Green with `vc_session_classify`.
4. **Wiring, fake solver** — `tests/verify-native/tests.sh`: extend the fake `bitwuzla` shim: when the last arg is not a file (stdin session) run a read loop that records the whole transcript to `$FAKE_BW_DIR/s.<n>.smt2`, answers each `(check-sat)` per `FAKE_BW_MODE` (`session_unsat`, `session_sat`, `session_unknown`), echoes sentinels, and supports `session_die_after_N`, `session_garbage`, `session_hang`. Assertions (new section "Incremental sessions (#1428)"; only the tests that assert one-`.smt2`-per-claim counts run under `VOW_VERIFY_NO_INCREMENTAL=1`; the failure-mode tests (`exit1`, `hang`, `oom_msg`, `garbage`, `kill_worker`, `crash_worker`, `hang_tree`, `mixed`) stay in **default** mode with a session-aware shim branch so "falls back cleanly if the solver exits" is exercised end to end):
   - N-claim function, `session_unsat`: status Verified, exactly **1** solver process and 0 `q.*.smt2`; transcript has one `set-logic`, N `push`/`pop` pairs, header asserted once.
   - `session_sat` on a hard claim: one session + one `q.*.smt2` (one-shot confirm), report byte-identical to the `VOW_VERIFY_NO_INCREMENTAL=1` run.
   - `session_die_after_1`: claim 1 from the session, claim 2+ via one-shot; verdict equals the all-one-shot run; no leftover process/temp (`no_leftovers`).
   - `session_garbage` / `session_error_line` (prints `(error ...)`, keeps reading) / no-scope-support (`push` ⇒ exit 1 at the open probe): fallback, same verdict, no claim answered from the session. `session_hang` with a small `--timeout`: `verify_status: "timeout"` as in one-shot, process killed (`pid` file check as in the existing `hang` test).
   - single-claim function: no session (`s.*.smt2` absent).
5. **Real-solver identity** — `scripts/full_test.sh` Section 4g (~1563): per fixture also run with `VOW_VERIFY_NO_INCREMENTAL=1` and require byte-identical JSON + exit code (verdicts, counterexamples, per-clause verdicts, warnings). New fixture `loop_symbolic_body_bound_16.vow` (`while i < 16`, body checked-adds a parameter so every unrolled copy has a solver claim; ensures a true, tight postcondition) exercises many claims/round and the `-- bound 2..16` round progression. Add a `fail/` twin with a wrong postcondition to cover the hard-`sat` → one-shot-model path with the real solver (skipped when bitwuzla is absent like the rest of 4g).
6. **Speedup evidence** — no new script: run the A/B timing from step 6 (default vs `VOW_VERIFY_NO_INCREMENTAL=1`, median of 5, wall-clock + peak RSS) on `build/vowc` and paste the table into the ADR. Acceptance "measured speedup on loop fixtures" is the loop-fixture rows that reach the solver; fixtures decided by the simplifier alone are listed as unchanged.
7. **Gates**: `build/vowc test compiler/` (or `--filter vc_`), `scripts/bootstrap.sh --skip-cargo --no-cache` on the final head SHA (record the head SHA; `scripts/seed.toml` does not exist yet, so no seed pin to record), `python3 scripts/parity.py c ...` only to confirm it is unaffected, `scripts/full_test.sh` (background + poll; ~40 min), `scripts/check_help_coverage.py`, `python3 scripts/generate_operations.py --check`, and `python3 scripts/verify_perf.py` (the replacement gate, which also caps RSS at 1.25× per fixture) must stay green; it needs the pinned ESBMC on PATH, otherwise record that it was not runnable.

## Verification surface
- No language, IR, contract or C-model change: nothing new for ESBMC to prove about user programs. `compiler/*.vow` is itself verified when `scripts/bootstrap.sh` builds `build/vowc`, so new functions are verified with the file; follow `vc_solver.vow`'s style (pure parsers carry no vow blocks unless a real, tight contract exists).
- No contract may be weakened/bounded to fit ESBMC; if a new function resists verification, mark it unverifiable per repo practice rather than distort a contract.
- Fixtures to grow: `tests/verify-native/pass/loop_symbolic_body_bound_16.vow`, `tests/verify-native/fail/loop_symbolic_body_bound_16_wrong_post.vow` [both new]. No `tests/run/` or `examples/` change.

## Risks
- **Counterexample drift**: session models can differ from fresh-solver models. Mitigated structurally — models are only ever taken from one-shot full queries (assumption 3); gate is Testing slice 5.
- **Hang / deadlock on pipes**: stderr is collected unboundedly and a full stdin pipe blocks (grammar.md "Interactive child processes"). Frames are small relative to pipe buffers but the base delta for a big unrolled function can be MBs: write must not deadlock against a solver that is busy; the solver reads stdin continuously and prints only a few bytes per claim, so the documented deadlock (large write + chatty child) cannot occur — verify with the largest `verify-stress` fixture.
- **Leaked child**: every piped ID must reach `process_kill`/`process_wait`. All 5 return paths of `vc_run_round` plus the dead-session path must close; `no_leftovers` + a mid-round-return test (first hard claim `sat` in default mode) cover it. Worker is already group-killed (`process_start_capped`, vc_worker.vow:427), so a killed worker cannot orphan the solver.
- **Peak RSS**: a long-lived solver retains terms and clauses across claims, so its peak can exceed the largest one-shot run; measured in the ADR table and bounded by the existing `verify_perf.py` 1.25× per-fixture gate. If a fixture regresses, tighten `vc_session_wanted` (or recycle the session after N claims).
- **Perf regression on independent-cone functions**: full-prefix session vs sliced one-shot. Mitigation: `vc_session_wanted` policy + measurement; one-shot sliced path remains.
- **Bitwuzla version drift** (binary is not version-checked yet, cli.md:76): a build without `push`/`pop` or `echo` yields an error/EOF → dead session → one-shot fallback, never a wrong verdict. Cover with the `session_garbage` fake.
- **Timeout parity**: a session read timeout falls through to one-shot with a `<= 0` budget, which returns `VC_ANS_TIMEOUT` with raw "time budget exhausted" (vc_solver.vow:233-235) instead of the watchdog's partial output; `vc_claim_result` (vc_cex.vow:86) maps both to `VERIFY_TIMEOUT` with `ans.raw` as the text, and the same text already appears whenever an earlier claim used the budget. Confirm the tests.sh `hang` expectations only match on status.
- **Fixed point / determinism**: new module only changes the verifier path; `FILES` order in `scripts/concat_vow.sh` must put `vc_session` before `vc_native` (else concat build fails). No `HashMap`; use `Vec` indices as elsewhere. Codegen ordering of existing modules is unchanged, so the binary fixed point should hold — confirm with the bootstrap triple.
- **Checklist items that do not apply**: no Rust source change (clippy gate unaffected), no syntax or printer change (`parse → print → parse` unaffected), no `vow-clif-shim`/stack-slot change, no emitter change (C parity unaffected).
- **Not in the Rust compiler**: by the scoped exception no Rust twin is required; do not add one. `vow/src/skill.rs` changes only via `generate_help.py` regeneration.
- **`--help`/skill staleness**: cli.md edit propagates to 4 generated files; forgetting regeneration fails `check_help_coverage.py`.

## Out of scope
- Cross-round reuse (prefix-stable unrolling: iteration-major instance order, bound-independent naming, one arena across bounds) — follow-up issue to file after this PR, referencing #1428 and #1398.
- `check-sat-assuming` + guard literals to combine slicing with sessions — follow-up only if measurement shows a loss on independent cones.
- Reusing one solver process across functions or across rounds (spawn-only gain) and a pool of solver processes.
- Changing `vc_unwind_bounds`, the slicer, the term simplifier, ESBMC/Rust verifier paths, `vow-verify`, or any user-facing flag; no formatting/refactor of `vc_native.vow` beyond the touched functions.
