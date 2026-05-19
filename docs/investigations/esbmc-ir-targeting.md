# Investigation: Targeting ESBMC IR Directly (Bypassing C Frontend)

**Date:** 2026-03-20
**Status:** Investigation complete, recommendation ready

## Current State

Vow currently verifies programs by:

1. Lowering Vow source to **vow-ir** (Pizlo-style SSA)
2. Emitting **C code** from vow-ir (`c_emitter.rs`, ~1300 lines)
3. Writing C to a temp file and invoking **ESBMC as a subprocess**
4. Parsing ESBMC's stdout for `VERIFICATION SUCCESSFUL/FAILED` and counterexample text

The C emission models collections with fixed-size arrays (`Vec` → `int64_t data[128]`,
`String` → `int8_t data[256]`, `HashMap` → 64-entry parallel arrays) and uses
`__ESBMC_assume`/`__ESBMC_assert` intrinsics for contracts. This works but introduces
artificial constraints and a lossy type mapping (all Vow types squeezed through C's
type system).

## ESBMC Architecture

ESBMC's internal pipeline is:

```
Source → Language Frontend → Symbol Table (irep) → GOTO Program → SymEx → SSA → SMT → Result
```

Key facts:

- **irep** is a tree-structured IR node type. Each node has an ID string and child nodes.
  There are two variants: `irep` (string-based, older, easier to use) and `irep2`
  (template-based, newer). Conversion utilities exist between them.
- **GOTO programs** are simplified CFGs: assignments, conditional/unconditional GOTOs,
  assumes, and asserts. Variables are in a symbol table, not SSA.
- **The irep/GOTO format is largely undocumented.** ESBMC's own docs acknowledge this:
  "The irep/structure of this data is entirely undocumented and closely coupled between
  the parsers and the goto-programs dir."
- ESBMC supports multiple frontends: C/C++ (via Clang), Python, Solidity, Java, Kotlin,
  and Rust (via goto-transcoder). Each frontend constructs irep nodes via C++ API calls.

## Three Approaches Investigated

### Approach A: Emit GOTO Binary Format (GBF) Directly

**What:** Serialize vow-ir into ESBMC's GOTO binary format (`.goto` file), then invoke
`esbmc --binary file.goto`.

**How GBF works:**
- Magic number `0x7f GBF`, version 4
- Symbol table: each symbol has irep-encoded type, value, location, plus name/module/flags
- Functions section: serialized GOTO instruction sequences
- 7-bit varint encoding for integers, escaped null-terminated strings, hash-based dedup

**Pros:**
- No C intermediate representation at all
- Full control over types — no need to squeeze Vow types through C
- Collection models can be more precise (not limited by C array syntax)
- Counterexample mapping would be direct (no C line-number indirection)

**Cons:**
- GBF format is undocumented and tightly coupled to ESBMC internals
- CBMC and ESBMC GBF formats diverge (the goto-transcoder project exists specifically
  to convert between them, and was archived in Feb 2026)
- Extremely brittle: any ESBMC version bump could break the binary format silently
- Must replicate ESBMC's symbol table construction, type encoding, and GOTO instruction
  encoding in Rust — a large, error-prone surface area
- No stability guarantees on the format

**Verdict: High risk, high maintenance burden. Not recommended as primary approach.**

### Approach B: Build a Custom ESBMC Frontend (C++ Library Linking)

**What:** Write a Vow-to-ESBMC frontend in C++ that uses ESBMC's internal irep API to
construct the symbol table and GOTO programs programmatically, then hand off to ESBMC's
SymEx/SMT pipeline.

**How other frontends do it (Python, Solidity):**
1. Parse source into a JSON AST
2. Annotate with types
3. Use C++ irep API to populate a symbol table (assignments, expressions, conditionals,
   loops, functions)
4. ESBMC's existing pipeline converts symbol table → GOTO → SymEx → SMT

**Pros:**
- Uses ESBMC's own API — the "supported" way to add a language
- Full access to ESBMC's type system, which is richer than C's
- Can model Vow-specific constructs (linear types, effects, blame) natively
- Collection models could use ESBMC's dynamic array reasoning rather than fixed bounds

**Cons:**
- Requires building against ESBMC as a C++ library (not just a CLI tool)
- ESBMC does not publish a stable C++ API or provide linkable libraries in releases
- The irep API is undocumented — you learn it by reading ESBMC source code
- Tight coupling to ESBMC internals; version upgrades are painful
- Significant C++ development effort for the frontend
- Would need an FFI bridge from Rust (or rewrite the frontend in C++)
- ESBMC's Python and Solidity frontends are maintained by the ESBMC team themselves;
  an external frontend would not get the same maintenance attention

**Verdict: Correct long-term architecture but premature. Requires ESBMC team collaboration.**

### Approach C: Emit Optimized C with ESBMC-Specific Extensions (Incremental Improvement)

**What:** Keep the C emission strategy but make it smarter, exploiting ESBMC-specific
features beyond basic `__ESBMC_assume`/`__ESBMC_assert`.

**Specific improvements:**
1. **Use `__ESBMC_assert` with structured metadata** — current counterexample parsing is
   fragile text scraping; ESBMC supports property labels that could carry richer Vow metadata
2. **Use ESBMC's `--overflow-check`** for checked arithmetic instead of manually modeling it
3. **Use `__ESBMC_atomic_begin`/`__ESBMC_atomic_end`** for future concurrency verification
4. **Switch from fixed arrays to `__ESBMC_assume`-bounded dynamic allocation** — instead of
   `int64_t data[128]`, use `malloc` + `__ESBMC_assume(len <= N)` for more flexible bounds
5. **Use `--ir` flag** to output ESBMC's internal GOTO representation for debugging
6. **Use `--smt-during-symex`** and incremental SMT for better performance on complex contracts
7. **Emit cleaner C** — the current emitter pre-declares all variables at function scope to
   avoid goto/scope issues; this could be improved with block-scoped declarations where safe

**Pros:**
- Minimal disruption — evolves the working system incrementally
- C remains a well-understood, stable interface to ESBMC
- ESBMC's C frontend (Clang-based) is the most mature and best-tested path
- Easy to debug (you can read the generated C)
- No dependency on ESBMC internals or build system

**Cons:**
- Still constrained by C's type system (no linear types, no effects, no blame in the type)
- Collection models still need fixed bounds (though bounds can be more flexible)
- Still doing text-based output parsing (though this can be improved)

**Verdict: Best near-term approach. Maximizes value with minimal risk.**

## Recommended Strategy: Phased Approach

### Phase 1: Optimize C Emission (Now)

Improve the existing `c_emitter.rs`:
- Replace fixed-array collection models with `__ESBMC_assume`-bounded dynamic models
- Add structured property labels to `__ESBMC_assert` for better counterexample mapping
- Leverage ESBMC-specific flags (`--overflow-check`, `--smt-during-symex`)
- Clean up variable declarations where possible

This is pure improvement with no architectural risk.

### Phase 2: Propose Upstream ESBMC API (Medium-term)

Engage with the ESBMC team (they're at University of Manchester / Federal University of
Amazonas) to propose a stable verification API. The ideal interface would be:

```
// Hypothetical ESBMC verification API
esbmc_context_t *ctx = esbmc_create_context();
esbmc_add_type(ctx, "VowVec", ...);
esbmc_add_function(ctx, "divide", params, body_as_goto_instructions);
esbmc_add_assume(ctx, precondition);
esbmc_add_assert(ctx, postcondition, "vow:0");
esbmc_result_t result = esbmc_verify(ctx, options);
```

This would be a C API (like Cranelift's, which Vow already wraps via `vow-clif-shim`)
that exposes GOTO program construction without requiring knowledge of irep internals.
The goto-transcoder project (now under esbmc org) demonstrates there's interest in
making GOTO programs a first-class interchange format.

**This requires ESBMC team buy-in.** It's worth proposing because:
- Vow is a concrete use case for "verification as a service"
- The ESBMC team is actively expanding language support (Rust, Python, Solidity)
- A stable API would benefit all external integrators, not just Vow

### Phase 3: Direct GOTO Emission (Long-term, contingent on Phase 2)

Once a stable API exists, replace `c_emitter.rs` with a `goto_emitter.rs` that:
- Maps vow-ir instructions directly to GOTO program instructions
- Constructs the symbol table via API calls (analogous to `vow-clif-shim` for Cranelift)
- Passes the GOTO program to ESBMC's verification engine
- Receives structured results (not text parsing)

The architecture would mirror Vow's existing Cranelift integration:

```
vow-ir ─┬─→ vow-clif-shim (FFI) → Cranelift → native code
         └─→ vow-esbmc-shim (FFI) → ESBMC API → verification result
```

## Why Not Jump Straight to GOTO Programs?

1. **The format is unstable and undocumented.** Even the goto-transcoder project (which
   exists solely to handle GOTO format conversion) was archived, suggesting the format
   is a moving target.

2. **ESBMC's C frontend is its best-tested path.** Every ESBMC release is validated
   against C benchmarks (SV-COMP). The Python/Solidity frontends are newer and less
   battle-tested. Going through C means Vow benefits from ESBMC's most mature code path.

3. **The C intermediate form is debuggable.** When verification produces unexpected
   results, inspecting the generated C is straightforward. Inspecting a binary GOTO
   program requires ESBMC-specific tooling.

4. **The current pain points are solvable within C.** Fixed collection bounds, imprecise
   counterexample mapping, and limited ESBMC flag usage are all addressable without
   changing the fundamental architecture.

5. **Vow's verification needs are modest.** Vow programs are small, contracts are
   first-order, and the verification scope is bounded. The overhead of C emission +
   Clang parsing inside ESBMC is negligible compared to SMT solving time.

## Appendix: ESBMC Ecosystem References

- [ESBMC repository](https://github.com/esbmc/esbmc)
- [ESBMC ARCHITECTURE.md](https://github.com/esbmc/esbmc/blob/master/ARCHITECTURE.md)
- [ESBMC Python frontend](https://github.com/esbmc/esbmc/blob/master/src/python-frontend/README.md)
- [goto-transcoder (archived, moved to esbmc org)](https://github.com/esbmc/goto-transcoder)
- [CBMC goto-programs documentation](https://diffblue.github.io/cbmc/group__goto-programs.html)
- [CBMC central data structures (irep)](https://diffblue.github.io/cbmc/central-data-structures.html)
- [ESBMC-Python paper (ISSTA 2024)](https://arxiv.org/html/2407.03472v1)
- [ESBMC-Solidity paper](https://arxiv.org/pdf/2111.13117)
- [Rust Foundation: ESBMC in Rust ecosystem](https://rustfoundation.org/media/expanding-the-rust-formal-verification-ecosystem-welcoming-esbmc/)
- [GOTO Transcoder proposal for verify-rust-std](https://github.com/model-checking/verify-rust-std/issues/108)
