# 0002. Restart postconditions are path-local guarantees

**Status:** accepted (2026-07-31)

## Context

The verified condition/restart proposal in
[`docs/live-programming-research.md`](../live-programming-research.md#1-verified-conditionrestart-system)
allows a callee-defined restart to promise less than the function's normal
`ensures` clauses. That is useful when the violated precondition makes normal
success impossible, but it creates a soundness boundary at the caller. A
caller must not receive the normal postcondition after selecting a recovery
that has not established it.

Condition/restart syntax and runtime support are not implemented yet. This ADR
fixes the caller-side semantics that a future implementation must preserve; it
does not select the remaining surface syntax, dispatch ABI, or continuation
representation.

## Terminology

For a function `f`:

- `P` is the conjunction of `f`'s `requires` clauses.
- `H'_f` is the heap when `f`'s ordinary body completes.
- `Q(H'_f, args, result)` is the conjunction of `f`'s normal `ensures`
  clauses.
- `W_f` is a sound over-approximation of the mutable heap locations that the
  ordinary body may write.
- `H_r` is the live heap immediately before restart `r` is invoked, after the
  selected handler arm has run. It includes mutations made through aliases of
  heap-backed call arguments.
- `H'_r` is the heap when that restart completes.
- `W_r` is a sound over-approximation of the mutable heap locations that
  restart `r` may write. Writes through any alias name the same locations.
- `A_r(H_r, args, restart_args)` is the conjunction of restart `r`'s argument
  preconditions. These may constrain live call arguments as well as selected
  restart arguments; `A_r` is `true` when the restart declares none.
- `r` is a declared restart and
  `R_r(H'_r, args, restart_args, result)` is its restart-specific
  postcondition.
- A **normal edge** is completion of `f`'s ordinary body after `P` holds.
- A **recovery edge** is completion through a restart selected by a `handle`
  site after a condition is raised.
- A restart is **selectable** at a handle site when that site's control flow can
  choose it. Merely declaring a restart does not add its guarantee to every
  caller.

A restart-specific postcondition may be weaker than, stronger than, or
incomparable with `Q`. If a restart omits a specific postcondition, its
postcondition defaults to `Q`.

## Decision

Restart postconditions are verifier facts attached to individual recovery
edges. They are not refinements of the function's return type and they are not
silently promoted to the function's normal postcondition.

### Callee obligations

The verifier checks every declared restart independently. Before checking a
restart, it must forget facts about mutable memory reachable through
heap-backed call and restart arguments and model that memory as arbitrary. This
is the interference that a handler can cause through Vow's shared-pointer
passing semantics. Under `A_r` evaluated in that arbitrary invocation state,
the restart expression must establish its declared `R_r`. In proof notation,
the obligation is universal over every `H_r` that satisfies `A_r`, not just the
heap at the point where the original condition was raised.

Callee verification must also derive `W_f` from the ordinary body and `W_r`
from each restart implementation. The actual writes on every execution must be
a subset of the corresponding footprint. A compiler may export a more precise
footprint, but when it cannot prove one, `W_f` must conservatively include all
mutable memory transitively reachable through heap-backed call arguments and
`W_r` must do the same for heap-backed call and restart arguments. Both also
include any other state that outcome is permitted to write. Under-approximating
either footprint is unsound.

This is a callee obligation, just like an ordinary `ensures` clause. A restart
may rely on immutable value arguments and on mutable facts explicitly
re-established by `A_r`; it may not rely on a failed-condition fact about
aliased memory unless `A_r` states that fact. Proving `R_r` under `A_r` does
not make `R_r` available for restart arguments or invocation states that
violate `A_r`.

Evaluating `P`, `Q`, `A_r`, and `R_r` must be observationally read-only. In
addition to the existing contract rule that rejects declared effects, the
condition/restart implementation must derive the transitive heap-write
footprint of every called helper and reject a clause that can write
pre-existing shared state. A helper may mutate private fresh storage only when
that storage cannot escape the helper or alias a contract binding. Thus an
effect-free helper that assigns through a struct argument is not valid in any
contract participating in these outcome summaries, while a helper that only
reads that struct is. This uses the same footprint analysis required for
outcome summaries; it does not add a new effect or type-system axis. Enforcing
this stronger meaning of contract purity for the existing `P` and `Q` clauses
is a prerequisite of condition/restart implementation, not a change to current
compiler behavior made by this documentation-only ADR.

The function's ordinary body is checked against `Q`, and its writes are checked
against `W_f`. A weaker `R_r` does not weaken those checks, and proving the
ordinary body does not prove the restart.

### Facts exposed at a handle site

A handled call lowers to explicit verifier control-flow edges:

1. Modular caller lowering constructs `H'_f` on the normal edge by havocing
   every location in the imported `W_f`, with the same alias-aware rule used
   for restart writes. It then carries
   `assume(Q(H'_f, actual_args, call_result))`; observational evaluation of
   `Q` does not add writes to `W_f` or change `H'_f`.
2. The selected handler arm runs, including any mutations it makes through
   aliases of the handled call's heap-backed arguments.
3. Immediately before invoking restart `r`, lowering creates the caller
   obligation
   `assert(A_r(H_r, actual_args, selected_restart_args))` in the resulting live
   heap state. Its observationally read-only evaluation leaves `H_r`
   unchanged.
4. Only after that obligation succeeds does the restart execute from `H_r`.
   Modular caller lowering constructs `H'_r` by havocing every location in the
   imported `W_r`; all aliases observe the same havoc, while facts about
   locations outside `W_r` remain available. Its recovery edge then carries
   `assume(R_r(H'_r, actual_args, selected_restart_args, call_result))` in the
   restart's completion state.
5. Handler code and all downstream proof obligations are checked on every
   reachable edge.

For a parameterized restart, the postcondition is instantiated with the actual
arguments supplied by the handler, after those arguments have satisfied the
restart's parameter contract. This lets a handler prove a stronger fact for a
particular valid selection even when the restart's general contract is weak.
The proof and invocation are one verifier transition: there is no stale
snapshot or unmodeled mutation window between checking `A_r`, applying the
restart's write footprint, and assuming `R_r`.

`args` in a restart contract denote the callee's live argument values, using
ordinary Vow passing semantics. Primitive arguments remain values. A
heap-backed argument remains the same shared reference, and its fields reflect
the invocation/completion heap appropriate to the clause. The verifier does
not silently substitute a deep call-time snapshot. A future explicit `old` or
snapshot feature could add such a relation, but restart contracts do not
introduce one implicitly.

The facts remain branch-sensitive in the verifier's ordinary CFG/SSA model. At
a join, their logical summary is:

```text
(normal_path && Q)
|| (restart_r_path && R_r)
|| (restart_s_path && R_s)
|| ...
```

The implementation must not replace this summary with an unconditional
`assume(Q)`. It also must not eagerly erase the path predicates into a single
coarse return type. Existing branch reasoning is sufficient; no
restart-refined type or new type-system axis is introduced.

An unhandled call keeps today's rule: its caller must prove `P` without
changing the heap, apply `W_f`, and then use `Q` on the normal edge. Declaring
a restart does not weaken ordinary calls.

### Handler and enclosing-function contracts

Contracts are never weakened automatically. A function containing a handled
call must establish its own declared `ensures` clauses over the normal edge and
every reachable recovery edge.

The author may declare a genuinely weaker enclosing postcondition when that is
the function's intended semantic contract, but consuming a weak restart does
not require such a declaration. The handler may instead re-establish the
stronger property, choose restart arguments that imply it, or prove that the
weak recovery edge is unreachable.

Consequently, a caller that relies on `Q` after selecting a restart that only
guarantees `R_r` is rejected whenever `R_r` and the handler's code cannot
discharge that reliance. Rejection is use-sensitive: selecting a weak restart
is not itself an error if every actual downstream obligation follows from its
guarantee.

## Worked proof shape

Suppose a function's normal outcome guarantees `result > 0`, while a restart
named `use_zero` guarantees only `result == 0`.

- A handler whose enclosing function promises `result >= 0` verifies: the
  normal edge proves it from `result > 0`, and the recovery edge proves it from
  `result == 0`.
- A handler that returns the handled result while promising `result > 0` fails:
  `use_zero` supplies a counterexample on the recovery edge.
- A handler that can prove the `use_zero` edge unreachable may retain
  `result > 0`; an unreachable recovery does not pollute the join.

This is path reasoning, not subtyping. The handled expression continues to
have the function's ordinary base return type.

### Aliased-state example

Suppose a failed precondition establishes `p.x <= 0` at the condition site and
`p` is a heap-backed argument. A `use_zero` restart returns `0` and promises
`result >= p.x`. The caller retains an alias to `p` and its handler can assign
`p.x = 1` before invoking `use_zero`.

The callee may not prove `use_zero` from the stale failure-state fact. With no
restart argument contract, havocing the reachable heap makes
`result >= p.x` unprovable, so the restart declaration is rejected as a
callee postcondition failure. If `A_use_zero` instead requires `p.x <= 0`, the
restart verifies for every invocation heap satisfying that requirement, while
the mutating handler fails with caller blame when it tries to select the
restart at `p.x == 1`. Restoring `p.x <= 0` before selection makes the recovery
sound.

### Restart-write example

Suppose the handler enters a restart while `p.x == 0`, the restart may assign
`p.x = 1`, and its only postcondition is `result == 0`. The handler retains an
alias to `p`. Interface metadata that exports only the postcondition would let
modular caller verification retain the stale fact `p.x == 0` after recovery.

Instead, `W_r` includes `p.x` (or conservatively the mutable object reachable
through `p`). Caller lowering havocs that shared location before assuming
`result == 0`, so neither alias retains the stale field value. If the restart
also promises `p.x == 1`, `R_r` re-establishes that fact in `H'_r`. A location
outside the sound footprint keeps its prior facts.

## Verification boundary

Caller verification remains modular. Compiled interface metadata for a
restart-capable function must include `Q`, sound `W_f`, every advertised `R_r`
and sound `W_r`, restart parameter contracts, and the source identities needed
for diagnostics. A caller imports those summaries and does not inline the
callee's body or restart expressions.

The soundness chain is:

1. Callee verification proves the normal body establishes `Q` with writes
   covered by `W_f`, each restart's actual writes are covered by `W_r`, and
   each restart establishes its own `R_r` for every invocation heap satisfying
   `A_r`.
2. Handle-site lowering applies `W_f` before exposing `Q` on normal completion.
   On recovery, it runs the handler, proves the selected restart's argument
   contract in the resulting live heap, applies the selected restart's `W_r`,
   then exposes `R_r`.
3. Caller verification proves downstream obligations independently on all
   reachable edges.

No step permits `Q` to be assumed on a recovery edge unless that edge's facts
independently imply `Q`. No step permits `R_r` to be assumed until the selected
arguments and live invocation heap have established `A_r`. Facts about aliased
locations in `W_f` are not carried across normal execution unless `Q`
re-establishes them. Facts about aliased memory at the original condition
failure are not carried across handler execution unless the argument contract
re-establishes them, and facts about locations in `W_r` are not carried across
restart execution unless `R_r` re-establishes them.

Clause evaluation itself adds no heap transition: observational validation
guarantees that `P`, `Q`, `A_r`, and `R_r` leave their respective input heaps
unchanged in verification and in debug-mode runtime checks.

## Diagnostics

There are three distinct failures, and their blame must remain distinct.

### Restart implementation violates its own postcondition

This is an ordinary callee postcondition failure. It uses the existing
top-level `VerifyFailed` status and is classified semantically as
`VowEnsuresViolated`. It points at the restart expression and failed restart
clause, and identifies the restart in the function/path context. It does not
blame a handler that selected a recovery advertised by the callee.

### A handler violates a restart argument contract

This is a caller precondition failure at the restart invocation. It uses the
existing top-level `VerifyFailed` status and is classified semantically as
`VowRequiresViolated`. It points at the invalid argument and failed restart
clause, identifies the selected restart, and uses `blame: "caller"`. The
verifier does not enter the recovery edge or assume `R_r` after this failure.
Its counterexample also appends an attempted selection to `recovery_path`, with
`entered: false`, so the callee, restart, call site, and handler arm remain
machine-readable without claiming that the recovery guarantee became
available.

### A caller relies on a guarantee absent from a recovery path

The primary counterexample remains the actual proof obligation that failed. For
example, if the enclosing function's `ensures` cannot be proved on a recovery
edge, the violation is that enclosing `ensures` clause with `blame: "callee"`.
Selecting a restart is not a precondition violation, so this must not be
reported as generic Caller blame.

Every counterexample whose trace crosses a recovery edge must add structured
`recovery_path` context. This is an array in execution order, with one entry
for every selected restart; nested, sequential, and repeated recoveries are
retained rather than collapsed. Successfully entered recovery edges use
`entered: true`. A restart argument-contract failure may add one final
`entered: false` entry for the attempted selection.

Every entry records the values that instantiate its contracts in two ordered
binding arrays and two ordered helper-evaluation arrays:

- `invocation_bindings` contains every selected restart argument, whether or
  not `A_r` mentions it, every callee argument free in `A_r`, and every heap
  projection that `A_r` reads directly outside a helper call. Values are
  observed in `H_r`.
- `invocation_evaluations` contains the counterexample value of every maximal
  scalar helper-derived expression evaluated from `A_r` on the concrete path,
  after substituting its actual arguments. For `score(state) >= minimum`, it
  records the value of `score(state)`, not every field or collection element
  read inside `score`.
- `completion_bindings` contains every selected restart argument plus every
  callee argument free in `R_r`, every heap projection that `R_r` reads
  directly outside a helper call, and `result` when referenced. Values are
  observed in `H'_r`.
- `completion_evaluations` contains the corresponding helper-derived scalar
  values for `R_r` in `H'_r`.

Both completion arrays are empty when `entered` is false because no completion
guarantee became available.

Each binding has a `role` (`restart_argument`, `callee_argument`,
`reachable_state`, or `result`), its source `name`, and its counterexample
`value` using the same string encoding as the top-level `values` map. The same
source may appear in both arrays with different values when the restart writes
it. A heap-backed argument's value is a stable identity local to the
counterexample, not a recursive serialization of its reachable graph. Each
helper evaluation has its source `expression`, counterexample `value`, and
source span. If a helper returns an aggregate and the clause projects from it,
the evaluation records the scalar projection used by the clause. Within each
phase, selected restart arguments appear in declaration order, followed by
direct bindings and helper evaluations in lexical first-use order; the first
occurrence of an identical observation wins.

The transitive read closure is used to prove that a helper is observationally
read-only, but it is deliberately not materialized in the diagnostic. Direct
bindings plus boundary evaluation values are sufficient to reconstruct the
instantiated clause while bounding each entry by the clause's surface size,
independent of the collection or heap-graph size traversed inside a helper.
No selected argument, direct observation, or boundary evaluation may be
omitted. Keeping both phases on the individual occurrence makes repeated
selections distinguishable and identifies the exact instantiations of `A_r`
and `R_r`; the top-level `values` map describes the failed proof obligation
and is not a substitute for this occurrence-local context.

The future schema addition contains the following field. This is an
illustrative counterexample fragment, not a complete schema-valid object;
unchanged required fields such as `values`, `vow_id`, and `source` are omitted:

```json
{
  "recovery_path": [
    {
      "callee": "read_positive",
      "restart": "use_value",
      "argument_contract": "value >= minimum",
      "postcondition": "result == value && is_valid(state)",
      "entered": true,
      "invocation_bindings": [
        { "role": "restart_argument", "name": "value", "value": "0" },
        { "role": "callee_argument", "name": "minimum", "value": "0" }
      ],
      "invocation_evaluations": [],
      "completion_bindings": [
        { "role": "restart_argument", "name": "value", "value": "0" },
        { "role": "callee_argument", "name": "state", "value": "heap#1" },
        { "role": "result", "name": "result", "value": "0" }
      ],
      "completion_evaluations": [
        {
          "expression": "is_valid(state)",
          "value": "true",
          "source": {
            "file": "reader.vow",
            "offset": 88,
            "length": 15
          }
        }
      ],
      "call_site": {
        "file": "reader.vow",
        "offset": 120,
        "length": 42
      },
      "handler_arm": {
        "file": "reader.vow",
        "offset": 150,
        "length": 12
      }
    }
  ]
}
```

These lowercase blame strings are the static counterexample wire values
defined by `counterexample.schema.json`. Debug-mode runtime `VowViolation`
output is a separate schema and retains its capitalized `"Caller"` and
`"Callee"` values.

Human output must render the whole recovery path in the same order, then relate
the relevant guarantee to the failed obligation. It must show invocation and
completion bindings and evaluations separately when present. For a single
entry: `recovery through read_positive::use_value with invocation {value = 0,
minimum = 0}, completion {value = 0, state = heap#1, result = 0}, and completion
evaluation {is_valid(state) = true} guarantees result == value &&
is_valid(state); this path does not establish ensures result > 0`.

This ADR does not allocate a new error code. The failure is produced by the
existing verifier, not the parser or type checker. The condition/restart
implementation must update `docs/spec/cli.md`, its JSON schema, and both
compilers together when `recovery_path` becomes real.

## Why this design

The decision satisfies Vow's language-design constraints:

- **Verifier impact stays local.** Restarts already require finite explicit
  outcome branches. Propagating a different verified assumption per branch
  uses existing CFG and SMT machinery rather than refinement typing. Modeling
  handler interference and outcome writes conservatively havocs existing heap
  state, while validating observational contract purity reuses the same write
  footprints; neither adds a borrow or alias axis to the type system.
- **It eliminates an agent bug class.** An agent cannot accidentally consume a
  degraded recovery as though normal success occurred; the false assumption
  becomes a counterexample on the exact recovery path.
- **It keeps contracts usable.** Agents see the available fact at the handler
  arm and receive a structured path back to the responsible restart when a
  proof fails.

## Considered alternatives

### Require every restart to establish `Q`

This gives every completion one uniform contract and is the simplest sound
rule. It was rejected because a failed precondition can make `Q` unattainable;
forbidding a weaker recovery would exclude the central use case rather than
model it. A restart without a specific postcondition still defaults to this
strict behavior.

### Collapse outcomes to one unconditional disjunction

The verifier could expose `Q || R_r || R_s` after every handled call. This is
sound but unnecessarily imprecise: it discards which handler arm ran and can
reject code that proves its obligation separately in each arm. Keeping path
predicates costs no new semantic mechanism.

### Add restart-refined result types

A sum or refinement type could encode every possible completion in the type of
`handle`. This was rejected because it creates a new type-system axis, expands
every function signature that mentions restarts, and duplicates facts that the
verification CFG already represents.

### Automatically weaken the enclosing function's contract

This was rejected because contracts state semantic truth, not inferred proof
limits. Silent weakening would make public contracts depend on an internal
recovery choice and could invalidate callers without an explicit source change.

### Treat call-time heap contents as an implicit snapshot

This would let a restart retain facts that were true before the handler ran,
but those facts would no longer describe the live shared object observed by
the restart or caller. Vow has no implicit deep-copy passing semantics, so
silently changing contract references into snapshots would be surprising and
would require a new verifier representation. Historical relations belong in a
future explicit snapshot feature, not restart dispatch.

### Forbid handler mutation through aliases

This would also be sound, but enforcing it requires a new borrow, ownership,
or effect distinction for memory reachable from handled-call arguments. The
conservative invocation-state rule uses Vow's existing alias semantics and
allows safe mutations whenever the handler can re-establish `A_r` afterward.

### Materialize every transitive helper read in diagnostics

This can make one recovery entry linear in the size of a collection or heap
graph traversed by a contract helper. Recording the scalar value observed at
the clause/helper boundary preserves the exact instantiated predicate with
space bounded by contract surface size. The helper's transitive read closure
is still analyzed for observational purity, but it is not diagnostic payload.

### Allow contract evaluation to mutate shared state

This would require another write transition before and after each `P`, `Q`,
`A_r`, and `R_r` evaluation, plus corresponding interface metadata. More
importantly, debug builds evaluate runtime contract checks while release builds
omit them, so a mutating contract helper would make program behavior depend on
build mode. Contracts are observations, not state transitions; rejecting
shared writes preserves that boundary. Private fresh, non-escaping helper
storage remains an implementation detail and is harmless.

## Deferred implementation choices

The following remain part of the broader condition/restart feature design:

- surface syntax for declaring `R_r`;
- mapping raised conditions to available restarts and checking handler
  exhaustiveness;
- runtime selection and parameter-passing ABI;
- continuation representation and whether restart invocation returns locally;
- exact IR nodes and the encoding or precision of `W_f` and `W_r` in interface
  metadata.

Those choices may change without changing this ADR's invariant: each reachable
outcome applies a sound write footprint and exposes only the postcondition
proved for that outcome.

## Future conformance tests

When condition/restart support is implemented in both compilers, its first
public-interface tests must cover:

1. a normal edge retaining the main `ensures`;
2. normal-body mutation invalidating aliased entry facts unless `Q`
   re-establishes them;
3. a weak recovery proving a weaker enclosing postcondition;
4. the same recovery failing a stronger enclosing postcondition;
5. a handler re-establishing the stronger property;
6. an unreachable weak recovery not contaminating the join;
7. parameterized restart postcondition instantiation;
8. rejection of invalid restart arguments before their postcondition becomes
   available;
9. handler mutation through an aliased call argument invalidating a stale
   failure-state fact unless `A_r` re-establishes it;
10. sequential and nested recoveries retaining every selected restart in
    execution order, including repeated selections with identical restart
    arguments but distinct invocation or completion bindings;
11. recovery diagnostics capturing helper-derived scalar evaluations without
    expanding the helper's internal heap reads;
12. diagnostic size remaining bounded by contract surface size when a helper
    scans a large collection or recursively traverses a heap graph;
13. rejection of effect-free helpers that mutate shared heap state from `P`,
    `Q`, `A_r`, or `R_r`, while accepting observationally read-only helpers;
14. restart mutation invalidating aliased pre-invocation facts unless `R_r`
    re-establishes them; and
15. Rust/self-hosted parity for `recovery_path` diagnostics.
