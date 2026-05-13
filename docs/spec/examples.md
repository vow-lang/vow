# Worked Examples

Verification workflow examples. The first three demonstrate Counterexample-Guided Inductive Synthesis (CEGIS) cycles: write spec, build, read JSON, diagnose, fix, verify. The fourth shows break-with-value in loop expressions. The fifth shows an EOF-safe interactive command loop using `stdin_read_line()`. The sixth shows bounded-memory streaming file input.

## 1. Safe Division — Requires Pattern

### Goal

Write a division function that is safe (cannot divide by zero).

### Step 1: Write the spec

```vow
module Divide

fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
} {
    x / y
}

fn main() -> i32 [io] {
    divide(10, 0);
    0
}
```

### Step 2: Build and verify

```
$ vow build examples/divide.vow
```

```json
{"status":"Verified","executable":"examples/divide","diagnostics":[],"counterexamples":[]}
```

ESBMC proves the contract: whenever `y != 0` holds, the division is safe.

### Step 3: Runtime behavior (debug mode)

The `main()` calls `divide(10, 0)` which violates `requires: y != 0`. In debug mode:

```
$ vow build --mode debug --no-verify examples/divide.vow -o /tmp/divide_debug
$ /tmp/divide_debug
```

Stderr:
```json
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"examples/divide.vow","offset":56,"values":{"y":0}}
```

The `blame: "Caller"` tells you: `main()` passed `y=0`, which violates the precondition.

---

## 2. CEGIS Broken → Fixed — The Core Workflow

### Goal

Write `safe_sub(a, b)` that always returns a non-negative result.

### Step 1: Initial attempt (broken)

```vow
module CegisBroken

fn safe_sub(a: i64, b: i64 where b >= 0) -> i64 vow {
    ensures: result >= 0
} {
    a - b
}

fn main() -> i32 [io] {
    print_i64(safe_sub(10, 3));
    0
}
```

### Step 2: Build

```
$ vow build examples/cegis_broken.vow
```

```json
{
  "status": "VerifyFailed",
  "executable": "examples/cegis_broken",
  "diagnostics": [],
  "function": "safe_sub",
  "counterexample": "[Counterexample]",
  "counterexamples": [
    {
      "function": "safe_sub",
      "inputs": { "a": "-9223372036854775808", "b": "0" },
      "violation": "ensures result >= 0",
      "vow_id": 1,
      "source": { "file": "examples/cegis_broken.vow", "offset": 76, "length": 20 }
    }
  ]
}
```

### Step 3: Diagnose

The counterexample shows `a = -9223372036854775808` (i64 min), `b = 0`. Then `a - b = a`, which is negative. The `ensures: result >= 0` is violated.

**Root cause:** We need `a >= b` to guarantee a non-negative result, and `a >= 0` to prevent negative inputs.

### Step 4: Fix

```vow
module CegisFixed

fn safe_sub(a: i64 where a >= 0, b: i64 where b >= 0) -> i64 vow {
    requires: a >= b,
    ensures: result >= 0
} {
    a - b
}

fn main() -> i32 [io] {
    print_i64(safe_sub(10, 3));
    0
}
```

### Step 5: Verify

```
$ vow build examples/cegis_fixed.vow
```

```json
{"status":"Verified","executable":"examples/cegis_fixed","diagnostics":[],"counterexamples":[]}
```

Verified. With `a >= 0`, `b >= 0`, and `a >= b`, ESBMC proves `result >= 0`.

---

## 3. Vec Fill — Loop Invariant

### Goal

Fill a vector with `n` elements and prove its length equals `n`.

### Step 1: Write the spec

```vow
module VecFill

fn fill_vec(n: i64) -> Vec<i64> vow {
    requires: n >= 0,
    requires: n <= 8,
    ensures: result.len() == n
} {
    let v: Vec<i64> = Vec::new();
    let mut i: i64 = 0;
    while i < n vow {
        invariant: i >= 0,
        invariant: i <= n
    } {
        v.push(i);
        i = i + 1;
    }
    v
}

fn main() -> i32 [io] {
    let v: Vec<i64> = fill_vec(5);
    print_i64(v.len());
    0
}
```

### Step 2: Build and verify

```
$ vow build examples/vec_fill.vow
```

```json
{"status":"Verified","executable":"examples/vec_fill","diagnostics":[],"counterexamples":[]}
```

**Key points:**
- `requires: n <= 8` keeps iterations tractable for verification
- `invariant: i >= 0, invariant: i <= n` is inductive: true on entry, preserved by the loop body
- The Vec model tracks `len`, so ESBMC can reason about `result.len() == n`

---

## 4. Linear Search — Break-with-Value

### Goal

Search a vector for a target value and return its index, or `-1` if not found. Uses `loop` with `break <value>` to produce a result directly from the loop expression.

### Step 1: Write the spec

```vow
module Search

fn linear_search(data: Vec<i64>, target: i64) -> i64
    vow { requires: data.len() > 0 }
{
    let mut i: i64 = 0;
    let n: i64 = data.len();
    let result: i64 = loop {
        if i >= n {
            break -1;
        }
        if data[i] == target {
            break i;
        }
        i = i + 1;
    };
    result
}

fn main() -> i32 [io] {
    let data: Vec<i64> = Vec::new();
    data.push(10);
    data.push(20);
    data.push(30);
    data.push(40);
    data.push(50);

    let idx: i64 = linear_search(data, 30);
    print_i64(idx);

    let idx2: i64 = linear_search(data, 99);
    print_i64(idx2);
    0
}
```

### Step 2: Build and verify

```
$ vow build examples/search.vow
```

```json
{"status":"Verified","executable":"examples/search","diagnostics":[],"counterexamples":[]}
```

**Key points:**
- `loop { ... break <value>; ... }` is an expression that evaluates to the break value
- All `break` expressions in a `loop` must produce the same type (`i64` here)
- `break <value>` is only allowed in `loop`, not in `while` (which always evaluates to `()`)
- The result is bound with `let result: i64 = loop { ... };`

---

## 5. Command Loop — EOF-Safe `stdin_read_line`

### Goal

Write a line-oriented command interpreter that reads from stdin, dispatches commands, skips empty lines, and exits cleanly on EOF. This is the canonical pattern for CI-safe interactive programs.

### Step 1: Write the program

```vow
module CmdLoop

fn trim_newline(s: String) -> String {
    let n: i64 = s.len();
    if n == 0 { return s; }
    let last: i64 = s.byte_at(n - 1);
    if last == 10 {
        if n >= 2 {
            let prev: i64 = s.byte_at(n - 2);
            if prev == 13 {
                return s.substring(0, n - 2);
            }
        }
        return s.substring(0, n - 1);
    }
    s
}

fn skip_spaces(s: String, start: i64) -> i64 {
    let mut i: i64 = start;
    let n: i64 = s.len();
    while i < n {
        if s.byte_at(i) != 32 { return i; }
        i = i + 1;
    }
    i
}

fn main() -> i32 [read, io] {
    let mut line: String = stdin_read_line();
    while line.len() > 0 {
        let cmd: String = trim_newline(line);

        if cmd.len() > 0 {
            if cmd.eq(String::from("quit")) {
                return 0;
            }

            if cmd.eq(String::from("hello")) {
                print_str(String::from("Hello, world!\n"));
            } else {
                if cmd.len() >= 5 {
                    let prefix: String = cmd.substring(0, 5);
                    if prefix.eq(String::from("echo ")) {
                        let start: i64 = skip_spaces(cmd, 5);
                        let text: String = cmd.substring(start, cmd.len());
                        print_str(text);
                        print_str(String::from("\n"));
                    } else {
                        print_str(String::from("unknown: "));
                        print_str(cmd);
                        print_str(String::from("\n"));
                    }
                } else {
                    print_str(String::from("unknown: "));
                    print_str(cmd);
                    print_str(String::from("\n"));
                }
            }
        }

        line = stdin_read_line();
    }
    0
}
```

### Step 2: Build

```
$ vow build --no-verify examples/cmdloop.vow -o /tmp/cmdloop
```

```json
{"status":"Unverified","executable":"/tmp/cmdloop","diagnostics":[],"counterexamples":[]}
```

No contracts here — this example focuses on the I/O pattern, not verification.

### Step 3: Run with piped input

```
$ printf 'hello\necho Vow is great\n\nbogus\nquit\n' | /tmp/cmdloop
Hello, world!
Vow is great
unknown: bogus
```

The `quit` command causes an early `return 0`. Empty lines are silently skipped.

### Step 4: Run with EOF (no quit)

```
$ printf 'hello\necho test\n' | /tmp/cmdloop
Hello, world!
test
```

When stdin is exhausted, `stdin_read_line()` returns `""` (length 0), the `while` condition fails, and the program exits cleanly with code 0.

### Key points

- **EOF detection:** `stdin_read_line()` returns `""` at EOF. Check `.len() > 0` to exit the loop.
- **Newline stripping:** `stdin_read_line()` includes the trailing `\n` (or `\r\n`). Strip it with `byte_at` + `substring` before comparing commands.
- **Empty line handling:** After trimming, `cmd.len() == 0` means the line was blank — skip it.
- **Effects:** `stdin_read_line()` requires `[read]`; `print_str()` requires `[io]`. The `main` function declares both.
- **CI-safe:** No blocking reads, no prompts — the program processes whatever stdin provides and exits at EOF. Safe to run in pipelines and test harnesses.

## 6. Streaming File Input

`fs_read(path)` materializes the entire file as one `String`. Use `fs_open` plus `fs_read_line` for newline-delimited files that may be large.

```vow
module StreamingFile

fn main() -> i32 [read, io] {
    let argv: Vec<String> = args();
    if argv.len() < 2 {
        eprintln_str(String::from("usage: streaming_file <path>"));
        return 1;
    }

    let h: i64 = fs_open(argv[1]);
    if h < 0 {
        eprintln_str(String::from("could not open input"));
        return 1;
    }

    let mut lines: i64 = 0;
    let mut bytes: i64 = 0;
    let mut line: String = fs_read_line(h);
    while line.len() > 0 {
        lines = lines + 1;
        bytes = bytes + line.len();
        line = fs_read_line(h);
    }

    if fs_status(h) != 1 {
        fs_close(h);
        eprintln_str(String::from("read error"));
        return 1;
    }
    if fs_close(h) != 0 {
        eprintln_str(String::from("close error"));
        return 1;
    }

    print_i64(lines);
    print_str(String::from("\n"));
    print_i64(bytes);
    print_str(String::from("\n"));
    0
}
```

Key points:

- `fs_read_line(handle)` includes the trailing newline when present.
- Blank lines are returned as `"\n"`; EOF returns `""`.
- Check `fs_status(handle)` after `fs_read_line(handle)` returns `""`: `1` means EOF, `-1` means invalid handle or read error.
- Close each successful handle with `fs_close(handle)` and check for a non-zero close result.

## 7. BTreeMap basic usage

`BTreeMap<i64, V>` is the deterministic alternative to `HashMap` — sorted ascending by key, binary-search lookup. Use it when iteration order affects program output (codegen, serialization, or any reproducible build).

```vow
module BTreeMapExample

fn fetch(m: BTreeMap<i64, i64>) -> Option<i64> [io] {
    let r: Option<i64> = m.get(7);
    let v: i64 = r?;
    print_i64(v);
    print_str(String::from("\n"));
    Option::Some(v)
}

fn main() -> i32 [io] {
    let m: BTreeMap<i64, i64> = BTreeMap::new();
    m.insert(7, 42);
    let prev: Option<i64> = m.insert(7, 99);
    // prev is Some(42); the second insert overwrote the first.
    fetch(m);
    print_i64(m.len());
    0
}
```

Note that `.insert` returns `Option<V>` (the previous value, if any), and `.get` returns `Option<V>`. Use `?` to short-circuit on `None`. Phase 1 only supports `i64` keys; using any other key type raises `BTreeMapKeyTypeMustBeI64`.

### Why BTreeMap and not HashMap

`HashMap.insert` returns `()` and its iteration order is unspecified. For maps whose iteration is observable in the output binary, the byte-identical bootstrap requirement (`stage1 == stage2` sha256) demands deterministic order. `BTreeMap` provides it; `HashMap` does not.
