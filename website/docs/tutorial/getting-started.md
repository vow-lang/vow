# Install & first program

## Build the compiler

`build/vowc` is the primary Vow compiler — a self-hosted, verified binary. Produce it
once from the Rust bootstrap compiler:

```console
$ git clone https://github.com/vow-lang/vow.git
$ cd vow
$ scripts/bootstrap.sh
```

This builds the Rust stage-0 compiler, then uses it to compile and verify the
self-hosted compiler, producing `build/vowc`. After this, `build/vowc` is all you
need for day-to-day work.

## Hello, world

Create `hello.vow`:

```vow
module Hello

fn main() -> i32 [io] {
    print_str("Hello, world!");
    0
}
```

A few things to notice:

- Every file declares a `module`.
- `main` returns `i32` and is annotated with the `[io]` **effect** — it performs
  input/output. Pure functions carry no effect annotation; calling an effectful
  function from a pure one is a type error.
- `print_str` is a builtin.

## Build and run

```console
$ ulimit -v 2000000; build/vowc build hello.vow
{"status":"Unverified","executable":"hello","diagnostics":[],"counterexamples":[]}

$ ulimit -v 2000000; ./hello
Hello, world!
```

## Reading the JSON status

`vow build` prints a single JSON object to stdout. The `status` field is the one to
branch on:

| `status` | Meaning | What to do |
|----------|---------|------------|
| `Verified` | Compiled and all contracts proven. | Done. Run `executable`. |
| `Unverified` | Compiled, but no contracts to check (or ESBMC unavailable). | Binary is still produced. |
| `CompileFailed` | Syntax or type error. | Read `diagnostics[]`, fix, retry. |
| `VerifyFailed` | A contract could not be proven. | Read `counterexamples[]`, fix, retry. |

`hello.vow` has no contracts, so it reports `Unverified` — it compiled fine, there was
just nothing to prove. In the [next step](first-contract.md) we add a contract and
get to `Verified`.
