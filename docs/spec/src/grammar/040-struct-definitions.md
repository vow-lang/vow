
### If / Else

```vow
if x > 0 {
    x
} else {
    0 - x
}
```

`if`/`else` is an expression — both branches must have the same type. The condition must have type `bool` (or `never`). There is no `else if` keyword; nest `if` inside `else`:

```vow
if x < lo {
    lo
} else {
    if x > hi {
        hi
    } else {
        x
    }
}
```

### While Loop

```vow
while i > 0 {
    i = i - 1;
}
```

### While Loop with Invariant

```vow
let mut i: u64 = 0u64;
while i < n vow {
    invariant: i <= n,
    invariant: v.len() == i
} {
    v.push(i);
    i = i + 1;
}
```

State the bound the loop actually maintains. `invariant: i >= 0` looks like a
lower bound but is always true once `i` is unsigned, and the type checker
rejects it as `TautologicalComparison`. `.len()` is `u64`, so a length clause
compares directly against a `u64` counter; a counter of any other type needs
an explicit `as` cast.

### Descending Loops

A descending index loop guards on `> 0` and decrements as the first statement
of the body:

```vow
let n: u64 = v.len();
let mut i: u64 = n;
let mut acc: u64 = 0u64;
while i > 0 vow { invariant: i <= n } {
    i = i - 1;
    acc = acc + v[i];
}
```

`i` starts at `n`, not at `n - 1`, and the decrement brings it into range
before the first use. The guard is therefore also the bounds check: every
`v[i]` in the body runs with `i < n`.

One constraint on the shape above is easy to miss: the body must stay pure.
A `print_*` call inside a contracted function gives that function an effect,
and the verifier model is restricted to pure functions, so the contract is
reported as `VerificationSkipped` rather than checked.

This idiom does not by itself make a length-bounded loop provable. A loop
without an `invariant` that runs longer than the verifier's unwind bound
(ESBMC's `--max-k-step`, or the native backend's internal bound of 64) returns
`unknown` unless the native backend closes it by induction (`docs/spec/cli.md` →
"Native backend", "Loops"), for this form and the signed `while i >= 0` form
alike. What the idiom buys is a
*writable* invariant rather than a stronger proof: `i <= n` is expressible on
an unsigned index, whereas the signed form's companion clause `i >= -1` is not
— a negative literal does not fit `u64` at all.

#### Never `while i >= 0`

```vow
let mut i: u64 = v.len() - 1;   // wraps on an empty collection
while i >= 0 { ... }            // never exits; rejected by the checker
```

Both lines are broken independently. `i >= 0` is universally true on an
unsigned type, so the loop has no exit; the type checker rejects the
comparison outright as `TautologicalComparison`, in a loop condition just as
in a contract clause. The initializer is the half no diagnostic catches: on an
empty collection `v.len() - 1` wraps to `18446744073709551615` and the
first index read runs far out of bounds.

#### `-` inside the guard, `-!` outside it

Plain wrapping `-` is the correct decrement in the canonical form. The guard
`i > 0` has already discharged the underflow obligation, so no check is
needed, and plain `-` is exactly what the verifier's bit-vector encoding models — a
*wrong* guard therefore stays observable through `invariant: i <= n`. Reserve
`-!` for decrements whose non-negativity the guard does not establish.

Note one asymmetry while reading that advice. `-!` traps at runtime under
`--mode debug`, reporting `{"error":"ArithmeticOverflow"}`, and the verifier
models that abort: an aborting execution never returns, so it cannot witness a
violated `ensures`, and a reachable abort adds an `ArithOverflowReachable`
warning instead of failing the build. `n -! 1` and `n - 1` are therefore
different obligations. Do not reach for `-!` expecting it to turn an underflow
into a failed proof.

#### Why there is no reverse-range syntax

Vow has no numeric range form, and adding one is rejected rather than pending:

- `loop` is not an escape hatch. `break value` is restricted to `loop`, and
  an unbounded `loop` is verifiable only through its `invariant` and `break`
  structure (the native backend models `loop`; ESBMC does not).
- A reverse for-each would cover very little. Most reverse scans need the
  *index*, not the element — they read a parallel array at `i`, or write a
  second collection at the same `i`.
- A range desugar would have to thread through index-type decisions that are
  hardcoded in both lowerers, where a miss is silent rather than loud: a
  signed comparison applied to unsigned operands raises no type error.
- A genuine range type would add a new type-system axis, which the language
  design rule rejects outright.

### For-Each Loop

```vow
for x in vec {
    print_i64(x);
}
```

Iterates over each element of a `Vec<T>`. The loop variable `x` is bound to each element in turn. Desugars to a `while` loop with index arithmetic — zero verification overhead.

### For-Each Loop with Invariant

```vow
for x in vec vow {
    invariant: total >= 0
} {
    total = total + x;
}
```

### Loop (Infinite)

`loop` creates an infinite loop. The expression returns the type of the `break` value:

```vow
let idx: i64 = loop {
    if data[i] == target {
        break i as i64;
    }
    i = i + 1;
    if i >= n { break -1; }
};
```

Under the ESBMC backend an unbounded `loop` cannot be verified; the native backend models `loop` with `break` and `continue`. Use `while` with invariants for loops that verify under both.

### Break

`break` exits the innermost loop. Inside `loop`, `break value` sets the loop's result:

```vow
break;           // exit while or loop (loop returns Unit)
break value;     // exit loop with a value (only inside loop, not while)
```

### Continue

`continue` skips the remaining statements in the current loop iteration and jumps back to the loop header:

```vow
continue;        // skip to next iteration of while, loop, or for
```

Inside `while` and `loop`, `continue` emits back-edge values for any mutated variables. Inside `for`, it also advances the loop index.

### Return

```vow
return;
return value;
```

## Struct Definitions

```vow
struct Point {
    x: i64,
    y: i64,
}
```

### Linear Structs

```vow
linear struct FileHandle {
    fd: i64,
}
```

Linear struct values carry a linear obligation. The obligation must either be consumed before the value's owning region closes or transferred to the caller by returning the value.

Owned enum wrappers inherit that obligation transitively. A user enum,
`Option<T>`, or `Result<T, E>` is linear when one of its owned payload paths is
linear; matching such a value consumes the wrapper exactly once and transfers
the obligation to the selected bound payload. A reference type (`&T`) is never a
linear owner. Collection types do not acquire linear ownership from
their element type, and they cannot hold linear values: a `Vec<T>` element, a
`HashMap<K, V>` value, or a `BTreeMap<K, V>` value that is or transitively
contains a linear owner (a `linear struct`, or an `Option`, `Result`, or user
enum wrapping one) is rejected where the collection type is written. The
containers copy and shift entries bitwise, so storing a linear value would
duplicate its obligation or let it escape the checker. `Vec` and `HashMap` use
`UnsupportedFeature`; `BTreeMap` uses `BTreeMapValueMustBeNonLinear`. A nested
collection (`Vec<Vec<Token>>`) is reported once, at the innermost collection that
holds the linear value, and a type alias is reported once, at its definition. A
`Vec` element test is about ownership: `Vec<&Token>` borrows and is accepted,
while a tuple that holds a linear owner is rejected.
A linear value that is no longer needed is discharged with the intrinsic
`drop(value)` (see [Linear Intrinsics](#linear-intrinsics)). Passing it to a
function that consumes it, returning it, or matching it are the other ways to
satisfy the obligation; there is no implicit end-of-scope discharge, so a live
obligation at scope exit is `RegionLinear`.
Assigning a new value to a `mut` linear local whose previous value was already
consumed starts a fresh obligation that must itself be consumed; overwriting a
value that is still live discards it and is reported as `RegionLinear`.
An unbound `_` match catchall cannot discard a still-reachable linear payload:
every variant that owns a linear payload must first have an explicit arm that
binds and consumes or transfers that payload.

Struct fields cannot own linear values, even when the containing struct is
`linear`, because field access does not provide move-out semantics. Allowing an
owned field would let repeated reads transfer the same obligation more than
once. Reference-typed (`&T`) and collection fields do not become linear owners
under this rule.

### Struct Literals

Struct literal names must be PascalCase:

```vow
let p: Point = Point { x: 1, y: 2 };
```

A struct literal must initialise every declared field. An omitted field is a
`TypeMismatch` (``struct `Point` literal is missing field `y` ``, one per missing
field in declaration order, at the literal's span), never zero-initialised.

An identifier that does not start with an upper-case letter is never a struct
literal: in `while c { }` and `if c { }` the `{` opens the body.

In the head of `if`, `while`, `for … in` and `match`, a `Name {` or
`Path::Variant {` is never a struct literal or brace-form enum constructor; the
`{` always opens the body. This is what lets `if DEBUG { }` and
`if c == Color::Red { }` parse. To use a struct literal there, parenthesise it
or bind it to a `let` first:

```vow
if (Point { x: 1, y: 2 }).x == 1 { }
```

Delimiters re-enable literals inside a head: `(…)`, call and method-call
arguments, `[…]`, blocks, `match` arms and `vow { … }` blocks. The canonical
printer parenthesises a head that would otherwise expose a struct literal, so
`parse → print → parse` stays idempotent.

### Field Access

```vow
p.x
```

### Field Assignment

```vow
p.x = 10;
```

### Passing Semantics

Structs are heap-allocated. A struct value is a pointer to a heap region, so passing a struct to a function passes the pointer — the function operates on the same heap data, not a copy. Field assignments inside the called function are visible to the caller:

```vow
fn shift_right(p: Point, dx: i64) {
    p.x = p.x + dx;
}

fn main() -> i32 [io] {
    let p: Point = Point { x: 0, y: 0 };
    shift_right(p, 5);
    print_i64(p.x);  // 5 — mutation visible to caller
    0
}
```

This enables in-place mutation patterns (e.g., make/unmake in search trees) without cloning. The same aliasing semantics apply when structs are stored in containers — see [Indexing](#indexing). To avoid aliasing, construct a fresh struct literal with the desired field values.

**Note:** For linear owner types (a `linear struct` or an owned enum wrapper that
contains one), passing the value to a function consumes it; the caller cannot
access it afterward. Returning a linear value transfers the obligation to the
caller, so this is the normal way to hand an updated linear value back out of a
function.

## Enum Definitions

```vow
enum Shape {
    Circle(i64),
    Rect(i64, i64),
    Empty,
}
```

Variant kinds: unit (`Empty`), tuple (`Circle(i64)`), struct (`Named { x: i64 }`).

### Enum Construction

```vow
let s: Shape = Shape::Circle(5);
let none: Option<i64> = Option::None;
let some: Option<i64> = Option::Some(42);
```
