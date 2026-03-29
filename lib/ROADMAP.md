# Vow Standard Library Roadmap

This document outlines candidate libraries for the Vow standard library,
prioritized by verification fitness, agent utility, and implementation
feasibility. GC, bignum, and math are already in progress.

## Tier 1 — High impact, low verifier cost

### Byte Buffer (`lib/bytes`)
Low-level `Vec<u8>` utilities for binary data manipulation.

**Functions:** `bytes_get_u16_be`, `bytes_get_u32_be`, `bytes_get_u64_be`,
`bytes_put_u16_be`, `bytes_put_u32_be`, `bytes_put_u64_be` (and `_le`
variants), `bytes_copy`, `bytes_equal`, `bytes_fill`, `bytes_slice`.

**Why now:** Bignum already needs byte-level manipulation. Crypto (hashing)
needs it. Serialization needs it. Pure functions with tight bounds — contracts
like `requires: offset + 4 <= v.len()` are trivially verifiable.

**Verifier impact:** Near zero. All operations desugar to Vec indexing with
offset arithmetic. Bounded by existing Vec limits (128 elements).

**Depends on:** Nothing (uses existing `Vec<u8>` and `u64`).

---

### String Builder / Formatter (`lib/fmt`)
Structured string construction from typed parts.

**Functions:** `fmt_i64(n) -> String`, `fmt_u64(n) -> String`,
`fmt_bool(b) -> String`, `fmt_pad_left(s, width, pad_byte) -> String`,
`fmt_pad_right(s, width, pad_byte) -> String`, `fmt_hex_u64(n) -> String`,
`fmt_join(parts: Vec<String>, sep: String) -> String`.

**Why now:** Agents building output strings (diagnostics, logs, protocol
messages) currently concatenate with `push_str` in ad-hoc ways. A small set
of pure formatting functions eliminates a class of off-by-one and
missing-separator bugs.

**Verifier impact:** Minimal. String operations are already modeled.
Formatting functions are pure and bounded.

**Depends on:** Nothing (uses existing String builtins).

---

### JSON (`lib/json`)
Parse and emit JSON. This is the lingua franca of agent communication — Vow's
own diagnostics are JSON.

**Types:** An enum-based `JsonValue` (Null, Bool, Number, Str, Array, Object).

**Functions:** `json_parse(s: String) -> Result<JsonValue, String>`,
`json_emit(v: JsonValue) -> String`, `json_get_field(obj, key) -> Option<JsonValue>`,
`json_get_index(arr, i) -> Option<JsonValue>`.

**Why now:** Any agent that consumes or produces structured data needs JSON.
Without it, agents hand-roll parsers that are error-prone and unverifiable.
A recursive-descent parser over `byte_at` is straightforward in Vow.

**Verifier impact:** Moderate. Recursive parsing over bounded strings is
tractable. The JsonValue enum exercises Vow's enum/match machinery well.

**Depends on:** `lib/fmt` (for number-to-string in emission).

---

## Tier 2 — Solid utility, modest verifier cost

### Sorting & Searching (`lib/sort`)
Extend beyond the existing `vec_sort` builtin with verified variants.

**Functions:** `insertion_sort(v: Vec<i64>) -> Vec<i64>` (with
`ensures: vec_is_sorted(result)`), `binary_search(v: Vec<i64>, target: i64) -> i64`
(with `requires: vec_is_sorted(v)`), `vec_partition(v, pivot) -> (Vec<i64>, Vec<i64>)`,
`vec_merge(a: Vec<i64>, b: Vec<i64>) -> Vec<i64>` (both sorted).

**Why now:** The benchmark suite already has sorting/searching tasks. Library
versions with verified contracts would let agents compose rather than
re-implement.

**Verifier impact:** Low-moderate. Sorting invariants (`ensures: is_sorted(result),
ensures: result.len() == v.len()`) are classic verification targets.

**Depends on:** `lib/math/vec_math` (for `vec_is_sorted`).

---

### Bitwise Operations (`lib/bits`)
Extend beyond the existing XOR operator.

**Functions:** `bit_and(a, b) -> i64`, `bit_or(a, b) -> i64`,
`bit_not(a) -> i64`, `shift_left(a, n) -> i64`, `shift_right(a, n) -> i64`,
`bit_count(a) -> i64` (popcount), `bit_test(a, n) -> i64`,
`bit_set(a, n) -> i64`, `bit_clear(a, n) -> i64`.

**Why now:** Crypto (SHA-256, etc.) requires all of these. Currently only XOR
exists as an operator. These would need to be wired as builtins through the
runtime since Vow lacks bitwise AND/OR/shift operators.

**Verifier impact:** Low. Bitwise ops are well-modeled in ESBMC's C backend.

**Depends on:** Runtime additions (new `extern "C"` functions in `vow-runtime`).

---

### Crypto Hashes (`lib/crypto`)
Pure hash functions over byte buffers.

**Functions:** `sha256(data: Vec<u8>) -> Vec<u8>` (returns 32 bytes),
`sha256_hex(data: Vec<u8>) -> String`, `hmac_sha256(key: Vec<u8>, data: Vec<u8>) -> Vec<u8>`.

**Why now:** Agents dealing with authentication, integrity checks, or content
addressing need hashing. SHA-256 is implementable as pure Vow code given
bitwise ops and byte buffer support.

**Verifier impact:** Moderate. The algorithm is fixed and deterministic.
Contracts can verify output length (`ensures: result.len() == 32`) and
known test vectors.

**Depends on:** `lib/bits`, `lib/bytes`.

---

## Tier 3 — Useful but larger scope

### Environment & Config (`lib/env`)
Access to environment variables and simple config parsing.

**Functions:** `env_get(key: String) -> Option<String>`,
`env_get_or(key: String, default: String) -> String`.
Possibly: `parse_ini(s: String) -> HashMap<String, String>`.

**Why:** Agents need configuration. Currently the only external input is
`args()` and `stdin_read`.

**Verifier impact:** Minimal (env_get is a new `[read]` effect builtin).

**Depends on:** Runtime addition for `getenv(3)`.

---

### CSV / TSV Parser (`lib/csv`)
Line-oriented structured text parsing.

**Functions:** `csv_parse_line(line: String, sep: String) -> Vec<String>`,
`csv_parse(data: String, sep: String) -> Vec<Vec<String>>`,
`csv_emit_line(fields: Vec<String>, sep: String) -> String`.

**Why:** Common data interchange format. Implementable purely using
`string_split` and `string_join`.

**Verifier impact:** Low. Pure string manipulation.

**Depends on:** Nothing.

---

### Ring Buffer (`lib/ring`)
Fixed-capacity FIFO queue with contracts.

**Types:** `Ring` struct with `data: Vec<i64>`, `head: i64`, `tail: i64`, `count: i64`, `cap: i64`.

**Functions:** `ring_new(cap) -> Ring`, `ring_push(r, val) -> Ring`
(with `requires: r.count < r.cap`), `ring_pop(r) -> Ring`
(with `requires: r.count > 0`), `ring_peek(r) -> i64`,
`ring_len(r) -> i64`, `ring_is_full(r) -> i64`.

**Why:** Bounded data structure with natural contracts. Useful for streaming
pipelines, rate limiting, sliding windows.

**Verifier impact:** Low. Modular arithmetic on bounded indices is simple.

**Depends on:** Nothing.

---

### Set (`lib/set`)
Integer set backed by sorted vector.

**Functions:** `set_new() -> Vec<i64>`, `set_insert(s, val) -> Vec<i64>`,
`set_contains(s, val) -> i64`, `set_remove(s, val) -> Vec<i64>`,
`set_union(a, b) -> Vec<i64>`, `set_intersection(a, b) -> Vec<i64>`,
`set_difference(a, b) -> Vec<i64>`, `set_size(s) -> i64`.

**Why:** Eliminates hand-rolled dedup/membership patterns. Sorted-vector
backing means `set_contains` uses binary search, and set invariant
(`ensures: vec_is_sorted(result)`) is verifiable.

**Depends on:** `lib/sort` (binary search), `lib/math/vec_math` (`vec_is_sorted`).

---

## Recommended sequencing

```
Already done/in progress:
  GC → Bignum → Math ✓

Next batch (Tier 1):
  Bytes → Fmt → JSON

Then (Tier 2, can parallelize):
  Bits → Crypto
  Sort → Set

Finally (Tier 3, as needed):
  Env, CSV, Ring
```

The Tier 1 batch (bytes, fmt, json) is the highest-leverage next step because
it enables agents to do structured I/O — which is the primary way agents
communicate with the outside world. Each library is small (5-15 functions),
pure, and verification-friendly.
