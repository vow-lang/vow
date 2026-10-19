# Vow repository guidance

Paths and commands below are relative to the repository root.

## Operation Catalogue

`docs/spec/operations.json` (assembled by `scripts/spec_assemble.py` from the authored text fragments
`docs/spec/src/operations.json/*.part`; edit the fragments, then run `generate_operations.py`, which assembles
first) is the Operation Catalogue: the single checked, hand-edited source of
runtime-symbol, ABI, return-shape, and doc facts for the Builtin Operations in its `operations` array
(the print, filesystem, stdin/args/stderr, process, hash, environment and query/utility builtins; epic
#375), plus the top-level `arena_routes` list and the `excluded` section. It is deliberately narrower than
`vow-types/src/env.rs::builtin_free_fn_signatures()` — that table (and its self-hosted counterpart in
`compiler/env.vow`) remains the single source for signature/effects and is never read or replaced by
this catalogue; the entry's `doc_signature`/`effects` are copies checked against `grammar.md` and the
help text, not a second source. `cargo test -p vow-types` and
`build/vowc test compiler/ --filter test_op_catalogue_env` compare them with both type-checker tables, via
fact tables generated into `vow-types/src/env.rs` and `compiler/tests/catalogue_facts.vow`. They sit
between their own `// GENERATE:OPERATION_FACTS:START` / `:END` markers, separate from the
`GENERATE:OPERATIONS` pair below, and `generate_operations.py --check` fails when they are stale.

Where the shipped catalogue departs from the epic's PRD (docs checked rather than generated, lookup
functions rather than typed Rust records, the `c_model`/`verifier_model` split, the exclusion list),
[ADR-2026-10-13-1048](../adr/2026-10-13-1048-operation-catalogue-prd-amendments.md) records the
amendment and the check that enforces its replacement.

`scripts/generate_operations.py` regenerates and validates the catalogue's projections:

```bash
python3 scripts/generate_operations.py          # regenerate all projections
python3 scripts/generate_operations.py --check  # validate without writing (CI-gating)
```

It splices generated lookup functions between `// GENERATE:OPERATIONS:START` / `:END` markers — the
same marker convention `scripts/generate_help.py` uses for `// GENERATE:SKILL_*` — into:

- `vow-ir/src/lower/mod.rs` (`catalogue_builtin_to_runtime`)
- `vow-codegen/src/cranelift_backend.rs` and `vow-clif-shim/src/lib.rs` (`catalogue_extern_sig`)
- `compiler/lower.vow` (`catalogue_builtin_to_extern`, `catalogue_builtin_ret_ty`)
- `compiler/vc_ops.vow` (`catalogue_verifier_known`, the native verifier's
  builtin model keyed by runtime symbol; `generate_operations.py` fails when an entry has no `verifier_model`
  or an effectful entry is `known`;
  `known` alone does not make a call verifiable until the executor encodes calls)
- `vow-verify/src/c_emitter.rs` and `compiler/c_emitter.vow` (`catalogue_c_model_known`, the ESBMC C
  model's known-symbol set, from the `c_model` field: `base` knows the runtime symbol, `base_and_arena`
  also its `_in_arena` variant, `unmodeled` neither; required on every pure (`[]`) entry, and only a pure
  entry can be known. It is separate from `verifier_model` on purpose: a `known` native entry changes
  the native skip code of every caller, and the C model goes away with the ESBMC path in epic #1398, P6.
  The hand-written `is_known_builtin` lists keep only what is not catalogued)
- `vow-ir/src/region.rs`, `vow-clif-shim/src/lib.rs` and `compiler/ir.vow` (the `arena_routes` section, a
  plain list of the base symbols of every fresh-aggregate builtin; the variant is always
  `<symbol>_in_arena`. Rust gets the `FRESH_ARENA_VARIANTS` table with hand-written lookups over it, Vow
  gets the `fresh_arena_base_extern` decision tree, and `fresh_arena_variant_extern` strips the suffix)

A call's result shape tag (`String`, `Vec` or `Option<T>` heap aggregate, which `pin_to_root` and the
arena inference read) is derived from the entry's documented return type, not stored: the Rust
`catalogue_builtin_result_tag` and the self-hosted `catalogue_builtin_result_tag` are generated, and
`arena_routing: heap_fresh` only records (and the generator validates) that the symbol has an
`_in_arena` variant in `arena_routes`. Every op returning a `String`, `Vec` or `Option` must declare
`arena_routing` (`heap_fresh` exactly when its symbol is a route; otherwise `none` plus a free-text
`arena_routing_reason` saying why the heap result has no arena variant; the ADR above lists today's one, `stdin_read_line`),
and `--check` fails on any disagreement. `i64_to_string`,
`string_matches_literal_at` (its literal operand lowers specially) and the narrowing intrinsics stay
hand-written.

`docs/spec/grammar.md`'s Builtin Function Signatures table and the generated help-JSON in
`compiler/main.vow`/`vow/src/skill.rs` are **checked against** the catalogue (not generated from it) —
`--check` fails if any drift (`check_doc_facts` compares each entry's signature and effects with the
grammar.md row and with the `skill.rs` and `main.vow` help lines).

`c_model` (ESBMC C model, parity until epic #1398 P6) and `verifier_model` (native verifier, exempt
from parity) are separate per-entry fields, and only pure entries may be known to either.
Builtins the type-checker tables hold that the catalogue deliberately does not describe (narrowing-conversion
families and the saturating `u8` operations, `__vow_clif_*`, `string_matches_literal_at`, the `debug_*` builtins, the
`i64_to_string` alias) are listed with a reason in the `excluded` section of
`operations.json`, each entry an explicit `names` list, a `prefix`, or a `contains` plus `suffixes` set
(no regex). `load_excluded` rejects a missing reason, a malformed matcher and a
name that is both catalogued and excluded; the vow-types and `test_op_catalogue_env` tests fail on a table entry
that is neither catalogued nor excluded and on an exclusion that matches no table entry. `pin_to_root` and the
internal runtime ops are not table entries, so they are documented in the ADR above only.

`load_catalogue` hand-validates every entry beyond the four fields' presence and type: `effects` must
match `^\[(token(, token)*)?\]$` against the closed vocabulary `{read, write, io, panic, unsafe}`, with
tokens in alphabetical order (matching `vow-syntax/src/printer.rs`'s `print_effects`, which sorts the
same way); `doc_signature` must parse as `fn(...) -> Type` with arity and
per-parameter/return types matching `params`/`return` (`String`/`Vec<...>` both denote `ptr`); the
`verifier_model` (`known`/`unmodeled`; `main()` rejects an entry without one, and an effectful entry
marked `known`), the `arena_routing` (`none`/`heap_fresh`; `main()` rejects a heap-returning entry without
one) and `c_model` (`unmodeled`/`base`/`base_and_arena`; `main()` rejects a pure entry without one)
fields, when present, must be in their closed sets; a pure `unmodeled` entry must carry a non-empty
free-text `verifier_skip_reason` (which semantics the native verifier does not model; `main()` rejects
its absence, and its presence on any other entry); an `Option<T>` return type
needs a `T` the generator can tag; and any field name not in the recognized set (required or optional)
is rejected outright, so a misspelled field can never silently defeat its own check. All
violations across all entries are collected and reported together, not just the first. `main()` never
lets a bad catalogue surface as a raw Python traceback — a malformed `operations.json` or a rejected
entry prints a clean message to stderr and exits 1. `scripts/full_test.sh` runs
`generate_operations.py --check` as its `ops/catalogue-drift` gate (Section 8), alongside CI's
`ci.yml` step, so staleness is caught locally too, not only on push. The generator's own unit tests
(`scripts/test_generate_operations.py`) run in both `ci.yml` and `full_test.sh` (gate
`ops/catalogue-generator-tests`, right after `ops/catalogue-drift`).

All the catalogue slices of epic #375 (#1270–#1275) have merged. Further work on the catalogue should
add entries, fields and target files to the existing generator rather than inventing a new mechanism.
