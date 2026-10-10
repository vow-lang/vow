# The native verifier's term simplifier and claim elision are in the trusted base

**Status:** accepted (2026-10-10)

Resolves #1419. Part of epic #1398.

## Context

The native verifier (`compiler/vc_*.vow`) turned every IR value into a
`define-fun`, never recognised a constant or a repeated condition, and sent one
solver query per claim, even a claim whose goal was literally `true`. Unrolling
a loop to bound 32 therefore produced hundreds of definitions and dozens of
solver spawns for a function whose every value is a compile-time constant.

## Decision

1. **The term arena is hash-consed** (`vc_term.vow`): an open-addressing table
   over the interned term ids, with each node's hash cached, so equal terms are
   one id. Variables are never interned.
2. **Every constructor simplifies before interning**: constant folding for
   widths up to 64 bits (`vc_bvfold.vow`, SMT-LIB semantics, total division and
   shifts), Bool laws, and identities that return an existing operand. Only
   rules valid at every sort the operands can have fire: the arena stores no
   sort for variables or applications, so a rule may use a width only when a
   bit-vector constant carries it. 128-bit operands are folded only by
   width-independent rules.
3. **Only composite terms are named.** A leaf is used as it is; a composite gets
   a `define-fun` the first time its term is built, and every later value with
   the same term shares the name. A name also stands for its definition, so a
   rule sees through it (`x or g` with `g := not x`).
4. **Decided claims are not asked.** A claim whose negated query folds to
   `false`, or whose `pc => goal` is already an assumption, is `unsat` under any
   assumptions and is not recorded. A soft claim (unwinding, checked arithmetic)
   whose negated query is the constant `true` with an empty assumption prefix is
   `sat` without a solver: the header holds only declarations and definitions.
   Hard claims always reach the solver, which supplies the counterexample.

## Trust

This moves a rewriter into the trusted base: a wrong rule would silently turn a
false contract into `proven`. The guards are tests, not proofs:

- `compiler/tests/test_vc_bvfold.vow` checks every fold helper over all width-8
  operand pairs and edge values at 16/32/64 bits against an independent
  `i128`/`u128` reference written from the SMT-LIB definitions.
- `compiler/tests/test_vc_term.vow` builds random term DAGs twice, as raw nodes
  and through the simplifying constructors, and compares both under random
  environments with an independent evaluator.
- `tests/verify-native/fail/` keeps the counterexample direction: a contradicted
  postcondition that folds to `false` must still reach the solver.

Deliberately not folded: 128-bit operations other than equality, `bvmul`
overflow predicates at widths between 33 and 63 bits, and unsigned 64-bit
division by a divisor above 2^63 when the dividend is not a constant.

## Cost

Per term the arena now holds seven parallel words (`kinds a b c aux hashes
named assumed`, nine with the table slot) instead of five. Terms are bounded by
the unrolled instruction cap, so the worst case stays far below the verify
worker's address-space cap, and deduplication lowers the count.
