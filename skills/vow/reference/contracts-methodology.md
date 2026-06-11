# Contract Methodology: What to Verify

This document answers a question that `contracts.md` does not: given a function,
**which** properties are worth proving, how do you tell a strong contract from a
hollow one, and how do you express the strong shapes within ESBMC's reach.

`contracts.md` is the *reference* (syntax, blame, the verification pipeline,
anti-patterns). This is the *methodology* (judgement). Read that first.

## The core principle: strength, not volume

A proven contract is worth nothing if it would also hold for an incorrect
implementation. The number of contracts a codebase proves is not a quality
signal — the *discriminating power* of each contract is.

This is measurable. Polikarpova, Furia, Pei, Wei, and Meyer (the originator of
Design by Contract), *"What Good Are Strong Specifications?"* (ICSE 2013), found
that testing implementations against **strong** specifications — comprehensive
functional pre/post/invariants — detected roughly **twice as many bugs** as
testing against standard/weak contracts. Their conclusion: *"the quality of
specifications limits the value of verification."*

A concrete Vow example of the trap. Many tag constants in the self-hosted
compiler carry this contract:

```vow
fn EFF_IO() -> i64 vow { ensures: result > 0 } { 1 }
```

ESBMC proves `1 > 0` in milliseconds. But `ensures: result > 0` also holds for
`{ 2 }`, `{ 99 }`, and every other positive constant — it does not pin the value
this function is *supposed* to return. It is a real postcondition, but a **weak**
one: it constrains the output to a half-line, not to a point. Proving 354 of
these does not make the compiler more correct; it makes the verification report
longer.

The fix is not "delete contracts" — it is "make each contract say something only
the correct implementation satisfies."

## A taxonomy of contract shapes

Each shape below lists its *intent*, *when it applies*, a *real Vow example*, and
*strength notes*. The expressibility/verifiability status of every shape is
collected in the matrix at the end.

### 1. Domain precondition (range / validity bound)

**Intent:** restrict the inputs the function promises to handle correctly. Blame
falls on the caller (`requires`).

**When:** the function is only correct on a subset of its parameter types — a byte
in `0..=255`, a non-zero divisor, an in-bounds index.

```vow
fn write_u8(out: Vec<i64>, v: i64) vow {
    requires: v >= 0,
    requires: v <= 255
} { out.push(v); }
```

**Strength:** a precondition is strong when it is the *true* domain of the
function — no wider (which would admit miscompilation) and no narrower (a
verifier-driven bound like `requires: n <= 8`, forbidden by `contracts.md`).
A bounds-check precondition such as `requires: i >= 0, requires: i < v.len()` is
the standard guard for every indexing operation.

### 2. Output-range postcondition (the weak default — use sparingly)

**Intent:** constrain the result to a range.

**When:** the range *is* the full functional spec — e.g. a function whose only
guarantee is non-negativity. This is rare. Most uses are the weak trap above.

```vow
fn item_kind(v: i64) -> i64 vow {
    requires: v >= 0,
    ensures: result >= 0          // weak: any non-negative value satisfies this
} { v / 4294967296 }
```

**Strength:** weak by default. Reach for shape 3, 4, or 5 instead whenever the
function actually computes a *specific* value. If you find yourself writing
`ensures: result >= 0` on a function that returns a computed quantity, ask what
the result *equals* or *inverts*, and assert that.

### 3. Exact functional postcondition (equality)

**Intent:** pin the result to the value the function is defined to produce.

**When:** the output is a closed-form function of the inputs (arithmetic,
bit-packing, encodings).

```vow
fn region_pack(kind: i64, val: i64) -> i64 vow {
    requires: kind >= 0,
    requires: kind <= 3,
    requires: val >= 0,
    requires: val <= 4294967295,
    ensures: result == val * 4 + kind     // exact: only the right answer passes
} { val * 4 + kind }
```

**Strength:** strong — a wrong shift or offset is caught immediately. Note the
preconditions are not verifier appeasement: they bound `val` and `kind` so the
packed result cannot overflow `i64` (a genuine semantic constraint). Contrast
`region_pack` (exact) with `item_pack`/`item_kind` (shape 2, only `>= 0`): the
same bit-packing pattern, one specified strongly and one weakly.

### 4. Round-trip / inverse

**Intent:** prove that an encode/decode (or pack/unpack, serialize/deserialize)
pair compose to the identity on the valid domain.

**When:** two functions are defined as inverses — `pack`/`unpack`,
`encode`/`decode`, `to_bytes`/`from_bytes`.

Specify each direction with an exact closed-form postcondition — shape 3 applied
to the extractor as well as the encoder, so the decoder is pinned to the exact
arithmetic that inverts the pack:

```vow
fn region_kind(r: i64) -> i64 vow {
    requires: r >= 0,
    ensures: result == r - (r / 4) * 4,   // exact extractor
    ensures: result <= 3
} { r - (r / 4) * 4 }
```

Because both directions are pinned to closed forms — `region_pack`'s exact
`ensures: result == val * 4 + kind` (shape 3 above) and the matching
`region_kind`/`region_val` extractors — a `region_pack` then
`region_kind`/`region_val` round-trip recovers `(kind, val)` exactly, and ESBMC
discharges that composition with no separate assertion. The inverse can also be
asserted directly: Vow allows pure-function calls in postconditions, so an
`ensures: region_kind(result) == kind` on `region_pack` is expressible and
modelable when the partner is pure (matrix shape 4). **Strength:** very strong —
round-trip is the property a serialization layer must have, and it catches the
entire class of "encoder and decoder drifted apart" bugs that output-range
contracts miss completely.

### 5. Dispatch totality (fail-closed decoders)

**Intent:** prove that a decoder/dispatcher maps **every** valid input to a
defined output and **never** silently falls through to a default.

**When:** a function switches over a tag/opcode/discriminator. This is the
single highest-value shape for Vow, because silent-fallback normalization
(mapping an unknown tag to a valid-looking default) is the bug class issue #81
was filed over.

The pattern has two halves — a validity precondition and an explicit error
sentinel for the unreachable tail:

```vow
fn is_valid_binop(op: i64) -> bool { op >= 0 && op <= 22 }

fn binop_opcode(op: i64, operand_ty: i64) -> i64 vow {
    requires: is_valid_binop(op)
} {
    if op == BINOP_ADD() { return ...; }
    // ... one arm per valid op ...
    -1                                    // unreachable under the precondition
}
```

**Strength — and a live hardening gap.** The precondition pins the domain, but
this contract does **not yet prove totality**: nothing asserts the function never
returns the `-1` sentinel. The strong form adds a postcondition that rules out
the fallthrough:

```vow
fn binop_opcode(op: i64, operand_ty: i64) -> i64 vow {
    requires: is_valid_binop(op),
    ensures: result != -1                 // proves every valid op is handled
} { ... }
```

With `ensures: result != -1`, ESBMC must show that on every `op` in `0..=22` some
arm returns before the sentinel — i.e. the dispatch is exhaustive. If an agent
later adds opcode 23 to `is_valid_binop` but forgets the matching arm,
verification fails instead of miscompiling. This is the contract that converts a
silent fallback into a caught error.

> Vow has no surface quantifier (`forall i in 0..n`) today, so "covers all valid
> inputs" is expressed as `requires` (pin the finite domain) + a postcondition
> that excludes the failure value, letting ESBMC enumerate the finite branch
> structure. Bounded quantifiers are tracked as a roadmap item (#467/#470).

### 6. Relational / cross-function (uniqueness, agreement)

**Intent:** state a property that spans more than one function or more than one
argument.

**When:** tags in a family must be distinct; two collections must have equal
length; a result must relate two inputs.

The argument-relational form is directly expressible:

```vow
fn build_pairs(ids: Vec<i64>, names: Vec<i64>) -> Vec<Pair> vow {
    requires: ids.len() == names.len()
} { ... }
```

The *cross-function uniqueness* form — "`tok_kw_fn() != tok_kw_let()` for every
pair in the family" — is expressible only as O(n²) pairwise inequalities, which
does not scale to dozens of tag constants. **The better fix is structural, not
contractual:** encode each family as a base+offset range or a generated table so
uniqueness is a property of the *encoding* rather than something every function
must restate. Treat a wall of zero-argument tag constants as an API smell that
shape-6 contracts cannot economically repair.

### 7. Loop invariant / frame

Covered in `contracts.md` (counter bounds, search-range invariants, the
inductiveness requirement). The methodology point: an invariant is strong when it
is the property the loop *maintains toward its postcondition*, not merely
`i >= 0`. See `contracts.md` §Loop Invariants and the worked CEGIS cycle in
`examples.md`.

## Hollow contracts: three failure modes to detect

A contract can pass verification while proving nothing. There are three distinct
ways this happens; a contract-quality tool should distinguish them.

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
(#81). For any function carrying a `requires`, it re-runs ESBMC over the same
model with a `vow_reach` label planted immediately after the `requires` assumes,
under `--error-label vow_reach`. If ESBMC reports the label **unreachable**
(`VERIFICATION SUCCESSFUL`), the conjoined preconditions are contradictory and
every `ensures` held only vacuously — all of the function's clauses are reported
`status: "vacuous"` and the command fails closed. If the label is **reachable**
(`VERIFICATION FAILED`), the precondition domain is non-empty and the proof is
live. This is operationally the dual of the classic `ensures: false` re-check —
asking "is the post-`requires` point reachable?" instead of "does `assert(false)`
still pass?" — but it needs only one extra run per function and is unaffected by
body divergence, since the label precedes the body. The label sits after the
requires prefix rather than at the function end precisely so an unbounded loop or
an `assume(0)` deeper in the body cannot make it spuriously unreachable.

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
- **modelable** — the C emitter / ESBMC model supports the operations used
  (pure, non-effectful helpers only; see `is_modelable` in the C emitter)
- **backend** — ESBMC actually discharges it within bounds

| Shape | expressible | typechecked | lowerable | modelable | backend |
|-------|:-----------:|:-----------:|:---------:|:---------:|:-------:|
| 1. Domain precondition | ✓ | ✓ | ✓ | ✓ | ✓ |
| 2. Output-range postcond. | ✓ | ✓ | ✓ | ✓ | ✓ (but weak) |
| 3. Exact equality | ✓ | ✓ | ✓ | ✓ | ✓ within overflow bounds |
| 4. Round-trip / inverse | ✓ | ✓ | ✓ | ✓ if partner is pure & modelable | ✓ for arithmetic |
| 5. Dispatch totality | ✓ | ✓ | ✓ | ✓ (pure dispatch) | ✓ over finite domain |
| 6a. Argument-relational | ✓ | ✓ | ✓ | ✓ | ✓ |
| 6b. Cross-fn uniqueness | ✓ (O(n²)) | ✓ | ✓ | ✓ | ✓ but unscalable → prefer structural encoding |
| 7. Loop invariant | ✓ | ✓ | ✓ | ✓ | partial (bounded / k-induction) |
| Bounded quantifier (`forall i in 0..n`) | ✗ (no surface syntax) | — | — | — | — (roadmap #467/#470) |

Contract expressions must be **pure** — they cannot call effectful functions
(`grammar.md` §Contract Purity). A property that needs an effectful helper is
blocked at the *modelable* axis, not the expressible one; classify such gaps as
model limitations, not contract-language limitations.

## Tooling

`vow contracts --verify` performs the **per-obligation** quality analysis tracked
in #81 / roadmap WS-3.2. Each clause gets an individual verification verdict (via
ESBMC `--multi-property`), plus the three quality signals above:

- **Tautology** — the static `quality` field flags constant clauses (no ESBMC).
- **Vacuity** — a contradictory `requires` is caught by the `--error-label`
  reachability probe and reported `status: "vacuous"` (fail-closed).
- **Weakness** — the body-replace probe reports `trivially_satisfiable: true` for
  an `ensures` a trivial `return <default>` body satisfies (informational).

The `summary` carries `vacuous` and `trivially_satisfiable` counts alongside the
status and quality tallies, so an author or CI can gate on hollow proofs.

## References

- N. Polikarpova, C. A. Furia, Y. Pei, Y. Wei, B. Meyer. *What Good Are Strong
  Specifications?* ICSE 2013. https://arxiv.org/abs/1208.3337
- I. Beer, S. Ben-David, C. Eisner, Y. Rodeh. *Efficient Detection of Vacuity in
  Temporal Model Checking.* Formal Methods in System Design 18:141–163, 2001.
- M. Dwyer, G. Avrunin, J. Corbett. *Property Specification Patterns for
  Finite-State Verification.* ICSE 1999.
- B. Meyer. *Object-Oriented Software Construction* (Design by Contract).
