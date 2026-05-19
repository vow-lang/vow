# Live Programming for Agents: Research Notes

What features could Vow support for debugging, fixing, and redeploying running
applications — so that agents can diagnose and patch production services without
downtime?

This document surveys the mechanisms that made Lisp, Erlang, and Smalltalk
legendary for live programming, evaluates each against Vow's architecture and
design criteria, and proposes five concrete features.

## Design Criteria

Every proposed feature must pass Vow's three-gate test:

1. **Does not make verification harder** — the ESBMC pipeline and C model must
   not become more complex or fragile.
2. **Eliminates a class of agent bugs** — the feature must make it harder for
   agents to generate incorrect code, not just shorter code.
3. **Makes agentic coding easier** — the feature must help agents produce
   correct, verified programs more reliably.

---

## Prior Art

### Common Lisp: Condition/Restart System

CL separates three concerns that most languages conflate into throw/catch:

- **Conditions** — objects that describe a situation (not necessarily an error).
- **Restarts** — named recovery strategies declared by the callee. They sit on
  the stack as callable continuations.
- **Handlers** — policy selectors declared by the caller. They run *with the
  callee's stack frame still intact*, inspect the condition, and invoke a
  restart.

The critical property: when a condition fires, the stack does not unwind. The
handler sees full program state and picks a recovery path. The callee defines
*what* recoveries are possible; the caller decides *which* one to use.

Two binding forms exist: `handler-bind` (handler runs without unwinding — the
powerful form) and `handler-case` (handler runs after unwinding — equivalent to
traditional catch).

**Why it matters:** Error recovery becomes composable. Library code and
application code each contribute their half of the recovery logic without
knowing each other's implementation.

### Common Lisp: Image-Based Development

A Lisp "image" is a serialized snapshot of the entire runtime heap — all
functions, classes, variables, loaded libraries, REPL history. You save it with
`save-lisp-and-die`, producing a single blob. Restarting from an image restores
the exact program state. No re-parsing, no re-compiling, no re-loading.

**Tradeoff:** Images are opaque and non-reproducible. Accidental mutations
persist forever. Images are tied to a specific runtime version. This conflicts
with reproducible builds and source-as-truth.

### Common Lisp: Hot Code Reloading and CLOS

Every `defun` immediately replaces the function object in the symbol's function
cell. All callers that dispatch through the symbol pick up the new definition on
next call.

CLOS (the object system) handles class redefinition with lazy instance
migration: when a class is redefined, existing instances are updated on next
access via `update-instance-for-redefined-class`. The protocol computes slot
differences (added, removed, changed), copies retained values, and initializes
new slots. User code can specialize the migration method per class.

**Key property:** This is a protocol, not a fixed mechanism. The runtime
provides detection and dispatch; user code provides migration semantics.

### Common Lisp: Inspector/Debugger

When an unhandled condition fires, the debugger activates with the stack intact.
It is not a post-mortem core dump — it is a live session in the error's context.
Capabilities: stack frame navigation, local variable inspection, restart
selection, frame restart (re-execute a function after redefining it), live
evaluation in any frame's lexical scope. SLIME/SLY present this as a structured
protocol over Emacs buffers.

### Erlang/BEAM: Two-Version Module Loading

The BEAM VM allows exactly two versions of any module to coexist: "current" and
"old". Running processes continue executing the old version. New fully-qualified
calls dispatch to the current version. A process migrates when it makes a
fully-qualified call to its own module (typically a tail-recursive loop entry
point).

**Prerequisites:** Process isolation (each process has its own heap, no shared
mutable state) and a well-defined migration protocol (`gen_server:code_change/3`
transforms process state from old schema to new).

If a third version is loaded while old-version processes still exist, those
processes are killed. The two-version limit keeps the system bounded.

### Erlang: Supervisors and "Let It Crash"

Supervisors monitor child processes and restart them according to a strategy
(`one_for_one`, `one_for_all`, `rest_for_one`). Processes are cheap, isolated,
and restartable. A crash in one process cannot corrupt another's state.
Supervisor trees make the system self-healing at each level.

### Smalltalk: Method-Level Editing

The unit of compilation is a single method. Edit it in the browser, hit
"Accept", and it compiles and installs immediately. All existing instances pick
up the new method on next dispatch. `doesNotUnderstand:` enables dynamic
dispatch fallbacks; `become:` swaps object identity. Both are powerful but
fundamentally at odds with static verification.

### Summary of Transferable Ideas

| Mechanism | Transferable? | Key tension with Vow |
|---|---|---|
| Condition/restart separation | Yes | Restart paths need verification |
| Image-based development | No | Conflicts with reproducible builds |
| Hot function reloading | Yes | Requires indirection layer |
| Lazy instance migration | Yes | Migration must be verified |
| Interactive debugger | Yes | Needs debug metadata |
| Incremental compilation | Yes | Verification may need whole-program |
| Two-version modules | Yes | Needs process/actor isolation |
| Let it crash + supervisors | Yes | Runtime pattern, not language feature |
| `doesNotUnderstand` / `become:` | No | Blocks static verification |

---

## Vow's Existing Infrastructure

Several architectural properties make live programming feasible without
fundamental redesign:

- **Blame semantics.** `requires` violations blame the Caller; `ensures`
  violations blame the Callee. This is the same axis as CL's handler/restart
  separation.
- **Per-function verification.** ESBMC verifies each function in isolation.
  Individual functions can be re-verified without recompiling the entire program.
- **Parallel codegen + verify.** The CLI driver already spawns verification in a
  background thread while codegen proceeds.
- **Compilation caching.** `CompileCache` stores object files keyed by source +
  mode + trace. `VerifyCache` stores results keyed by C source content.
- **Source location tracking.** Every IR instruction carries `origin: Span`.
  Every function has `local_names` mapping instruction IDs to variable names.
- **Effect system.** Functions declare effects (`io`, `read`, `write`, `panic`,
  `unsafe`). Effect propagation is checked at compile time. Pure functions have
  empty effect sets.
- **Debug/trace modes.** `--mode debug` inserts runtime vow checks.
  `--debug-trace full` emits JSON events for function entry/exit and vow
  pass/fail. `--mode profile` adds call counters with an atexit report.
- **VowEntry bindings.** Contract metadata includes `bindings: Vec<(String,
  InstId)>` — the names and values of all free variables in each predicate,
  captured at the check point.

**Current limitations:**

- Static linking (vow-runtime and vow-clif-shim are linked into the binary).
- Direct calls only (no function pointer indirection layer).
- Violations call `exit(1)` — no recovery path.
- Module merging flattens the namespace — no scoped imports.
- No actor or process model.

---

## Proposed Features

### 1. Verified Condition/Restart System

**Problem:** Today, a vow violation kills the process. The agent learns what
went wrong (structured JSON with vow ID, blame, variable bindings) but cannot
recover. The only option is to recompile with different logic and restart.

**Proposal:** Allow `vow` blocks to declare named restarts — callee-defined
recovery paths that the caller selects:

```vow
fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0,
    restart use_default: 0,
    restart use_value(v: i64): v,
} {
    x / y
}

fn process(data: i64) -> i64 [io] {
    handle divide(data, get_divisor()) {
        use_default => { log("used default"); use_default() },
        use_value   => use_value(1),
    }
}
```

Each restart is a pure expression producing the function's return type. The
`handle` block at the call site selects which restart to invoke when the
precondition fails.

**Desugaring:** A nullary restart (`use_default: 0`) is a branch in the
existing vow-check IR. Today, a `VowRequires` instruction branches to a
violation block (which calls `__vow_violation` and exits) or a continuation
block. With nullary restarts, the violation block branches to the restart
expression instead — no new IR opcodes are needed.

Parameterized restarts (`restart use_value(v: i64): v`) are harder: the value
`v` supplied at the `handle` site must flow into the callee's failure branch.
That is a continuation-passing edge, not a simple retarget. It needs either a
new IR construct for caller-supplied restart arguments or a calling-convention
extension that threads restart arguments alongside the precondition check.
Either way, this is more than a branch-target tweak and should be sized
accordingly.

**Verification:** Each restart expression is a separate ESBMC verification
target. The verifier proves that the restart's return value satisfies the
function's `ensures` clause (or a weaker restart-specific postcondition). The
restart paths are finite, statically enumerable, and bounded.

**Agent bug class eliminated:** Unrecoverable contract violations. Agents can
program compositional recovery strategies without recompilation.

### 2. Two-Version Function Patching with Verification Gates

**Problem:** Fixing a bug in a running service requires recompiling the entire
program and restarting. Even if the fix is a one-line change to a single
function, the service goes down.

**Proposal:** A `vowc patch` subcommand for hot-patching individual functions:

```bash
vowc patch --target /tmp/my_service --function serve_request app/server.vow
```

The pipeline:

1. Parse and type-check the changed function against the existing module's type
   environment.
2. Verify the new function's contracts via ESBMC.
3. Verify ABI compatibility: same parameter types, same return type, compatible
   effects (new version may have fewer effects, not more).
4. Compile the new function to a shared object.
5. Atomically swap the function pointer in a global dispatch table.

Functions opt into patchability:

```vow
patchable fn serve_request(req: Request) -> Response [io] vow {
    ensures: result.status >= 200,
} { ... }
```

`patchable` functions are compiled with one level of indirection: calls go
through a function pointer table instead of direct Cranelift calls. The cost is
one pointer load per call.

**Verification gate:** A patch is rejected unless ESBMC proves the new version
satisfies its contracts. An agent cannot deploy unverified code. This is the key
difference from Erlang's trust-the-developer model.

**ESBMC and patchable call sites:** Function-pointer indirection makes the call
target statically opaque to ESBMC's call-graph analysis. The verifier handles
patchable callees as opaque calls: the contract is the interface — `requires`
clauses are asserted at the call site, `ensures` clauses are assumed on return.
Both the patched callee and any patch candidate must therefore verify against
the same contract for the caller's proof to remain sound. This preserves
modular verification at the cost of giving up cross-function reasoning across
the patchable boundary.

**Effect system role:** The effect system constrains what changes are safe. A
pure function can always be patched (callers depend only on the type and
contract, not the implementation). An effectful function requires checking that
callers handle the new effect set.

**Agent bug class eliminated:** Must-restart-to-fix. Agents can fix, verify, and
deploy single functions while the service continues running.

### 3. Structured Introspection Protocol

**Problem:** Today an agent can only observe a running Vow program through
stderr logs (`--debug-trace`) or post-mortem violation JSON. There is no way to
query a running program's state interactively.

**Proposal:** An `--inspect` build mode that links in an introspection server
exposing a JSON-RPC protocol over a Unix socket:

```json
{"method": "stack_frames"}
{"method": "frame_locals", "params": {"frame_id": 3}}
{"method": "vow_status", "params": {"function": "divide"}}
{"method": "active_restarts"}
{"method": "invoke_restart", "params": {"restart": "use_default"}}
{"method": "list_functions"}
{"method": "function_contracts", "params": {"function": "divide"}}
```

This extends existing infrastructure:

- `__vow_trace_enter/exit` already track function entry/exit — add a frame
  stack.
- `local_names` already maps instruction IDs to variable names — emit DWARF-like
  debug info.
- `VowEntry.bindings` already captures variable values at check points — expose
  them via the protocol.

The inspection server runs in a dedicated thread with its own `IO` effect
context, isolated from the inspected program. Pure functions remain pure from
the program's type-system and verification perspective — their effect set,
contracts, and emitted code are unchanged. The inspector observes locals from
the outside, so an `--inspect` build is not strictly side-effect-free at the
process level; the side channel is intentional and only available when the
binary is built with `--inspect`.

**Agent workflow:** Agent detects anomalous behavior via monitoring. Connects to
the inspection socket. Queries stack frames and variable values. Identifies the
bug. Uses `vowc patch` to deploy a fix. Verifies the fix took effect via the
inspection protocol. All without restart.

**Agent bug class eliminated:** Printf-only debugging. Agents get structured,
queryable access to runtime state — the Lisp debugger experience, over a
machine-readable protocol.

### 4. Verified State Migration

**Problem:** When hot-patching code that operates on structs, a change to the
struct's shape invalidates all existing values in memory. Without a migration
path, the patch must be rejected.

**Proposal:** When a `patchable` struct changes shape, the agent provides a
migration function with contracts. The example below uses explicit `_v1` /
`_v2` suffixes; a real implementation needs a versioned-type reference (e.g.
`old::Config`) the type checker can resolve to the previous shape during patch
compilation, since both definitions cannot coexist under one name:

```vow
// Old version
struct Config_v1 { timeout: i64, retries: i64 }

// New version
struct Config_v2 { timeout: i64, retries: i64, backoff_ms: i64 }

// Migration function (verified)
fn migrate_config(old: Config_v1) -> Config_v2 vow {
    ensures: result.timeout == old.timeout,
    ensures: result.retries == old.retries,
    ensures: result.backoff_ms > 0,
} {
    Config_v2 { timeout: old.timeout, retries: old.retries, backoff_ms: 1000 }
}
```

The `vowc patch` command detects struct shape changes, requires a migration
function, verifies it via ESBMC, and applies it to all live instances using type
metadata emitted in inspect builds. The two-version naming above is
presentational; the actual surface syntax for referring to a pre-patch type
(versioned module path, `old::Config`, or similar) is left to be designed.

This is the Vow-native version of CLOS's
`update-instance-for-redefined-class` and Erlang's `gen_server:code_change/3` —
but with the migration itself formally verified.

**Agent bug class eliminated:** Struct shape changes forcing full restart. Agents
can evolve data structures in place with proof that invariants are preserved.

### 5. Effect-Guided Incremental Recompilation

**Problem:** Today, any source change invalidates all compiled objects (the
cache keys on full source content). Verification caching is per-function, but
there is no dependency tracking to know which functions need re-verification
when a contract changes.

**Proposal:** `vowc` maintains a dependency graph mapping functions to their
contract dependencies. On recompilation, it computes the minimal re-verification
set:

| Change kind | Re-verify |
|---|---|
| Body changed, contracts unchanged | Only the changed function |
| `ensures` weakened | Changed function + all callers (any caller may have relied on the stronger postcondition; pruning to a true subset requires whole-program postcondition-dependency analysis) |
| `requires` strengthened | Changed function + all call sites |
| Effects changed | Re-type-check all callers (effect propagation) |
| Pure function, same signature | Only the changed function |

```bash
vowc build --incremental app.vow
```

The effect system already computes function-level effect sets. Extending the
compilation cache with contract-dependency edges lets the system prune
unnecessary verification work.

**Verification story:** This makes verification *faster*, not harder. It uses
information already computed (effect sets, contract signatures) to avoid
redundant ESBMC invocations.

**Agent bug class eliminated:** Slow iteration cycles. An agent fixing a bug
gets sub-second feedback on whether the changed function verifies, without
re-verifying the entire program.

---

## Feature Interaction

The five features compose naturally:

1. **Incremental recompilation** (5) provides fast iteration during development.
2. **Condition/restart** (1) lets the running program survive contract
   violations while the agent investigates.
3. **Introspection** (3) lets the agent diagnose the root cause in the live
   process.
4. **Function patching** (2) lets the agent deploy the fix without restart.
5. **State migration** (4) handles the case where the fix requires changing data
   structures.

The end-to-end agent workflow:

```
Monitor detects anomaly
  → Agent connects to inspection socket (3)
  → Queries stack frames, variable values, vow status
  → Identifies broken function
  → Edits source, runs incremental verify (5)
  → Verification passes
  → Deploys patch to running process (2)
  → If struct shape changed, verified migration runs (4)
  → Meanwhile, condition/restart (1) kept the service alive
  → Agent confirms fix via inspection protocol (3)
```

This is the Lisp dream — fix and continue without losing a request — but with
every step verified.

---

## Implementation Priority

Suggested ordering by effort-to-value ratio:

1. **Incremental recompilation** — lowest effort (extends existing caching),
   highest daily impact for agent iteration speed.
2. **Condition/restart system** — medium effort (new syntax, IR branch targets,
   verification of restart paths), high value for runtime resilience.
3. **Introspection protocol** — medium effort (extends existing trace
   infrastructure), enables the rest of the live-programming workflow.
4. **Function patching** — high effort (dispatch table, shared object
   compilation, atomic swap), but transforms deployment model.
5. **State migration** — high effort (type metadata at runtime, instance
   scanning), but only needed when patching changes struct shapes.

---

## What We Explicitly Reject

- **Image-based development.** Conflicts with reproducible builds, source-as-truth,
  and verification. Incremental caching achieves the speed benefit without the
  opacity.
- **`doesNotUnderstand` / dynamic dispatch fallbacks.** Introduces dispatch
  opacity that blocks static verification. Fundamentally at odds with decidable
  type checking.
- **`become:` / identity swapping.** Powerful but makes verification of pointer
  identity intractable.
- **Speculative execution with rollback.** Would require STM or copy-on-write
  heaps — significant complexity for unclear agent benefit.

---

## References

- Common Lisp condition system: ANSI CL specification, Chapter 9 (Conditions)
- Erlang/OTP hot code loading: OTP Design Principles, Release Handling
- CLOS instance migration: ANSI CL, `update-instance-for-redefined-class`
- Smalltalk live programming: Pharo by Example, Chapter 1 (A Quick Tour)
- SLIME/SLY protocol: SLIME manual, Section 3 (SLDB: The SLIME Debugger)
