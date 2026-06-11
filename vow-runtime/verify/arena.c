/*
 * Standalone ESBMC verification harness for the arena primitive.
 * Mirrors vow-runtime/src/lib.rs semantics in pure C so ESBMC sees
 * ordinary malloc/free per docs/design/arena_memory.md §10.1.
 *
 * Verifies the §10.4 invariants:
 *   - cursor <= chunk_end at all times.
 *   - Allocation result lies within the current chunk's usable range.
 *   - Chunk chain walked by close frees every chunk (malloc/free pairs).
 *   - try_extend modifies only cursor and (on success) last_alloc_size;
 *     last_alloc_start is never changed; new last_alloc_size == new_size.
 *
 * Run via `make verify` (single-shot `--unwind 5`); the reachable chunk chain
 * is shallow, so incremental BMC is unnecessary here and far more costly (#516).
 */

#include <assert.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/* Forward-declare ESBMC intrinsics ESBMC links at verification time. */
void __ESBMC_assume(_Bool);
unsigned int nondet_uint(void);
size_t       nondet_size(void);

struct VowArena {
    void*     first_chunk;
    void*     current_chunk;
    uintptr_t cursor;
    uintptr_t chunk_end;
    void*     last_alloc_start;
    uintptr_t last_alloc_size;
    uintptr_t retained_bytes;
};

/* Chunk header layout: [next: 8][total|oversized-flag: 8]. The total word
 * at offset 8 records the chunk's libc::malloc size and also carries a high
 * bit recording whether the chunk was allocated via the oversized path.
 * The path-flag (not the size) is what arena_try_free_oversized_chunk
 * consults — a path-oversized chunk can have total <= normal_chunk_total()
 * for sub-4096-byte allocations (issue #391). */
#define CHUNK_LINK_BYTES        16
#define CHUNK_TOTAL_OFFSET      8
#define CHUNK_OVERSIZED_FLAG    ((uintptr_t)1 << 62)
/* Intentionally aliased to CHUNK_OVERSIZED_FLAG: addresses must stay strictly
 * below the flag bit so chunk-base arithmetic never collides with the
 * oversized marker stored in the size word. Moving the flag bit moves the
 * cap with it — that coupling is the point of the alias, not a coincidence. */
#define ARENA_VERIFY_ADDR_CAP   CHUNK_OVERSIZED_FLAG
#define CHUNK_PAYLOAD           4096
#define OVERSIZED_THRESHOLD     2048

/* `addr + align - 1` could wrap uintptr_t for adversarial inputs. Safe here
 * because the harness constrains `align` to {1, 8, 16, 4096} and bounds every
 * chunk to alloc_chunk's non-wrapping low-address model:
 * `total < ARENA_VERIFY_ADDR_CAP` and
 * `(uintptr_t)base <= ARENA_VERIFY_ADDR_CAP - total`. Widening either bound
 * (larger symbolic alignments, or removing the chunk-base bound) requires a
 * checked-add guard or explicit __ESBMC_assume on the sum. */
static uintptr_t align_up(uintptr_t addr, uintptr_t align) {
    return (addr + align - 1) & ~(align - 1);
}

static uintptr_t normal_total(void) {
    return CHUNK_LINK_BYTES + CHUNK_PAYLOAD;
}

static uintptr_t oversized_total(uintptr_t bytes, uintptr_t align) {
    return CHUNK_LINK_BYTES + bytes + (align - 1);
}

static void* alloc_chunk(uintptr_t total, int oversized) {
    void* base = malloc(total);
    if (base != NULL) {
        uintptr_t base_addr = (uintptr_t)base;
        /* ESBMC models malloc addresses symbolically. Constrain the abstract
         * pointer to the intended non-wrapping low-address model before any
         * base + total arithmetic, matching real hosts that do not map the
         * top of the address space. The bound is strict: addresses must stay
         * strictly below the flag bit (ARENA_VERIFY_ADDR_CAP aliases
         * CHUNK_OVERSIZED_FLAG), or `total == CHUNK_OVERSIZED_FLAG` would set
         * the oversized marker in the size word and `chunk_total()` would mask
         * the real size to 0. */
        __ESBMC_assume(total < ARENA_VERIFY_ADDR_CAP);
        __ESBMC_assume(base_addr <= ARENA_VERIFY_ADDR_CAP - total);
        /* Regression assert in derived form: implied by the two assumes above
         * but as a distinct expression, so ESBMC actually exercises it. A
         * tautological repeat of the assume would be a verification no-op. */
        assert(base_addr + total <= ARENA_VERIFY_ADDR_CAP);
        *(void**)base = NULL;  /* next-chunk link */
        uintptr_t word = total | (oversized ? CHUNK_OVERSIZED_FLAG : 0);
        *(uintptr_t*)((char*)base + CHUNK_TOTAL_OFFSET) = word;
    }
    return base;
}

static uintptr_t chunk_total(void* base) {
    return *(uintptr_t*)((char*)base + CHUNK_TOTAL_OFFSET) & ~CHUNK_OVERSIZED_FLAG;
}

static int chunk_is_oversized(void* base) {
    return (*(uintptr_t*)((char*)base + CHUNK_TOTAL_OFFSET) & CHUNK_OVERSIZED_FLAG) != 0;
}

static uintptr_t chunk_usable_start(void* base, uintptr_t align) {
    return align_up((uintptr_t)base + CHUNK_LINK_BYTES, align);
}

void __vow_arena_close(struct VowArena* a);

void __vow_arena_init_closed(struct VowArena* a) {
    a->first_chunk = NULL;
    a->current_chunk = NULL;
    a->cursor = 0;
    a->chunk_end = 0;
    a->last_alloc_start = NULL;
    a->last_alloc_size = 0;
    a->retained_bytes = 0;
}

void __vow_arena_open(struct VowArena* a) {
    if (a->first_chunk != NULL) {
        return;
    }

    uintptr_t total = normal_total();
    void* base = alloc_chunk(total, 0);
    __ESBMC_assume(base != NULL);  /* OOM is out of scope for §10.4; covered by OutOfMemory trap */
    a->first_chunk = base;
    a->current_chunk = base;
    a->cursor = chunk_usable_start(base, 8);
    a->chunk_end = (uintptr_t)base + total;
    a->last_alloc_start = NULL;
    a->last_alloc_size = 0;
    a->retained_bytes = total;
}

void __vow_arena_close(struct VowArena* a) {
    void* chunk = a->first_chunk;
    while (chunk != NULL) {
        void* next = *(void**)chunk;
        free(chunk);
        chunk = next;
    }
    a->first_chunk = NULL;
    a->current_chunk = NULL;
    a->cursor = 0;
    a->chunk_end = 0;
    a->last_alloc_start = NULL;
    a->last_alloc_size = 0;
    a->retained_bytes = 0;
}

void* __vow_arena_alloc(struct VowArena* a, uintptr_t bytes, uintptr_t align) {
    uintptr_t aligned = align_up(a->cursor, align);
    if (aligned + bytes <= a->chunk_end) {
        a->cursor = aligned + bytes;
        a->last_alloc_start = (void*)aligned;
        a->last_alloc_size = bytes;
        return (void*)aligned;
    }
    int oversized = (bytes > OVERSIZED_THRESHOLD || bytes + (align - 1) > CHUNK_PAYLOAD);
    uintptr_t total = oversized ? oversized_total(bytes, align) : normal_total();
    void* new_base = alloc_chunk(total, oversized);
    __ESBMC_assume(new_base != NULL);
    *(void**)a->current_chunk = new_base;
    a->current_chunk = new_base;
    uintptr_t start = chunk_usable_start(new_base, align);
    uintptr_t chunk_end_addr = (uintptr_t)new_base + total;
    /* Seal oversized chunks so subsequent allocations cannot land in the
     * alignment-slack tail; arena_try_free_oversized_chunk relies on the
     * resulting single-resident invariant. See lib.rs:__vow_arena_alloc. */
    a->cursor = oversized ? chunk_end_addr : (start + bytes);
    a->chunk_end = chunk_end_addr;
    a->last_alloc_start = (void*)start;
    a->last_alloc_size = bytes;
    a->retained_bytes += total;
    return (void*)start;
}

int64_t __vow_arena_try_extend(struct VowArena* a, void* ptr,
                                uintptr_t old_size, uintptr_t new_size) {
    if (ptr != a->last_alloc_start) return 0;
    if (a->last_alloc_size != old_size) return 0;
    if (new_size < old_size) return 0;
    uintptr_t delta = new_size - old_size;
    if (a->cursor + delta > a->chunk_end) return 0;
    a->cursor += delta;
    a->last_alloc_size = new_size;
    return 1;
}

/* Issue #391: release the chunk containing `ptr` if it is oversized and
 * non-tail. Mirrors arena_try_free_oversized_chunk in vow-runtime/src/lib.rs.
 * Used by arena_grow_backing after a growth that moved a Vec/String/HashMap
 * backing into a freshly allocated chunk. */
static int arena_try_free_oversized_chunk(struct VowArena* a, const void* ptr) {
    if (ptr == NULL) return 0;
    void* prev = NULL;
    void* chunk = a->first_chunk;
    while (chunk != NULL) {
        uintptr_t total = chunk_total(chunk);
        uintptr_t base = (uintptr_t)chunk;
        uintptr_t payload_start = base + CHUNK_LINK_BYTES;
        uintptr_t limit = base + total;
        if ((uintptr_t)ptr >= payload_start && (uintptr_t)ptr < limit) {
            if (!chunk_is_oversized(chunk)) return 0;
            if (chunk == a->current_chunk) return 0;
            void* next = *(void**)chunk;
            if (prev == NULL) {
                a->first_chunk = next;
            } else {
                *(void**)prev = next;
            }
            /* Plain unsigned subtraction by design (asymmetric with the Rust
             * mirror's `saturating_sub`): if the `retained_bytes >= total`
             * invariant is ever violated, ESBMC sees the resulting underflow
             * as an arithmetic anomaly rather than a silently-clamped zero. */
            a->retained_bytes -= total;
            free(chunk);
            return 1;
        }
        prev = chunk;
        chunk = *(void**)chunk;
    }
    return 0;
}

int main(void) {
    struct VowArena a;
    __vow_arena_init_closed(&a);
    __vow_arena_open(&a);
    /* Invariant at open: cursor <= chunk_end. */
    assert(a.cursor <= a.chunk_end);
    void* opened_first_chunk = a.first_chunk;
    void* opened_current_chunk = a.current_chunk;
    uintptr_t opened_cursor = a.cursor;
    uintptr_t opened_chunk_end = a.chunk_end;
    uintptr_t opened_retained_bytes = a.retained_bytes;
    __vow_arena_open(&a);
    assert(a.first_chunk == opened_first_chunk);
    assert(a.current_chunk == opened_current_chunk);
    assert(a.cursor == opened_cursor);
    assert(a.chunk_end == opened_chunk_end);
    assert(a.retained_bytes == opened_retained_bytes);

    /* Perform up to two symbolic allocations bounded in size. With large
     * symbolic alignments each alloc may take the oversized path (own
     * chunk), so close iterates up to 1 (first) + 2 (oversized allocs) = 3
     * chunks — well within the single-shot `--unwind 5` bound. */
    unsigned int n = nondet_uint();
    __ESBMC_assume(n <= 2);

    for (unsigned int i = 0; i < n; i++) {
        size_t sz = nondet_size();
        __ESBMC_assume(sz >= 1 && sz <= 64);
        /* Alignment symbolic over the power-of-two spectrum callers may use,
         * including large values that stress the normal-vs-oversized path
         * selection. */
        size_t align = nondet_size();
        __ESBMC_assume(align == 1 || align == 8 || align == 16 || align == 4096);
        uintptr_t before_end = a.chunk_end;
        void* before_last_start = a.last_alloc_start;

        void* p = __vow_arena_alloc(&a, sz, align);
        __ESBMC_assume(p != NULL);
        /* §10.4: cursor <= chunk_end, always. */
        assert(a.cursor <= a.chunk_end);
        /* Allocation pointer lies within the current chunk's usable range. */
        assert((uintptr_t)p >= (uintptr_t)a.current_chunk + CHUNK_LINK_BYTES);
        assert((uintptr_t)p + sz <= a.chunk_end);
        /* Returned pointer honours the requested alignment (issue #430):
         * align is part of the allocator contract, so a buggy align_up that
         * returned an in-bounds but under-aligned pointer must be rejected. */
        assert(((uintptr_t)p & (align - 1)) == 0);
        /* last_alloc_size is what we just requested. */
        assert(a.last_alloc_size == sz);

        /* Unused in no-op branches but keeps model honest. */
        (void)before_end;
        (void)before_last_start;

        /* Exercise try_extend semantics on the last alloc. */
        size_t grow = nondet_size();
        __ESBMC_assume(grow <= 16);
        size_t new_size = sz + grow;
        void* saved_start = a.last_alloc_start;
        uintptr_t saved_cursor = a.cursor;
        uintptr_t saved_chunk_end = a.chunk_end;
        /* Snapshot the non-owned header fields for the frame condition
         * (issues #431/#432): try_extend modifies *only* cursor and, on
         * success, last_alloc_size. */
        void* saved_first_chunk = a.first_chunk;
        void* saved_current_chunk = a.current_chunk;
        uintptr_t saved_retained = a.retained_bytes;
        /* try_extend must return 1 *exactly* when a valid extension fits
         * (issue #433): p is the last allocation and new_size >= sz, so the
         * sole deciding factor is whether the grow fits before chunk_end.
         * Without this the failure branch below is also satisfied by an
         * implementation that always returns 0, masking a growth regression. */
        int should_fit = (saved_cursor + grow <= saved_chunk_end);
        int64_t r = __vow_arena_try_extend(&a, p, sz, new_size);
        assert(r == (should_fit ? 1 : 0));
        if (r == 1) {
            /* Post-conditions per §10.4. */
            assert(a.last_alloc_size == new_size);
            assert(a.last_alloc_start == saved_start);
            assert(a.cursor == saved_cursor + grow);
            assert(a.cursor <= a.chunk_end);
        } else {
            /* On failure the arena header is unchanged. */
            assert(a.last_alloc_size == sz);
            assert(a.last_alloc_start == saved_start);
            assert(a.cursor == saved_cursor);
        }

        /* Frame condition (#431/#432): regardless of success/failure,
         * try_extend never touches any field other than cursor and (on
         * success) last_alloc_size. A regression that corrupted, e.g.,
         * retained_bytes or current_chunk while preserving cursor would
         * otherwise pass the branch assertions above. */
        assert(a.first_chunk == saved_first_chunk);
        assert(a.current_chunk == saved_current_chunk);
        assert(a.chunk_end == saved_chunk_end);
        assert(a.last_alloc_start == saved_start);
        assert(a.retained_bytes == saved_retained);

        /* Negative cases: try_extend must reject mismatched calls (#433) —
         * a wrong pointer, a wrong old_size, or a shrink each return 0. */
        void* cur_start = a.last_alloc_start;
        uintptr_t cur_size = a.last_alloc_size;
        assert(__vow_arena_try_extend(&a, (char*)cur_start + 1, cur_size, cur_size + 1) == 0);
        assert(__vow_arena_try_extend(&a, cur_start, cur_size + 1, cur_size + 2) == 0);
        assert(__vow_arena_try_extend(&a, cur_start, cur_size, cur_size - 1) == 0);
    }

    /* Directed scenario for issue #391: drop an oversized non-tail chunk and
     * confirm the chain still terminates and close frees the remainder. The
     * 4097-byte request unambiguously takes the oversized path because
     * `bytes + (align - 1) = 4104 > CHUNK_PAYLOAD = 4096`, so it cannot
     * fit in the fresh arena's initial normal chunk via the fast path
     * (the Rust mirror uses a 64-byte warmup alloc instead; this works
     * regardless of where libc::malloc places the initial chunk). */
    void* big = __vow_arena_alloc(&a, 4097, 8);  /* path-oversized chunk */
    void* big_chunk = a.current_chunk;
    void* tail_marker = __vow_arena_alloc(&a, 8, 8); /* forces a new chunk */
    (void)tail_marker;
    assert(a.current_chunk != big_chunk);
    uintptr_t bytes_before = a.retained_bytes;
    int freed = arena_try_free_oversized_chunk(&a, big);
    assert(freed == 1);
    assert(a.retained_bytes < bytes_before);
    /* Walking the chain must not encounter the freed chunk. */
    void* walk = a.first_chunk;
    while (walk != NULL) {
        assert(walk != big_chunk);
        walk = *(void**)walk;
    }

    __vow_arena_close(&a);
    /* After close the chain is emptied. */
    assert(a.first_chunk == NULL);
    assert(a.current_chunk == NULL);
    return 0;
}
