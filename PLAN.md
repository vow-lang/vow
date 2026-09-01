# Plan — issue #1087: self-hosted miscompiles a loop-body `let` reading a reassigned loop-carried variable

Branch: `sym/vow/1087-self-hosted-miscompiles-loop-body-let-reading-a-reassigned-loop-carried-variable-gcd-returns-0`

## 1. Problem restated

`build/vowc` miscompiles any loop whose back-edge writes a loop-carried variable
that another back-edge write reads in the same group — the textbook Euclid swap
(`let tmp = y; y = x % y; x = tmp;`), which returns `0` for every input. The IR
produced by `compiler/lower.vow` is correct Pizlo SSA: the body ends with a set of
`Upsilon` instructions (`Upsilon(phi_y ← rem)`, `Upsilon(phi_x ← phi_y)`) whose
Pizlo semantics are a **simultaneous** copy — each `Upsilon` writes the target
`Phi`'s *shadow*, and the `Phi` reads that shadow only when control re-enters its
block. `vow-clif-shim` instead lowers `Upsilon` as an immediate `stack_store` into
the `Phi`'s own value slot and lowers `Phi` as a no-op, so the copies execute
**sequentially**: `Upsilon(phi_y ← rem)` clobbers `phi_y`'s slot, and the next
instruction, `Upsilon(phi_x ← phi_y)`, reads the already-overwritten slot. This is
the classic lost-copy/swap defect of naive phi elimination. `collect_assigned_vars`
in `compiler/lower.vow:1601` emits the back-edge `Upsilon`s in source-assignment
order, so `b = 0; a = tmp;` yields exactly the hostile order and `a` becomes `0`.
The Rust backend (`vow-codegen/src/cranelift_backend.rs:1427-1434`) routes
`Phi`/`Upsilon` through Cranelift block params, which Cranelift resolves as a real
parallel copy, and both C emitters (`vow-verify/src/c_emitter.rs:2472`,
`compiler/c_emitter.vow:2860`) already stage every `Upsilon` source into a
`__ups_<src>` temp before writing any target — so all three other implementations
are correct and only the Cranelift shim is wrong. That also explains why
`benchmarks/medium/M13_gcd` **verifies** cleanly while **executing** wrong: the
proof path and the execution path disagree.

The issue body's suspicion (`lctx_assign` in-place mutation in `lower.vow`) is not
the cause: `lctx_assign(ctx, "b", …)` rewrites the scope entry for `b`, never the
separate entry for `tmp`, so `tmp` correctly keeps the header `Phi`'s id. The
issue's own delta table corroborates the shim diagnosis — `let tmp = 7` agrees
(no `Upsilon` reads a `Phi`), and `a = b` without a `let` agrees (the conflicting
pair collapses).

## 2. Files to touch

**Fix (Rust, the only broken implementation):**

- `vow-clif-shim/src/lib.rs` — the whole change. Five sites:
  - `~1849-1866` — the `cross_block_refs` scan; also collect the block's `Phi` ids.
  - `~1868-1884` — the `slot_map` allocation loop; add a parallel shadow-slot map.
  - `~1894-1904` — the stack-slot zero-init loop; extend to shadow slots.
  - `~2418` `IOP_PHI => {}` — becomes the shadow → value-slot copy.
  - `~2419-2432` `IOP_UPSILON` — store to the shadow slot, not the `Phi`'s slot.
  - Helpers `load_slotted_value` / `store_slotted_value` at `~3052-3080` stay as is.

**Tests:**

- `vow-clif-shim/src/lib.rs` (`mod tests`, from `:3914`) — new hand-built-IR
  execution tests, using the existing `add_test_block` / `add_test_inst` /
  `declare_test_function` / `link_float_phi_test_object` harness that already
  compiles, links with `cc`, and runs a real binary.
- `tests/run/loop_body_let_from_carried_var.vow` — **remove** the
  `// TEST: known-divergence 1087 …` line (fixture already landed via #1086).
- `tests/run/euclid_gcd_swap_loop.vow` — same.

**Docs / bookkeeping:**

- `docs/equivalence/ledger.json` — flip the three `issue: 1087` corpus entries
  (`benchmarks/medium/M13_gcd/reference.vow`, `tests/run/euclid_gcd_swap_loop.vow`,
  `tests/run/loop_body_let_from_carried_var.vow`) from `"status": "open"` to
  `"status": "fixed"`, rewrite each `note` to past tense naming the root cause, and
  bump the top-level `"updated"` date. `status: "fixed"` entries are retained by
  design (see the `tests/error/undefined_function.vow` precedent) so a
  reappearance reads as a regression.
- `CLAUDE.md` — the `vow-clif-shim architecture` section currently states only
  "The shim uses stack slots instead of SSA values for all instruction results."
  Add one or two sentences recording that each `Phi` owns **two** slots (shadow +
  value), that `Upsilon` writes the shadow and `Phi` copies shadow → value, and why
  (back-edge `Upsilon`s are a parallel copy). Two lines, no restructuring.

**No `docs/spec/*.md` change is required.** This is a backend miscompile, not a
change to syntax, semantics, types, builtins, operators, effects, or CLI flags.
`docs/spec/` already describes the intended behaviour; the compiler was wrong.

**No `compiler/*.vow` change is required.** CLAUDE.md's "modify BOTH compilers"
rule targets language-semantics changes that exist twice. Here the defect lives in
a single shared Rust crate that only the self-hosted backend links; `compiler/
lower.vow` emits correct IR, `compiler/clif.vow` only marshals it across the FFI,
and `compiler/c_emitter.vow` already implements the parallel copy. Adding a
self-hosted mirror of a Rust-only defect would be inventing work.

## 3. TDD slices

Each slice is a separate commit on this branch. Slices 1-3 need only
`cargo test -p vow-clif-shim` (seconds); the expensive bootstrap is slice 4.

### Slice 1 — RED: pin the ordering hazard at the shim level

*Test:* `vow-clif-shim/src/lib.rs`, `mod tests`, new
`back_edge_upsilons_are_a_parallel_copy_not_a_sequential_one`.

Build, by hand with `add_test_block`/`add_test_inst`, a four-block i64 function
that is the exact shape `lower.vow` emits for the 9-line reproducer:

- entry: `const 0` (a-init), `const 1` (b-init), `Upsilon(phi_a ← a-init)`,
  `Upsilon(phi_b ← b-init)`, `Jump header`
- header: `Phi phi_a`, `Phi phi_b`, `const 0`, `GtI64(phi_b, 0)`,
  `Upsilon(exit_a ← phi_a)`, `Upsilon(exit_b ← phi_b)`, `Branch(body, exit)`
- body: `const 0`, **`Upsilon(phi_b ← const0)` first, then
  `Upsilon(phi_a ← phi_b)`** — the hostile order — then `Jump header`
- exit: `Phi exit_a`, `Phi exit_b`, `Return exit_a`

`main` calls it, compares against `1`, and returns the comparison, mirroring
`assert_cross_block_float_phi_runtime_value` (`:4222`). Assert the linked binary
exits `0`.

*Expected:* FAILS today, returning `0` — mechanically confirming the diagnosis
without needing a bootstrap. Landing this red test first is the whole point: it
proves the defect is in the shim and not in `lower.vow`, which the issue body
guessed differently.

*Production code:* none.

### Slice 2 — GREEN: shadow slots for every `Phi`

*Production code:* `vow-clif-shim/src/lib.rs`.

1. In the `cross_block_refs` scan, also collect `phi_ids: BTreeSet<i64>` for
   `inst_ops[ii] == IOP_PHI`. **`BTreeSet`/`BTreeMap` only** — `HashMap` iteration
   order would break deterministic codegen and therefore the binary fixed point
   (the existing `slot_map` is a `BTreeMap` for exactly this reason).
2. Allocate `phi_shadow_slots: BTreeMap<i64, StackSlot>`, one per phi id, using the
   same `(16, 4)` / `(8, 3)` sizing rule as `slot_map` so a 128-bit `Phi` does not
   lose its high limb. Allocate in a deterministic order (iterate `phi_ids`, or
   filter the existing `inst_ids[..n_insts]` walk).
3. Extend the zero-init loop to zero shadow slots too (offset `0`, plus offset `8`
   for wide slots), preserving today's "uninitialized cross-block refs read as 0"
   behaviour.
4. `IOP_UPSILON` (`~2419`): store into `phi_shadow_slots[&phi_id]` instead of
   `slot_map[&phi_id]`. Keep the existing guard shape — an `Upsilon` whose
   `dv` was never backpatched (`lower.vow` uses the `2147483647` sentinel) still
   finds no entry and stays a no-op, exactly as today.
5. `IOP_PHI` (`~2418`): copy shadow → value slot. Use a **raw bit copy**, not the
   type-aware helper pair:
   ```rust
   let lo = builder.ins().stack_load(types::I64, types::I64, shadow, 0);
   builder.ins().stack_store(types::I64, lo, value_slot, 0);
   if wide { /* same at offset 8 */ }
   ```
   `store_slotted_value` sign-extends `I16`/`I32` and zero-extends `I8` while
   `load_slotted_value` `ireduce`s them, so routing the copy through that pair
   would add a second extend/reduce round trip and risk changing the raw slot bits
   for `u32`/`bool` operands. A raw copy leaves the value slot bit-identical to what
   the `Upsilon` wrote, so the *only* behaviour change in this PR is the copy's
   **timing** — which is the bug.

*Why this design and not the alternative:* the narrower fix — detect a contiguous
run of `Upsilon`s and stage every source into an SSA value before storing any
target, mirroring the two C emitters — also fixes the reported case, because
`lower.vow` always emits back-edge `Upsilon`s as one group before the terminator.
But it is a fix conditioned on an emission-ordering property that nothing enforces:
one future `lower.vow` change that interleaves an `Upsilon` with ordinary code
silently reintroduces the miscompile. Shadow slots implement the IR's documented
Pizlo semantics directly and are correct for any `Upsilon` placement, at the cost
of one extra slot and one load/store per `Phi` execution. Take the robust one.

*Expected:* slice 1 goes green.

*Placement precondition to re-check while implementing:* the copy happens at the
`Phi`'s own position, which is correct only if no `Upsilon` targeting that same
`Phi` precedes it in the same block. Verified for every construct `lower.vow`
emits — `while`/`loop`/`for` headers open with their `Phi`s
(`compiler/lower.vow:2782`), `if`/`else` merge blocks open with theirs (`:2705`),
`match` merge blocks likewise (`:4383`), and loop-exit `Phi`s are pre-emitted at
the top of the exit block (`:2803`). If implementation turns up a counterexample,
hoist the copies to block entry instead and say so in the commit message.

### Slice 3 — REFACTOR/HARDEN: pin true parallel-copy semantics

*Test:* `vow-clif-shim/src/lib.rs`, two more cases:

- `back_edge_upsilons_swap_two_loop_carried_values` — a genuine 2-cycle
  (`Upsilon(phi_a ← phi_b)`, `Upsilon(phi_b ← phi_a)`), which no ordering of
  sequential copies can satisfy. Guards the property, not just the reported order.
- `wide_phi_shadow_slot_preserves_both_limbs` — the same hostile order on an
  `ITY_I128` `Phi`, asserting the high limb survives. Guards the `(16, 4)` sizing
  in slice 2 step 2, which is easy to get wrong and which #1166/#1168 show this
  codebase has already been bitten by.

*Production code:* none expected; if the wide case fails, the sizing/offset-8 copy
is the fix.

### Slice 4 — De-quarantine the tier-1 fixtures and the ledger

*Preconditions:* `cargo build --release -p vow` then `scripts/bootstrap.sh
--skip-cargo` (there is no `target/` and no `build/vowc` in this workspace, so a
cold build is required; budget for it). Confirm `build/vowc` reaches the byte-
identical fixed point.

*Changes:*

- Delete the `// TEST: known-divergence 1087 "…"` line from both
  `tests/run/loop_body_let_from_carried_var.vow` and
  `tests/run/euclid_gcd_swap_loop.vow`. Keep the explanatory comment blocks —
  they document why the fixtures exist. `scripts/full_test.sh:169` **hard-fails**
  a fixture that still carries the directive once the two compilers agree, so
  this must land in the same PR as the fix; conversely, removing it turns
  `compare_runtime` into a real gate on both fixtures.
- Update the three `issue: 1087` entries in `docs/equivalence/ledger.json` to
  `"status": "fixed"` with past-tense notes naming the shim's sequential
  `Upsilon` lowering, and bump `"updated"`.
- Add the two-sentence shadow-slot note to CLAUDE.md's `vow-clif-shim
  architecture` section.

*Verification:* `python3 scripts/test_equivalence.py` (validates the ledger
against `docs/equivalence/ledger.schema.json`) and `python3 scripts/test_parity.py`.

### Slice 5 — Full gate

Run each as its own command, never `&&`-chained:

1. `cargo fmt --all -- --check`
2. `cargo clippy --all -- -D warnings` (note: no `--all-targets`; CI's gate
   excludes test targets, so match CI exactly and do not chase lints CI never runs)
3. `cargo test --all`
4. `scripts/bootstrap.sh --skip-cargo` — the binary fixed point
5. `scripts/full_test.sh` — Sections 4 and 4b must be green with the directives
   gone; sections 1-3 catch collateral damage
6. Spot check the real-world case:
   `build/vowc build --no-verify benchmarks/medium/M13_gcd/reference.vow` and run
   it; also `build/vowc verify benchmarks/medium/M13_gcd/reference.vow` to confirm
   the proof path is unchanged
7. `python3 scripts/equivalence.py` over at least `tests/` and
   `benchmarks/medium/` — it reports `NO LONGER DIVERGING` for stale ledger
   entries, so it double-checks slice 4's bookkeeping

If any run test regresses under the self-hosted compiler after the shim change,
that is a real signal: some fixture was silently depending on the sequential-copy
behaviour. Investigate rather than reverting.

PR title (this is the squash subject — Conventional Commits, lower-case, no
trailing period, ≤92 chars before ` (#N)`):
`fix(clif-shim): give each Phi a shadow slot so back-edge Upsilons copy in parallel`

## 4. Verification surface

No contract is written, weakened, or bounded by this change, and no ESBMC
property changes. The C model is emitted by `vow-verify/src/c_emitter.rs` and
`compiler/c_emitter.vow`, both of which already stage `Upsilon` sources into
`__ups_<src>` temps before writing targets — i.e. the proof path already had the
correct parallel-copy semantics and is untouched here. That asymmetry is itself
the interesting fact for the issue: `M13_gcd` proved correct while executing
wrong, so the fix moves execution into agreement with a proof that was already
right. Re-running `build/vowc verify benchmarks/medium/M13_gcd/reference.vow`
after the fix (slice 5 step 6) confirms the proof path did not move.

Test fixtures under `tests/run/` do not need to grow: #1086 already landed both
the minimal and the Euclid-level fixture. What they need is the quarantine
directive removed so they become blocking (slice 4). `examples/` needs nothing.

## 5. Risk areas

- **Binary fixed point.** Adding stack slots changes every frame layout that
  contains a `Phi`, so `build/vowc`'s bytes will change — expected. What must hold
  is `sha256(compiler_b) == sha256(compiler_c)`, and it will, because stage 1 and
  stage 2 link the same rebuilt shim. The live hazard is **nondeterministic slot
  allocation order**: use `BTreeSet`/`BTreeMap` for the phi id set and the shadow
  map, never `HashSet`/`HashMap`. Slice 5 step 4 is the gate.
- **Stack frame growth.** One extra 8-byte (16 for i128) slot per `Phi`. Functions
  in `compiler/*.vow` with many loops grow their frames. Watch for stack-guard
  interaction (`__vow_init_stack_guard`) during bootstrap; if a deeply recursive
  compiler function starts overflowing, that is the signal, not a mysterious crash.
- **Wide (i128/u128) `Phi`s.** The high limb is lost if the shadow slot is sized 8
  or the copy omits offset 8. Slice 3's wide test exists specifically for this;
  #1166 and #1168 are recent evidence this class of slip is live in this codebase.
- **Type round-tripping.** Routing the shadow → value copy through
  `load_slotted_value`/`store_slotted_value` adds an extend/reduce round trip for
  `I8`/`I16`/`I32` operands. Use the raw i64 (plus high-limb) copy to keep the
  value slot bit-identical to what the `Upsilon` wrote.
- **`Phi` placement assumption.** See the precondition note in slice 2. Cheap to
  re-verify, expensive to get wrong.
- **Vow-binding capture.** `IOP_VOW_*` reads `slot_map[binding.inst_id]` directly
  for captured values. For a `Phi` binding this now reads the value written at the
  `Phi`, not the last `Upsilon` — which is the correct value, but it does change
  what a debug-mode `VowViolation` reports inside a loop. `scripts/full_test.sh`
  compares counterexample values across compilers, so a change here surfaces as a
  parity failure rather than silently.
- **`cargo clippy --all -- -D warnings`.** The new code is small; the likely lints
  are `too_many_arguments` on a widened helper (the file already uses
  `#[allow(clippy::too_many_arguments)]`) and needless borrows in the new
  `BTreeMap` lookups. CI does not pass `--all-targets`, so lints that only fire in
  test modules are not this PR's problem.
- **`parse → print → parse` idempotency.** Not touched — no lexer, parser, AST, or
  printer change. Listed only to record that it was considered.

## 6. Out of scope

- **Rewriting `collect_assigned_vars` ordering** (`compiler/lower.vow:1601`). The
  hostile `Upsilon` order it produces is legal Pizlo IR; making the order friendly
  would hide the backend defect rather than fix it, and would not help the genuine
  2-cycle swap of slice 3.
- **Any `compiler/*.vow` change.** The self-hosted lowerer and C emitter are both
  correct here. See §2.
- **Migrating the shim off stack slots to real SSA / block params.** The stack-slot
  design exists because the self-hosted IR has cross-block references between
  sibling branches that violate Cranelift dominance (CLAUDE.md, `lib.rs:1844-1848`).
  Converging the two backends is a real and worthwhile project; it is not a
  bug fix and must not ride along.
- **Extending `scripts/full_test.sh` to sweep `benchmarks/`.** The issue's "why
  every existing guard missed it" section is right that this is a coverage gap, but
  it is coverage work with its own runtime budget question. File it separately;
  `scripts/equivalence.py` (#1081) already covers the corpus at tier 2.
- **Auditing other `IOP_*` handlers in the shim for similar timing bugs.** Worth
  doing; not in a fix PR. If slice 5 turns up a second miscompile, file it rather
  than widening this change.
- **Reformatting, comment cleanups, or import reordering** anywhere in
  `vow-clif-shim/src/lib.rs` outside the five sites named in §2.

## Follow-ups to file (do not bundle)

1. Sweep `benchmarks/` differentially in `full_test.sh` or in a nightly job — the
   coverage gap that let this bug live for the whole life of the benchmark suite.
2. Audit the remaining shim opcode handlers for slot-timing hazards of the same
   family.
3. Evaluate retiring the shim's stack-slot model in favour of block params, now
   that the region/routing work in `clif.vow` has matured.
