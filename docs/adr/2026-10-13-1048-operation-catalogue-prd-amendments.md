# The Operation Catalogue amends its PRD: checked docs, lookup projections, two verifier categories, an explicit exclusion list

**Status:** accepted (2026-10-13)

Part of epic #375 (PRD: Operation Catalogue for builtin and runtime
operations). Builds on
[ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md)
(the native verifier is exempt from dual-compiler parity; `c_emitter.{rs,vow}`
keep byte-identical C until epic #1398 P6).

## Context

The PRD in #375 was written before any slice landed. Five slices have now
merged (print, filesystem/stdin/args/stderr, process, query/utility, and the
validation/docs hardening), and the catalogue covers 83 Builtin Operations plus
the `arena_routes` section. Building it showed that four PRD decisions were
either wrong or answered a narrower question than they appeared to. The epic
can only close with no gaps if each such decision is either implemented or
amended out of the PRD, with the reason recorded and a check that holds the
replacement in place. This ADR is that record. It changes no code.

Throughout, "the type-checker tables" means
`vow-types/src/env.rs::builtin_free_fn_signatures()` and its self-hosted
counterpart in `compiler/env.vow`.

## Decision

Each item names what it amends, what replaces it, and the check that enforces
the replacement. A check marked *pending* belongs to a sibling slice of #375
that was open when this ADR was written; the amendment stands either way, and
the item is closed by that slice.

### (a) Docs are checked against the catalogue, not generated from it

**Amends** user stories 12 ("help output consumes catalogue facts") and 29
("generated builtin tables come from structured facts"), the implementation
decisions "the schema must carry human-readable short descriptions to
regenerate grammar rows and help losslessly" and "allow the help generator to
consume catalogue facts", and the testing decision "help-generation checks
showing that builtin help output is generated from the catalogue".

`docs/spec/grammar.md` is the canonical prose spec, and
`scripts/generate_help.py` already derives `--help` and the embedded skill
from it into `vow/src/skill.rs` and `compiler/main.vow`. The Builtin Function
Signatures table is not a flat list: it is split into subsections with
explanatory prose and per-builtin semantics notes, none of which belong in a
machine catalogue. Generating those rows from the catalogue would force the
catalogue to carry the prose (the PRD itself warned that it must not become a
substitute for the Markdown specs) or would split one table across two owners.

The catalogue therefore carries `doc_signature` and `effects` only as facts to
check. `check_doc_facts` in `scripts/generate_operations.py` requires, for
every catalogued op, a matching grammar.md table row (signature and effects), a
help line in `vow/src/skill.rs` and a help line in `compiler/main.vow`, and
reports every mismatch at once. `scripts/check_help_coverage.py` independently
checks grammar.md features against the `--help` JSON of both compilers, and
`generate_help.py --check` keeps the generated help in step with grammar.md.

**Rejected:** generating grammar.md table rows and the help text from the
catalogue.

**Enforced by:** `generate_operations.py --check` (CI step "Operation
Catalogue projection freshness" and `full_test.sh` gate `ops/catalogue-drift`),
`test_real_catalogue_doc_facts_are_consistent` in
`scripts/test_generate_operations.py`, and the `check_help_coverage.py` runs in
`full_test.sh`.

### (b) Rust projections are lookup functions and tables, not typed records

**Amends** user story 13 ("generated Rust projections use typed records and
enums") and the implementation decision "use typed records and enums for Rust
Operation Projections".

Every consumer of an operation fact asks one narrow question about a name or a
runtime symbol: which runtime symbol does this builtin lower to, with what
return type; what is this symbol's Cranelift extern signature; what result tag
does this call carry; which arena variant does this symbol have. The generator
emits one function per question, spliced between `GENERATE:OPERATIONS`
markers:

- `vow-ir/src/lower/mod.rs`: `catalogue_builtin_to_runtime` and
  `catalogue_builtin_result_tag`, a `match` returning the existing
  `BuiltinResultTag` enum;
- `vow-codegen/src/cranelift_backend.rs` and `vow-clif-shim/src/lib.rs`:
  `catalogue_extern_sig`, a `match` over runtime symbols;
- `vow-ir/src/region.rs` and `vow-clif-shim/src/lib.rs`: the
  `FRESH_ARENA_VARIANTS` table with hand-written lookups over it;
- `vow-verify/src/c_emitter.rs`: `catalogue_c_model_known`.

A typed `Operation` record would be a second wide interface every caller goes
through, to answer questions each of which needs one field. It would also have
made the Rust projections a different shape from the self-hosted ones
(`compiler/lower.vow`, `compiler/ir.vow`, `compiler/vc_ops.vow` and
`compiler/c_emitter.vow` get string-keyed decision trees, the shape the
`lower.vow` if-chains already had), which is the opposite of the parity this
epic exists to give. Type-safety of the facts is not lost: `load_catalogue`
rejects an unknown field, an unknown return, effect, arena-routing or model
token, a duplicate name or symbol, and a `doc_signature` that disagrees with
`params`/`return`, collecting every violation before failing, and the Rust
result tag is a closed enum.

**Rejected:** generated typed `Operation` records in Rust.

**Enforced by:** `LoadCatalogueTest` and the `Gen*BlockTest` classes (rendered
text, determinism) in `scripts/test_generate_operations.py`, plus
`generate_operations.py --check` for freshness of every splice.

### (c) The verifier fact is two categories, and only pure ops may be known

**Amends** user stories 8 and 27 ("the verifier-model category replaces the
hand-mirrored `is_known_builtin`/`is_modelable` classifiers") and the
implementation decision "the first verifier projection exposes verifier-model
categories". `is_modelable` itself stays a structural check on a whole function
(effects, recursion, reserved symbols, and the callees it reaches); only the
per-operation known-symbol set it consults is a catalogue fact.

The PRD assumed one classifier. There are two consumers with different
lifetimes, so the catalogue holds two independent fields:

| Field | Consumer | Values | Lifetime |
|---|---|---|---|
| `c_model` | the ESBMC C model: `catalogue_c_model_known` in `vow-verify/src/c_emitter.rs` and `compiler/c_emitter.vow` | `unmodeled`, `base`, `base_and_arena` (the `_in_arena` variant is known too) | byte-identical C parity until epic #1398 P6 deletes both emitters |
| `verifier_model` | the native verifier: `catalogue_verifier_known` in `compiler/vc_ops.vow`, read by the gate in `compiler/vc_gate.vow` | `known`, `unmodeled` | exempt from parity per ADR-2026-10-08-1421 |

They differ on purpose. A `known` native entry changes the skip code of every
caller of that builtin, so it may only be set together with a symbolic
executor model; the C model's answer is the stage 0 verifier's, and goes away
with it. Folding them into one field would either make the native verifier
inherit the C model's list (it models none of them today, so
`catalogue_verifier_known` generates `return false`) or leave the C model's
hand-mirrored copies in place.

Only pure builtins may be known to either model. The C model's `is_modelable`
rejects any function with a non-empty effect set before it consults the
classifier. The native gate (`vc_skip_reason_with`, via `vc_effects_modeled`)
rejects every effect set except `{panic}`, which it admits for `.unwrap()`; it
then consults the classifier for each call in such a function. No catalogued
builtin carries `panic` today, so no effectful op is reachable through either
classifier, but that rests on the catalogue's contents, not on the gate: a
`known` effectful entry is a claim no symbolic executor model has been written
for, which is why the rule is stated on the catalogue and not left to the gates.
Every entry must decide explicitly: `check_verifier_models` rejects an op with
no `verifier_model`, and `check_c_models` rejects a pure op with no `c_model`,
an effectful op with `base`/`base_and_arena`, and `base_and_arena` on a symbol
with no arena route.

**Rejected:** a single `verifier_model` field generating both the C-model and
native classifiers.

**Enforced by:** `check_verifier_models` and `check_c_models` (run on every
invocation, including `--check`), with
`test_undecided_builtin_exits_cleanly_naming_verifier_model`,
`test_pure_builtin_without_c_model_is_reported_by_name`,
`test_effectful_builtin_cannot_be_known_to_the_c_model`,
`test_arena_variant_needs_a_route` and
`test_real_catalogue_has_a_decision_for_every_pure_builtin` in
`scripts/test_generate_operations.py`, and `scripts/parity.py c` over
`tests/verify*/` in `full_test.sh` Section 2c for the emitted C. The same
purity rule for `verifier_model: known`, and the controls that prove it, are
*pending* (the verifier-model purity-invariant slice).

### (d) The catalogue has an explicit exclusion list

**Amends** the PRD's narrower "Out of Scope" list (`pin_to_root`,
`string_matches_literal_at`, constructors and methods), which left the rest of
the type-checker table's builtins neither catalogued nor declared otherwise,
so a builtin missing from the catalogue was indistinguishable from a forgotten one. Slice
by slice, the catalogue grew to 83 operations, covering 79 of the 96 names the
type-checker table lists literally and the four `parse_<narrow int>` builtins
it generates. The remaining literal names, and the families the table
generates in loops, are deliberately not catalogued. That is to be recorded (by the
coverage slice named below) as an `excluded` list in
`docs/spec/operations.json`, each entry with a reason, so that every
builtin name known to the type checker is either catalogued or excluded with a
stated reason. The reasons:

| Excluded | Why it is not a direct Builtin Operation fact |
|---|---|
| Narrowing-conversion families: `<src>_to_<dst>_{try,wrap,sat}` for u8, i8, i16, u16, i32, u32 targets, the 128-bit same-width sign changes `i128_to_u128_{wrap,sat}` and `u128_to_i128_{wrap,sat}` (no `_try`), and `add_sat_u8`, `sub_sat_u8`, `mul_sat_u8` | A combinatorial family the type-checker table builds in loops, lowered by hand-written arms after the catalogue lookup in `vow_static_builtin_to_runtime`. Listing every member would copy the family's structure into JSON; they need a family mechanism, not 100+ rows. The C emitter keeps its hand-written arms for them, outside the generated `catalogue_c_model_known`. |
| `__vow_clif_*` (create, add_string, declare_extern, declare_function, fn_begin, fn_block, fn_inst, fn_vow, fn_end, finish, link, destroy) | The internal FFI of the self-hosted compiler to `vow-clif-shim`, not user surface (absent from grammar.md). Their ABI is owned by the shim crate, which is the consumer rather than a projection target. |
| `pin_to_root` | A compiler intrinsic monomorphised per argument type with a hand-written lowering; it consumes the result-tag projection rather than being described by it. It is special-cased in the checkers (`vow-types/src/check.rs`, `compiler/checker.vow`), not listed in the type-checker tables, so it is documented here rather than in the gate's scope. |
| `string_matches_literal_at` | Special lowering that rewrites the literal argument into a data pointer plus length before calling the runtime. |
| `debug_str`, `debug_i64`, `debug_u64` | Pure by effect, but lowered to the `DebugCall` opcode (`vow_debug_builtin_to_runtime`), not the generic call path the catalogue projects. They are dropped in release and profile modes and are no-ops in verification, so they have no ABI or verifier fact to record. |
| `i64_to_string` | A hand-written alias of the catalogued `int_to_string`; a second entry would need a second runtime symbol claim for the same `__vow_string_from_i64`, which `load_catalogue` rejects as a duplicate. |
| Internal runtime ops (`__vow_vec_*`, `__vow_string_eq`, the `_in_arena` variants and similar) | Runtime Operations with no surface name, emitted by lowering rather than called from source. They are not in the type-checker tables, so there is no builtin to catalogue and they are documented here only, not entered in `excluded`; the `_in_arena` variants of the catalogued fresh-aggregate builtins are derived from `arena_routes`. |

The exclusion does not move any fact. The type-checker tables remain the
single source for signatures and effects (this is user story 5, unchanged);
the catalogue's `doc_signature` and `effects` are copies checked against
grammar.md and the help text by (a). Today nothing compares them with the
type-checker tables, and the purity rules of (c) read the catalogue's `effects`,
so the coverage gate must also close that gap (see its requirement below). A
builtin enters the catalogue by an entry in `operations`, leaves the exclusion
list by deleting its line, and is never in both.

**Enforced by:** the coverage gate and its generator unit tests (work item
`ops-catalogue-coverage-and-type-env-tie`, *pending*). The requirement on that
slice: the gate must fail when a name in either compiler's type-checker table
is neither an `operations` entry nor an `excluded` entry, when a name is in
both, when an `excluded` entry has no reason, and when an `operations` entry's
`doc_signature` or `effects` disagrees with the type-checker table's signature
for that name in either compiler.

## Consequences

- The PRD's amended stories (8, 12, 13, 27, 29) are amended here rather than
  reimplemented, and the catalogue's shape is what the generator validates, not
  what the PRD sketched. The remaining implementation is tracked in #1274 and
  the pending slices named above. Stories 5 and 28, which the
  amendments could be mistaken for touching, are unchanged.
- `CLAUDE.md`'s Operation Catalogue section is updated in the same change to
  describe the current scope, fields and gates, so that it stops describing
  the print-only tracer bullet.
- When epic #1398 P6 deletes `c_emitter.{rs,vow}`, `c_model` and its two
  generated targets are deleted with them; `verifier_model` stays.
- Extending the catalogue to a family (for example the narrowing conversions)
  is a new decision that supersedes the corresponding row of (d); it is not
  covered by this ADR.
