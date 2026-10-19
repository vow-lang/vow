
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
> markers require more than one hidden caller-arena slot (for example, the
> value is stored into two distinct parameter targets, or returned and also
> stored into a parameter) have no single caller arena that outlives every
> destination, so their inferred region widens to the root region (`Root`) —
> a strictly wider placement than any one escaped pointer requires, hence
> sound (leak-but-safe) — and they compile without a blocking error (issue
> #871). Such a widen-to-root placement is not silent, though: it surfaces a
> non-blocking `RegionRootEscape` **note** (issue #366; see below), so the
> permanent root-region placement is still visible. `RegionConflict`
> therefore fires only when a value's inferred region is a concrete block
> strictly narrower than the target container's region.

```vow
fn store_into(out: Vec<String>, prefix: String) [io] {
    let s: String = String::from(prefix);
    s.push_str(String::from(" world"));
    out.push(s);  // s is allocated in this function's scope but escapes into out's region
}
```

**Fix:** Move the allocation to a wider scope, or copy the value into the target region (e.g., `String::from(s)` into the outer arena). For routings that compile cleanly but you'd like to know about (root-region placement), see `RegionRootEscape` below. See `docs/design/arena_memory.md` §4.4 for the full rejection vs. visibility distinction.

Neither compiler rejects the snippet above today: it compiles and carries only a `RegionRootEscape` note (the self-hosted frontend reaches `RegionConflict` only through an internal-compiler-error path). Returning the value from the constructing function is the canonical shape and triggers neither:

**Right:**

```vow
fn make_greeting(prefix: String) -> String {
    let s: String = String::from(prefix);
    s.push_str(String::from(" world"));
    s
}
```

### RegionRootEscape

**Phase:** Region Inference (arena-per-scope, Phase 3)
**Severity:** Note (informational — does not fail the build)
**Meaning:** A heap allocation may land in the never-freed root region (`__vow_root_arena`). The note fires in either of two cases:

1. The allocation's inferred region is `Caller` and the surrounding function publishes a `FreshInCaller` return summary or store effect — so the value may flow up the caller chain to `main`.
2. The allocation's region **widens to the root region** without an intrinsic root pin (`pin_to_root` / a literal) — either because it is routed into more than one distinct hidden caller slot (a multi-slot widen), or because it reaches a container through a Phi over caller containers (a Phi widen). Both are sound (leak-but-safe) placements, but the allocation lives for the whole process.

This is a memory-cost decision the compiler surfaces visibly per `docs/design/arena_memory.md` §4.4: silent root-region placement caused growth-with-no-signal in earlier compiler versions, and the note restores that signal without conflating it with unsoundness (`RegionConflict`).

The note is conservative — it fires for any qualifying allocation in a function that could route to a caller or that widens to root, even if the actual concrete chain in this program doesn't reach `main`. False positives are tolerated because the diagnostic is non-blocking. A widen-to-root allocation is flagged even when it is also returned: being returned does not undo a root-region placement.

**Wrong:**

```vow
fn store_into(out: Vec<String>, prefix: String) {
    out.push(String::from(prefix));
}
```

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

**Right:**

```vow
fn make_name(prefix: String) -> String {
    String::from(prefix)
}
```

### CodegenUnsupported

**Phase:** Code Generation
**Meaning:** The program is valid Vow, but the selected native backend cannot
yet lower one of its operations or representations. The build fails closed:
`status` is `CompileFailed` and `executable` is `null`. No new executable is
produced — but a file already at the `-o` path from an earlier successful
build is left untouched, so branch on `status`/`executable` rather than on the
output path existing. The diagnostic currently has a file-level span because
backend errors do not carry an instruction origin.

**Wrong:**

```vow
fn remainder(a: f64, b: f64) -> f64 {
    a % b
}
```

**Fix:** Restructure the program to avoid the named backend limitation, or use
a supported representation until the backend implements it. Agents may branch
on `CodegenUnsupported` without parsing the human-readable `message` field.

**Right:**

```vow
fn remainder(a: i64, b: i64) -> i64 {
    a % b
}
```

### CodegenFailed

**Phase:** Code Generation
**Meaning:** The native backend failed while declaring, defining, or emitting
otherwise-lowered code. This distinguishes an internal backend failure from a
source-level parse/type error and from a known unsupported operation.

**Fix:** Inspect the diagnostic message for backend context. If the program
uses only documented supported operations, report a compiler bug with the
source and diagnostic; changing source syntax or types may not resolve an
internal backend failure.

**Right:** No source rewrite applies; the program is not at fault. Keep it unchanged and report the bug.

### LinkFailed

**Phase:** Linker
**Meaning:** Code generation produced an object, but the native linker could
not create the executable. A missing `libvow_runtime.a` is also reported with
this code.

**Wrong:**

```sh
vowc build f.vow   # libvow_runtime.a not found
```

**Fix:** Inspect the diagnostic message and verify that the host linker and Vow
runtime archive are installed and accessible. When building Vow itself, run
`cargo build --release --all` or set `VOW_RUNTIME_PATH` to the runtime archive.

**Right:**

```sh
VOW_RUNTIME_PATH=target/release/libvow_runtime.a vowc build f.vow
```

### IoError

**Phase:** Module loading, Code Generation

**Meaning:** The compiler could not read or write a file. Three build-pipeline
sites produce this code: a `use` declaration naming a module the driver cannot
read, a generated object's output directory the backend cannot create, and a
generated object file the backend cannot write (full disk, read-only
filesystem, or another write failure). The object bytes are already built by
the time either the directory creation or the write is attempted, so this is
the filesystem's answer rather than a backend defect — which is why it is not
`CodegenFailed`. When this code comes from module loading, compilation stops
before type-checking, so diagnostics do not include follow-on errors for names
from the missing module.

**Wrong:**

```vow
module Main

use util.mathh
```

**Fix:** Check the path in the diagnostic message. For a module load, verify
the `use` path resolves relative to the importing file. For an object write,
the compiler creates missing parent directories automatically; verify that no
path component is a regular file, that permissions allow creating directories
and writing the object, and that the filesystem is writable and has free
space. Re-running the same build after fixing the filesystem succeeds;
changing the source will not help.

For a module-load failure, the path must resolve to `util/math.vow` (or `util/math.vow.d`) next to the importing file:

**Right:**

```vow
module Main

use util.math
```
