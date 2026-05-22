# Vow Error Catalog

Every Vow error has a machine-readable `error_code` in the JSON output. This document lists all error codes, their phase, meaning, an example trigger, and how to fix them.

## Compile-Time Errors

These appear in the `diagnostics` array of the build output JSON.

### UnterminatedString

**Phase:** Lexer
**Meaning:** A string literal was opened with `"` but never closed.

```vow
fn f() -> () [io] {
    print_str("hello);
}
```

**Fix:** Close the string with a matching `"`.

### InvalidCharacter

**Phase:** Lexer
**Meaning:** The source contains a character the lexer does not recognize.

```vow
fn f() -> i64 {
    x @ y
}
```

**Fix:** Remove the invalid character. Vow has no `@` operator.

### UnexpectedToken

**Phase:** Parser
**Meaning:** The parser encountered a token it did not expect at that position.

```vow
module M 123
```

**Fix:** Check the syntax around the reported span. Common causes: missing `{`, `}`, `(`, `)`, or a keyword in the wrong position.

### MissingDelimiter

**Phase:** Parser
**Meaning:** A matching delimiter (`}`, `)`, `]`) is missing.

```vow
fn f() -> i64 {
    42
```

**Fix:** Add the missing closing delimiter.

### TypeMismatch

**Phase:** Type Checker
**Meaning:** An expression has a different type than expected.

```vow
fn f() -> i32 {
    true
}
```

**Output:** `function body has type 'bool' but declared return type is 'i32'`

**Fix:** Change the expression or the declared type to match.

### LiteralOutOfRange

**Phase:** Type Checker
**Meaning:** An integer literal appears in a typed context (annotated `let`, function argument, struct field, or const declaration) whose target type cannot hold the literal's value. The check runs after context coercion, so the offending literal is the one written in the source, not a widened intermediate.

```vow
let x: u8 = 300;
const NEG: u16 = -1;
```

**Output:** `literal 300 does not fit in u8 (range 0..=255)`

**Fix:** Use a value within the target type's range, change the target type, or write an explicit narrowing intrinsic (`i64_to_u8_try`, `i64_to_u8_wrap`, `i64_to_u8_sat`) if you intend to convert a wider value at runtime.

### NarrowingCastNotAllowed

**Phase:** Type Checker
**Meaning:** The `as` operator was used to convert a wider integer type to a narrower one. `as` is widening-only; narrowing must use a named intrinsic so the agent chooses an explicit semantics (range-checked vs. truncating vs. saturating). See `grammar.md` §Type Cast.

```vow
fn f(big: i64) -> u8 {
    big as u8
}
```

**Output:** `cannot cast 'i64' to 'u8' via 'as'; use 'i64_to_u8_try', 'i64_to_u8_wrap', or 'i64_to_u8_sat' to choose the narrowing semantics`

**Fix:** Replace the cast with the narrowing intrinsic that matches your intent:
- `i64_to_u8_try(big) -> Option<u8>` — reject out-of-range with `None`
- `i64_to_u8_wrap(big) -> u8` — truncate (keep low bits)
- `i64_to_u8_sat(big) -> u8` — clamp to `0..=255`

### ShiftCountOutOfRange

**Phase:** Type Checker
**Meaning:** A constant-expression shift count is greater than or equal to the bit-width of the left operand. Shifting an `N`-bit value by `>= N` bits is undefined in the underlying C model and is rejected at compile time when the count is statically known. Dynamic shift counts (non-const expressions) get a Vow contract on the operation and are checked by ESBMC and at runtime in debug mode.

```vow
fn f(x: u8) -> u8 {
    x << 8
}
```

**Output:** `shift count 8 is out of range for u8 (max 7)`

**Fix:** Use a count less than the LHS bit-width. To shift a narrow value by a larger amount, widen first: `((x as u32) << 12) as u8` is illegal under widening-only `as`; use `(x as u32) << 12` and then a narrowing intrinsic.

### StaticLiteralRequired

**Phase:** Type Checker
**Meaning:** A compiler intrinsic requires a string literal operand so it can be lowered without allocation.

```vow
fn f(s: String, key: String) -> i64 {
    string_matches_literal_at(s, 0, key)
}
```

**Output:** `string_matches_literal_at requires a string literal as its third argument`

**Fix:** Pass a literal directly, for example `string_matches_literal_at(s, 0, "name")`.

### EffectViolation

**Phase:** Type Checker
**Meaning:** A function calls another function with effects not declared in its own signature.

```vow
fn f() -> () {
    print_str("hi");
}
```

**Fix:** Add the required effect to the function signature: `fn f() -> () [io]`.

### LinearTypeViolation

**Phase:** Type Checker
**Meaning:** A value of a `linear struct` type is used in a way that is immediately invalid before region inference runs, such as consuming it twice, consuming it inside a loop that may execute more than once, or consuming it after only some control-flow paths already consumed it.

```vow
linear struct Handle { fd: i64 }

fn f(h: Handle) -> Handle {
    let h2: Handle = h;
    let h3: Handle = h;  // h was already consumed
    h2
}
```

**Fix:** Restructure ownership so each path uses a consumed linear value at most once. Obligations that are simply left live at scope exit are reported later as `RegionLinear`.

### RegionLinear

**Phase:** Region Inference
**Meaning:** A `linear struct` value can remain live when its owning region closes. Returning the value transfers the linear obligation to the caller; consuming it before the close satisfies the obligation.

```vow
linear struct Handle { fd: i64 }

fn f() -> i64 {
    let h: Handle = Handle { fd: 1 };
    0
}
```

**Fix:** Consume the value before the region closes, or return it so the caller receives the obligation.

### NonExhaustiveMatch

**Phase:** Type Checker
**Meaning:** A `match` expression does not cover all possible variants.

```vow
fn f(o: Option<i64>) -> i64 {
    match o {
        Option::Some(x) => x,
    }
}
```

**Fix:** Add a `_ => ...` wildcard arm or cover all variants (`Option::None => ...`).

### UnknownMethod

**Phase:** Type Checker
**Meaning:** A method call uses a name that does not exist on the receiver type.

```vow
fn f() -> () {
    let v: Vec<i64> = Vec::new();
    v.psh(42);
}
```

**Output:** `unknown method 'psh' on type 'Vec<i64>'`

**Fix:** Check the method name for typos. Use `--help` to see available methods for each type.

### UnsupportedFeature

**Phase:** Type Checker
**Meaning:** A language feature that is not supported in Vow was used.

```vow
trait Foo {
    fn bar() -> i64;
}
```

**Output:** `trait blocks are not supported in Vow`

**Fix:** Remove the unsupported construct. Vow does not support traits or impl blocks.

### BTreeMapKeyTypeMustBeI64

**Phase:** Type Checker
**Meaning:** A `BTreeMap<K, V>` was instantiated with `K` not equal to `i64`. Phase 1 of the BTreeMap stdlib only supports `i64` keys; the runtime helpers and ESBMC C model are hard-coded to i64.

```vow
fn f() -> () {
    let m: BTreeMap<bool, i64> = BTreeMap::new();
    m.insert(true, 1);
}
```

**Output:** `BTreeMap key type must be i64; found 'bool'`

**Fix:** Use `BTreeMap<i64, V>`. If you need string or struct keys, hash or intern them to `i64` at the call site and keep a side-table for the originals.

### BTreeMapValueTypeMustBeI64

**Phase:** Type Checker
**Meaning:** A `BTreeMap<K, V>` was instantiated with `V` not equal to `i64`. Phase 1 only supports `i64` values; the runtime helpers and ESBMC C model are hard-coded to i64 values. Widening V to struct payloads is a planned follow-up to the BTreeMap stdlib work.

```vow
fn f() -> () {
    let n: BTreeMap<i64, String> = BTreeMap::new();
}
```

**Output:** `BTreeMap value type must be i64 in Phase 1; found 'String'`

**Fix:** Use `BTreeMap<i64, i64>`. For richer values, store an integer index/handle and keep the actual values in a separate `Vec<V>`.

### MissingContract

**Phase:** Type Checker
**Meaning:** An `extern "C"` block was declared without a `vow { ... }` contract. Every foreign function call requires a mandatory contract specifying expected behavior.

```vow
extern "C" {
    fn write(fd: i32, ptr: i64, len: i64) -> i64 [io];
}
```

**Output:** `extern block requires a vow contract`

**Fix:** Add a `vow { ... }` block to the extern declaration with `requires` and/or `ensures` clauses.

### ContractTypeMismatch

**Phase:** Type Checker
**Meaning:** A `requires`, `ensures`, or `invariant` clause expression does not have type `bool`.

```vow
fn add(a: i64, b: i64) -> i64 vow {
    requires: a + b
} {
    a + b
}
```

**Output:** `` `requires` clause has type `i64` but must be `bool` ``

**Fix:** Ensure every contract clause is a boolean expression (comparison, logical operator, or a call to a predicate function returning `bool`).

### VowRequiresViolated

**Phase:** Verification (ESBMC)
**Meaning:** ESBMC found inputs that violate a `requires` precondition. This is a **static** verification error — it means the function's callers can reach it with invalid arguments.

**Fix:** Strengthen the `requires` clause, or fix the callers to pass valid arguments.

### VowEnsuresViolated

**Phase:** Verification (ESBMC)
**Meaning:** ESBMC found inputs where the function's return value does not satisfy the `ensures` postcondition.

**Fix:** Fix the function body to satisfy the postcondition, or weaken the `ensures` clause.

### VowInvariantViolated

**Phase:** Verification (ESBMC)
**Meaning:** ESBMC found a loop iteration where the `invariant` does not hold.

**Fix:** Strengthen the invariant or fix the loop body.

### EsbmcNotFound

**Phase:** Verification
**Meaning:** ESBMC is not installed or not on `$PATH`. When verification is enabled (the default for `vowc build`, always for `vowc verify`), the compiler checks for ESBMC upfront before compilation. If ESBMC is not found, the build aborts immediately with exit code 1.

**Fix:** Install ESBMC, or use `--no-verify` to skip verification: `vowc build --no-verify <file>`.

### RegionConflict

**Phase:** Region Inference (arena-per-scope, Phase 3)
**Meaning:** A heap-typed value's required lifetime cannot be satisfied by the regions the surrounding code provides. This fires when an interprocedural store-effect constraint is unsatisfiable against the **inferred** region — that is, the value's `region(I) = LUB(must_outlive(I))` resolves to a concrete block strictly narrower than the target container's region.

> **Coverage note (as of issue #314):** the check is now semantic, consulting
> the inferred region populated by §4.1 step 3's LUB pass rather than the
> raw IR opcode. A fresh allocation routed through a callee's store-effect
> chain into a parameter container has its inferred region widened to
> `Caller(HiddenRegionIdx(N))` by §4.1 step 2's must-outlive marker
> propagation, where `N` is the precise slot index implied by the
> destination (issue #317 slot-aware inference). Such single-slot routings
> satisfy the constraint and are accepted. Allocations whose caller-region
> markers require more than one hidden caller-arena slot resolve to
> `Caller(HiddenRegionIdx::AMBIGUOUS)` and are rejected when the directly
> fresh heap value is stored into a parameter-rooted target; allocations
> whose inferred region is a strictly narrower block also fire
> `RegionConflict`.

```vow
fn store_into(out: Vec<String>, prefix: String) [io] {
    let s: String = String::from(prefix);
    s.push_str(String::from(" world"));
    out.push(s);  // s is allocated in this function's scope but escapes into out's region
}
```

**Fix:** Move the allocation to a wider scope, or copy the value into the target region (e.g., `String::from(s)` into the outer arena). For routings that compile cleanly but you'd like to know about (root-region placement), see `RegionRootEscape` below. See `docs/design/arena_memory.md` §4.4 for the full rejection vs. visibility distinction.

### RegionRootEscape

**Phase:** Region Inference (arena-per-scope, Phase 3)
**Severity:** Note (informational — does not fail the build)
**Meaning:** A heap allocation's inferred region is `Caller`, and the surrounding function publishes a `FreshInCaller` return summary or store effect — so the allocation may flow up the caller chain to `main` and ultimately land in the root region (`__vow_root_arena`, never freed). This is a memory-cost decision the compiler surfaces visibly per `docs/design/arena_memory.md` §4.4: silent root-region placement caused growth-with-no-signal in earlier compiler versions, and the note restores that signal without conflating it with unsoundness (`RegionConflict`).

The note is conservative — it fires for any `Caller`-region allocation in a function that could route to a caller, even if the actual concrete chain in this program doesn't reach `main`. False positives are tolerated because the diagnostic is non-blocking.

```json
{
  "error_code": "RegionRootEscape",
  "severity": "note",
  "message": "allocation may live in the root region: routed via store-effect chain to a caller whose target_region ultimately resolves to root",
  "hints": [
    "if intentional (e.g. program-lifetime data), no action needed; if you want this allocation freed earlier, restructure so the value is returned rather than stored into a parameter container"
  ]
}
```

**Fix:** Often none — if the program is short-lived (a checker, a CLI tool) or the values are genuinely program-lifetime, the note is informational. To free the allocation earlier, restructure so the value is **returned** from the constructing function rather than stored into a parameter container; the canonical `FreshInCaller` return path (`fn make_X() -> X`) does not trigger the note for the returned value or any allocation installed as a field of the returned struct (e.g. `Item { name: String::from("hi") }`). The exemption applies only to the *currently-installed* field initializers — a field overwritten before the return (`x.f = A; x.f = B; return x`) does not suppress the dead allocation `A`, which fires the note as expected (per-block last-write dedup, issue #326).

### VerificationSkipped

**Phase:** Verification (Warning surfaced alongside `BuildStatus::Skipped`)
**Meaning:** The function carries a `vow {}` block but its body uses opcodes the verifier's C model cannot represent — most commonly `RegionAlloc` and `FieldSet` produced by struct construction, also `Load`/`Store`, `RemF*`, and the `Linear*` family. The function is skipped before any C is emitted or ESBMC is invoked. The contract becomes documentary: runtime checks still apply in `--mode debug`, but no static proof is attempted.

```json
{
  "error_code": "VerificationSkipped",
  "severity": "warning",
  "message": "skipped verification of `ir_inst_set_region`: function `ir_inst_set_region` is not modelable in the verifier (contains unsupported opcode `RegionAlloc`)",
  "hints": [
    "the contract is documentary; runtime checks still apply in --mode debug"
  ]
}
```

**Why the build fails closed.** Per `CLAUDE.md`'s "Contract Authoring" guidance, contracts express semantic correctness and must not be weakened to fit the verifier. When the verifier's bounded model checker cannot represent a function's body, the function is skipped with a structured warning instead of tripping the defense-in-depth `__ESBMC_assert(0, "vow:UNSUPPORTED_OP_VOW_ID")` that historically broke the bootstrap on every vowed struct-builder. But a skipped contract is still an unproved contract, so the build lifts its overall status to `Skipped` (exit 1). Use `--no-verify` if you explicitly want a non-failing path that does not invoke ESBMC at all (`Unverified`, exit 0).

**Fix:** Refactor the function so its body uses only modelable opcodes — typically by splitting allocation/initialisation away from the contract-bearing computation. Alternatively, run with `--no-verify` if the contract is intentionally documentary.

## Runtime Errors

These are emitted to stderr as JSON when a compiled program runs (debug mode for VowViolation).

### VowViolation

**When:** Debug mode only (`--mode debug`). A `requires`, `ensures`, or `invariant` predicate evaluates to false at runtime.

```json
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"divide.vow","offset":42,"values":{"y":0}}
```

The `blame` field indicates who is at fault:
- `Caller` — a `requires` was violated (the caller passed bad arguments)
- `Callee` — an `ensures` or `invariant` was violated (the function has a bug)

**Fix:** See the `description` and `values` fields to understand which predicate failed and with what runtime values.

### ArithmeticOverflow

**When:** A checked arithmetic operator (`+!`, `-!`, `*!`, `/!`, `%!`) overflows at runtime.

```json
{"error":"ArithmeticOverflow"}
```

**Fix:** Use wrapping arithmetic (`+`, `-`, etc.) if overflow is acceptable, or add bounds contracts to prevent overflow.

### UnwrapOnNone

**When:** `.unwrap()` is called on `Option::None`.

```json
{"error":"UnwrapOnNone"}
```

**Fix:** Use `match` to handle `None`, or add contracts that guarantee the value is `Some`.

### IndexOutOfBounds

**When:** A `Vec` index access (`v[i]` or `v[i] = val`) uses an index outside `0..v.len()`.

```json
{"error":"IndexOutOfBounds"}
```

**Fix:** Add a bounds check before indexing, or add contracts: `requires: i >= 0, requires: i < v.len()`.

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

**Fix:** Obtain an explicit mutable copy before mutation: `String::from(value)`, or construct a fresh mutable container and copy the entries you need before mutating.

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

**Fix:** Add a base case to recursive functions, or restructure the algorithm to use iteration instead of recursion.

### OutOfMemory

**When:** A runtime arena operation (`__vow_arena_open` or `__vow_arena_alloc`) failed because the underlying `malloc` returned null. Non-recoverable from within Vow (`docs/design/arena_memory.md` §3.3, §16).

```json
{"error":"OutOfMemory","operation":"arena_alloc"}
```

The `operation` field is `arena_open` for the initial chunk allocation or `arena_alloc` for a later fallback chunk allocation.

**Fix:** Reduce working-set size, raise the process memory limit, or run on a machine with more memory. This is not a Vow program error.

## Warnings

### LoweringWarning

**Phase:** IR Lowering
**Meaning:** The IR lowerer could not resolve a struct type tag or field name, defaulting to index 0. This usually indicates a missing type annotation on a `let` binding, causing the compiler to lose track of which struct type a pointer refers to.

**Fix:** Add an explicit type annotation: `let x: MyStruct = ...;` so the compiler can track struct type tags through the IR.
