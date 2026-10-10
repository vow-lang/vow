# Plan: document tmp_path cleanup ownership at `verify_start`

## Goal
Pin, with a doc comment at the spawn site, that `verify_start` never cleans up `tmp_path`: on every
non-success return the caller owns cleanup via the `-1`/`-3` handle sentinels. Comment-only; no
behaviour change.

## Assumptions
- Size: Small (1 file, comment only; no adversarial review needed).
- The issue's pointer `compiler/main.vow:417-421` is stale. The `h == -1` cleanup now lives in
  `verify_collect` (`compiler/verifier.vow:1921-1922`), reached from `verify_collect_and_report`
  (`compiler/main.vow:1618`, `:1624`) via `verify_collect_traced` (`main.vow:1186`). The comment
  names the function, not line numbers, so it cannot go stale again.
- Scope is the issue's request (a comment at `verify_start`). Not adding a code change, a helper, or a
  Rust-side mirror: the Rust driver has no `verify_start` counterpart (tmp files there are
  `tempfile` RAII in `vow-verify/src/esbmc.rs`), and a comment has no codegen/parity effect.
- Decision on the deferred disagreement (self-evident vs. hidden invariant): document it. The
  ownership is split across `verify_start` -> `start_esbmc` -> `verify_collect` / `cmd_test`, which
  is exactly the cross-file coupling the #450 sibling comment (`/esbmc.c` suffix, verifier.vow:444-447,
  :453) already pins.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `compiler/verifier.vow` | add comment above `verify_start` | `verify_start` 1849-1867; `start_esbmc` 629-650 (existing `-3` note at 645); `verify_collect` 1905-1935; `cleanup_verify_tmp_path` 470-479; `verify_tmp_path` 427-450; `EsbmcStart` 59-64 |
| `compiler/main.vow` | read-only reference: other consumers of the sentinels | `verify_collect_and_report` 1618-1624; build/verify enqueue 1235-1239, 2052-2056; `cmd_test` `oh == -1 \|\| oh == -3` cleanup 2466-2469 and 2515-2518 |

## Facts the comment must state (verified against the tree)
1. By the time `verify_start` returns, `tmp_path` is on disk when `verify_tmp_path()` succeeded: the
   mktemp dir was created there and `start_esbmc` `fs_write`s the C source before spawning. So
   `handle == -1` (`process_start` failed) and `handle == -3` (`esbmc_bin` empty, no spawn) both leave
   a live dir + file behind.
2. `verify_start` does not remove it; it returns `EsbmcStart { handle, tmp_path, bounds }` and the
   caller owns cleanup, always via `cleanup_verify_tmp_path(tmp_path)`.
3. Owners: `verify_collect` (`handle == -3` and `handle == -1` arms, then the normal/timeout paths) for
   build/verify through `verify_collect_and_report`; the `oh == -1 || oh == -3` branches in `cmd_test`
   (main.vow, two sites) which skip `verify_collect`.
4. The no-contracts early return yields `handle: -1, tmp_path: ""`; `cleanup_verify_tmp_path` no-ops on
   an empty path, so that return needs no cleanup and is the only exception.
5. Consequence for refactors: any new consumer of `EsbmcStart`, or a change that stops routing `-1`/`-3`
   through `verify_collect`, must keep calling `cleanup_verify_tmp_path` or the mktemp dir leaks.

## Steps

### 1. Add the doc comment above `verify_start`
- **File**: `compiler/verifier.vow` (insert at line 1849, immediately above `fn verify_start`)
- **Change**: a 6-9 line `//` block (line comments only; the language has no block comments) covering
  facts 1-5 above. Match nearby tone (see 1855-1860, 1869-1878). No code, no signature change.
- **Reuses**: wording of the existing `start_esbmc` note at `verifier.vow:645`; update that note only if
  it reads inconsistently ("run_test's sentinel branch" is really `cmd_test`) - fix that one phrase to
  name `cmd_test` and the `verify_collect` sentinel arms, in the same hunk-sized edit.

### 2. Keep the three comments consistent
- **File**: `compiler/verifier.vow`
- **Change**: ensure the `verify_start` comment, the `start_esbmc` `-3` note (645) and the
  `verify_tmp_path`/`verify_tmp_dir_for_path` suffix notes (444-447, 453) do not contradict each other;
  cross-reference by function name only.

## TDD slices
No behaviour changes, so no new red/green test. The existing tests are the regression guard:
1. Pre-change: `build/vowc build --no-verify compiler/main.vow -o $TMPDIR/vow_main` builds (baseline).
2. Post-change: same build; `build/vowc test compiler/tests/ --filter verif` (any verifier-stem tests)
   still passes.
3. Optional stronger check: comments are stripped at lex time, so IR is unchanged. Compare
   `--dump-ir` of `compiler/verifier.vow` before/after (empty diff).

## Verification surface
- No contract, codegen, or C-model change; ESBMC proves nothing new; no `tests/run/` or `examples/`
  fixture changes. `vow-verify/src/c_emitter.rs` / `compiler/c_emitter.vow` are untouched, so verifier C
  parity is trivially preserved.
- Gates to run: `scripts/bootstrap.sh --skip-cargo` (fixed point must hold; comment-only edit yields
  an identical binary), `VOW_FULL_TEST_SKIP_CARGO=1 VOW_FULL_TEST_TIER15_ONLY=1 scripts/full_test.sh`
  if time allows. Record the head SHA in the PR checklist per CLAUDE.md.
- Use `VOW_CACHE_DIR=$(mktemp -d)` for any codegen comparison (stale compile cache).

## Risks
- Stale line numbers: the issue's `main.vow:417-421` is already wrong. Mitigation: reference function
  names only, never line numbers, in the comment.
- Comment must be accurate: `-3` also leaves `tmp_path` on disk (not only `-1`), and the no-contract
  path returns an empty `tmp_path`. Overstating "always on disk" would be wrong; wording follows facts 1 and 4.
- Binary fixed point / `parse -> print -> parse`: unaffected (line comments are dropped at lex time).
  The self-hosted `concat_vow.sh` bootstrap path concatenates modules; comment text containing no
  `module`/`use` line prefix is safe - avoid starting a comment line with those words.
- Clippy/cargo gates: no Rust file touched.
- Dual-compiler rule: not applicable (no syntax, semantics, builtin, or CLI change; Rust driver has no
  analogous `verify_start`). Stated in the PR body.

## Out of scope
- Moving cleanup into `verify_start` or introducing an RAII/owner type for `tmp_path`.
- Unifying the `cmd_test` sentinel branches with `verify_collect`.
- Any `docs/spec/*.md` change, Rust-side comment, or other cleanups in `verifier.vow`.

## PR notes (for the implementation stage)
- `git rm PLAN.md` before opening the PR.
- Title (<= ~92 chars, lower-case): `docs(verifier): pin tmp_path cleanup ownership at verify_start`
- Body: `Closes #473`; note the stale line pointer in the issue and where the owners actually live now.
