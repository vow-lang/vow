# Contract Methodology: What to Verify

This document answers a question that `contracts.md` does not: given a function,
**which** properties are worth proving, how do you tell a strong contract from a
hollow one, and how do you express the strong shapes within the verifier's reach.

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

The verifier proves `1 > 0` in milliseconds. But `ensures: result > 0` also holds for
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
A bounds-check precondition such as `requires: i < v.len()` (with `i: u64`, so no
lower-bound clause is needed) is the standard guard for every indexing operation.

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

The static classifier rates this clause `substantive`, and `vow contracts --verify`'s
body-replace probe reports it `trivially_satisfiable: true` — both are correct, because
they measure different things. The probe replaces the body with `return 0` (the `i64`
default); `0 != -1` holds, so by the definition in **Weakness** (below) this is a *true*
positive: `ensures: result != -1` does not constrain the op→opcode *mapping* — a constant
non-sentinel body (`return 5`) satisfies it for every valid `op`. What the clause *does*
prove is dispatch **totality**: every valid `op` reaches an arm before the `-1` fallthrough
(delete an arm and verification fails). Totality is the silent-fallback property #81
targets, and — absent a quantifier to say "result is the correct opcode for `op`" — it is
the strongest property a `!= sentinel` postcondition can express. So read the
`trivially_satisfiable: true` as accurate (the clause pins totality, not the mapping), not
as a probe artifact to dismiss. This is *not* the constant-result false positive noted in
**Weakness**: `binop_opcode`'s correct result varies per `op`, so it is not genuinely the
type default.

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
