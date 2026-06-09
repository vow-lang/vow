# Verifier model bounds are decoupled from the language

**Status:** Normative design specification. Implemented.

This document records the decision and rationale for issue
[#278](https://github.com/vow-lang/vow/issues/278): the removal of the
verify-only `--vec-max` / `--string-max` / `--hashmap-max` / `--btreemap-max`
capacity flags. It is the authoritative statement of how bounded-model-checker
limits relate to the Vow language. When this document and an implementation
disagree, this document is normative.

It fulfills roadmap workstream **WS-3.4** ("Stop leaking verifier bounds into
contracts") of `docs/roadmap-0.3.0-foundations.md`.

## The principle

**The language and its contracts are decoupled from what any particular prover
can prove.** A property a Vow program asserts (`requires`, `ensures`,
`invariant`) is a statement about the program, not about the tool that checks
it. The prover's reach — how much of the state space it can actually discharge —
is a separate concern that lives entirely inside the verifier.

The test for any verification mechanism is therefore:

> If we replaced the current prover with an infinitely powerful, unbounded model
> checker, would the source, the contracts, and the CLI keep working unchanged —
> the only difference being that we now prove the whole state space instead of a
> bounded slice?

If the answer is "no, something in the language would have to change," that
something is a leak and does not belong in the language. The bounded model
checker we use today (ESBMC) must be swappable for a stronger backend with **no
edits to any `.vow` source, any contract, or any user-facing CLI surface.**

## What was wrong

ESBMC is a *bounded* model checker. Its C model represents each collection as a
fixed-size array:

```c
typedef struct { int64_t len; int64_t data[VEC_MAX]; } __vow_vec_t;
__ESBMC_assert(v.len < VEC_MAX, "vec capacity");
```

so the model needs a finite capacity per collection type. We had exposed those
capacities as four CLI flags with per-collection defaults (`--vec-max 128`,
`--string-max 256`, `--hashmap-max 64`, `--btreemap-max 64`).

This leaked the verifier's implementation into the language surface:

- A user reading `--btreemap-max <N>` reasonably asks "why does my data
  structure have a maximum?" It does not. A `Vec`/`String`/`HashMap`/`BTreeMap`
  in a compiled Vow program grows dynamically on the heap with no fixed cap. The
  flag described ESBMC's model, not the language.
- It invited the exact anti-pattern `docs/spec/contracts.md` forbids: encoding a
  verifier bound (`requires: v.len() <= 128`) as if it were a semantic contract.
- It did not scale: every new collection type (BTreeSet, future trees) would
  grow yet another `--xxx-max` flag.

An unbounded backend would make all four flags meaningless — proof that they
were never language properties.

## The decision

1. **Remove all four flags** from both compilers, from every subcommand
   (`build`, `verify`, `test`, `contracts`, and the legacy bare form), and from
   all help / skill / capability output.
2. **Keep a single safe internal default per collection type**, passed to ESBMC
   by the verifier. These defaults are not user-tunable and are not part of the
   language. The current values are `Vec` 128, `String` 256, `HashMap` 64,
   `BTreeMap` 64 — chosen because they verify the entire reference + benchmark
   corpus and the self-hosted compiler.
3. **Keep automatic, invisible per-function widening** where the model *must*
   accommodate static content: a string literal longer than the default
   `String` capacity transparently raises that function's `String` model size
   (`limits_with_literal_string_capacity` in Rust, `string_max_for_literals` in
   the self-hosted compiler). This is the verifier adapting its own model to the
   program; it is never surfaced and never tunable.
4. **No escape hatch on the language side.** A program that genuinely cannot be
   proven within the model is a *prover* limitation, addressed by a stronger or
   unbounded backend — never by a language knob. See "Failure semantics" below.

## Why not the alternatives

The issue floated several directions. All were rejected in favor of the above
because they either relocated the leak or contradicted the principle:

- **Per-function annotation** (`vow { capacity: 100, ... }`) moves the verifier
  bound *into the contract block* — the worst possible place — and adds a new,
  non-predicate clause axis to the grammar. Directly contradicts WS-3.4.
- **Auto-size from contracts** (read `requires: m.len() <= 100`) rewards writing
  a verifier bound as a contract, the precise anti-pattern we forbid, and
  couples the contract text to the model's array sizing.
- **Construction-site bound** (`Vec::with_capacity(100)` doubles as the model
  bound) adds surface syntax to an intentionally small language, does not bound
  function-parameter collections (the common case has no construction site), and
  conflates runtime reservation with verification bounding.
- **Rename to `--verify-*`** only renames the smell; the per-collection-flag
  proliferation and the conceptual mismatch remain.
- **Lift to ESBMC heap/SMT arrays** is a large re-architecture of the collection
  model with high verifier cost and is out of scope for retiring the flags;
  under BMC, loops still unwind to a bound, so a bound does not actually
  disappear. It can be revisited independently as a prover-side improvement —
  and, by this document's principle, doing so would require **no** language
  change.

Crucially, an unbounded backend (or the ESBMC-SMT-array direction) can be
adopted later with zero changes to the language, contracts, or CLI. That is the
whole point of the decoupling.

## Where the bound lives

The bound is defined in exactly one place per compiler and threaded only through
internal verification code:

- **Rust:** `vow-verify/src/c_emitter.rs` — the `VEC_MODEL_CAP` /
  `STRING_MODEL_CAP` / `HASHMAP_MODEL_CAP` / `BTREEMAP_MODEL_CAP` constants and
  `VerifyLimits::default()`. The CLI (`vow/src/main.rs`) constructs
  `VerifyLimits` from `..VerifyLimits::default()` and only overrides
  `max_k_step` (a genuine, retained verification-performance knob).
- **Self-hosted:** `compiler/c_emitter.vow` — `VOW_VEC_MAX()` /
  `VOW_STRING_MAX()` / `VOW_HASHMAP_MAX()` / `VOW_BTREEMAP_MAX()`. The CLI
  (`compiler/main.vow`) sources the bounds from these constants instead of
  parsing flags.

`max_k_step` is **not** in scope for this decision. It is a verification-effort
control (how far incremental BMC unwinds), not a language-visible property of a
data type, and it remains a CLI flag.

## Failure semantics (follow-up)

Under this model, exceeding a collection's model capacity is a *verifier
limitation*, not a contract violation: it means "this prover could not reason
about this program within its model," not "this program is wrong." The honest
classification is therefore a verifier-model outcome (a `bug`/unprovable status
under roadmap §5.3 "No false `FAILED`"), distinct from a real contract `FAILED`
backed by a counterexample. Today the capacity assertions already carry
`blame = none` and no `vow_id`, so they are not attributed to any user contract.
Tightening the *reported status* for a model-capacity hit so it never reads as a
contract `FAILED` is tracked as follow-up work alongside #552; it does not
affect the flag removal, which is complete.

## Migration

The flags were verify-only and had no effect on compiled binaries, so removing
them changes no program's runtime behavior. Any script, CI step, or harness that
passed `--vec-max` / `--string-max` / `--hashmap-max` / `--btreemap-max` must
drop those arguments; the defaults they were almost always set to are now the
built-in behavior. There is no replacement flag — by design.
