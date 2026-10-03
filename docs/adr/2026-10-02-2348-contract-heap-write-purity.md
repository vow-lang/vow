# Contract clauses may not write through their arguments

**Status:** accepted (2026-10-02)

## Context

`docs/adr/0002-restart-postcondition-propagation.md` (lines 86-94) requires
restart contracts `A_r`/`R_r` to be observationally read-only: validation
derives the transitive heap-write footprint of every called helper and
rejects a clause that can write pre-existing shared state. Review of that ADR
(issue #1032, raised by `chatgpt-codex-connector[bot]` as a P1 finding on
#1010) observed that the same restriction was never applied to an ordinary
function's `ensures` (or `requires`/`invariant`) clause — `check_vow_purity`
(`vow-types/src/effects.rs`) and its self-hosted counterpart
`check_clause_purity` (`compiler/checker.vow`) only rejected calls to callees
with a *declared* effect. `read`/`write`/`io`/`panic`/`unsafe` cover
filesystem/stdio/panic/FFI only; they say nothing about an ordinary
heap write through a parameter, because Vow passes structs, `Vec`, `String`,
and maps by pointer, so an effect-free helper can still mutate caller-visible
state through one of its arguments.

Concretely, before this change:

```vow
fn mark(p: Point) -> bool {
    p.x = 1;
    true
}

fn make_point(x: i64, y: i64) -> Point vow {
    ensures: mark(result)
} {
    Point { x: x, y: y }
}
```

type-checked. `mark` declares no effect, so the declared-effect check never
looked at it. Empirically (checked during implementation, not just inferred
from reading the C emitter): `vow verify` proves `ensures: mark(result)` by
modeling the field write for real — `FieldSet` is in ESBMC's modelable
instruction subset — and a built `--mode debug` *and* a plain `release`
binary both actually execute `mark`'s write too, because IR lowering always
emits the predicate's instructions (`vow-ir/src/lower/vow.rs::lower_predicate`
calls `lower_expr_pub` unconditionally); only the pass/fail *check* itself
(`emit_vow_check`) is gated on `mode.has_debug_checks()`, not the predicate's
side effects. So this specific construction does not reproduce a
verify-vs-release *divergence* in the current codebase — the write happens
in every mode today. The underlying problem is broader than that one
framing: a contract clause is supposed to be a side-effect-free check of the
function's behavior, and today it can instead be live code with a real,
undeclared heap-mutating effect, in whichever modes do evaluate it. Issue
#1032's own title names the verify/release-divergence framing, but the
fix below does not depend on that framing holding in every case — it
applies whenever a clause can write through a parameter, independent of how
many modes currently happen to execute that write.

## Decision

A `requires`/`ensures`/`invariant` clause must not evaluate to a write
reachable from any of its free variables: a struct-field or `Vec`/map-index
assignment, a call to a mutating builtin method, or a call (direct or
transitive) to a user function whose own body performs such a write. This is
checked at type-check time, in both compilers, and reported as the existing
`EffectViolation` diagnostic (`Blame::Callee` in the Rust frontend; the
self-hosted frontend's existing `EffectViolation` path does not thread blame
today, which is an existing cross-compiler difference this change does not
address — see "Deferred").

The check is a single AST walker
(`vow-types/src/effects.rs::collect_may_write_sites`,
`compiler/checker.vow::collect_may_write_sites_in_expr`) that:

- Flags an `Assign` whose left-hand side is a `FieldAccess` or `Index` —
  a write to a local binding (`x = 5`) is not flagged, since it cannot
  affect caller-visible state.
- Flags a builtin method call whose method is not on a small read-only
  allowlist (`len`, `get`, `eq`, `contains`, `contains_key`, `byte_at`,
  `substring`, `parse_i64`, `parse_u64`, `unwrap`) — an **allowlist**, not a
  denylist of known-mutating methods, so a method added later to `Vec`,
  `String`, `HashMap`, or `BTreeMap` is write-suspect by default rather than
  silently permitted.
- Flags a call to a function whose own may-write bit is set, computed as a
  monotone fixed point over the module's call graph (direct write → `true`;
  otherwise `true` if any callee is `true`; iterated to a fixed point, so
  mutual recursion with one writing member taints the whole cycle). An
  unresolvable callee name fails closed (treated as a write) rather than
  being silently assumed pure. A **declaration** (`fn f(..) -> T;`, no body —
  the `.vow.d` stub form `use` can load instead of a module's real source,
  see `tests/multi/decl_stub_preference/`) also fails closed: its parsed
  "body" is an empty placeholder, so analyzing it the way an ordinary
  function is analyzed would find no writes and silently treat an opaque,
  no-implementation-in-this-translation-unit function as pure. Declarations
  are seeded to `true` directly rather than run through the fixed point.
- Searches *through* control flow (`if`, `match`, loops) rather than
  rejecting it — a clause may still contain a `for`-loop or `match` as long
  as nothing reachable inside it writes.

This generalizes ADR 0002's own validation (built for `A_r`/`R_r`
specifically) to every clause on every function, closing the gap the issue
identified without waiting for condition/restart syntax to land.

## Why not ADR 0002's rule verbatim

ADR 0002 rejects *any* call to a user-defined function inside `A_r`/`R_r`,
not just a writing one. Applied wholesale to ordinary contracts, that rule
is too broad: it would reject `compiler/lexer.vow`'s and
`compiler/lower.vow`'s own `ensures`/`requires` helpers (`is_alpha`,
`is_valid_binop`, …) and `tests/multi/bignum_legacy/bignum.vow`'s
`bignum_cmp_abs`/`bignum_is_zero`, all of which only read through their
parameters — breaking the self-hosted compiler's own bootstrap-verified
contracts. Restart contracts are a narrower, not-yet-implemented surface
where the stricter rule may still be the right call when they land; this ADR
does not revisit that choice for `A_r`/`R_r`.

## Considered alternatives

- **Include postcondition-evaluation writes in the normal-outcome transition
  and footprint summary** (issue #1032's option 2) — i.e. build the `W_f`
  write-footprint machinery ADR 0002 anticipates and let a clause write, as
  long as the footprint accounts for it. Rejected: no such footprint
  mechanism exists for ordinary (non-restart) calls today, building one
  would be a large new verification-surface addition for a problem that a
  much smaller type-check-time rejection already solves, and it does not
  remove the deeper issue that a "pure predicate" would still be live code
  with a real effect.
- **Reuse `vow_ir::RegionSummary::store_effects`** (the Phase-3 arena/lifetime
  write-effect summary) as the "may write" signal. Rejected: it is an
  IR-level artifact computed during region inference, which runs after type
  checking (where `check_vow_purity` runs) and is scoped to arena/lifetime
  propagation, not general heap-write detection; depending on it would
  couple contract purity to Phase-3 internals that can change independently,
  with no self-hosted analog.
- **Real escape/alias analysis**, to permit a helper that mutates a struct
  provably never read back by the caller. Rejected in favor of
  over-approximation (reject some theoretically-safe programs) to keep the
  check a local, dedicated AST pass with near-zero verifier impact, matching
  this project's "surface sugar only when it desugars to today's core
  semantics with near-zero verifier impact" design principle.
- **A denylist of known-mutating builtin methods** instead of an allowlist of
  known-read-only ones. Rejected: a denylist silently admits a future
  mutating method nobody remembered to add to it; the allowlist fails closed
  instead.

## Deferred

- **Self-hosted blame.** The self-hosted checker's `EffectViolation`
  diagnostics (old and new) do not set `Blame::Callee` the way the Rust
  frontend's do; `env_emit_error_code` has no blamed variant. Pre-existing,
  not introduced by this change; not fixed here to keep this a surgical,
  behavior-preserving-elsewhere change.
- **`.unwrap()` purity.** `check_vow_purity` already collects panic
  expressions inside a clause and does not diagnose them. Pre-existing,
  unrelated gap; tracked separately, not bundled here.
- **Catalogue-generated method allowlist.** Once the Operation Catalogue
  (`docs/spec/operations.json`) covers container methods (issues
  #1271-#1275), the read-only allowlist could be generated rather than
  hand-maintained in two compilers. Not attempted here.
