
### Collection Models for Verification

**Native backend.** A `Vec<T>` whose element `T` is `bool` or an integer of up to 64 bits (`new`, `push`, `pop`, `clear`, `truncate`, `len`, indexing, indexed assignment) is an array plus a 64-bit symbolic length. There is no capacity: a property over every length is proven for every length, an index at or past the length is its own obligation (`index out of bounds`), and no `ModelCapacityAssumed` note is ever emitted. `String` is modelled the same way (see `docs/spec/cli.md` → "Native backend", "Strings"). Elements of 128 bits or of a float type, `sort`, `from_raw_parts_copy` and `pin_to_root` are modelled too, and so is a pointer element (`String`, `Vec`, struct) that is pushed or stored; a pointer element read back and the `Vec` operation `reserve` is `Skipped` (see `docs/spec/cli.md` → "Native backend", "Collections"). The rest of this section describes the ESBMC backend.

**ESBMC backend.** ESBMC is a *bounded* model checker, so it models collection types as
fixed-size arrays and reasons about them up to a finite capacity. These
capacities are an internal property of the verifier, not of the language:

| Type              | Model Capacity | Supported Operations |
|-------------------|----------------|----------------------------------------------|
| `Vec<T>`          | 128            | `new`, `push`, `pop`, `len`, `truncate`, indexing (`v[i]`) |
| `String`          | 256            | `from`, `len`, `push_byte`, `push_str`, `byte_at`, `matches_literal_at` |
| `HashMap<K, V>`   | 64             | `new`, `insert`, `get`, `contains_key`, `len`|
| `BTreeMap<K, V>`  | 64             | `new`, `insert`, `get`, `contains_key`, `len`|
| User structs (heap) | 1024 slots   | construction (`RegionAlloc`), field reads and writes |

The numbers are defaults. The effective `String` capacity is raised to the
longest string literal in the module (see below), and the
[`ModelCapacityAssumed`](errors.md#modelcapacityassumed) note reports the
capacity that was actually applied.

**These bounds are not a language feature and are not user-tunable.** A `Vec`
in a Vow program grows dynamically on the heap with no fixed maximum; the
capacity above only describes how far the *bounded* model checker reasons. The
language and its contracts are deliberately decoupled from what any particular
prover can prove: replace ESBMC with a stronger (or unbounded) checker, as the native backend does for `Vec`, and the
same source, the same contracts, and the same CLI keep working — the only
difference is that the verifier can cover more of the state space. For this
reason a `requires`/`ensures` clause must never encode a verifier bound (e.g.
`requires: v.len() <= 128`); see "Verification-Driven Bounds (Anti-Pattern)"
below and `docs/design/verifier-model-bounds.md`.

These models support the same operations as the runtime but with bounded
storage. String literals carry their concrete length and bytes in verification,
and `String::from` copies that model from its source value. The effective string
model capacity is automatically at least the longest static string literal, so
literal byte initializers always fit the model array. Operations whose bytes are
not statically known, such as `String::from_cstr`, produce a nondeterministic
length (0 to max-1). `string_matches_literal_at` is modeled against the
literal's concrete bytes and byte length; the third argument must be a string
literal so the verifier never has to infer static text from a dynamic `String`.
A length passed to `String::from_raw_parts_copy` or
`Vec::from_raw_parts_copy` that is *provably constant* and does not fit the
model capacity fails closed with the capacity-limit diagnostic rather than being
assumed away. "Provably constant" covers a literal, `+`/`-`/`*` (wrapping or
checked) over constants, an integer cast of a constant, and a `let mut` whose
every assignment is the same constant; it is computed over the IR, so
`n + 300` with a constant `n` is recognised exactly like `301`.

`from_raw_parts_copy` models the runtime's null-pointer behaviour: a null source
(`ptr == 0`) yields an empty value whatever the length is, and the capacity
restriction applies only to a non-null source. `ensures: result.len() == n`
therefore needs a real `requires: p != 0`, exactly as the runtime does. A
source that is provably the null constant (a literal `0`, or a phi of them) is
empty with no assumption at all. `String::new` shares the same model: it is a
null source with a zero length, and a constant length outside the capacity
fails closed for a non-null source just like `from_raw_parts_copy`.

**A proof is bounded when the model had to assume a capacity.** Any *non-constant*
collection length — a `Vec`/`String`/`HashMap`/`BTreeMap` parameter, a
collection read from a struct field, `String::from_cstr`, a non-constant
`from_raw_parts_copy` length — is modelled as nondeterministic but restricted to
the capacity above (`__ESBMC_assume(len <= CAP)`). The user-struct heap is
bounded the same way: every struct allocation bump-allocates slots from a model
heap (1024 slots by default) and prunes executions that allocate more
(`__ESBMC_assume(__vow_heap_top <= 1024)`). That prunes every longer or larger
execution, so a `Verified` result for such a function means "verified for
collections no longer than the model capacity and at most that many struct slots",
not "verified for all collections". The verifier makes this explicit rather than
silent: every function proved from a model that carries such an assumption adds
one [`ModelCapacityAssumed`](errors.md#modelcapacityassumed) **note** (severity
`note`, on a `Verified` result) to the result's `diagnostics[]`, naming each
bounded kind and its capacity. The note changes neither the `status` nor the
exit code, and the verdict itself is never altered. The note is read off the
emitted model (every pruning capacity assumption is tagged
`/* vow:model-bound <Kind> <digits> */` in the C source), so it cannot disagree
with what was actually checked. It is a statement about the *prover*, never
about the program: do not respond to it by adding a length bound to a contract
(see the anti-pattern below) — an unbounded verifier removes the note without
any source change.

The marker is the *only* way the model prunes by capacity. Every other
`__ESBMC_assume` in the emitted C encodes something that is not a capacity: a
`requires` clause, an unreachable point or a checked-arithmetic abort (an
aborting execution never returns), the range of a nondeterministic value of its
own type (an `Option` tag and payload, a byte read from a `String`), or the
sorted-key representation invariant of a `BTreeMap`. None of these removes an
execution the runtime can produce, so none is reported.

## Blame Model

| Clause      | Blame  | Who is at fault                                    |
|-------------|--------|----------------------------------------------------|
| `requires`  | Caller | The caller passed invalid arguments                |
| `ensures`   | Callee | The function body doesn't satisfy the postcondition|
| `invariant` | Callee | The loop body breaks the invariant                 |

### Callers Without a `vow` Block

A function with no `vow` block is still a verify target when it **directly calls a function that has `requires`** and the verifier can model it (pure, only modelable operations, and no collection passed as an argument to a user function). Every parameter is nondeterministic — the equivalent of `requires: true` — and each callee `requires` is asserted at the call, so a caller that can violate it is reported `VowRequiresViolated` with `blame: "Caller"`.

Only the callee `requires` are obligations of such a function. Its own bounds, capacity, and checked-arithmetic checks are assumed, not asserted: a helper may rely on an invariant its callers keep, and it owes no contract of its own. A helper that forwards a parameter into a call whose `requires` it cannot establish (`fn g(x: i64) -> i64 { f(x) }` with `f` requiring `x >= 0`) is reported, and the fix is a real `requires` on `g`.

A caller with effects (such as `main() [io]`) or any other non-modelable caller is **not** verified. Neither is one whose proof the verifier cannot finish (timeout, `unknown`, memory limit): with no contract of its own it has nothing to leave unproved, so the build is not failed — unlike a contracted function, whose undecided proof still fails closed. Its calls into contracted functions are reported once per function as a `VerificationSkipped` **Note** (`calls from ... were not verified: ...`, ending in `cannot be modelled`, `the verifier timed out for ...`, or `the verifier could not decide them for ...`); the build status is unaffected, and the callee `requires` is checked at runtime in `--mode debug` only.

## Clause Purity and Heap Writes

A clause must not write through any argument while it is being evaluated — not just avoid declared effects. Vow passes structs, `Vec`, `String`, and maps by pointer, so a plain helper with no declared effect can still perform a real heap write (a field assignment, a `Vec`/map element write, a mutating builtin method) through a parameter. If such a write were allowed in a contract clause, it would happen every time the clause is evaluated — under `vow verify`, under `--mode debug`, and (today, since nothing elides a predicate's side effects at codegen time) in release too — making the clause's own evaluation an unaccounted-for part of the program's real behavior rather than a side-effect-free check of it. Rejecting this at type-check time (`EffectViolation`, `Blame::Callee`) keeps the predicate a predicate: see `docs/spec/grammar.md` → "Contract Purity" for the exact rule and the `mark(p)` example, and `docs/adr/2026-10-02-2348-contract-heap-write-purity.md` (ADR-2026-10-02-2348) for why this is checked for every clause rather than deferred to a write-footprint mechanism.

## Tuples in Clauses

Tuples are not first-class values (see the `let` tuple-pattern rules in `grammar.md`), so a
tuple expression, including the empty tuple `()`, cannot appear anywhere inside a `requires`,
`ensures` or `invariant` clause, or in a parameter `where` clause — not as a comparison operand (`requires: t != (1, 2)`) and not as
the initializer of a `let` inside a clause block. Both compilers reject it at type-check time with
`UnsupportedFeature` ("tuple expressions are not supported in contract predicates") at the
tuple's span. Compare the elements instead: `requires: a != 1 || b != 2`.

## Counterexample Replay (Differential Test)

`vow verify --replay-cex` (also `vow build --replay-cex`) cross-checks a counterexample against the executable's runtime semantics. After the verifier reports a violation, Vow maps the symbolic assignment to concrete Vow inputs, builds a `--mode debug` harness that calls the failing function with them, and checks whether the runtime `VowViolation` matches — **same `vow_id` and same blame**.

This is a *differential test*, **not part of the proof**. The static verdict and exit code are unchanged whether or not replay is requested. Its purpose is to detect drift between the two independent lowerings of a contract: the verifier's model (the C model, `requires` → `__ESBMC_assume` and `ensures`/`invariant` → `__ESBMC_assert`, under ESBMC; the SMT-LIB encoding under the native backend) and `vow-codegen`'s debug-mode runtime checks. A `confirmed` replay grounds the counterexample in real execution; a `diverged` replay flags either a model false-positive or values that do not reach the violation at runtime. See `docs/spec/cli.md` → "Counterexample replay" for the JSON shape and v1 input scope. Under `--backend native` the replay is also the oracle for the native checker: a counterexample that does not reproduce is reported as a `VerifierBug` diagnostic.

## Integer Contracts
