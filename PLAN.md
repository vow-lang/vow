# Plan: heap polish — single left-index call in sift_down, reuse parent_idx in is_*_heap

## Goal
Close #404: remove the duplicated `*_left_idx(i)` evaluation per `*_sift_down` iteration and replace the
inlined `(i - 1) / 2` in `is_min_heap` / `is_max_heap` with the existing `*_parent_idx` helper. Behavior-preserving; stdlib-only.

## Assumptions
- Restructure option for sift_down: `while !done { let l = ...; if l >= n { done = true; } else { ... } }` (the issue's second sketch) over hoisting a `let mut l` updated at loop end — one call site, no second update on every path (best guess).
- Dual-compiler rule does not apply: only `stdlib/heap/*.vow` change, no compiler/language/builtin/CLI/contract-semantics change; no `docs/spec/*.md` edit needed (`docs/spec/stdlib.md#heap` lists only public signatures/contracts, which are unchanged).
- Issue text says `min_parent_idx` has `ensures: result >= 0`; the actual file (`min_heap.vow:41-46`) has only `requires: i > 0, ensures: result < i` (u64 makes `>= 0` vacuous). Contracts are NOT touched.
- No `build/vowc` / `target/release/vow` exist in the workspace yet; implementation must build one (`cargo build --release -p vow -j2`, then `scripts/bootstrap.sh --skip-cargo` if self-hosted binary needed).

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `stdlib/heap/min_heap.vow` | min heap; apply both edits | `min_sift_down` 114-142 (cond at 120, `let l` at 124); `is_min_heap` 158-175 (inline at 168); `min_parent_idx` 41-46; `min_left_idx` 95-100 |
| `stdlib/heap/max_heap.vow` | structural mirror; apply identical edits | `max_sift_down` ~114-142 (cond ~120, `let l` ~124); `is_max_heap` ~158-175 (inline at ~168); `max_parent_idx` 41-46 |
| `stdlib/heap/main.vow` | existing demo; runtime coverage (push/pop/sift_down multi-level, `is_*_heap`) — read-only, no change expected | whole file |
| `scripts/full_test.sh` | Section 6 (line ~1593) builds `stdlib/heap/main.vow` with both compilers, compares verify JSON and runtime output — the regression gate | 1593-1618 |

## Steps

### 1. Capture baseline (before any edit)
- Build a compiler, then record output of `stdlib/heap/main.vow` in both release and `--mode debug` (debug enables the `requires`/`ensures` checks that make the double-call visible) and the `vow verify stdlib/heap/main.vow` JSON. Save under `$TMPDIR`.
- Expected demo stdout: `1 1 3 4 5 8 9 1 0 2 1 5 3 7 9 \n1 9 8 5 4 3 1 \n` (confirm from actual run; compare, don't hardcode).

### 2. Hoist `min_left_idx` in `min_sift_down`
- **File**: `stdlib/heap/min_heap.vow` (114-142)
- **Change**: loop becomes `while !done vow { invariant: i < n, invariant: n <= data.len() } { let l = min_left_idx(i); if l >= n { done = true; } else { let r = min_right_idx(i); ...existing smallest/swap logic unchanged... } }`. Keep the loop invariants verbatim; keep the explanatory comment (109-113), still accurate.
- Note `data[l]` bound now follows from the `l < n` else-branch, and `min_right_idx` is no longer called when there is no left child (a slight, safe reduction).
- **Reuses**: `min_left_idx` (95), `min_right_idx` (102), `min_swap` (48).

### 3. Same change in `max_sift_down`
- **File**: `stdlib/heap/max_heap.vow` — mirror of step 2 (comparators `>` as already present). Keep the two files structurally diffable (`diff` of the two files should differ only in names/comparators/comments, as before).

### 4. Use `*_parent_idx` in `is_min_heap` / `is_max_heap`
- **Files**: `min_heap.vow:168`, `max_heap.vow:~168`
- **Change**: `let p: u64 = (i - 1) / 2;` → `let p: u64 = min_parent_idx(i);` / `max_parent_idx(i)`. Loop invariant `i >= 1` already satisfies `requires: i > 0`; no invariant change.
- **Reuses**: `min_parent_idx` (41), `max_parent_idx` (41) — same call pattern as `min_sift_up` line 65.
- Do not edit the "Unverified" doc comment above the predicates beyond what stays true.

### 5. Run gates (separate commands, not `&&`-chained; long ones in foreground with explicit timeout)
- `build/vowc build --no-verify stdlib/heap/main.vow -o $TMPDIR/heap_demo` then run; diff against step 1 baseline (release and `--mode debug`): must be byte-identical stdout, exit 0.
- `vow verify stdlib/heap/main.vow` JSON must equal the baseline (README documents it as `VerifyFailed`/Skipped for env reasons; the change must not alter the status or add new failures — in particular `min_sift_down` must not newly fail).
- `scripts/full_test.sh` is ~40 min; run Section 6 only if the script supports filtering, otherwise run the two compile+compare commands for both compilers (`target/release/vow` and `build/vowc`) manually. Confirm no change to `docs/spec` needed: `python3 scripts/generate_operations.py --check` untouched.

## Testing (TDD note)
Pure behavior-preserving refactor: there is no failing-first test for "calls once instead of twice". Slices are characterization-first:
1. Baseline capture (step 1) = the green test the refactor must keep green (existing `stdlib/heap/main.vow` already exercises multi-level sift_down in both min and max heaps and both predicates, including a nonempty-valid case).
2. Slice A: min_sift_down edit → rerun demo+debug diff.
3. Slice B: max_sift_down edit → rerun.
4. Slice C: `*_parent_idx` in predicates → rerun (debug mode proves `requires: i > 0` holds, since a violation would abort with `VowViolation`).
No new fixture: adding a `tests/multi/heap_*` copy would duplicate the stdlib module for no new coverage. If the implementer wants a debug-mode guard, the existing demo under `--mode debug` is sufficient.

## Verification surface
- Contracts unchanged; ESBMC obligations: `min_parent_idx` `requires i > 0` at the new call site in `is_*_heap` (loop invariant `i >= 1`); `min_left_idx` `requires i <= 2^63-1` now discharged once per iteration; `data[l]` in-bounds from `l < n <= data.len()`. Per README/`docs/spec/stdlib.md#verification-status`, heap functions are mostly `Skipped` (Vec/RegionAlloc unmodelable) so verify output is expected unchanged; do not weaken contracts to chase it.
- No `tests/run/` or `examples/` growth required.

## Risks
- Verifier C parity / binary fixed point: no compiler source or emitter touched; stdlib is not part of `compiler/` bootstrap, so no fixed-point or `c_emitter` impact. `cargo clippy` gate unaffected (no Rust change).
- Restructured loop adds one nesting level; ensure `done = true` in the `l >= n` branch (omitting it makes an infinite loop since `i` doesn't change).
- Loop invariant must still hold at loop entry/after body: `i` only changes to `l` or `r`, both `< n` in the else-branch.
- Sandbox: e2e run tests may SKIP/panic without a linked runtime (known environmental issue); verify baseline vs post-change on the same toolchain, never against a different one.
- Build cache may serve stale objects after rebuilding the compiler: use `VOW_CACHE_DIR=$(mktemp -d)` for before/after runs.
- Build parallelism: cap `cargo build -j2`.

## Out of scope
- Any compiler/spec/CLI change; deduplicating min/max heap files (needs generics); changing `*_parent_idx`/`*_left_idx` contracts or adding `result >= 0`; new tests/fixtures; README/stdlib.md edits; bounds/overflow-guard comment rewording; perf measurement.

## Commit / PR
- Conventional commit, lower-case subject, e.g. `refactor(stdlib): call heap left_idx and parent_idx helpers once` (≤ 92 chars for the PR title). `git rm PLAN.md` before opening PR. PR body: "Closes #404".
