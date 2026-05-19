# Build System Audit: `vowc` Feature Parity

**Date:** 2026-04-08
**Status:** Research / Proposal
**Comparison targets:** Cargo (Rust), Go toolchain, Zig build system

## Current Capabilities

`vowc` today is a **single-file-to-executable compiler** with integrated formal verification. Here is what works:

| Capability | Details |
|---|---|
| Compile to native executable | ELF via Cranelift, linked with system `cc` |
| Formal verification (ESBMC) | `vow build` (on by default), `vow verify` (verification only) |
| Module system | `use foo.bar` resolves to `<root>/foo/bar.vow`, recursive DFS loading |
| Declaration files | `vow decl` emits `.vow.d` type-signature stubs; importers prefer `.vow.d` over `.vow` |
| Build modes | `--mode debug` (runtime contract checks), `release` (default), `profile` (call counters) |
| Compilation + verification cache | On by default, disable with `--no-cache` |
| Structured JSON output | All diagnostics dual-emit JSON (stdout) + human-readable (stderr) |
| Runtime tracing | `--debug-trace calls\|full` emits JSON trace events to stderr |
| Contract listing | `vow contracts` lists all contracts with optional verification status |

### Linker details (`vow-codegen/src/linker.rs`)

- Uses system `cc` as linker driver
- Statically links `libvow_runtime.a` (and optionally `libvow_clif_shim.a`)
- Links `-lpthread -ldl -lm` for Rust std support
- Only produces executables; no library output path exists

### Module system details (`vow/src/module_loader.rs`)

- `use foo.bar` maps to `<root_dir>/foo/bar.vow` (or `.vow.d` if present)
- Recursive dependency loading in DFS order
- All modules merged into a single flat `Module` — no namespace isolation
- Duplicate imports are deduplicated via visited set

## Gap Analysis

### Tier 1: Required for distributable libraries

These are the minimum additions needed for Vow code to be reusable across projects.

#### 1.1 Static library output (`--lib`)

**What's missing:** The linker only invokes `cc` to produce an executable. There is no mode
to invoke `ar rcs` to create a `.a` archive from compiled object files.

**Cargo equivalent:** `crate-type = ["staticlib"]` in `Cargo.toml`
**Go equivalent:** Implicit — every package is a compiled unit
**Zig equivalent:** `addStaticLibrary()` in `build.zig`

**Implementation sketch:**
- Add `--output-type <exe|staticlib|cdylib>` flag to `vow build`
- For `staticlib`: skip the `link()` call, instead run `ar rcs <output>.a <objects...>`
- For `cdylib`: invoke `cc -shared -o <output>.so <objects...> <runtime>`
- Emit a companion `.vow.d` declaration file alongside the library

**Runtime linking policy.** Executable builds today statically link `libvow_runtime.a`
and pull in `-lpthread -ldl -lm` (`vow-codegen/src/linker.rs`). For `staticlib` output the
audit recommends producing **thin** archives: the `.a` contains only Vow-compiled object
files, and consumers are expected to link `libvow_runtime.a` plus its system dependencies
themselves. The companion `.vow.d` file documents the required link line. Producing "fat"
archives (using `ar` to merge `libvow_runtime.a` into the output) is rejected because it
would silently embed a particular runtime build into every library and break runtime
upgrades and cross-language LTO. `cdylib` output is the opposite — the shared object is
self-contained and links the runtime directly via `cc -shared`, because dynamic loading
consumers cannot be expected to provide it.

**Complexity:** Low. The object files are already produced by Cranelift; only the final
linking step changes.

#### 1.2 Shared library output (`--cdylib`)

**What's missing:** No `cc -shared` path. No way to produce `.so` / `.dylib` files.

**Why it matters:** Plugins, FFI-callable libraries, embedding Vow in other runtimes.

**Implementation:** Same flag as above with a different linker invocation.

#### 1.3 Project manifest (`vow.toml`)

**What's missing:** All configuration is via CLI flags. There is no way to declare:
- Project name and version
- Output type (executable vs library)
- Dependencies (even local path deps)
- Default build mode or flags
- Module root / source directory

**Cargo equivalent:** `Cargo.toml`
**Go equivalent:** `go.mod`
**Zig equivalent:** `build.zig.zon`

**Proposed format:**
```toml
[package]
name = "mylib"
version = "0.1.0"

# A package may declare any combination of `[lib]` and `[[bin]]` targets.
# Both are optional; if neither is present, vowc falls back to the
# convention entry points `src/lib.vow` and/or `src/main.vow`.

[lib]
output = "staticlib"           # staticlib | cdylib
path = "src/lib.vow"

[[bin]]
name = "mytool"
path = "src/main.vow"

[[bin]]
name = "mytool-helper"
path = "src/helper.vow"

[dependencies]
utils = { path = "../utils" }  # local path deps first; registry later

[build]
mode = "release"
verify = true
unwind = 10
```

**Implementation sketch:**
- `vowc` looks for `vow.toml` in the current directory (or parent directories)
- CLI flags override manifest values
- Each `[[bin]]` produces one executable; the `[lib]` section produces one library artifact
  (a package may emit both a library and one or more binaries, like Cargo)
- Convention entry points: `src/lib.vow` for the library, `src/main.vow` for the default
  binary when neither section names a path explicitly
- Dependencies are resolved as local paths initially, with registry support deferred

**Complexity:** Medium. Requires a TOML parser (or minimal hand-rolled format), manifest
resolution logic, and integration into the CLI driver.

#### 1.4 Namespace isolation

**What's missing:** `merge_modules()` concatenates all items from all modules into a single
flat list. Two modules defining a function `helper()` will collide silently.

**Cargo equivalent:** Crate-level namespacing, `use` paths
**Go equivalent:** Package-level namespacing
**Zig equivalent:** Per-file namespacing

**Impact:** At scale (>10 modules), name collisions become inevitable. This is the most
architecturally significant change because it touches the type checker, IR lowering, and
codegen (mangled names).

**Implementation sketch:**
- Prefix all non-`pub` items with their module name during merge
- For `pub` items, require explicit `use Module.item` to reference them
- Mangle function names with a **collision-free** scheme. A naive `Module__function_name`
  delimiter is ambiguous because identifiers may contain underscores
  (`vow-syntax/src/lexer.rs`), so e.g. module `a` with function `b_c` and module `a_b` with
  function `c` would both mangle to `a__b_c`. Use length-prefixed segments
  (`_V2a2bc` for `a::bc`, `_V3a_b1c` for `a_b::c`) or a reserved character sequence that is
  not part of the identifier alphabet (`a$bc` vs `a_b$c`). Length-prefixing is preferred
  because it stays inside C-identifier rules and so requires no escaping in the linker.
- Type checker tracks which module each item belongs to

**Complexity:** High. Touches most compiler passes. Per the CLAUDE.md dual-compiler sync
rule, this work must land simultaneously in the Rust stage-0 compiler **and** the
self-hosted `compiler/*.vow` modules — at minimum `checker.vow`, `ir.vow`, `lower.vow`, and
`clif.vow` — within the same change set. The binary-fixed-point invariant means a partial
implementation would either fail to bootstrap or produce non-reproducible binaries; the
upfront cost of the dual-edit is non-negotiable for this feature.

### Tier 2: Required for ecosystem / multi-project use

These enable a healthy ecosystem of reusable Vow packages.

#### 2.1 Test runner (`vow test`)

**Current state:** The command exists as a stub that prints "not yet implemented"
(`vow/src/main.rs`).

**Cargo equivalent:** `cargo test` discovers `#[test]` functions, runs them, reports results
**Go equivalent:** `go test` discovers `Test*` functions in `*_test.go` files

**Proposed design:**
- Test functions are annotated: `test fn test_division() { ... }`
- `vow test` discovers all `test fn` in the target and its dependencies
- Each test is compiled and executed; exit code 0 = pass, nonzero = fail
- Optionally run `vow verify` on test functions' contracts as well
- JSON output for each test: `{"test": "test_division", "status": "pass"|"fail", "ms": 12}`

**Differentiator:** `vow test --verify` could verify contracts on test functions, combining
runtime testing with formal verification in a single command. No other build tool does this.

**Complexity:** Medium. Requires a `test` keyword in the grammar, discovery logic, a test
harness that calls each test function, and result aggregation.

**Spec-sync prerequisites.** Because `test fn` is new surface syntax, the implementation
must — before or alongside the compiler work — update `docs/skill/grammar.md` (which is the
authoritative grammar spec per CLAUDE.md), re-run `uv run python scripts/generate_help.py`
to regenerate `--help` in both compilers, and let the staleness detector in
`scripts/check_help_coverage.py` (executed by `full_test.sh`) confirm `grammar.md` and
`--help` agree. Skipping the spec update would make the feature land out of compliance
with the project's "spec is the source of truth" rule.

#### 2.2 Dependency management

**What's missing:** No lock files, no version resolution, no remote fetching.

**Phase 1 (local path deps):** The manifest `[dependencies]` section supports
`name = { path = "..." }`. Module resolution checks dependency paths before the
filesystem-relative default.

**Phase 2 (git deps):** `name = { git = "https://...", rev = "abc123" }`. Fetch into a
local cache directory, resolve from there.

**Phase 3 (registry):** A central registry (like crates.io) with version constraints and
a resolver. This is a large undertaking and should be deferred.

**Complexity:** Phase 1 is low (manifest + path resolution). Phase 2 is medium. Phase 3 is
high.

#### 2.3 Workspace / multi-package projects

**What's missing:** No way to organize a project into multiple packages that share
dependencies and build configuration.

**Cargo equivalent:** `[workspace]` in root `Cargo.toml`
**Go equivalent:** Go workspace mode (`go.work`)

**Proposed:**
```toml
# vow-workspace.toml
members = ["compiler", "runtime", "tools/fmt"]
```

**Complexity:** Medium. Requires manifest discovery, shared dependency resolution, and
coordinated builds.

#### 2.4 Install command (`vow install`)

**What's missing:** No way to install a compiled binary to a system-wide or user-local
location (e.g., `~/.vow/bin`).

**Cargo equivalent:** `cargo install` → `~/.cargo/bin/`
**Go equivalent:** `go install` → `$GOPATH/bin/`

**Complexity:** Low. Build the executable, copy to a known directory, ensure it's on PATH.

#### 2.5 Init / new scaffolding (`vow init`)

**What's missing:** No project creation command.

**Proposed:** `vow init myproject` creates:
```
myproject/
  vow.toml
  src/
    main.vow
```

**Complexity:** Low.

### Tier 3: Required for cross-platform / production deployment

#### 3.1 Cross-compilation

**What's missing:** `cranelift_backend.rs` calls `cranelift_native::builder()` which locks
to the host platform. There is no `--target` flag.

**Cargo equivalent:** `cargo build --target aarch64-unknown-linux-gnu`
**Go equivalent:** `GOOS=linux GOARCH=arm64 go build`
**Zig equivalent:** `zig build -Dtarget=aarch64-linux-gnu`

**Implementation sketch:**
- Accept `--target <triple>` flag
- Use `cranelift_codegen::isa::lookup(triple)` instead of `cranelift_native::builder()`
- Cross-link requires a cross-compiler toolchain (`cc` replacement or explicit `--linker` flag)

**Complexity:** Medium for codegen (Cranelift supports it). Higher for linking (need
cross-toolchain support).

#### 3.2 Run command (`vow run`)

**What's missing:** Must `vow build -o /tmp/foo src/main.vow && /tmp/foo` manually.

**Proposed:** `vow run src/main.vow -- arg1 arg2` builds to a temp directory and executes.

**Complexity:** Trivial. Build to tempdir, exec, clean up.

#### 3.3 Formatter command (`vow fmt`)

**What's missing as a CLI command:** The canonical printer exists in `vow-syntax` (the
`parse -> print -> parse` idempotent printer). It is not exposed as a `vowc fmt` subcommand.

**Cargo equivalent:** `cargo fmt` (invokes rustfmt)
**Go equivalent:** `gofmt` / `go fmt`

**Complexity:** Low. The printer already exists; just wire it to a subcommand.

#### 3.4 Documentation generation (`vow doc`)

**What's missing:** No API documentation generator.

**Cargo equivalent:** `cargo doc` → HTML docs from doc comments
**Go equivalent:** `go doc` / `godoc`

**Complexity:** Medium. Requires a doc-comment convention and an HTML/Markdown emitter.

#### 3.5 Clean command (`vow clean`)

**What's missing:** No way to clear the build cache or artifacts directory.

**Complexity:** Trivial. Remove `build/` and cache directories.

#### 3.6 Benchmarking

**Current state:** `--mode profile` provides call counters, but there is no
micro-benchmarking harness.

**Cargo equivalent:** `cargo bench` with Criterion
**Go equivalent:** `go test -bench`

**Complexity:** Medium.

### Tier 4: Nice to have

| Feature | Notes | Complexity |
|---|---|---|
| REPL | Uncommon for compiled languages; useful for exploration | High |
| Linter (`vow lint`) | Verification covers many lint concerns already | Medium |
| Build scripts / codegen hooks | Pre/post-build steps | Medium |
| Conditional compilation (`cfg`) | Build tags, platform-specific code | Medium |
| C header generation | Enable C/C++ interop with Vow libraries | Low-Medium |

## Recommended Implementation Roadmap

### Phase 1: Library output (enables code reuse)
1. `vow build --output-type staticlib` — invoke `ar` instead of `cc`
2. `vow build --output-type cdylib` — invoke `cc -shared`
3. Auto-emit `.vow.d` alongside library artifacts

### Phase 2: Manifest + namespace isolation (enables multi-file projects and reusable libraries)
4. `vow.toml` parser and manifest resolution
5. Local path dependencies in `[dependencies]`
6. Default entry points (`src/main.vow`, `src/lib.vow`)
7. **Namespace isolation in the module system** — a Tier 1 capability, intentionally landed
   here alongside path dependencies because the moment two packages can be combined into a
   build, silent symbol collisions become unavoidable. Without this, "library output" from
   Phase 1 is not actually safe to consume.

### Phase 3: Developer experience (enables practical workflows)
8. `vow test` — test discovery, execution, and optional verification
9. `vow run` — build + execute in one step
10. `vow fmt` — expose canonical printer
11. `vow clean` — remove build artifacts and caches
12. `vow init` — project scaffolding

### Phase 4: Ecosystem (enables sharing)
13. `vow install` — install binaries to `~/.vow/bin`
14. Git-based dependencies
15. Cross-compilation via `--target`

### Phase 5: Scale (enables large projects)
16. Workspace support (`vow-workspace.toml`)
17. Registry-based dependencies
18. Documentation generation (`vow doc`)
19. C header generation for FFI

## Summary

`vowc` is currently a **compiler**, not a **build system**. It compiles single files (with
recursive module loading) into executables and verifies contracts. To reach feature parity
with Cargo/Go/Zig as a build tool, the highest-impact additions are:

1. **Library output** — the single biggest unlock; everything else builds on this
2. **Project manifest** — makes multi-file projects manageable without CLI flag gymnastics
3. **Test runner** — especially with verification integration, this is a unique differentiator

The verification pipeline is a genuine competitive advantage that no other build tool offers.
The roadmap should preserve this strength while adding the project management capabilities
that developers expect from a modern language toolchain.
