# 0002. Restart postconditions supplement the function contract

**Status:** accepted (2026-07-31)

## Context

The verified condition/restart proposal in
[`docs/live-programming-research.md`](../live-programming-research.md#1-verified-conditionrestart-system)
originally allowed a restart to replace a function's normal postcondition with
a weaker restart-specific postcondition. That is unsound if a caller continues
to rely on the function contract, and preserving the weaker outcome as a
modular caller fact would require new outcome summaries and heap-footprint
machinery.

Condition/restart syntax and runtime support are not implemented yet. This ADR
fixes the verification invariant that a future implementation must preserve;
it does not select the remaining surface syntax, dispatch ABI, or continuation
representation.

## Terminology

For a function `f`:

- `P` is the conjunction of `f`'s ordinary `requires` clauses.
- `Q(args, result)` is the conjunction of `f`'s ordinary `ensures` clauses.
- `r` is a restart declared by `f`.
- `A_r(args, restart_args)` is the conjunction of `r`'s restart-argument
  preconditions. It is `true` when none are declared.
- `R_r(args, restart_args, result)` is `r`'s restart-specific postcondition. It
  is `true` when none is declared.
- A **normal edge** completes `f`'s ordinary body after the caller proves `P`.
- A **recovery edge** completes through a restart selected by a `handle` site
  after a condition is raised.

`args` in `A_r`, `Q`, and `R_r` use ordinary Vow passing semantics. A
heap-backed argument denotes the live object at restart invocation and
completion; restart contracts do not create an implicit call-time snapshot.

## Decision

Every successful completion of a function, normal or recovered, must establish
the function's ordinary postcondition `Q`. A restart-specific postcondition is
an additional path-local fact, not a replacement for `Q`. Restart expressions
are restricted to heap-read-only computation so recovery does not require a
new write-footprint summary.

The exported guarantee of recovery through `r` is therefore:

```text
Q(args, result) && R_r(args, restart_args, result)
```

If a restart expression establishes its declared `R_r` but cannot establish
`Q`, the verifier rejects the restart declaration. Vow does not admit a
selectable recovery whose successful return changes the called function's
public contract.

This rule deliberately gives a negative answer to the premise of this issue:
a postcondition that is genuinely weaker than `Q` cannot be the sole guarantee
of a successful restart. An author who wants such an outcome must state the
weaker truth in the function's ordinary contract, return an explicit result
variant such as `Option`, or expose a separate function with that contract.
Recovery policy must not silently rewrite a function interface.

### Verification model

Each restart is verified as a heap-read-only synthetic target using the
verifier's existing function-contract machinery. Conceptually, restart `r`
has this shape:

```text
requires A_r
ensures Q
ensures R_r
```

The synthetic target receives the callee arguments and selected restart
arguments using their ordinary representations. Its proof starts from
`A_r`; it does not inherit mutable facts from the point where the original
condition failed. In particular, a handler may have changed a shared object
before selecting the restart. If the restart needs a fact about the live
object to establish `Q`, that fact must be included in `A_r`, and the handler
must prove it at the invocation site.

For the initial feature, heap-read-only is a structural restriction on the
restart expression's lowered instructions. Literals, scalar operators,
callee/restart arguments, and reads through those arguments are permitted.
Assignments, field or indexed writes, mutable allocation, and user-defined
function or method calls are rejected. Known read-only compiler builtins may
be admitted directly. This fail-closed subset prevents a nominally
effect-free helper from hiding a write and requires only local opcode
validation, not transitive write or escape analysis. The subset may be widened
later only when existing compiler summaries can prove the same read-only
property without adding a new verifier mechanism.

This ADR does not strengthen purity rules for existing functions or contracts.
If those rules are independently found insufficient, that is a general
soundness issue and must not be hidden inside the restart feature.

### Facts exposed at a handle site

A handled call lowers to ordinary verifier control flow:

1. The normal edge proves `P`, executes the ordinary call, and exposes `Q` by
   the existing modular call rule.
2. A selected handler arm runs in the caller's current state.
3. Immediately before invoking restart `r`, the caller proves `A_r` with the
   selected restart arguments and the then-current callee arguments.
4. The recovery edge invokes the verified heap-read-only restart target and
   exposes both `Q` and `R_r`.
5. Downstream obligations are checked on every reachable edge.

The facts remain branch-sensitive in the verifier's ordinary CFG. At a join,
their logical summary is:

```text
Q
&& (
     normal_path
     || (restart_r_path && R_r)
     || (restart_s_path && R_s)
     || ...
   )
```

`Q` is available after every successful outcome. `R_r` is available only on
the `r` edge or under a guard that proves that edge was selected. No
restart-refined result type is introduced.

An unhandled call keeps today's rule: the caller proves `P`, invokes `f`, and
uses `Q`. Merely declaring a restart does not weaken or otherwise change an
ordinary call.

### Handler and enclosing-function contracts

A `handle` block is never required or permitted to weaken its enclosing
function's postconditions automatically. The enclosing function is checked
against its declared contract on the normal edge and every reachable recovery
edge, as it is for ordinary branches.

Because every recovery establishes `Q`, a caller may rely on the called
function's ordinary postcondition after the handled call. A caller may rely on
`R_r` only where control flow proves that `r` was selected. If a useful restart
cannot establish `Q`, the error belongs to the restart declaration, not to all
of its potential callers.

## Worked proof shape

Suppose `read_positive` guarantees `result > 0`.

- A restart `use_value(v)` with `A_use_value: v > 0`, expression `v`, and
  `R_use_value: result == v` verifies. Its recovery edge exposes both
  `result > 0` and `result == v`.
- The same restart with `A_use_value: v >= 0` does not verify, because the
  selected value `0` is allowed but cannot establish the function's
  `result > 0` contract. Declaring only `R_use_value: result >= 0` does not
  weaken that obligation.
- A handler can retain the strong function contract by selecting
  `use_value(1)`. It proves `A_use_value` at the selection site and then uses
  `Q` after recovery.

If returning zero is a valid semantic outcome, `read_positive` must advertise
that truth in its normal contract (for example, `result >= 0`) or return a type
that distinguishes ordinary success from fallback. A restart is not a hidden
exception to the contract.

### Live shared-state example

Suppose `Q` includes `result >= state.minimum`, where `state` is heap-backed.
A handler can mutate `state.minimum` before selecting a restart. The restart
proof receives the live `state`, not the stale failure-state contents. A
restart `use_value(v)` may require `v >= state.minimum`; the handler proves
that requirement after its mutations, and the restart expression `v` then
establishes `Q` under the same live-state relation.

No restart-specific heap footprint is needed: the restart expression is
heap-read-only, and the handler mutation is already ordinary caller code
checked before the restart invocation.

## Diagnostics

No new caller-side "weaker recovery" error or JSON schema is needed, because a
weaker recovery guarantee is never exported.

### A restart cannot establish the function postcondition

This is an ordinary callee postcondition failure. It uses the existing
top-level `VerifyFailed` result and `VowEnsuresViolated` classification with
callee blame. The diagnostic points to the failed ordinary `ensures` clause
and identifies the synthetic restart target in the existing function/path
context, for example:

```text
verification failed in read_positive::restart use_value:
ensures result > 0 is not established when v = 0
```

Failure of the additional `R_r` clause has the same classification and points
to that clause instead. The handler is not blamed for selecting a restart that
the callee falsely advertised; invalid restarts are rejected before callers
can import them.

### A handler violates a restart argument contract

This is an ordinary caller precondition failure at the restart invocation. It
uses `VerifyFailed`, `VowRequiresViolated`, and caller blame, points to the
failed `A_r` clause and selected arguments, and identifies the handle arm as
the call site. The recovery edge is not entered and neither `Q` nor `R_r`
becomes available.

### A downstream caller obligation fails

This remains the diagnostic for the actual failed obligation. Existing trace
and source context may show the selected handle arm, but caller soundness does
not depend on a new `recovery_path` payload. In particular, a downstream use
of `Q` cannot fail merely because a valid restart ran: establishing `Q` was a
callee verification obligation for that restart.

The condition/restart implementation must update `docs/spec/cli.md` and both
compilers together if it later extends the diagnostic schema, but such an
extension is not required by this decision.

## Why this design

The decision satisfies Vow's language-design constraints:

- **Verifier impact is small.** Each restart reuses existing
  `requires`/`ensures`, modular-call, opcode-validation, and CFG machinery. It
  does not require outcome write footprints, transitive alias summaries,
  refinement typing, or a new diagnostic schema.
- **It eliminates an agent bug class.** An agent cannot accidentally treat a
  degraded recovery as normal success because successful recovery is required
  to honor the same public contract.
- **It keeps agentic coding direct.** Restart-specific facts are available on
  the selected branch, while the ordinary function contract remains stable
  for every caller and across module boundaries.

## Considered alternatives

### Let `R_r` replace `Q` on a recovery edge

This preserves weaker recoveries with path-local caller assumptions, but it
changes a function's exported guarantee according to caller-selected runtime
policy. A modular implementation also has to summarize normal and restart
writes so callers do not retain stale facts through aliases. That requires new
transitive write/escape analysis, alias-aware havoc, and interface metadata in
both compilers. The complexity violates Vow's requirement that surface
features have near-zero verifier impact, so this design is rejected.

### Automatically weaken the enclosing function's contract

Contracts state semantic truth, not inferred proof limits. Silent weakening
would make public contracts depend on an internal recovery choice and could
invalidate callers without an explicit interface change.

### Add restart-refined result types

A sum or refinement type could encode every completion in the type of
`handle`. This adds a new type-system axis and duplicates facts already
represented by verifier branches. An author who needs semantically distinct
outcomes should use an explicit ordinary data type in the function signature.

### Check the stronger postcondition only in debug mode

Runtime contract checks are omitted in release builds, so a dynamic check
cannot justify a static caller assumption. Every restart must prove `Q`
before code generation just as the ordinary body does.

### Infer that a particular handler makes a weak restart safe

Inlining each caller's handler into callee verification would give up modular
verification and make the validity of a restart depend on its current callers.
Restart argument contracts retain modularity: the callee proves `Q` under
`A_r`, and each handler separately proves the chosen arguments satisfy `A_r`.

## Deferred implementation choices

The following remain part of the broader condition/restart feature design:

- surface syntax for restart argument contracts and `R_r`;
- mapping raised conditions to available restarts and checking handler
  exhaustiveness;
- runtime selection and parameter-passing ABI;
- continuation representation and whether restart invocation returns locally;
- exact IR representation of the pure synthetic restart target.

Those choices may change without changing this ADR's invariant: every
successful outcome establishes `Q`, and a restart adds only path-local facts.

## Future conformance tests

When condition/restart support is implemented in both compilers, its first
public-interface tests must cover:

1. a normal edge exposing `Q`;
2. a restart independently establishing `Q` without assuming `P` or stale
   failure-state facts;
3. rejection of a restart that establishes only a weaker `R_r`;
4. acceptance of a restart whose `A_r` is sufficient to establish `Q`;
5. caller blame when a handler violates `A_r` in its live invocation state;
6. exposure of `Q && R_r` on the selected recovery edge;
7. retention of `Q`, but not an unguarded `R_r`, after control-flow joins;
8. no automatic weakening of an enclosing function's contract;
9. existing ensures diagnostics identifying a restart that cannot establish
   `Q`; and
10. rejection of restart expressions containing writes or user-defined calls;
    and
11. Rust/self-hosted parity for all restart verification and diagnostics.
