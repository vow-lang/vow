# Level 3 Agent Capability Test Results

**Date:** 2026-03-06
**Target:** `compiler/lexer.vow` (416 lines, 10 functions)
**Tool:** `vow verify compiler/lexer.vow` (ESBMC 8.0.0 backend)

## Summary

**Level 3 Capability Assessment: PARTIAL**

An agent can retrofit contracts onto simple, self-contained functions in existing
compiler modules. However, three categories of ESBMC limitations prevent
verification of functions with cross-module calls, String method usage, or deep
branching.

## Pre-existing Contracts (6 functions)

These functions already had contracts before this test:

| Function | Contracts | Status |
|---|---|---|
| `is_whitespace` | `requires: b >= 0, requires: b <= 255` | Verified |
| `is_alpha` | `requires: b >= 0, requires: b <= 255` | Verified |
| `is_digit` | `requires: b >= 0, requires: b <= 255` | Verified |
| `is_ident_start` | `requires: b >= 0, requires: b <= 255` | Verified |
| `is_ident_cont` | `requires: b >= 0, requires: b <= 255` | Verified |
| `suffix_len` | `requires: suffix >= -1, ensures: result >= 0, ensures: result <= 5` | Verified |

## Retrofitted Contracts (4 functions attempted)

### 1. `keyword_bool_val` — VERIFIED

**Contract added:**
```
ensures: result == 0 || result == 1
```

**Result:** Verified on first attempt. No CEGIS iterations needed.

**Analysis:** Simplest function (4 lines). Two branches return literal `1` or `0`.
No cross-module function calls — only `s.eq(String::from("true"))` which ESBMC
models correctly. This is the ideal case for contract retrofitting.

### 2. `keyword_tag` — FAILED (spurious counterexample)

**Contract attempted:**
```
ensures: result >= 0
```

**Result:** ESBMC produced a counterexample claiming the function could return a
negative value. The counterexample is **spurious** — all 30+ `tok_*()` functions
(defined in `token.vow`) return non-negative integer constants (0–92).

**Root cause:** ESBMC cannot reason through the 30-branch if-else chain where each
branch calls a different cross-module function. The function calls `tok_kw_fn()`,
`tok_kw_let()`, ..., `tok_ident()` — all defined in `token.vow` and returning
constants 0–92. ESBMC's bounded model checking exhausts its unwind budget before
proving all branches return non-negative values.

**CEGIS iterations:** 1 (contract was correct; failure is an ESBMC scale limit).

**Limitation category:** Cross-module function inlining + deep branching.

### 3. `try_suffix` — FAILED (requires cascade)

**Contract attempted (iteration 1):**
```
requires: pos >= 0
requires: src_len >= 0
ensures: result >= -1
```

**Result:** ESBMC found a caller-blame violation — the `lex` function calls
`try_suffix` and ESBMC cannot prove `pos >= 0` at the call site without loop
invariants on `lex`'s main loop.

**Contract attempted (iteration 2):** Removed `requires`, kept only `ensures: result >= -1`.

**Result:** Still failed. ESBMC found a path through `is_ident_cont` (called within
`try_suffix`) that violates `is_ident_cont`'s `requires: b >= 0, requires: b <= 255`.
The chain: `try_suffix` → `is_ident_cont(src.byte_at(pos + N))` — ESBMC can't prove
`byte_at` returns values in [0, 255].

**CEGIS iterations:** 2 (weakened contract; failure is a requires-cascade).

**Limitation category:** Contract dependency cascade — existing `requires` contracts
on helper functions create obligations that propagate upward to callers, requiring
all callers to also have contracts.

### 4. `lex` (main entry point) — FAILED (C codegen error)

**Contract attempted:**
```
while pos < src_len vow {
    invariant: pos >= 0
} {
```

**Result:** ESBMC parsing error. The generated C verification code has type
mismatches: String variables typed as `int64_t` have struct member access
(`v24.len`, `v24.data[v24.len]`). The `push_byte` method on String objects
generates invalid C code for ESBMC — the String ESBMC model doesn't handle
`push_byte` correctly within a loop body.

Specific errors:
- `member reference base type 'int64_t' is not a structure or union` (20+ instances)
- `assigning to '__vow_string_t' from incompatible type 'int64_t'`

**CEGIS iterations:** 1 (infrastructure error, not a contract issue).

**Limitation category:** ESBMC String model — `push_byte` method generates invalid
C code in the verification harness.

## ESBMC Limitations Encountered

### 1. Cross-Module Function Inlining
ESBMC cannot verify properties that depend on the return values of many cross-module
functions. The `keyword_tag` function calls 30+ `tok_*()` functions from `token.vow`,
all returning non-negative constants. ESBMC's bounded model checking cannot inline
all of them within its unwind budget.

**Impact:** Any function whose postcondition depends on cross-module function return
values cannot be verified.

### 2. Contract Dependency Cascade
Existing `requires` contracts on helper functions create upward obligations. When
`try_suffix` calls `is_ident_cont(src.byte_at(pos + 2))`, the `requires: b >= 0,
b <= 255` on `is_ident_cont` forces ESBMC to prove `byte_at` returns [0, 255].
Without a model for `byte_at`'s range, verification fails.

**Impact:** Adding contracts to mid-level functions requires either (a) contracts on
all callers to establish preconditions, or (b) ESBMC models for all builtin methods
(e.g., `byte_at` always returns [0, 255]).

### 3. String Model Incompleteness
The `push_byte` method on String objects generates invalid C code in the ESBMC
harness. String variables are represented as `int64_t` in some IR paths but the
`push_byte` model accesses `.len` and `.data[]` as struct fields.

**Impact:** Any function using `push_byte` (common in lexers/parsers) cannot be
verified at all. This is the most severe limitation for compiler-module-scale
verification.

## What Worked

1. **Simple leaf functions** with no cross-module calls verify cleanly
   (`keyword_bool_val`).
2. **Pre-existing helper contracts** (is_whitespace, is_alpha, etc.) all pass —
   these are pure, single-expression functions with byte-range preconditions.
3. **Compilation is not affected** — contracts are purely additive. `vow build
   --no-verify` succeeds, and the self-hosted compiler produces correct output.
4. **CEGIS diagnostics** provided clear counterexamples and error messages,
   enabling rapid diagnosis of each failure mode.

## What Broke and How It Was Fixed

No code was broken. Failed contracts were removed after recording results. The only
contract retained is `keyword_bool_val`'s `ensures: result == 0 || result == 1`,
which verified successfully.

## Recommendations for Reaching Full Level 3

1. **Add `byte_at` range model to ESBMC:** `byte_at(i)` should assume
   `result >= 0 && result <= 255` when `i >= 0 && i < len`. This unblocks
   `try_suffix` and many other functions.
2. **Fix String `push_byte` C codegen:** The ESBMC String model needs consistent
   typing — either all `int64_t` (opaque) or all `__vow_string_t` (struct), not a
   mix.
3. **Add function summary support:** Instead of inlining all 30 `tok_*()` calls,
   ESBMC could use their `ensures` contracts as summaries. If each `tok_*()` had
   `ensures: result >= 0`, `keyword_tag` could verify without inlining.
4. **Propagate contracts bottom-up:** Start with leaf functions, add `ensures`
   contracts, then use those as assumptions when verifying callers.

## Final State

| Function | Contract | Verification |
|---|---|---|
| `is_whitespace` | `requires: b >= 0, b <= 255` | Verified |
| `is_alpha` | `requires: b >= 0, b <= 255` | Verified |
| `is_digit` | `requires: b >= 0, b <= 255` | Verified |
| `is_ident_start` | `requires: b >= 0, b <= 255` | Verified |
| `is_ident_cont` | `requires: b >= 0, b <= 255` | Verified |
| `suffix_len` | `requires: suffix >= -1, ensures: result >= 0, result <= 5` | Verified |
| **`keyword_bool_val`** | **`ensures: result == 0 \|\| result == 1`** | **Verified (new)** |
| `keyword_tag` | *(none — removed after failure)* | Spurious counterexample |
| `try_suffix` | *(none — removed after failure)* | Requires cascade |
| `lex` | *(none — removed after failure)* | C codegen error |
| `empty_str` | *(none)* | N/A |
