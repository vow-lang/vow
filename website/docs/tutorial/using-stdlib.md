# Using the standard library

Vow ships a [standard library](../stdlib.md) of reusable, contract-annotated modules —
`math`, `heap`, `stack`, `geometry`, `bignum`, and `gc`. This page shows how to pull one
into your own program.

## The consumption model

Vow has **no module search path** (yet). A `use foo` declaration resolves to
`foo.vow` **in the directory of the entry file** you pass to the compiler, and every
transitive `use` resolves against that same directory. There is no global import, and
`--module-root` exists only on `vow test`.

So there are two practical ways to use a stdlib module:

### 1. Run a module's demo in place

Each module ships a `main.vow` you can build and run directly:

```console
$ ulimit -v 2000000; build/vowc build --no-verify stdlib/math/main.vow -o /tmp/math_demo
$ ulimit -v 2000000; /tmp/math_demo
```

### 2. Copy the module into your project

Because `use` resolves next to your entry file, copy the library file there:

```console
$ cp stdlib/math/arithmetic.vow myproject/arithmetic.vow
```

```vow
module Main
use arithmetic

fn main() -> i32 [io] {
    print_i64(clamp(15, 0, 10));   // 10
    print_i64(max_example());
    0
}

fn max_example() -> i64 {
    max(3, 7)                      // 7
}
```

```console
$ ulimit -v 2000000; build/vowc build --no-verify myproject/main.vow -o myproject/app
$ ulimit -v 2000000; myproject/app
```

For a multi-file module, copy **all** its sibling files together — e.g. `geometry`
ships `shape.vow`, which internally does `use point`, so `point.vow` must come along.

## What the contracts buy you

When you call `clamp(15, 0, 10)`, you inherit its contract: `requires: lo <= hi` and
`ensures: lo <= result <= hi`. If your call site can violate the precondition, the
verifier tells you — with blame on your code, the caller.

!!! warning "Not every module verifies statically — yet"
    Today only **`geometry`** verifies end-to-end under the ESBMC model. Modules that
    allocate `Vec`s per call (`stack`, `bignum`, most of `heap`) are currently skipped
    by the verifier, `math` is blocked by a name collision with C's `abs`, and `gc` has
    an open precondition issue. Their contracts are still **enforced at runtime in
    `--mode debug`** — they are precise specifications, just not all statically proven
    yet. The [Standard library reference](../stdlib.md#verification-status) documents
    the exact status of each module.

That's the tour. From here, the [Language reference](../reference/index.md) has the full
details, and the [Standard library](../stdlib.md) documents every module's API.
