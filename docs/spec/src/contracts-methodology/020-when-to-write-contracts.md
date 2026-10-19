
### Weakness

The clause is satisfiable and true, but so loose that an incorrect
implementation also satisfies it (`ensures: result >= 0` on a computed value).
This is the 354-contract problem.

**Detection (the body-replace probe).** `vow contracts --verify` ships this check
(#81). It mutates the implementation in the strongest possible way — replaces the
whole body with a trivial `return <type-default>` — and re-verifies the
`ensures`. If the contract still proves against that body, a constant-returning
implementation satisfies it, so it does not constrain the real computation: each
such `ensures` is reported `trivially_satisfiable: true`. This is exactly the
`body-replace` mutation of `vowc mutants` with ESBMC as the oracle.

The signal is **one-sided (sound, not complete)**: a `true` verdict is a proof of
weakness (a specific trivial body satisfies the contract), but a `false` verdict
does not prove strength — the probe uses a single default value and skips
non-scalar returns, returned parameters, and φ-merged/branchy results, so it can
miss weak contracts it cannot witness this way. It is informational and never
changes the exit code; pair it with the static `quality` field. The one known
false positive is a function whose correct result genuinely *is* the type default
(e.g. a constant `ensures result == 0` on a `{ 0 }` body) — an equivalent mutant,
the standard caveat of mutation testing.

### Tautology

The clause is true independent of the program — `ensures: true`,
`ensures: result == result`, `ensures: x >= 0 || x < 0`. **Detection:** the clause
is valid (provable) with the function body removed; a cheap check folds constant
clauses and flags any clause with no dependence on parameters or `result`.

### Vacuity (antecedent failure)

The clause is proved only because its **preconditions are unsatisfiable**, so the
path it guards is dead and the postcondition never has to hold. Because Vow
lowers `requires` to `__ESBMC_assume`, a contradictory or over-strong precondition
makes *any* `ensures` provable — an assume-false / dead-path proof.

This is the classic vacuity of Beer, Ben-David, Eisner, and Rodeh, *"Efficient
Detection of Vacuity in Temporal Model Checking"* (Formal Methods in System
Design, 2001): a subformula is vacuous when replacing it changes nothing about
the result. Their industrial data is the reason to take it seriously — across
years of hardware verification at IBM, ~20% of formulas were trivially valid on
first runs, and trivial validity *always* indicated a real defect in the design,
spec, or environment.

**Detection (the reachability probe).** `vow contracts --verify` ships this check
(#81). For any function carrying a `requires`, it re-checks the same model with
a reachability probe immediately after the `requires` assumes. Under ESBMC the
probe is a `vow_reach` label run under `--error-label vow_reach`; under
`--backend native` the function is cut right after its last `requires` and the
cut point must be unreachable. If the probe reports the point **unreachable**
(ESBMC: `VERIFICATION SUCCESSFUL`), the conjoined preconditions are contradictory and
every `ensures` held only vacuously — all of the function's clauses are reported
`status: "vacuous"` and the command fails closed. If the point is **reachable**
(ESBMC: `VERIFICATION FAILED`), the precondition domain is non-empty and the proof is
live. This is operationally the dual of the classic `ensures: false` re-check —
asking "is the post-`requires` point reachable?" instead of "does `assert(false)`
still pass?" — but it needs only one extra run per function and is unaffected by
body divergence, since the label precedes the body. The label sits after the
requires prefix rather than at the function end precisely so an unbounded loop or
an `assume(0)` deeper in the body cannot make it spuriously unreachable.

Under `--backend native` the probe is not a label but a rewrite of the same IR
that the native engine checks: the function is cut right after its last
`requires` and that point must be unreachable, so `&&`/`||` requires (which
lower to branches) are covered. The `trivially_satisfiable` probe is a rewrite
too. See "Native backend: `contracts` and `test`" in `cli.md`.

**Interesting witnesses.** Beer et al. also propose the dual of a counterexample:
for a proof that holds, emit a non-trivial *witness* — concrete inputs that
exercise the property for a substantive reason — so the author can confirm the
proof is not hollow. Vow's structured output is well-suited to carrying a witness
alongside each `Verified` result.

## When to write contracts

### Builtins and `extern` blocks

Runtime functions (`Vec.push`, `String.from`, `HashMap.insert`) are implemented in
Rust/C and cannot be verified by ESBMC. Their behavior enters verification through
the `vow` contract on the `extern "C"` block, which becomes an **assumed**
(`__ESBMC_assume`) surface for callers. Because these contracts are *assumed, not
checked*, they are the most dangerous place for an error: a wrong `ensures` on an
extern block silently weakens every proof that depends on it. Extern contracts
must be reviewed as assumptions, audited against the runtime implementation, and
kept minimal. (Omitting the block is a `MissingContract` error — see `errors.md`.)

### Library functions (written in Vow)

Public Vow functions are fully within verification reach. Give each one its true
domain precondition and the strongest postcondition shape that applies (3–6, not
2). Add contracts when the function's contract is *known*, which is usually at
definition time for pure utilities and after the signature stabilizes for APIs.

### Application code (including agent-generated)

Vow's target author is an AI agent, and the failure mode to design against is
*volume over substance* — an agent emitting many `ensures: result >= 0` clauses
because the prompt said "add contracts." Skill guidance should push the opposite:
for each function, identify which shape applies (equality, round-trip, dispatch
totality, relational) and write that one; prefer one discriminating contract to
five weak ones. The Specification Pattern System of Dwyer, Avrunin, and Corbett
(ICSE 1999) — a survey-validated catalog built specifically to turn imprecise
intent into precise specifications — is the model for guiding an author from "this
should be valid" to a postcondition that says what *valid* means.

## Expressibility and verifiability matrix

Whether a shape is usable depends on five independent axes, not just "can the
syntax say it":

- **expressible** — surface Vow has the syntax
- **typechecked** — the checker validates the clause to `bool` (added in #81 Phase 0)
- **lowerable** — the lowerer emits IR for the clause
- **modelable** — the verifier's model supports the operations used
  (pure, non-effectful helpers only; see `is_modelable` in the C emitter)
- **backend** — the verifier actually discharges it (ESBMC: within bounds; native: for every unwinding it can close, otherwise `unknown`)

| Shape | expressible | typechecked | lowerable | modelable | backend |
|-------|:-----------:|:-----------:|:---------:|:---------:|:-------:|
| 1. Domain precondition | ✓ | ✓ | ✓ | ✓ | ✓ |
| 2. Output-range postcond. | ✓ | ✓ | ✓ | ✓ | ✓ (but weak) |
| 3. Exact equality | ✓ | ✓ | ✓ | ✓ | ✓ within overflow bounds |
| 4. Round-trip / inverse | ✓ | ✓ | ✓ | ✓ if partner is pure & modelable | ✓ for arithmetic |
| 5. Dispatch totality | ✓ | ✓ | ✓ | ✓ (pure dispatch) | ✓ over finite domain |
| 6a. Argument-relational | ✓ | ✓ | ✓ | ✓ | ✓ |
| 6b. Cross-fn uniqueness | ✓ (O(n²)) | ✓ | ✓ | ✓ | ✓ but unscalable → prefer structural encoding |
| 7. Loop invariant | ✓ | ✓ | ✓ | ✓ | partial (bounded; native also by induction) |
| Bounded quantifier (`forall i in 0..n`) | ✗ (no surface syntax) | — | — | — | — (roadmap #467/#470) |

Contract expressions must be **pure** — they cannot call effectful functions
(`grammar.md` §Contract Purity). A property that needs an effectful helper is
blocked at the *modelable* axis, not the expressible one; classify such gaps as
model limitations, not contract-language limitations.

## Tooling

`vow contracts --verify` performs the **per-obligation** quality analysis tracked
in #81 / roadmap WS-3.2. Each clause gets an individual verification verdict (via
ESBMC `--multi-property`, or from one `verify-worker` per function under
`--backend native`), plus the three quality signals above:

- **Tautology** — the static `quality` field flags constant clauses (no verifier is run).
- **Vacuity** — a contradictory `requires` is caught by the reachability probe
  (ESBMC `--error-label`, or the post-`requires` cut under `--backend native`) and
  reported `status: "vacuous"` (fail-closed).
- **Weakness** — the body-replace probe reports `trivially_satisfiable: true` for
  an `ensures` a trivial `return <default>` body satisfies (informational).

The `summary` carries `vacuous` and `trivially_satisfiable` counts alongside the
status and quality tallies, so an author or CI can gate on hollow proofs.

**CI weak-gate.** `scripts/check_contract_quality.py` ratchets on the static
quality of the self-hosted compiler's own contracts: it fails if the `weak` or
`tautological` count exceeds a committed baseline, so a new `ensures result >= 0`
cannot slip in unnoticed. It runs in `scripts/full_test.sh`. The baseline is an
upper bound to ratchet down as contracts harden.

`vow contracts` follows `use` edges, so an entry point covers its own module
graph and nothing outside it. The gate therefore runs once per entry point,
with `--label` so a breach names which one regressed:

| Entry point              | Why it is gated |
|--------------------------|-----------------|
| `compiler/main.vow`      | the compiler's own `use` graph |
| `compiler/module_io.vow` | deliberately-unwired `.vmod` parity infrastructure, **not** in `main.vow`'s `use` graph — it had zero coverage until it was listed here, and it must not drift |

`tests/`, `stdlib/`, `examples/` and `benchmarks/` are deliberately **not**
gated. They are the corpus — the *input* to contract cleanup — and they
legitimately carry weak and tautological clauses today; a baseline aggregated
over hundreds of files is a number nobody can ratchet down meaningfully.
Corpus-side cleanup is enforced instead by the hard
[`TautologicalComparison`](errors.md#tautologicalcomparison) type error, which
fails the build per site rather than counting, plus reviewer discipline.

The ratchet runs the self-hosted compiler only, so `scripts/full_test.sh` also
carries a `contract-quality/parity` case comparing the `(function, kind,
quality)` triples both compilers derive from
`tests/fixtures/contracts/quality_shapes.vow`. Without it the Rust classifier
has no end-to-end coverage and the two can drift. The dispatch-totality example above
(`binop_opcode`, `ensures: result != -1`) and `binop_result_ty`
(`ensures: result == ITY_BOOL() || result == ITY_U64() || result == ITY_I64()`)
are enforced in `compiler/lower.vow` today.

**Tag families are structural, not contracted.** The bulk of the old `weak`
count was nullary tag constants — `fn IOP_VOW_REQ() -> i64 { 73 }`, the `ITY_*`,
`EXPR_*`, `BINOP_*`, `RSUM_KIND_*`, … enum families. A per-constant `ensures
result >= 0` proves nothing: a constant's value is the only fact about it, and
that fact is structural (each is a distinct literal). So these carry **no**
contract. Their correctness is established where it matters — at use sites: the
dispatch-totality contracts above prove every valid tag is handled, the IR
validator and serializer round-trips exercise every kind, and the binary
fixed-point bootstrap miscompiles if any two tags collide. Removing the
contracts cut the compiler's `weak` count from 408 to 11; the remaining bit-packers — the
region/span packers and friends (`region_pack`/`region_kind`/`region_val`,
`span_pack`, `item_kind`, `marker_caller_store`, `suffix_len`) — were then hardened
with exact functional
postconditions: `item_kind` with `result == v / 4294967296`, and `suffix_len` with a
per-suffix conditional mapping (`(suffix != tok_suffix_i64() || result == 3) && …`, one
conjunct per suffix plus an unknown-suffix → `0` clause), bringing `weak` to **0** (#81).
The CI weak-gate now holds that baseline: no weak
contract may enter the self-hosted compiler.

## References

- N. Polikarpova, C. A. Furia, Y. Pei, Y. Wei, B. Meyer. *What Good Are Strong
  Specifications?* ICSE 2013. https://arxiv.org/abs/1208.3337
- I. Beer, S. Ben-David, C. Eisner, Y. Rodeh. *Efficient Detection of Vacuity in
  Temporal Model Checking.* Formal Methods in System Design 18:141–163, 2001.
- M. Dwyer, G. Avrunin, J. Corbett. *Property Specification Patterns for
  Finite-State Verification.* ICSE 1999.
- B. Meyer. *Object-Oriented Software Construction* (Design by Contract).
