# Linear values are discharged with `drop`, never stored in collections

**Status:** accepted (2026-10-04)

## Context

`linear struct` values must be consumed exactly once. The spec said collections
cannot hold linear values (they copy and shift entries bitwise), but only
`BTreeMap` and `HashMap` values enforced it. `Vec<LinearStruct>` was accepted by
both compilers, and it was the only way anywhere in the corpus to get rid of a
linear value that had no further use: `v.push(h)` counted as a consume, after
which the value sat in a vector that nothing ever drained. The obligation was
"discharged" by leaking it. A function that took a linear parameter and did not
return it could not be written without that trick (`RegionLinear`), and
the `RegionLinear` hint already told agents to "use drop()", a function that did
not exist.

## Decision

1. `Vec<T>` is rejected, in both compilers and at every site where the type is
   written, when `T` is or contains a linear owner (`UnsupportedFeature`, with a
   hint). `HashMap` and `BTreeMap` values already followed the same rule. The
   test is owner semantics: a reference element (`Vec<&Token>`) borrows and stays
   legal, a tuple holding a linear owner is rejected, and a nested collection is
   reported once, at the innermost `Vec`. A type alias is reported once, at its
   definition, not at each use.
   `Option` and `Result` are not collections: they are linear owners that `match`
   consumes, and they stay legal.
2. A new intrinsic `drop(value)` is the terminal discharge. It accepts only a
   linear owner (`TypeMismatch` otherwise), has the empty effect set, consumes
   its argument exactly once, and has no runtime effect: no destructor, no free.
   It lowers to the existing consume marker and a unit value. In the verifier's
   C model the consume marker is a no-op, so verdicts are unchanged.

## Why this meets the language-design criteria

- **Does not make verification harder.** The consume marker neither reads nor
  writes data, so modelling it as a no-op is exact. Functions that previously
  fell out of the verifier (`Skipped`, because of the consume opcode) are now
  modelled. The marker is a no-op for every consume, not only `drop`; verify and
  verify-fail fixtures pin that a call argument, a user-enum match payload, an
  `Option` payload, a region-allocated value and a returned linear value are
  each proven when correct and refuted when the contract is false, identically in
  both compilers.
- **Eliminates a class of agent bugs.** An agent can no longer satisfy the
  linear checker by parking a value in a vector that is never emptied. The only
  ways to end a linear value's life are explicit: consume it in a callee, return
  it, match it, or `drop` it.
- **Makes agentic coding easier.** There is now exactly one spelling for "I am
  done with this value", and the compiler's own hint points at it.

## Alternatives rejected

- **(b) Migrate the fixtures without a discharge primitive.** Run tests cannot
  end a linear value at all, so the corpus would lose its positive coverage of
  match/option/region consumption, and agents would be left with the Vec trick or
  `RegionLinear`.
- **(c) Keep `Vec<linear>` and relax the spec.** Entries that are copied and
  shifted bitwise duplicate the obligation, and nothing ever consumes the stored
  values. This contradicts the consume-exactly-once rule the type exists to
  enforce.

## Consequences

- Any program that used a `Vec` of linear values must keep an integer handle in
  the `Vec` and consume the linear value directly, or drop it.
- A user function named `drop` shadows the intrinsic, so existing programs keep
  their meaning. The checker and both lowerers each decide independently (by
  looking the name up in their function tables); `tests/run/drop_user_defined.vow`
  pins that they agree.
