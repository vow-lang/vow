# Plan: #1403 feat(runtime): real HashMap and a hash builtin

Part of epic #1398 (native verifier, task G4/D12 "enabling work first"). PR title (squash subject, <= 92 chars):
`feat(runtime): hashed HashMap backing and hash_str/hash_u64 builtins`.

## 1. Problem restated

`HashMap<K, V>` is a Rust-runtime (`vow-runtime/src/lib.rs`, ~L4376-4530) array of `(key, val)` pairs scanned linearly:
`insert`/`get`/`contains`/`remove` are O(n), so building an n-entry map is O(n^2). The planned `compiler/vc_*.vow`
verifier (hash-consed term arena, symbol tables) cannot be built on that. Replace the backing with an open-addressing
hash table behind the *identical* API/ABI (`__vow_map_*` symbols, signatures, arena routing, `Option` result), and add
a pure hash builtin so Vow programs can reduce non-scalar keys (the spec's own advice for `String` keys: "hash or
intern to a `u64` and keep a side table") without hand-rolling wrapping-arithmetic mixers.

Key facts established while reading the code:
- Both compilers link the **same** Rust `vow-runtime`, so the map rewrite is a single-site change; "both compilers" applies
  to the **builtin registration**, docs and tests. No `HashMap` iteration API exists, so slot order is unobservable and
  cannot affect the binary fixed point. No codegen reads `VowMap` fields inline (only `__vow_map_*` externs).
- Map ABI symbols are unchanged, so `compiler/clif.vow:301`, `compiler/ir.vow:406`, `compiler/region.vow`,
  `vow-ir/src/region.rs` arena routes, `c_emitter.{rs,vow}` (ESBMC models maps abstractly, not via the runtime) need **no** edits.
- Unmodeled externs make the calling function `Skipped` in the verifier (fail-closed, D10); hash builtins therefore
  need no C-model entry in this PR (their native model is the verifier epic's job).

## 2. Design decisions

**Table** (header stays `VowMap { ptr, len, cap, owner }`, 32 bytes, so every test/`RODATA` literal that builds a header and the `cap == VOW_CAP_RODATA` mutation trap keep working):
- `len` = live entries; `cap` = slot count, always a power of two (initial `MAP_INITIAL_CAP = 8`).
- One buffer in the owner arena: `cap * 16` bytes of `(key, val)` slots followed by `cap` control bytes (0 empty / 1 full), 8-byte aligned, zero-initialised. A separate control array is required because every `i64` key value (incl. 0, -1, `i64::MIN`) is legal, so no key sentinel exists.
- Linear probing from `hash_u64(key as u64) & (cap - 1)`. Max load 3/4: insert of a *new* key grows first when `(len + 1) * 4 > cap * 3`.
- **Removal uses backward-shift deletion** (no tombstones): lookup cost never degrades after churn and `len` stays the single load measure.
- Growth = allocate a new buffer of `2*cap` in the owner arena (respecting the existing root-arena-lock logic of `arena_grow_map_buffers`: same `arena_is_root`/`ROOT_LOCK_HELD`/`with_root_arena` dance, new helper `arena_alloc_map_buffer`) and rehash. In-place `arena_grow_backing` extension cannot be used (layout depends on `cap`; rehash needs the old copy). The old buffer remains arena-owned and is reclaimed with the arena; total waste is geometric (<= final buffer size). **Implementer must check** whether the arena exposes a release-last-allocation primitive and use it if so; otherwise document the bound in `docs/design/arena_memory.md` (ownership story required by CLAUDE.md).
- Hash function is **deterministic** (no per-process seed): required for reproducible binaries/fixed point. Consequence (document): adversarially crafted keys can force long probe chains; this is acceptable for compiler-style workloads and explicitly stated in the spec.
- The internal mixer is the same function as the `hash_u64` builtin (one implementation, `fn map_hash(u64) -> u64`).

**Builtins** (pure, effects `[]`, minimal surface; two functions because `u64` is the only key shape `HashMap` accepts and `String` is the shape agents most often need to key by):

| Name | Signature | Definition |
|---|---|---|
| `hash_u64` | `fn(x: u64) -> u64` | SplitMix64 output function applied to state `x` (`z = x + 0x9E3779B97F4A7C15; z = (z ^ z>>30) * 0xBF58476D1CE4E5B9; z = (z ^ z>>27) * 0x94D049BB133111EB; z ^ z>>31`, wrapping). `hash_u64(0) == 0xE220A8397B1DCDAF`. Bijective on `u64`. |
| `hash_str` | `fn(s: String) -> u64` | FNV-1a 64-bit over the bytes (offset `0xCBF29CE484222325`, prime `0x100000001B3`). Null/empty string hashes to the offset basis. |

Language-principles justification (CLAUDE.md three criteria): pure externs add no type-system axis and no verifier
surface (Skipped when called, like other unmodeled externs); they remove the class of "hand-rolled hash with wrong
wrapping/overflow or non-portable constants" and the `String`-key-compares-by-pointer trap the spec already warns about;
they let agents reach the existing documented side-table idiom in one call. The spec must state that a hash is **not**
an identity: colliding strings must be disambiguated by a side-table comparison.

Rejected: `hash_combine` (composable as `hash_u64(a ^ hash_u64(b))` in Vow), `hash_i64`/per-width variants (cast to `u64`), randomised seeding (breaks reproducibility), a `HashMap<String, V>` key extension (separate type-system change, out of scope).

## 3. Files to touch

Runtime (single implementation shared by both compilers):
- `vow-runtime/src/lib.rs` - rewrite `VowMap` section (new buffer layout helpers, `__vow_map_insert_in_arena`, `__vow_map_get_in_arena`, `__vow_map_contains`, `__vow_map_remove`; `__vow_map_new_in_arena` allocs 136-byte-class buffer; `len`, `*_in_arena` wrappers and null-arena/RODATA traps unchanged); add `__vow_hash_u64`, `__vow_hash_str`; update `perf_map_cost` comment and costs (L324-345); update existing unit tests that assert `cap`/growth/entry layout (~L5258, 5300, 6059-6240, 7270-7400, 7676-7780, incl. `make_rodata_map`).

Builtin registration (template: `parse_f64_bits`):
- `docs/spec/operations.json` - add `hash_u64` (`__vow_hash_u64`, params `["u64"]`, return `u64`, effects `[]`, `doc_signature "fn(x: u64) -> u64"`) and `hash_str` (`__vow_hash_str`, params `["ptr"]`, return `u64`, `fn(s: String) -> u64`). Using the catalogue means the generator emits the symbol/ABI sites below; do **not** hand-edit between `GENERATE:OPERATIONS` markers.
- `python3 scripts/generate_operations.py` regenerates: `vow-ir/src/lower/mod.rs` (`catalogue_builtin_to_runtime`), `vow-codegen/src/cranelift_backend.rs` + `vow-clif-shim/src/lib.rs` (`catalogue_extern_sig`), `compiler/lower.vow` (`catalogue_builtin_to_extern`, `catalogue_builtin_ret_ty`). Verify the `arith`/non-`ptr` return `u64` shape is supported by the generator; if the generator cannot express it, fall back to hand-adding both symbols to the `parse_f64_bits` sites (`vow-ir/src/lower/mod.rs:159`, `cranelift_backend.rs:2808`, `vow-clif-shim/src/lib.rs:3858`, `compiler/lower.vow:1961/2039`) and file a follow-up.
- `vow-types/src/env.rs` - `builtin_free_fn_signatures()` (`def("hash_u64", vec![Ty::U64], Ty::U64, &[])`, `def("hash_str", vec![Ty::Str], Ty::U64, &[])`) and the signature snapshot list (~L897-990).
- `compiler/env.vow` (~L504) - mirrored `env_define_fn` entries (must stay in the same order/shape as Rust; this is the single source of signatures, not the catalogue).
- Check for hand-maintained "pure builtin"/side-effect/dead-code tables in `vow-ir/src/` passes and `compiler/ir.vow`/`region.vow`/`lower.vow:1629` (the `format_f64_bits`/`proc_sample` list is for heap-returning builtins; scalar `u64` results should NOT be added there). `grep -n "parse_f64_bits"` is exhaustive for the template; mirror only the sites that are not catalogue-generated.

Docs / generated:
- `docs/spec/grammar.md` - L197 type table row (drop "linear scan", say "hash table, expected O(1) amortised `insert`/`get`/`contains_key`/`remove`"); the HashMap methods section (~L1082-1100: deterministic hashing, no iteration order, adversarial-collision caveat, String-key idiom now via `hash_str` + side table, hash is not identity); Builtin Function Signatures table (~L1432 region) for `hash_u64`/`hash_str` with the exact algorithms above.
- `docs/design/arena_memory.md` (~L450-495) - new layout, growth path, ownership of abandoned buffers, remove no longer "linear-scan".
- Regenerate: `uv run python scripts/generate_help.py` (updates `compiler/main.vow` help JSON/skill text, `vow/src/skill.rs`, `skills/vow/reference/*`), then `python3 scripts/check_help_coverage.py` and `python3 scripts/generate_operations.py --check`.
- `CHANGELOG.md` only if it is hand-maintained (it is release-generated by semantic-release; leave alone).

Tests / fixtures (new):
- `tests/run/hashmap_collisions.vow`, `tests/run/hashmap_growth.vow`, `tests/run/hashmap_remove_churn.vow`, `tests/run/hashmap_stress.vow`, `tests/run/hash_builtins.vow` (each with `// TEST: stdout`/`exit`).
- `tests/verify-skip/hash_builtin_call_skipped.vow` (caller of `hash_str` is reported `Skipped`, not proven) - confirm the `TEST:` directive format of existing `verify-skip` fixtures first.
- `bench/memory/programs/alloc_loop_hashmap_growth.vow` (+ `// BENCH: max-rss-kb` = measured + 4096 by hand, plus `expected.toml` entry; **never** `run.sh --record`).
- Optionally `compiler/tests/` unit test pinning the `hash_*` -> `__vow_hash_*` mapping if catalogue tests do not already cover it.

## 4. TDD slices (each one commit; conventional-commit subjects lower-case)

1. **Red: behaviour + complexity tests for the map (runtime unit tests, `vow-runtime/src/lib.rs` `#[cfg(test)]`).**
   Add a probe counter under `#[cfg(test)]` (thread-local, incremented per slot inspected) and tests: (a) inserting 100k sequential keys and 100k keys that are multiples of 2^20 (identical low bits) averages < 4 probes per op; (b) growth keeps every key retrievable and `cap` a power of two with `len*4 <= cap*3`; (c) backward-shift remove in the middle of a forced cluster keeps later cluster members findable, then reinsertion works; (d) sentinel-like keys `0, -1, i64::MIN, i64::MAX` as ordinary keys; (e) overwrite does not change `len`; (f) remove of missing key is a no-op; (g) RODATA map still traps on insert/remove; (h) map in non-root owner arena with root insert wrapper (existing routing tests). Red today only for the probe-count and layout assertions; the pure-behaviour tests document and lock existing semantics before the rewrite (state this honestly in the PR).
2. **Green: hashed table.** Implement the layout/probe/grow/backward-shift code in `vow-runtime`; update the pre-existing tests that hard-code linear-scan layout. Keep `__vow_map_*` signatures byte-identical.
3. **Perf cost model.** Update `perf_map_cost` (+ comment at L324): `get`/`contains`/`remove` charge a constant, `insert` a constant amortising growth; test that charge no longer scales with `len`. Confirm no `vow-perf` test or fixture depends on the old linear cost (`grep perf_count_map`; none found outside the shim/backend extern lists).
4. **Red: hash builtin unit tests** (`vow-runtime`): FNV-1a vectors (`""`=0xCBF29CE484222325, `"a"`=0xAF63DC4C8601EC8C, `"foobar"`=0x85944171F73967E8), `hash_u64(0)=0xE220A8397B1DCDAF` plus a second independently computed vector, null string pointer, bijectivity spot-check, determinism across calls, non-ASCII bytes. **Green:** `__vow_hash_u64`/`__vow_hash_str` and make `map_hash` call the shared mixer (`sanitize_on_read` as `__vow_parse_f64_bits` does).
5. **Red: registration tests then green.** `vow-types` signature snapshot/test and a Rust `vow-ir` lowering test (builtin -> `__vow_hash_*`, `Ty::U64`); `tests/run/hash_builtins.vow` calling both from a **pure** function (effect set `[]` must type-check) and from `main`. Make green via `operations.json` + `generate_operations.py` + `vow-types/src/env.rs` + `compiler/env.vow`. Run the fixture through `./target/release/vow` and `build/vowc` and compare.
6. **Vow-level runtime fixtures** in `tests/run/` (both compilers, run by `scripts/full_test.sh` Section 4):
   - `hashmap_collisions.vow`: keys sharing low bits (`i * 1048576`), negative keys, `u8`/`i32`/`bool` widened keys, same-hash-bucket removal order.
   - `hashmap_growth.vow`: insert 0..50_000, verify `len`, every `get`, miss keys; grow across several doublings; nested maps / map inside a struct and a map passed to a callee that inserts (region routing: `__vow_map_insert_in_arena` path).
   - `hashmap_remove_churn.vow`: insert N, remove evens, verify odds + misses, reinsert, repeat for 20 rounds with a checksum (guards backward-shift bugs and tombstone-free steady state).
   - `hashmap_stress.vow`: 300k inserts + 300k gets + 300k removes (+ final `len == 0`). Quadratic scan would take tens of seconds and trip the harness timeout; amortised O(1) finishes well under a second. Use the probe-count unit test (slice 1) as the deterministic gate; this fixture is the end-to-end sanity gate (state this so nobody mistakes wall-clock for the assertion).
   - `bench/memory` program `alloc_loop_hashmap_growth.vow` pinning peak RSS of a large growing map; re-run `scripts/check_memory_bounds.py --compiler vow` and `--compiler vowc` to ensure the existing `alloc_*hashmap*`/`alloc_loop_map` bounds still hold (header/initial buffer changes by one control array; raise a bound by hand only with measured evidence).
7. **Docs + generated artefacts** (grammar.md, arena_memory.md, help/skills regeneration, `verify-skip` fixture). `check_help_coverage.py` and `generate_operations.py --check` must pass.
8. **Final gates (separate commands, background + poll, never chained):** `cargo fmt --all --check`, `cargo clippy --all --all-targets -- -D warnings`, `cargo test -p vow-runtime`, `cargo test -p vow-types`, `cargo test -p vow-ir`, `cargo test --all`, `scripts/bootstrap.sh --skip-cargo --no-cache` (rebuild `target/release/vow` first, `cargo build --release -p vow`), `build/vowc test compiler/` (relevant files), then `scripts/full_test.sh` (~40 min; at minimum Sections 0b/2c parity/4/8). Record the checked head SHA in the PR checklist line for the bootstrap fixed-point claim (CLAUDE.md rule). Known pre-existing failures per memory (e2e tests SKIP-panic in sandbox, `u64_marker_propagation`, `contracts_tmp_cleanup`, `concrete-block-region-parity`) must be verified against clean `origin/main` before blaming this change.

Acceptance criteria mapping:
- amortised O(1) stress -> slice 1 probe-count test + `hashmap_stress.vow` + perf cost model;
- builtin documented/registered in both compilers -> slices 5, 7 (`grammar.md`, `env.rs`, `env.vow`, catalogue-generated lowering/codegen sites);
- runtime tests in `tests/run` for collisions/growth/removal -> slice 6;
- fixed point -> slice 8 bootstrap (`--no-cache`, SHA recorded).

## 5. Verification surface

- No contracts change; no ESBMC property needs proving. The verifier's HashMap model lives in `c_emitter.{rs,vow}` (abstract), independent of the runtime representation -> byte-identical C parity (`scripts/parity.py c`, Section 2c) must stay green with **zero** C-emitter edits. If parity moves, something unintended changed.
- `hash_u64`/`hash_str` are unmodeled externs: any function calling them is `Skipped` (fail-closed). The new `verify-skip` fixture pins that; nothing is "proven" about hash outputs. Do not add a nondet C model (a nondet `hash(x)` would be unsound for `hash(x)==hash(x)`).
- Contracts on any new Vow fixtures must be true contracts (no ESBMC-driven bounds); fixtures that exercise the maps at 50k-300k entries are `tests/run` programs without contracts, so verification cost is nil.

## 6. Risk areas

- **Binary fixed point / determinism:** the compiler itself uses `HashMap` (e.g. `compiler/module_io.vow`, env tables). Any slot-order dependence would be a latent iteration bug, but there is no iteration API; confirm with a grep that nothing reads map internals. No seed/pointer/time may feed the hash.
- **Arena ownership of the growth buffers:** root vs block vs caller arenas and `ROOT_LOCK_HELD` re-entrancy (existing `arena_grow_map_buffers` logic must be reproduced for the fresh-allocation path); null `owner` must still trap (`null_arena_trap("map growth")`), and `__vow_map_insert` on RODATA must still trap *before* touching the table. Test at least: root map, block-arena map, map whose owner differs from the inserting arena.
- **Memory bounds:** new initial buffer = 128 + 8 bytes; growth doubles with an abandoned old buffer. `check_memory_bounds.py` for both compilers; arena Section 8a is Linux-only.
- **Hash quality vs probe length:** linear probing with a good 64-bit mixer at <= 3/4 load; if the probe-count unit test shows > 4 average on structured keys, tighten max load to 5/8 rather than changing the mixer.
- **Catalogue generator limits:** `u64` return and `ptr` param are supported by existing entries (`print_u64`, `fs_read`); if validation rejects `effects: "[]"` or a scalar-return/ptr-param combo, fix by extending the generator in a separate preceding commit or fall back to hand registration (see section 3).
- **Spec/help drift:** `grammar.md` <-> `--help` <-> `skill.rs` <-> `skills/vow/reference/*`; regenerate, never hand-edit generated blocks. `parse -> print -> parse` is unaffected (no syntax change). `cargo clippy -D warnings` covers `--all-targets`; new `unsafe` blocks need the repo's existing style, probe counter must be `#[cfg(test)]`-gated.
- **`vow-clif-shim`/stack-slot layout:** untouched (symbol ABI unchanged); only `catalogue_extern_sig` gains two entries.
- **Commit hygiene:** conventional commits, lower-case subjects, header <= 100 chars; `git rm PLAN.md` before opening the PR.

## 7. Out of scope (deliberately not bundled)

- `HashMap<String, V>` / struct / tuple keys, 128-bit keys or values, any new type-system change.
- Map iteration (`keys`, `values`, `iter`), `clear`, `reserve`, `shrink_to_fit`, shrinking on removal.
- `BTreeMap` changes (it also has O(n) insert; separate issue if wanted) and `Vec`/`String` runtime changes.
- Randomised hashing / hash-flooding defence (conflicts with deterministic builds; documented instead).
- Native-verifier modelling of `hash_*` (epic #1398 later phases), C-emitter model changes, `verifier_model` annotation wiring.
- Refactoring the `__vow_map_*` root-wrapper/arena-routing structure, formatting-only changes, or other builtins (#1404+ G13 `getenv`/`mktemp`).
