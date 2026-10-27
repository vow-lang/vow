
### Counterexample replay

With `--replay-cex`, each object in `counterexamples[]` gains a `replay` field — a **differential test** of the verifier's model against the executable's debug-mode runtime semantics. It is **not part of the soundness proof**: it neither strengthens nor weakens the static verdict, and the exit code is unchanged. It exists to catch *drift* between the two independent lowerings — `vow-verify`'s C emitter (`requires` → `__ESBMC_assume`, `ensures`/`invariant` → `__ESBMC_assert`) and `vow-codegen`'s debug runtime checks.

For each counterexample, Vow maps the verifier's assignment back to concrete Vow inputs, synthesizes a `--mode debug` harness that calls the failing function with those inputs, runs it, and compares the observed `VowViolation`.

| `replay` value | Meaning |
|----------------|---------|
| `"confirmed"`  | The harness fired `VowViolation` with the **same `vow_id` and the same blame** the counterexample predicted. High-confidence: the model agrees with runtime. |
| `"diverged"`   | The harness exited cleanly, or fired a *different* `vow_id`/blame. Either the verifier C model is wrong (a model false-positive) or the counterexample values do not reach the violation in real execution. `replay_reason` explains which. |
| `"aborted"`    | The harness emitted a structured runtime-abort diagnostic before reaching any `VowViolation` (for example, `IndexOutOfBounds`). The program failed, but through a different mechanism than the counterexample predicted; `replay_reason` names the abort and the unreached `vow_id`/blame. |
| `"skipped"`    | Replay was not attempted (e.g. an input type outside v1 scope, a Unit/aggregate parameter, the function is not defined in the entry file, or harness compilation failed). `replay_reason` gives the cause. |

`"aborted"` is distinct from `"diverged"`: it preserves positive runtime evidence that the
program failed while keeping `"diverged"` for runs where the verifier and runtime disagree about
whether the predicted failure is reachable. Runtime aborts use the reserved exit status `134` and
the structured diagnostics documented under [Runtime Errors](errors.md#runtime-errors).

**v1 input scope.** Reconstruction supports scalar parameters (`i64`, `u64`, `bool`) and bounded `Vec` of those scalars. `String`, `HashMap`, `BTreeMap`, struct, reference, and nested-aggregate parameters are reported as `"skipped"` with a reason (the native backend additionally rebuilds struct, enum, `Option`, `Result`, `Vec`, `String`, `HashMap` and `BTreeMap` parameters and every scalar width, see "Native backend" below). The self-hosted compiler's v1 reconstructs scalars only and reports `Vec` parameters as `"skipped"` (the Rust compiler additionally reconstructs bounded `Vec`s); both report identical outcomes for scalar and aggregate-skip cases. An entry file that already defines `main` replays like any other: both compilers set that `main` aside in the harness and call the failing function from a synthesized `main`. A counterexample for `main` itself is `"skipped"` (the harness cannot call the entry function).

`replay`/`replay_reason` are present on a counterexample only when `--replay-cex` was passed.

**Native backend.** Under `vowc verify --backend native` the replay contract is stricter, because the native model is meant to be checked against the runtime rather than trusted. The counterexamples for a division or remainder abort (zero divisor; signed `MIN / -1`), for an `unwrap()` on `None` or for a `Vec` index past the end (a read `v[i]`, a write `v[i] = x`, the same for a `Vec<i128>` or `Vec<u128>`), which carry the reserved unattributed `vow_id` and blame `None`, are replayed instead of skipped: the harness calls the function with the counterexample's inputs and the replay is `"confirmed"` when the program aborts with `ArithmeticOverflow` (division, remainder), `UnwrapOnNone` (unwrap) or `IndexOutOfBounds` (`Vec` index), exit status `134`. The runtime envelope names the abort kind but not the site, so this confirms the kind of failure for those inputs, not which operation raised it. Two obligations that share the violation text `index out of bounds` do not share a runtime outcome: a `String.byte_at` index past the end returns `-1` instead of aborting, and a shift count is masked, so those counterexamples stay `"skipped"` (`replay_reason` names the obligation) and are never reported as a verifier bug. A scalar parameter of any width (`bool`, `i8`..`i128`, `u8`..`u128`, `f64`) is rebuilt from its value in `values`, and so is a struct, enum, `Option` or `Result` parameter whose fields are scalars (the same types; `()` as a payload) or, recursively, structs, enums, `Option`s and `Result`s (up to 8 levels), from the `<param>.<slot>` values, where an aggregate held in slot `k` of `p` is an object whose slot `j` is the entry `p.k.j` (`o.1.0` is the first field of the struct an `Option` holds in `o`), a slot the model never read of a held object taking zero like a scalar field (see "Aggregates" above): the harness binds `Pair { lo: 3, hi: -4 }`, `Shape::Rect(5, 6)` or `Option::Some(9)` from the source declaration, giving a 128-bit field its two slots. A slot the model never read takes `0`/`false`, and an unused variant is never inspected. A parameter whose type is a `type` alias is rebuilt as its target, and two parameters of one object agree on a type once both are resolved (`a: Vec<Id>` and `b: Vec<i64>` under `type Id = i64`). A `Vec` of `bool` or integers up to 64 bits and a `String` parameter are rebuilt from the `<param>.len` and `<param>[<i>]` values (see "Counterexample contents of `Vec` and `String` parameters" above): the harness builds the value in a block that pushes each element (`push_byte` for a `String`). A `Vec` or `String` held in a field of a struct, enum, `Option` or `Result` is rebuilt the same way from `<param>.<slot>.len` and `<param>.<slot>[<i>]` ("Memory read through a field" above), and one the model never read is empty. An element the claim never read takes whatever the model gave it. A `HashMap` or `BTreeMap` parameter whose key is `bool` or an integer up to 64 bits and whose value is `bool`, such an integer or `()` is rebuilt from its listed entries: `new`, one `insert` per present key, and then as many other keys, none of them a key the counterexample lists, as `<param>.len` needs (a key type with too few keys for that, a length above 512 and any other key or value type are `"skipped"`). A `Vec<f64>` parameter marked `<param>.elem` = `f64` is rebuilt from its decimal elements, spelled as the `f64` parameters are (`NaN` as `(0.0 / 0.0)`, a negative value as `(0.0 - x)`). The replay is `"skipped"`, with a reason, for an `f32` parameter or field (no source text denotes one), a `Vec` whose elements are not `bool`, integers up to 64 bits or (marked) `f64`, a `Vec` or `String` whose contents the counterexample does not list (length above 512), a `bool` element whose slot is neither 0 nor 1, a struct, enum, `Option` or `Result` held in a field that has an entry of its own (the function read it through memory, and the counterexample lists its address, not its fields), a held `Vec` or `String` whose address is listed without a length, whose length is listed without an address, or whose elements are not `bool` or integers up to 64 bits (such a `Vec` is rebuilt only when its length is 0), any other non-scalar, non-aggregate type, an aggregate nested deeper than 8 levels, a reference, an alias used as a field or payload type, or a type not declared in the program, and for a discriminant outside the enum's variants when every variant is one the program tests. The model does not constrain the tag, so the solver may pick one (`-1`); a run with such a tag fails every `tag == k` test, which a variant the program never tests also does, so replay builds the smallest variant no `match` arm or `unwrap` selects (the last arm of a `match` is reached either way and does not count) and skips only when every variant is tested; that is a property of the model, not a verifier bug. A `Vec`, `String` or struct field whose value is the address `<param>.addr` of another `Vec`, `String` or struct parameter (a parameter may be the very object a field holds) is that parameter's object: the harness binds each parameter once, whatever their order, and the field names the binding, so the harness has one object where the model has one. The replay is `"skipped"` when the field and the parameter have different types, and when the field holds the very value it belongs to (a `struct Node { next: Node }` whose `next` is the `Node` itself, a type no value can be built for, since a field must hold a value before the value exists) or two parameters hold each other. A function that returns a `linear struct`, or an `Option`, `Result` or enum that owns one, is replayed like any other: the harness hands the result to `drop`, which discharges the obligation a bare call would leave. The ESBMC backend and the Rust compiler keep reporting aggregate parameters as `"skipped"`. A native counterexample whose replay ends `"diverged"` or `"aborted"` additionally yields one [`VerifierBug`](errors.md#verifierbug) error diagnostic: the model and the runtime disagree, so the defect is in the verifier. `"skipped"` never ran and is not reported as a verifier bug. The `status` and exit code are unchanged. The ESBMC backend keeps skipping the unattributed ids.

## Contracts Output JSON

`vow contracts` emits a single JSON object to stdout. Schema: [`schemas/contracts-result.schema.json`](schemas/contracts-result.schema.json).

### Example (without --verify)

```json
{
  "contracts": [
    {
      "vow_id": 0,
      "function": "divide",
      "kind": "requires",
      "description": "requires y != 0",
      "blame": "Caller",
      "source": { "file": "divide.vow", "offset": 42 },
      "status": "not_verified",
      "quality": "substantive"
    }
  ],
  "summary": { "total": 1, "proven": 0, "failed": 0, "timeout": 0, "error": 0, "not_verified": 1, "skipped": 0, "vacuous": 0, "trivially_satisfiable": 0, "quality": { "weak": 0, "tautological": 0, "substantive": 1 } }
}
```

### Example (with --verify)

```json
{
  "contracts": [
    {
      "vow_id": 0,
      "function": "divide",
      "kind": "requires",
      "description": "requires y != 0",
      "blame": "Caller",
      "source": { "file": "divide.vow", "offset": 42 },
      "status": "proven",
      "quality": "substantive"
    }
  ],
  "summary": { "total": 1, "proven": 1, "failed": 0, "timeout": 0, "error": 0, "not_verified": 0, "skipped": 0, "vacuous": 0, "trivially_satisfiable": 0, "quality": { "weak": 0, "tautological": 0, "substantive": 1 } }
}
```
