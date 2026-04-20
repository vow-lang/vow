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
one pointer (8 bytes) for the next-chunk link, stored at the **start**
of each chunk; usable allocation space begins at byte offset 8 within
the chunk, giving 4096 usable bytes per normal chunk. Allocations
larger than 2048 bytes (half-chunk threshold) are placed in a
custom-sized chunk whose total size is
`8 + bytes + (align - 1)`, where `align` is the requested alignment
of the oversized allocation. The `align - 1` slack covers worst-case
alignment padding after the 8-byte link and guarantees the
allocation fits regardless of alignment; without it, high-alignment
requests (e.g., 16-byte-aligned SIMD backings) would exceed the
chunk's usable range and trigger repeated fallback allocation or
out-of-bounds arithmetic.

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
allocation. On success, bumps the cursor by the additional bytes,
**updates `a->last_alloc_size` to `new_size`**, and returns `1`;
the caller may treat the allocation as extended in place without
copying. The `last_alloc_size` update is required so that a
subsequent consecutive `try_extend` on the same allocation (e.g.,
two back-to-back `Vec::push` calls hitting the fast path) sees
the post-extend size in its `old_size` comparison rather than the
stale pre-extend size. On failure, returns `0` and leaves the
arena header unchanged; the caller MUST fall back to a fresh
`__vow_arena_alloc` + `memcpy`. Callers test the result with
`!= 0`.

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
operations. A block's region is considered **non-empty** if any of
the following holds:

1. The block directly contains a heap-producing instruction `I`
   with `region(I) == Block(B)` for this block.
2. The block contains a call to a function with a non-empty
   `store_effects` entry whose target's substituted region is
   this block.
3. The block contains a call to a function whose `return_region ==
   FreshInCaller` where the caller's hidden-region argument
   routes to this block — i.e., the callee allocates into this
   block's arena via `target_region`.
4. The block contains a call whose return is stored or otherwise
   pinned such that `must_outlive` places the returned value in
   this block (the return aliases an argument that lives in this
   block, or a later use transitively requires it).

Only blocks that meet none of the above are elided. The region
pass emits `__vow_arena_open` / `__vow_arena_close` for every
non-empty block region. Criterion (3) is the specifically
hazardous case: omitting it would leave a block whose only
allocations are performed by a callee through a hidden
`target_region` routed to an unopened header, which is undefined
behavior in the runtime model (§3.3).

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
   `must_outlive(I)` — the set of lifetime markers the value must
   remain live across for every use to be legal. Each element is
   one of:
   - a concrete block in the same function (the value must
     outlive that block's close),
   - the virtual caller node (the value escapes to the caller),
   - `Root` (the value is pinned into the root region via
     `pin_to_root`, or is stored into a `Root`-regioned value;
     its lifetime requirement is the entire process), or
   - `Rodata` (the value originates from a compile-time literal;
     treated as strictly longer-lived than every runtime region).

   The `Root` and `Rodata` markers participate in LUB (step 3)
   per the following coercions:
   - `LUB({Rodata})` = `Rodata` (the value already lives
     statically — no region placement needed).
   - `Rodata ⊔ concrete-block` = `Rodata` (rodata strictly
     outlives every block).
   - `Root ⊔ concrete-block` = `Root` (root strictly outlives
     every block).
   - `Rodata ⊔ Root` = `Root` (a mixed-path value that is
     sometimes a `.rodata` literal and sometimes
     root-pinned is placed in the root region; root covers
     the runtime-writability dimension that `.rodata` cannot).
   - `Rodata ⊔ virtual-caller` = `Rodata` (a function that
     returns a `.rodata` literal has a `ConstantGlobal` return).
   - `Root ⊔ virtual-caller` = `Root` (a function that returns
     a root-pinned value — e.g., `return pin_to_root(s)` — also
     has a `ConstantGlobal` return; root strictly outlives any
     caller lifetime, so the caller does not need to supply a
     region, and the runtime pointer is valid for the entire
     process).
   - `anything ⊔ FreshInCaller` = `FreshInCaller` is a
     `return_region` rule (§4.3), not a `must_outlive` coercion;
     it applies only in the summary lattice.

   Step 5 is extended to recognise these non-block `region(I)`
   outcomes: if `region(I)` is `Root` or `Rodata`, the
   allocation is lowered with `RegionId::Root` / `RegionId::Rodata`
   respectively (per §12.1), and the function's summary records
   `return_region = ConstantGlobal` for values that reach the
   return expression along such a path. The virtual-caller
   branch in step 5 still fires for values whose `region(I)` is
   the virtual caller node itself, not for values coerced into
   `Root`/`Rodata`.

   This keeps stores into `pin_to_root`-ed targets or into
   program-lifetime containers from being misclassified as
   caller escapes (which would spuriously add a hidden-arg ABI
   parameter), and handles `return pin_to_root(s)` correctly as
   a `ConstantGlobal` return rather than a spec violation.
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
5. Dispatch on `region(I)`:
   - **Virtual caller node.** The function's summary (§4.2)
     records the allocation as escaping to the caller
     (`return_region` widens toward `FreshInCaller`; store
     effects widen per the target parameter's region).
   - **`Root`.** The allocation is lowered with
     `RegionId::Root` and routed to `__vow_root_arena`. The
     function's summary records `return_region = ConstantGlobal`
     for values reaching the return along this path (root
     lifetime is program-lifetime, equivalent from the caller's
     perspective to static storage).
   - **`Rodata`.** The allocation slot is lowered with
     `RegionId::Rodata`; no runtime allocation is emitted
     because the backing already exists in `.rodata`. The
     function's summary records `return_region = ConstantGlobal`
     for values reaching the return along this path.
   - **Concrete block `B`.** The allocation is lowered with
     `RegionId::Block(B)` and placed in that block's arena
     (§5.3).

`must_outlive(I)` is computed by following use-def chains:

- A use as the return expression adds the virtual caller node.
- A use as the source of `obj.field = I` or `vec.push(I)` or
  `map.insert(k, I)` adds the region of `obj` (respectively `vec`,
  `map`).
- A use as an argument to a call must account for both the
  callee's store effects **and** the callee's `return_region`:
  - **Store effects.** If the callee's store effects say
    parameter `p` is written into parameter `j`'s region, and
    `I` is the argument at position `p`, the caller adds
    `region(j-th argument)` to `must_outlive(I)` and records
    the store-effect inequality checked in step 4 above.
  - **Return aliasing.** If the callee's `return_region` is
    `AliasOf(j)` or `AliasOfAny(S)` and `I` is the argument at
    one of the aliased parameter positions (`j` or any element
    of `S`), the call's return value may carry `I` into a wider
    region in the caller's use-def graph. The caller
    propagates every `must_outlive` member of the return value
    back to `I`: for each block `B` in `must_outlive(return
    value)`, add `B` to `must_outlive(I)`. This prevents a
    dangling reference after `I`'s original block closes in
    patterns like `let t = id(s); use(t)` where `id : AliasOf(0)`
    outlives `s`'s defining block.
  - `ConstantGlobal` / `FreshInCaller` returns do not alias any
    argument and contribute no extra constraints on `I` beyond
    those already captured by store effects.

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

- `return_region`: the four `RegionConstraint` variants form a
  join-semilattice that is **not** a total chain — `AliasOf(i)`
  and `ConstantGlobal` are incomparable because `AliasOf(i)`
  claims the return always aliases parameter `i`'s region, while
  `ConstantGlobal` claims it always points into static storage;
  neither implies the other. The Hasse diagram (bottom is
  uninitialised):

  ```
                          FreshInCaller
                        /               \
                AliasOfAny(S)            |
                        |                |
                    AliasOf(i)    ConstantGlobal
                         \               /
                           ⊥ (seed)
  ```

  The diamond structure makes the incomparability explicit:
  `AliasOf(i)` / `AliasOfAny(S)` (left branch) and
  `ConstantGlobal` (right branch) have no comparable
  relationship with each other; their only common upper bound
  is `FreshInCaller`.

  Joins:

  - `AliasOf(i)` ⊔ `AliasOf(j)` = `AliasOfAny([i, j])` when
    `i ≠ j`; `AliasOf(i)` otherwise.
  - `AliasOf(i)` ⊔ `AliasOfAny(S)` = `AliasOfAny(S ∪ {i})`.
  - `AliasOfAny(S)` ⊔ `AliasOfAny(T)` = `AliasOfAny(S ∪ T)`.
  - `AliasOf(i)` ⊔ `ConstantGlobal` = `FreshInCaller` (they sit
    on parallel branches; their least upper bound is the top of
    the lattice).
  - `AliasOfAny(S)` ⊔ `ConstantGlobal` = `FreshInCaller` (same
    reason).
  - Anything ⊔ `FreshInCaller` = `FreshInCaller`.

  `ConstantGlobal` is permissive (no caller-side region or hidden
  parameter). `AliasOf(i)` and `AliasOfAny(S)` require the caller
  to understand the aliasing constraint but not to pass a hidden
  arena. `FreshInCaller` is the most restrictive state (caller
  must supply a hidden `*VowArena`). Summaries tighten upward
  along this order as more escape sites are discovered; the
  lattice has finite height, so iteration terminates.

  The semantic rationale for the parallel-branch topology: a
  function whose body contains one path returning a `.rodata`
  literal and another returning a parameter alias does **not**
  satisfy either `ConstantGlobal` or `AliasOf(i)` individually
  (the first would mis-type the parameter-aliasing path; the
  second would mis-type the literal path). The only sound
  summary is `FreshInCaller`, obtained by joining the two. An
  alternative total-chain design where `ConstantGlobal ⊑
  AliasOf(i)` was considered and rejected: it would make the
  join `AliasOf(i)` for such functions, over-constraining the
  caller to provide a parameter at position `i` whose region
  the return is falsely claimed to alias.
- `store_effects`: ordered by set inclusion — the empty set is the
  most permissive (no caller obligations), and each added
  `StoreEffect` is strictly more restrictive. Summaries tighten
  upward (grow) as more escape-via-store sites are discovered.
- `param_regions`: placeholders; substitution happens at call sites
  and does not participate in the SCC lattice.

Iteration:

1. **Seed** every function in the SCC with the bottom-of-lattice
   unknown state. Because `return_region`'s published variants
   (`ConstantGlobal`, `AliasOf(i)`, `AliasOfAny(S)`,
   `FreshInCaller`) form a semilattice with incomparable branches
   (§4.2), no published variant is a sound seed — seeding at any
   of them forces the first join to that variant or above, which
   would over-approximate functions whose true summary sits on a
   parallel branch. The region pass therefore uses an internal
   `Uninit` variant as the bottom element during SCC iteration:

   ```
   InternalRegionConstraint = Uninit | <any published variant>
   ```

   `Uninit` satisfies `Uninit ⊔ x = x` for every published
   variant `x`. It MUST NOT be written to module metadata and
   MUST NOT appear in any stored summary after iteration
   terminates; every function's final `return_region` is a
   published variant.

   Seed every function in the SCC with
   `return_region = Uninit`; `store_effects = {}` (the already-
   bottom state for set inclusion); `param_regions` are free
   placeholders.
2. Re-analyze each function with the current summaries. Each
   re-analysis may tighten the function's own summary —
   `return_region` moves upward from `Uninit` toward
   `FreshInCaller` via the join rules in §4.2; `store_effects`
   grows by adding newly discovered effects. Use `⊔` to combine
   the current summary with every newly discovered escape's
   contribution.
3. Update summaries.
4. Repeat until no summary changes.
5. At termination, resolve any remaining `Uninit` by examining
   the function's return *expression* structure (not just its
   heap-producing instructions):
   - If the return expression always evaluates to a parameter
     value (or field of a parameter) at index `i`, the final
     summary is `AliasOf(i)` — or `AliasOfAny(S)` if multiple
     parameter sources are possible along different paths. This
     covers pass-through functions like
     `fn id(s: String) -> String { s }`, which has no
     heap-producing instruction in the function body but is
     semantically an alias-of-parameter return and MUST NOT be
     summarized as `ConstantGlobal`.
   - If the return expression is always a `.rodata` literal or a
     `ConstantGlobal` call, the final summary is
     `ConstantGlobal`.
   - If the return is of a scalar type (never a heap-typed
     value), the final summary is `ConstantGlobal` — scalar
     returns carry no region obligation.
   - Otherwise (the function has no statically-determinable
     return value pattern — dead code, an unreachable-by-design
     stub, or a truly empty heap-return function), the final
     summary is `ConstantGlobal` as the benign default.

   The key property is that `Uninit` does **not** silently
   default to `ConstantGlobal` for every heap-typed return path;
   alias-only pass-through functions would otherwise be
   misclassified, causing callers to treat parameter-aliased
   returns as program-lifetime values and skip required region
   constraints.

Convergence is guaranteed because the lattice is finite and each
update is monotone in the tightening direction: `return_region`
never relaxes from `FreshInCaller` back toward `Uninit`, and
`store_effects` never shrinks. The final summaries represent the
most permissive assumption consistent with all observed escapes —
exactly the information callers need to satisfy §4.1, step 4.

This direction is mandatory. Starting `return_region` from any
non-bottom point is **not** valid:

- Seeding at `FreshInCaller` (most restrictive) permanently
  over-approximates every SCC member as requiring a caller-
  provided region, even when the true summary is
  `ConstantGlobal` or `AliasOf(i)`. Every such function would
  be emitted with an incorrect hidden `*VowArena` parameter,
  producing ABI drift and suppressing `RegionConflict`
  diagnostics at call sites that rely on the true summary.
- Seeding at `ConstantGlobal` (a non-bottom element) does **not**
  fix this: because `ConstantGlobal` and `AliasOf(i)` are
  incomparable (§4.2), a recursive function whose true summary
  is `AliasOf(i)` causes the first join to compute
  `ConstantGlobal ⊔ AliasOf(i) = FreshInCaller`, and since
  summaries only tighten, `AliasOf(i)` becomes unreachable and
  the function gets a spurious hidden arena parameter.
- Seeding at `AliasOf(i)` / `AliasOfAny(S)` fails symmetrically
  against a true `ConstantGlobal` summary.

Only `Uninit` is a sound seed; every other starting point is
biased toward one branch of the semilattice and cannot reach the
other.

`store_effects` has the opposite direction but the same hazard on
the wrong end: it only grows and never shrinks, so it must be
seeded at the empty set. A non-empty seed would require a shrink
step to reach the true minimum, which is not monotone.

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
- Scalar-returning functions: no hidden parameter from the return.
  Store effects may still require hidden parameters — see below.
  **Exception:** `main` always receives `target_region` regardless
  of its return type; see §5.4.

**Store-effect-driven hidden region parameters.** A function's
hidden-region parameter set is a projection of the full summary,
not of the return type alone. Each `StoreEffect` in §4.2 has the
form `{ target: param_index, source: RegionConstraint }` — so
every store target is, by construction, a parameter of the
callee. For each distinct store target `param_index` appearing in
`store_effects`, the ABI adds one hidden `*VowArena` parameter
that carries the arena header for that parameter's region. The
parameter itself (e.g., a `Vec<T>` descriptor) does not embed the
arena header; per §3.1, block-region headers live in the caller's
stack frame, so the caller must pass the address of the header
the callee will allocate into.

The hidden-region parameter set for a function is therefore:

```
hidden_regions(f) = ({ target_region } if return_region == FreshInCaller else ∅)
                    ∪ { store_target(e) : e ∈ f.store_effects }
```

with duplicates removed. One `*VowArena` parameter is appended to
the ABI per element, in a stable order (`target_region` first if
present, then store-target hidden regions in ascending `param_index`
order). Common cases:

- Pure heap-returning function: one hidden `target_region`.
- Scalar-returning function with no escaping store effects: zero
  hidden parameters.
- Scalar-returning function that stores fresh allocations into a
  `Vec<T>` / `HashMap<K,V>` parameter: one hidden parameter
  carrying that parameter's arena header.
- Multiple distinct store targets: one hidden parameter per
  distinct target. In practice this is almost always zero or one.

### 5.3. Within-function allocation

Allocations whose `region(I)` is a concrete block `B` inside the
callee lower to:

```
__vow_arena_alloc(&B_arena, bytes, align)
```

where `B_arena` is the stack-allocated VowArena header for block
`B`.

Allocations whose `region(I)` is `Caller(k)` lower to:

```
__vow_arena_alloc(hidden_region[k], bytes, align)
```

where `hidden_region[k]` names the k-th hidden region parameter
appended by the ABI (§5.2). Functions with a single hidden region
always use `Caller(0)`, which names `target_region` for a
`FreshInCaller` return or the single store-target hidden region
for a pure scalar mutator.

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
The wrapper form depends on whether the returned payload is a
**flat** byte buffer or a **pointer-containing** structure:

**Flat payloads (POD / byte buffers).** For values whose memory
contains no further heap pointers — `u8`-backed strings / byte
arrays, scalar-only structs, fixed-width numeric arrays — the
wrapper:

1. Calls the extern.
2. Copies the bytes into the wrapper's `target_region` via
   `__vow_arena_alloc` + `memcpy`.
3. Calls the corresponding C-side `free` on the extern's pointer.
4. Returns the Vow-placed value.

**Pointer-containing payloads (structs / arrays with nested
heap-typed fields).** The flat memcpy+free pattern is **not sound**
for these: a byte copy preserves stale pointers into the C-side
backing, and the subsequent `free` of the outer pointer leaves
those inner pointers dangling. For such payloads the wrapper MUST
perform a **type-directed deep copy**:

1. Call the extern.
2. Walk the returned value's type recursively, allocating a
   fresh copy in `target_region` for each pointer-containing
   sub-object (using `__vow_arena_alloc` + `memcpy` for each
   flat leaf segment along the way, the same way standard
   container constructors do).
3. Call the corresponding C-side `free` for every pointer in the
   original graph — the outer pointer and every nested pointer
   that C owns — following whatever ownership discipline the
   extern documents. The wrapper MUST NOT assume a single
   top-level `free` suffices.
4. Return the Vow-placed value.

The `String::from_raw_parts_copy` / `Vec::from_raw_parts_copy`
helpers in §8.4 cover the flat-payload case. Deep-copy wrappers
for pointer-containing payloads are per-type and MUST be written
by hand (or generated alongside the extern's type definition);
this spec does not propose a generic deep-copy intrinsic.

The wrapper's region summary emerges from its body: a wrapper that
allocates into `target_region` has `FreshInCaller`; a wrapper that
returns a `.rodata` pointer has `ConstantGlobal`.

### 8.4. Stdlib helpers

The stdlib MUST provide:

- `pin_to_root(value) -> value` for every heap-typed value type —
  allocates in the root region and moves the value there.
  Idempotent on already-root values. Because Vow does not have
  user-facing generics (see `docs/spec/grammar.md` — only the
  built-in container types `Vec<T>` / `Option<T>` are
  parameterized), `pin_to_root` is a **compiler intrinsic**, not
  a generic function the programmer writes. The type checker
  monomorphises each call site by the argument's concrete type
  (same mechanism used for `Vec::new()` today).
- `String::from_raw_parts_copy(ptr: *const u8, len: i64) -> String`
  — copies bytes from a raw C pointer into `target_region` as a
  `String`. `FreshInCaller`. `len` is declared as `i64` to match
  the existing length-bearing APIs in `docs/spec/grammar.md`
  (Vow has no `usize` surface type today); at the FFI boundary
  the value is converted to `uintptr_t` before calling C, the
  same conversion every existing length-aware extern uses.
- `Vec::from_raw_parts_copy(ptr, len: i64) -> Vec<T>` for each
  supported element type `T` — analogous for `Vec`. Also a
  compiler intrinsic, monomorphised per call site. Same
  `i64 ↔ uintptr_t` ABI conversion at the FFI boundary.

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
the maximum chain length the analysis admits. For **Phase 1
standalone arena verification** (§10.4), an ESBMC `--unwind` depth
of 4 is sufficient for most block-local arenas; this is an ESBMC
command-line parameter scoped to the arena primitive's own
verification, separate from `VerifyLimits.max_k_step` (which
governs incremental BMC step count for general program verification
and whose default is set elsewhere). Accumulating loops in
user-program regions may require higher `--unwind`, exposed via
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
- `__vow_arena_try_extend` modifies only `cursor` and (on success)
  `last_alloc_size`; it never copies data and never changes
  `last_alloc_start`. On success, `a->last_alloc_size == new_size`
  (post-condition required by §3.3 so that consecutive extends on
  the same allocation see the up-to-date size in subsequent
  `old_size` comparisons).

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
    | Block(BlockId)           // named block within the current function
    | Root                     // root region
    | Rodata                   // .rodata static storage
    | Caller(HiddenRegionIdx)  // one of the function's caller-provided
                               // hidden `*VowArena` parameters (§5.2)
```

**Naming-collision note.** `vow-ir` already defines a
`pub struct RegionId(pub u32)` newtype at `vow-ir/src/types.rs:15`
used by the pre-arena effects-tracking machinery (`InstData` and
`AbstractHeap::Region`). The two definitions cannot coexist under
the same name. Phase 2 (§15) MUST atomically rename the existing
newtype to `AbstractRegionId` across `vow-ir` and every
downstream consumer, freeing the `RegionId` name for the enum in
this document. The rename is a single mechanical step bundled
with the Phase 2 IR extension, equivalent in spirit to Phase 1's
`__vow_arena_alloc → __vow_malloc` rename.

`HiddenRegionIdx` is a zero-based index into the ordered list of
hidden region parameters that the ABI appends to the function's
signature. Index `0` always refers to `target_region` when the
function's `return_region == FreshInCaller`; subsequent indices
enumerate the store-target hidden parameters in the stable order
defined in §5.2. Lowering uses `HiddenRegionIdx` to select the
correct `*VowArena` parameter in the emitted
`__vow_arena_alloc(...)` call; functions with a single hidden
region always use `Caller(0)`.

Every heap-producing `Inst` carries a `region: RegionId` field.

### 12.2. No new opcodes for allocation placement

The existing per-object allocation opcode is `RegionAlloc` (see
`vow-ir/src/types.rs`, `Opcode::RegionAlloc`), with a companion
`RegionFree`. No per-type allocation opcode layer (`StringNew`,
`VecNew`, etc.) exists or is introduced by this document.

Phase 2 (§15) extends `RegionAlloc` with a `region: RegionId`
field; the region pass attaches that field and lowering consumes
it to select the correct `*VowArena` header in the emitted
`__vow_arena_alloc(...)` call. `RegionFree` becomes a no-op in
Phase 4 (its body is gutted once lowering routes all reclamation
through arena close) and is deleted entirely in Phase 8.

Note the naming neighborhood: `RegionAlloc` / `RegionFree` are
**per-object** IR opcodes, distinct from `RegionOpen(BlockId)` /
`RegionClose(BlockId)` (§12.3), which are **per-arena boundary**
opcodes. Phase 2 may rename `RegionAlloc` to, e.g., `HeapAlloc` if
this neighborhood becomes confusing — the choice is a phase-2
implementation detail and not load-bearing on the spec.

### 12.3. Block region opcodes

Two new IR opcodes mark region boundaries:

- `RegionOpen(BlockId)` — emitted at block entry when the block
  region is non-empty under the four-criterion rule in §3.5.
  For a loop body, `RegionOpen` is emitted at the start of every
  iteration (immediately after the loop header's condition is
  checked and control enters the body).
- `RegionClose(BlockId)` — emitted at **every** exit from the
  block, enumerated exhaustively: normal fall-through to the
  block's end, `break`, `continue`, `return`, and any other
  control-flow edge that leaves the block (e.g., early exits
  from `match` arms, `?`/`try` short-circuits once those land).

  `continue` MUST close every open block region from the
  innermost enclosing block **through and including the target
  loop's body**, in innermost-first order. A fresh
  `RegionOpen(loop_body)` then fires at the start of the next
  iteration. Stopping short of the loop body — leaving its
  region open across iterations — would both (a) turn
  per-iteration reclamation into function-lifetime retention in
  long-running loops and (b) risk reopening an already-open
  header on the next iteration's `RegionOpen`, which is
  undefined in the runtime model. The loop body's region thus
  has the same open/close cadence as one iteration of its body.

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

The runtime error (`RegionLiteralMutation`) is emitted from
`vow-runtime` directly to stderr, not routed through `vow-diag`.
It uses the same outer envelope as the existing runtime-error
family in `docs/spec/errors.md` — a compact
`{"error": "<Name>", ...}` object — but individual runtime errors
in that family vary in which auxiliary fields they carry:

- Bare envelope (no auxiliary fields beyond `error`):
  `ArithmeticOverflow`, `UnwrapOnNone`, `IndexOutOfBounds`.
- Extended envelope: `VowViolation` carries `vow_id`, `blame`,
  `description`, `file`, `offset`, `values`.

`RegionLiteralMutation` falls into the *extended* group: it
adds `operation` and `origin` to the envelope (see §13.3).
"Follows the runtime-error family" here means the outer envelope
shape, not a strict match with any single existing code.

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
through `vow-diag`. JSON shape follows the outer runtime-error
envelope (see `docs/spec/errors.md`: `ArithmeticOverflow`,
`UnwrapOnNone`, and `IndexOutOfBounds` use bare `{"error": "..."}`
objects; `VowViolation` uses the same envelope with auxiliary
fields `vow_id`/`blame`/`description`/`file`/`offset`/`values`)
and extends it with operation-specific fields:

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
inconsistency; not a program-level error. `span` names the
caller's call site on a best-effort basis: linkers often do not
have precise source positions, so an all-zero span
(`{file: "...", offset: 0, length: 0}`) is a legitimate fallback
indicating "no source position available". The summary hashes are
rendered into `message` rather than surfaced as separate typed
fields, matching the convention used by other build-inconsistency
diagnostics today.

Example with best-effort (no source position):

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
exports two companion functions in the global-allocator shim
that share the `__vow_arena_*` prefix with this spec's new
symbols:

- `pub extern "C" fn __vow_arena_alloc(size: usize, align: usize)
  -> *mut u8` (grep `pub extern "C" fn __vow_arena_alloc` in
  `vow-runtime/src/lib.rs` — line numbers drift with the file;
  at the time of writing it is near line 532, but do not rely on
  the number).
- `pub unsafe extern "C" fn __vow_arena_free(ptr: *mut u8, size:
  usize, align: usize)` (grep
  `pub unsafe extern "C" fn __vow_arena_free` in the same file).

Both are declared as imports in
`vow-codegen/src/cranelift_backend.rs` and
`vow-clif-shim/src/lib.rs`. Neither is related to the
arena-per-scope model; they are generic `malloc`/`free`-style
entry points whose names happen to collide. The two `_alloc`
symbols cannot coexist, and leaving `__vow_arena_free` unrenamed
would produce a confusingly-named companion alongside the new
`__vow_arena_open` / `__vow_arena_close` / `__vow_arena_alloc`
primitives in this document. Phase 1 MUST atomically rename both
shims:

- `__vow_arena_alloc` → `__vow_malloc`
- `__vow_arena_free`  → `__vow_free`

across every site that references either name, in a single PR
step bundled with the Phase 1 runtime additions:

- `vow-runtime/src/lib.rs` — both `pub extern "C" fn`
  definitions.
- `vow-runtime/src/lib.rs` — every unit test that calls either
  function directly. At the time of writing:
  - `arena_alloc_free_roundtrip` — references both old symbols.
  - `arena_alloc_zero_returns_sentinel` — references
    `__vow_arena_alloc` only.
  - `arena_free_null_is_noop` — references `__vow_arena_free`
    only.
  - `arena_free_zero_size_is_noop` — references
    `__vow_arena_free` only.

  Future test additions referring to either old name must follow
  the rename in the same commit.
- `vow-codegen/src/cranelift_backend.rs` — the `declare_function`
  import sites for both symbols.
- `vow-clif-shim/src/lib.rs` — the `declare_function` import
  sites for both symbols.

Every occurrence is mechanical. Bundling the rename with Phase 1
keeps CI green across the rename (tests that referenced the old
names are renamed in the same commit) and leaves the
`__vow_arena_*` symbol space entirely free for the spec's new
primitives at the moment Phase 1 lands.

Standalone C-level tests + ESBMC verification on the primitive in
isolation (§10.4). Runtime ships unused by the compiler.

### Phase 2 — IR extension (both compilers, atomic)

**Symbol-collision note.** `vow-ir/src/types.rs:15` already
defines `pub struct RegionId(pub u32)` as a newtype used by the
pre-arena effects-tracking machinery (`InstData` and
`AbstractHeap::Region`). Phase 2 MUST atomically rename that
existing newtype to `AbstractRegionId` across `vow-ir` and every
downstream consumer, freeing the `RegionId` name for the new
enum defined in §12.1. Bundle this rename with the Phase 2 IR
additions so CI stays green through the change (equivalent in
spirit to Phase 1's `__vow_arena_alloc → __vow_malloc` rename).

**2a (Rust):** after the rename above, `vow-ir` gains the new
`RegionId` enum (§12.1), a `region: RegionId` field on the
existing `RegionAlloc` opcode (§12.2), and a `RegionSummary`
slot in function metadata. Module-format version bump. Default
every `RegionAlloc` to `RegionId::Root` (conservative
placeholder; Phase 3 replaces this with inferred values).

**2b (self-hosted):** `compiler/ir.vow` gets matching
`RegionId` representation. `compiler/ast.vow` gains
`RegionSummary` slot on the function record.
`compiler/ir_printer.vow` prints region info.

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

**Gate:** `cargo test --all` passes. The project's integration
test suite passes end-to-end — the concrete runner at Phase 4
landing time is whichever `vow test` driver is in production
(Rust-bootstrap `target/release/vow test` today; a self-hosted
equivalent added to `build/vowc` once `cli.md` documents it for
the self-hosted binary). The point of this gate is correctness
parity against pre-Phase-4 behavior, independent of which binary
hosts the runner. No correctness regression. Performance
regression allowed; logged for Phase 9.

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

Delete:

- `track_heap_alloc`, `mark_escaped` helpers.
- The no-op `__vow_string_free` / `__vow_vec_free` /
  `__vow_hashmap_free` free-helper functions (they became no-ops
  in Phase 4).
- The `RegionFree` IR opcode itself: remove it from
  `Opcode` in `vow-ir/src/types.rs`, from the self-hosted
  `compiler/ir.vow` opcode enumeration, and from any codegen
  match arms that dispatch on it. After Phase 4 every
  `RegionFree` instance in lowered IR is a no-op, so this
  removal is mechanical — no semantic change from pre-Phase-8
  behavior.
- PR #181's conservative patch. The patch leaves two identifier
  signatures behind in the tree, one per compiler side:

Both compilers carry a matching `// Tag but don't track: can't
distinguish owned heap from arena alias without ownership
annotations.` comment at the conservative-patch site:

- **Rust:** `vow-ir/src/lower/mod.rs` (grep for
  `Tag but don't track`; comment text is stable, line numbers
  drift).
- **Self-hosted:** `compiler/lower.vow` (same grep anchor).

Plus `lctx_mark_escaped` call sites in `compiler/lower.vow`
that the patch introduces — grep for `lctx_mark_escaped` to
surface them all for deletion.

Phase 8 MUST delete both the comments and the call sites
atomically. Close issue #186.

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
- **Root-region OOM — external error envelope.** The OOM policy
  itself is **settled**: any arena operation whose underlying
  `malloc` fails (initial chunk in `__vow_arena_open`, fallback
  chunk in `__vow_arena_alloc`, `pin_to_root` into the root
  region) traps with a structured runtime error. The trap is not
  recoverable from within Vow. The open piece that remains is
  the **exact JSON wire shape** of the OOM error envelope — it
  is not yet documented in `docs/spec/errors.md` and is expected
  to be added alongside Phase 1 when the primitive ships. §3.3
  refers to this policy as "a structured OOM error" on that
  basis; the guarantee is the trap-and-exit semantics, not any
  particular field layout.
- **Verifier `--unwind` auto-tuning.** The default of 4 may need
  raising for deeply nested calls or accumulating loops. `VerifyLimits`
  exposes the knob; defaults may be revised after Phase 9.
