# Replay renames the entry file's `main` instead of stripping it

**Status:** accepted (2026-10-10)

Resolves #1550.

## Context

`verify --replay-cex` builds a harness from the entry file's own text plus a
synthesized `fn main` that calls the failing function with the counterexample
inputs. A file that already defines `main` would end up with two. The Rust
compiler removes the first `main` in the merged module by span
(`vow/src/replay.rs`, `generate_replay_harness`); the self-hosted compiler
instead skipped such files by searching the source for the text `fn main(`.
That search was wrong in both directions (it fired on comments and string
literals, and missed `fn  main ()`), and it left the P5 "every counterexample
replays" oracle of epic #1398 vacuous for any program with a `main`.

## Decision

The self-hosted compiler locates the entry file's `main` in the merged AST and
renames it to `__replay_orig_main` in the harness text, then appends the
synthesized `main`. The rename uses only the span **start** of the function.

## Why

- **Span lengths are clamped.** The self-hosted parser packs spans into
  `start * 65536 + len` and clamps `len` to 65535 (`span_pack_to_here`). Removing
  a `main` by span would truncate one longer than 64 KiB and corrupt the harness.
  The start offset is exact.
- **Same outcome as removal.** The original `main` never runs in a harness, so a
  renamed `main` and a removed `main` replay identically. Rust keeps removal; no
  behaviour-visible Rust change was needed.
- **AST, not text.** Only items whose originating file is the entry file count,
  so a `main` in a `use`d dependency is not mistaken for the entry's.
- **A stale offset cannot corrupt a harness.** `replay_rename_main` re-validates
  `fn`, whitespace and the exact identifier `main` at the offset and otherwise
  reports the replay as `skipped`.

## Alternatives rejected

- **IR-level entry** (replay entry chosen in lowering, no source splicing):
  larger change across codegen and the verify surface, and it keeps the temp-file
  rule. Possible future work.
- **Runtime entry override** (an environment switch selecting the entry
  function): needs a language/runtime decision and adds new surface, which the
  language design criteria in `CLAUDE.md` reject for this purpose.

## Consequences

- `tests/verify-fail/replay_confirm_with_main.vow`, `replay_confirm_main_spaced.vow`
  and `replay_confirm_main_text_only.vow` pin the behaviour, and
  `scripts/full_test.sh` Section 4c checks every `// TEST: replay <status>`
  directive on both compilers.
- Known divergence, not fixed here: Rust selects the first `main` of the merged
  module, which can be a dependency's, whereas the self-hosted compiler filters
  by originating file. When only a dependency defines `main` the Rust harness
  strips the wrong span; the self-hosted harness appends a second `main`, fails to
  compile and reports an honest `skipped`.
