# Arena-per-scope memory model

**Status:** Normative design specification. In progress.

This document specifies Vow's arena-per-scope memory model. It is the
authoritative design document for heap allocation, reference lifetimes,
and the compiler passes that implement them. `docs/vow_design.md §5.6`
lists arena-per-scope as the intended memory model; this document is
the concrete semantics, ABI, and migration plan that fulfills it.

When this document and the implementation disagree, this document is
normative. Implementation artifacts that conflict with it are
transition debt.

## 0. Relationship to existing design

- `docs/vow_design.md §5.6` establishes arena-per-scope as the intended
  memory model. This document is that model's concrete form.
- `docs/vow_design.md §5.3` commits to a small, nominal, decidable type
  system. This spec respects that: **regions are not part of the type
  system**. No type carries a region parameter. No surface syntax names
  a region.
- Issue #186 proposed `owned` / `alias` return qualifiers. This model
  subsumes that proposal: implicit region inference replaces the
  manual annotations, and the underlying bug class (misplaced `free`
  on arena-aliasing returns) is eliminated by construction.

## 1. Terminology

- **Block.** A brace-delimited syntactic scope in Vow source. Function
  bodies, loop bodies, `if`/`else` arms, `match` arms, and freestanding
  `{ ... }` expressions are all blocks.
- **Region.** A logical ownership domain associated with zero or one
  block. Every block is a potential region; the compiler elides
  regions for blocks that make no heap allocations.
- **Arena.** The runtime realization of a region. Holds heap
  allocations associated with its region. See §3.
- **Heap-typed value.** A value whose representation contains a
  pointer into memory the compiler manages via regions. `String`,
  `Vec<T>`, `HashMap<K, V>`, and struct/enum values containing
  heap-typed fields are heap-typed. `i64`, `bool`, `f64`, and other
  primitives are not heap-typed.
- **Region summary.** A compact per-function record describing how
  the function treats heap values at its boundary. See §4.2.
- **Escape.** A value is said to escape a region when it is
  referenced, directly or transitively, from outside that region.
  The region pass places escaping values in the innermost enclosing
  region large enough to cover all references.
- **Root region.** A single arena created at program startup and
  never closed. Parent of `main`'s top-level region. See §6.

## 2. Allocation model

### 2.1. Uniform arena allocation

Every heap-typed value MUST be allocated in exactly one arena. There
is no other heap-allocation mechanism exposed by the language. In
particular:

- `String`, `Vec<T>`, `HashMap<K, V>` backings and descriptors are
  arena-allocated.
- Struct and enum values that contain heap-typed fields are
  arena-allocated; their heap-typed fields are pointers into some
  arena (possibly the same one as the containing struct, possibly
  another).
- Scalar values (`i8`..`i128`, `u8`..`u128`, `bool`, `f32`, `f64`,
  `usize`) are not heap-typed and are not arena-allocated. They
  live in registers or on the machine stack as today.

The compiler assigns each heap-producing instruction a region via a
compiler pass (§4). The assignment determines which arena backs the
allocation.

### 2.2. No explicit free

Vow source code MUST NOT contain explicit free operations. No
`drop`, no `free`, no `release`. Heap memory is reclaimed exactly
when its containing arena closes.

Early reclamation of a specific value is not supported. The
mechanism for shortening a value's lifetime is restructuring the
source so the value is allocated in a tighter inner block; the
region pass will then close the tighter arena sooner.

### 2.3. Allocation cost

Allocation is bump-pointer in the common case: compare cursor
against chunk end, advance cursor, return the aligned pointer.
Chunk overflow triggers a single `malloc` for the next chunk.
Arena close walks the chunk chain and calls `free` for each
chunk.

## 3. Runtime representation

### 3.1. Arena header

Each arena is represented by a 48-byte header:

```c
struct VowArena {
    void*     first_chunk;      // head of chunk chain
    void*     current_chunk;    // active chunk (tail)
    uintptr_t cursor;           // next allocation address within current chunk
    uintptr_t chunk_end;        // one past last usable byte in current chunk
    void*     last_alloc_start; // most recent allocation, for try_extend
    uintptr_t last_alloc_size;  // size of most recent allocation
};
```

Arena headers MUST be stack-allocated wherever possible. A block
region's header lives in the stack frame of the enclosing function.
The root region (§6) is the exception: its header lives in `.bss`.

### 3.2. Chunk layout

Chunks are allocated via libc `malloc`. Each chunk is 4096 bytes plus
one pointer (8 bytes) for the next-chunk link. Allocations larger
than 2048 bytes (half-chunk threshold) are placed in a custom-sized
chunk sized to fit the allocation plus the link.

Chunks form a singly-linked list rooted at `first_chunk`. The
`current_chunk` always points to the tail. The next-chunk pointer of
the tail is `NULL`.

### 3.3. Runtime API

The following C-callable primitives MUST be provided by `vow-runtime`:

```c
void  __vow_arena_open(struct VowArena* a);
void  __vow_arena_close(struct VowArena* a);
void* __vow_arena_alloc(struct VowArena* a, uintptr_t bytes, uintptr_t align);
bool  __vow_arena_try_extend(struct VowArena* a, void* ptr,
                             uintptr_t old_size, uintptr_t new_size);
```

**`__vow_arena_open(a)`**: initializes `*a` to an arena with one
freshly-allocated chunk of 4 KB.

**`__vow_arena_close(a)`**: walks the chunk chain, calls `free` on
each chunk, leaves `*a` in an undefined state. Callers MUST NOT
dereference `a` after close.

**`__vow_arena_alloc(a, bytes, align)`**: returns an aligned pointer
into `a`'s current chunk. If the current chunk does not have room,
allocates a new chunk (size per §3.2) and links it at the tail.
Updates `cursor` and `chunk_end` to the new chunk. Also records
`ptr` and `bytes` in `last_alloc_*`.

**`__vow_arena_try_extend(a, ptr, old_size, new_size)`**: returns
`true` if and only if `ptr == a->last_alloc_start` AND
`a->last_alloc_size == old_size` AND the current chunk has
`(new_size - old_size)` bytes remaining after the existing
allocation. On success, bumps the cursor by the additional bytes
and returns `true`; the caller may treat the allocation as extended
in place without copying. On failure, returns `false`; the caller
MUST fall back to a fresh `__vow_arena_alloc` + `memcpy`.

### 3.4. Determinism

All runtime operations MUST be deterministic. Chunk size is fixed
at 4 KB (with the oversized-allocation exception). No random padding,
no allocator-dependent layout. Binary fixed point is preserved only
if the runtime is deterministic.

### 3.5. Empty-region elision

A block region with no heap allocations MUST NOT emit any arena
operations. The region pass tracks which blocks contain heap
allocations (directly or via inner calls with store effects into the
block's region) and emits `__vow_arena_open` / `__vow_arena_close`
only for non-empty regions.

## 4. Region inference

### 4.1. Algorithmic shape

Region inference is a **parallel metadata pass** that runs after type
and effect checking. It operates on IR, not on AST. It MUST NOT
modify types.

The algorithm is block-tree dataflow:

1. For each function, build the block tree. The block tree's root is
   the function body; children are nested blocks. A virtual "caller"
   node is the parent of the function body for purposes of escape.
2. For each heap-producing instruction `I` at block `B`, compute
   `must_outlive(I)` — the set of blocks the value must remain live
   in for every use to be legal. Members of `must_outlive(I)` are
   either concrete blocks in the same function or the virtual caller
   node.
3. Compute `region(I) = LUB(must_outlive(I))` in the block tree.
   The LUB is the innermost block that is an ancestor of every block
   in `must_outlive(I)`.
4. If no such LUB exists (for example, the value must outlive two
   sibling blocks with no ancestor relationship), the program is
   rejected with `E_REGION_CONFLICT` (§13).
5. If `region(I)` is the virtual caller node, the function's summary
   (§4.2) records the allocation as escaping to the caller.
6. Otherwise, `region(I)` names a concrete block; the allocation is
   placed in that block's arena.

`must_outlive(I)` is computed by following use-def chains:

- A use as the return expression adds the virtual caller node.
- A use as the source of `obj.field = I` or `vec.push(I)` or
  `map.insert(k, I)` adds the region of `obj` (respectively `vec`,
  `map`).
- A use as an argument to a call is resolved via the callee's
  region summary: if the callee's store effects say the parameter
  is stored into another parameter `j`'s region, the caller adds
  `region(j-th argument)` to `must_outlive(I)`.

### 4.2. Region summary

Every function in a compiled module MUST carry a region summary in
the module's metadata:

```
RegionSummary {
    param_regions: Vec<RegionVar>,
    return_region: RegionConstraint,
    store_effects: Vec<StoreEffect>,
}

RegionVar = abstract placeholder named by parameter index

RegionConstraint =
    | FreshInCaller                   // allocates a value returned to the caller
    | AliasOf(param_index)            // return aliases parameter param_index's region
    | AliasOfAny(Vec<param_index>)    // return aliases an LUB of multiple parameters
    | ConstantGlobal                  // return points into .rodata or the root region

StoreEffect {
    target: param_index,              // the parameter being written into
    source: RegionConstraint,         // region of stored values (MUST be >= target's)
}
```

`RegionVar` values are placeholders meaningful only within a
function. At each call site, the caller substitutes its concrete
block regions for the callee's `RegionVar`s.

Summaries are inferred, never written by the programmer. They appear
in module metadata only; there is no surface syntax.

### 4.3. Fixed-point for recursion

Mutually recursive functions form strongly-connected components in
the call graph. Within an SCC, summaries MUST be computed via
monotone fixed-point iteration:

1. Seed every function in the SCC with a conservative summary:
   `return_region = FreshInCaller` for heap returns; empty store
   effects; `param_regions` are free variables.
2. Re-analyze each function with the current summaries.
3. Update summaries.
4. Repeat until no summary changes.

Convergence is guaranteed because the `RegionConstraint` lattice is
finite and the update is monotone (summaries only become more
permissive).

### 4.4. Rejection, not over-approximation

When region inference encounters a placement conflict (no LUB exists
in the block tree), the program MUST be rejected with
`E_REGION_CONFLICT`. The compiler MUST NOT silently promote the
value to the root region.

Rationale: silent root-region placement causes memory growth for the
program lifetime with no visible signal to the agent. Structured
rejection is the feedback signal the CEGIS workflow depends on.

### 4.5. Interaction with other passes

Region inference runs after: type checking, effect checking, linear
consumption checking.

Region inference runs before: lowering to Cranelift IR, lowering to
ESBMC C model.

The linear-region interaction check (§9) runs between region
inference and lowering.

## 5. ABI and return convention

### 5.1. Hidden target_region parameter

A function whose region summary says `return_region = FreshInCaller`
MUST have an additional hidden parameter of type `*VowArena`, named
`target_region` in documentation. The ABI is:

```
fn user_visible(args...) -> T
    // Rust/self-hosted ABI
lowers to
fn user_visible(args..., target_region: *VowArena) -> T
    // machine ABI
```

Callers pass the address of whichever region the return should be
placed into (typically an enclosing block's arena).

### 5.2. ABI by summary

For each function:

- `FreshInCaller` return: ABI adds `target_region`.
- `AliasOf(i)` return: ABI adds no hidden parameter. Return is a
  pointer computed from parameter `i`.
- `AliasOfAny(...)` return: ABI adds no hidden parameter. Return is a
  pointer computed from parameters; caller is responsible for
  ensuring the LUB of the aliased parameters' regions is the
  intended region.
- `ConstantGlobal` return: ABI adds no hidden parameter. Return
  points into `.rodata` or the root region.
- Scalar-returning functions: no hidden parameter regardless of
  parameters.

Multiple escaping heap outputs require one `target_region` per
distinct target region in the summary. In practice this is almost
always one.

### 5.3. Within-function allocation

Allocations whose `region(I)` is a concrete block `B` inside the
callee lower to:

```
__vow_arena_alloc(&B_arena, bytes, align)
```

where `B_arena` is the stack-allocated VowArena header for block
`B`.

Allocations whose `region(I)` is the virtual caller node lower to:

```
__vow_arena_alloc(target_region, bytes, align)
```

using the function's hidden parameter.

### 5.4. Main entry point

`main` receives `target_region = &__vow_root_arena` from the runtime
startup shim. If `main`'s return is `i64` (the typical case), the
parameter is unused. The uniform ABI is preserved regardless.

## 6. Program-lifetime storage

### 6.1. Class I: compile-time literals

String literals, array literals, and any other constant known at
compile time MUST be placed in `.rodata`. The surrounding descriptor
(e.g., the 24-byte `{ptr, len, cap}` of `String`) is constructed
with `cap: 0`, indicating read-only backing.

Any operation that would mutate the backing of a `cap: 0` value
MUST trap at runtime with `E_REGION_LITERAL_MUTATION` (§13). The
compiler MUST NOT silently promote literal-backed values to heap
copies on mutation. Programs that need a mutable copy use
`String::from(literal)` or equivalent explicit copy.

### 6.2. Class II/III: root region

A single arena named `__vow_root_arena` MUST be initialized by the
runtime before `main` is called. Its header lives in `.bss`. It is
never closed.

The root region holds:

- Module-level `const` expressions with heap types (when such
  constants are admitted by the language).
- Startup-computed values that must survive for the program
  lifetime.
- Values pinned via `pin_to_root` (§7).

Functions whose region summary returns `ConstantGlobal` either
point into `.rodata` (compile-time-known) or into the root region
(runtime-computed). The distinction is transparent to callers; both
are valid for arbitrary lifetimes.

### 6.3. Root region never shrinks

The root region's chunks are not reclaimed during program execution.
Memory allocated there lives until process exit. Routing a value
into the root region is a one-way operation.

Accidental root-region placement is prevented by §4.4: region
inference rejects rather than over-approximates to root. Explicit
root placement (`pin_to_root`) is a visible source operation.

## 7. Container growth

### 7.1. Growth strategy

`Vec<T>`, `HashMap<K, V>`, and `String` grow by allocating a new
larger backing in the same arena as the current backing and copying.
The old backing is not freed. It remains allocated in the arena
until the arena closes.

At peak, a container that has doubled through `N` growths holds
`O(N)` bytes of orphaned backings. The total is bounded by twice
the current live capacity (identical to classical doubling
`realloc`).

### 7.2. Zero-copy extension

Growth MUST attempt `__vow_arena_try_extend` before falling back to
fresh allocation. For the "build up one buffer" pattern where the
container's backing is the most recent allocation in the arena,
extension succeeds and growth is O(1) amortized with no copy and no
orphaned backing.

### 7.3. Mutation of literal-backed containers

`cap: 0` containers are literal-backed (§6.1). Operations that would
grow or mutate the backing (`Vec::push`, `String::push_str`,
`HashMap::insert`, etc.) MUST trap with `E_REGION_LITERAL_MUTATION`.
Agents wanting a mutable copy must explicitly copy via
`Vec::from(literal)`, `String::from(literal)`, etc.

### 7.4. FFI visibility of orphaned backings

When a container's backing is shared across an FFI boundary (§8) and
the container grows, the C side retains a valid pointer to the old
backing until the arena closes. This is strictly safer than the
analogous `realloc` behavior (which would have freed the backing).
Documented as a positive consequence, not a requirement on the
caller.

## 8. FFI boundary

### 8.1. Opaque boundary

The region system stops at `extern` declarations. Extern
declarations MUST NOT carry region annotations; region semantics at
the FFI boundary live in the Vow-side wrapper, not in the extern
signature.

Extern functions themselves have no region summary. Callers treat
them as `[unsafe]` calls that interact with C-ABI pointers.

### 8.2. Vow → C (passing Vow values to C)

The default is **call-scoped sharing**: C receives a raw pointer
into the current Vow arena. The wrapper assumes C reads/writes for
the duration of the call and does not retain.

If C retains the pointer (stores it in a static, registers a
callback, etc.), the wrapper MUST place the value in a region with
program-lifetime — in practice, the root region via `pin_to_root`.

### 8.3. C → Vow (C returns a pointer)

Externs that return heap pointers MUST be wrapped by a Vow function.
The wrapper:

1. Calls the extern.
2. If the C pointer is heap-allocated, copies the bytes into the
   wrapper's `target_region` via `__vow_arena_alloc` + `memcpy`.
3. Calls the corresponding C-side `free` on the extern's pointer.
4. Returns the Vow-placed value.

The wrapper's region summary emerges from its body: a wrapper that
allocates into `target_region` has `FreshInCaller`; a wrapper that
returns a `.rodata` pointer has `ConstantGlobal`.

### 8.4. Stdlib helpers

The stdlib MUST provide:

- `pin_to_root<T>(value: T) -> T` — allocates in the root region
  and moves the value there. Idempotent on already-root values.
- `String::from_raw_parts_copy(ptr: *const u8, len: usize) -> String`
  — copies bytes from a raw C pointer into `target_region` as a
  `String`. `FreshInCaller`.
- `Vec::from_raw_parts_copy<T>(ptr: *const T, len: usize) -> Vec<T>`
  — analogous for `Vec<T>`.

These helpers encapsulate the canonical wrapper pattern so agents
can compose FFI without hand-rolling the region wiring.

## 9. Linear types × regions

### 9.1. Post-region check

After region inference, the compiler MUST run a linear-region
consistency check. The check verifies, for every `linear` value
`v`:

```
consumed_at(v) <= region_close(region(v))
```

That is, the consumption site of `v` MUST occur at or before the
close of the region containing `v`. Violation is rejected with
`E_REGION_LINEAR` (§13).

### 9.2. Region close does not consume

Arena close reclaims memory. It MUST NOT be treated as a
consumption site for linear values. A linear value whose region is
about to close and which has not been explicitly consumed is a
compile error, not an implicit drop.

### 9.3. Escape transfers the obligation

A linear value returned from a function (or otherwise escaping its
allocating region) has its linear obligation transferred to the
caller. The callee's region close is satisfied by the escape; the
caller inherits the consume-exactly-once requirement.

## 10. ESBMC modeling

### 10.1. C model

The arena primitives lower to ordinary `malloc` / `free` in the C
model seen by ESBMC:

- `__vow_arena_open` → `malloc(4096 + sizeof(void*))` for first
  chunk + initialization of the header.
- `__vow_arena_alloc` → pointer-in-bounds check + `malloc` on
  overflow.
- `__vow_arena_close` → loop `free` over the chunk chain.
- `__vow_arena_try_extend` → pure bookkeeping; no allocation.

ESBMC MUST see standard `malloc` / `free` pairs. No new primitive is
introduced to ESBMC's model.

### 10.2. Unwinding

ESBMC unwinds the chunk-chain walk in `__vow_arena_close` bounded by
the maximum chain length the analysis admits. The default
`--unwind` SHOULD be 4, sufficient for most block-local arenas.
Accumulating loops may require higher `--unwind`, exposed via
`VerifyLimits`.

### 10.3. Root region

`__vow_root_arena` is modeled as an ordinary arena whose
`__vow_arena_close` is never reached in the call graph. Its chunks
appear in the model as `malloc` without matching `free`. This is
semantically correct (program lifetime) and does not trigger
false-positive leak detection because leak detection respects
process-exit semantics.

### 10.4. Verified invariants of the arena primitive itself

The arena primitive MUST be verified in isolation during Phase 1
(§15). Specifically:

- `cursor <= chunk_end` at all times.
- Every pointer returned by `__vow_arena_alloc` lies within the
  current chunk's usable range.
- The chunk chain is acyclic.
- Every chunk is freed by `__vow_arena_close`.
- `__vow_arena_try_extend` modifies only `cursor`, never copies data.

## 11. Contracts × regions

### 11.1. Contracts stay purely logical

`requires` / `ensures` / `invariant` clauses MUST NOT reference
regions. No predicate of the form `lifetime(x) >= lifetime(y)` or
`region(x) == some_region`. No new contract vocabulary.

Contract predicates reason about values. Region inference guarantees
that every reference named in a contract is live at the contract's
check site. Failing that guarantee is a region-pass error, not a
contract-verification failure.

### 11.2. Purity unchanged

A function with no effect annotation is pure (§5.5 of
`docs/vow_design.md`). Arena allocation is not a visible effect;
pure functions may still allocate into arenas.

### 11.3. Memory leaks are not a contract property

A program that accidentally routes values to longer-lived regions
still satisfies every contract. Contracts are about correctness;
memory use is a separate concern. If memory-bound verification is
desired in the future, it is a new analysis, not an extension to
vow contracts.

## 12. IR representation

### 12.1. Region identifier

The IR gains a `RegionId` enum:

```
RegionId =
    | Block(BlockId)     // named block within the current function
    | Root               // root region
    | Rodata             // .rodata static storage
    | Caller             // caller-provided target_region (FreshInCaller returns)
```

Every heap-producing `Inst` carries a `region: RegionId` field.

### 12.2. No new opcodes for allocation placement

Existing allocation opcodes (`StringNew`, `VecNew`, etc.) are not
renamed. The region pass attaches the `RegionId` to the existing
opcode; lowering consumes the `RegionId` to select the correct
arena pointer in the emitted call to `__vow_arena_alloc`.

### 12.3. Block region opcodes

Two new IR opcodes mark region boundaries:

- `RegionOpen(BlockId)` — emitted at block entry when the block has
  at least one allocation assigned to its region.
- `RegionClose(BlockId)` — emitted at all exits of the block
  (normal exit, `break`, `return`).

Empty-region blocks do not emit these opcodes.

### 12.4. Function metadata

The IR's `FunctionData` structure gains a `RegionSummary` field
(§4.2). The summary is computed by the region pass and read by
callers during interprocedural analysis.

## 13. Error taxonomy

New diagnostic error codes introduced by the arena model:

### 13.1. `E_REGION_CONFLICT`

Emitted when region inference cannot compute a consistent LUB for a
value.

JSON shape:
```json
{
  "error": "E_REGION_CONFLICT",
  "severity": "error",
  "value_span": { "start": N, "len": M },
  "conflicting_sources": [
    { "span": { "start": N, "len": M }, "region_hint": "parameter `a`" },
    { "span": { "start": N, "len": M }, "region_hint": "parameter `b`" }
  ],
  "suggestions": [
    "copy at one branch to unify regions",
    "return the value and let the caller place it"
  ]
}
```

### 13.2. `E_REGION_LINEAR`

Emitted when a linear value is unconsumed at the close of its
region.

JSON shape:
```json
{
  "error": "E_REGION_LINEAR",
  "severity": "error",
  "value_name": "f",
  "value_span": { "start": N, "len": M },
  "allocated_at": { "start": N, "len": M },
  "expected_consumption_before": { "start": N, "len": M },
  "suggestions": [
    "consume the value via a sink function before block exit",
    "return the value to transfer the linear obligation to the caller"
  ]
}
```

### 13.3. `E_REGION_LITERAL_MUTATION`

Runtime trap emitted when a mutation operation is attempted on a
`cap: 0` literal-backed container.

JSON shape (runtime):
```json
{
  "error": "E_REGION_LITERAL_MUTATION",
  "severity": "runtime",
  "operation": "String::push_str",
  "value_origin": ".rodata literal",
  "suggestion": "use String::from(literal) to obtain a mutable copy"
}
```

### 13.4. `E_REGION_ABI_MISMATCH`

Emitted at link/load time when a callee's region summary has
changed and callers have stale metadata. Indicates a build
inconsistency; not a program-level error.

JSON shape:
```json
{
  "error": "E_REGION_ABI_MISMATCH",
  "severity": "error",
  "callee": "module::function",
  "caller_expected_summary_hash": "sha256:...",
  "callee_actual_summary_hash": "sha256:...",
  "suggestion": "rebuild the caller module against the current callee metadata"
}
```

### 13.5. Error stability

Error codes MUST be stable across compiler versions. New codes may
be added; existing codes MUST NOT change meaning. Documented in
`docs/spec/errors.md`.

## 14. Out of scope (v1)

The following are intentionally not part of v1 of this model. They
may be revisited later but are outside the scope of the initial
implementation:

- **Block-level sub-regions computed purely for peak-memory
  optimization.** v1 places every allocation at the tightest region
  the block-tree LUB permits, but does not perform aggressive
  hoisting/sinking to improve peak memory further.
- **Concurrent arenas.** §5.9 of `docs/vow_design.md` does not
  commit to a concurrency model. Arenas described here are
  single-threaded. When concurrency lands, arenas become
  thread-local; cross-thread escape is a future design question.
- **Dynamic long-lived regions.** Only the root region is
  program-lifetime. Future workloads may need multiple long-lived
  regions with explicit lifetime boundaries; v1 does not provide
  them.
- **Eager per-value free.** The "drop this early" operation is
  deliberately absent. If a pattern needs it, the solution is a
  tighter inner block.
- **Region predicates in contracts.** §11 explicitly excludes
  these. If a future use case demands them, it requires a separate
  design document.
- **Mandatory contracts on `extern` declarations.** Listed as
  `Target` in `docs/vow_design.md §5.8`. Orthogonal to this model;
  can land later without changing region semantics.

## 15. Migration plan

Migration is phased. Each phase is reviewable in isolation. Phases
that change observable semantics MUST update both compilers (Rust
bootstrap and self-hosted) atomically — the two must agree on
lowering at every merge boundary.

### Phase 0 — Design spec (this document)

Land this document as a reviewed PR. Cross-reference from
`docs/vow_design.md §5.6`. Upgrade arena-per-scope status from
`Target` to `In Progress`.

### Phase 1 — Runtime arena primitive

Add `__vow_arena_open`, `__vow_arena_close`, `__vow_arena_alloc`,
`__vow_arena_try_extend` to `vow-runtime`. Implement the VowArena
header, chunk-chained bump allocator, 4 KB chunks.

Standalone C-level tests + ESBMC verification on the primitive in
isolation (§10.4). Runtime ships unused by the compiler.

### Phase 2 — IR extension (both compilers, atomic)

**2a (Rust):** `vow-ir` gains `RegionId`, `region: RegionId` field
on heap-producing `Inst`s, `RegionSummary` in function metadata.
Module-format version bump. Default every heap inst to
`RegionId::Root` (conservative placeholder).

**2b (self-hosted):** `compiler/ir.vow` gets matching `RegionId`
representation. `compiler/ast.vow` gains `RegionSummary` slot on
the function record. `compiler/ir_printer.vow` prints region info.

**Gate:** bootstrap triple still passes; no behavior change.

### Phase 3 — Region pass (both compilers, atomic)

**3a (Rust):** `vow-ir/src/region.rs` (new). Block-tree dataflow,
SCC fixed-point, per-function summary computation,
`E_REGION_CONFLICT` emission. Runs after type/effect/linear checks;
populates IR region fields; lowerer still ignores them.

**3b (self-hosted):** `compiler/region.vow` (new). Port of 3a.

**Gate:** the two region passes MUST produce identical summaries on
the self-hosted compiler's own source. Zero diff required.

**Side CI job:** run the region pass on `compiler/*.vow` and emit
any `E_REGION_CONFLICT` as non-blocking warnings. This surfaces
source-level fix-ups needed in Phase 5 early.

### Phase 4 — Lowering cutover (both compilers, atomic)

**4a (Rust):** rewrite `vow-ir/src/lower/mod.rs`:
- Remove `track_heap_alloc` / `mark_escaped` for heap values.
- Emit `__vow_arena_alloc(region_handle, bytes, align)` using
  `inst.region`.
- Emit `__vow_arena_open` / `__vow_arena_close` at block
  entry/exit for non-elided regions.
- Thread hidden `target_region` through `FreshInCaller` callers.

**4b (self-hosted):** mirror 4a in `compiler/lower.vow` and
`compiler/clif.vow`.

`__vow_string_free` / `__vow_vec_free` / `__vow_hashmap_free`
become no-ops (deleted in Phase 8).

**Gate:** `cargo test --all` passes. `build/vowc test` passes. No
correctness regression. Performance regression allowed; logged for
Phase 9.

Binary fixed point is temporarily broken; re-established in Phase 5.

### Phase 5 — Bootstrap triple re-establishment + source fix-ups

**5a.** Run the Phase 4 Rust compiler on every `compiler/*.vow`
file. Collect `E_REGION_CONFLICT` emissions.

**5b.** Fix each conflict at the source level. Each fix is a small
`.vow` edit plus a regression test.

**5c.** Bootstrap triple:
1. Rust compiler → `/tmp/compiler_a`.
2. `/tmp/compiler_a` → `/tmp/compiler_b`.
3. `/tmp/compiler_b` → `/tmp/compiler_c`.
4. `sha256sum /tmp/compiler_b /tmp/compiler_c` MUST match.

**5d.** Replace `build/vowc` with the fixed-point binary.

**Gate:** binary fixed point re-established. No Phase 6+ begins
until this succeeds.

### Phase 6 — Linear × region integration

Implement the unconsumed-linear-at-region-close check (§9) in both
compilers. Add `E_REGION_LINEAR` to `vow-diag`. Audit and fix
existing `linear struct` uses where the new check rejects them.
Update the embedded skill document.

### Phase 7 — FFI wrapper stdlib

Implement `pin_to_root`, `String::from_raw_parts_copy`,
`Vec::from_raw_parts_copy` in the stdlib. Migrate all existing
`extern` usage to wrapper patterns. `vow-clif-shim` FFI migrates
to the wrapper idiom.

Bootstrap triple MUST still pass.

### Phase 8 — Cleanup

Delete `track_heap_alloc`, `mark_escaped`, the no-op
`__vow_string_free` / `__vow_vec_free` / `__vow_hashmap_free`.
Delete PR #181's conservative patch and the `// Tag but don't
track` comments at `vow-ir/src/lower/mod.rs:946` and
`compiler/lower.vow:1242`. Close issue #186.

Final binary fixed-point re-verification.

### Phase 9 — Performance pass

Profile the arena runtime on the benchmark suite. Address
regressions from Phase 4. Candidate optimizations:

- Aggressive elision of no-alloc block regions.
- Vec-builder zero-copy via `try_extend` fast path.
- Arena header register caching.
- Tune 4 KB chunk size if measurement warrants.

## 16. Open questions

These remain open at the spec level and are expected to be resolved
during implementation:

- **Chunk size tuning.** 4 KB is chosen as a reasonable default.
  Phase 9 may revise based on measurement.
- **Stack placement of small non-escaping aggregates.** A codegen
  optimization that places struct-of-scalars values on the machine
  stack rather than in an arena when escape analysis proves
  non-escape. Does not change the model; purely an implementation
  detail.
- **Incremental recompilation on summary change.** Today
  `build/vowc` rebuilds everything; when incremental builds matter,
  summary-diff propagation becomes load-bearing.
- **Root-region OOM policy.** `pin_to_root` of a very large value
  could OOM. Runtime policy: trap with a structured error.
  Documentation needed; behavior identical to existing OOM paths.
- **Verifier `--unwind` auto-tuning.** The default of 4 may need
  raising for deeply nested calls or accumulating loops. `VerifyLimits`
  exposes the knob; defaults may be revised after Phase 9.
