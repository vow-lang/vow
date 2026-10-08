# Plan: #1399 docs(adr): verifier lives only in compiler/ (dual-compiler exception)

Child of epic #1398 (decision D1 and the "Boot" row). Docs-only: one new ADR and one scoped CLAUDE.md amendment. No compiler, spec, or test code changes.

## 1. Problem restated

CLAUDE.md requires every change to land in both the Rust stage 0 compiler and the self-hosted compiler. The native verifier (epic #1398: symbolic execution of the self-hosted SSA IR, SMT-LIB, Bitwuzla; `compiler/vc_*.vow`) will exist only in `compiler/`. Writing a Rust twin would defeat the point of replacing ESBMC and would double a 35k-line-scale verification surface. We need an ADR recording this as a scoped exception to the dual-compiler rule, and a CLAUDE.md amendment stating the exception (verifier only) and its consequence for the bootstrap guarantee: how the Rust stage 0 gets verification (a pinned seed `vowc verify`) and what bootstrap does and does not prove about verification before and after the seed lands.

## 2. Files to touch

- **New** `docs/adr/2026-10-08-HHMM-verifier-lives-only-in-compiler.md`. Use the real UTC time at authoring (`date -u +%Y-%m-%d-%H%M`), per `docs/adr/README.md`. Reference it as `ADR-YYYY-MM-DD-HHMM`.
- **Edit** `CLAUDE.md`, section "Vow Compiler" (the "always modify BOTH" paragraph and the paragraph after it). Add a short "Scoped exception: the verifier" paragraph with a link to the ADR. Keep it narrow: it must not weaken the "Rust-only refactors are not exempt" sentence. Also adjust the "Bootstrap (Rust Compiler)" or "Verifier C Parity" prose only if it would now contradict the exception. Make the C-parity section say the parity rule stays in force for `c_emitter.{rs,vow}` until they are deleted in P6.
- No `docs/spec/*.md` change (no syntax, semantics, builtin, or CLI flag change). The `--backend native`, seed, and `Unverified` surface belong to the CLI/status ADR (#1401) and the later implementation issues, so the ADR should cross-reference them and not define them.
- `docs/adr/README.md`: no change needed. It describes naming only.
- Docs-only CI: `scripts/ci_docs_only.py` treats `docs/adr/*` and `CLAUDE.md` as prose, so CI should take the fast path. Do not touch anything under `docs/spec/`.

## 3. TDD slices

This is documentation, so there is no red-green code cycle. Each slice is a reviewable commit-sized step with a mechanical check.

1. **Draft the ADR skeleton.** Sections, in the style of `2026-10-04-0615-linear-discharge.md`: title, `**Status:** accepted (date)`, Context, Decision, Consequences (bootstrap guarantee), "Why this meets the language-design criteria" (verification gets simpler; no new language surface, so criteria 2 and 3 are N/A or neutral), Alternatives rejected, Scope/non-goals.
   Check: `markdownlint`-style sanity by eye; `python3 scripts/ci_docs_only.py` (or its test `python3 -m unittest scripts/test_ci_docs_only.py`) still classifies the new path as prose.
2. **Fill in Context and Decision** from the epic's D1 and Boot rows:
   - The checker (`compiler/vc_*.vow`, `vowc verify-worker`) exists only in the self-hosted compiler. No Rust twin: no `vow-verify` port of the new checker, no parity requirement for it.
   - The Rust stage 0 `vow verify`/`vow build` delegates verification by shelling out to a pinned seed `vowc verify` (`scripts/seed.toml`: version, per-platform URL, SHA-256). The seed is a released `vowc`, so it is a binary artifact, not source.
   - Scope of the exception: verifier only (checker, SMT encoder, solver driver, counterexample mapping, worker subprocess). Everything else (lexer, parser, type checker, IR lowering, codegen, runtime builtins, CLI flags and diagnostics outside the verifier) stays dual-compiler. In particular, enabling work such as HashMap, `getenv`, `mktemp` and the dominance validator keeps both-compiler landing wherever the Rust compiler also compiles it. State plainly that the dominance validator lives in `compiler/` only, because it exists for the verifier. Confirm this reading against D13 and flag it as the author's interpretation.
   - Transitional state: until the first native-verifier release is pinned, ESBMC and `c_emitter.{rs,vow}` stay in stage 0 and the existing byte-identical C parity rule (`scripts/parity.py c`) continues unchanged. After P6, they are deleted.
3. **Fill in the bootstrap-guarantee section** (acceptance criterion 3), as a before/after table or two short lists:
   - *Before the seed lands (today)*: Stage 1 `target/release/vow build` verifies the self-hosted compiler with ESBMC (Rust path); Stage 2 and 3 `vowc build` verify with ESBMC (self-hosted path). The SHA-256 fixed point (`vowc2 == vowc3`) is a codegen guarantee. Verification does not alter codegen, so `--no-verify` runs keep the fixed point meaningful.
   - *After the seed lands (P6)*: Stage 1 verification is performed by the pinned seed `vowc verify`, not by Rust code. So the Rust stage 0 contributes no verification logic. The seed is trusted on a hash pin, not rebuilt from the tree. Stages 2 and 3 are verified by the tree's own checker, so the checker is verifying the compiler that contains it. Consequences to state honestly:
     - A soundness bug in the seed can let an unsound tree pass Stage 1. It is bounded to one seed version and surfaces later as the Stage 2/3 native checker (a different build) verifying the same tree.
     - Stage 1 verification is no longer a function of the checked-out tree alone, so a "green locally" claim depends on the seed pin. Per CLAUDE.md, record the SHA of the final head.
     - The fixed point still covers codegen only; it says nothing about the checker's soundness. Circularity is mitigated by the epic's oracles (#1084 differential, mandatory `--replay-cex`, CI-only Z3 cross-check), not by bootstrap.
   - *Degraded modes*: offline or no seed means `--no-verify` (result `Unverified`), so bootstrap then guarantees the fixed point only. Name this as a weaker guarantee, not a pass. Do not specify the `Unverified` JSON/status shape here (that is #1401); link to it.
   - *Seed bumps*: the seed advances only by a PR that updates `scripts/seed.toml`. A seed has to be a released build that already passed the epic's acceptance gate.
4. **Alternatives rejected** (short): (a) a Rust twin checker, rejected for duplicated verification surface and a parity burden that grows as the checker does, and because the epic's goal is to remove the Rust-side C model; (b) Rust stage 0 skipping verification entirely and `--no-verify` at Stage 1, rejected as it silently lowers the guarantee and conflicts with the fail-closed principle; (c) vendoring the checker as an embedded binary or linking the self-hosted checker into the Rust binary, rejected as it needs an FFI or build cycle between the two compilers. Mark (c) as the author's reconstruction of why it was not chosen, and cut it if unsure when reviewing.
5. **Amend CLAUDE.md** with a compact paragraph (about 8 lines):
   - States the exception, scoped to the verifier only, and that the "primary compiler" argument (the self-hosted compiler is primary) is exactly why a one-sided landing is allowed here.
   - States that no other component may claim it, and that a change that is "Rust-only or Vow-only" outside the verifier remains drift debt.
   - States the bootstrap consequence in two sentences: fixed point guards codegen; Stage 1 verification comes from the pinned seed after P6, so checklist claims must record both the head SHA and the seed pin.
   - Links to the ADR by its `ADR-YYYY-MM-DD-HHMM` id and path.
   - Keep the exception text to what holds today ("will" for the not-yet-landed seed), so CLAUDE.md does not describe `scripts/seed.toml` as if it already exists.
   Check: `grep -n "BOTH the Rust compiler" CLAUDE.md` still returns the original rule; the new paragraph sits directly after it.
6. **Cross-reference check.** Verify links resolve: `ls docs/adr/<new file>`; and that every `#NNNN` named (1398, 1400, 1401, 1084) exists (`gh issue view`). Run `python3 -m unittest scripts/test_ci_docs_only` in `scripts/` to confirm that the prose classification still holds. Run `pre-commit run --files CLAUDE.md docs/adr/<new file>` if hooks are installed, and run the commit-msg hook via a normal `git commit`.
7. **Commit and PR.** One commit: `docs(adr): record verifier-only-in-compiler dual-compiler exception`. The PR title must be a conventional-commit subject of at most about 92 characters, lower-case, e.g. `docs(adr): exempt native verifier from the dual-compiler rule`. Body: `Closes #1399`; "Part of #1398". The `git rm PLAN.md` happens before opening the PR, per the stage handoff. Do not tick any "bootstrap green" checklist, since this PR changes no code, and say so in the PR body.

## 4. Verification surface

None. No contracts, codegen, C model, or fixtures change. No `tests/run/` or `examples/` growth. `generate_help.py` and `check_help_coverage.py` are unaffected because `docs/spec/` is untouched. Do not run the full bootstrap or `full_test.sh` for this PR, as it is prose-only.

## 5. Risk areas

- **Binary fixed point, parse-print idempotency, clippy:** untouched.
- **Over-claiming.** The seed, `scripts/seed.toml`, `--backend native`, and `vowc verify-worker` do not exist yet. The ADR must say "will", and distinguish today's state from the target state, or the doc will be wrong the day it merges. Verify with `ls scripts/seed.toml` (it should be absent) and `grep -rn "verify-worker" compiler | head`.
- **Exception creep.** CLAUDE.md already says that Rust-only landings are "outstanding debt, not precedent". The new paragraph must be unambiguous that this exempts the verifier and nothing else, or reviewers and agents will cite it to skip the Rust half of unrelated changes.
- **Contradicting existing text.** `docs/equivalence/README.md` and `.claude/commands/equivalence-review.md` treat the `c_emitter` pair as a compared pair. That stays true until P6, so leave them alone and have the ADR note that those docs get updated when the C emitters are deleted (P6), not here.
- **ADR filename.** Use the timestamped scheme, not `0004-`. Numbers 0001-0003 are frozen.
- **Docs-only CI path.** If CI classifies the PR as code (it should not), investigate `scripts/ci_docs_only.py` and do not edit it to compensate.

## 6. Out of scope

- The verification-semantics ADR (#1400) and the CLI/status-surface ADR (#1401); the ADR only cross-references them.
- Any Bitwuzla pin, `scripts/seed.toml`, bootstrap-script changes, `--backend native`, `vc_*.vow` modules, or deleting ESBMC/`c_emitter`.
- Updating `docs/equivalence/*`, the `equivalence-review` command, `docs/roadmap.md`, or `docs/spec/*`.
- Reformatting or tidying any other part of CLAUDE.md.
- Closing or editing issue #1398/#1335.
