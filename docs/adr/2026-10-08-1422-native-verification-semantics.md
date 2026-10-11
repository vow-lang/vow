# The native verifier proves with k-induction, mandatory unwinding assertions, unbounded collections, exact bitvectors and IEEE floats

**Status:** accepted (2026-10-08). Describes the *target* semantics of the native
verifier; implementation is tracked by epic #1398. Until it lands,
[`docs/spec/contracts.md`](../spec/contracts.md) keeps describing the ESBMC
pipeline, which is what the compiler does today.

## Context

Epic #1398 replaces the ESBMC pipeline (SSA IR to C to ESBMC) with a verifier
written in Vow (SSA IR to SMT-LIB to Bitwuzla). Its decisions D5-D7 fix what the
new verifier means by `proven`, but they only exist as a table row in an issue
body. This ADR records them, with the verdict change each one causes compared
with today.

Today's ESBMC behaviour, as implemented and documented:

- `esbmc_cli_args` in `vow-verify/src/esbmc.rs` runs ESBMC with
  `--incremental-bmc --max-k-step <N> --no-bounds-check --no-pointer-check --64`
  (`N` defaults to 50). Incremental BMC is base case plus forward condition;
  there is **no inductive step** (`contracts.md`, "ESBMC Configuration").
- `proven` means "verified within the configured model, including its finite
  unwind and collection capacities" (same section).
- Collections are fixed-size arrays: `Vec` 128, `String` 256 (raised to the
  longest literal), `HashMap`/`BTreeMap` 64, user-struct heap 1024 slots.
  A non-constant length is nondeterministic but pruned by
  `__ESBMC_assume(len <= CAP)`, and the result carries a `ModelCapacityAssumed`
  note (`contracts.md`, "Collection Models for Verification").
- Integers are emitted as C fixed-width types, `__int128` for 128-bit scalars.
  `proven-ir` is a fallback that re-checks under unbounded integer arithmetic
  after the bitvector run timed out.
- Floats are C `float`/`double`. `RemF32`/`RemF64` are rejected by the
  modellability gate in `c_emitter.rs` (the function is `Skipped`); the
  `v = 0` the emitter writes for them is unreachable behind that gate.

The semantic contract rule is unchanged and stays normative: contracts state
what is true of the program and never encode a verifier bound
(`docs/design/verifier-model-bounds.md`, `contracts.md` "Semantic Contracts Are
Backend-Independent"). Everything below is a property of the verifier, not of
the language. No source contract may need editing to move from ESBMC to the
native verifier.

## Decision

### Verdict vocabulary

The native verifier returns one of `proven`, `failed` (a concrete
counterexample, replayable), or `unknown` (with a structured reason). A result
is `proven` only when every query that supports it is discharged. Nothing
unexplained is ever reported `proven`.

### Rule 1: incremental BMC plus k-induction

Verification of a function with loops runs incremental BMC with an internal
depth bound and, on every iteration, an induction attempt. Two inductive
strategies apply, in this order:

1. **Invariant-based.** For a loop with a user `invariant:` clause: check the
   invariant on entry; havoc every variable the loop modifies, assume the
   invariant and the loop condition, execute one iteration, and check the
   invariant is preserved; on exit, assume the invariant and the negated
   condition and check the postcondition. This is sound without any bound.
2. **Automatic k-induction** for loops with no (or a non-inductive) invariant:
   base case (no violation within `k` iterations from the initial state),
   forward condition (the loop cannot run `k` iterations, so `k` covers every
   execution), and step case (`k` consecutive iterations from an arbitrary
   state satisfying the property imply the next one satisfies it).

Outcome matrix:

| Base case | Forward / step | Verdict |
|---|---|---|
| violation found | n/a | `failed` (counterexample from a reachable state) |
| no violation | forward condition holds | `proven` |
| no violation | step case holds | `proven` |
| no violation | neither holds within the internal bound | `unknown` |

A **step-case failure with no base-case failure is `unknown`, never `failed`**:
the step case starts from an arbitrary state, so its counterexample may be
unreachable. Only base-case counterexamples are reported as failures.

The user-visible knob is `--timeout`. The internal bound replaces
`--max-k-step`; the flag removal belongs to the CLI-surface ADR and is only
cross-referenced here. Invariants stay program facts with true semantics; they
are never sized to a `k`.

*Verdict impact versus ESBMC:* ESBMC has no inductive step, so a loop that it
leaves `unknown` at its unwind bound can become `proven` (stronger). A user
invariant that is not inductive stays `unknown` with the same diagnostic class
as today (same). Nothing ESBMC proves becomes weaker through this rule.

### Rule 2: the unwinding assertion is mandatory

Every BMC query includes the unwinding assertion: the claim that no execution
runs a loop more times than the current bound. If the assertion can fail and no
induction argument closes the gap, the verdict is `unknown`, never `proven`.
The assertion is part of the query construction and cannot be disabled by a
flag or configuration.

*Verdict impact:* `contracts.md` already states that a contract is `proven`
only when ESBMC completes the configured model checks, and `unknown` otherwise,
so verdicts on loops deeper than the bound are expected to match today's. What
changes is the meaning of `proven`: the caveat "`proven` describes the finite
unwind" in `contracts.md` disappears, because a proof now covers all iteration
counts (by the forward condition or by induction). Fixtures ESBMC proves at
`k <= 50` must still prove; fixtures needing more than 50 iterations may now be
`proven` by induction (stronger). The corpus-wide comparison is the epic's
verdict-parity gate, not something this ADR asserts.

### Rule 3: `Vec` and `String` are an SMT array plus a symbolic length

A `Vec<T>` or `String` is modelled as `(Array (_ BitVec 64) elem)` plus a
64-bit symbolic length. There are **no capacity caps**: no `len <= 128`, no
`len <= 256`, no heap-slot budget, and so no `ModelCapacityAssumed` note.

- An index in bounds is an explicit assertion (`i < len`), not a side effect of
  array size. A `push` grows the length symbolically.
- `from_raw_parts_copy` keeps the runtime's null-pointer rule: a null source
  yields an empty value whatever the length; `ensures: result.len() == n`
  needs a real `requires: p != 0`.
- Bytes that are not statically known (`String::from_cstr`) have an
  unconstrained length, not `0..max-1`.
- `string_eq` is exact: equal lengths and every byte equal. No length-prefix
  shortcut, no hashing, no uninterpreted function.
- `HashMap`/`BTreeMap` and user-struct heap follow the same no-cap rule where
  the native model covers them. Anything not modelled is `Skipped` (fail
  closed), never approximated.

*Verdict impact:* `ModelCapacityAssumed` is removed (record it for the later
`errors.md` rewrite). Properties ESBMC proved only for lengths up to the
capacity now need an unbounded argument (an invariant, induction) or become
`unknown`, so **some ESBMC `proven` results may degrade to `unknown`**. This is
an accuracy gain: those proofs only held for short collections. It is also an
exception to the epic's gate "never weaker than ESBMC", which is reconciled as
follows: the gate is measured on the fixture corpus; each fixture that degrades
gets a written explanation, or a true-semantics `invariant:` added to the
*fixture*. A capacity cap or length bound in a contract is never the fix.
Properties that need `len <= CAP` to hold were never proven for the runtime.

### Rule 4: fixed-width bitvectors for every integer width

`i8`..`i64` and `u8`..`u64` are `(_ BitVec w)` of their own width; `i128` and
`u128` scalars are `BitVec 128`. Signed and unsigned operations pick `bvs*` or
`bvu*`. Wrapping operators are modular. Checked operators (`+!`, `-!`, `*!`,
`/!`, `%!`) assert the absence of overflow and model the abort, as the
runtime does. Shift counts follow the language's rules in `grammar.md` (count
type `u32`, compile-time and dynamic range checks). Aggregates with 128-bit
fields stay gated (`Skipped`) as they are today. *Addendum (#1421):* a 128-bit
struct field or enum payload is one `BitVec 128` term; the gate is lifted.

The unbounded-integer fallback is dropped: there is no `proven-ir` result.
A function ESBMC only proved under integer arithmetic after the bitvector run
timed out is now `proven` (the bitvector query finished) or `unknown`.

*Verdict impact:* same as ESBMC's bit-precise model for scalars; no change is
expected. `proven-ir` results become `proven` or `unknown`; none stays
approximate.

### Rule 5: IEEE-754 floats

`f32` and `f64` use the SMT-LIB `FloatingPoint` theory (`Float32`, `Float64`)
with round-to-nearest-even. Comparisons are IEEE: `NaN != NaN`, `NaN < x` is
false, `-0.0 == 0.0`. Integer/float casts follow the runtime. Bit-level
conversions (`parse_f64_bits`, `format_f64_bits`) are exact on their `u64` side.

SMT-LIB has a single NaN per format, so the verifier has a **single canonical
NaN**. Float remainder (`RemF32`, `RemF64`) stays `Skipped` with reason
`float-rem-unsupported` until codegen defines the operation; the verifier does
not invent semantics ahead of the runtime.

*Verdict impact:* ESBMC also skips `RemF*`, so the skip set is unchanged. Float
properties that do not depend on NaN payload or sign bits are expected to keep
their verdicts. Properties that distinguish NaN bit patterns are listed under
known limits.

## Verdict impact summary

| Rule | ESBMC today | Native | Direction | Gate handling |
|---|---|---|---|---|
| 1. Induction | BMC only, no inductive step | invariant-based, then k-induction | stronger | none needed |
| 2. Unwinding | `unknown` past `--max-k-step`; `proven` is "within unwind" | assertion mandatory; `proven` covers all iterations | same on verdicts, stronger meaning | fixtures needing `k > 50` may newly prove |
| 3. Collections | caps 128/256/64/64/1024; `ModelCapacityAssumed` | array plus symbolic length, no caps, exact `string_eq` | possibly weaker (`unknown`), more honest | per-fixture explanation or fixture invariant; never a contract cap |
| 4. Integers | C fixed-width, `__int128`, `proven-ir` fallback | `BitVec w`; `proven-ir` dropped | same | none needed |
| 5. Floats | C `float`/`double`; `RemF*` skipped | `FloatingPoint`, RNE, canonical NaN; `RemF*` skipped | same | NaN-payload fixtures excluded |

## Known limits

- **NaN payload and sign.** Only one canonical NaN exists. `f64::to_bits` of a
  NaN, and `bits -> f64 -> bits` round trips for NaN payloads, are neither
  provable nor refutable faithfully. Fixtures must not rely on them.
- **String parse/format opacity.** `parse_f64_bits` and `format_f64_bits` are
  exact on the `u64` side only; the `String` side is an uninterpreted relation.
  `format(parse(s)) == s` and decimal-text properties are `unknown`.
- **Float `%`** is `Skipped` until codegen defines it.
- **Recursion and effects** are `Skipped`, except `panic`: the abort of
  `.unwrap()` on `None`/`Err` is a verification claim, reported unattributed
  (vow id `4294967293`, blame `none`) like the division aborts (#1417).
- **Aggregates** (structs, enums, `Option`, `Result`) are scalarised: one `i64`
  or `Bool` term per field, merged field by field at joins. Parameter fields and
  discriminants, and fields no construction wrote, are unconstrained. A field
  write is modelled only on a value built in the same block; any other write,
  and any aggregate nested in another, is `Skipped` (#1417).
- **Callee inlining** (epic D4) makes verification cost grow with the call tree;
  modular assume-guarantee verification is a separate future ADR.
- **128-bit aggregates** (struct fields) stay gated. *Addendum (#1421):* struct
  fields and enum payloads of 128 bits are modelled, one 128-bit term each;
  `Vec<i128>` elements stay `Skipped` until the collection model (#1423).
- **Solver limits.** Timeout or memory exhaustion yields `unknown` with a
  structured reason, never `proven`.
- **Floating-point cost.** FP queries bit-blast and can be slow; they time out
  to `unknown`.

## Contracts.md rewrite plan

The rewrite lands with the P3 child "docs(spec): update contracts.md" after the
native verifier can be the default, not before: the spec must not describe
behaviour the compiler does not have. By existing heading:

- **Keep:** "Semantic Contracts Are Backend-Independent", the blame model,
  the Integer/Vec/String/HashMap contract patterns, the anti-patterns.
- **Replace:** "Verification Pipeline" (IR to SMT-LIB to Bitwuzla), "ESBMC
  Configuration" (BMC plus k-induction, no `--max-k-step`), "Collection Models
  for Verification" (capacity table and `ModelCapacityAssumed` paragraphs
  deleted).
- **Edit:** "Unbound Loop Iterations" (the unwinding-assertion and `unknown`
  story), "Non-Inductive Loop Invariant" (now checked by an inductive step),
  "Interpreting Counterexamples" (step-case failures are `unknown`),
  "Counterexample Replay".
- **Add:** a floats section and a known-limits section.

Follow-ups that own the other spec files: `errors.md` (remove
`ModelCapacityAssumed`), `cli.md` (flag and `proven-ir` removal), `grammar.md`
and the regenerated `--help`/skill. `docs/design/verifier-model-bounds.md` gains
a "superseded by" note once native is the default; its principle is not
weakened by this ADR.

## Consequences

- Later children implement and test these rules: an unwinding assertion in every
  BMC query; a step-case failure is never `failed`; `string_eq` differential
  against the runtime; IEEE comparison semantics for NaN, ±0 and infinity;
  bitvector widths match runtime wrap and abort; `RemF*` is `Skipped`. Float
  fixtures belong to P2, unbounded-length collection fixtures to P3.
- The D1 exception to the checker-only-in-`compiler/` rule and the CLI/status
  surface (flags, `Skipped` reason names) are decided in sibling ADRs and not
  repeated here.
- This ADR does not change language syntax, types, builtins, effects or CLI
  flags, so no `docs/spec` or help regeneration accompanies it.

## Addendum (#1418): bounded loop unwinding

Rule 2 is implemented for loops by unrolling the reducible natural loops of a
function into an acyclic one before symbolic execution.

- The bound counts back edges. A back edge past the bound goes to one sink block
  whose reachability is the unwinding assertion: "no run takes more back edges
  than the bound". It is a claim only and assumes nothing afterwards.
- The internal schedule is 2, 4, 8, 16, 32, 64; the largest is at least ESBMC's
  default `--max-k-step`. It is not a flag. All rounds share the function's
  `--timeout`.
- A failed hard claim inside the unrolled prefix is a real run and is `failed`
  at once, whatever the unwinding claim says. An unwinding claim that is still
  `sat` at the largest bound is `unknown`, never `proven`, unless invariant
  induction closes the loops (addendum #1422).
- `invariant` is a claim checked on every header visit and is not assumed by
  bounded unwinding. Its inductive use is the addendum (#1422) below; automatic
  k-induction for loops with no usable invariant is the addendum (#1425).
- Soft arithmetic sites are collected per source site from the round that proves
  the function, so a loop body does not repeat its warning once per iteration.

## Addendum (#1416): inlined calls with blame and vow-id mapping

The "callee inlining" decision is implemented in `compiler/vc_inline.vow`. The
call graph below a verify target is flattened before symbolic execution; the
executor still sees one acyclic function.

- A call of a user function with integer, `Bool` or `Unit` arguments and result
  is replaced by a copy of the callee: the call's block ends with a jump into
  the copy, each `Return` becomes an Upsilon into the call's Phi (a Unit
  result has a constant instead) and a jump to the continuation block. Every
  copy gets fresh value and block ids, so a callee spliced twice shares
  nothing. Aggregates do not cross a call yet; such a call is `Skipped` as
  `unsupported-opcode`. Callees may build and use aggregates internally.
- Clauses and abort-bearing instructions of a copy carry the frame they came
  from in `IrInst.ds`. Inside a frame a `requires` is a claim blamed on the
  caller, an `ensures` or loop `invariant` a claim blamed on the callee, and
  both are assumed afterwards. The `vow_id` stays the callee-local id; the
  result's `callee_precondition_*` / `callee_postcondition_*` carry the callee's
  function id, which is what `build_ce_from_result` already consumes. A
  callee's `requires` precedes its body, so a caller's violation is reported
  before an `ensures` that would only fail because of it.
- A function without a `vow` block that calls a contracted function is the
  caller-precondition role: only callee `requires` (and the unwinding claim) are
  obligations, every other claim is assumed, as `caller_preconditions_only_source`
  does for ESBMC.
- Recursion, direct or through other functions, is `Skipped` as
  `recursion-unsupported` with the cycle as detail (`a -> b -> a`); a target
  that merely reaches a recursive function gets the same code. A callee outside
  the subset is `non-modelable-callee: <callee> (<code>)`. The gate vets the
  flattened function, not the parts: splitting a block at a call can change the
  aggregate and control-flow shape the gate sees.
- The inlined instruction count is computed from a memoised call-tree walk
  before anything is built. A tree above 200000 instructions (the unroller's
  bound) is `unknown` with a reason that starts `inlining budget:`, never
  `Skipped` (the code list is closed) and never `proven`.
- A counterexample for a claim inside a callee carries the chain of calls that
  leads to it in `call_sites`, outermost first (function the call is made in
  and the call's source range). Argument values at depth 1 are recovered as
  before; deeper `violating_args[].value` stay empty. Each call site's `file` is
  the source file of the function the call is made in. `call_sites` is filled for
  callee-blame counterexamples too, which the schema description ("caller-blame
  failures") does not say yet; the wording is the spec rewrite's to widen.
- The worker's result wire tag is `VOWRES2`: the failing claim's call chain
  follows the arithmetic sites.

## Addendum (#1422): invariant-based induction

The first strategy of Rule 1 is implemented in `compiler/vc_induct.vow`. The
implementation is an IR-to-IR cut, so the symbolic executor still walks one
acyclic function.

- **Obligations.** For every loop the cut yields three pieces. The *entry copy*
  is the loop's invariant region (the header's Phis, the clause code and the
  last `VowInv`) on the state the loop is entered with: the invariants are
  claims there. The *header proper* is the original loop with each header Phi
  replaced by an unconstrained constant of its type and each invariant of the
  region by an assumption; its condition, body and exit are untouched, so the
  claims of the body and of everything after the loop (including `ensures`) are
  decided from an arbitrary state that satisfies the invariant. The *back copy*
  is the region again, fed by the back edges: the invariants are claims there
  (preservation), and it ends in a marker that carries no claim. A `&&`/`||` in
  a clause lowers to branches, so the region may span several blocks; the cut
  copies the blocks that lead to the last `VowInv` and stops right after it.
- **Eligibility.** Every loop of the function must have an `invariant`, no
  header Phi may be a pointer, and every `invariant` must lie in its loop's
  region. Otherwise the cut does not apply and bounded unwinding decides alone,
  as before. A loop without an invariant is never cut with an implicit `true`.
- **Verdicts.** Induction only ever adds `proven`. A query that is not `unsat`
  is "not proved inductively", never `failed` (Rule 1: a step-case failure is
  `unknown`), because its counterexample may be a state no run reaches. A wrong
  invariant is still `failed`: bounded unwinding checks the invariant on every
  header visit and finds a real run. An invariant that is true but not
  inductive (too weak to give the postcondition, not preserved from an
  unreachable state), or violated only past the largest bound, is `unknown`.
- **Order.** The inductive attempt always runs first and costs one query set.
  When it proves a function that has no checked-arithmetic claim, the function is
  `proven` without bounded unwinding, so an unbounded or nested loop is cheap.
  When the function has such a claim, the bounded rounds also run, both to find
  the abort sites and to supply the verdict: every proof, counterexample and
  `ArithOverflowReachable` set they produce is unchanged, and the induction
  proof is used only where they end `unknown`: open at the largest bound, over
  the size budget, or undecided (solver unknown, timeout). A bounded
  counterexample or a solver error is never overturned. This ordering cannot
  change a verdict a bounded round reaches: an inductive proof holds for every
  finite prefix, so it cannot coexist with a real counterexample. The attempt
  shares `--timeout` with the rounds, so a bounded proof that finished just
  under the budget can now run out of it.
- **Warnings.** An inductive proof reports the abort sites of the last
  completed bounded round, which are the ones reachable within the unrolled
  prefix, plus any the round that ended undecided found before it ended. A
  checked-arithmetic claim `sat` in the induction query is ignored: the
  havoc state may be unreachable. A function proven only inductively can
  therefore have reachable aborts that no warning names.
- **Partial correctness.** A proof covers terminating runs: the exit obligation
  is "from the invariant and the negated condition", so a loop that never exits
  satisfies any `ensures` vacuously (`while true vow { invariant: true }` is
  `proven`; bounded unwinding alone says `unknown` for it, and so does ESBMC).
  This is a deliberate verdict change, pinned by
  `tests/verify-native/pass/loop_nonterminating_partial_correctness.vow`.
- **Inlined callees.** A callee's loop is cut like the target's. Its invariants
  are claims blamed on the callee in the entry and back copies and a plain
  assumption in the havoc copy (the frame marker is cleared there).
- **Verdict impact.** Stronger, as Rule 1 and Rule 2 allow: a loop that bounded
  unwinding leaves `unknown` (a parameter bound, a literal bound above 64) is
  `proven` given an inductive invariant. `tests/verify-native/unknown/loop_literal_100.vow`
  carried an invariant and was `unknown`; it is `pass/loop_literal_100_invariant.vow`
  now, and the `unknown/` fixture kept the invariant-free loop until the
  addendum (#1425) below proved it too. No ESBMC `proven` becomes weaker.

## Addendum (#1425): automatic k-induction for loops with no usable invariant

Rule 1's second strategy is implemented in the unroller (`vc_unroll_step`), the
executor (hypothesis blocks) and the driver (`vc_step_attempt`). No new flag,
no C-emitter or ESBMC change; it exists only in the self-hosted verifier.

- **Step function.** After a bounded round at bound `k` ends open (base case
  `unsat` for all hard claims, unwinding claim `sat`), the same function is
  unrolled to `k` back edges from an arbitrary header state: each header Phi of
  copy 0 is a havoc constant and the entry Upsilons into it are dropped; every
  iteration copy below `k` is a hypothesis block, whose claims (guards, callee
  clauses, the target's `invariant`) are assumed instead of asked; every edge
  out of the loop from such a copy is cut, so only runs that took the back edge
  continue; copy `k` asks its claims, then assumes its `invariant`, and its
  back edge ends in a marker that claims and assumes nothing. Values defined
  before the loop stay real, which is stronger than a full havoc and still
  sound.
- **Soundness.** The bounded round supplies the base case (copies `0..k` are
  claim-checked) and the step proves that `k` consecutive iterations that hold
  the property are followed by one that does, so it holds at every iteration of
  every run. The loop condition of the earlier copies is an assumption, so a
  plain counting loop closes at `k = 2` with no invariant at all. The
  postcondition is a goal at the exit of copy `k`, never a hypothesis.
- **Outcome.** All claims of the step `unsat` is `proven`, used exactly as a
  1-induction proof is (addendum #1422): a function with a checked-arithmetic
  claim keeps running the bounded rounds for its warnings and uses the proof
  where they end `unknown`. Anything else says nothing: a counterexample to the
  step may be a state no run reaches, so it is never `failed` and the bounded
  schedule goes on. The step runs only after a bounded round that did not end
  and was open, and only when the 1-induction of a user invariant did not
  already prove the function, so it never changes a counterexample or a proof
  a bounded round reaches. It shares `--timeout` with the rounds.
- **Scope.** A function whose loop-closed form has exactly one loop (hence no
  nesting), no pointer carried by its header Phi and no aggregate field written
  in the loop. A written aggregate would keep its entry state in copy 0 while
  the scalars are havoc'd, which is not an arbitrary state; such a function and
  a function with several loops keep their bounded and #1422 verdicts. The
  step is tried at every bound of the schedule (2 to 64), the first one that
  closes ends the check.
- **Partial correctness.** As for invariant induction, a loop that never exits
  has no run that reaches the `ensures`: `while true { ... }` with no invariant is
  `proven` (`tests/verify-native/pass/loop_infinite_partial_correctness.vow`;
  ESBMC's k-induction also reports success there).
- **Verdict impact.** Stronger, as Rule 1 and Rule 2 allow. Moved from
  `unknown/` to `pass/`: `loop_symbolic_bound`, `loop_literal_100`,
  `loop_invariant_k_inductive` (was `loop_invariant_too_weak`: the invariant is
  not enough for the 1-induction, the step case at `k = 2` closes it) and
  `loop_infinite_partial_correctness`. Still `unknown/`: `loop_bug_beyond_bound`,
  `loop_kinduction_not_inductive` and `loop_kinduction_break_not_inductive`,
  where the step has a model for every `k`. New `pass/` fixtures that only the
  step proves: `loop_kinduction_alternating_divisor`, `_callee_requires`,
  `_invariant_not_one_inductive` and `_break`.
