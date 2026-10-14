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

Per term the arena now holds eight parallel words (`kinds a b c aux hashes
named assumed`, plus two to four table slots) instead of five. Terms are bounded by
the unrolled instruction cap, so the worst case stays far below the verify
worker's address-space cap, and deduplication lowers the count.

## Amendment (#1560, 2026-10-11)

One rule joins the simplifier: `bvsub x (bvmul (bvsdiv x d) d)` is `bvsrem x d`,
and the `bvudiv`/`bvurem` pair likewise, with the multiplication in either
order. Operands are compared through bound names, so the idiom `x - (x / m) * m`
matches although each intermediate value is a bound variable by the time the
subtraction is built.

The decision is unchanged and the rule is valid at every sort: SMT-LIB division
and remainder are total, so the identity holds for a zero divisor and for the
signed minimum over -1 as well. Division-by-zero and overflow aborts are
separate claims built from the operands, not from the rewritten term, so they
are unaffected.

The reason it cannot be left to the solver is that Bitwuzla decides the signed
identity only at width 8: the division-multiplication-subtraction chain over 16
to 64 bits times out, while the same query over `bvsrem` is answered in
milliseconds. The guard is `check_divrem_exhaustive` in
`compiler/tests/test_vc_term.vow`, which compares the raw and simplified terms
over all 65 536 width-8 operand pairs for both signednesses and multiply orders,
plus the random-DAG differential, which builds the idiom with matching and
near-miss divisors. A bound extension node shares operation codes with the
arithmetic nodes, so a rule that follows a name must re-check the node kind.

Not folded, tracked as follow-ups: the dual `(x / d) * d + x % d` to `x`, bounds
such as `x / d * d <= x`, and an interval domain for `bvmul`/`bvsdiv`.

## Amendment (#1682, 2026-10-14)

A second rule, for the other idiom that makes the solver diverge: a
multiplication-overflow guard written with a division. `v <= c / d` and its
mirror `c / d >= v`, and the negations `v > c / d` and `c / d < v`, with `c` a
constant (non-negative for the signed `bvsdiv`; any value for `bvudiv`) and `v`
and `d` non-constant terms, become

    ite(d > 0, inner, original)

where `inner` is `not umulo(v, d) and v * d <= c` unsigned and
`smulo(v, d) ? v < 0 : v * d <= c` signed, negated for the `>` and `<` forms,
and `original` is the comparison as it was built. For `d > 0` and `c >= 0`,
`v <= floor(c / d)` holds exactly when the unbounded product `v * d` is at most
`c`, and an overflowing product has the sign of `v`. For a zero or negative
divisor the else branch keeps the SMT-LIB meaning of the division, so the rule
holds at every divisor with no side condition. It uses only the width carried by
`c` and no 128-bit term, so it applies at every width up to 64 bits.

The reason it cannot be left to the solver: `requires: w >= 0, h >= 0,
h == 0 || w <= MAX / h` with `ensures: w * h >= 0` over 64-bit values is
answered by ESBMC in about 5 s but ran for more than 300 s in Bitwuzla (a
`bvsdiv` by a variable feeding the `bvmul` it guards). The `u32` version of the
same shape also timed out. With the rewrite the queries are decided in
milliseconds. The solver options (`--bv-solver`, `--abstraction`) did not help.

The guard is `check_quotient_cmp_exhaustive` in
`compiler/tests/test_vc_term.vow`: for both signednesses, all eight comparison
forms and constants at the edges (0, 1, 5, 127, 128, 255) it compares the raw
and simplified terms over all 65 536 width-8 operand pairs and checks that
exactly the four covered forms (and non-negative signed constants) are
rewritten. The same check runs at width 16 over constants at the edges (0, 1,
5, 300, 32767, 32768, 65535) and an 18-value operand sample (the edges, the
sign boundary and mid-range values), so a width-dependent slip is not hidden
by the 8-bit domain; that part is sampled, not exhaustive. `check_quotient_rules` pins the shapes that must not match (a
constant `v`, a variable or wide or negative dividend, the other signedness).

Not folded: the strict `v < c / d` and `v >= c / d`, which are not overflow
guards, and a guard whose dividend is not a constant (`a * b / b == a`).
