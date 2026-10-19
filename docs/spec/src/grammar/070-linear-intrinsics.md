
#### Linear Intrinsics

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `drop`           | `fn(value: L) -> ()` for a linear owner `L` | `[]`       |

`drop` is a compiler intrinsic, not a user-defined generic. `L` must be a linear owner: a `linear struct`, or an owned enum wrapper (`Option`, `Result`, or a user enum) that contains one. Any other argument type, or an argument count other than one, is a `TypeMismatch`. `drop` consumes the value exactly once (a second use is `LinearTypeViolation`) and discharges its obligation. It has no runtime effect beyond that: it runs no destructor, frees nothing, and lowers to no instruction other than the consume marker the type and region passes already track. It is verifier-neutral: the consume marker is a no-op in the C model, so a function that drops a linear value is verified exactly as if the call were absent. A user-defined function named `drop` takes precedence over the intrinsic.

`String::from_raw_parts_copy(ptr: i64, len: u64)` copies `len` bytes from a raw C pointer into a fresh `String`. `Vec::from_raw_parts_copy(ptr: i64, len: u64)` copies `len` one-slot scalars into a fresh `Vec<T>` (`T` is not `i128`/`u128`). The pointer is `i64` and the length is `u64`, so a signed length must be converted explicitly (`n as u64`); the code generator converts pointer and length values to the platform pointer-sized ABI type at the FFI boundary. Both helpers have a `FreshInCaller` return summary.

For pointer-containing C payloads, a wrapper must be written per type: call the extern, recursively copy every Vow-owned heap subobject into the target region, free every C-owned pointer according to the extern's ownership contract, then return the Vow-placed value. A bytewise copy of a pointer-containing payload is unsound because it preserves stale pointers into C-owned storage.

#### Print / IO

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `print_str`      | `fn(s: String) -> ()`                      | `[io]`     |
| `print_i64`      | `fn(v: i64) -> ()`                         | `[io]`     |
| `print_u64`      | `fn(v: u64) -> ()`                         | `[io]`     |
| `eprintln_str`   | `fn(s: String) -> ()`                      | `[io]`     |
| `try_eprintln_str` | `fn(s: String) -> i64`                   | `[io]`     |

`try_eprintln_str` writes `s` and a newline to stderr and reports the outcome instead of ignoring it: `0` when written, `1` when stderr is a closed pipe (`BrokenPipe`), `2` for any other write failure. A closed stderr descriptor (`EBADF`) counts as `0`, matching Rust's standard library. SIGPIPE is blocked for the duration of the call, so a closed pipe returns `1` instead of terminating the process. `eprintln_str` ignores write failures entirely.

#### Debug

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `debug_str`      | `fn(s: String) -> ()`                      | `[]`       |
| `debug_i64`      | `fn(v: i64) -> ()`                         | `[]`       |
| `debug_u64`      | `fn(v: u64) -> ()`                         | `[]`       |

**Debug print semantics:** Debug prints are effect-free and callable from pure functions. In debug and sanitize modes (`--mode debug`, `--mode sanitize`), they write to stderr. In release and profile modes, the debug call itself is not emitted — no function call occurs. However, argument expressions are still evaluated (a direct literal such as `"label"` is static, while `String::from("label")` still allocates a mutable copy). They are also no-ops during verification. Use them to trace values inside pure kernel code without restructuring the effect hierarchy.

#### Filesystem

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `fs_read`        | `fn(path: String) -> String`               | `[read]`   |
| `fs_open`        | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_read_line`   | `fn(handle: i64) -> String`                | `[read]`   |
| `fs_status`      | `fn(handle: i64) -> i64`                   | `[read]`   |
| `fs_read_status` | `fn() -> i64`                              | `[read]`   |
| `getenv`         | `fn(name: String) -> String`               | `[read]`   |
| `path_lookup`    | `fn(name: String) -> String`               | `[read]`   |
| `mktemp_dir`     | `fn(prefix: String) -> String`             | `[io]`     |
| `fs_close`       | `fn(handle: i64) -> i64`                   | `[read]`   |
| `fs_write`       | `fn(path: String, data: String) -> i64`    | `[write]`  |
| `fs_exists`      | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_mkdir`       | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_listdir`     | `fn(path: String) -> Vec<String>`          | `[read]`   |
| `fs_remove`      | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_remove_dir`  | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_remove_dir_all` | `fn(path: String) -> i64`               | `[io]`     |
| `fs_is_dir`      | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_is_symlink`  | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_rename`      | `fn(old: String, new: String) -> i64`      | `[io]`     |

#### String Operations

| Function              | Signature                                        | Effects |
|-----------------------|--------------------------------------------------|---------|
| `string_substr`       | `fn(s: String, start: u64, len: u64) -> String`  | `[]`    |
| `string_split`        | `fn(s: String, delim: String) -> Vec<String>`    | `[]`    |
| `string_starts_with`  | `fn(s: String, prefix: String) -> i64`           | `[]`    |
| `string_ends_with`    | `fn(s: String, suffix: String) -> i64`           | `[]`    |
| `string_matches_literal_at` | `fn(s: String, pos: u64, literal: String literal) -> i64` | `[]` |
| `string_trim`         | `fn(s: String) -> String`                        | `[]`    |
| `string_to_upper`     | `fn(s: String) -> String`                        | `[]`    |
| `string_to_lower`     | `fn(s: String) -> String`                        | `[]`    |
| `string_replace`      | `fn(s: String, from: String, to: String) -> String` | `[]` |
| `string_join`         | `fn(parts: Vec<String>, sep: String) -> String`  | `[]`    |

#### Conversion

**Formatting** uses four baselines; widen via `as` for narrower types. The
128-bit formatters exist because most 128-bit values do not fit `i64`/`u64`, so
they cannot be formatted by narrowing first:

| Function            | Signature                                  | Effects    |
|---------------------|--------------------------------------------|------------|
| `int_to_string`     | `fn(v: i64) -> String`                     | `[]`       |
| `uint_to_string`    | `fn(v: u64) -> String`                     | `[]`       |
| `i64_to_string`     | `fn(v: i64) -> String` (alias of `int_to_string`) | `[]` |
| `int128_to_string`  | `fn(v: i128) -> String`                    | `[]`       |
| `uint128_to_string` | `fn(v: u128) -> String`                    | `[]`       |

The 128-bit formatters render the full decimal value (`i128::MIN` is 40
characters, `u128::MAX` is 39). The verifier has no model of their result, so a
function that calls one is reported `Skipped` rather than proven.

```vow
let small: u8 = 42;
print_str(uint_to_string(small as u64));  // widen then format
```

**Parsing** exposes a try-form for every integer width but `u64`:

| Function       | Signature                                | Effects |
|----------------|------------------------------------------|---------|
| `parse_i8`     | `fn(s: String) -> Option<i8>`            | `[]`    |
| `parse_i16`    | `fn(s: String) -> Option<i16>`           | `[]`    |
| `parse_i32`    | `fn(s: String) -> Option<i32>`           | `[]`    |
| `parse_i64`    | `fn(s: String) -> Option<i64>`           | `[]`    |
| `parse_i128`   | `fn(s: String) -> Option<i128>`          | `[]`    |
| `parse_u8`     | `fn(s: String) -> Option<u8>`            | `[]`    |
| `parse_u16`    | `fn(s: String) -> Option<u16>`           | `[]`    |
| `parse_u32`    | `fn(s: String) -> Option<u32>`           | `[]`    |
| `parse_u128`   | `fn(s: String) -> Option<u128>`          | `[]`    |

`parse_i64` is also available as the `String` method `.parse_i64()`. There is
no free `parse_u64`; parse a `u64` with the method `.parse_u64()`.

Each `parse_X` returns `Option::None` for malformed input, empty strings, or
values outside the target type's range. Parsing never substitutes a numeric
sentinel for failure; callers that need a fallback must choose it explicitly
when handling `Option::None`.

In particular, `parse_i8`, `parse_i16`, `parse_u8`, `parse_u16`, `parse_i32`,
`parse_u32`, `parse_i128`, and `parse_u128` enforce their exact signed or
unsigned fixed-width ranges.

Every `parse_X` shares one set of lexical rules, identical at every width:
surrounding Unicode whitespace is trimmed first; a leading `+` and leading zeros
are accepted (`"+7"` and `"0007"` parse as `7`); `"-0"` is `Some(0)` for a signed
type and `None` for an unsigned one; and `"+"`, `"-"`, the empty string, and any
interior non-digit (`"1 2"`) are `None`.

**Float bit patterns.** The compiler's own float literals go through two
helpers that carry an `f64` as its IEEE-754 bit pattern in a `u64`:

| Function          | Signature                          | Effects |
|-------------------|------------------------------------|---------|
| `parse_f64_bits`  | `fn(s: String) -> u64`             | `[]`    |
| `format_f64_bits` | `fn(bits: u64) -> String`          | `[]`    |

`parse_f64_bits` trims surrounding whitespace and returns the bits of the
parsed `f64`, or the bits of `0.0` (`0`) for text that is not a float; it never
fails. `format_f64_bits` renders the `f64` with those bits in the shortest
decimal form that reads back as the same value. The verifier has no model of
either (their other side is a `String`): a function that calls one is reported
`Skipped`.

**Narrowing intrinsics** (per [Type Cast](#type-cast)): for every narrowing
pair the compiler emits `<src>_to_<tgt>_try`, `<src>_to_<tgt>_wrap`, and
`<src>_to_<tgt>_sat` free functions with the semantics described in that
section.

#### Collections

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `vec_sort`       | `fn(v: Vec<i64>) -> Vec<i64>`              | `[]`       |

#### Time

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `time_unix`      | `fn() -> i64`                              | `[io]`     |
| `time_unix_ms`   | `fn() -> i64`                              | `[io]`     |
| `time_micros`    | `fn() -> i64`                              | `[io]`     |

`time_unix()` and `time_unix_ms()` return wall-clock seconds and milliseconds since the Unix epoch (`0` if the clock is before it). `time_micros()` is a monotonic clock in microseconds counted from its first call in the process (the first call returns about `0`); use it to measure elapsed time, never as a timestamp.
