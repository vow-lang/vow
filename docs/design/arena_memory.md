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

### 2.1. Uniform placement rule

Every heap-typed value MUST be placed in exactly one of:

1. **An arena** (the runtime heap-allocation mechanism defined in
   §3) — the default for values produced at runtime.
2. **`.rodata`** — Class I compile-time literals (§6.1). These are
   heap-typed by the type system (e.g., a string literal has type
   `String`) but live in read-only static storage, not in any
   arena. They carry the read-only-backing sentinel defined in
   §6.1.
3. **The root region** — Class II/III program-lifetime values
   (§6.2). The root region is itself an arena (`__vow_root_arena`),
   but it is singular and never closes; references into it are
   valid for the entire process lifetime.

Arenas are the only runtime heap-allocation mechanism exposed by
the language. `.rodata` and the root region are described here only
to make the exhaustive placement rule explicit; neither adds a new
surface-level allocation primitive.

In particular:

- `String`, `Vec<T>`, `HashMap<K, V>` backings and descriptors are
  placed per the above rule: arena-allocated when constructed at
  runtime, `.rodata`-backed when produced from a compile-time
  literal (§6.1), root-region-allocated when pinned via
  `pin_to_root` (§7 / §8.4).
- Struct and enum values that contain heap-typed fields follow the
  same rule recursively; their heap-typed fields are pointers into
  some arena or into `.rodata`.
- Scalar values (`i32`, `i64`, `u8`, `u64`, `f32`, `f64`, `bool` —
  the current primitive set in `docs/spec/grammar.md`) are not
  heap-typed and are not arena-allocated. They live in registers
  or on the machine stack as today. Any future primitive scalar
  type additions are also not heap-typed by construction.

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
void     __vow_arena_open(struct VowArena* a);
void     __vow_arena_close(struct VowArena* a);
void*    __vow_arena_alloc(struct VowArena* a, uintptr_t bytes, uintptr_t align);
int64_t  __vow_arena_try_extend(struct VowArena* a, void* ptr,
                                uintptr_t old_size, uintptr_t new_size);
```

Return-type note: `__vow_arena_try_extend` returns `int64_t` (`1`
for success, `0` for failure), not `bool`. Rust's `bool` has a
1-byte C ABI; the self-hosted compiler's FFI shim (see
`vow-clif-shim`) uses 64-bit integer returns uniformly for boolean
values to avoid the ABI-mismatch class documented in `CLAUDE.md`
under "Self-Hosted Compiler Gotchas" (`__vow_string_eq` precedent).
Every boolean-valued runtime primitive in this spec MUST follow the
same convention. In the C header the type is written `int64_t`
(from `<stdint.h>`); in the Rust definition and in Vow FFI bindings
the corresponding `i64` ABI-compatible type is used.

**`__vow_arena_open(a)`**: initializes `*a` to an arena with one
freshly-allocated chunk of 4 KB plus the 8-byte next-chunk link
(4104 bytes total, per §3.2). If the underlying `malloc` fails, the
runtime traps with a structured OOM error (consistent with the
root-region OOM policy in §16); the trap is not recoverable from
within Vow.

**`__vow_arena_close(a)`**: walks the chunk chain, calls `free` on
each chunk, leaves `*a` in an undefined state. Callers MUST NOT
dereference `a` after close.

**`__vow_arena_alloc(a, bytes, align)`**: returns an aligned pointer
into `a`'s current chunk. If the current chunk does not have room,
allocates a new chunk (size per §3.2) and links it at the tail.
Updates `cursor` and `chunk_end` to the new chunk. Also records
`ptr` and `bytes` in `last_alloc_*`.

**`__vow_arena_try_extend(a, ptr, old_size, new_size)`**: returns
`1` (success) if and only if `ptr == a->last_alloc_start` AND
`a->last_alloc_size == old_size` AND the current chunk has
`(new_size - old_size)` bytes remaining after the existing
allocation. On success, bumps the cursor by the additional bytes
and returns `1`; the caller may treat the allocation as extended
in place without copying. On failure, returns `0`; the caller MUST
fall back to a fresh `__vow_arena_alloc` + `memcpy`. Callers test
the result with `!= 0`.

If `__vow_arena_alloc`'s fallback `malloc` for a new chunk fails,
the runtime traps with the same structured OOM error as
`__vow_arena_open` (§16).

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
   Because the virtual caller node is the unique root of the tree,
   this LUB is always well-defined: the innermost block that is an
   ancestor of every block in `must_outlive(I)`, falling back to the
   virtual caller node when no concrete common ancestor exists.
4. Validate `region(I)` against the **interprocedural store-effect
   constraints** collected at each call site that takes `I` as an
   argument. A store-effect constraint has the form
   `region(arg_source) ⊒ region(arg_target)` — the source value
   must outlive the container it is written into. If the caller's
   concrete region assignments make any such constraint unsatisfiable
   (for example, the callee requires storing `I` into parameter
   `p_target`'s region, but `region(I)` is a strictly shorter-lived
   descendant block of `region(p_target)` that closes before
   `p_target`'s region does), the program is rejected with
   `RegionConflict` (§13). This is the only path on which
   `RegionConflict` fires; step 3's LUB itself never fails.
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
  `region(j-th argument)` to `must_outlive(I)`, and additionally
  records the store-effect inequality checked in step 4 above.

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
monotone fixed-point iteration over the **constraint lattice** —
note the direction carefully, because a naive reading of §4.2
suggests the wrong direction.

The summary lattice, per field:

- `return_region`: partially ordered from most permissive
  (`ConstantGlobal`) to most restrictive (`FreshInCaller`). Summaries
  tighten downward (become more restrictive) as more escape sites are
  discovered.
- `store_effects`: ordered by set inclusion — the empty set is the
  most permissive (no caller obligations), and each added
  `StoreEffect` is strictly more restrictive. Summaries tighten
  upward (grow) as more escape-via-store sites are discovered.
- `param_regions`: placeholders; substitution happens at call sites
  and does not participate in the SCC lattice.

Iteration:

1. **Seed** every function in the SCC with the most-permissive
   summary: `return_region = ConstantGlobal` (no caller-side region
   work needed); `store_effects = {}` (no caller-side obligations);
   `param_regions` are free placeholders.
2. Re-analyze each function with the current summaries. Each
   re-analysis may tighten the function's own summary —
   `return_region` downward toward `FreshInCaller`, `store_effects`
   upward by adding newly discovered effects.
3. Update summaries.
4. Repeat until no summary changes.

Convergence is guaranteed because the lattice is finite and each
update is monotone in the tightening direction: `return_region`
never relaxes from `FreshInCaller` back to `ConstantGlobal`, and
`store_effects` never shrinks. The final summaries represent the
most permissive assumption consistent with all observed escapes —
exactly the information callers need to satisfy §4.1, step 4.

This direction is mandatory. Starting from the most-restrictive
end — e.g., empty `store_effects` combined with `return_region =
FreshInCaller` — is **not** valid: an empty effect set is already
the most-permissive state, and monotone convergence cannot add to
it. Any implementation that seeds store_effects as empty and claims
"summaries only become more permissive" will silently suppress real
`RegionConflict` diagnostics, because a fresh constraint discovered
during re-analysis cannot be incorporated without breaking
monotonicity.

### 4.4. Rejection, not over-approximation

When region inference encounters a placement conflict — an
interprocedural store-effect constraint (§4.1, step 4) that cannot
be satisfied by the caller's concrete region assignments — the
program MUST be rejected with `RegionConflict`. The compiler MUST
NOT silently promote the value to the root region or to any wider
region than `must_outlive(I)` demands.

Rationale: silent root-region placement causes memory growth for the
program lifetime with no visible signal to the agent. Structured
rejection is the feedback signal the CEGIS workflow depends on.

Note that the block-tree LUB in step 3 always succeeds (the virtual
caller node is the universal root); the rejection condition is
solely the interprocedural constraint check in step 4.

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
- Scalar-returning functions: no hidden parameter **unless** the
  summary has non-empty `store_effects` whose target region is not
  already reachable from a parameter (see below). **Exception:**
  `main` always receives `target_region` regardless of its return
  type; see §5.4.

**Store-effect-driven hidden region parameters.** A function's
hidden-region parameter set is a projection of the full summary,
not of the return type alone. For every distinct target region in
the summary — whether introduced by `return_region = FreshInCaller`
or by a `StoreEffect` that allocates into a region not already
carried by an explicit parameter — the ABI adds one hidden
`*VowArena` parameter. Common cases:

- Pure heap-returning function: one hidden `target_region`
  (FreshInCaller).
- Scalar-returning function that stores fresh allocations into a
  `Vec<T>` / `HashMap<K,V>` / struct passed as a parameter: zero
  hidden parameters — the target region is reachable through the
  container parameter's descriptor.
- Scalar-returning function that stores fresh allocations into a
  region that is neither its return target nor reachable through
  an existing parameter: one hidden region parameter per distinct
  such target. This case is unusual but MUST be emitted; otherwise
  the callee has no ABI mechanism to route the allocation and
  would silently alias into its own block region, breaking the
  store-effect contract.
- Multiple escaping heap outputs: one `target_region` per distinct
  target region. In practice this is almost always one.

The hidden-region parameter set for a function is therefore
determined by taking the union of `{FreshInCaller target}` (if
applicable) and `{target regions of store_effects whose region is
not reachable via any explicit parameter}`, then emitting one
`*VowArena` parameter per element.

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

`main` is a formal exception to the scalar-return ABI rule in §5.2:
it always receives `target_region = &__vow_root_arena` from the
runtime startup shim regardless of its declared return type. If
`main`'s return is `i64` (the typical case), the parameter is
unused by the body but remains present in the ABI so the startup
shim can invoke every well-formed `main` signature uniformly.

This is the only function whose ABI deviates from the table in
§5.2. Every other function's hidden-parameter presence is a direct
projection of its region summary.

## 6. Program-lifetime storage

### 6.1. Class I: compile-time literals

String literals, array literals, and any other constant known at
compile time MUST be placed in `.rodata`. The surrounding descriptor
(e.g., the 24-byte `{ptr, len, cap}` of `String`) is constructed
with the read-only-backing sentinel `cap = VOW_CAP_RODATA`.

`VOW_CAP_RODATA` is a reserved `usize` value that is distinguishable
from every legal runtime capacity. It MUST NOT collide with the
existing "empty, not yet allocated, growable" sentinel used by the
current runtime (`cap = 0`; see `__vow_vec_reserve` in
`vow-runtime`, which lazily allocates `VEC_INITIAL_CAP` when
`v.cap == 0`). The Phase 1 implementation MUST choose the sentinel
explicitly — the recommended value is `usize::MAX`, reserving it
from the legal capacity range; any alternative (e.g., an unused
high bit, or a distinct flag word added to the descriptor) is
acceptable provided the read-only and lazy-empty states remain
strictly disjoint at runtime. Phase 1 runtime changes MUST update
`__vow_vec_reserve`, `__vow_vec_push`, `__vow_string_push_str`,
`__vow_hashmap_insert`, and every other mutation entry point to
test for `VOW_CAP_RODATA` first and trap before any growth logic
is entered.

Any operation that would mutate the backing of a
`cap == VOW_CAP_RODATA` value MUST trap at runtime with
`RegionLiteralMutation` (§13). The compiler MUST NOT silently
promote literal-backed values to heap copies on mutation. Programs
that need a mutable copy use `String::from(literal)` or equivalent
explicit copy.

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

Containers whose descriptor carries `cap == VOW_CAP_RODATA` are
literal-backed (§6.1). Operations that would grow or mutate the
backing (`Vec::push`, `String::push_str`, `HashMap::insert`, etc.)
MUST trap with `RegionLiteralMutation` before any allocation path
runs. The trap check is required ahead of the existing
`cap == 0` → lazy-allocate path so the two sentinels never alias
in practice. Agents wanting a mutable copy must explicitly copy via
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
`RegionLinear` (§13).

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

New diagnostic error codes introduced by the arena model. Names
follow the existing `vow-diag::ErrorCode` PascalCase convention
(e.g., `LinearTypeViolation`, `VowRequiresViolated`,
`EsbmcNotFound`); they are added as new variants to that enum.

Compile-time region diagnostics (`RegionConflict`, `RegionLinear`,
`RegionAbiMismatch`) MUST be emitted through the same `vow-diag` →
CLI adapter path that every existing compile-time diagnostic uses.
The external CLI schema is the one authoritatively documented in
`docs/spec/cli.md` (see `CompileFailed` example): each entry in the
build output's `diagnostics[]` array has the shape

```json
{
  "error_code": "<ErrorCode variant>",
  "message": "...",
  "severity": "error",
  "span":      { "file": "...", "offset": N, "length": M }
}
```

The examples below render what agents and tooling will see on the
CLI — identical key names, identical value casing, identical span
shape — for forward compatibility with every consumer of
`vow build` / `vow verify` JSON.

Structured auxiliary info (region hints, multiple conflicting
sources, parameter names) is surfaced through the existing
secondary-span / hint mechanism as and when `cli.md` documents
extensions to the diagnostic shape; this design explicitly does not
propose a new external wire format. Until then, auxiliary context
belongs in the rendered `message` string.

The runtime error (`RegionLiteralMutation`) follows the existing
runtime-error shape used by `VowViolation`, `ArithmeticOverflow`,
and `IndexOutOfBounds` (see `docs/spec/errors.md`): a compact
`{"error": "<Name>", ...}` object emitted from `vow-runtime`
directly to stderr, not routed through `vow-diag`.

### 13.1. `RegionConflict`

Emitted when region inference cannot satisfy an interprocedural
store-effect constraint (§4.1, step 4): the caller's concrete
region assignments would require storing a value into a region that
outlives `region(I)`. `span` names the escaping value.

```json
{
  "error_code": "RegionConflict",
  "message": "value `v` is placed in region(b) which closes before region(a), the container it is stored into; move the allocation to a wider scope",
  "severity": "error",
  "span": { "file": "f.vow", "offset": 1024, "length": 3 }
}
```

### 13.2. `RegionLinear`

Emitted when a linear value is unconsumed at the close of its
region (see §9). `span` names the unconsumed-value binding; the
`message` identifies the allocation site and the region close that
triggered rejection in rendered form.

```json
{
  "error_code": "RegionLinear",
  "message": "linear value `f` (allocated at g.vow:120) not consumed before region close at g.vow:340",
  "severity": "error",
  "span": { "file": "g.vow", "offset": 200, "length": 1 }
}
```

### 13.3. `RegionLiteralMutation`

Runtime trap emitted when a mutation operation is attempted on a
container whose descriptor carries the `VOW_CAP_RODATA` sentinel
(§6.1). Emitted by `vow-runtime` directly to stderr, not routed
through `vow-diag`. JSON shape matches the existing runtime-error
family (see `docs/spec/errors.md`):

```json
{
  "error": "RegionLiteralMutation",
  "operation": "String::push_str",
  "origin": "rodata"
}
```

The handler prints `hint: use String::from(literal) to obtain a
mutable copy` to stderr alongside the JSON, consistent with how
`VowViolation` surfaces its `description`.

### 13.4. `RegionAbiMismatch`

Emitted at link/load time when a callee's region summary has
changed and callers have stale metadata. Indicates a build
inconsistency; not a program-level error. `span` names the caller's
call site; the summary hashes are rendered into `message` rather
than surfaced as separate typed fields, matching the convention
used by other build-inconsistency diagnostics today.

```json
{
  "error_code": "RegionAbiMismatch",
  "message": "region summary for `module::function` changed since caller was compiled (expected sha256:ab…, actual sha256:cd…); rebuild caller module",
  "severity": "error",
  "span": { "file": "caller.vow", "offset": 0, "length": 0 }
}
```

### 13.5. Error stability

Error codes MUST be stable across compiler versions. New codes may
be added; existing codes MUST NOT change meaning. These four codes
will be added to `docs/spec/errors.md` as part of the phase that
introduces them in `vow-diag::ErrorCode` (Phase 3 for
`RegionConflict`, Phase 6 for `RegionLinear`, Phase 4 for
`RegionLiteralMutation` and `RegionAbiMismatch`); no entries are
added here because the codes do not yet exist in the emitter.

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

**Symbol-collision note.** The existing `vow-runtime` already
exports a function named `__vow_arena_alloc` with a different
signature: `pub extern "C" fn __vow_arena_alloc(size: usize, align:
usize) -> *mut u8` (see `vow-runtime/src/lib.rs`, declared as an
import in `vow-codegen/src/cranelift_backend.rs` and
`vow-clif-shim/src/lib.rs`). That older function is a global-allocator
shim and is unrelated to the arena-per-scope model. The two symbols
cannot coexist. Phase 1 MUST atomically rename the older shim to
`__vow_malloc` across `vow-runtime`, `vow-codegen`, and
`vow-clif-shim`, freeing the `__vow_arena_alloc` symbol for the new
signature in this document. The rename is a single mechanical PR
step bundled with the Phase 1 runtime additions — not a separate
cleanup — so that "runtime ships unused by the compiler" is true of
the *new* symbol at the moment Phase 1 lands.

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
`RegionConflict` emission. Runs after type/effect/linear checks;
populates IR region fields; lowerer still ignores them.

**3b (self-hosted):** `compiler/region.vow` (new). Port of 3a.

**Gate:** the two region passes MUST produce identical summaries on
the self-hosted compiler's own source. Zero diff required.

**Side CI job:** run the region pass on `compiler/*.vow` and emit
any `RegionConflict` as non-blocking warnings. This surfaces
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
file. Collect `RegionConflict` emissions.

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
compilers. Add `RegionLinear` to `vow-diag`. Audit and fix
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
track` comments in `vow-ir/src/lower/mod.rs` and
`compiler/lower.vow` (grep for `Tag but don't track` — the
comment text is stable, file line numbers will drift). Close
issue #186.

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
