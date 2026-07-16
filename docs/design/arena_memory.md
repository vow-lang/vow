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
allocation. Heap-producing instructions include `RegionAlloc` and
runtime allocation externs that create fresh heap descriptors, such as
canonical `__vow_vec_new` / `__vow_vec_new_val` calls emitted for
`Vec::new`.

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

Each arena is represented by a 56-byte header:

```c
struct VowArena {
    void*     first_chunk;      // head of chunk chain
    void*     current_chunk;    // active chunk (tail)
    uintptr_t cursor;           // next allocation address within current chunk
    uintptr_t chunk_end;        // one past last usable byte in current chunk
    void*     last_alloc_start; // most recent allocation, for try_extend
    uintptr_t last_alloc_size;  // size of most recent allocation
    uintptr_t retained_bytes;   // total bytes retained by chunk chain
};
```

Arena headers MUST be stack-allocated wherever possible. A block
region's header lives in the stack frame of the enclosing function.
The root region (§6) is the exception: its header lives in `.bss`.

### 3.2. Chunk layout

Chunks are allocated via libc `malloc`. Each chunk carries a 16-byte
header at offset 0: an 8-byte next-chunk link followed by an 8-byte
total chunk size. Usable allocation space begins at byte offset 16
within the chunk, giving 4096 usable bytes per normal chunk (total
size 4112). Allocations larger than 2048 bytes (half-chunk threshold)
are placed in a custom-sized chunk whose total size is
`16 + bytes + (align - 1)`, where `align` is the requested alignment
of the oversized allocation. The `align - 1` slack covers worst-case
alignment padding after the 16-byte header and guarantees the
allocation fits regardless of alignment; without it, high-alignment
requests (e.g., 16-byte-aligned SIMD backings) would exceed the
chunk's usable range and trigger repeated fallback allocation or
out-of-bounds arithmetic.

The total-size word carries an additional bit (CHUNK_OVERSIZED_FLAG)
that records *which allocation path* produced the chunk — not just
its size. The chain walker in §7.1 consults this flag, not a
size-derived predicate, because a path-oversized chunk can have a
`total` below, equal to, or above `normal_chunk_total()`: for
example, a 3000-byte single-resident string backing has total 3016
< 4112, yet is still single-resident and reclaimable.

**Single-resident invariant for oversized chunks.** After an oversized
allocation, the arena's `cursor` is parked at `chunk_end` of the new
chunk. This intentionally wastes the chunk's alignment-padding tail
(up to `align - 1` bytes for the alignment-driven path) so no
subsequent allocation can land in the slack via the fast path. The
invariant — *each oversized chunk holds exactly one allocation* — is
the precondition that lets §7.1 free the entire chunk when its
backing is abandoned without dangling pointers into a still-live
allocation in the same chunk. Normal chunks continue to use the
bump cursor and may hold many allocations.

Chunks form a singly-linked list rooted at `first_chunk`. The
`current_chunk` always points to the tail. The next-chunk pointer of
the tail is `NULL`.

### 3.3. Runtime API

The following C-callable primitives MUST be provided by `vow-runtime`:

```c
void     __vow_arena_open(struct VowArena* a);
void     __vow_arena_close(struct VowArena* a);
void     __vow_arena_init_closed(struct VowArena* a);
void*    __vow_arena_alloc(struct VowArena* a, uintptr_t bytes, uintptr_t align);
int64_t  __vow_arena_try_extend(struct VowArena* a, void* ptr,
                                uintptr_t old_size, uintptr_t new_size);
uint64_t __vow_memory_root_arena_bytes(void);
uint64_t __vow_memory_peak_bytes(void);
uint64_t __vow_memory_alloc_count_since_start(void);
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

**`__vow_arena_init_closed(a)`**: initializes `*a` to the closed
all-zero state. The compiler emits this once when a block-arena stack
slot is first materialized, before the first open or close can observe
the header.

**`__vow_arena_open(a)`**: initializes `*a` to an arena with one
freshly-allocated chunk of 4 KB plus the 8-byte next-chunk link
(4104 bytes total, per §3.2). If `a` already names an open arena,
`__vow_arena_open(a)` is a no-op; this protects structured entries
that reach an already-open region root. If the underlying `malloc`
fails, the runtime traps with a structured OOM error (consistent with
the root-region OOM policy in §16); the trap is not recoverable from
within Vow.

**`__vow_arena_close(a)`**: walks the chunk chain, calls `free` on
each chunk, and zeros `*a`. The zeroed state makes double-close a safe
no-op and updates memory-query counters consistently.

**`__vow_arena_alloc(a, bytes, align)`**: returns an aligned pointer
into `a`'s current chunk. If the current chunk does not have room,
allocates a new chunk (size per §3.2) and links it at the tail.
Updates `cursor` and `chunk_end` to the new chunk. Also records
`ptr` and `bytes` in `last_alloc_*`, increments the successful arena
allocation request counter, and increases `retained_bytes` when a new
chunk is linked.

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

The `__vow_memory_*` functions expose low-overhead runtime query
builtins. They return root-region retained chunk bytes, peak live
retained chunk bytes across all open arenas, and successful arena
allocation request count since process start. `__vow_arena_try_extend`
does not increment the allocation request count because it reuses the
current chunk without issuing a fresh arena allocation.

#### Vec runtime allocation API

The existing root-region Vec symbols remain ABI-stable:

```c
void* __vow_vec_new(uintptr_t elem_size, uintptr_t align);
void* __vow_vec_new_val(void);
void  __vow_vec_push(void* vec, const void* value_ptr,
                     uintptr_t elem_size, uintptr_t elem_align);
void  __vow_vec_push_val(void* vec, int64_t value);
```

These are root wrappers: they open `__vow_root_arena` if needed, then
delegate to the corresponding explicit-arena primitive with
`&__vow_root_arena`.

The explicit-arena forms are:

```c
void* __vow_vec_new_in_arena(struct VowArena* arena,
                             uintptr_t elem_size, uintptr_t align);
void* __vow_vec_new_val_in_arena(struct VowArena* arena);
void  __vow_vec_push_in_arena(struct VowArena* arena, void* vec,
                              const void* value_ptr,
                              uintptr_t elem_size, uintptr_t elem_align);
void  __vow_vec_push_val_in_arena(struct VowArena* arena, void* vec,
                                  int64_t value);
void  __vow_vec_reserve_in_arena(struct VowArena* arena, void* vec,
                                 uintptr_t additional,
                                 uintptr_t elem_size,
                                 uintptr_t elem_align);
```

Every explicit-arena Vec entry traps with
`RuntimeInvariantViolation` and `reason = "null arena"` before
dereferencing a null arena pointer. Growth for both root and
explicit-arena Vecs uses the shared arena grow path: first
`__vow_arena_try_extend`, then `__vow_arena_alloc` + copy + zero-fill
on fallback. `__vow_vec_from_raw_parts_copy_val(arena, ptr, len)`
already has the explicit-arena shape and copies the raw slots into the
supplied arena.

#### String runtime allocation API

The existing root-region String symbols remain ABI-stable:

```c
void* __vow_string_new(const char* ptr, uintptr_t len);
void* __vow_string_from_cstr(const char* ptr);
void* __vow_string_clone(const void* source);
void  __vow_string_push_str(void* dest, const void* src);
void  __vow_string_push_byte(void* string, int64_t byte);
void* __vow_string_substr(const void* string, int64_t start, int64_t len);
void* __vow_string_substring(const void* string, int64_t start, int64_t end);
void* __vow_string_from_i64(int64_t value);
void* __vow_string_split(const void* haystack, const void* separator);
void* __vow_string_trim(const void* string);
void* __vow_string_to_upper(const void* string);
void* __vow_string_to_lower(const void* string);
void* __vow_string_replace(const void* string, const void* from, const void* to);
void* __vow_string_join(const void* vec, const void* separator);
```

These are root wrappers: they open `__vow_root_arena` if needed, then
delegate to the corresponding explicit-arena primitive with
`&__vow_root_arena`.
`__vow_string_clone` deep-copies a `String` descriptor and backing bytes into
the root arena; `String::from(s)` lowers to this wrapper. Direct string
literals do not call a runtime wrapper: codegen points at a static
`{ptr,len,cap=VOW_CAP_RODATA}` descriptor in `.rodata`.

The explicit-arena forms are:

```c
void* __vow_string_new_in_arena(struct VowArena* arena,
                                const char* ptr, uintptr_t len);
void* __vow_string_from_cstr_in_arena(struct VowArena* arena,
                                      const char* ptr);
void* __vow_string_clone_in_arena(struct VowArena* arena,
                                  const void* source);
void  __vow_string_push_str_in_arena(struct VowArena* arena,
                                     void* dest, const void* src);
void  __vow_string_push_byte_in_arena(struct VowArena* arena,
                                      void* string, int64_t byte);
void* __vow_string_substr_in_arena(struct VowArena* arena,
                                   const void* string,
                                   int64_t start, int64_t len);
void* __vow_string_substring_in_arena(struct VowArena* arena,
                                      const void* string,
                                      int64_t start, int64_t end);
void* __vow_string_from_i64_in_arena(struct VowArena* arena,
                                     int64_t value);
void* __vow_string_split_in_arena(struct VowArena* arena,
                                  const void* haystack,
                                  const void* separator);
void* __vow_string_trim_in_arena(struct VowArena* arena,
                                 const void* string);
void* __vow_string_to_upper_in_arena(struct VowArena* arena,
                                     const void* string);
void* __vow_string_to_lower_in_arena(struct VowArena* arena,
                                     const void* string);
void* __vow_string_replace_in_arena(struct VowArena* arena,
                                    const void* string,
                                    const void* from, const void* to);
void* __vow_string_join_in_arena(struct VowArena* arena,
                                 const void* vec,
                                 const void* separator);
```

Every explicit-arena String entry traps with
`RuntimeInvariantViolation` and `reason = "null arena"` before
dereferencing a null arena pointer. String literals and C string
payloads remain in `.rodata`; the arena owns the allocated String
descriptor/header and any later growth allocation. Growth for both root
and explicit-arena Strings uses the same arena grow path as Vec: try to
extend in place, then allocate in the selected arena and copy on
fallback.

#### HashMap runtime allocation API

The existing root-region `HashMap` symbols remain ABI-stable:

```c
void*    __vow_map_new(void);
void     __vow_map_insert(void* map, int64_t key, int64_t val);
int64_t  __vow_map_get(const void* map, int64_t key);
_Bool    __vow_map_contains(const void* map, int64_t key);
void     __vow_map_remove(void* map, int64_t key);
uintptr_t __vow_map_len(const void* map);
```

`__vow_map_contains` predates the `__vow_arena_*` / `__vow_string_eq`
boolean-ABI convention (§3.3, intro): the live runtime returns Rust
`bool` and both Cranelift signature tables model it as `I8`, so it is
declared here as `_Bool` rather than `int64_t`. C/FFI callers must read
exactly the 1-byte boolean from the return register; the upper bits are
undefined. This legacy ABI is preserved for compatibility with the
pre-arena `__vow_map_*` symbols and is unrelated to the arena routing
introduced by the `_in_arena` forms below.

`__vow_map_new` and `__vow_map_insert` are root wrappers: they acquire
the root-arena lock, ensure the root arena is open, then delegate to the
corresponding explicit-arena primitive with `&__vow_root_arena`.
`__vow_map_remove` is **not** a root wrapper — it performs an in-place
linear-scan removal that never touches the arena, so the relationship
inverts: `__vow_map_remove_in_arena` traps on a null arena and then
delegates to `__vow_map_remove` directly. The non-allocating accessors
(`__vow_map_get`, `__vow_map_contains`, `__vow_map_len`) read the map
in place and never touch any arena.

The explicit-arena forms are:

```c
void* __vow_map_new_in_arena(struct VowArena* arena);
void  __vow_map_insert_in_arena(struct VowArena* arena, void* map,
                                int64_t key, int64_t val);
void  __vow_map_remove_in_arena(struct VowArena* arena, void* map,
                                int64_t key);
```

Every explicit-arena HashMap entry traps with
`RuntimeInvariantViolation` and `reason = "null arena"` before
dereferencing a null arena pointer. The bucket array and the map
header are both allocated in the supplied arena: a fresh `HashMap`
allocates a 24-byte header plus an initial 8-entry × 16-byte backing.
Growth on `insert` uses the shared arena grow path — first
`__vow_arena_try_extend` against the current backing, then
`__vow_arena_alloc` + memcpy on fallback — and the new bucket array
lives in the same arena as the header. `__vow_map_remove_in_arena` is
exposed for ABI symmetry with the other in-arena forms; the operation
itself never allocates and the arena pointer is consumed only for the
null-arena trap, then ignored.

The current MVP runtime uses i64 keys and i64 values with an O(n)
linear-scan backing (matching the existing root-region implementation).
Heap-typed keys or values are not yet supported; the surface grammar
permits them syntactically (`HashMap<K, V>`), but only `i64` × `i64`
is wired through the runtime.

### 3.4. Determinism

The arena **allocation strategy**, the **chunk-size policy**, and
the **code emitted** for every primitive MUST be deterministic.
Chunk size is fixed at 4 KB (with the oversized-allocation
exception). No random padding. No in-chunk layout that varies
across runs. Binary fixed point is preserved only if the
compiler's choice of regions, chunk sizes, and generated
instruction sequences is deterministic.

Raw runtime addresses returned by `malloc` are inherently
allocator- and OS-dependent, and the model does **not** require
them to be stable across executions. Programs MUST NOT observe
addresses in a way that leaks into compiler output or contract
verification; address values are not semantically observable by
construction (Vow has no pointer-to-integer casts in source and
no contract vocabulary for pointer equality beyond structural
identity). Determinism is therefore a property of what the
compiler emits, not of what the runtime's pointer values happen
to be.

### 3.5. Empty-region elision

A block region with no heap allocations MUST NOT emit any arena
operations. A block's region is considered **non-empty** iff at
least one runtime allocation actually targets it. The three
sources of such allocations are:

1. The block directly contains a heap-producing instruction `I`
   with `region(I) == Block(B)` for this block.
2. The block contains a call to a function whose `store_effects`
   entry writes into a container whose substituted region is
   this block — the callee allocates fresh elements into the
   block's arena through that store.
3. The block contains a call to a function whose `return_region
   == FreshInCaller` and whose hidden `target_region` argument
   (§5.2) routes to this block — the callee allocates the return
   value directly into the block's arena.

Only blocks that meet none of the above are elided. Criterion
(3) is the specifically hazardous case: omitting it would leave
a block whose only allocations are performed by a callee through
a hidden `target_region` routed to an unopened header, which is
undefined behavior in §3.3's runtime model.

Pure alias returns (`return_region ∈ {ConstantGlobal, AliasOf,
AliasOfAny}`) do **not** allocate into the caller's block, so
they never pin a block as non-empty on their own — they only
contribute to region inference through the `must_outlive`
propagation in §4.1.

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
                          Uninit (⊥ seed)
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

### 4.4. Rejection vs. visibility

Region inference distinguishes two concerns:

**Rejection** (`RegionConflict`, severity Error). When the
interprocedural store-effect constraint (§4.1, step 4) cannot be
satisfied by the caller's **inferred** region assignments — that is,
when a value's `region(I) = LUB(must_outlive(I))` resolves to a
concrete block strictly narrower than the target container's region
— the program MUST be rejected with `RegionConflict`. The compiler
MUST NOT silently promote the value to a wider region than the
inference's must-outlive markers already demand.

Operationally, the conflict check consults the inferred `region(I)`
populated by step 3's LUB pass, NOT the IR opcode shape of the
producing instruction. A fresh `RegionAlloc` whose must-outlive set
includes a `CallerStoreTarget(p)` marker (added by §4.1 step 2's
call-site propagation when the value is passed to a callee whose
store-effect target traces back to the current function's parameter
`p`) has `region(I) = Caller(HiddenRegionIdx(N))` after LUB, where
`N` is the slot index that codegen's hidden-arena layout
(§5.4) assigns to parameter `p`. Such a value satisfies any
parameter-region target whose slot matches. Rejecting such a value
would force programmers to refactor sound arena patterns into
less-direct forms, with no soundness benefit.

**Slot-aware inference (issue #317).** The LUB pass mints
`Caller(HiddenRegionIdx(N))` per allocation, where `N` is computed
from the function's published summary using the same formula codegen
applies in `hidden_region_idx_for_store_target`:
  * slot 0 = return arena, iff `summary.return_region == FreshInCaller`;
  * subsequent slots = sorted, deduplicated store-effect target
    parameters in ascending order.

Allocations with markers spanning a single destination resolve to
that destination's slot precisely; codegen routes the alloc into the
correct hidden arena. Allocations whose marker set spans more than one
hidden caller-arena slot (for example, the same allocation is both
returned through `FreshInCaller` and stored into a parameter target, or
stored into two distinct parameter targets) have no single caller arena
that outlives every destination, so their LUB widens to the root region
(`Root`). This is a strictly wider placement than any one escaped pointer
requires — hence sound (leak-but-safe) — and the program compiles without
a `RegionConflict` (issue #871). Widening is the conservative resolution:
choosing an arbitrary slot would route at least one escaped pointer into
the wrong arena lifetime, and rejecting would refuse valid code. The
leak-vs-signal trade is surfaced instead by `RegionRootEscape` (below)
when the widened region bottoms out at the root region.

**Visibility** (`RegionRootEscape`, severity Note, non-blocking).
When `region(I)` resolves through caller-region routing to a chain
that bottoms out at the root region (§5.4: `main`'s `target_region
= &__vow_root_arena`), the compiler MUST emit a `RegionRootEscape`
note. Root never shrinks (§6.2); program-lifetime placement is a
memory-cost decision the agent should be aware of. The note is
informational — it does not fail the build.

Conservative approximation is acceptable for the note: rather than
performing full call-graph reachability from `main`, an
implementation MAY emit the note for any heap-producing instruction
whose inferred region is `Caller` in a function that publishes
`FreshInCaller` or any store effect (i.e., the alloc CAN escape via
the caller chain to root). False positives are tolerated because
the diagnostic is non-blocking and informational.

The note MUST also fire for an allocation whose region **widens to
root** without an intrinsic root pin — the leak-vs-signal case from
the Resolution paragraph above. Two shapes reach it: a marker set
spanning more than one hidden caller slot (the multi-slot widen),
and a container reached through a Phi over caller containers (the
Phi widen). The two are distinguished from a genuine `pin_to_root`
placement by their marker: a widen carries no intrinsic root marker
(the multi-slot case has only per-slot caller markers, and the Phi
case carries a dedicated widened-caller-root marker), whereas a pin
carries the intrinsic root marker. Only the pin is exempt from the
note; both widens are flagged. This distinction is what closes the
gap issue #366 identified — the earlier gate suppressed the note for
functions with more than one hidden slot, and for Phi-reached
containers the widen was indistinguishable from a pin.

Unlike the caller-routing case, a widen-to-root allocation is
flagged **even when it is also returned**. The return exemption
below rests on the caller owning the value's lifetime; a widen-to-
root placement has already committed the value to the never-freed
root arena, so returning it does not free it and does not suppress
the note.

The note SHOULD NOT fire for the canonical `FreshInCaller`
return-value pattern (`fn make_X() -> X`), where the alloc is the
return value — that's the documented mechanism for producing values
in a caller's arena and carries no hidden surprise. The exemption
extends transitively to allocations installed as fields of the
returned struct via field initializers (`Item { name:
String::from("hi") }` from `make_item() -> Item`): those field
allocations share the parent struct's caller-arena lifetime, and
the parent's note (when applicable) already conveys the full
escape information — surfacing the children adds noise without
information. The exemption is strictly structural: it follows the
return value through Phi arms (via Upsilon) and through the
**currently-installed** FieldSet edges — that is, the textually-last
FieldSet to each `(target, field_idx)` pair within each basic
block. The exemption is gated on the FieldSet target pointer being
itself a fresh `RegionAlloc` (not a `GetArg` parameter alias or
other non-fresh value). The structural test is **`GetArg` vs
`RegionAlloc` as the FieldSet target**, not `Store` vs `FieldSet`:
parameter mutation (`target.name = ...; return target`) and callee
store-effect routing both correctly fall outside the exemption.
Per-block last-write dedup is what restores precision against the
construct-then-overwrite pattern (`x.f = A; x.f = B; return x`):
allocation `A` is dead by the end of the block — no longer
reachable from the returned struct — so it is excluded from the
skip-set and remains flaggable (issue #326). The dedup is
conservative across blocks: a FieldSet in one block whose
`(target, field_idx)` is overwritten by a FieldSet in another
block is still treated as live, biasing toward false positives in
keeping with the Note's non-blocking informational character.
`Store` is also out of scope for the exemption: it represents
arbitrary post-allocation mutation through a pointer with unknown
aliasing, semantically distinct from constructor-time field
initialization.

Rationale: structured rejection on genuine constraint failures is
the CEGIS feedback signal; structured visibility on root routing
prevents silent program-lifetime growth without conflating it with
unsoundness. Issue #314 motivated splitting these concerns: a
shape-based rejection check (rejecting any `RegionAlloc` source
flowing to a parameter container) was too coarse, blocking
legitimate arena patterns. The semantic check restores the
spec-mandated rejection rule while the note preserves the spirit
of the original silent-root concern.

Note that the block-tree LUB in step 3 always succeeds (the virtual
caller node is the universal root); the rejection condition is
solely the interprocedural constraint check in step 4 against the
LUB-computed region.

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

**Return materialization.** `FreshInCaller` is a **representation
promise**, not only an ABI promise: the callee guarantees that
every heap-typed return value it produces is **already located in
the arena pointed to by `target_region`** at the moment of
return. If the body of the function would otherwise return a
value whose current storage is not `target_region` — for
example, a `.rodata` literal on one path, a parameter alias on
another, or a value allocated in some inner block that closes
before return — the compiler MUST insert a copy of that value
into `target_region` before the return edge. Concretely:

- A `.rodata` literal returned on a `FreshInCaller` path is
  materialized with an equivalent explicit copy into `target_region`.
- A parameter alias returned on a `FreshInCaller` path is
  lowered as a deep copy (§8.3 deep-copy discipline) into
  `target_region`.
- A value allocated in a strictly-inner block and then returned
  is lowered so its final allocation is in `target_region` from
  the start — the region pass places such values directly, not
  in the inner block.

This rule is what makes the §4.3 lattice join `AliasOf(i) ⊔
ConstantGlobal = FreshInCaller` sound: when a function is
summarised as `FreshInCaller`, every caller assumes the returned
pointer refers to storage in the arena they supplied, and the
function body's lowering honours that assumption regardless of
which source path the value came from.

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

Receiver-growth effects use the same target projection even when
no heap source is stored. For example, `String::push_byte`,
`String::push_str`, and raw `Vec::push` on a parameter receiver
publish a `StoreEffect` with `source = ConstantGlobal`; the source
constraint is inert, but the target membership makes the receiver's
arena available to growth code inside the callee.

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

**Rodata store targets are statically rejected.** A `store_effect`
target is always a parameter (§4.2), but that parameter may at
call-time resolve to a `.rodata`-backed value (e.g., `f(literal)`
where `f`'s summary has `store_effects` on its first parameter).
There is no arena header to pass for `.rodata`: literals live in
read-only static storage, not in any arena (§2.1, §6.1). This
spec resolves the case statically: the region pass MUST reject
any call site whose substituted store-effect target region is
`Rodata`, with `RegionLiteralMutation` (§13). The rejection
fires at compile time, **before** ABI materialisation — the
runtime ABI therefore never has to consider passing a null or
sentinel `*VowArena` for a rodata target. Programmers who want
to mutate a literal-backed value must produce a mutable copy
via `String::from(literal)` / `Vec::from(literal)` / equivalent
first, exactly the same path §7.3 prescribes for direct
mutation.

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

### 6.3. Root-region lifetime guarantees

Routing a value into the root region is a one-way operation:
every value visible to Vow source code that lives in root remains
live for the entire process lifetime. The root region's chunk
chain is never freed by an `__vow_arena_close` call.

There is one exception that is invisible to Vow source code:
container growth in any arena (§7.1) reclaims the dedicated
oversized chunk of an abandoned backing, including in the root
region. This shrinks the root arena's total resident bytes but
does not affect any live Vow value — the abandoned backing is
unreachable from Vow by construction (the descriptor's `ptr`
points at the new buffer).

C code that retains a raw pointer into a root-region container's
backing across calls MUST not let the container grow between
those calls, or MUST re-fetch the pointer after any operation
that may grow it. §7.4 documents the FFI shape.

Accidental root-region placement is prevented by §4.4: region
inference rejects rather than over-approximates to root. Explicit
root placement (`pin_to_root`) is a visible source operation.

## 7. Container growth

### 7.1. Growth strategy

`Vec<T>`, `HashMap<K, V>`, and `String` grow by allocating a new
larger backing in the same arena as the current backing and copying.

The old backing is reclaimed in two ways:

1. **Oversized abandoned backings are returned to libc immediately.**
   The old backing is always the sole resident of its oversized
   chunk (>2048 bytes): §3.2 seals the cursor at `chunk_end` of every
   oversized chunk at allocation time, so the fast path cannot place
   a subsequent allocation in the alignment-slack tail. The runtime
   walks the chunk chain after the growth's `memcpy`, finds the chunk
   containing the abandoned backing, confirms `total >
   normal_chunk_total()`, unlinks it, and calls `free` on it. This
   bounds the steady-state footprint of a grow-then-shrink loop to
   ~1× the current live capacity for the oversized portion of the
   backing.
2. **Normal-chunk abandoned backings are retained until arena close.**
   Normal chunks are shared with other allocations and cannot be
   reclaimed mid-arena; the abandoned bytes remain orphaned in the
   chunk's interior until the arena's chunk chain is freed.

For the typical pattern — a container that has doubled through `N`
growths past the oversized threshold — peak resident bytes are
bounded by ~2× current live capacity at the moment of the copy
(the old backing plus the new one) and settle back to ~1× after
each growth's chunk free. This restores the classical `realloc`
asymptotic footprint for large containers while preserving the
arena's bump-allocation cost model for small ones.

The chunk-free path is not a user-visible free operation: the
`Vec`/`String`/`HashMap` value itself is unchanged, and no Vow
source-level free/drop is introduced. §2.2's prohibition on
explicit free continues to apply.

FFI holders of the old backing pointer would still observe a
freed pointer after the chunk is returned to libc; §7.4 documents
the FFI shape that prevents this — wrappers that hand a pointer
to C must keep the underlying value alive across the call, and
Vow's region inference prevents a container from being grown
while a foreign pointer to its old backing is on a live FFI
boundary.

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
the container grows, the C side's view of the old backing depends on
where the backing lived:

- **Normal-chunk backings** remain valid until the arena closes. The
  containing chunk is shared with other allocations and cannot be
  released mid-arena, so the old bytes stay readable.
- **Oversized-chunk backings** are returned to libc as soon as the
  growth's `memcpy` completes (§7.1). A C-side pointer captured
  before the growth is dangling from that moment forward.

The default Vow → C convention is call-scoped sharing (§8.2): the C
side reads/writes the pointer for the duration of the call and does
not retain. Under that convention chunk release is harmless — no
growth happens during the call. C code that retains the pointer
across calls MUST treat container growth on the Vow side as
invalidating the pointer and re-fetch from the value's descriptor
after any operation that may grow the backing; pinning the value
into the root region stabilizes the descriptor's *identity* but
does not pin the underlying buffer address across growth (§6.3).

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
  places a copy of the value in the root region and returns a
  descriptor pointing into root storage. Because Vow does not
  have user-facing generics (see `docs/spec/grammar.md` — only
  the built-in container types `Vec<T>` / `Option<T>` are
  parameterized), `pin_to_root` is a **compiler intrinsic**,
  not a generic function the programmer writes. The type
  checker monomorphises each call site by the argument's
  concrete type (same mechanism used for `Vec::new()` today).

  **Always deep-copy.** `pin_to_root` MUST always perform a
  **type-directed deep copy** into the root region, following
  the same discipline as §8.3's pointer-containing FFI path.
  The intrinsic does not attempt to detect whether the input
  already lives in root and short-circuit: the `{ptr, len, cap}`
  container descriptor has no free bit for a region tag (`cap`
  is reserved by `VOW_CAP_RODATA` for literal-backed values
  (§6.1), and root-allocated containers carry real mutable
  capacities), and scanning `__vow_root_arena`'s chunk chain
  for pointer containment would add a per-call cost the
  intrinsic cannot amortise.

  Double-pinning is therefore the programmer's concern:
  `pin_to_root(pin_to_root(x))` copies twice and produces two
  distinct root-region values. The second copy is wasted but
  not incorrect. The compiler MAY elide the outer call at
  source-level constant-folding time when the argument
  expression is itself a `pin_to_root` call on the same value;
  the spec does not require this optimisation. Programmers
  wanting to avoid redundant copies should structure code so
  `pin_to_root` is called once at the region-boundary site.

  A shallow descriptor copy is only sound for flat (POD /
  byte-buffer) values; for anything richer, shallow-copying
  would leave inner pointers referring to the source region,
  and those inner regions may close long before root —
  producing a silent dangling reference the type checker cannot
  see. The intrinsic's monomorphisation therefore synthesises a
  per-type deep-copy walk at each call site.
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
`v`, that the consumption obligation is discharged before the
region containing `v` closes. The exact obligation depends on
the kind of region assigned to `v` by §4.1:

- `region(v) = Block(B)`: `consumed_at(v) <= region_close(B)`.
  The consumption site MUST lie at or before the block's close.
  Violation is rejected with `RegionLinear` (§13).
- `region(v) = Caller(k)`: the obligation transfers outward to
  the caller; the callee's lowering is not required to consume
  `v` before return. See §9.3.
- `region(v) = Root`: the root region never closes, so the
  deadline is program exit. A program MAY choose to leave a
  root-region linear value unconsumed — there is no finite
  close to race against — but doing so silently retains the
  value for the process lifetime. Because that is rarely
  intended, the linear-region check emits a
  **warning-not-error**: `RegionLinear` with a `hint` pointing
  at `pin_to_root` as the likely cause. Programs that
  legitimately want program-lifetime retention silence the
  warning by consuming the value at process shutdown (no
  semantic effect; purely a linter signal).
- `region(v) = Rodata`: a linear value MUST NOT be placed in
  `Rodata`. `.rodata` is read-only storage; a linear obligation
  is inherently mutable state (the consume operation mutates).
  Any inference that would assign `Rodata` to a linear value is
  a compiler bug and the region pass MUST reject the program
  with `RegionLinear` explaining that linear values cannot have
  Rodata region.

### 9.2. Region close does not consume

Arena close reclaims memory. It MUST NOT be treated as a
consumption site for linear values. A linear value whose block
region is about to close and which has not been explicitly
consumed is a compile error, not an implicit drop. (This rule
applies to block regions only; `Root` closes at program exit
and behaves per §9.1's warning discipline, and `Rodata` cannot
host linear values.)

### 9.3. Escape transfers the obligation

A linear value returned from a function (or otherwise escaping
its allocating region via a `FreshInCaller` return or a
`store_effect` into caller-owned storage) has its linear
obligation transferred to the caller. The callee's region close
is satisfied by the escape; the caller inherits the
consume-exactly-once requirement. For `Caller(k)`-regioned
linear values specifically, the obligation is always satisfied
by the return edge (or an earlier store-into-caller-container)
— §9.1's `Caller(k)` rule is the compile-time expression of
this transfer.

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

The project uses ESBMC in **incremental BMC mode** (`--incremental-bmc
--max-k-step N`), matching `vow-verify/src/esbmc.rs`. In this mode
`--unwind` is unused; iteration bounds are controlled exclusively by
`--max-k-step`. The Phase 1 standalone arena verification harness
(§10.4) uses the same convention: `--incremental-bmc --max-k-step 10`
is sufficient for block-local arenas under the harness's bounded
symbolic inputs. This is an ESBMC command-line parameter used only
by the Phase-1 arena-primitive verification harness; it is **not**
exposed on the `vowc build` / `vowc verify` CLI surface.

`VerifyLimits` is the public-facing verification configuration
type defined at `vow-verify/src/c_emitter.rs:55`. It carries
four fields: `max_k_step` (incremental-BMC iteration cap),
`vec_max`, `string_max`, `hashmap_max` (container size caps for
the verification model). `max_k_step` governs incremental BMC step
count across the whole program; the Phase-1 harness uses a locally
scoped value that does not interact with `VerifyLimits`.

If a future user-program region requires a higher `max_k_step`
than the harness default, raising the harness default is a local
change to `vow-runtime/verify/Makefile`. Raising the public-facing
`VerifyLimits.max_k_step` default would require a coordinated update
to `docs/spec/cli.md` as well.

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
`__vow_arena_alloc(...)` call or to select the explicit-arena
runtime symbol for a routed container allocation; functions with a
single hidden region always use `Caller(0)`.

Every heap-producing `Inst` carries a `region: RegionId` field. This
includes canonical Vec creation extern calls (`__vow_vec_new`,
`__vow_vec_new_val`) even though they remain `Opcode::Call` in IR.

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

Vec and String creation are intentionally not new opcodes. The lowerer
may keep emitting canonical root-wrapper extern symbols
(`__vow_vec_new`, `__vow_vec_new_val`, `__vow_string_new`,
`__vow_string_from_cstr`, and related String copy constructors). After
`infer_regions`, codegen reads the call's `region` field: `Root` keeps
the ABI-stable wrapper symbol, while `Block(_)` and `Caller(_)` prepend
the selected `*VowArena` and call the matching `_in_arena` symbol. Vec
and String growth calls follow the same rule using the receiver's
defining region: root-owned receivers keep the wrapper symbol, and
block/caller-owned receivers route to the matching `_in_arena` symbol.

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

  At the IR level, every loop back-edge `P -> H` refreshes each
  non-empty block region `Block(B)` on that loop-body path by
  emitting `RegionClose(B)` before `P`'s terminator. The matching
  `RegionOpen(B)` is the normal entry marker at `B`; it fires only
  if the next trip actually re-enters the region, so a header exit
  immediately after a back-edge cannot leave a freshly reopened body
  arena live. `B` is selected only when it is reachable from `H`
  through non-back-edge forward edges and can reach `P`; multiple
  refreshed regions on the same predecessor are ordered by `BlockId`.
  Empty loop-body regions remain elided by §3.5.

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

After the JSON object, the handler prints a separate
human-readable hint line to stderr:
`hint: use String::from(literal) to obtain a mutable copy`.
This is a distinct plain-text line, **not** a JSON field —
different surfacing mechanism from `VowViolation`, whose
equivalent guidance lives in the JSON object's `description`
field. Machine consumers read the structured envelope above
and ignore the trailing hint line; humans see both. If a future
runtime wants to machine-surface the hint, the preferred path
is adding a `hint` field to the JSON envelope; this spec does
not require it.

### 13.4. `RegionAbiMismatch`

Emitted during **module-metadata validation** at the start of a
compilation that imports a module whose region summary has
changed since the caller's compiled import record was recorded.
This is not a native-linker "link time" error (Vow's
`build/vowc` does whole-program compilation with no separate
native-linker stage visible to the caller) and not a
dynamic-load error (Vow has no dynamic loading today). It fires
when stale metadata is detected during import resolution.

Indicates a build inconsistency; not a program-level error.
`span` names the caller's call site on a best-effort basis —
module-metadata validation often resolves the stale-summary
discovery before the caller has been lowered to per-call-site
spans, so an all-zero span (`{file: "...", offset: 0, length: 0}`)
is a legitimate fallback indicating "no call-site position
available". The summary hashes are rendered into `message`
rather than surfaced as separate typed fields, matching the
convention used by other build-inconsistency diagnostics today.

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

Rename existing conflicting runtime allocation shims in the same
commit so the `__vow_arena_*` symbol space is free for this
spec's new primitives.

Standalone C-level tests + ESBMC verification on the primitive
in isolation (§10.4). Runtime ships unused by the compiler.

### Phase 2 — IR extension (both compilers, atomic)

Rename the existing conflicting `RegionId` newtype out of the
way in the same commit, freeing the `RegionId` name for the new
enum defined in §12.1.

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

**Spec sync requirement.** These three symbols are
surface-visible builtins. Per `CLAUDE.md` ("Any change to Vow
syntax, semantics, types, builtins, operators, effects, or CLI
flags MUST include a corresponding update to the relevant
`docs/spec/*.md` file"), Phase 7 MUST include a matching
update to `docs/spec/grammar.md` documenting:

- `pin_to_root`: the signature (per-type intrinsic,
  monomorphised at each call site), its deep-copy semantics,
  its `ConstantGlobal` return summary.
- `String::from_raw_parts_copy` and `Vec::from_raw_parts_copy`:
  per-type signatures with `len: i64`, the `FreshInCaller`
  return summary, and the documented `i64 → uintptr_t` FFI
  conversion.

Phase 7 also regenerates `--help` via `scripts/generate_help.py`
and the embedded skill document per the normal spec-sync
workflow described in `CLAUDE.md`.

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
- PR #181's conservative ownership patch comments in both
  compilers plus the self-hosted `lctx_mark_escaped` call sites.

Close issue #186.

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
- **Verifier `--max-k-step` auto-tuning.** The harness default (10)
  and `VerifyLimits.max_k_step` default may need raising for deeply
  nested calls or accumulating loops. Defaults may be revised after
  Phase 9.
