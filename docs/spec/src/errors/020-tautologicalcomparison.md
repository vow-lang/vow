
### TautologicalComparison

**Phase:** Type Checker
**Meaning:** A comparison against a constant `0` can never fail (or can never hold) because the other operand has an unsigned integer type. `x >= 0` and `0 <= x` are always true for unsigned `x`; `x < 0` and `0 > x` are always false. Such a clause admits every implementation while reading like a real bound, so it is rejected rather than warned about — there is no suppression form. The const fold sees through `as` casts, so `x >= 0 as u64` is caught too.

`x > 0`, `x <= 0`, `x == 0` and `x != 0` all constrain an unsigned value and keep compiling. Signed operands are untouched.

**Wrong:**

```vow
fn sum(n: u64) -> u64 vow {
    ensures: result >= 0
} {
    n
}
```

**Output:** ``comparison with 0 is always true for unsigned type `u64` ``

**Widening casts do not launder the tautology.** A cast can move the compared type to a signed one while leaving the value provably non-negative, so the rule follows the value, not the printed type. Because `as` is widening-only plus same-width signedness change (see [Type Cast](grammar.md#type-cast)), the condition is exact: the comparison is tautological iff the source is unsigned **and** the target is strictly wider, i.e. the cast zero-extends.

```vow
fn f(x: u32) -> bool { (x as i64) >= 0 }   // error — u32 zero-extends into i64
fn g(x: u64) -> bool { (x as i64) >= 0 }   // ok — same width, `u64::MAX as i64` is -1
fn h(x: i32) -> bool { (x as i64) >= 0 }   // ok — a signed source sign-extends
```

A chain does not hide it either: `(x as i32 as i64) >= 0` with `x: u32` is rejected, and the diagnostic names the original unsigned source.

**Output:** ``comparison with 0 is always true for `u32` zero-extended to `i64` ``

**Fix:** Drop the dead clause, or state the bound you actually meant. An index invariant becomes `invariant: i < v.len()`; a lower bound that matters on an unsigned counter is `i > 0`, not `i >= 0`. If the subject really can be negative, its type should be signed.

**Right:**

```vow
fn sum(n: u64) -> u64 vow {
    ensures: result == n
} {
    n
}
```

### StaticLiteralRequired

**Phase:** Type Checker
**Meaning:** A compiler intrinsic requires a string literal operand so it can be lowered without allocation.

**Wrong:**

```vow
fn f(s: String, key: String) -> i64 {
    string_matches_literal_at(s, 0, key)
}
```

**Output:** `string_matches_literal_at requires a string literal as its third argument`

**Fix:** Pass a literal directly, for example `string_matches_literal_at(s, 0, "name")`.

**Right:**

```vow
fn f(s: String) -> i64 {
    string_matches_literal_at(s, 0, "name")
}
```

### EffectViolation

**Phase:** Type Checker
**Meaning:** A function calls another function with effects not declared in its own signature, **or** a `requires`/`ensures`/`invariant` clause or parameter `where` clause calls a function or builtin method that writes through one of its arguments. The second form is blamed on the callee, even in a `requires` clause, because the clause itself is at fault for evaluating the write — not the caller.

**Wrong:**

```vow
fn f() -> () {
    print_str("hi");
}
```

**Fix:** Add the required effect to the function signature: `fn f() -> () [io]`. Effects are independent: `[io]` does not cover `read` or `write`, so a function that prints and reads a file declares `[io, read]`.

**Right:**

```vow
fn f() -> () [io] {
    print_str("hi");
}
```

**Wrong:**

```vow
fn mark(p: Point) -> bool {
    p.x = 1;
    true
}

fn make_point(x: i64, y: i64) -> Point vow {
    ensures: mark(result)
} {
    Point { x: x, y: y }
}
```

**Fix:** Move the write out of the clause — contracts may only read, e.g. `ensures: result.x == x`. See `docs/spec/grammar.md` → "Contract Purity".

**Right:**

```vow
struct Point { x: i64, y: i64 }

fn make_point(x: i64, y: i64) -> Point vow {
    ensures: result.x == x
} {
    Point { x: x, y: y }
}
```

### LinearTypeViolation

**Phase:** Type Checker
**Meaning:** A linear owner value is used in a way that is immediately invalid,
such as consuming it twice, consuming a value declared outside a loop from
inside that loop (it may execute more than once), or consuming it after only
some control-flow paths already consumed it. A value declared inside the loop
body and consumed there is fresh each iteration and is accepted. The error also
rejects a struct field whose type owns a linear obligation, because field
access has no move-out semantics and repeated reads could expose that
obligation more than once. Linear owners include `linear struct` values and owned enum,
`Option`, or `Result` wrappers that transitively contain one; matching such a
wrapper consumes it and transfers the obligation to the selected payload. An
unbound `_` catchall also violates the rule while any unhandled enum variant can
carry a linear payload.

**Wrong:**

```vow
linear struct Handle { fd: i64 }

fn f(h: Handle) -> () {
    drop(h);
    drop(h);  // h was already consumed
}
```

**Fix:** Restructure ownership so each path uses a consumed linear value at most
once. Discharge a value that is no longer needed with `drop(value)`. Keep linear owners out of struct fields until move-out field access is
supported. In a match, add explicit arms that bind and consume or transfer every
linear payload before using `_`. Obligations that are simply left live at scope
exit are reported later as `RegionLinear`.

**Right:**

```vow
linear struct Handle { fd: i64 }

fn f(h: Handle) -> () {
    drop(h);
}
```

### RegionLinear

**Phase:** Region Inference
**Meaning:** A `linear struct` value can remain live when its owning region closes. Returning the value transfers the linear obligation to the caller; consuming it before the close satisfies the obligation.

**Wrong:**

```vow
linear struct Handle { fd: i64 }

fn f() -> i64 {
    let h: Handle = Handle { fd: 1 };
    0
}
```

**Fix:** Consume the value before the region closes, or return it so the caller receives the obligation.

**Right:**

```vow
linear struct Handle { fd: i64 }

fn f() -> Handle {
    let h: Handle = Handle { fd: 1 };
    h
}
```

### NonExhaustiveMatch

**Phase:** Type Checker
**Meaning:** A `match` expression does not cover all possible variants.

**Wrong:**

```vow
fn f(o: Option<i64>) -> i64 {
    match o {
        Option::Some(x) => x,
    }
}
```

**Fix:** Add a `_ => ...` wildcard arm or cover all variants (`Option::None => ...`).

**Right:**

```vow
fn f(o: Option<i64>) -> i64 {
    match o {
        Option::Some(x) => x,
        Option::None => 0,
    }
}
```

### UnsupportedPattern

**Phase:** Type Checker
**Meaning:** A parsed `match` pattern or scrutinee, or a `let` pattern, is not
in the subset that the compiler can lower safely.

Match currently accepts enum-valued scrutinees, qualified unit variants,
qualified tuple variants with `_` or immutable identifier payloads, and final
catchall `_` or immutable identifier arms. A tuple-variant pattern must bind
exactly the number of payloads declared by the variant.

**Wrong:**

```vow
fn f(n: i64) -> i64 {
    match n {
        0 => 5,
        _ => 9,
    }
}
```

**Output:** `literal match patterns are not supported`

**Fix:** Use `if`/`else` comparisons for scalar or literal cases. For enum
payloads, provide exactly one `_` or immutable identifier for each declared
payload and inspect bound values separately. Unsupported patterns fail before
lowering and never produce an executable.

**Right:**

```vow
fn f(n: i64) -> i64 {
    if n == 0 {
        5
    } else {
        9
    }
}
```

`let` only binds identifier, wildcard, and tuple patterns; all other pattern
kinds (literals, enum variants, struct patterns, or-patterns) are refutable
and rejected outright, since `let` requires the pattern to always match:

**Wrong:**

```vow
fn f(o: Option<i64>) -> i64 {
    let Option::Some(x) = o;
    x
}
```

**Output:** `let bindings only support identifier, wildcard, and tuple patterns`

**Fix:** `match` the value and handle every variant.

**Right:**

```vow
fn f(o: Option<i64>) -> i64 {
    match o {
        Option::Some(x) => x,
        Option::None => 0,
    }
}
```

A tuple `let` pattern additionally requires the initializer to be a tuple
literal of exactly the same arity, at every nesting level — tuple *values*
have no runtime representation, so `let (a, b) = ...` is a compile-time
desugaring into `let a = ...; let b = ...;`, never a projection out of a
materialized tuple:

**Wrong:**

```vow
fn pair() -> (i64, i64) { (1, 2) }

fn f() -> i64 {
    let (a, b) = pair();
    a + b
}
```

**Output:** `tuple destructuring requires a tuple literal initializer with matching arity`

**Fix:** Rewrite the pattern as `let (a, b) = (expr1, expr2);` with one
initializer element per pattern slot, or bind a single identifier and access
elements once tuples are supported as first-class values (tracked separately;
not yet implemented). A tuple `let` pattern also cannot bind a linear-typed
element — bind the whole owner to a single identifier instead.

**Right:**

```vow
fn f() -> i64 {
    let (a, b) = (1, 2);
    a + b
}
```

### ImmutableAssignment

**Phase:** Type Checker
**Meaning:** A binding not declared `mut` was reassigned. Bindings are immutable
by default; `mut` is required only for whole-binding reassignment `x = e`. Field
writes (`s.f = e`) and index writes (`v[i] = e`) are allowed through any binding.

**Wrong:**

```vow
fn f() -> i64 {
    let x: i64 = 1;
    x = 2;
    x
}
```

**Fix:** Declare the binding `mut`: `let mut x: i64 = 1;`.

**Right:**

```vow
fn f() -> i64 {
    let mut x: i64 = 1;
    x = 2;
    x
}
```

### UnusedMut

**Phase:** Type Checker
**Meaning:** A `let mut` binding is never reassigned, so the `mut` is dead. Only
whole-binding reassignment counts as a use of `mut` — a binding mutated solely
via `s.f = e`, `v[i] = e`, or a method call does not need `mut`.

**Wrong:**

```vow
fn f() -> i64 {
    let mut x: i64 = 1;
    x
}
```

**Fix:** Remove `mut`: `let x: i64 = 1;`.

**Right:**

```vow
fn f() -> i64 {
    let x: i64 = 1;
    x
}
```

### UnknownMethod

**Phase:** Type Checker
**Meaning:** A method call uses a name that does not exist on the receiver type.

**Wrong:**

```vow
fn f() -> () {
    let v: Vec<i64> = Vec::new();
    v.psh(42);
}
```

**Output:** ``unknown method `psh` on type `Vec` ``

**Fix:** Check the method name for typos. Use `--help` to see available methods for each type. `Vec` has no `get` method: read an element with `v[i]` (see [Indexing](grammar.md#indexing)), and call `HashMap::get` or `BTreeMap::get` for maps. A `Vec` receiver calling `get` carries the structured hint ``use `v[i]` to read an element; `Vec` has no `get` `` in the diagnostic's `hints` array, identically in both compilers. Any other unknown method on a `String`, `Vec`, `HashMap`, `BTreeMap`, `Option` or `Result` receiver carries ``did you mean `push`?`` for the nearest builtin method within an edit distance of 3 (the earliest in the method list wins a tie; names over 128 bytes get none), and otherwise `available methods: len, push, pop, clear, truncate`, again identically in both compilers.

**Right:**

```vow
fn f() -> () {
    let v: Vec<i64> = Vec::new();
    v.push(42);
}
```
