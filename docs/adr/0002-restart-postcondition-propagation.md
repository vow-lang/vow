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
- `Q(args, result)` is the conjunction of `f`'s normal `ensures` clauses.
- `A_r(args, restart_args)` is the conjunction of restart `r`'s argument
  preconditions; it is `true` when the restart declares none.
- `r` is a declared restart and `R_r(args, restart_args, result)` is its
  restart-specific postcondition.
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

The verifier checks every declared restart independently. In the symbolic
state at the point where the restart can run, and under the assumption `A_r`,
the restart expression must establish its declared `R_r`. This is a callee
obligation, just like an ordinary `ensures` clause. Proving `R_r` under `A_r`
does not make `R_r` available for restart arguments that violate `A_r`.

The function's ordinary body is still checked against `Q`. A weaker `R_r` does
not weaken that check, and proving the ordinary body against `Q` does not prove
the restart.

### Facts exposed at a handle site

A handled call lowers to explicit verifier control-flow edges:

1. The normal edge carries `assume(Q(actual_args, call_result))`.
2. Selecting restart `r` first creates the caller obligation
   `assert(A_r(actual_args, selected_restart_args))` at the restart invocation.
3. Only after that obligation succeeds does the recovery edge carry
   `assume(R_r(actual_args, selected_restart_args, call_result))`.
4. Handler code and all downstream proof obligations are checked on every
   reachable edge.

For a parameterized restart, the postcondition is instantiated with the actual
arguments supplied by the handler, after those arguments have satisfied the
restart's parameter contract. This lets a handler prove a stronger fact for a
particular valid selection even when the restart's general contract is weak.

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

An unhandled call keeps today's rule: its caller must prove `P`, and only its
normal edge and `Q` are available. Declaring a restart does not weaken ordinary
calls.

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

## Verification boundary

Caller verification remains modular. Compiled interface metadata for a
restart-capable function must include `Q`, every advertised `R_r`, restart
parameter contracts, and the source identities needed for diagnostics. A
caller imports those summaries and does not inline the callee's body or restart
expressions.

The soundness chain is:

1. Callee verification proves the normal body establishes `Q` and each restart
   establishes its own `R_r`.
2. Handle-site lowering proves the selected restart's argument contract, then
   exposes exactly one verified summary on each outcome edge.
3. Caller verification proves downstream obligations independently on all
   reachable edges.

No step permits `Q` to be assumed on a recovery edge unless that edge's facts
independently imply `Q`. No step permits `R_r` to be assumed until the selected
arguments have established `A_r`.

## Diagnostics

There are three distinct failures, and their blame must remain distinct.

### Restart implementation violates its own postcondition

This is an ordinary callee postcondition failure. It uses the existing
`VerifyFailed` / `VowEnsuresViolated` family, points at the restart expression
and failed restart clause, and identifies the restart in the function/path
context. It does not blame a handler that selected a recovery advertised by the
callee.

### A handler violates a restart argument contract

This is a caller precondition failure at the restart invocation. It uses the
existing `VerifyFailed` / `VowRequiresViolated` family, points at the invalid
argument and failed restart clause, identifies the selected restart, and uses
`blame: "Caller"`. The verifier does not enter the recovery edge or assume
`R_r` after this failure. Its counterexample also includes `recovery_path`
context so the callee, restart, call site, and handler arm remain
machine-readable.

### A caller relies on a guarantee absent from a recovery path

The primary counterexample remains the actual proof obligation that failed. For
example, if the enclosing function's `ensures` cannot be proved on a recovery
edge, the violation is that enclosing `ensures` clause with `blame: "Callee"`.
Selecting a restart is not a precondition violation, so this must not be
reported as generic Caller blame.

Every counterexample whose trace crosses a recovery edge must add structured
`recovery_path` context. The future schema addition has this minimum shape:

```json
{
  "function": "read_strictly_positive",
  "violation": "ensures result > 0",
  "blame": "Callee",
  "recovery_path": {
    "callee": "read_positive",
    "restart": "use_zero",
    "postcondition": "result == 0",
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
}
```

Human output should explain the same path directly: `recovery through
read_positive::use_zero guarantees result == 0; this path does not establish
ensures result > 0`.

This ADR does not allocate a new error code. The failure is produced by the
existing verifier, not the parser or type checker. The condition/restart
implementation must update `docs/spec/cli.md`, its JSON schema, and both
compilers together when `recovery_path` becomes real.

## Why this design

The decision satisfies Vow's language-design constraints:

- **Verifier impact stays local.** Restarts already require finite explicit
  outcome branches. Propagating a different verified assumption per branch
  uses existing CFG and SMT machinery rather than refinement typing.
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

## Deferred implementation choices

The following remain part of the broader condition/restart feature design:

- surface syntax for declaring `R_r`;
- mapping raised conditions to available restarts and checking handler
  exhaustiveness;
- runtime selection and parameter-passing ABI;
- continuation representation and whether restart invocation returns locally;
- exact IR nodes and interface-metadata encoding.

Those choices may change without changing this ADR's invariant: each reachable
outcome exposes only the postcondition proved for that outcome.

## Future conformance tests

When condition/restart support is implemented in both compilers, its first
public-interface tests must cover:

1. a normal edge retaining the main `ensures`;
2. a weak recovery proving a weaker enclosing postcondition;
3. the same recovery failing a stronger enclosing postcondition;
4. a handler re-establishing the stronger property;
5. an unreachable weak recovery not contaminating the join;
6. parameterized restart postcondition instantiation;
7. rejection of invalid restart arguments before their postcondition becomes
   available; and
8. Rust/self-hosted parity for `recovery_path` diagnostics.
