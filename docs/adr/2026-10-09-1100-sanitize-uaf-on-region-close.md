# Sanitize mode reports use-after-region-close; DoubleFree and StaleIndex are retired

Status: accepted (issue #738)

## Context

`--mode sanitize` advertised three runtime errors: `UseAfterFree`, `DoubleFree`
and `StaleIndex`. None could fire. The Vec hooks were wired into the runtime, but
nothing ever marked a tracked Vec freed, no source event produced a second free,
and no value carried an `expected_gen` to compare against a slot generation.

Vow has no per-value free: `drop` reclaims nothing, and memory returns to libc only
when a region's arena closes (`__vow_arena_close`). A Vec's descriptor and its
backing store both live in its owner arena.

## Decision

- **`UseAfterFree` is defined as a Vec/String operation after the owning region
  closed.** `__vow_arena_close` marks every tracked descriptor inside each chunk
  it frees; `alloc_chunk` purges tombstones inside a freshly malloc'd range so a
  recycled address never flags its new occupant. Every runtime-owned descriptor
  is tracked (`alloc_owned_vow_vec_descriptor`). This is a check on the region
  analysis, and a hit means the analysis placed a value too deep.
- **`DoubleFree` is retired.** Arena close is idempotent by design and closing a
  never-opened arena is legal, so a second close cannot be told from a legal one,
  and there is no user-visible free to repeat.
- **`StaleIndex` is retired.** Indices are plain integers; no handle type carries a
  generation. Adding one is a new type-system axis, which fails the language-design
  criteria in CLAUDE.md. Out-of-range reads are already `IndexOutOfBounds` in every
  mode.
- The per-slot generation counters, `SANITIZE_GLOBAL_GEN`,
  `__vow_sanitize_vec_generation` and `__vow_sanitize_check_generation` are
  removed with them.

## Consequences

- The fix lives entirely in `vow-runtime`, which both compilers link, so no
  codegen, IR, clif-shim or C-emitter change is needed and the verifier is untouched.
- The shadow table is ordered (`BTreeMap`) so chunk ranges can be matched in
  O(log n); the cost is paid only when sanitize is enabled.
- Tombstones for chunks whose address is never reallocated stay until exit,
  bounded by the number of distinct tracked descriptors ever freed.
- A future handle or iterator type would justify reviving `StaleIndex`.
