# Plan: make `--mode sanitize` real — UseAfterFree on region close; retire DoubleFree/StaleIndex

## Goal
Close #738 by making the one sanitizer diagnostic that has a sound meaning in Vow's memory model
(`UseAfterFree`) actually fire, and by removing the two (`DoubleFree`, `StaleIndex`) that cannot be
reached without a new language feature, so the advertised JSON errors stop being false.

## Findings (what the issue got wrong / what is really true at HEAD f25dd03f)
- The issue's premise (codegen must emit `check_generation`/`on_free` calls) is stale and not the
  right fix. The sanitizer hooks are already wired **inside the runtime**: every Vec builtin calls
  `sanitize_on_push/set/pop/clear/truncate/read` (`vow-runtime/src/lib.rs:1661,1737,1751,1763,1776,1797`
  and ~60 `sanitize_on_read` sites). Codegen only needs `__vow_sanitize_init` (already emitted:
  `vow-codegen/src/cranelift_backend.rs:2275,3376`, `vow-clif-shim/src/lib.rs:1634,1911`).
- The real defect: `ShadowVec.freed` (`vow-runtime/src/lib.rs:5300-5303`) is never set `true`
  outside a hand-built test (`:8740`). Nothing ever marks a Vec freed.
- Vow has **no per-Vec free**: `drop` frees nothing (`docs/spec/grammar.md:1319`); memory is reclaimed
  only when a region's arena is closed (`__vow_arena_close`, `vow-runtime/src/lib.rs:1007`, emitted
  for `RegionClose` at `vow-codegen/src/cranelift_backend.rs:1721` / `vow-clif-shim/src/lib.rs:2713-2734`).
  Arena close is the only "free" event; a Vec (descriptor + backing both live in its owner arena)
  touched afterwards is a real use-after-free — i.e. the sanitizer then validates the region
  analysis (`vow-ir/src/region.rs`).
- **DoubleFree has no source event.** `__vow_arena_close` is deliberately idempotent and a close of a
  never-opened (lazily opened) arena is a legitimate no-op (`:1010-1025`), so a second close cannot be
  told from a legal one, and there is no user-visible free to repeat.
- **StaleIndex has no source event.** Indices are plain integers; no handle/iterator type carries an
  `expected_gen`. Nothing in the language can supply one, and adding such a type fails CLAUDE.md's
  language-design criteria (new type-system axis). `__vow_sanitize_check_generation` is dead outside
  `#[cfg(test)]`. Out-of-range reads are already `IndexOutOfBounds` in every mode.
- Verified not inlined: `Vec::len`, `v[i]`, `v[i] = x`, push/pop/clear/truncate are all emitted as
  `CallExtern` of `__vow_vec_len|get_val|set_val|push*|pop|clear|truncate`
  (`vow-codegen/src/cranelift_backend.rs:2696-2760`, `vow-clif-shim/src/lib.rs:3775-3830`,
  `compiler/lower.vow:3770,3874,4597`), and each runtime entry runs its `sanitize_on_*` hook **before**
  dereferencing the descriptor (`__vow_vec_len` `:1699`, `__vow_vec_get_ptr` `:1790`). So no codegen
  routing change is needed. Slice 1 pins this ordering with one worker per hooked builtin.
- Every `VowVec` descriptor construction outside tests is `alloc_owned_vow_vec_descriptor` (arena
  chunk memory) or the per-thread `STDIN_LINE_SCRATCH` (`:1438-1458`, inline TLS, not `libc::malloc`).
  Tombstone addresses are therefore always inside freed arena chunks, and only a later `alloc_chunk`
  (the sole `libc::malloc` that can recycle them) can reuse them — which the slice-2 purge covers.
- `vow-runtime` is a single crate linked by both compilers, so a runtime-only fix lands in the Rust and
  self-hosted compiler simultaneously. No `compiler/*.vow` codegen, `vow-clif-shim` ABI, IR, or C-emitter
  change is needed → no binary-fixed-point or verifier-C-parity exposure.

## Assumptions
- Chosen option is a hybrid of the issue's (a)/(b): implement (a) for `UseAfterFree`, retire the two
  unreachable errors per (b). (best guess; no operator reachable — recorded on the issue by the impl stage)
- `UseAfterFree` is redefined (spec) as: a Vec operation on a Vec whose owning region's arena has
  been closed. Existing `"op"` values (`push|set|pop|clear|truncate|read`) are kept.
- Only descriptors registered by `sanitize_on_vec_new` (`__vow_vec_new_in_arena`, `:1519`) are tracked in
  the first slice; widening to every owned descriptor is slice 4 (conditional) / follow-up.
- Exit code and envelope stay as `sanitize_emit_error` emits today (stderr JSON line + text line,
  exit 134 = `VOW_RUNTIME_ABORT_EXIT`).

## Key Files
| File | Role | Lines |
|------|------|-------|
| `vow-runtime/src/lib.rs` | shadow table, arena close/alloc_chunk, all sanitizer hooks, tests | 831-856 (`alloc_chunk`), 858 (`chunk_total`), 917 (`next_chunk`), 1007-1028 (`__vow_arena_close`), 1419 (`alloc_owned_vow_vec_descriptor`), 5294-5490 (sanitizer), 6432 (`sanitize_generation_tracking` test), 8720-8760 (`rodata_trap_worker` UAF pattern), 8987 (`option_cells_shadow_untracked`) |
| `docs/spec/cli.md` | line 21 mode row; 685-707 UAF/DoubleFree/StaleIndex sections | |
| `skills/vow/reference/cli.md` | copy of cli.md; confirm whether `generate_help.py` rewrites it (it lists `reference/cli.md` at `:330`) — if not, hand-edit identically | 21, 685-707 |
| `vow/src/skill.rs`, `compiler/main.vow` | generated help/skill text (regenerated, not hand-edited) | mode row, "Runtime Error JSON" |
| `tests/debug/sanitize_vec.vow` | existing sanitize fixture (no-false-positive baseline) | |
| `tests/run/sanitize_region_close.vow` [new] | block-scoped Vec in a loop under sanitize: clean run, address recycling | |
| `scripts/full_test.sh` | Section 5c sanitize fixtures (add the new fixture to the loop at 1747) | 1708-1760 |
| `docs/adr/2026-10-09-NNNN-sanitize-uaf-region-close.md` [new] | records the redefinition + retirement | |
| `scripts/generate_help.py` | regenerates help/skill from spec | |

## Steps (TDD slices; each = one commit)

### 1. RED/GREEN — arena close marks tracked Vecs freed (runtime unit)
- **Test** (`vow-runtime/src/lib.rs` tests, subprocess pattern of `perf_vec_sort_use_after_free` at ~8730
  via `rodata_trap_worker`, because the abort calls `process::exit`): `__vow_sanitize_init`; open an arena;
  `__vow_vec_new_in_arena` + `__vow_vec_push_val_in_arena`; `__vow_arena_close`; then
  `__vow_vec_push_val`/`__vow_vec_pop`/`__vow_vec_len` read on the stale descriptor must exit 134 with
  `"error":"UseAfterFree"` and the right `op`. One worker op per hook (push/set/pop/clear/truncate/read).
  Control case: a Vec in a still-open sibling arena is untouched.
- **Impl**: change `SHADOW_TABLE` from `HashMap<usize, ShadowVec>` to `BTreeMap` (range queries; sanitize-only
  cost, O(log n)). Add `sanitize_on_arena_close(chunks)`: for each chunk `[base, base+chunk_total(base))`
  collect during the chain walk in `__vow_arena_close` **before** `libc::free`, then
  `range_mut(..)` set `freed = true` and `generations = Vec::new()` (release the per-slot storage). Early-out
  on `!sanitize_is_enabled()` (one relaxed load; no lock when disabled).
- **Reuses**: `chunk_total` (`:858`), `next_chunk` (`:917`), `sanitize_emit_error` (`:5324`).

### 2. RED/GREEN — recycled addresses never false-positive
- **Test**: worker: Vec in arena A, close A, allocate a new chunk (same `libc::malloc` size so the address
  is typically recycled), `__vow_vec_new_in_arena` at the recycled address and push → must NOT abort
  (`sanitize_on_vec_new` already resets `freed=false`). Second case: a non-Vec allocation lands on a
  tombstoned address and a *different* object is passed to a sanitized read → must not abort.
- **Impl**: in `alloc_chunk` (`:831`) under `sanitize_is_enabled()`, purge shadow entries in
  `[base, base+total)` (they are stale tombstones). Also covers chunks freed early by
  `arena_try_free_oversized_chunk` (`:1153`) whose address is reused.

### 3. Fixture — Vow-level no-false-positive regression, both compilers
- **Test**: `tests/run/sanitize_region_close.vow` [new] with `// TEST: exit 0` and `// TEST: stdout`
  lines: a loop whose body builds a block-local `Vec<i64>`, pushes, sums, and lets the region close each
  iteration; plus a function returning a Vec up one region. Add the stem to the
  `for sanitize_fixture in ...` loop at `scripts/full_test.sh:1747` (it reads `tests/run/<stem>.vow`,
  builds in sanitize mode with both compilers, runs `compare_runtime`, and checks stdout against the
  fixture's `TEST: stdout`). Under `tests/run/` it also runs as a normal release fixture in Section 4,
  which is desired (identical output). Not under `tests/debug/`: Section 5 builds that directory in
  debug mode with a default expected exit of 134.
- **Why no true-UAF Vow fixture**: region analysis rejects/prevents a source-level dangling use; the
  true-positive coverage is the runtime workers in slices 1–2.

### 4. (Conditional) widen tracking to every owned descriptor
- Move `sanitize_on_vec_new(addr)` from `__vow_vec_new_in_arena` (`:1519`) into
  `alloc_owned_vow_vec_descriptor` (`:1419`) so String/clone/sort/collect results are tracked too.
  Gate on a corpus sweep (below) showing zero new aborts; if any appear, **do not widen** — file a
  follow-up issue naming the program and the region-analysis gap (that is the sanitizer working).
  Keep `option_cells_shadow_untracked` (`:8987`) green: Option cells still must not register.

### 5. Retire DoubleFree and StaleIndex (spec + dead code)
- **Spec**: `docs/spec/cli.md` 685-707: keep UseAfterFree with the new definition ("operation on a Vec
  whose owning region was already closed"); delete the DoubleFree and StaleIndex sections; update the
  line-21 mode row to "sanitize adds debug checks + use-after-region-close detection for Vecs".
  `skills/vow/reference/cli.md`: regenerated if `generate_help.py` writes it, else hand-edited to match.
  The full set of files naming the retired errors (grep over all file types, excluding target/.git):
  `docs/spec/cli.md`, `skills/vow/reference/cli.md`, `vow/src/skill.rs`, `compiler/main.vow`,
  `vow-runtime/src/lib.rs`. `docs/spec/schemas/*.json` have no hits. Re-run the grep after the change; it
  must be empty.
- **Code** (own commit, same PR): delete `__vow_sanitize_check_generation`,
  `__vow_sanitize_vec_generation` (`:5452-5490`), the `generations: Vec<u64>` field and its writes in
  `sanitize_on_push/set/pop/clear/truncate`, and `SANITIZE_GLOBAL_GEN` together — they are coupled
  (leaving `generations` written-but-unread trips rustc dead-code under `clippy -D warnings`; leaving the
  exported functions would keep an emitter for an undocumented error). Rewrite the
  `sanitize_generation_tracking` test (`:6432`) to assert the freed lifecycle only. This also removes an
  8-byte-per-element shadow and a global-lock round-trip per element write that nothing can observe.
  Slice 1's `generations = Vec::new()` line is dropped along with the field.
- **Regenerate**: `uv run python scripts/generate_help.py`, then `cargo build --release -p vow` and
  `scripts/bootstrap.sh --skip-cargo`; commit regenerated `vow/src/skill.rs` and `compiler/main.vow`
  (never hand-edit them; `scripts/check_help_coverage.py` gates drift).
- **ADR**: `docs/adr/2026-10-09-NNNN-sanitize-uaf-region-close.md` [new] — context, the two retired
  errors and why (no source event; would need a new language feature), consequences.

## Testing / verification commands
- `cargo test -p vow-runtime` (new workers + updated lifecycle test); `cargo test -p vow-codegen -p vow-clif-shim`
  (unchanged, must stay green); `cargo clippy --all --all-targets -- -D warnings`; `cargo fmt --all`.
- Corpus sweep (slice 4 gate and false-positive hunt), under `$TMPDIR`, both compilers, capped `-j2`:
  build every `tests/run/*.vow` (skipping `verify-only`/`skip`) with `--mode sanitize --no-verify`, run, and
  compare stdout/exit with the release build. Any sanitizer abort is either a runtime bug or a real
  region-analysis finding — triage before merging.
- `scripts/full_test.sh` Section 5c (sanitize) and `scripts/check_help_coverage.py`,
  `python3 scripts/generate_operations.py --check`. Run `scripts/bootstrap.sh --skip-cargo --no-cache`
  against the final head SHA before ticking any "green locally" claim (CLAUDE.md).

## Verification surface (ESBMC / C model)
None. No contract, IR, lowering, or `c_emitter.{rs,vow}` change; runtime sanitizer code is not part of the
verified model (sanitize hooks are runtime-only). No `tests/verify*` fixtures change.

## Risks
- **False positives from address recycling** (freed chunk reused by non-Vec data): mitigated by the
  `alloc_chunk` tombstone purge (slice 2) and `sanitize_on_vec_new` reset; covered by worker tests.
- **Region-analysis unsoundness surfacing**: enabling real UAF may turn latent compiler bugs red under
  sanitize. That is the intended signal; gate via the corpus sweep and file issues rather than loosening
  the check.
- **Deadlock**: `sanitize_on_*` take `SHADOW_TABLE` and `sanitize_emit_error` exits while held; the close
  hook must release the lock before `libc::free`, and must not call a hook that re-locks (see the guard
  comment at `:8731`).
- **Non-arena descriptors**: none exist besides TLS `STDIN_LINE_SCRATCH`, which is not `libc::malloc`
  memory. Add a slice-2 worker that tombstones a freed chunk, then passes the stdin scratch descriptor
  to a sanitized read, asserting no abort; if the impl audit finds another Rust-heap descriptor, register
  it or skip tombstoning for it.
- **Unbounded tombstones**: a freed chunk whose address is never reallocated leaves its entries until
  exit. Bounded by distinct Vec addresses ever freed; accepted for a debug mode, and note that slice 1
  drops per-slot storage on free. A follow-up could cap it by purging on `__vow_arena_open` of the same
  arena slot.
- **Perf in sanitize mode**: BTreeMap + range ops O(log n); acceptable for a debug mode; disabled path is
  one atomic load.
- **Concurrency**: shadow table is a global `Mutex`; close from worker threads (process pipes,
  `piped.rs`) must stay lock-order safe — no nested `ROOT_ARENA` + `SHADOW_TABLE` acquisition in a
  different order than existing paths (`with_root_arena` holds the root lock first).
- **Dual-compiler rule**: satisfied by construction (shared `vow-runtime`); only generated help text
  changes in `compiler/main.vow`. Re-run bootstrap to confirm fixed point unchanged except help bytes.
- **Commit hygiene**: lower-case conventional subjects, e.g. `fix(runtime): mark Vecs freed on arena
  close in sanitize mode`, `docs(spec): retire DoubleFree and StaleIndex sanitizer errors`.

## Out of scope
- Any new language feature (index handles/generational refs) to make `StaleIndex` reachable.
- Per-allocation free/`drop` semantics; changing arena close idempotence.
- Sanitizing non-Vec aggregates (structs, maps, Option cells) or buffer-overrun detection.
- Codegen/IR/clif-shim changes, verifier changes, unrelated sanitize-hook refactors or formatting.
- `docs/feature-matrix.md` (lives on another branch).
