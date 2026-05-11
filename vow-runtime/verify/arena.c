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
 * Run via `make verify` (uses --incremental-bmc --max-k-step 10), matching
 * the vow-verify pipeline's ESBMC invocation convention.
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

#define CHUNK_LINK_BYTES    8
#define CHUNK_PAYLOAD       4096
#define OVERSIZED_THRESHOLD 2048

/* `addr + align - 1` could wrap uintptr_t for adversarial inputs. Safe here
 * because the harness constrains `align` to {1, 8, 16, 4096} and bounds
 * every chunk address via alloc_chunk's `(uintptr_t)base + total <= 1<<62`
 * assumption. Widening either bound (larger symbolic alignments, or removing
 * the chunk-base bound) requires a checked-add guard or explicit
 * __ESBMC_assume on the sum. */
static uintptr_t align_up(uintptr_t addr, uintptr_t align) {
    return (addr + align - 1) & ~(align - 1);
}

static uintptr_t normal_total(void) {
    return CHUNK_LINK_BYTES + CHUNK_PAYLOAD;
}

static uintptr_t oversized_total(uintptr_t bytes, uintptr_t align) {
    return CHUNK_LINK_BYTES + bytes + (align - 1);
}

static void* alloc_chunk(uintptr_t total) {
    void* base = malloc(total);
    if (base != NULL) {
        /* Real-world malloc returns addresses far below UINTPTR_MAX; no OS
         * maps the top of the address space. Constrain the abstract pointer
         * so ESBMC does not consider overflow scenarios that cannot occur
         * on any real host. */
        __ESBMC_assume((uintptr_t)base + total <= ((uintptr_t)1 << 62));
        *(void**)base = NULL;  /* next-chunk link */
    }
    return base;
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
    void* base = alloc_chunk(total);
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
    uintptr_t total = (bytes > OVERSIZED_THRESHOLD || bytes + (align - 1) > CHUNK_PAYLOAD)
        ? oversized_total(bytes, align)
        : normal_total();
    void* new_base = alloc_chunk(total);
    __ESBMC_assume(new_base != NULL);
    *(void**)a->current_chunk = new_base;
    a->current_chunk = new_base;
    uintptr_t start = chunk_usable_start(new_base, align);
    a->cursor = start + bytes;
    a->chunk_end = (uintptr_t)new_base + total;
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
     * chunks — fits comfortably within incremental BMC's step bound. */
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
        int64_t r = __vow_arena_try_extend(&a, p, sz, new_size);
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
    }

    __vow_arena_close(&a);
    /* After close the chain is emptied. */
    assert(a.first_chunk == NULL);
    assert(a.current_chunk == NULL);
    return 0;
}
