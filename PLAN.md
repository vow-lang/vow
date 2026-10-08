# Plan: #1400 docs(adr): native verification semantics

## 1. Problem restated

Epic #1398 replaces ESBMC with a native in-Vow verifier (SSA IR -> SMT-LIB -> Bitwuzla). Decisions D5-D7 in
the epic fix the *semantics* that verifier implements, but they only exist as a table row in an issue body.
Before any `vc_*` module lands, the repo needs one accepted ADR that states each semantic rule, says what
changes in verdicts compared with today's ESBMC pipeline, lists the known limits, and records how
`docs/spec/contracts.md` will be rewritten (the rewrite itself is a later P3 child, "docs(spec): update
contracts.md ...", not this PR). This issue is documentation only: one new ADR file, no compiler, spec,
help-text or test changes.

## 2. Files to touch

| File | Change |
|---|---|
| `docs/adr/2026-10-08-HHMM-native-verification-semantics.md` (new) | The ADR. Name per `docs/adr/README.md`: UTC timestamp at authoring (`date -u +%Y-%m-%d-%H%M`), kebab slug. Reference as `ADR-YYYY-MM-DD-HHMM`. |

Deliberately **not** touched:
- `docs/spec/contracts.md` (and `cli.md`, `errors.md`, `grammar.md`): the ADR is not yet implemented, and the
  spec is "what the compiler does". Editing it now would make the spec lie about ESBMC. The rewrite is a
  planned section of the ADR and a P3 child issue.
- `docs/spec/index.md`, `--help`, embedded skill: unchanged, so `generate_help.py` /
  `check_help_coverage.py` need no rerun. (Run `check_help_coverage.py` anyway as a sanity check; it must be a no-op.)
- `compiler/`, Rust crates: the "dual compiler" rule does not apply to a docs-only change.
- `docs/design/verifier-model-bounds.md`: stays normative for the "bounds never leak into contracts" principle;
  the ADR cites it and does not supersede it.
- `CLAUDE.md`: the D1 exception (checker only in `compiler/`) belongs to the sibling D1 ADR issue, not this one.
  The ADR here only cross-references it.

Implementation-stage steps beyond the file: `git rm PLAN.md` before opening the PR; PR title
`docs(adr): native verification semantics (k-induction, unwinding, collections, floats)` — check length:
the subject plus ` (#N)` must be <= 100 chars, and `subject-case` needs lower-case. That title is 82 chars; OK.
Commit type `docs`, squash-merge only.

## 3. TDD slices

There is no code, so "red" is a reviewable checklist against the acceptance criteria, applied per slice.
Each slice is a small commit-sized unit of the ADR; do them in order and tick the check before moving on.

1. **Skeleton + context.** Header `Status: accepted (2026-10-08)`, Context (ESBMC today: `--incremental-bmc`
   with `--max-k-step 50`, `--no-bounds-check --no-pointer-check`, capacity models 128/256/64/64/1024 slots,
   `__int128`, `float`/`double` in C, `RemF*` not modelled, `proven` means "within the configured model"),
   links to epic #1398, #1335, `docs/design/verifier-model-bounds.md`, `docs/spec/contracts.md`.
   Check: every ESBMC fact quoted is verifiable against `vow-verify/src/esbmc.rs:433-436`,
   `vow-verify/src/c_emitter.rs` (`ir_ty_to_c`, `Opcode::RemF*` ~L926/993/2154), `contracts.md` L34-126.
2. **Rule 1 - Incremental BMC + k-induction.** Invariant-based first (havoc loop-modified vars, assume
   invariant, check preservation and loop exit), then automatic k-induction (base, forward, step). Internal
   bound; `--timeout` is the only user knob (`--max-k-step` removed; that removal is owned by the CLI-surface ADR,
   only cross-referenced). State the proof/refutation/unknown matrix. A step-case failure with no base-case
   failure is `unknown`, never `failed` (induction counterexamples may be unreachable).
   Verdict impact: ESBMC has no inductive step, so loops it leaves `unknown` at the unwind bound can become
   `proven`; a user invariant that is not inductive stays `unknown` with the same diagnostic class as today;
   nothing that ESBMC proves may become weaker (epic acceptance gate 1).
3. **Rule 2 - Mandatory unwinding assertion.** A loop deeper than the current bound yields
   `unknown`, never `proven`; the unwinding assertion is part of every BMC query and cannot be disabled.
   Verdict impact: documents ESBMC parity (incremental BMC already refuses to claim `proven` past
   `--max-k-step`) but closes the "proven describes the finite unwind" caveat in `contracts.md` L36 for
   everything except the explicit capacity-free model below. Note ESBMC's `--max-k-step 50` -> internal bound
   differences: fixtures that ESBMC proves at k<=50 must still prove; fixtures needing k>50 may now be proven by induction.
4. **Rule 3 - Vec/String = SMT array + symbolic length; no capacity caps; exact `string_eq`.** Specify the
   representation (`(Array (_ BitVec 64) elem)` + 64-bit length; index in-bounds is an explicit assertion, not a
   solver side effect), the null-pointer/`from_raw_parts_copy` rule carried over from `contracts.md` L74-95,
   `String::from_cstr` length is unconstrained (not 0..max-1), `string_eq` compares length and every byte
   (no length-prefix shortcut, no hashing). `HashMap`/`BTreeMap`/user-struct heap: state they follow the same
   no-cap rule where modelled and `Skipped` otherwise (fail closed). Verdict impact: `ModelCapacityAssumed`
   note disappears (add it to a "removed diagnostics" list for the later errors.md rewrite); properties that
   ESBMC proved only for len <= 128/256 now require an unbounded argument (induction/invariant) or become
   `unknown` — i.e. some ESBMC `proven` could degrade to `unknown`. This is a *soundness-honest* regression the
   ADR must flag explicitly against epic gate 1 ("never weaker"): state the reconciliation (gate applies to
   fixtures; each degraded fixture gets a written explanation or an invariant added to the *fixture*, never a
   capacity cap in a contract).
5. **Rule 4 - Fixed-width bitvectors for every integer width.** i8..i64 and u8..u64 as BV of that width; i128/u128
   scalars as BV128 (aggregates containing 128-bit fields remain gated by G8/D12). Signed/unsigned ops pick
   `bvs*`/`bvu*`; wrapping ops are modular; checked ops (`+!` etc.) assert no overflow and model the abort;
   shifts with count >= width follow the runtime's defined behaviour (read `vow-codegen` / grammar.md before
   writing — do not guess). Verdict impact: same as ESBMC's bit-precise `__int128` model for scalars; no
   change expected; `proven-ir` (unbounded-int mode) is dropped (D2/D8), so formerly `proven-ir` results become
   `proven` or `unknown` via BV.
6. **Rule 5 - IEEE-754 floats.** f32/f64 via SMT `FloatingPoint` (Float32/Float64); RNE rounding mode;
   comparisons are IEEE (NaN != NaN, `NaN < x` false) — contracts see the same semantics as runtime;
   int<->float casts and `parse_f64_bits`/`format_f64_bits` exact on the u64 side. Single canonical NaN
   (SMT-LIB FP has one NaN per format). Float `%` (`RemF32/RemF64`) stays `Skipped` with reason
   `float-rem-unsupported` until codegen defines it. Verdict impact: ESBMC today also `Skipped`s functions
   containing `RemF*` at the gate, while the emitter itself writes `v = 0` for it (`c_emitter.rs:2154-2156`,
   unreachable if the gate holds — verify before asserting this in the ADR; if reachable, flag it as an existing
   unsoundness the native design deliberately does not copy). NaN-sensitive contracts: ESBMC's `float`/`double`
   C model with its own NaN handling vs the native canonical NaN — list known divergences, expect none on
   non-NaN-payload properties.
7. **Known limits section.** Required by the acceptance criteria: (a) NaN payload/sign bits are not
   distinguishable (single canonical NaN), so `f64::to_bits` of a NaN and `bits -> f64 -> bits` round-trips
   for NaN payloads are not provable/refutable faithfully — fixtures must not rely on them; (b) string
   parse/format opacity: `parse_f64_bits`/`format_f64_bits` are exact on the u64 side only; the String side is
   an uninterpreted relation, so `format(parse(s)) == s` and decimal-text properties are `unknown`;
   (c) float `%` skipped; (d) recursion and effects `Skipped` (D10); (e) callee inlining (D4) so verification
   cost grows with call-tree size; (f) 128-bit aggregates gated; (g) solver `unknown` on timeout/OOM
   with a structured reason; (h) FP bit-blasting cost.
8. **Verdict-impact table.** One table, one row per rule: *Rule | ESBMC today | Native | Verdict direction
   (stronger / same / possibly weaker -> `unknown`) | Gate-1 handling*. This is the acceptance criterion
   "each rule states its verdict impact versus today's ESBMC behaviour"; the per-rule prose in slices 2-6 must
   agree with the table (check by reading both).
9. **Contracts.md rewrite plan.** A section listing, by existing heading, what happens: keep "Semantic
   Contracts Are Backend-Independent", Blame Model, Integer/Vec/String/HashMap contract pattern sections,
   Anti-Patterns; **replace** "Verification Pipeline" (diagram becomes IR -> SMT -> Bitwuzla), "ESBMC
   Configuration" (L34-39: BMC + k-induction, no `--max-k-step`), "Collection Models for Verification" (L41-126,
   capacity table and the `ModelCapacityAssumed` paragraphs deleted); **edit** "Unbound Loop Iterations"
   (L396-407: no longer "ESBMC may timeout at max-k-step 50"; becomes the unwinding-assertion/`unknown` story),
   "Non-Inductive Loop Invariant" (now actually checked by an inductive step), "Interpreting Counterexamples"
   (step-case failures are `unknown`, not a counterexample), "Counterexample Replay" (mandatory
   `--replay-cex`); **add** floats and known-limits sections. Name the follow-ups that own `errors.md`
   (`ModelCapacityAssumed` removal), `cli.md` (flag removal) and `grammar.md`/help regeneration, and state the
   timing: the rewrite lands with the P3 "docs(spec)" child, *after* native is default-capable, not before.
10. **Review pass.** Re-read for: (i) no weakening of the "contracts never encode verifier bounds" rule (the
    backend-independence test: native must not require editing any source contract — and the ADR itself
    must say k-induction invariants are *fixture/program* `invariant:` clauses with true semantics, never
    k-sized); (ii) every number/flag/line reference checked against the tree at the time of writing
    (line numbers drift; prefer symbol names in the ADR text); (iii) `subject-case`/length of PR title;
    (iv) run `scripts/check_help_coverage.py` and `pre-commit run --files <adr>` (markdown/whitespace hooks only).

## 4. Verification surface

None at runtime: no contracts, codegen, C-model, or fixtures change; no `tests/run/` or `examples/` growth.
The ADR *specifies* what later issues must prove:
- Soundness properties for later tests: unwinding assertion present on every BMC query; k-induction step
  failure never reported as `failed`; `string_eq` exact (differential vs runtime #1084); FP compare semantics
  match runtime for NaN/±0/inf; BV widths match runtime wrap/abort behaviour; float `%` -> `Skipped`.
- The ADR should name the fixtures that later children must add (float fixtures in P2, collection-length-unbounded
  fixtures in P3), without creating them here.

## 5. Risk areas

- **Internal contradiction with epic gate 1** ("never weaker than ESBMC"): dropping capacity caps and making
  the unwinding assertion mandatory can turn some ESBMC `proven` into native `unknown`. The ADR must say this
  openly and state the reconciliation (slice 4); do not paper over it.
- **Overclaiming ESBMC behaviour.** Verify each "today" claim in code before writing it (esp. the `RemF*`
  gate vs `v = 0` emission, and whether incremental BMC reports `unknown` vs `proven` past `--max-k-step`).
  If unverifiable cheaply, phrase as "as documented in contracts.md" with that citation.
- **Scope bleed** into the D1 ADR (CLAUDE.md exception) and the CLI/status-surface ADR (flag removals,
  `Skipped` reason names). Cross-reference only; if an overlapping decision is needed, state it once and point to
  the sibling.
- **Naming collision / lint:** ADR filename must follow the timestamp scheme, not `0004-`. `commitlint`
  `subject-case` (lower-case subject), header <= 100 incl. ` (#N)`.
- **Fixed point / idempotency / clippy:** not affected (docs only). Binary fixed point, `parse -> print -> parse`
  and `cargo clippy` gates are untouched; no need to run bootstrap or full_test.sh.
- **Spec truthfulness:** because `contracts.md` is not edited, no spec text may claim native behaviour yet;
  the ADR is the only place that does, and it is marked as describing the target, with a status line saying
  implementation is tracked by epic #1398.

## 6. Out of scope

- Rewriting `contracts.md`, `cli.md`, `errors.md`, `grammar.md`, regenerating help/skill (P3 docs child).
- The D1 CLAUDE.md-exception ADR and the CLI/status/`Skipped`-reason ADR (sibling P0 issues).
- Any `vc_*` module, `operations.json` `verifier_model` changes, drift-check changes, or ESBMC removal.
- Modular assume-guarantee verification (separate future ADR per D4), recursion/effect modelling.
- Defining float `%` codegen semantics; deciding NaN payload preservation.
- Formatting/cleanup of existing ADRs or `docs/design/verifier-model-bounds.md`.
- Follow-up list (file as issues only if not already children of #1398): add a "Superseded by ADR-..." note to
  `docs/design/verifier-model-bounds.md` once native is default; remove `ModelCapacityAssumed` from `errors.md`.
