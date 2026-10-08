# The native verifier lives only in `compiler/` (scoped dual-compiler exception)

**Status:** accepted (2026-10-08)

Part of epic #1398 (decision D1 and the "Boot" row).

## Context

`CLAUDE.md` requires every change to land in both the Rust stage 0 compiler and
the self-hosted compiler, and treats a one-sided landing as drift debt. Epic
#1398 replaces ESBMC with a verifier written in Vow: it symbolically executes the
self-hosted SSA IR, encodes to SMT-LIB, runs Bitwuzla, and maps counterexamples
back through `Origin`/`VowEntry` (`compiler/vc_*.vow`, run in `vowc verify-worker`
subprocesses). A Rust twin of that checker would duplicate a very large
verification surface, and a byte-level parity requirement between two checkers
would grow with every feature the checker gains. It would also keep alive the
Rust-side C model the epic exists to delete.

Today none of this exists: verification is ESBMC, driven by `vow-verify` in the
Rust compiler and `compiler/verifier.vow` in the self-hosted one, and the
byte-identical C rule (`scripts/parity.py c`) keeps the two emitters in step.

## Decision

1. **The checker exists only in the self-hosted compiler.** There is no Rust port
   of it and no parity requirement for it. The exception covers the verifier
   alone: the checker, SMT encoder, solver driver, counterexample mapping and the
   worker subprocess (`compiler/vc_*.vow`, `vowc verify-worker`).
2. **The Rust stage 0 delegates verification to a pinned seed.** `vow verify` and
   `vow build` will shell out to a seed `vowc verify`. The seed is a released
   `vowc` binary pinned in `scripts/seed.toml` (version, per-platform URL,
   SHA-256), not source built from the tree. Offline or without a seed, the Rust
   compiler falls back to `--no-verify`, and the result is `Unverified`. The
   status and CLI shape of `Unverified` belongs to the CLI/status ADR (#1401),
   not here.
3. **Nothing else is exempt.** The lexer, parser, type checker, IR lowering,
   codegen, runtime builtins, and CLI flags and diagnostics outside the verifier
   stay dual-compiler. Enabling work such as `HashMap`, `getenv`/`mktemp` and
   `fs_read` error reporting keeps landing in both compilers wherever the Rust
   compiler also compiles it. The dominance validator (D13) lives in `compiler/`
   only, because it exists to feed the verifier. This is the author's reading of
   D13 and the epic's "Enabling work" list; a reviewer who reads it differently
   should say so on the PR.
4. **Transitional state.** Until the first native-verifier release is pinned, ESBMC
   and `c_emitter.{rs,vow}` stay in stage 0 and the byte-identical C parity rule
   continues unchanged. They are deleted in P6, and the pair-comparison docs
   (`docs/equivalence/`, the `equivalence-review` command) are updated then, not
   here.

## Bootstrap guarantee

`scripts/bootstrap.sh` has three stages: the Rust compiler builds `build/vowc`
(Stage 1), `vowc` rebuilds itself (Stage 2), and that result rebuilds itself again
(Stage 3). It then checks `sha256(vowc2) == sha256(vowc3)`.

**Before the seed lands (today).** Stage 1 is verified by ESBMC through the Rust
verifier path, and Stages 2 and 3 by ESBMC through the self-hosted path. Both
verifiers see the same tree. The fixed point is a codegen guarantee only;
verification does not change codegen, which is why `--no-verify` runs still give a
meaningful fixed point.

**After the seed lands (P6).**

- Stage 1 verification is performed by the pinned seed `vowc verify`, not by Rust
  code. The Rust stage 0 contributes no verification logic.
- Stages 2 and 3 are verified by the tree's own checker, so the checker verifies
  the compiler that contains it.
- The seed is trusted on its hash pin; it is not rebuilt from the tree. A
  soundness bug in the seed can let an unsound tree pass Stage 1. The exposure is
  bounded to one seed version, and the tree's own checker (a different build)
  verifies the same tree at Stages 2 and 3.
- Stage 1 verification is no longer a function of the checked-out tree alone. A
  "green locally" claim therefore depends on the seed pin as well as the head SHA,
  and both must be recorded.
- The fixed point still covers codegen only; it says nothing about checker
  soundness. The circularity of the checker verifying itself is mitigated by the
  epic's oracles (the #1084 differential against the runtime, mandatory
  `--replay-cex`, and a CI-only Z3 cross-check of emitted queries), not by
  bootstrap.
- A seed advances only through a PR that edits `scripts/seed.toml`, and only to a
  released build that already passed the epic's acceptance gate.

**Degraded mode.** With no seed (offline, or an unsupported platform) bootstrap
runs with `--no-verify`. It then guarantees the fixed point only. That is a
weaker guarantee and must be reported as such, never as a verified pass.

## Why this meets the language-design criteria

This changes no language surface, so criteria 2 and 3 do not apply.

- **Does not make verification harder.** It removes the second implementation of
  the verifier and the parity burden that would come with it, so the verification
  surface has one definition instead of two.

## Alternatives rejected

- **A Rust twin of the checker.** Duplicates the verification surface, adds a
  parity burden that grows with the checker, and keeps the Rust-side C model the
  epic is deleting.
- **Stage 0 skips verification (`--no-verify` at Stage 1).** Silently lowers the
  guarantee and conflicts with the fail-closed principle. The pinned seed keeps
  Stage 1 verified.
- **Linking or embedding the self-hosted checker into the Rust binary.** Needs an
  FFI or build cycle between the two compilers. (This is the author's
  reconstruction of why it was not chosen.)

## Consequences

- `CLAUDE.md` carries a scoped exception to the dual-compiler rule, pointing here.
  The "Rust-only is not an exemption" rule stays in force for everything else.
- A change that touches both the verifier and a dual-compiler component lands the
  dual-compiler part in both compilers and the verifier part in `compiler/` only.
- The seed, `scripts/seed.toml`, `--backend native` and `vowc verify-worker` do
  not exist yet. Later issues in epic #1398 introduce them; this ADR records the
  decision they implement. Verification semantics are covered by #1400 and the CLI
  and status surface by #1401.
