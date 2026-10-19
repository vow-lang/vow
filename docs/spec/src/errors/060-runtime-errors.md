
### VerifierBug

**Phase:** Verification (self-hosted `vowc verify --backend native --replay-cex` only)
**Meaning:** Under the native backend, a counterexample did not replay: the `--mode debug` harness built from its concrete inputs did not reproduce the predicted `VowViolation` (same `vow_id` and blame) or, for a division or remainder abort, the predicted `ArithmeticOverflow` abort, or, for an `unwrap()` abort, the predicted `UnwrapOnNone` abort, or, for a `Vec` index past the end, the predicted `IndexOutOfBounds` abort. The counterexample's `replay` is `"diverged"` or `"aborted"` and `replay_reason` says why. A `"skipped"` replay (a harness limitation such as an unsupported parameter type) never ran and is not reported. The verifier model and the runtime disagree, so this is a defect in the verifier, not in the program. One diagnostic is emitted per such counterexample; the `status` stays `VerifyFailed` and the exit code stays 1.

```json
{
  "error_code": "VerifierBug",
  "severity": "error",
  "message": "native verifier counterexample for `rem` did not replay (diverged: harness exited cleanly; the predicted ArithmeticOverflow abort did not occur)",
  "hints": [
    "a counterexample that does not reproduce at runtime is a bug in the verifier model; report it"
  ]
}
```

**Fix:** Report the function, its counterexample `values` and `replay_reason` as a verifier bug. Do not weaken the contract to avoid the counterexample on the strength of this diagnostic alone.

**Right:** No source rewrite applies; the program and its contract are not at fault. Keep both unchanged and report the bug.

### ModelCapacityAssumed

**Phase:** Verification (Note; the build status stays `Verified`). **ESBMC backend only:** never emitted under `--backend native`, which models collections with a symbolic length and no capacity. The diagnostic is retired together with the ESBMC backend.
**Meaning:** The function's contracts were proved, but the proof's model restricts at least one capacity to the verifier's model capacity (defaults: `Vec<T>` 128, `String` 256, `HashMap<K, V>` and `BTreeMap<K, V>` 64, user-struct `Heap` 1024 slots; the `String` capacity is raised to the longest string literal in the module, and the note's message reports the capacity actually applied; see [`contracts.md`](contracts.md#collection-models-for-verification)). A parameter, struct-field collection, `String::from_cstr` result, or non-constant `from_raw_parts_copy` length is modelled as nondeterministic *within* its capacity, and struct allocations stop at the heap capacity, so executions with longer collections or more struct allocations were pruned, not checked: `Verified` means "verified for executions up to the capacities".

**Wrong:**

```vow
fn first_or_zero(v: Vec<i64>) -> i64 vow {
    requires: v.len() <= 128
    ensures: v.len() > 0 || result == 0
} {
    if v.len() > 0 {
        v[0]
    } else {
        0
    }
}
```

```json
{
  "error_code": "ModelCapacityAssumed",
  "severity": "note",
  "message": "proof of `first_or_zero` is bounded: only executions within the verifier model capacity were checked (Vec<T>: 128)",
  "hints": [
    "the bound belongs to the bounded model checker, not to the program or its contracts; executions beyond it (longer collections, more struct allocations) were not checked",
    "do not add a length bound to a contract to silence this note; an unbounded verifier removes it without any source change"
  ]
}
```

The `message` lists every bounded kind as `Kind: capacity`, in the fixed order `Vec<T>`, `String`, `HashMap<K, V>`, `BTreeMap<K, V>`, `Heap`, one entry per kind. When a model applied several capacities to one kind, the smallest is listed: the proof is claimed only for executions within every bound the model applied. At most one note is emitted per proved function. The span is empty (the note describes the model, not a source location).

**Why this is a Note and not a failure.** The verdict is genuine for every execution inside the bound and the verifier never changes a verdict because of the note. It is informational so that an agent or CEGIS loop can tell a bounded proof from an unbounded one — for example, to treat a `Verified` function that handles untrusted-length collections with extra scrutiny, or to test it with the runtime checks of `--mode debug`.

**Fix:** None is required. Do **not** add `requires: v.len() <= 128` (or any other capacity) to silence it: that distorts a true contract to fit a prover limitation, and an unbounded backend removes the note without any source change.

**Right:** Remove the length bound and keep the note; the source needs no other change.

```vow
fn first_or_zero(v: Vec<i64>) -> i64 vow {
    ensures: v.len() > 0 || result == 0
} {
    if v.len() > 0 {
        v[0]
    } else {
        0
    }
}
```

**Ordering and presence.** Notes describe a *completed* proof run, so they are emitted only when the overall `status` is `Verified`, one per proved function, after every other diagnostic and in function declaration order. A run that ends in a counterexample, `unknown`, `timeout`, `error`, or a fail-closed skip carries no note: a halt cuts the set of verified functions short in a scheduling-dependent way, and a note about one of them would be noise next to the real failure.

**Not emitted when:** the function's model restricts no capacity (scalar-only functions that allocate no struct, and collections built by the function itself from constant lengths or from a null source), or the overall status is not `Verified`. The same note is emitted by `vow verify` and `vow build`.

## Runtime Errors

These are emitted to stderr as JSON when a compiled program runs (debug mode for VowViolation).

**Exit status.** Every runtime abort below terminates the process with the reserved exit status **`134`** (128 + `SIGABRT`, the conventional "aborted" status), never a plain `1`. A runtime abort is an environment or soundness failure, not an application result. `134` is **reserved for aborts by convention**: a runtime abort never *spontaneously* collides with an application's own `return N` from `main`, so a program that does not itself return — or `process_exit` — `134` can treat any `134` exit as a runtime abort (a checker that returns `0`/`1`/`2` for accepted/rejected/declined will never mistake an out-of-memory or a contract violation for a genuine "rejected"). The runtime does not *enforce* the reservation — `process_exit(134)` and `return 134i32` still exit `134` — so a program that deliberately uses `134` opts out of the distinction; applications that care should reserve around it. The JSON envelope on stderr still names the specific abort. This is separate from the *compiler* exit codes in [`cli.md`](cli.md), which describe `vowc build`/`vowc verify`.

### VowViolation

**When:** Debug mode only (`--mode debug`). A `requires`, `ensures`, or `invariant` predicate evaluates to false at runtime.

```json
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"divide.vow","offset":42,"values":{"y":0}}
```

`file` is the source file that defines the violated clause (the module file for a
clause in an imported module, not the entry file) and `offset` is the byte offset
at which that clause starts. Both compilers report the same `file` and `offset`
for the same program, never an empty placeholder.

The `blame` field indicates who is at fault:
- `Caller` — a `requires` was violated (the caller passed bad arguments)
- `Callee` — an `ensures` or `invariant` was violated (the function has a bug)

**Wide integer values.** `values` entries are bare JSON number tokens and are
not bounded by `i64`. A `u64`, `i128`, or `u128` binding reports its full
magnitude, which can exceed the range an IEEE-754 double represents exactly —
`u128::MAX` renders as `340282366920938463463374607431768211455`. A consumer
that acts on the exact value must read it with an arbitrary-precision JSON
number parser; a default `double` parser silently rounds it.

**Wrong:**

```vow
fn divide(a: i64, b: i64) -> i64 vow {
    requires: b != 0
} {
    a / b
}

fn main() -> i32 {
    let q: i64 = divide(10, 0);
    0
}
```

**Fix:** See the `description` and `values` fields to understand which predicate failed and with what runtime values.

**Right:** (`divide` unchanged)

```vow
fn main() -> i32 {
    let q: i64 = divide(10, 2);
    0
}
```

### ArithmeticOverflow

Not gated by `[panic]`: checked operators are modelled as aborting in pure functions.

**When:** A checked arithmetic operator (`+!`, `-!`, `*!`, `/!`, `%!`)
overflows at runtime; or checked or unchecked division/remainder encounters a
zero divisor; or signed `/` or `/!` evaluates `MIN / -1`.

```json
{"error":"ArithmeticOverflow"}
```

**Fix:** For addition, subtraction, or multiplication, use wrapping arithmetic
if overflow is acceptable, or add bounds contracts to prevent overflow. For
division and remainder, rule out a zero divisor and, for signed division, the
`MIN / -1` case.

**Wrong:**

```vow
fn main() -> i32 {
    let a: i64 = 9223372036854775807;
    let b: i64 = a +! 1;
    0
}
```

**Right:**

```vow
fn main() -> i32 {
    let a: i64 = 9223372036854775807;
    if a < 9223372036854775807 {
        let b: i64 = a +! 1;
    }
    0
}
```

The abort is emitted in every build mode, release included, so this cannot be
deferred to a debug run. A checked operator's abort is part of the operator's
meaning, not a debug-mode check. Division and remainder also abort when no
result exists, regardless of whether their checked or unchecked spelling is
used.

### UnwrapOnNone

**When:** `.unwrap()` is called on `Option::None` or `Result::Err`.

```json
{"error":"UnwrapOnNone"}
```

**Wrong:**

```vow
fn main() -> i32 [panic] {
    let o: Option<i64> = Option::None;
    let x: i64 = o.unwrap();
    0
}
```

**Fix:** Use `match` to handle the empty variant — `None` for `Option`, `Err` for
`Result` — or add contracts that guarantee the value is `Some`/`Ok`.

**Right:**

```vow
fn main() -> i32 {
    let o: Option<i64> = Option::None;
    match o {
        Option::Some(x) => x,
        Option::None => 0,
    };
    0
}
```

The abort is emitted in every build mode, release included, so this cannot be
deferred to a debug run.

### IndexOutOfBounds

Not gated by `[panic]`: the abort is a verification obligation of the (pure) indexing function.

**When:** A `Vec` index access (`v[i]` or `v[i] = val`) uses an index outside `0..v.len()`.

```json
{"error":"IndexOutOfBounds"}
```

**Wrong:**

```vow
fn main() -> i32 {
    let v: Vec<i64> = Vec::new();
    let x: i64 = v[0];
    0
}
```

**Fix:** Add a bounds check before indexing, or add a contract: `requires: i < v.len()`. Do not pair it with `requires: i >= 0` — on an unsigned index that clause is always true and is rejected as [TautologicalComparison](#tautologicalcomparison).

**Right:**

```vow
fn main() -> i32 {
    let v: Vec<i64> = Vec::new();
    if v.len() > 0 {
        let x: i64 = v[0];
    }
    0
}
```

### RegionLiteralMutation

**When:** A `Vec`, `String`, or `HashMap` mutation is attempted on a literal-backed container — one whose descriptor carries the `VOW_CAP_RODATA` sentinel (backing lives in `.rodata` or was pinned to the root region). Calls that statically trace a mutating target to a literal are rejected during compilation with this code; a runtime fallback emits the JSON shape below if an unchecked mutation reaches a `VOW_CAP_RODATA` descriptor. See `docs/design/arena_memory.md` §6.1, §7.3.

```json
{"error":"RegionLiteralMutation","operation":"String::push_str","origin":"rodata"}
```

A plain-text hint follows on the next line (not a JSON field). The hint text is dispatched on the operation's type prefix:

```
hint: make an explicit mutable copy with String::from(value) before mutating  # for String::* operations
hint: construct a mutable Vec and copy entries before mutating                # for Vec::*    operations
hint: construct a mutable HashMap and copy entries before mutating  # for HashMap::* operations
```

The `operation` field identifies the source-level method that trapped (e.g., `Vec::push`, `Vec::pop`, `HashMap::insert`, `String::clear`). The `origin` field identifies the storage class of the immutable backing; today only `rodata` is emitted.

**Wrong:**

```vow
fn main() -> i32 {
    let s: String = "abc";
    s.push_str("d");
    0
}
```

**Fix:** Obtain an explicit mutable copy before mutation: `String::from(value)`, or construct a fresh mutable container and copy the entries you need before mutating.

**Right:**

```vow
fn main() -> i32 {
    let s: String = String::from("abc");
    s.push_str("d");
    0
}
```

### StackOverflow

**When:** The native call stack is exhausted, typically due to unbounded recursion.

```json
{"error":"StackOverflow"}
```

In debug or sanitize mode, the diagnostic includes call depth and the function that was executing when the overflow occurred:

```json
{"error":"StackOverflow","depth":10693,"function":"recurse"}
```

The signal handler is installed in **all** build modes. The `depth` and `function` fields are only available in debug/sanitize mode where call-depth instrumentation is emitted.

**Wrong:**

```vow
fn count_down(n: u64) -> u64 {
    count_down(n + 1)
}

fn main() -> i32 {
    let r: u64 = count_down(1);
    0
}
```

**Fix:** Add a base case to recursive functions, or restructure the algorithm to use iteration instead of recursion.

**Right:**

```vow
fn count_down(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    count_down(n - 1)
}

fn main() -> i32 {
    let r: u64 = count_down(1);
    0
}
```

### OutOfMemory

**When:** A runtime arena operation (`__vow_arena_open` or `__vow_arena_alloc`) failed because the underlying `malloc` returned null. Non-recoverable from within Vow (`docs/design/arena_memory.md` §3.3, §16).

```json
{"error":"OutOfMemory","operation":"arena_alloc"}
```

The `operation` field is `arena_open` for the initial chunk allocation or `arena_alloc` for a later fallback chunk allocation.

Like every runtime abort, an OOM exits with the reserved status **`134`** (see *Exit status* above), so it is distinguishable from an application's own `exit 1`.

**Fix:** Reduce working-set size, raise the process memory limit, or run on a machine with more memory. This is not a Vow program error.

**Right:** No source rewrite applies; the program is not at fault.

## Warnings

### LoweringWarning

**Phase:** IR Lowering
**Meaning:** The IR lowerer could not resolve a struct type tag or field name, defaulting to index 0. This usually indicates a missing type annotation on a `let` binding, causing the compiler to lose track of which struct type a pointer refers to.

**Fix:** Add an explicit type annotation: `let x: MyStruct = ...;` so the compiler can track struct type tags through the IR.

**Right:**

```vow
struct MyStruct { a: i64 }

fn f() -> i64 {
    let x: MyStruct = MyStruct { a: 1 };
    x.a
}
```
