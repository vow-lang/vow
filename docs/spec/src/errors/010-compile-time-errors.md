# Vow Error Catalog

Every Vow error has a machine-readable `error_code` in the JSON output. This document lists all error codes, their phase, meaning, an example trigger, and how to fix them.

## Compile-Time Errors

These appear in the `diagnostics` array of the build output JSON.

### UnterminatedString

**Phase:** Lexer
**Meaning:** A string literal was opened with `"` but never closed before the
end of the file, or the file ends with a `\` escape that has nothing to escape
(`unterminated string escape`). An escaped quote (`"abc\"`) does not close the
literal.

**Wrong:**

```vow
fn f() -> () [io] {
    print_str("hello);
}
```

**Fix:** Close the string with a matching `"`.

**Right:**

```vow
fn f() -> () [io] {
    print_str("hello");
}
```

### InvalidCharacter

**Phase:** Lexer
**Meaning:** The source contains a character the lexer does not recognize. The
message is `unexpected character '<c>'` and the span covers that one byte.
Lexing stops at the first lexical error.

**Wrong:**

```vow
fn f() -> i64 {
    x @ y
}
```

**Fix:** Remove the invalid character. Vow has no `@` operator.

**Right:**

```vow
fn f(x: i64, y: i64) -> i64 {
    x + y
}
```

A float literal whose decimal magnitude overflows `f64` (parses to
infinity) is also an `InvalidCharacter` error, raised at lex time rather
than allowed through to produce an uncompilable C `inf` token later in
the pipeline:

**Wrong:**

```vow
fn f() -> f64 {
    999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999.0
}
```

**Fix:** Use a literal whose magnitude is within `f64::MAX` (~1.7976931348623157e308).

**Right:**

```vow
fn f() -> f64 {
    1000000000000000000000.0
}
```

### InvalidIntSuffix

**Phase:** Lexer
**Meaning:** An integer literal carries a type suffix that names a type Vow does not have.

**Wrong:**

```vow
fn f() -> u64 {
    5usize
}
```

**Output:** ``unknown integer suffix `usize`: Vow has no `usize`/`isize` type (fixed-width only)``

**Fix:** Use a fixed-width suffix. Vow's integer widths are pointer-independent by
design (see `docs/adr/0001-numeric-tower-narrow-ints.md`), so there is no `usize`
or `isize`; write `5u64` for a machine-word-sized unsigned value and `5i64` for a
signed one.

**Right:**

```vow
fn f() -> u64 {
    5u64
}
```

### UnexpectedToken

**Phase:** Parser
**Meaning:** The parser encountered a token it did not expect at that position.

**Wrong:**

```vow
module M 123
```

**Fix:** Check the syntax around the reported span. Common causes: missing `{`, `}`, `(`, `)`, or a keyword in the wrong position.

**Right:**

```vow
module M

fn f() -> i64 {
    123
}
```

### MissingDelimiter

**Phase:** Parser
**Meaning:** A matching delimiter (`}`, `)`, `]`) is missing.

**Wrong:**

```vow
fn f() -> i64 {
    42
```

**Fix:** Add the missing closing delimiter. No compiler emits `MissingDelimiter` today: an
unclosed block at end of file is reported as `UnexpectedToken`.

**Right:**

```vow
fn f() -> i64 {
    42
}
```

### TypeMismatch

**Phase:** Type Checker
**Meaning:** An expression has a different type than expected, or a qualified
enum pattern names an enum other than the scrutinee's enum.

**Wrong:**

```vow
fn f() -> i32 {
    true
}
```

**Output:** `function body has type 'bool' but declared return type is 'i32'`

**Fix:** Change the expression or declared type to match. For an enum pattern,
qualify the variant with the scrutinee's enum name.

**Right:**

```vow
fn f() -> bool {
    true
}
```

A struct literal must name every declared field. Each omitted field is a
`TypeMismatch` in both compilers, reported once per field in declaration order
at the literal's span; a field is never zero-initialised:

**Wrong:**

```vow
struct Pair { a: i64, b: i64 }

fn f() -> Pair {
    Pair { a: 1 }
}
```

**Output:** ``struct `Pair` literal is missing field `b` ``

**Fix:** Initialise the field explicitly (`Pair { a: 1, b: 0 }`).

The `drop` intrinsic also reports `TypeMismatch` when it is called with an argument
count other than one or with a value that is not a linear owner
(``drop requires a linear value, found `i64` ``); see
[Linear Intrinsics](grammar.md#linear-intrinsics).

A `Vec` index (`v[i]`, `v[i] = val`) and the `Vec::truncate`
argument must have exactly the type `u64`. An index of any other integer type
(for example an `i64` counter) or of a non-integer type is a `TypeMismatch`
in both compilers:

**Wrong:**

```vow
fn f(v: Vec<i64>, i: i64) -> i64 {
    v[i]
}
```

**Output:** ``index has type `i64` but must be `u64` ``

**Fix:** Keep index counters `u64` (`.len()` is already `u64`), or convert at
the index site with `as u64` (`v[i as u64]`). Unsuffixed integer literals
coerce to `u64` without a cast. `String` offsets (`byte_at`, `substring`,
`string_substr`, `string_matches_literal_at`) follow the same exact-`u64` rule,
and `push_byte` takes exactly `u8`; see
[String offsets](grammar.md#string-offsets).

**Right:**

```vow
fn f(v: Vec<i64>, i: u64) -> i64 {
    v[i]
}
```

Only `Vec<T>` has an index operator. Indexing any other type (`HashMap`,
`BTreeMap`, `String`, `Option`, ...) is a `TypeMismatch` whose message begins
`index operation on non-indexable type` and prints the full receiver type (for example `HashMap<i64, i64>` or `String`, identically in both compilers); read a map entry
with `m.get(k)` and a string byte with `s.byte_at(i)`.

`Vec::new()`, `HashMap::new()`, and `BTreeMap::new()` take their element, key,
and value types from the binding's annotation. A method call on a collection
whose type was never annotated is a `TypeMismatch` in both compilers, one per
call:

**Wrong:**

```vow
fn f() -> () {
    let m = HashMap::new();
    m.insert(1, 2);
}
```

**Output:** `cannot infer the collection type of the receiver of `insert`: annotate its binding with a full type`

**Fix:** Annotate the binding: `let m: HashMap<i64, i64> = HashMap::new();`.

**Right:**

```vow
fn f() -> () {
    let m: HashMap<i64, i64> = HashMap::new();
    m.insert(1, 2);
}
```

An undefined name is a `TypeMismatch` whose message begins `undefined variable`, in both compilers, and the expression then has an error type so it does not cascade into further type errors. A parameter `where` clause is name-resolved in a scope holding only its own parameter, so a sibling parameter or `result` is an undefined name there; the hint says to move such a condition to `requires`/`ensures`.

**Wrong:**

```vow
fn ordered(a: i64, b: i64 where b > a) -> i64 {
    b
}
```

**Output:** ``undefined variable `a` ``, hint ``a `where` clause can only reference its own parameter `b`; put a condition on several parameters in `requires` ``

**Fix:** State the condition in `requires: b > a`.

**Right:**

```vow
fn ordered(a: i64, b: i64) -> i64 vow {
    requires: b > a
} {
    b
}
```

### LiteralOutOfRange

**Phase:** Type Checker
**Meaning:** An integer literal appears in a typed context (annotated `let`, function argument, struct field, or const declaration) whose target type cannot hold the literal's value. The check runs after context coercion, so the offending literal is the one written in the source, not a widened intermediate.

**Wrong:**

```vow
let x: u8 = 300;
const NEG: u16 = -1;
let y = 256u8;   // a suffixed literal is checked against its suffix type
```

**Output:** `literal 300 does not fit in u8 (range 0..=255)`

**Fix:** Use a value within the target type's range, change the target type, or write an explicit narrowing intrinsic (`i64_to_u8_try`, `i64_to_u8_wrap`, `i64_to_u8_sat`) if you intend to convert a wider value at runtime.

**Right:**

```vow
let x: u8 = 255;
const NEG: i16 = -1;
let y = 255u8;
```

### NarrowingCastNotAllowed

**Phase:** Type Checker
**Meaning:** The `as` operator was used to convert a wider integer type to a narrower one. `as` is widening-only; narrowing must use a named intrinsic so the agent chooses an explicit semantics (range-checked vs. truncating vs. saturating). See `grammar.md` §Type Cast.

**Wrong:**

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

**Right:**

```vow
fn f(big: i64) -> u8 {
    i64_to_u8_wrap(big)
}
```

`i128`/`u128` to `i64`/`u64` has no narrowing intrinsic family; the diagnostic's hint says so instead of naming one. Narrow through `i32` or smaller with `i128_to_i32_try`-style intrinsics, or keep the value as `i128`/`u128`. See `grammar.md` §Type Cast.

### ShiftCountOutOfRange

**Phase:** Type Checker
**Meaning:** A constant-expression shift count is negative or greater than or equal to the bit-width of the left operand. Shifting an `N`-bit value by `>= N` bits is undefined in the underlying C model and is rejected at compile time when the count is statically known. Dynamic shift counts (non-const expressions) are checked by the verifier, which requires `0 <= count < width` at the shift (a negative signed count is a violation). At runtime only 8-bit shifts trap: an `i8`/`u8` shift whose count is `>= 8` aborts with `ArithmeticOverflow` in every build mode. Wider shifts are not trapped; the hardware shift masks the count to the operand width (`(x: i64) << 65` is `x << 1`), so only verification excludes them.

**Wrong:**

```vow
fn f(x: u8) -> u8 {
    x << 8
}
```

**Output:** `shift count 8 is out of range for u8 (max 7)`

**Fix:** Use a count less than the LHS bit-width. To shift a narrow value by a larger amount, widen first: `(x as u32) << 8` is legal (it shifts the widened `u32` value by 8), but the result is `u32`; to get back to `u8`, use a narrowing intrinsic such as `u32_to_u8_wrap`.

**Right:**

```vow
fn f(x: u8) -> u32 {
    (x as u32) << 8
}
```
