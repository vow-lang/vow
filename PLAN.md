# Plan: issue #1032 — contract clauses can write shared state without that write appearing anywhere release code runs

## 1. Problem restated

A `requires`/`ensures`/`invariant` expression may call a plain, zero-declared-effect
user-defined function (or a builtin container method) that performs a field or
indexed write through one of its arguments — Vow passes structs, `Vec`, `String`,
and maps by pointer, so an ordinary helper can mutate caller-visible heap state
without declaring `io`/`read`/`write`/`panic`/`unsafe` (those effects cover
filesystem/stdio/panic/FFI only, never heap mutation through a parameter).
`check_vow_purity` (`vow-types/src/effects.rs`) and its self-hosted counterpart
`check_clause_purity` (`compiler/checker.vow`) only reject calls to callees with a
*declared* effect, so a call like `ensures: mark(p)` where `mark(p: Point) -> bool
{ p.x = 1; true }` is accepted today. Contract clauses are evaluated for real in
`--mode debug` and during ESBMC verification (struct-field writes, i.e.
`FieldSet`, are inside the verifier's modelable instruction subset — confirmed at
`vow-verify/src/c_emitter.rs:755`, they are not skipped), but never evaluated at
all in release builds (contracts are stripped entirely). So `mark`'s write to
`p.x` really happens whenever the contract is checked or verified, and never
happens in the binary users actually ship — any later code that depends on
`p.x == 1` because "the contract said so" sees a value that was only ever true
under instrumentation. This is a real, demonstrable debug/verify-vs-release
behavioral divergence today, independent of ADR 0002 and independent of whether
a per-function write-footprint ("W_f") mechanism ever gets built — grepping this
checkout finds no `havoc`/`write_footprint` machinery for ordinary calls, and
ADR 0002 itself never names `W_f`; that term is the issue's own shorthand, not
code in this repo.

## 2. Files to touch

**Rust compiler (type-checking stage, `vow-types`):**
- `vow-types/src/effects.rs` — add one unified "may-write" walker (see §3
  slice 1) and extend `check_vow_purity` to reject what it finds, both in a
  clause's own expression and transitively through called functions.
  `collect_calls_in_expr`'s `MethodCall` arm currently recurses into the
  receiver/args but never records the method call itself, so `v.push(1)`
  inside a clause is invisible to the purity check today — fixed by the same
  walker, not a separate pass (an earlier draft of this plan split direct
  writes, method calls, and transitive writes into three independent checks;
  that under-counts a helper whose *own* write is a method call rather than a
  field assignment — see §3 slice 1 for why they must be one walker).
- A new per-module side table, computed once before `check_fn_effects`/
  `check_vow_purity` run: `BTreeMap<String, bool>` from function name to
  "may write heap state reachable from its parameters." **Not** a new field on
  `FnSig` — `FnSig { .. }` is constructed in 13 places across `vow-types`
  (builtin signature tables, tests); a side table avoids touching all of them.
- New small constant table (co-located in `effects.rs`, or a new
  `vow-types/src/builtin_mutability.rs` if it grows large): the read-only-builtin
  **allowlist** used by the method-call check (see risk notes, §5).
- Whatever currently calls `check_fn_effects` per top-level `FnDef` — must also
  reach into every nested `while ... vow { invariant: ... }` loop, which it
  does not today (see §3 slice 1a — this is a pre-existing traversal gap, not
  new to this fix, but the new check is inert on invariants until it's closed).

**Self-hosted compiler (`compiler/`), same session:**
- `compiler/checker.vow` — mirror the new unified walker in
  `check_clause_purity` / `check_vow_clause`, and the same loop-invariant
  traversal fix.
- `compiler/env.vow` — add the parallel may-write side table (same
  `Vec<i64>`-bitmask-style convention as the existing `fn_effects`, keyed by
  function index rather than name to match existing self-hosted idiom).
- Wherever `collect_calls_in_expr` lives for the self-hosted AST walker — same
  method-call recording fix.

Vow has no user-defined methods — `MethodCall` targets are always a builtin
type's method (`Vec`, `String`, `HashMap`, `BTreeMap`, `Option`, …; confirmed
by grepping `vow-syntax`/`docs/spec/grammar.md` for an `impl`-block or
user-method construct and finding none) — so the method-call arm never needs
to resolve a callee's own may-write bit the way a `Call` to a user function
does; it only needs a name lookup against the allowlist.

**Docs (required by CLAUDE.md's "any change to semantics/effects must update
docs/spec" rule):**
- `docs/spec/grammar.md` — §"Contract Purity" (around line 1072-1074): state
  that a clause also may not call a function/method that writes through any of
  its arguments, even if it declares no effect; add the `mark(p)` counter-example.
- `docs/spec/contracts.md` — short new subsection cross-referencing the above,
  stated in terms of blame (`Blame::Callee`, reusing the existing
  `EffectViolation` diagnostic) and the debug/release divergence rationale.
- `docs/spec/errors.md` — update the `EffectViolation` entry's description to
  cover this new trigger (no new `ErrorCode` variant — see §5).
- After editing `grammar.md`, re-run `uv run python scripts/generate_help.py`
  and rebuild both compilers (`cargo build --release -p vow`,
  `scripts/bootstrap.sh --skip-cargo`) — `scripts/check_help_coverage.py`
  (run by `full_test.sh`) fails if `--help`/the embedded skill drift from
  `grammar.md`.
- New `docs/adr/0004-contract-heap-write-purity.md` — the "own design pass" the
  issue explicitly asks for, scoped to ordinary contracts on every function (not
  just restart-capable ones), explicitly *not* adopting ADR 0002's blanket
  "reject every user-defined call" rule, with the alternatives considered (§6 of
  this plan maps directly onto that ADR's "Considered alternatives" section).
  Add it to `docs/adr/README.md`'s index too.

**Tests:** see §3 for exact new/changed test files.

## 3. TDD slices

**Slice 0 — characterize the reproducer (implementation-stage only, not merged
as a permanent fixture unless useful as a regression test).**
Bootstrap (`scripts/bootstrap.sh`, ~5 min) was not run during planning — no
`target/release/vow` or `build/vowc` exists in this checkout yet. Before writing
the fix, the implementation agent should build three throwaway `.vow` files
under `$TMPDIR` and run each through `vowc verify`, `--mode debug`, and release:
  (a) a struct-field-write helper called from `ensures`,
  (b) a `Vec` indexed-write helper (goes through `__vow_vec_set_val`, confirmed
      in `is_known_builtin`) called from `ensures`,
  (c) a direct mutating builtin-method call inside `ensures` (e.g. `v.push(1)`).
Record whether ESBMC accepts (false proof) or whether `is_modelable` instead
skips verification (`VerificationSkipped`) for each, and whether `--mode debug`
vs release output actually diverges. This doesn't change the fix (all three must
be rejected regardless), but it fixes the exact wording of the issue close-out
comment and the ADR's problem statement with empirical evidence instead of
inference from reading `c_emitter.rs`.

**Slice 1 — one walker, direct writes detected both ways (Rust).**
A helper's own write can be a field/index `Assign` *or* a non-allowlisted
method call (`v.push(1)` is as much a direct write as `p.x = 1` — they must
share one detector, not two independent checks that could individually miss a
case the other catches).
- Red (`vow-types/src/effects.rs` test module, using the existing
  `make_fn`/`call_expr`-style fixture builders):
  1. `fn mark(p: Point) -> bool { p.x = 1; true }` called from
     `ensures: mark(p)` → one `EffectViolation`, `Blame::Callee`.
  2. `fn mark(v: Vec<i64>) -> bool { v.push(1); true }` called from
     `ensures: mark(v)` → same.
  3. `ensures: { p.x = 1; true }` (the write written directly in the clause,
     no helper at all) → same. Clauses can be block expressions with
     statements today (`tests/fixtures/complexity/predicate_control_flow.vow`
     has a `for`-loop inside a `requires:` clause), so this must be checked
     too, not just calls reached through `collect_calls_in_expr`.
  4. Confirm pre-existing passing uses keep passing: `.len()`/`.is_empty()`/
     `.contains_key()` inside `ensures`
     (`tests/run/vec_reverse_descending.vow`, `tests/run/string_build.vow`,
     `tests/run/string_empty.vow`,
     `benchmarks/medium/M08_map_insert_lookup/reference.vow`,
     `benchmarks/medium/M09_map_update/reference.vow`).
- Green: one new walker (mirrors `collect_calls_in_block`/
  `collect_calls_in_expr`'s traversal shape, so it stays in sync with any
  future `ExprKind` variant) that, over a given expression or block, reports
  every: (a) `Assign` whose `lhs` is `FieldAccess`/`Index`; (b) `MethodCall`
  whose method is not in the read-only-builtin allowlist (§5); (c) `Call`
  whose callee name is not resolvable in the module (fails closed — treated
  as a write) or whose callee direct-write bit (from this same walker applied
  to the callee's body) is set. Run it in two places: directly over each
  clause's own expression (catches case 3), and once per function body to
  populate the side table consumed by the fixed point in slice 2 (catches
  cases 1-2 once slice 2 adds transitivity).
  `collect_calls_in_expr`'s `Call` arm only matches `ExprKind::Ident(name)`
  callees — confirm during implementation whether any module-qualified or
  otherwise non-`Ident` call form reaches a clause; if so it must also fail
  closed (treated as a write) rather than being silently skipped the way
  `MethodCall` is today.

**Slice 1a — reach loop invariants at all (pre-existing traversal gap, Rust).**
Today `check_fn_effects` only calls `check_vow_purity` on `fn_def.vow` — the
*function-level* vow block. `ExprKind::While` carries its own `vow` field
(loop invariants) and nothing walks into it for purity purposes; the existing
declared-effects purity check is silently inert on `while c vow { invariant:
mark(p) } { .. }` today, and the new write check would be too unless this is
fixed first.
- Red: `while c vow { invariant: mark(p) } { .. }` with the same `mark` from
  slice 1 → must emit `EffectViolation`. Verify this is red even against
  today's pre-fix code for the *existing* declared-effect check (e.g.
  `invariant: read_file(p) > 0`), to confirm the gap predates this issue.
- Green: have the body walker that finds `ExprKind::While` also call
  `check_vow_purity` on its `vow` block, recursively (loops can nest).

**Slice 2 — transitive writes (helper A calls helper B which writes).**
- Red: `inner(p)` writes `p.x = 1`; `mark(p)` calls `inner(p)` and returns
  `true`; `ensures: mark(p)` must still be rejected, even though `mark`'s own
  body has no direct write.
- Green: build the per-function side table as a monotone fixed point:
  initialize each function's bit from slice 1's direct-write walker applied to
  its own body, then iterate `may_write(f) |= OR over f's callees' may_write`
  until no change. Iterate functions and call edges in a deterministic order
  (`BTreeMap`/sorted `Vec`, not `HashMap`) — see §5. Handles (mutual) recursion
  safely by construction (monotone over a 2-element lattice, finite function
  set, so it terminates and a cycle with one writing member correctly taints
  the whole cycle).

**Slice 3 — self-hosted parity (`compiler/checker.vow`, `compiler/env.vow`).**
- Port slices 1, 1a, and 2 verbatim: the unified walker, the loop-invariant
  traversal fix, and the fixed point, keyed by function index in a
  `Vec<i64>` paralleling `fn_effects`. Add
  `tests/error/contract_write_through_helper.vow` (new fixture, mirrors the
  issue's own `mark(p)` example) and run it through *both* compilers —
  confirm in `tests/run_tests.sh`/`scripts/full_test.sh` which harness(es)
  already exercise `tests/error/` for each compiler, and add self-hosted
  coverage if only the Rust side is wired today.

**Slice 4 — regression safety net for existing legitimate read-only
contract-helper patterns.**
This is the highest-value test slice: it must stay green, or the fix is too
broad. Already hand-checked during planning that none of these write anything
(all are scalar comparisons or field/`.len()`/index *reads*, confirmed by
reading their bodies):
  `compiler/lexer.vow` (`is_alpha`, `is_digit` called from `ensures`),
  `compiler/lower.vow` (`is_valid_binop`, `is_binop_result_ty`),
  `tests/multi/bignum_legacy/bignum.vow` (`bignum_cmp_abs`, `bignum_is_zero`,
  reads `.digits`/`.sign`/`.len()`/index),
  `tests/fixtures/complexity/callee_free_vars.vow` (`id`),
  `tests/fixtures/complexity/predicate_control_flow.vow` (`pair_ordered`,
  `bucket_nonneg`, plus the `for`-loop-inside-`requires`/`match`-inside-
  `requires` patterns in the same file — these must keep parsing and checking
  as read-only even though they contain loop/match control flow, since the
  walker must look *through* control flow for writes, not reject control flow
  itself).
Still must be re-confirmed empirically once the fix lands (hand-reading a body
is not a substitute for running the checker on it) — run `cargo test --all`
and the self-hosted `scripts/full_test.sh` in full before calling this done. A
false positive here breaks the self-hosted compiler's own bootstrap-verified
contracts: a release-blocking regression, not a test failure.

**Slice 5 — codecov/patch coverage for every new branch.**
The repo's `codecov/patch` gate blocks at 95% and `.vow` fixtures run
uninstrumented (per project memory), so each new branch needs its own `cargo`
unit test, not just end-to-end `.vow` fixtures: the fixed-point loop
(slice 2), the method-call arm and the direct-clause-block arm (slice 1), the
unresolvable/extern-callee fail-closed arm (slice 1), and the loop-invariant
traversal path (slice 1a).

## 4. Verification surface

No new ESBMC properties, no C-emitter changes, no new verification conditions —
this fix runs entirely in the type-checking stage (`vow-types`/`checker.vow`),
strictly before IR lowering, so it never touches the verifier. It *shrinks* the
set of programs the verifier ever sees (previously-modelable-but-unsound
programs are now rejected at type-check time instead of reaching ESBMC). No
`tests/run/` or `examples/` fixtures need to grow the provable surface; the new
fixtures (§3 slices 1 and 3) are negative (`tests/error/`) by construction.

## 5. Risk areas

- **Allowlist vs. denylist for builtin methods.** Use an explicit allowlist of
  known-read-only methods (`len`, `is_empty`, `get`, `contains`, `contains_key`,
  etc., enumerated from `docs/spec/grammar.md`'s method tables), not a denylist
  of known-mutating ones. A denylist silently permits any method the author
  forgot to list (e.g. a newly-added mutating method ships unguarded); an
  allowlist fails closed. This is the single highest-leverage correctness
  decision in the whole change — get it wrong and the fix has a soundness hole
  shaped exactly like the one it's closing.
- **False positives on `compiler/*.vow`'s own contracts.** If the may-write
  fixed point ever misclassifies a read (e.g. treats reading a field through a
  heap-typed param as a write), it breaks `compiler/lexer.vow` /
  `compiler/lower.vow`'s own verified contracts and therefore
  `scripts/bootstrap.sh`'s verification step — covered by slice 4, but calling
  out explicitly because the failure mode is "the self-hosted compiler can no
  longer bootstrap," not just "a test goes red."
- **Determinism / binary fixed point.** The new `may_write` fixed-point
  iteration must visit functions and call edges in a stable order (`BTreeMap`
  or a sorted `Vec` indexed by a stable function id, not `HashMap` iteration
  order) in both compilers, consistent with the existing `BTreeMap`-for-
  `slot_map` precedent in `vow-clif-shim`. Diagnostic emission order depends on
  it, and any snapshot/golden test over diagnostic JSON could otherwise flap
  between bootstrap stages A/B/C.
- **Rust/self-hosted parity.** The two fixed-point implementations (Rust
  `BTreeMap<String, bool>`-based vs. self-hosted `Vec<i64>` bitmask-based) must
  agree on every fixture; run both compilers against the same `tests/error/`
  fixtures (slice 3).
- **`cargo clippy --all -- -D warnings`.** New Rust code should follow
  `effects.rs`'s existing plain-recursive-match style; no new abstractions.
- **Not affected:** `parse → print → parse` idempotency (purely semantic check,
  no AST/printer change), codegen ordering, `vow-clif-shim` stack-slot layout —
  none of this touches lowering or codegen.

## 6. Out of scope (deliberately not bundled)

- **ADR 0002's blanket "ban every user-defined call in a contract clause" rule,
  applied wholesale to ordinary contracts.** Verified by grep and by reading
  the called functions' own bodies that this would break `compiler/lexer.vow`,
  `compiler/lower.vow`, and `tests/multi/bignum_legacy/bignum.vow`'s own
  read-only predicate-helper calls — too broad, rejected as a candidate in the
  new ADR, not attempted here.
- **Reusing `vow_ir::RegionSummary::store_effects`** (the Phase-3 arena/lifetime
  inference summary) as an existing "may-write" signal. It is real and already
  computed, but it is an IR-level, post-region-inference artifact built for a
  different purpose (arena ownership/lifetime propagation, not general heap-
  write detection), it isn't available at the point `check_vow_purity` runs
  today (type-checking happens before lowering and before region inference),
  and depending on it would couple contract-purity semantics to Phase-3 arena
  internals that can change independently, with unclear self-hosted parity.
  Considered and rejected in favor of a small, dedicated AST-level analysis
  that lives next to the existing purity check it extends.
- **`.unwrap()`/panic-expression purity gap.** `check_vow_purity` already
  collects `panic_exprs` and silently drops them without emitting any
  diagnostic for panic-inside-a-clause — a real, pre-existing, but unrelated
  gap. File as a separate follow-up issue; do not bundle into this fix.
- **Builtin-mutability-list maintenance via the Operation Catalogue.**
  `docs/spec/operations.json` currently only covers `print_*` (per CLAUDE.md,
  epic #375's tracer bullet). Once it's broadened (issues #1271-#1275), the
  read-only-method allowlist could become catalogue-generated instead of
  hand-maintained in two compilers. Not attempted here; note as a follow-up.
- **Real escape/alias analysis** to narrow false positives (e.g. permitting a
  helper that mutates a struct which provably never escapes the helper). The
  deliberate tradeoff here is over-approximation: reject some theoretically-safe
  programs in exchange for zero transitive-write/escape-analysis machinery,
  matching the "near-zero verifier impact" design principle and ADR 0002's own
  stated preference for local opcode inspection over escape analysis.
- **Condition/restart implementation itself.** Still fully unimplemented; this
  fix only generalizes the *principle* ADR 0002 already committed to for
  restarts, it does not implement any restart syntax or runtime.
