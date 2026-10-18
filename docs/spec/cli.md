# Vow CLI Reference

## Commands

### `vow build` (default)

Compile source to native executable. Verifies contracts by default.

```
vow build [OPTIONS] <source.vow>
vow [OPTIONS] <source.vow>          # bare form (identical to `vow build`)
```

The bare `vow <source.vow>` form is exactly `vow build`: it verifies by default, accepts the same flags, prints the same build-result JSON and fails closed (non-zero exit and `VerifyFailed` when verification fails). The former legacy-only `--emit-c` and `--verify` flags are not accepted and are rejected with the usage error (exit 2) like any unknown flag; `--no-verify` is the opt-out and `--dump-ir` prints the IR. Without `-o` the executable is written to `build/<stem>`.

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `-o, --output`    | `build/<stem>` | Output executable path                  |
| `--mode <debug\|release\|profile\|sanitize>` | `release` | Build mode: debug inserts runtime vow checks, profile inserts call counters and prints report on normal exit, sanitize adds debug checks + use-after-region-close detection for Vecs |
| `--no-verify`     | (off)       | Skip static verification                  |
| `--dump-ir`       | (off)       | Print IR text to stdout and exit (no JSON output, no codegen) |
| `--debug-trace <off\|calls\|full>` | `off` | Emit JSON trace lines to stderr at runtime |
| `--no-cache`    | (off)       | Disable verification result caching, and (for `--no-verify` builds) the compile-object cache. See "Compile-object cache behavior" below |
| `--max-k-step <N>` | `50`     | ESBMC backend only; a usage error under `--backend native`. ESBMC incremental BMC max iterations |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC backend only; a usage error under `--backend native`. ESBMC SMT solver; auto selects per-function via heuristic |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC backend only; a usage error under `--backend native`. ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 |
| `--timeout <N>` | `300` (or `30` when `--encoding` is `auto`) | Per-function verification timeout in seconds; the only resource knob of the native backend, where the default is a fixed `300`. ESBMC backend: under `--encoding auto`, a 30s default is applied so the BV-timeout fallback to `--encoding ir --solver z3` can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit `--timeout` overrides both. `--timeout 0` is an immediate watchdog kill under ESBMC and a zero budget (no solver is spawned; statically decided claims still resolve) under `--backend native` |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent verification jobs (ESBMC processes, or `verify-worker` subprocesses under `--backend native`) |
| `--backend <esbmc\|native>` | `esbmc` | Verification backend. `native` is accepted only by `vowc verify`, `contracts --verify` and `test --verify`; on `build` and the bare form it is a usage error (exit 1) that points at `vowc verify`, and `esbmc` is accepted silently |
| `--replay-cex`    | (off)       | Differential test the verifier model against runtime semantics (same as `vow verify --replay-cex`; see below) |
| `--perfetto <path>` | (off) | Write a gzipped Chrome Trace Event Format trace of this compilation to `<path>` (load directly at ui.perfetto.dev). Captures per-phase spans, codegen/link and, under the ESBMC backend, per-function ESBMC proof spans, the compiler→ESBMC handoff, and time-series CPU/RSS for the compiler and each ESBMC process. Distinct from `--mode profile`, which instruments the *compiled program*. Pure side artifact: never affects codegen, the build JSON, or the cache. |

**Compile-object cache behavior.** The on-disk compile-object cache (`$VOW_CACHE_DIR` or `~/.cache/vow/`, where each entry is a `<key>.o` artifact keyed by a content hash of all dependencies, mode, and trace settings) is automatically disabled whenever static verification is active. This guarantees the linked binary always comes from the same codegen run whose IR was verified, closing the integrity gap where a stale or attacker-supplied `.o` could be linked against freshly-verified IR. Concretely the cache only activates on `vow build --no-verify` invocations; it is bypassed on the default `vow build` path. `--no-cache` additionally disables the cache for `--no-verify` builds.

### `vow verify`

Verify contracts only — no executable output. Emits the same JSON format as `vow build` but `executable` is always `null`.

```
vow verify [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `--no-cache`      | (off)       | Disable verification result caching; under `--backend native` it disables the native result cache of `verify` (see "Result cache" below) |
| `--max-k-step <N>` | `50`       | ESBMC backend only; a usage error under `--backend native`. ESBMC incremental BMC max iterations |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC backend only; a usage error under `--backend native`. ESBMC SMT solver; auto selects per-function via heuristic |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC backend only; a usage error under `--backend native`. ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 |
| `--timeout <N>` | `300` (or `30` when `--encoding` is `auto`) | Per-function verification timeout in seconds; the only resource knob of the native backend, where the default is a fixed `300`. ESBMC backend: under `--encoding auto`, a 30s default is applied so the BV-timeout fallback to `--encoding ir --solver z3` can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit `--timeout` overrides both. `--timeout 0` is an immediate watchdog kill under ESBMC and a zero budget (no solver is spawned; statically decided claims still resolve) under `--backend native` |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent verification jobs (ESBMC processes, or `verify-worker` subprocesses under `--backend native`) |
| `--backend <esbmc\|native>` | `esbmc` | Verification backend. `native` is the symbolic checker with Bitwuzla, run in `verify-worker` subprocesses (epic #1398); self-hosted `vowc` only, see "Native backend" below. Accepted by `verify`, `contracts --verify` and `test --verify`; `contracts` and `test` without `--verify`, `build` and the bare `vowc <file>` form (identical to `build`) reject `--backend native` with a usage error (exit 1) instead of silently running ESBMC or reporting `not_verified` as if the native backend had run. Any value other than `esbmc` or `native` is a usage error |
| `--replay-cex`    | (off)       | Differential test of the verifier model against runtime semantics. After the verifier reports a counterexample, build a `--mode debug` harness that calls the failing function with the counterexample's concrete inputs and check that the runtime `VowViolation` agrees (same `vow_id` and blame). Adds a `replay` field to each counterexample (see "Counterexample replay" below). Opt-in, off by default; also accepted by `vow build`. |
| `--perfetto <path>` | (off) | Write a gzipped Chrome Trace Event Format trace of this verification run to `<path>` (load directly at ui.perfetto.dev). Captures frontend phase spans and, under the ESBMC backend, per-function ESBMC proof spans, the compiler→ESBMC handoff, and time-series CPU/RSS for the compiler and each ESBMC process; under `--backend native` it records the frontend phase spans (`module-load`, `parse`, `typecheck`, `lower`, ...) and one `verification` span (no per-claim worker or solver spans, no per-worker RSS yet). Accepted only by `build` and `verify`. Pure side artifact. |

#### Native backend (`vowc verify --backend native`; also `contracts --verify` and `test --verify`, see "Native backend: `contracts` and `test`" below)

Opt-in; the default stays `esbmc`. Implemented only by the self-hosted `vowc`: the Rust `vow` does not accept `--backend` until verification is delegated to a pinned seed `vowc`. Scope (issues #1408, #1411):

- **Subset.** A function is verified when its only effect is `panic` (or it has none), its loops are reducible `while`/`loop` loops (see "Loops" below), and it uses only these operations on integers of every width (`i8`..`i128`, `u8`..`u128`), `f32`, `f64` and `bool`: constants, `bool`, integer and float parameters, wrapping `+ - * / %`, the checked `+! -! *! /! %!`, `& | ^`, `<<` and `>>` (the shifted type's signedness picks arithmetic or logical `>>`), integer `as` casts (widening, same-width, and the truncating casts the lowerer emits for coerced literals), comparisons (`==`/`!=` also on `bool`), float `+ - * /` and comparisons (see "Floats" below), `!`, `if`/`else` (including nested, early `return`, and the `&&`/`||` that lower to branches) with the variables they update, `while` and `loop` with `break`/`continue`, `requires`/`ensures` and loop `invariant` clauses, and struct, enum and `Option`/`Result` values (see "Aggregates" below). Integers are fixed-width bit-vectors, so wrapping, signedness and 128-bit values are exact. Everything else (calls of builtins; calls of other functions are inlined, see "Calls" below), the `io`/`read`/`write`/`unsafe` effects, float `%` and unary minus, `String`) is **`Skipped`**, never `Verified`. A `Vec` of `bool`, integers (`i128` and `u128` included) or floats, and a `HashMap` or `BTreeMap` with keys and values of `bool` or an integer up to 64 bits, is modelled (see "Collections" below). The diagnostic is a `VerificationSkipped` warning whose message is ``skipped verification of `f`: <code>: <detail>`` with a code from the closed list in [`errors.md`](errors.md#verificationskipped) and a free-form detail (the runtime symbol, `Op[type]`, or a control-flow shape such as `irreducible loop (back edge to a block that does not dominate its source)`, `loop value used outside its loop`, `entry block is a loop header` or `loop nest deeper than 8`). A machine-readable `reason_code` field is not emitted yet.
- **Proof obligations.** One query per `ensures` clause and per abort site, in walk order (blocks in reverse postorder); a `requires` is an assumption only for the obligations after it, and so is every earlier abort guard on the same path (an aborting run never reaches a later obligation). An obligation is asked only on the paths that reach it: its query asserts the block's path condition, and an assumption made inside a branch holds only under that branch's condition. A variable updated in a branch is merged with one `ite` over the incoming edges, and only when the arms supply different values; a branch whose condition is constant, or was already decided by an enclosing branch on the same value, leaves the other arm unexecuted. The query grows with the number of branches and updated variables, not with the number of paths. Three kinds of abort follow the language:
  - *Wrapping `/` and `%`* abort on a zero divisor, and signed `/` aborts on `MIN / -1` (`MIN % -1` is `0`). A shift aborts unless `0 <= count < width` of the shifted type, tested in the count's own type so a negative signed count is rejected. Each is its own obligation: a counterexample is `VerifyFailed`, with the unattributed vow id `4294967293`, blame `None` and violation text `division or remainder by zero`, `signed division overflow (MIN / -1)` or `shift amount exceeds the operand's bit width`.
  - *`.unwrap()` on `None` or `Err`* is an obligation of its own: any path that reaches the abort is a counterexample with the unattributed vow id `4294967293`, blame `None` and violation text `unwrap() called on None`. A function whose `Option` parameter is unwrapped without a guard is `VerifyFailed`; one that unwraps a value it just built as `Some`, or sits on the `Some` arm of a `match`, is `Verified`. ESBMC skips every function with the `panic` effect, so it has no verdict to compare with.
  - *Checked `+! -! *! /! %!`* abort on overflow (and on a zero divisor, and signed `MIN /! -1`). The contract is judged on returning executions only, so a counterexample to such an abort is **not** a failure: the function is still `Verified` when its contracts hold, and each reachable abort adds an `ArithOverflowReachable` warning, as under ESBMC.

  A function with no obligation is `Verified` without a solver call. Counterexample values are decimal text of the declared type (unsigned types as magnitudes, 128-bit values in full).
- **Loops.** Each loop is unrolled to an internal bound and every obligation is checked on the unrolled prefix, together with a mandatory *unwinding assertion*: no run takes more back edges than the bound. The verifier tries the bounds 2, 4, 8, 16, 32 and 64 in turn (the largest is at least ESBMC's default `--max-k-step`); the bound is not a flag and `--timeout` stays the only knob, shared by all rounds. An `ensures` or `invariant` violated by a run inside the bound is a real counterexample: `VerifyFailed` with the clause's vow id, blame and source span, found at the smallest bound that reaches it. Unwinding alone proves a function only when no run needs more iterations than the largest bound. When every loop of the function has an `invariant`, the loops are also tried inductively: each loop is cut at its header, the variables it changes are unconstrained (a struct field the loop writes, and a `Vec` or `String` the loop writes too, except that a loop which only pushes to it leaves it no shorter than when the loop was entered, and a `Vec` or `String` that each iteration creates is not carried across iterations), and the function is `Verified` when each invariant holds on entry, is preserved by one iteration from any state that satisfies it and the loop condition, and, with the negated condition, establishes the rest of the function (including `ensures`). So a loop that can run longer than the bound (a bound that is a parameter, `while true`, a literal bound above 64) is `Verified` given an inductive invariant. Induction never produces `VerifyFailed`: a wrong invariant fails because unwinding finds a run that violates it, and an invariant that is true but not inductive, or violated only beyond the largest bound, leaves the verdict `verify_status: "unknown"` with a `verify_message` that starts `unwinding assertion:`, never `Verified`. A proof by induction is partial correctness: it covers the runs that terminate, so a loop that never exits satisfies any `ensures`. When a function has a checked operator, the unrolled rounds still run, so its warnings are the aborts they reach within the unrolled prefix; an inductive proof does not add any. The inductive cut also says which aborts any run can reach (it over-approximates every run), so the rounds end as soon as one has found all of them, and a proof whose cut reaches none needs no round. Once the cut has proved the function the rounds only look for the aborts a run reaches: they get a quarter of `--timeout` on top of what the proof took, and a round that does not finish in it is answered with the aborts the cut found, so the function keeps its proof and a warning is never missed. When a round of unwinding ends open (every claim holds inside the bound, but some run needs more iterations), the k-induction *step case* is tried at that round's bound `k`, with or without an invariant, for a function with exactly one loop whose header carries no pointer and which writes no aggregate, Vec or map: the loop is unrolled `k` times from an arbitrary state, the claims of the earlier copies (and the loop test, which they took) are assumed, and the function is `Verified` when every claim of the last copy and of the exit from it holds. It proves a property that is true for every trip count but not 1-inductive (an alternating divisor, a counter against a parameter bound) and, like invariant induction, is partial correctness. A failing step case proves nothing and is never `VerifyFailed`; the unwinding verdict stands. When the rounds end open at the largest bound and some loop has no `invariant`, the inductive attempt is made once more with the clause `true` at the head of each such loop (an arbitrary state at the top of the header, the same cut): a claim that holds there for every state, such as a bound read through the loop variable or a callee's `requires` under a guard, is `Verified` for every trip count (a `HashMap` filled by one loop and queried by another needs no invariant for that), and a fact about the state after the loop is out of its reach. A failing cut proves nothing and is never `VerifyFailed`, and a loop that carries a pointer or writes an aggregate is still not cut. A function with several loops that the cut does not close, or a loop that carries a pointer or writes an aggregate, a Vec or a map, is decided by unwinding and invariant induction alone. An unrolled program above the verifier's size budget is `unknown` with the same prefix unless induction proves it. Loops that cannot be unrolled stay `Skipped` as `unsupported-opcode` (details above): irreducible control flow, a loop value read after its loop through a path the verifier cannot close, a loop in the entry block, nesting deeper than 8. Checked-arithmetic warnings are reported once per source site, not once per iteration.
- **Extra proofs.** ESBMC reports 128-bit checked arithmetic and 128-bit struct fields or enum payloads as `Skipped`; native models them, so the same program can be `Verified` (with `ArithOverflowReachable` warnings) or `VerifyFailed` natively where ESBMC says `Skipped`.
- **Aggregates.** A struct, enum, `Option` or `Result` value is scalarised: each field is its own `bool`, integer or float term (a 128-bit field is one 128-bit term; a float field is a term of its float sort, and a slot written at one sort and read at another reads as unconstrained, so no bit pattern is reinterpreted), so construction, field access and `match` need no heap model. A parameter's fields are unconstrained, and so is its discriminant: no range is assumed for an enum tag. A field a construction did not write (the payload of `None`) is unconstrained too, never `0`. Two aggregates merged by an `if`/`else` or a `match` are merged field by field. A field write is path-conditional: a write in a branch keeps the old value on the paths that skip it, so a struct literal whose field value contains an `if`, a `match` or an `.unwrap()`, a field of a value built before a branch, and a field of the only struct or enum parameter are all modelled. A counterexample lists the fields of a struct, enum, `Option` or `Result` parameter in `values` under `<param>.<slot>` (the IR carries no field names): a struct's fields are `p.0`, `p.1`, ... in declaration order; an enum's discriminant is `p.0` (variant index in declaration order; `Option` is `None` = 0, `Some` = 1; `Result` is `Ok` = 0, `Err` = 1) and the chosen variant's payload starts at `p.1`; a 128-bit field takes two slots and so shifts the slots after it by one. Only slots the function reads are listed, and a claim lists those read before it in the walk (a slot read only after the claim, or never, is absent). A slot the function reads at two different sorts (an `i64` payload in one variant, a `bool` or an `f64` in another) gets a `:bool`, `:f32`, `:f64` or `:<bits>` suffix on every one of its names (`o.1:64`, `o.1:bool`). `bool` fields print as `true` or `false`. A field write through a merged value (`let p = if c { a } else { b }; p.f = 1`) is a write to each value the merge may stand for, under the condition that selects it. Parameters may be one object (the caller can pass one value twice): when a function writes a field through a parameter and reads or writes fields of, or merges, at least two struct, enum, `Option` or `Result` parameters, every such parameter carries a free identity and two parameters are the same object exactly when their identities are equal. A write to one is then also a write to every other under that test, and their unwritten fields are equal when they alias. Two parameters may alias only when their declared types have the same head (`Cell` and `Cell`, `Option<A>` and `Option<B>`): the lowering records each parameter's type head with type aliases resolved, and parameters of different heads, such as two structs of different types, are never one object; a parameter alone in its head has no identity, and when a head is not known all of them are one group, as under ESBMC. A counterexample lists each identity as `<param>.alias` (equal values mean one object), a parameter that is the same object as an earlier one shows that one's value for each field, reading a field of a parameter also lists the same slot of every earlier such parameter, and `--replay-cex` passes one value for parameters whose identities are equal. A call of a user function is inlined and its result is a merge of the objects the callee returns, so a field write to it reaches each of them under the condition that selects it (a callee that returns its parameter writes the caller's value). The following are **`Skipped`** as `unsupported-opcode`: a field write to a value that is not an allocation, a parameter, a merge of them or a user call's result (an extern call's result, say), and a field of any other type (a float or 128-bit field of a value reached through another field). A pointer written into a field (`S { ids: a }`, `h.items = v`) is stored as the address of the object, which stays a model of its own (see "Memory read through a field" below). The payload of an enum, `Option` or `Result` parameter that is itself an aggregate (a struct, or another enum) is an object of its own, one per parameter and slot, whose fields are symbols like a parameter's (`o.1` holds the payload, so its first field is `o.1.0` in a counterexample, and an object it holds in turn is `o.1.1.0`): two reads of one payload are one object, the payloads of two parameters are two, and nothing is assumed about their fields. The payload must be read from a parameter (or from the payload of one, through an inlined call as well): the payload of an enum the function builds or of a merge of parameters, a payload used as a `Vec` or a `String`, and a payload read beside a write to a parameter (the payload may be that parameter) or in a function whose parameters may be one object are **`Skipped`** as `unsupported-opcode`, never guessed at. `--replay-cex` rebuilds the payload object from the entries under its path (see "Native backend" in the `--replay-cex` section). A struct, `Vec` or `String` held in a field of a parameter, and an element of a `Vec`, are not scalarised: they are read through memory (see the same section). A `linear struct` value is an ordinary aggregate here: linearity is enforced by the type checker, so the verifier reads a linear parameter, a linear result and a linear value built in the function like any struct (`drop` and a consume are no-ops, and a counterexample lists a linear parameter's fields as `h.0`, `h.1`, ...). A linear value passed to or returned from a call, or held in an enum payload, is modelled as any other struct there. A loop that writes a field of a value built outside its body (a parameter, or an allocation made before the loop) is cut like one that writes a `Vec`: each slot the loop writes is unconstrained at the top of the header, so the loop is decided by its `invariant` rather than by unwinding (a field of a value the body itself builds is fresh each iteration and needs nothing). A write the model cannot attribute to a parameter or an allocation (through the element address of a `Vec<i128>`) leaves the loop to unwinding alone. A `Vec<i128>` or `Vec<u128>` element is `Skipped` as `unmodeled-builtin` (see "Collections").
- **Collections.** A `Vec<T>` with `T` of `bool`, an integer (`i128` and `u128` included) or a float (`new`, `push`, `v[i]`, `v[i] = x`, `len`, `pop`, `clear`, `truncate`, `vec_sort`, `Vec::from_raw_parts_copy` and `pin_to_root`) is an array plus a 64-bit symbolic length; a 128-bit element is two 64-bit slots of that array, read and written together, so its length, bounds obligation and counterexample contents are those of any other `Vec` (a listed element is one 128-bit value); a float element is the same 64-bit slot the runtime keeps (an `f32` in its low 32 bits), read through `to_fp` and written as the bits of a float constant or, for any other float, a fresh bit pattern assumed to be one of that float, so a float written and read back is equal to itself (every NaN is the one SMT NaN, so a NaN's payload does not survive, and a property that needs `x == x` states it) and a listed element is the float as a decimal; `Vec::from_raw_parts_copy(p, n)` is empty for a null `p` or `n == 0` and otherwise `n` elements of unknown contents (a length at the runtime's limit traps, so it never returns), and `pin_to_root` is a copy: the same length and elements in a separate object. There is no capacity: a parameter's length is unconstrained apart from the runtime's own limit (a Vec's length is below `2^63 - 1`, so `v.len() as i64 >= 0`), and a `push` is assumed to return only while the length stays below it, so a property over every length is proven for every length, not only up to ESBMC's model capacity. An index at or past the length is an obligation of its own on `get` and `set`: a counterexample is `VerifyFailed` with the unattributed vow id `4294967293`, blame `None` and violation text `index out of bounds`. A `Vec` parameter's length is listed in `values` as `<param>.len` and its elements below that length as `<param>[0]`, `<param>[1]`, ... (see "Counterexample contents of `Vec` and `String` parameters" below); two `Vec` parameters are distinct. A `String`, `Vec` or struct pushed or stored (`Vec<Vec<_>>`, `Vec<String>`, `Vec<struct>`) takes the 8-byte slot of its address. A function that never looks at the slot again keeps it as a fresh constant: the length, the bounds of every index and the other elements are exact, and the `Vec` can be returned or passed on. A function that reads an element back holds the address of the object instead (the address a pointer stored in a struct field has, see "Memory read through a field"): `outer[i]` is the `String`, `Vec` or struct stored at `i`, and a read through it (`len`, `get`, `byte_at`, a field) is a read of that object at that point of the function, so a push made after the store, an overwritten slot, one object in two slots and `pop`, `clear` and `truncate` behave as they do at run time. That needs every `Vec` a pointer is stored into to be one the function created empty with `Vec::new` (the elements of a parameter are memory the caller gave, which the model cannot tell from the stored object), every pointer stored to be a parameter, an allocation, a literal or a `String` or `Vec` the function built and uses as an object (a handle read out of another `Vec` is not an address the model knows), and no element read of any other `Vec` to be used as an object. **`Skipped`** is a function outside that which could look at the slot, as `unsupported-opcode`: it reads such an element back with `get` (`a pointer element is read back`, which includes reading an element of an element, `outer[i][j]`), copies or sorts a `Vec` that holds one (`a Vec of pointer elements is copied`), or reads memory through a handle (`a pointer element is stored in a function that reads memory through a handle`), because the stored object may be what a loaded handle names. A function that reads memory through a handle and never reads an element back is still modelled when every `Vec` a pointer is stored into is one the function built and no pointer an element read through memory returned is used as an object. The remaining cases are tracked by #1663. **`Skipped`** also: `reserve`, a 128-bit element read through a struct field, an index that is not 64-bit. Two Vecs merged by an `if`/`else` (or returned on two paths by a callee) are one value that stands for the one picked: a read sees that Vec, and a write reaches exactly that Vec under the condition that picks it, so every other name for either sees it (`let v = if c { a } else { b }; v.push(1)` pushes onto `a` when `c` holds and onto `b` otherwise). A loop that switches the Vec a variable names is not cut by its `invariant`: a symbolic bound there is decided by unwinding alone, `unknown` when the unwinding assertion cannot be discharged. Reading an element of a `Vec<struct>`, `Vec<Vec<_>>` or `Vec<String>` is modelled as memory (see "Memory read through a field"). `vec_sort(v)` is a new `Vec` of `v`'s length that is the sorted permutation of `v` (ascending, signed): every slot is at or below its successor and the slots are exactly `v`'s in another order, stated with quantifiers for every length, so the result is determined; sorting leaves `v` unchanged and a later write to `v` does not move the result. A Vec passed to another function is inlined (see "Calls"). **Maps.** A `HashMap<K, V>` or `BTreeMap<K, V>` whose key is `bool` or an integer of at most 64 bits and whose value is `bool`, such an integer or `()` (`new`, `insert`, `get`, `contains_key`/`contains`, `remove` on a `HashMap`, `len`) is three terms: a presence array from 64-bit keys to 0 or 1, a value array and a 64-bit symbolic length, with no capacity (the only length fact is the runtime's, that a table of that many entries is addressable, so a length is below `2^63 - 1`). A key or value is the 8-byte slot the runtime stores, so two keys are equal exactly when their slots are, and `get` and a `BTreeMap`'s `insert` return an `Option` that is `None` with payload 0 for an absent key. A `BTreeMap` is modelled like a `HashMap`: the runtime has no iteration API, so neither its key order nor the slot a `HashMap` key hashes to is observable. A map built in the function (`new`, then `insert` and `remove`) is exact. A map parameter, or the state a loop cut forgets, is an arbitrary key set and length, with the one fact a real map has that can be stated without a quantifier: the number of distinct keys the function names that are present is at most the length. With 64-bit keys that is all a real map needs, so a counterexample is a run. A `HashMap` whose key is narrower than 64 bits, or whose key type no operand shows (every key is an integer literal or a constant), may also have a length above its key domain, so a counterexample that depends on the keys or length of such a parameter is `verify_status: "unknown"` with a message that starts `unconstrained map state:`, unless the failure holds for every state. Two map parameters are distinct, as for `Vec`. **`Skipped`** as `unsupported-opcode` (detail names the cause): a key or value that is not `bool`, an integer up to 64 bits or `()` (a `String`, `Vec`, struct or map value, or a 128-bit integer), a map passed to another function, stored in a struct field, or merged by an `if`/`else` that yields different maps. A loop that writes a map is decided by unwinding and invariant induction alone, and the map's contents are unconstrained at the top of the loop in the inductive attempt.
- **Memory read through a field.** A struct, `Vec` or `String` that is a field of a struct, and an element of a `Vec`, is reached through a pointer the function loaded. The IR types such a pointer `i64`, so the verifier treats it as a 64-bit address, never as an object: a field `k` of the object at `h` is `hf<k>[h]`, the length of the collection at `h` is `hlen[h]` and its element `i` is `helem[h ++ i]` (a 128-bit key), each an unconstrained SMT array that is never written. Two reads of the same field are one address, so `b.insts.len()` and `b.insts[n - 1]` agree on which `Vec` `b.insts` is, and the model puts no cap on a length, a field or an element (a length is below `2^63 - 1`, the runtime's own limit, like every `Vec`). A `Bool` or integer field of up to 64 bits, a `Vec` or `String` length, a `Vec` element (a 128-bit one is the two slots `2i` and `2i + 1`) and a `String` byte can be read this way, to any depth (`ctx.func.blocks[i].insts[n - 1].op`); an index at or past the length is the same obligation as for any `Vec`. The arrays are never written, and a parameter may be the very object an address names (`q.child` is `p`), so a function that reads memory this way and also writes a parameter (`FieldSet` on a struct parameter, `push`, `set`, `pop` or a `String` mutation on a `Vec` or `String` parameter) lays the state of its parameters over them: each `Vec` or `String` parameter and each struct parameter the function uses carries a free 64-bit address, and a read through the address `h` is that parameter's own current state (the length, element, byte or field the writes before the read made it) when `h` is its address, and the array's otherwise. That is exact and has no quantifier, and a loop's cut havocs the parameter the loop writes as it does without reads through memory. A `Vec` or `String` is never a struct, so a length, element or byte read can name only a `Vec` or `String` parameter and a field read only a struct parameter, and a `String` operation names only a `String` parameter, a `Vec` operation only a `Vec` parameter; two `Vec` or `String` parameters are distinct objects, and two struct parameters are one object exactly when their identities say so (see "Aggregates"). A handle may stand for any parameter of its class even where the source types differ. A counterexample lists each address as `<param>.addr`, so a field whose value is an address shows which parameter it aliases. A function declared to return a `String`, `Vec` or struct may return such an address (a getter, `fn name(p: Person) -> String { p.name }`, or `v[0]` of a `Vec<String>`): the call's result is the handle, read through the memory model like any other, and a Phi that merges only handles is one. A merge of a handle with an object the function built (`if c { v[0] } else { String::from("x") }`) has no model and is **`Skipped`** as `unsupported-opcode` (`Upsilon type differs from its Phi`). Building and mutating a fresh value is unaffected: no handle can name it. Writing through an address (a field write, `push`, a `String` operation other than `len` and a byte read) and reading a float or 128-bit field through one are **`Skipped`** too. A pointer written into a field of a struct (`S { ids: a, names: b }`, `h.items = v`, `o.inner = i`) stores the object's 64-bit address in the slot, and a read of the field returns that address, so the collection or struct read through it is the object itself at that point of the function: a `push` after the store is what the read sees, and two fields hold two objects. An object the function allocated gets a free address of its own when it is first stored, distinct from the address of every other object, and a parameter that is stored has the address a parameter has for the aliasing rule above. A pointer chosen by a branch stores the address of the object each edge supplies, so a read sees the object of the path taken, and a store in a branch keeps the old address on the paths that skip it. A value read out of the memory the function was given (a parameter's field, a field or an element read through a handle) is assumed to be none of the objects the function allocates, since an allocation is no object that existed before it; only an address the function stored can be the handle a read then names. A store into a parameter's field is a field  write like any other and follows the aliasing rule of "Aggregates". A write through a collection held in a field (`b.insts.push(x)`, `b.insts[i] = x`, a `String` operation that changes the string) is a write to the object the field holds when exactly one store of the field may name the struct (a struct the function or a callee built, stored once with a pointer whose store dominates the read): `b.insts` is then that stored object wherever an object is needed, and a loop that writes `b.insts` is a loop that writes that `Vec`. A second store of the field, a store after the read, a store through a pointer the function loaded when the struct was itself stored into a field, pushed into a collection or passed as a value, and a field of a parameter are not resolved, and a write through such a handle is **`Skipped`**. A counterexample lists the fields of a parameter read through memory like any other, so a `Vec` field of `b` is `b.<slot>` with the address as its value; the contents of the collection are not listed. `--replay-cex` reports it as `"skipped"`.
- **Strings.** A `String` is the same array plus 64-bit symbolic length as a `Vec`, with no capacity and the runtime's own length limit (below `2^63 - 1`); byte `i` is the low 8 bits of its slot, so every read is in `0..=255`. Modelled: `String::new`, a literal, `String::from` and `pin_to_root` (each a copy), `String::from_raw_parts_copy` (empty for a null source or a zero length, else that many bytes of unknown contents; a length at the runtime's limit traps and so never returns), `len`, `push_byte`, `push_str`, `clear`, `byte_at`, `eq`, `substring`, `string_substr`, `string_starts_with`, `string_ends_with`, `contains` and `string_matches_literal_at`, with the clamping and the no-wrap `pos + len` rule of [`grammar.md`](grammar.md#string-offsets). `byte_at` at or past the length is an obligation of its own, `VerifyFailed` with the unattributed vow id `4294967293`, blame `None` and violation text `index out of bounds` (the verifier is stricter than the runtime, which returns `-1`). `eq` is exact, for strings of any length: equal lengths and every byte equal, never a length shortcut, a hash or an uninterpreted function. It is a finite conjunction when both lengths are small constants and otherwise a quantified assumption (the query then uses the `ABV` logic instead of `QF_ABV`; the quantifier is never in the negated goal). `contains` is exact too: true for an empty needle and when some start up to `len - needle.len` matches, false otherwise, stated by a witness start for the true direction and a Skolem position per start (a binder in an assumption, plus its first and last instance) for the false one, so a property that needs the false direction of two related searches (`a.contains(b) && b.contains(c)` implies `a.contains(c)`) is a quantified query the solver may not close, and is then `verify_status: "timeout"` or `"unknown"`, never a proof. A function that reads no byte of any string is given no byte facts at all. A `String` parameter's length and bytes are listed in `values` as `<param>.len` and `<param>[<i>]` (see "Counterexample contents of `Vec` and `String` parameters" below); two `String` parameters are distinct objects, as for `Vec`. The equality is exact, and two differently built equal strings (`(a + b) + c` against `a + (b + c)`) are decided: the byte at a differing index is read through the appends that built it, so the solver splits on the lengths instead of instantiating the quantified definitions. That unfolding is bounded in size and falls back to the array beyond it, which keeps the query exact but may make it slow, and then `verify_status` is `"timeout"` or `"unknown"`, never a proof. `hash_str` is exact: the runtime's FNV-1a fold over every byte (offset basis `0xCBF29CE484222325`, prime `0x100000001B3`, wrapping), a closed term when the length is a small constant (so the hash of a literal folds to the runtime's value) and otherwise the entry `len` of a fresh prefix array that a binder in an assumption defines (`h[0]` is the basis and `h[i + 1] == (h[i] xor s[i]) * prime` below `len`). `hex_encode` is exact: `2 * v.len()` bytes, the digits `0123456789abcdef` of the high and the low nibble of each element's low 8 bits. `hex_decode` is exact, including the runtime's strictness: empty unless the length is even and every pair is a hex digit, optionally after a `+` (`u8::from_str_radix` accepts a leading plus sign), else `len / 2` bytes. Their binders stand only in assumptions, so a proof or a counterexample that has to reason through one may end `verify_status: "timeout"` or `"unknown"`, never a wrong verdict. `parse_<int>` (`parse_i8` to `parse_u128`, and the `u64` method), `string_trim`, `string_to_upper` and `string_to_lower` follow Rust's Unicode-aware `str` methods (a non-ASCII character can change the length, invalid UTF-8 gives the empty string or `None`), which the model does not encode, so each is an opaque total function of the bytes of its string, like `parse_f64_bits`: a fresh constant (the Some flag and a payload of the parsed width, or an array and a length for a text), equal strings give equal results, and the facts every run has are kept (a payload fits its type, `string_trim` never grows a string, a case map keeps the empty string empty). `replace(s, from, to)` is the same kind of function of its three strings: a fresh text whose length is below the runtime's limit, shared by calls on the same operand terms and equal for equal operand strings. `split(s, sep)` is a `Vec<String>` of a fresh array and length with the facts every run has (at least one piece, at most `s.len() + 1`, exactly one for an empty `sep`, and the same count for equal operand strings); its elements are addresses of strings the model does not hold, so an element read through is a read of unconstrained memory and a counterexample that needs one is `unknown`. `join(parts, sep)` is a fresh text whose length is below the runtime's limit and is empty for no parts; it states no congruence, because the runtime reads the contents of every part, which the model does not hold for an object the function stored. A claim that holds for every result is proven; a counterexample that depends on a result is `unknown` (`opaque builtin result`), never a counterexample. **`Skipped`**: `String::from_cstr` and the arena variants (`unmodeled-builtin`; the reason is in `compiler/vc_strops.vow`), a `String` stored in a struct field and passed to another function (merged by an `if`/`else` that yields different strings is modelled like a `Vec`), and a string literal that is mutated (the runtime traps on it). None of the skips is a cap: each is a semantics the model does not encode exactly or a result it cannot hold. `from_cstr` reads memory behind a C string, and an arena variant takes an extra region operand that has no model; a byte-wise approximation or an unconstrained result would let a counterexample be spurious, so the caller is skipped instead.
- **Counterexample contents of `Vec` and `String` parameters.** The model of such a parameter is an SMT array plus a length, and an array cannot be listed whole. A counterexample therefore lists the length as `<param>.len` and, for every index below it, the element as `<param>[<i>]`, in the element's own type, as the program reads it back from the 8-byte slot the runtime keeps: a signed integer is signed (`-7`, not `18446744073709551609`), a narrow integer is the slot's low bits (the high bits of the slot are arbitrary in the model and no read observes them), a `bool` is `true` for any nonzero slot, a `String` byte is its low 8 bits (`97`, not `353`), an element of a `Vec<i128>` or `Vec<u128>` is one decimal value of its own type, from both slots, and an element of a `Vec<f64>` or `Vec<f32>` the function reads or writes is the float as a decimal, `NaN`, `inf` or `-inf`, with the parameter's `<param>.elem` entry holding `f64` or `f32` (an integer-valued float, `3`, prints like a raw slot, so the entry is how a reader tells the two apart; a read or write of a copy made by `pin_to_root` or `clone` marks the parameter it was copied from). An element of a parameter the driver cannot find the source declaration of keeps the slot as an unsigned 64-bit decimal. A parameter the claim's path never reads has no entries. To make the contents finite, the verifier asks the failing claim again with every such length bounded by 2, then 8, 64 and 512 and shrunk to the shortest length the solver finds, so a counterexample needing 101 elements has 101 and not the solver's first pick; any model of the claim's query is a counterexample, so the bound never excludes a run from verification. When no model with every length at most 512 exists, or the second query does not finish in the budget, only `<param>.len` is listed (it may then be as large as `2^63 - 2`) and `--replay-cex` reports the counterexample as `"skipped"`.
- **Numeric builtins.** `hash_u64` is the runtime's SplitMix64 output function encoded as one closed 64-bit term (three xor-shifts and two multiplications by constants, all wrapping), so two calls on equal operands agree with no axiom, a literal operand folds to the runtime's value (`hash_u64(0)` is `0xE220A8397B1DCDAF`) and nothing else is assumed: whether it is injective, or avoids a value, is for the solver to decide. `int_to_string`, `uint_to_string` and their 128-bit forms give the exact decimal text of the operand: the length is a term of the operand (one digit per power of ten its magnitude reaches, at least one, plus a `-` for a negative `i64`, so at most 20 for a 64-bit operand) and, in a function that reads a byte of some string, so is every byte (digit `k` from the right is `(magnitude / 10^k) % 10` plus `0x30`; `i64::MIN` has magnitude 2^63). Calls on one operand give one string. A property that relates two formatted strings of different operands (for example that the text is injective) is a quantified equality the solver may not close, and is then `verify_status: "timeout"` or `"unknown"`, never a proof. `int128_to_string` and `uint128_to_string` are the same with up to 39 digits (40 with the sign).
- **Narrowing intrinsics.** `<src>_to_<dst>_wrap`, `_sat` and `_try` ([`grammar.md`](grammar.md#type-cast)) are exact closed terms of their operand, never an unconstrained result. `_wrap` is the truncating cast (the low bits of the operand; two's-complement, so `i64_to_u8_wrap(-1)` is `255`); `_sat` clamps the operand into the destination's range, compared by the *source's* signedness (`i32_to_u8_sat(-5)` is `0`, `u64_to_i16_sat(40000)` is `32767`, and the 128-bit same-width `i128_to_u128_sat` clamps only below `0`); `_try` returns an `Option` that is `Some` exactly when the operand is in range, whose payload is the wrapped value, so a `match` on it reads exact values and a counterexample through it replays. The 128-bit sources and the same-width sign changes `i128_to_u128_{wrap,sat}` and `u128_to_i128_{wrap,sat}` are modelled too (ESBMC's C model does not model the same-width pair and skips those functions).
- **Floats.** An `f32` or `f64` is a term of the SMT-LIB `FloatingPoint` sort (`Float32`, `Float64`), so a function that uses one is asked in the `QF_BVFP` logic (`QF_ABVFP` with a `Vec`). `+`, `-`, `*` and `/` round to nearest, ties to even, the only mode the language has; division by zero is not an abort but an infinity or a NaN, and no float operation is an obligation. Unary `-` is `fp.neg`, an exact sign flip with no rounding, so `-0.0` and `-(-x)` hold. Comparisons are the IEEE predicates: every comparison with a NaN operand is false, except `!=`, which is true; `-0.0 == 0.0`. A float parameter is unconstrained, so it may be a NaN or an infinity, and a contract that needs a finite value states it (`requires: x - x == x - x` holds exactly for finite `x`). A float struct field or enum payload is a slot term of its own float sort (see "Aggregates" below). A counterexample shows a float as the shortest decimal that reads back as the same `f64` (`NaN`, `inf`, `-inf`; an `f32` is widened first, which is exact). `--replay-cex` rebuilds an `f64` parameter or field (including `NaN`, the infinities and `-0.0`, by exact arithmetic, since the language has no such literals) but reports an `f32` one as `"skipped"`: the language has no `f32` literal and no cast to or from a float, so no source text denotes a given `f32`. Known limits: SMT-LIB has one NaN per format, so the verifier has a single canonical NaN and a NaN's payload or sign bit is neither provable nor refutable; float `%` is `Skipped` as `float-rem-unsupported` until codegen defines it; a float `Vec` element is the runtime's 8-byte slot (see "Collections" above), and `--replay-cex` rebuilds a `Vec<f64>` parameter the counterexample marks `<param>.elem` = `f64` (see "Counterexample contents of `Vec` and `String` parameters" above) and reports a `Vec<f32>`, or a `Vec<f64>` whose elements the function never reads as floats, as `"skipped"` (a listed float and a raw slot are both digits for an integer-valued float, so without the mark replay cannot tell them apart); `parse_f64_bits`/`format_f64_bits` are exact on their `u64` side and a deterministic function of their `String` side, but the text conversion itself is opaque (no digit grammar, no round trip), so a counterexample that depends on the value `parse_f64_bits` returns or the text `format_f64_bits` returns is `unknown` (`opaque builtin result`), never `Verified` and never a counterexample (the `parse_<int>` and case/trim helpers of the Strings bullet above are opaque in the same way); nonlinear float arithmetic (`*`, `/`) is expensive to bit-blast, so a hard property over it can end `unknown` or `timeout`, never `Verified`.
- **Calls.** A call of another function whose arguments and result are integers, floats, `bool`, `()`, structs, enums, `Option`/`Result` values, `Vec`s or `String`s is inlined: the callee's body is spliced into the caller before execution, so the caller is verified together with everything it calls and the callee's clauses become obligations of the caller. An aggregate or collection argument is the caller's own object (the splice substitutes it for the callee's parameter), so a callee that pushes onto a `Vec` or writes a field of a struct it was given changes the caller's value, and the same object passed twice is one object (`push_pair(v, v)`); a returned aggregate or collection is the object the callee returned, and a callee that returns different collections on different paths returns the merge of them (see "Collections"). A callee `requires` is an obligation blamed on the caller (the counterexample's `vow_id` is the callee's own id, `function` is the function being verified, and `call_sites` lists the calls from that function down to the failing clause, outermost first); a callee `ensures` or loop `invariant` is an obligation blamed on the callee. Each is assumed once asked, so the first violated clause on a path is the one reported. A function without a `vow` block that calls a contracted function is verified only for those callee preconditions, as under ESBMC. A function on a call cycle, directly or through other functions, is `Skipped` as `recursion-unsupported` with the cycle as detail (`a -> b -> a`); a function whose callee is outside the subset is `Skipped` as `non-modelable-callee` with the callee and its own code as detail (`g (float-rem-unsupported)`). A call tree that flattens to more than 16 384 instructions is decided with callee summaries where it can be, else `Skipped` as `inlining-too-large` with the size as detail: a worker does not decide it within its memory cap, and the skip says so at once instead of ending in `unknown` after minutes. A *summary* stands for a callee that was proven on its own in the same run (every claim of it, as for a function with a contract) when the caller is above that budget and the callee is above a quarter of it: the call keeps the callee's `requires` as obligations blamed on the caller and leaves every field of a struct parameter the callee may write, and its result, unconstrained; the callee's own clauses, `ensures` and loop invariants are not asked again, and its `ensures` is assumed after the call (over the state the call left, so a clause about a field the callee wrote constrains the unconstrained value; a clause the verifier cannot copy out of the callee is not assumed). A callee that is not proven, that mutates a `Vec`, `String` or map it was given, or that stores or returns a pointer it was given or read out of memory, has no summary. A counterexample found with a summary standing for a callee is `verify_status: "unknown"` with a message that starts `callee summary:`, never `VerifyFailed`, because the summary admits states the callee cannot produce. A call tree whose inlined body exceeds the verifier's own size budget (200 000 instructions) is `verify_status: "unknown"` with a message that starts `inlining budget:`, never `Verified`.
- **Verdict divergence from ESBMC.** The ESBMC model does not check `MIN / -1` for `/`, so a function that guards a signed division by `requires: y != 0` alone (body `x / y`) is `VerifyFailed` under `--backend native` (counterexample `x = i64::MIN`, `y = -1`) while ESBMC proves it. Native is right: the runtime aborts with `ArithmeticOverflow` (exit 134) and `--replay-cex` confirms the counterexample. The true domain of a signed division excludes both aborts, so the repository's divide programs (`examples/divide.vow`, the benchmark `E05_safe_divide`) also state `requires: x >= -9223372036854775807 || y != -1` (that is `x != i64::MIN || y != -1`), and the verification corpus carries no program whose label depends on ESBMC missing it. Ruling for #1688.
- **Workers.** The functions in the subset are verified by `verify-worker` subprocesses of the same `vowc` binary (internal; its flags and output are not part of the CLI contract), at most `--verify-jobs` of them. A worker is handed the module the parent lowered (it lowers the program itself only when that module cannot be read) and decides the functions it takes from a shared queue one after another, so the frontend runs once in the parent and never per worker or per function. A worker runs in its own process group, so killing it also kills its solver. On Linux it runs under a fixed internal address-space cap; there is no user-facing memory flag. Results are reported in IR order whatever order the workers finish in, and reporting stops at the first function that is not proven, so the output is the same for every `--verify-jobs` value. A worker that runs out of memory (the runtime's structured `OutOfMemory` report, a solver out-of-memory message, or a kill from outside) is `verify_status: "unknown"` with `verify_message: "memory limit exceeded"`, never a proof and never a compiler crash. Any other abnormal worker death (a signal, a nonzero exit, no result) is `verify_status: "panicked"` and the message names the signal or exit status. Expiry of `--timeout` on a function kills the whole group of the worker that is deciding it and is `"timeout"` for that function; the functions that worker had not reached are taken by another worker. A worker that runs out of memory after it has already decided other functions may only have been squeezed by what they left behind, so the function it was on is retried once in a fresh worker and only a second failure is `"unknown"`.
- **Solver.** `bitwuzla` is resolved from `PATH`. The obligations of one round of a function that has at least two for the solver share one interactive solver process of the worker (each obligation asked in its own scope); only an `unsat` answer, or a `sat` on a checked-operator or unwinding obligation, is taken from it. Any other answer, and any misbehaviour of that process (an error, an unrecognised reply, a timeout, an exit), is decided by one self-contained `.smt2` per obligation, written to a private temp directory owned by the parent (removed on every path, including a killed worker) and run as a child process of the worker, which alone supplies a counterexample, so the result does not depend on whether the process was shared. Only `unsat` is a proof. `unknown` is `verify_status: "unknown"`; a solver that outlives its budget is killed (`"timeout"`); a non-zero exit, `[error]` output, an unrecognised answer or an unparsable model is `"error"`. If `bitwuzla` is not on `PATH` and a function needs it, the result is `VerifyFailed` with `verify_status: "tool_not_found"` and no counterexample (ESBMC is not consulted). A module in which every function is `Skipped` needs no solver. The binary is not hash-checked; its `--version` output names it in the result cache key.
- **Flags.** `--timeout <N>` (seconds, default `300`) is the budget for each function, measured from the moment a worker starts deciding it (the one-time decoding of the module by the worker is not counted); `--timeout 0` is an immediate `timeout` without spawning the solver. `--verify-jobs <N>` caps concurrent workers (default half the CPUs). `--no-cache` disables the native result cache of `verify` (see "Result cache" below). `--max-k-step`, `--solver` and `--encoding` are ESBMC options and are rejected with a usage error under `--backend native`. `--replay-cex` works as for ESBMC; `--perfetto` is accepted and records the spans listed under the flag table. `--replay-cex` also replays the division, remainder, `unwrap()` and `Vec` index abort counterexamples and reports a counterexample that does not reproduce as a `VerifierBug` diagnostic (see "Counterexample replay" below).
- **Result cache.** Every solver answer that is `sat` (with its model) or `unsat` is stored on disk and replayed by a later run, so a warm run reports exactly what the cold run did without asking the solver again. An entry is keyed by a hash of the exact `.smt2` text of the query plus the solver's `bitwuzla --version` output and the fixed solver argument list, and holds that text, which a lookup compares byte for byte: a stale entry cannot exist (a new solver version or any change to the query is a different key), a hash collision is a miss, and a truncated, garbled or hand-edited entry is a miss. A claim is looked up under its full query text before a shared solver process is asked; an `unsat` from that process is stored under the same text, so a standalone run and a shared-process run replay each other's entries. A `sat` from the shared process of a soft claim carries no model and is not stored. `unknown`, `timeout`, memory, error and tool-not-found answers depend on the budget or the environment and are never stored, so they are asked again. A sliced query and the full query are cached under their own text; a cached `sat` for the sliced query still sends the claim to the full query. A non-positive budget is a `timeout` before the cache is consulted. The cache lives in `$VOW_CACHE_DIR/verify-native/` (default `~/.cache/vow/verify-native/`), sharded by the first two hex digits of the key, at most 64 entries per shard (a full shard drops an arbitrary half before it takes a new entry), written by temporary file and rename so concurrent workers never see a partial entry. Any problem with the directory or with the solver's `--version` disables the cache for that run and never fails verification. Only `verify` uses the cache: `contracts --verify` and `test --verify` ask the solver every time. `--no-cache` neither reads nor writes it, and the performance gate (`scripts/verify_perf.py`) always runs with it off and fails a sample if the cache directory is written. Unlike the ESBMC driver's cache, which only ever persists failures, this cache stores proofs: whoever can write the cache directory can forge an `unsat`, so use `--no-cache` when the result must come from a fresh solver run (a "green locally" claim). A solver call that timed out cold can be decided warm, since a hit costs nothing; the identical-verdict guarantee holds for runs in which no solver call times out.

#### Native backend: `contracts` and `test`

`vowc contracts --verify --backend native` and `vowc test --verify --backend native` (issue #1427) run the same native checker and worker pool as `verify`, with the status vocabulary, JSON schemas and exit codes of the ESBMC path unchanged. Self-hosted `vowc` only; the default stays `esbmc`.

- **Selection.** Both commands check the functions that carry a `vow` block, exactly as the ESBMC path does (`verify` also checks callers of contracted functions, these do not). A function outside the native subset is `skipped` in `contracts` (a status only, no diagnostic, as for ESBMC) and makes the file `contract_skipped` in `test` (with one `VerificationSkipped` diagnostic of severity `error` in the entry's `diagnostics`, fixed message `contract verification skipped: function not modelable`, no reason code), and needs no solver.
- **No early stop.** `verify` reports up to the first function that is not proven; a `contracts` report or a `test` verdict spans functions, so a failed function never cancels a later one. Every function is verified at most `--verify-jobs` at a time, and the result does not depend on which worker finishes first.
- **Clause status (`contracts`).** Each `ensures` and `invariant` clause carries the verdict of its own obligations, from one `verify-worker` that decides every obligation of the function. A `requires` clause (an assumption, never an obligation) is `proven` when the function is proven and `unknown` otherwise, as for ESBMC's `--multi-property` mode. A function whose only counterexample is an abort (no clause refuted) has no `proven` clause: they are all `unknown`. Statuses are `proven`, `failed`, `unknown`, `timeout`, `error`, `skipped` and `vacuous`; `proven-ir` is an ESBMC-only status and is never produced. When `bitwuzla` is not on `PATH`, every clause of a function that needs it is `error` and the run exits 1; a function without obligations, or a skipped one, still resolves without a solver.
- **Weak-contract probes (`contracts`).** The two probes of the ESBMC path are IR rewrites checked by the same engine, each its own task with the full `--timeout` budget, so a slow probe loses only its own answer. *Vacuity*: the function is cut right after its last `requires` and the cut point must be unreachable. If the `requires` clauses are jointly unsatisfiable, every clause of the function is `vacuous` (this overrides the clause verdicts, as for ESBMC). The probe applies to functions with a `requires`. *Trivial*: the single returned value is replaced by the type's default (`0`, `false`, `+0.0` for `f32` and `f64`) and every `ensures` must still hold; if it does, each `ensures` of the function has `trivially_satisfiable: true`. The computation that produced the value stays in the function, so what it can abort on is still checked. It applies to functions with an `ensures`, a scalar result and one `return` of a value computed in the body (not a parameter), as for ESBMC. A probe that is not proven (failed, `unknown`, timeout, error) never claims a weakness.
- **Fail-closed `test`.** Every function with a `vow` block must be `proven`: `failed`, `unknown`, `timeout`, `error` and a worker that died all make the file `verify_failed` and the test program is not run. The ESBMC path lets an `error` outcome pass; the native path does not, because nothing was established for that function. Each function has the fixed `300` s budget; `test --timeout` is the per-test execution timeout in milliseconds and does not apply to verification. `--jobs` workers each run their own verify pool, so the total grows with `--jobs` times `--verify-jobs`.
- **Verdict divergence from ESBMC.** The ESBMC path plants its vacuity label only in the entry block. A `requires` written with `&&` or `||` lowers to branches and lands in a later block, so ESBMC reports `vacuous` for a satisfiable `requires: a > 0 && a < 100`; the native probe follows the definition above and does not. The `MIN / -1` and 128-bit differences listed under "Verdict divergence from ESBMC" above apply to `contracts` and `test` as well. Two more are specific to the clause report. A function that fails only on an abort (a division or remainder by zero, `MIN / -1`, an over-wide shift, `unwrap()` on `None`, an out-of-bounds `Vec` index) and refutes no clause has no `proven` clause: its `ensures` and `invariant` clauses are `unknown`, like its `requires` clauses, and the run exits 1, as `verify` reports the same function `VerifyFailed`. ESBMC instead lets the run continue past the abort with an unconstrained value, so a clause over that value is `failed` there, while a clause that does not depend on it, or a function with no `requires`, can be `proven` and the run exit 0. A loop with no `invariant` whose bound is symbolic is decided by unwinding alone, so its clauses are `unknown` where ESBMC's incremental BMC may close the forward condition. A function whose retained computation can still abort on a checked operator is never flagged `trivially_satisfiable`, as under ESBMC.
- **Flags.** `--timeout <N>` (seconds) and `--verify-jobs <N>` are honoured by `contracts`; `--max-k-step`, `--solver` and `--encoding` are rejected with a usage error, as for `verify`. `--no-cache` is accepted and has no effect. `--replay-cex` and counterexamples are not part of either command.

### `vow contracts`

List all contracts (requires, ensures, invariant) in a program. Runs frontend only by default (no codegen, no verification).

```
vow contracts [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `--verify`        | (off)       | Run verification (the ESBMC backend by default, or the native backend with `--backend native`) and report per-contract status |
| `--no-cache`      | (off)       | Disable verification result caching        |
| `--max-k-step <N>` | `50`       | ESBMC backend only; a usage error under `--backend native`. ESBMC incremental BMC max iterations (with --verify) |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC backend only; a usage error under `--backend native`. ESBMC SMT solver (with --verify) |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC backend only; a usage error under `--backend native`. ESBMC encoding mode (with --verify); ir requires z3 |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent `verify-worker` subprocesses under `--backend native`; a no-op under ESBMC (the ESBMC loop is serial) |
| `--backend <esbmc\|native>` | `esbmc` | Verification backend; `native` requires `--verify` (see "Native backend: `contracts` and `test`") |
| `--timeout <N>` | `300` (native) | Seconds per function (and per probe) under `--backend native`; ESBMC's own timeout otherwise |

**Exit code.** With `--verify`, `vow contracts` fails closed exactly like `vow build --verify` and `vow verify`: it exits **1** if any contract's `status` is not proven — i.e. any `failed`, `timeout`, `unknown`, `error`, `skipped`, or `vacuous` — and **0** only when every contract is `proven` (or `proven-ir`, an ESBMC-only status no compiler produces today). Without `--verify` every contract is `not_verified` and the command exits 0. (This is independent of the static `quality` classification, which never affects the exit code.)

When `--verify` is requested but the verifier is not installed (ESBMC, or `bitwuzla` under `--backend native`), the command still emits the full contracts-result JSON schema with every entry's `status` set to `error` (under native, only for functions that need the solver) and exits with code 1 (fail-closed). Install the verifier, or omit `--verify`, to obtain proven/failed/unknown statuses.

### `vow skill`

Generate or install the Claude Code skill document for the current compiler version. The skill is embedded in the compiler binary, ensuring the documentation always matches the installed toolchain.

```
vow skill              # print skill document to stdout (default: print)
vow skill print        # print concise Claude Code SKILL.md entrypoint
vow skill print --bundle  # print self-contained bundle for raw API harnesses
vow skill install      # prompt for local or global install target
vow skill install --local   # install to ./.claude/skills/vow/
vow skill install --global  # install to $HOME/.claude/skills/vow/ on Linux
```

`print` writes the concise installed `SKILL.md` entrypoint (with YAML frontmatter) to stdout. `print --bundle` writes a complete self-contained skill document to stdout for non–Claude Code harnesses that cannot load supporting files.

`install` writes `SKILL.md` plus supporting files under `reference/`, `examples/`, and `schemas/`. Claude Code discovers the skill from the `.claude/skills/` directory and uses the frontmatter description/`when_to_use` metadata to load it for `.vow` file work as well as creation and verification-debugging prompts before a `.vow` file exists.

**Atomic install.** `install` builds the whole tree in a staging directory next to the target (`.claude/skills/.vow-install-*`) and renames it into place, writing `SKILL.md` last, so a failed or interrupted install never leaves a `SKILL.md` that links to missing `reference/`, `examples/`, or `schemas/` files. When `skills/vow/` already exists, the old tree is renamed aside, the new one renamed in, and the old one restored if that rename fails; the directory is briefly absent between the two renames, never half-written. Files the install does not own (hand-added notes, stale files an older toolchain left in `reference/`) are copied from the old tree into the staged one before the swap, so a re-install rewrites owned files and leaves everything else alone; the install needs write access to `.claude/skills/` itself. A `skills/vow` symlink (dotfile managers) is kept and written through in place, support files first and `SKILL.md` last. A pre-existing partial tree is repaired by the next explicit `install`; auto-install never touches a tree that already holds `SKILL.md`.

When no scope flag is provided, `install` prompts on stderr for local (`./.claude`) or global (`$HOME/.claude`) installation. Scripts and agents should pass `--local` or `--global` explicitly. `--local` requires the current directory to contain both `.git` and `.claude/`; otherwise it exits with an error and writes nothing. `--global` installs under `$HOME/.claude/skills/vow/` and fails if `$HOME` is unset or empty.

**Auto-install on build.** The first time `vow build` (or the bare `vow <source.vow>` form) runs in a directory that already contains a `.claude/` subtree but no `.claude/skills/vow/SKILL.md`, the compiler installs the skill silently. This bootstraps Claude Code projects without requiring an explicit `vow skill install`. Unlike explicit `--local`, auto-install only requires `.claude/`; it does not require the directory to be a git checkout. Auto-install is skipped when `.claude/` does not exist (so it never pollutes non–Claude Code projects) and when the skill file is already present (so user edits are never overwritten). Auto-install never fails the build and stays silent on failure; a failed auto-install leaves no `SKILL.md`, so the next build retries it.

**Migrating an existing install.** An existing `.claude/skills/vow/SKILL.md` is never rewritten, and while it is present a build does not add or repair support files, so an older single-file skill keeps working but lacks `reference/`, `examples/`, and `schemas/`. To migrate, refresh after a compiler upgrade, or repair, run `vow skill install` with the scope being replaced: `--local` for `./.claude/skills/vow/` or `--global` for `$HOME/.claude/skills/vow/` (a global install does not refresh a project-local one). Without `.git`, `--local` is unavailable: delete `.claude/skills/vow/SKILL.md` and run a build instead. Explicit install rewrites `SKILL.md` and every support file it owns, overwriting local edits, and leaves other files alone; review the diff of a committed project install. A pre-rename `.claude/skills/vow-toolchain/` install is neither detected nor modified, so a build installs `.claude/skills/vow/` beside it: delete the old directory by hand.

### `vow test`

Discover, compile, run, and report on Vow test files. Tests are normal `.vow` programs with `main() -> i32` — no test-specific syntax.

```
vow test [OPTIONS] [<path>]
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `<path>`          | `.`         | Directory to scan or single `.vow` file    |
| `--verify`        | (off)       | Run verification (ESBMC, or the native backend with `--backend native`) on test files |
| `--backend <esbmc\|native>` | `esbmc` | Verification backend; `native` requires `--verify` (see "Native backend: `contracts` and `test`") |
| `--filter <pat>`  | (none)      | Only run tests whose file stem contains pat |
| `--module-root <path>` | (auto)  | Resolve `use` declarations against `<path>`. Defaults to the scan path when it's a directory, otherwise the nearest ancestor of the entry file that resolves all of its `use` declarations. |
| `--jobs <N>`      | `min(num_cpus/2, 8)` (`1` with `--verify`) | Max test files compiled and run concurrently |
| `--mode debug`    | (default)   | Insert runtime vow checks                 |
| `--mode release`  | `debug`     | Omit all vow checks for performance       |
| `--timeout <ms>`  | `30000`     | Per-test execution timeout in milliseconds |
| `--max-k-step <N>` | `50`       | ESBMC backend only; a usage error under `--backend native`. ESBMC incremental BMC max iterations (with --verify) |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent verification jobs (with --verify) |

Test discovery: files matching `test_*.vow` or `*_test.vow` under the given directory **and its subdirectories**, sorted alphabetically. Each test must contain `main() -> i32` returning 0 on success.

**Module resolution for directory scans.** When `<path>` is a directory, every discovered test resolves its `use` declarations against `<path>` rather than the test file's own parent directory. This lets internal-unit tests live in a subdirectory like `compiler/tests/test_region.vow` and still `use region;` to import the module under test (which lives at `compiler/region.vow`). A single-file invocation without `--module-root` (`vow test compiler/tests/test_region.vow`) infers the module root the same way: starting at the file's own directory and walking up through its ancestors, it picks the nearest directory against which every `use` declaration of the file resolves (to `<path>.vow`, or `<path>.vow.d`). The walk stops after the first directory that contains `.git` (the repository root), at a `..` path component, or at `.` / `/`. If the file's own directory already resolves every `use`, if the file has no `use` declarations, or if no directory resolves them all, the file's parent directory is used and any unresolved module is reported as an ordinary `IoError`. When the file's own directory does not shadow a module of the tree, the inferred root is the one the directory form would use, so the test gives the same result in both forms; a module in the file's own directory takes precedence over one in an ancestor. This rule applies to `vow test` only; `vow build` and `vow verify` keep resolving `use` against the entry file's parent directory. This existence-only walk never inspects file contents, so it is unaffected by the `.vow.d` stub-vs-source contract fallback described under [Use Declarations](grammar.md#use-declarations): that check only runs once a concrete `use` is actually loaded for a build/verify/test run, not during root inference's probing of candidate directories.

**Concurrency.** Test files in a scan run concurrently, at most `--jobs` at a time, and the `tests` array is always in sorted-path order regardless of completion order. A test's `duration_ms` covers its own compile, verification, and execution. Beyond the first, a worker starts a new file only while the machine is not under memory or IO stall pressure (Linux PSI `/proc/pressure/memory` `some avg10` and `/proc/pressure/io` `full avg10`, both below 20), so a loaded machine degrades to fewer workers rather than thrashing; with no PSI available the limit is just `--jobs`. `--jobs 1` runs files strictly one after another. The self-hosted compiler runs each file of a concurrent scan in a worker subprocess of itself, using an internal `--worker-entry` flag whose output is not part of the CLI contract; a worker that dies without a result is reported as a `compile_error` entry so the suite stays fail-closed.

**Test Output JSON:**

```json
{
  "status": "TestsPassed",
  "total": 3,
  "passed": 3,
  "failed": 0,
  "skipped": 0,
  "tests": [
    {
      "file": "compiler/test_arith.vow",
      "name": "test_arith",
      "status": "passed",
      "exit_code": 0,
      "stdout": "7",
      "stderr": "",
      "duration_ms": 72,
      "diagnostics": [],
      "counterexamples": []
    }
  ],
  "contract_density": {
    "functions_total": 1,
    "functions_with_vows": 0,
    "density_pct": 0.0
  }
}
```

| Status Field   | Meaning                                           |
|----------------|---------------------------------------------------|
| `TestsPassed`  | All tests passed                                  |
| `TestsFailed`  | One or more tests failed                          |

Per-test status: `passed`, `failed`, `timeout`, `compile_error`, `verify_failed`, `contract_skipped`, `skipped`.

Each test binary is run directly, with no memory limit, and is killed when `--timeout` elapses (status `timeout`). A binary that exits non-zero, or is killed by a signal, is `failed`; a signal death or a `timeout` has no `exit_code` (`null`, or omitted by the Rust compiler).

A `timeout` counts toward `failed`, like `compile_error`, `verify_failed` and `contract_skipped`: a run containing one yields `TestsFailed` and exit code 1.

A `<path>` that does not exist is itself a failure: the result is `TestsFailed` with `total: 1` and `failed: 1`, holding one synthetic `failed` entry whose `file` is the path as given and whose `stderr` names it, and the exit code is 1. `total` equals the length of `tests` in every result, so the synthetic entry is counted.

`contract_skipped` means no proof was attempted because a vowed function is non-modelable (distinct from `verify_failed`, where the verifier found a violation or could not decide). Both are fail-closed — a `contract_skipped` test counts toward `failed` and yields a `TestsFailed` overall status.

### `vow decl`

Emit declaration file output only.

```
vow decl [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `-o, --output`    | `<source>.vow.d` | Output declaration file path          |

`decl` type-checks the source with its `use` dependencies, then writes one declaration stub for the merged module: dependency items first, the entry module's items last, with no `use` lines. Function bodies are dropped; signatures, effects, `vow` blocks, struct/enum/const/type-alias definitions and `extern "C"` blocks are kept. The path written is reported on stderr as `wrote <path>` and stdout stays empty. An existing output file is overwritten. Frontend diagnostics go to stderr, followed by `vow decl: <parse|module load|type> error`, and the exit code is 1 with no file written. `decl` accepts only `-o/--output`, `--help` and `--human`.

The self-hosted compiler cannot print an `enum` with a struct-like variant (`V { f: T }`): its AST drops the field names. `vowc decl` exits 1 with `vow decl: enum <Name> has a struct-like variant, which the self-hosted compiler cannot print` instead of writing a stub with a different meaning.

### `vow mutants` (self-hosted only)

Run mutation testing on a Vow source tree. Implemented in the self-hosted compiler only; the Rust bootstrap compiler emits an error pointing the user to `build/vowc`. See `docs/mutants.md` for full details on output schema, mutation kinds, skip-list, and known limitations.

```
vowc mutants version
vowc mutants list  [--root DIR] [--shard X/Y]
vowc mutants run   [--root DIR] [--shard X/Y]
                   [--tier1-cmd 'cmd'] [--tier15-cmd 'cmd'] [--tier2-cmd 'cmd']
                   [--tier1-timeout-secs N] [--tier15-timeout-secs N] [--tier2-timeout-secs N]
                   [--tier2-budget-secs N]
                   [--workdir DIR] [--output-dir DIR] [--force-unlock] [--skip-baseline]
```

| Flag | Default | Notes |
|---|---|---|
| `--root` | `compiler` | Directory whose `*.vow` files are mutated. `test_*.vow` files are excluded. |
| `--shard X/Y` | `0/1` | Round-robin split of the deterministic mutant ID space. Mutant `id` is selected iff `id % Y == X`. |
| `--tier1-cmd` | `scripts/bootstrap.sh --skip-cargo` | Fast oracle. Anything but exit 0 = caught at Tier 1. |
| `--tier15-cmd` | `VOW_FULL_TEST_SKIP_CARGO=1 VOW_FULL_TEST_TIER15_ONLY=1 scripts/full_test.sh` | Checkpoint oracle, run on Tier-1 survivors only. Fast, behavior-exercising prefix of `full_test.sh` (Sections 0-8c) — see `docs/mutants.md`. |
| `--tier2-cmd` | `VOW_FULL_TEST_SKIP_CARGO=1 scripts/full_test.sh` | Full oracle. Only run on Tier-1.5 survivors. The env var skips the redundant `cargo build --all --release` step — see `docs/mutants.md`. |
| `--tier1-timeout-secs` | `180` | Per-mutant Tier-1 wall-clock cap. |
| `--tier15-timeout-secs` | `1200` | Per-mutant Tier-1.5 wall-clock cap. |
| `--tier2-timeout-secs` | `3600` | Per-mutant Tier-2 wall-clock cap. |
| `--tier2-budget-secs` | `7200` | Per-shard total Tier-2 budget. Once exhausted, surviving Tier-1.5 mutants are emitted with `status:"unrun"`. |
| `--workdir` | `/tmp/vow-mutants-<ms>` | Path of the throwaway `git worktree` used for all mutations. |
| `--output-dir` | `mutants.out` | Directory for `mutants.json`, `outcomes.json`, status text files, `diff/`, `logs/`. |
| `--force-unlock` | off | Remove a stale `output_dir/.lock` before starting. |
| `--skip-baseline` | off | Skip the baseline run of Tier 1 and Tier 1.5 on the unmutated worktree. Without it, a failing baseline aborts the run with exit 1 — see `docs/mutants.md`. |

Output schemas: see `docs/spec/schemas/mutants-result.schema.json`.

### `vow complexity`

Report per-function complexity metrics as deterministic, **byte-identical** JSON (the Rust and self-hosted compilers produce identical output). Every structural metric sits next to its size; the single 0–100 `complexity_score` is a readability / refactor-priority gate — explicitly **not** a defect predictor. The component vector, not the scalar, is the source of truth; gating on the scalar alone is opt-in and discouraged as the sole signal.

```
vow complexity <source.vow>
               [--cog-anchor N] [--nloc-anchor N]
               [--max-score N] [--max-cognitive N] [--max-cyclomatic N]
```

| Flag | Default | Notes |
|---|---|---|
| `--cog-anchor <N>` | `15` | Cognitive-complexity value mapped to sub-score `0.800` (SonarQube's default flag line). |
| `--nloc-anchor <N>` | `60` | NLOC value mapped to sub-score `0.800` (~50–60 line guidance). |
| `--max-score <N>` | (unset) | CI gate: exit nonzero if any function's `complexity_score` exceeds N. The recommended line is 80, but gating is opt-in only. |
| `--max-cognitive <N>` | (unset) | CI gate: exit nonzero if any function's `cognitive` exceeds N. |
| `--max-cyclomatic <N>` | (unset) | CI gate: exit nonzero if any function's `cyclomatic` exceeds N. |

**Exit code.** Nonzero on frontend/read failures, malformed numeric flags, or when a `--max-*` threshold is passed and exceeded. With no `--max-*` flag the command is pure reporting once the input is readable and valid — no threshold gates by default (per the decouple-language-from-prover principle).

**Numeric convention.** The non-integer metrics (`halstead.volume`/`difficulty`/`effort` and `score_factors.*`) are emitted as fixed-3-decimal JSON numbers computed in **integer fixed-point** (scale 1000) — never native floats — so both compilers stay byte-identical. `complexity_score` is an integer in `[0, 100]`. The score's saturating anchor map uses a rational curve (`0.800` at the anchor, asymptoting to `1.000`), not an exponential, because the self-hosted compiler has no floating point.

**Contract identifier convention.** `vow.contract.free_vars` counts distinct value identifiers referenced by clause predicates. It excludes the `result` binding, function callee identifiers, and method names; receiver and argument expressions still count when they are values.

Output schema: see `docs/spec/schemas/complexity-result.schema.json`.

### `vow --help`

`vow --help` is agent-first. It emits versioned JSON capability data for the tool, command set,
language surface, result schemas, and implementation status. `--help --human` exists only as a
legacy compatibility mode and is not the canonical interface.

```
vow --help               # versioned JSON tool-help protocol
vow --help --human       # legacy compatibility text
vow build --help         # same JSON (works on all subcommands)
vow verify --help --human  # same legacy text (works on all subcommands)
```

## Exit Codes

| Code | Meaning                                                                            |
|------|------------------------------------------------------------------------------------|
| `0`  | Success (`Verified` or `Unverified`)                                               |
| `1`  | Failure (`CompileFailed`, `VerifyFailed`, or `Skipped`)                            |
| `2`  | Usage error: an unknown flag (`error: unexpected argument '<flag>' found`)         |

Both compilers reject a flag the selected subcommand does not implement before doing any work, with
exit `2` and `error: unexpected argument '<flag>' found` on stderr. This includes flags that were
retired prover-model flags and typos: they are never
silently ignored. The self-hosted `vowc` only accepts the `--flag value` form for value flags;
`--flag=value` is reported as an unknown argument with a hint. `vowc mutants` forwards its flags to
its own parser and is not validated by the driver.

`vow build` and `vow verify` both fail closed on `Skipped`: if the verifier was asked to verify a vowed
function but the verifier could not model the function body, the contract was not statically
proved, so the run exits non-zero. Use `--no-verify` if you genuinely want to skip verification —
that path produces `Unverified` (exit 0).

The table above is the exit status of the `vowc` **compiler**. A **compiled Vow program** exits
with whatever its `main` returns, with one reserved exception: any runtime abort — out-of-memory,
contract violation, arithmetic overflow, division or remainder by zero, unwrap-on-`None`,
index-out-of-bounds, region-literal mutation, stack overflow, or a sanitizer trap — terminates with
the reserved status **`134`**. By convention `134` is reserved for aborts: it is never produced
*spontaneously* by a normal `main` return, so a program that does not itself return or
`process_exit(134)` can treat any `134` as a runtime abort rather than an application result. The
reservation is a convention, not enforced — a program that deliberately exits `134` opts out. See
the *Exit status* note under Runtime Errors in [`errors.md`](errors.md) for the full list and
rationale.

## Build Output JSON

`vow build` and `vow verify` emit a single JSON object to stdout. Schema: [`schemas/build-result.schema.json`](schemas/build-result.schema.json).

**Note:** `--dump-ir` suppresses JSON output — only IR text is printed.

### Status Values

| Status          | Meaning                                     |
|-----------------|---------------------------------------------|
| `Verified`      | Compiled + every vowed function's contract was statically proved by the verifier, and every call from a verifiable uncontracted function into a contracted function satisfies the callee's `requires` (see [Callers Without a `vow` Block](contracts.md#callers-without-a-vow-block)). "Verifiable" excludes uncontracted callers that cannot be modelled (e.g. `main() [io]`) or whose proof the verifier could not finish; each carries a `VerificationSkipped` *Note* instead, and the note does not change the status. May still carry `ArithOverflowReachable` *Warnings* in `diagnostics[]`: those report a checked operator (`+!`, `-!`, `*!`, `/!`, `%!`) whose `ArithmeticOverflow` abort is reachable. The abort is the operator's specified behaviour and the contract is proved for every returning execution, so the status stays `Verified` (exit 0). See [`errors.md`](errors.md#arithoverflowreachable). It may also carry one `ModelCapacityAssumed` *Note* per proved function whose proof is bounded by a verifier model capacity; the status and exit code are unchanged. See [`errors.md`](errors.md#modelcapacityassumed). |
| `Unverified`    | Compiled but no verifier was invoked (`--no-verify`, `--dump-ir`). Exit 0. A missing verifier is not `Unverified`: it is `VerifyFailed` (exit 1). |
| `Skipped`       | Verification was requested but at least one vowed function could not be modelled. The reason is one of the closed list in [`errors.md`](errors.md#verificationskipped) under the native backend, or free text such as `LinearBorrow`, `Load`/`Store`, `RemF*`, effects under ESBMC. **ESBMC backend:** struct construction (`RegionAlloc`) and field reads/writes (`FieldGet`/`FieldSet`) **are** modelled via the user-struct heap model, except at 128-bit width: that slot is 8 bytes, so a `FieldGet`/`FieldSet` carrying an `i128`/`u128` is reported `FieldGet at 128-bit width` / `FieldSet at 128-bit width` instead of being modelled (the native backend models it). Each skipped function appears as a `VerificationSkipped` *Warning* in `diagnostics[]`, also when another function fails verification (the status is then `VerifyFailed`, and the warnings remain). Their contracts are runtime-checked under `--mode debug` but were not statically proved; the run fails closed with exit 1. |
| `CompileFailed` | Parse error, type error, module load error, unsupported code generation (a named backend limitation, such as `%` on floats), backend failure, link failure, or a diagnostic-emission I/O failure (e.g. a broken stderr/stdout pipe other than the tolerated case, or a full disk). Inspect `diagnostics[]`; backend failures use `CodegenUnsupported`, `CodegenFailed`, `LinkFailed`, or `IoError`. |
| `VerifyFailed`  | The verifier produced a non-Verified outcome: a counterexample, timeout, `VERIFICATION UNKNOWN` (`verify_status: "unknown"`; under the native backend also an unwinding failure, `unwinding assertion: ...`, an exhausted inlining or size budget, a counterexample that depends on an opaque builtin result, `opaque builtin result: ...`, or on the keys or length of a map the function did not build, `unconstrained map state: ...`, a counterexample found with a callee summary, `callee summary: ...`, or an out-of-memory worker, `memory limit exceeded`), tool error, the tool was not found (`tool_not_found`: ESBMC, or `bitwuzla` under native), or the verifier worker crashed (`verify_status: "panicked"`). Inspect `counterexamples[]` (definitive failures) and `verify_status`/`verify_message` (soft failures) to distinguish. |

For a multi-function `verify` or verified `build`, a halt-class result (definitive
counterexample, timeout, unknown, tool error, missing tool, or worker panic) stops new function
checks from being launched. Checks already claimed by parallel workers finish, and the compiler
reports the halt from the lowest module-declaration-order function in that claimed set. Therefore
`counterexamples[]` contains at most one entry; a reported soft failure leaves it empty and uses
`verify_status` instead. Fix the reported failure and run verification again to surface a later
failure.

### Verified Example

```json
{
  "status": "Verified",
  "executable": "examples/divide",
  "diagnostics": [],
  "counterexamples": []
}
```

### CompileFailed Example

```json
{
  "status": "CompileFailed",
  "executable": null,
  "diagnostics": [
    {
      "error_code": "TypeMismatch",
      "message": "function body has type `bool` but declared return type is `i32`",
      "severity": "error",
      "span": {
        "file": "bad.vow",
        "offset": 25,
        "length": 8
      }
    }
  ],
  "message": "type error",
  "counterexamples": []
}
```

### VerifyFailed Example

```json
{
  "status": "VerifyFailed",
  "executable": "examples/cegis_broken",
  "diagnostics": [
    {
      "error_code": "VowEnsuresViolated",
      "message": "contract violation in `safe_sub`: ensures result >= 0",
      "severity": "error",
      "span": {
        "file": "examples/cegis_broken.vow",
        "offset": 76,
        "length": 20
      },
      "hints": ["function `safe_sub` failed to establish its postcondition"],
      "blame": "Callee"
    }
  ],
  "function": "safe_sub",
  "counterexample": "[Counterexample]",
  "counterexamples": [
    {
      "function": "safe_sub",
      "values": { "a": "-9223372036854775808", "b": "0" },
      "violation": "ensures result >= 0",
      "vow_id": 1,
      "source": {
        "file": "examples/cegis_broken.vow",
        "offset": 76,
        "length": 20
      },
      "blame": "Callee"
    }
  ]
}
```

Every counterexample also yields one `error` diagnostic, appended after any
warnings. Its code follows the counterexample's `blame` (`Caller` →
`VowRequiresViolated`, `Callee` → `VowEnsuresViolated`; a failed `invariant` is
callee-blamed, so it reports `VowEnsuresViolated`), or is
`VerifierAssertionUnattributed` when the failure is not attributed to a vow
clause. The `message` is ``contract violation in `<function>`: <violation>`` (or
``verification failed in `<function>` on an unattributed property: <violation>``),
`span` is the counterexample's `source` (an empty file and zero offset/length
when `source` is `null`), `secondary` lists the counterexample's `call_sites`,
and `hints` name the failing function and, for caller blame, each violating
argument. A `timeout`, `unknown`, `error` or `panicked` outcome has no
counterexample and adds no such diagnostic. Both compilers emit the same
`diagnostics[]`, `counterexample` and `counterexamples[]` for the same source;
the only fields that are not specified byte for byte are `verify_message`
(the verifier's free text) and, under ESBMC, `values` entries named `$esbmc$...` (solver temporaries).

A diagnostic's `span` is exactly `{file, offset, length}`: positions are byte
offsets, and neither compiler adds line or column fields.

For caller-blame failures where a verified function violates a callee's
`requires` clause, the counterexample reports the callee clause in `violation`
and `vow_id`, and includes the caller expression in `call_sites`. When the
callee precondition binds a parameter, `violating_args` names the callee
parameter, the counterexample value when available, and the caller argument span.
If `violating_args[].value` is `""`, Vow could not statically recover the
caller argument value; `arg_offset` and `arg_length` still identify the
argument expression.

When a callee verified alongside its caller fails its own `ensures` or
`invariant`, the counterexample is attributed to that callee: `function` names
the callee, `violation`, `source` and `blame` (`"Callee"`) are the callee's
clause, and `vow_id` is the callee-local id. The outcome's top-level `function`
remains the function being verified. Such a counterexample carries no
`execution_path` or `branch_decisions` (block ids are not attributable to the
callee), and `--replay-cex` reports it as `"skipped"` because its inputs are
the caller's.

`blame` uses one casing on every surface: `Caller` or `Callee` in diagnostics
(omitted when no party is at fault), `Caller`, `Callee` or `None` in
counterexamples, and the same `Caller`/`Callee` in `vow contracts` and the
runtime `VowViolation` line.

When `blame` is `"None"`, `violation` describes the failed verifier-model check
(such as division by zero, collection bounds or capacity, unwrap-on-None, or
shift count) rather than exposing raw verifier output.

### Fields Reference

| Field              | Type                | When Present      | Description                               |
|--------------------|---------------------|-------------------|-------------------------------------------|
| `status`           | string              | Always            | One of the four status values             |
| `executable`       | string \| null      | Always            | Path to binary, null on compile failure or library module (no main) |
| `diagnostics`      | array               | Always            | Compiler diagnostics (see schema)         |
| `message`          | string              | CompileFailed     | Compatibility error category/detail (for example "parse error", "type error", "module load error", backend/link detail, or "failed to emit frontend diagnostics: {io_error}"). Agents should branch on `diagnostics[].error_code`, not parse this free text. |
| `function`         | string              | VerifyFailed      | Function where verification failed        |
| `counterexample`   | string              | VerifyFailed      | Legacy description string: `"[Counterexample]"` when the verifier produced a counterexample, otherwise the soft-failure text (`verification timed out`, `verification result unknown: <reason>`, `esbmc error: <message>` (the prefix is kept whichever backend ran), ...) |
| `counterexamples`  | array               | Always            | Structured counterexamples (see schema); contains at most one entry per run under the multi-function stopping policy above |
| `verify_status`    | string              | On backend failure | `"timeout"`, `"unknown"`, `"error"`, `"tool_not_found"`, or `"panicked"` (verifier worker thread crashed — no counterexample available) |
| `verify_message`   | string              | On backend failure | Verifier/backend error detail             |

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

**v1 input scope.** Reconstruction supports scalar parameters (`i64`, `u64`, `bool`) and bounded `Vec` of those scalars. `String`, `HashMap`, `BTreeMap`, struct, reference, and nested-aggregate parameters are reported as `"skipped"` with a reason (the native backend additionally rebuilds struct, enum, `Option`, `Result`, `Vec` and `String` parameters and every scalar width, see "Native backend" below). The self-hosted compiler's v1 reconstructs scalars only and reports `Vec` parameters as `"skipped"` (the Rust compiler additionally reconstructs bounded `Vec`s); both report identical outcomes for scalar and aggregate-skip cases. An entry file that already defines `main` replays like any other: both compilers set that `main` aside in the harness and call the failing function from a synthesized `main`. A counterexample for `main` itself is `"skipped"` (the harness cannot call the entry function).

`replay`/`replay_reason` are present on a counterexample only when `--replay-cex` was passed.

**Native backend.** Under `vowc verify --backend native` the replay contract is stricter, because the native model is meant to be checked against the runtime rather than trusted. The counterexamples for a division or remainder abort (zero divisor; signed `MIN / -1`), for an `unwrap()` on `None` or for a `Vec` index past the end (a read `v[i]`, a write `v[i] = x`, the same for a `Vec<i128>` or `Vec<u128>`), which carry the reserved unattributed `vow_id` and blame `None`, are replayed instead of skipped: the harness calls the function with the counterexample's inputs and the replay is `"confirmed"` when the program aborts with `ArithmeticOverflow` (division, remainder), `UnwrapOnNone` (unwrap) or `IndexOutOfBounds` (`Vec` index), exit status `134`. The runtime envelope names the abort kind but not the site, so this confirms the kind of failure for those inputs, not which operation raised it. Two obligations that share the violation text `index out of bounds` do not share a runtime outcome: a `String.byte_at` index past the end returns `-1` instead of aborting, and a shift count is masked, so those counterexamples stay `"skipped"` (`replay_reason` names the obligation) and are never reported as a verifier bug. A scalar parameter of any width (`bool`, `i8`..`i128`, `u8`..`u128`, `f64`) is rebuilt from its value in `values`, and so is a struct, enum, `Option` or `Result` parameter whose fields are scalars (the same types; `()` as a payload) or, recursively, structs, enums, `Option`s and `Result`s (up to 8 levels), from the `<param>.<slot>` values, where an aggregate held in slot `k` of `p` is an object whose slot `j` is the entry `p.k.j` (`o.1.0` is the first field of the struct an `Option` holds in `o`), a slot the model never read of a held object taking zero like a scalar field (see "Aggregates" above): the harness binds `Pair { lo: 3, hi: -4 }`, `Shape::Rect(5, 6)` or `Option::Some(9)` from the source declaration, giving a 128-bit field its two slots. A slot the model never read takes `0`/`false`, and an unused variant is never inspected. A `Vec` of `bool` or integers up to 64 bits and a `String` parameter are rebuilt from the `<param>.len` and `<param>[<i>]` values (see "Counterexample contents of `Vec` and `String` parameters" above): the harness builds the value in a block that pushes each element (`push_byte` for a `String`). An element the claim never read takes whatever the model gave it. A `Vec<f64>` parameter marked `<param>.elem` = `f64` is rebuilt from its decimal elements, spelled as the `f64` parameters are (`NaN` as `(0.0 / 0.0)`, a negative value as `(0.0 - x)`). The replay is `"skipped"`, with a reason, for an `f32` parameter or field (no source text denotes one), a `Vec` whose elements are not `bool`, integers up to 64 bits or (marked) `f64`, a `Vec` or `String` whose contents the counterexample does not list (length above 512), a `bool` element whose slot is neither 0 nor 1, a `Vec`/`String` field or any other non-scalar, non-aggregate type, an aggregate nested deeper than 8 levels, a reference, an alias or a type not declared in the program, and for a discriminant outside the enum's variants when every variant is one the program tests. The model does not constrain the tag, so the solver may pick one (`-1`); a run with such a tag fails every `tag == k` test, which a variant the program never tests also does, so replay builds the smallest variant no `match` arm or `unwrap` selects (the last arm of a `match` is reached either way and does not count) and skips only when every variant is tested; that is a property of the model, not a verifier bug. The ESBMC backend and the Rust compiler keep reporting aggregate parameters as `"skipped"`. A native counterexample whose replay ends `"diverged"` or `"aborted"` additionally yields one [`VerifierBug`](errors.md#verifierbug) error diagnostic: the model and the runtime disagree, so the defect is in the verifier. `"skipped"` never ran and is not reported as a verifier bug. The `status` and exit code are unchanged. The ESBMC backend keeps skipping the unattributed ids.

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

### Contract Fields

| Field         | Type    | Description                                              |
|---------------|---------|----------------------------------------------------------|
| `vow_id`      | integer | Unique contract identifier within the program            |
| `function`    | string  | Function containing this contract                        |
| `kind`        | string  | `"requires"`, `"ensures"`, or `"invariant"`              |
| `description` | string  | Full contract text                                       |
| `blame`       | string  | `"Caller"` (requires) or `"Callee"` (ensures/invariant)  |
| `source`      | object  | `{ "file": string, "offset": integer }`                  |
| `status`      | string  | `"proven"`, `"proven-ir"`, `"failed"`, `"unknown"`, `"timeout"`, `"error"`, `"not_verified"`, `"skipped"`, or `"vacuous"` |
| `quality`     | string  | Static clause-shape classification (no verifier): `"weak"`, `"tautological"`, or `"substantive"` |
| `trivially_satisfiable` | bool | `--verify` only: true when a trivial `return <default>` body still satisfies this `ensures` (verification-confirmed weakness). Always false for `requires`/`invariant` and without `--verify`. Informational — never affects the exit code. See `docs/spec/contracts-methodology.md`. |

`description` is the clause kind followed by the predicate rendered by the canonical
printer, and the two compilers produce byte-identical text. The same text is carried
into `VowViolation.description` and the counterexample `violation` field. Integer
literals print in unsigned decimal at full 128-bit width (a `u64` literal above
`i64::MAX` such as `18446744073709551614` is never shown as `-2`), a negated literal is
`-` followed by its magnitude, a suffixed literal prints as the cast it denotes
(`5u64` is `5 as u64`), a cast prints its real target type, and a nested binary
operator is parenthesized exactly when its precedence requires it. Block, `if`/`else`,
`match` and loop expressions print multi-line (so `description` may contain newlines):
4-space indentation per nesting level for statements, match arms and loop `vow` clauses,
`let` statements with their pattern and type annotation, and every closing brace aligned
with the line its block opened on, so a block nested inside another block, arm or loop
indents by its depth exactly as the canonical printer renders it.

`source.offset` anchors differently depending on where the clause comes from: for a
clause inside a `vow { ... }` block (`requires`, `ensures`, `invariant`), it is the byte
offset of the clause keyword, not the predicate expression, in both compilers. For a
parameter's inline `where` refinement (also reported as `kind: "requires"`), both
compilers anchor on the byte offset of the parameter name instead.

### Status Values

| Status          | Meaning                                              |
|-----------------|------------------------------------------------------|
| `not_verified`  | Verification not requested (no `--verify` flag)      |
| `proven`        | The verifier established this contract. ESBMC: for the configured bounded verification model (bit-vector encoding, overflow modeled). Native: for every unwinding and collection length, never after a resource-limited weakened retry |
| `proven-ir`     | ESBMC backend only, never produced natively (and not emitted as a clause status by either compiler today). ESBMC established this contract for the configured bounded model under integer-arithmetic encoding after BV timed out; overflow is not modeled by IR, but the BV caller preconditions still guard against it |
| `failed`        | The verifier found a counterexample violating this contract |
| `unknown`       | The verifier could not conclude for this contract — under the native backend a solver `unknown`, an unwinding failure (`unwinding assertion:`), a counterexample that depends on an opaque builtin result (`opaque builtin result:`) or on the keys or length of a map the function did not build (`unconstrained map state:`), a counterexample found with a callee summary (`callee summary:`), a budget or out-of-memory worker, or a function that fails only on an abort; under ESBMC either `VERIFICATION UNKNOWN` was reported for the containing function (the incremental-BMC forward condition was unable to prove or falsify), or the function's verification failed overall and ESBMC's per-clause `--multi-property` run returned no individual verdict for this clause |
| `timeout`       | The verifier timed out on the containing function (ESBMC: BV and — when applicable — IR fallback both timed out) |
| `error`         | Verifier error, or the tool was not found (ESBMC, or `bitwuzla` under `--backend native`) |
| `skipped`       | The containing function is outside what the verifier models, with a reason from the closed list in [`errors.md`](errors.md#verificationskipped) (e.g. effects, recursion, an unmodelled builtin, `Load`/`Store`). A machine-readable reason code is not emitted yet. Contract is documentary; runtime checks still apply under `--mode debug`. Surfaces as a `VerificationSkipped` Warning in the build JSON's `diagnostics[]` and lifts the overall build/verify status to `Skipped` (fail-closed, exit 1) — use `--no-verify` if you want a non-failing path that does not invoke a verifier at all. |
| `vacuous`       | The containing function's `requires` clauses are contradictory, so every `ensures` is satisfied vacuously — the verifier proved nothing of substance (antecedent failure). Detected by a second run: under ESBMC a `vow_reach` label (`--error-label`) planted after the `requires` assumes is unreachable; under `--backend native` the function is cut right after its last `requires` and the cut point must be unreachable. All of the function's clauses are reported `vacuous` (fail-closed, exit 1). See `docs/spec/contracts-methodology.md`. |

The `proven` / `proven-ir` split and the rule that a resource-limited retry (e.g. the BV→IR fallback) may never report a weakened check as `proven` are the verifier's soundness discipline — the safe-vs-unsafe retry rules are specified in `docs/verifier-discipline.md`.

### Quality Values

`quality` is a static classification of each clause's *shape*, computed without running a verifier and independent of `status`. It surfaces the "proven but trivial" problem: a `weak` contract can be `proven` while constraining almost nothing. See `docs/spec/contracts-methodology.md` for the full taxonomy.

| Quality        | Meaning                                                                                      |
|----------------|----------------------------------------------------------------------------------------------|
| `weak`         | An `ensures` that only bounds `result` by an integer literal (e.g. `result >= 0`). Satisfied by almost any implementation. |
| `tautological` | A constant clause that references no program value (e.g. `true`, `0 >= 0`). Constrains nothing. |
| `substantive`  | Everything else — equality, relational, inverse/round-trip, dispatch-totality, or function-call shapes. The classifier is conservative: anything not provably weak/tautological is reported `substantive`. |

`as` casts are transparent to both verdicts. `result >= 0 as i64` is `weak` exactly as `result >= 0` is, `result as i64 >= 0` folds the same way, and `0 as i64 >= 0` is `tautological` — a cast never makes a constant bound substantive. This matters because an agent can still wrap a literal bound in a cast (`0 as i64`) and the clause would otherwise read as `substantive`; without the fold a `weak` ratchet stops meaning anything. Only a primitive numeric target is folded, so `x as Foo` is left alone, and a cast never turns a program value into a literal (`result >= x as i64` stays `substantive`).

An always-true or always-false comparison against `0` on an unsigned — or zero-extended unsigned — operand is not a quality verdict at all: it is the hard [`TautologicalComparison`](errors.md#tautologicalcomparison) type error, so such a clause never reaches `vow contracts`.

## Trace Output (stderr, --debug-trace)

When `--debug-trace=calls` or `--debug-trace=full` is used, the compiled binary emits JSON lines to stderr:

### calls mode
```json
{"event":"enter","fn":"main"}
{"event":"enter","fn":"divide"}
{"event":"exit","fn":"divide"}
{"event":"exit","fn":"main"}
```

### full mode (adds vow check results)
```json
{"event":"enter","fn":"divide"}
{"event":"vow","fn":"divide","vow_id":0,"passed":true}
{"event":"exit","fn":"divide"}
```

## Profile Output (stderr, profile mode)

When `--mode profile` is used, the compiled binary prints a call-count report to stderr on normal exit (via `atexit`). The report is not printed if the program is killed by a signal or calls `abort()`.

```
--- vow profile report ---
function                                        calls       %
-------------------------------------------------------------
infer                                         4812399   48.2%
is_def_eq_core                                3201882   32.1%
whnf                                           984201    9.9%
main                                                1    0.0%
-------------------------------------------------------------
total calls: 9998483, unique functions: 12
```

The report lists the top 20 most-called functions sorted by call count. No vow checks are emitted in profile mode.

## Runtime Error JSON (stderr, debug/sanitize mode)

When a compiled program runs in debug mode (`--mode debug`) or sanitize mode (`--mode sanitize`) and violates a vow at runtime, it emits JSON to stderr before aborting.

Two of the diagnostics below are **mode-independent** and are emitted in release
builds as well: `UnwrapOnNone` and `IndexOutOfBounds`. Both report a partial
operation failing on its own terms rather than a vow check, so they are never
compiled out. Each is marked below.

### VowViolation

```json
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"divide.vow","offset":42,"values":{"y":0}}
```

Schema: [`schemas/vow-violation.schema.json`](schemas/vow-violation.schema.json).

`values` entries are bare JSON number tokens and are not bounded by `i64`:
`u64`, `i128`, and `u128` bindings render at full width, so a consumer that
acts on the exact value needs an arbitrary-precision number parser. See
[`errors.md`](errors.md#vowviolation).

### ArithmeticOverflow

```json
{"error":"ArithmeticOverflow"}
```

Emitted when a checked arithmetic operator (`+!`, `-!`, etc.) overflows, or
when checked or unchecked division/remainder has no result because the divisor
is zero or signed division evaluates `MIN / -1`.

### UnwrapOnNone

```json
{"error":"UnwrapOnNone"}
```

Emitted when `.unwrap()` is called on `Option::None` or `Result::Err`. Emitted in
**every build mode**, release included — `.unwrap()` is a partial operation, not a
vow check.

### IndexOutOfBounds

```json
{"error":"IndexOutOfBounds"}
```

Emitted when a `Vec` index is out of bounds. Emitted in **every build mode**,
release included.

### UseAfterFree (sanitize mode only)

```json
{"error":"UseAfterFree","op":"push","vec":"0x55a1b2c3d4e0"}
```

Emitted when a Vec or String operation is attempted after the region that owns it
has been closed. Vow has no per-value free, so closing a region is the only event that
reclaims a Vec's descriptor and backing store; a later access is a use-after-free and
means region analysis placed the value too deep. `op` is one of `push`, `set`, `pop`,
`clear`, `truncate`, or `read`. Exit code 134.

## Agent Decision Tree

```
Parse JSON from stdout
├── status == "Verified"       → Success. Binary at `executable`.
├── status == "Unverified"     → Compiled but unverified (--no-verify or --dump-ir).
├── status == "CompileFailed"  → Read `diagnostics[]` for error details.
│   ├── error_code is parse error  → Fix syntax (see grammar.md)
│   ├── error_code is type error   → Fix types (see errors.md)
│   ├── error_code == "CodegenUnsupported" → Avoid the named backend limitation.
│   ├── error_code == "CodegenFailed" → Inspect backend context; report a compiler bug if supported source triggered it.
│   └── error_code == "LinkFailed" → Check the host linker and Vow runtime archive.
└── status == "VerifyFailed"   → Read `counterexamples[]`.
    ├── Check `inputs` for the violating values
    ├── Check `violation` for which contract failed
    ├── Check `source` for the location
    └── Fix the contract or the implementation, then rebuild
```

Always check stderr for human-readable diagnostics alongside the JSON on stdout.
