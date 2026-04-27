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
**Meaning:** A value of a `linear struct` type was not consumed exactly once.

```vow
linear struct Handle { fd: i64 }

fn f() -> () {
    let h: Handle = Handle { fd: 1 };
}
```

**Fix:** Ensure every linear value is consumed (passed to a function, returned, or destructured) exactly once.

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
**Meaning:** A heap-typed value's required lifetime cannot be satisfied by the regions the surrounding code provides. This fires when an interprocedural store-effect constraint is unsatisfiable — for example, a value allocated in an inner block is stored into a container that outlives that block.

> **Coverage note (as of Phase 5):** the alloc→param-via-callee shape is
> detected and emits this code with `Blame: Callee` and a hint pointing
> at the three resolution strategies (copy into outer arena, hoist the
> allocation, or restructure the data flow). Cross-parameter cases that
> require a published store-effect, Phi-of-mixed-origins, and the full
> block-tree LUB stay deferred to Phase 9 (issue #204). The spec shape
> below is stable; the residual cases above silently promote to a
> conservative `Root` region today rather than emitting a diagnostic.

```vow
fn store_into(out: Vec<String>, prefix: String) [io] {
    let s: String = String::from(prefix);
    s.push_str(String::from(" world"));
    out.push(s);  // s is allocated in this function's scope but escapes into out's region
}
```

**Fix:** Move the allocation to a wider scope, or copy the value into the target region (e.g., `String::from(s)` into the outer arena). The compiler does NOT silently promote values to the root region — see `docs/design/arena_memory.md` §4.4.

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

**When:** A `Vec`, `String`, or `HashMap` mutation is attempted on a literal-backed container — one whose descriptor carries the `VOW_CAP_RODATA` sentinel (backing lives in `.rodata` or was pinned to the root region). See `docs/design/arena_memory.md` §6.1, §7.3.

```json
{"error":"RegionLiteralMutation","operation":"String::push_str","origin":"rodata"}
```

A plain-text hint follows on the next line (not a JSON field). The hint text is dispatched on the operation's type prefix:

```
hint: use String::from(literal) to obtain a mutable copy       # for String::* operations
hint: use Vec::from(literal) to obtain a mutable copy          # for Vec::*    operations
hint: construct a mutable HashMap and copy entries before mutating  # for HashMap::* operations
```

The `operation` field identifies the source-level method that trapped (e.g., `Vec::push`, `Vec::pop`, `HashMap::insert`, `String::clear`). The `origin` field identifies the storage class of the immutable backing; today only `rodata` is emitted.

**Fix:** Obtain an explicit mutable copy before mutation: `String::from(literal)`, `Vec::from(literal)`, etc.

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
