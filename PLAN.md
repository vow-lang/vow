# Plan: #1353 — self-hosted diagnostics on linear/module/string-literal checks still report offset 0

## 1. Problem restated

`scripts/parity.py` compares the Rust and self-hosted compilers' structured diagnostics for every
`tests/error/*.vow` fixture. For 15 fixtures the self-hosted compiler still reports `offset 0,
length 0` for a diagnostic the Rust compiler locates correctly, and these 15 are carved out via
`SPANLESS_SELF_FIXTURES` in `scripts/parity.py` so the gate doesn't fail on them. Root-cause reading
of both compilers shows **the Rust compiler is already correct in every one of the 15 cases** — the
defect is entirely in the self-hosted compiler's span bookkeeping, split across four independent
causes: (a) synthetic `Upsilon`/`LinearConsume` instructions inserted by the self-hosted lowerer for
implicit linear-value consumption never receive a patched span, (b) synthetic `Phi` instructions
created for mutation-merging across `if`/`while`/`for`/`match` branches never receive a patched span,
(c) the self-hosted AST's `Module.uses` list captures no span at all for `use` declarations, and (d)
the `push_byte` string method's dispatch table entry has `patches_span = false`, so its `IOP_CALL`
result (the fallback span source for the region-literal-mutation check) also stays at `0,0`. Because
Rust is already correct, this is a self-hosted-only bug-parity fix, not a language-semantics change —
no Rust crate changes and no `docs/spec/*.md` changes are required; the documented contract (e.g.
`docs/spec/errors.md`'s `IoError` section) already describes the Rust behavior as the intended one.

The 47 fixtures noted in the issue as disagreeing on *anchor* (non-zero span in both compilers, but a
different node) are explicitly out of scope for this issue — they are not in `SPANLESS_SELF_FIXTURES`
and are not touched here.

## 2. Files to touch

All production changes are in the self-hosted compiler (`compiler/`). No Rust crate changes. No
`docs/spec/*.md` changes (the Rust-documented contract is already correct; self-hosted is catching
up to it, not changing behavior).

- `compiler/lower.vow`
  - `lctx_emit_linear_consume_if_needed` (~line 343) and its ~6 call sites (field-store RHS ~3598,
    index-store RHS ~3626, match wildcard/ident-arm scrutinee discard ~4274/4369/4422,
    `lower_consumed_expr` ~4805): thread the relevant AST span through so the synthesized
    `IOP_LINEAR_CONSUME` inst gets `lctx_set_inst_span` called on it immediately after `lctx_emit`,
    mirroring the existing `lctx_set_inst_span(ctx, result, expr_span(a, eid))` idiom used elsewhere
    in this file.
  - Every `IOP_UPSILON` emission site (~30, all currently hardcoded to an implicit `0,0` span since
    `lctx_emit` has no span parameter — see Slice 0 below) and the mutation-merge/value-merge `Phi`
    emission sites feeding `linear_inst_starts_origin` in `compiler/region.vow`: set a span via
    `lctx_set_inst_span` on whichever of these `linear_emit_consumed_error`
    (`region.vow:434`, triggers on `LINEAR_CONSUME`, `UPSILON`, *or* `RETURN`) and
    `linear_emit_live_errors`/`linear_inst_starts_origin` (`region.vow`, triggers on `GET_ARG`,
    `REGION_ALLOC`, `CALL`, `PHI`, or an owned field transfer) actually anchor on for each of the 11
    fixtures — determined by Slice 0, not assumed in advance. Where Rust already threads a real
    `span` local into the matching `ctx.emit(Opcode::Phi/Upsilon, ..., span)` call in
    `vow-ir/src/lower/mod.rs`, copy that choice verbatim rather than picking an anchor
    independently. **Slices for `LinearTypeViolation` (6 fixtures) and `RegionLinear` (5 fixtures)
    are likely the same underlying fix** (Upsilon/Phi span patching in the same control-flow
    lowering code), not two independent ones — do not assume a clean split until Slice 0 confirms it.
  - `builtin_method_spec` (~line 1621), the `"push_byte"` row: flip the trailing `patches_span`
    argument from `false` to `true`, matching the already-`true` rows (`parse_i64`, `parse_u64`,
    `get`, `contains`). This makes the `IOP_CALL` for `.push_byte(...)` carry `expr_span(a, eid)`,
    which `emit_region_literal_mutation` (`compiler/region.vow` ~2071) falls back to when the field
    target itself (`b.s`) has no span — which it currently doesn't, since `EXPR_FIELD()` lowering
    (~line 3660–3696) never patches a span either. This is a minimal fix sufficient to stop the drop
    (neither fixture is in `STRICT_SPAN_FIXTURES`, so an exact-anchor match with Rust isn't required)
    — see Slice 4's note on verifying against Rust's actual anchor before treating this as final.

- `compiler/ast.vow`
  - `Module` struct (~line 515): add a parallel `uses_spans: Vec<i64>` field (packed spans, same
    encoding as `expr_span`/`stmt_span`), alongside the existing `uses: Vec<i64>`. Update both
    `Module { ... }` literal sites: `compiler/parser.vow:174` (`parse_module_into`'s return value)
    and `compiler/frontend.vow:179` (the merged-module literal in `frontend_prepare_path_traced` —
    this one can pass `Vec::new()` for `uses_spans` since the merged module's own `uses` list is
    already empty at that point and is never read again for module loading).

- `compiler/parser.vow`
  - `parse_module_into` (~line 146): in the `while at(p, tok_kw_use())` loop (~line 155), capture
    `let span_start: i64 = peek(p).span_start;` before `advance(p)` consumes the `use` keyword, and
    push `span_pack_to_here(p, span_start)` onto a new `uses_spans` vec in lock-step with
    `uses.push(path)`. Thread `uses_spans` into the returned `Module { ... }` literal (line 174).

- `compiler/frontend.vow`
  - `load_frontend_deps` (~line 98): add an `importing_file: String` parameter — the path whose
    source the new span's `offset`/`length` index into (`m.uses_spans[ui]` is a span over the
    *importing* file's bytes, not `dep_path`'s). Replace the hardcoded
    `diag_error(EC_IO_ERROR(), ..., dep_path, 0, 0)` at line 109 with
    `diag_error(EC_IO_ERROR(), ..., importing_file, span_start, span_len)`, unpacked from
    `m.uses_spans[ui]` via `span_unpack_start`/`span_unpack_len` (both already defined in
    `compiler/diag.vow`). Using `importing_file` rather than `dep_path` matters beyond the offset
    being meaningless against the wrong file's bytes: `dep_path`'s source is never registered via
    `diag_ctx_add_source` when the read fails (only the success branch registers it), so a diagnostic
    labeled `dep_path` would silently lose its computed `line`/`column` in both the JSON and human
    emitters (`diag_ctx_lookup_source` returns `""` for an unregistered file, and both
    `diag_to_json`/`diag_to_human` in `compiler/diag.vow` skip line/column whenever the looked-up
    source is empty — confirmed safe, not a crash, but degraded output). `importing_file` is always
    already registered by the time `load_frontend_deps` runs on it. Thread the new parameter through
    both call sites: the top-level call in `frontend_prepare_path_traced` (~line 168, pass `path`)
    and the recursive call inside `load_frontend_deps` itself (~line 115, pass `dep_path` — `dep_m`'s
    own `use` declarations are relative to `dep_path`, which becomes *its* dependencies' importing
    file). Fixes the 2 `IoError` module-resolution fixtures (`missing_module`,
    `missing_module_symbol_reference` — both fail at the same `use`-resolution site).

- `scripts/parity.py`
  - Remove each of the 15 fixture names from `SPANLESS_SELF_FIXTURES` (~line 63–81) as its category's
    fix lands and is verified — not all at once, so a partial implementation run still leaves an
    accurate suppression set. `_span_errors` (~line 374) already fails the gate if an entry is fixed
    but not removed, so this removal is required, not optional cleanup.

No Rust crate files change. No `IrModule` wire-format (`compiler/module_io.vow`) change: that format
serializes the lowered IR (`IrModule`), not the parsed AST `Module` — the new `uses_spans` field lives
only on the AST-level `Module` struct in `compiler/ast.vow`, which is never round-tripped through the
binary module cache.

## 3. TDD slices

Setup (once): `cargo build --all --release` (builds `$RUST` = `./target/release/vow`), then
`$RUST --no-verify compiler/main.vow -o $TMPDIR/vowc_self` to get the self-hosted binary (`$SELF`).
Use `VOW_CACHE_DIR=$(mktemp -d)` for every self-hosted invocation below — the compile cache keys on
source content at a fixed compiler revision and silently serves stale objects across a compiler
rebuild at the same source rev (see project memory).

Per-fixture check, matching exactly what `scripts/full_test.sh`'s error-fixture loop
(~line 344–360) does:
```
rust_json=$($RUST build --no-verify tests/error/<fixture>.vow -o $TMPDIR/rust_<fixture> 2>/dev/null)
self_json=$(VOW_CACHE_DIR=$(mktemp -d) $SELF build --no-verify tests/error/<fixture>.vow -o $TMPDIR/self_<fixture> 2>/dev/null)
# both are structured build-result JSON on stdout; compare with:
python3 scripts/parity.py error <(printf '%s' "$rust_json") <(printf '%s' "$self_json") $rust_exit $self_exit tests/error/<fixture>.vow
```
(`scripts/parity.py`'s `main` takes file paths, not stdin directly — write `$rust_json`/`$self_json`
to temp files first, exactly as `run_parity` in `full_test.sh` does, rather than using process
substitution.) `tests/error/*.vow` fixtures already exist for all 15 cases — no new fixtures are
needed; existing fixtures are the red/green oracle throughout.

0. **Diagnose the actual anchor instruction per fixture — required before slices 1–2.**
   `region.vow:434`'s `linear_emit_consumed_error` fires on `IOP_LINEAR_CONSUME`, `IOP_UPSILON`, *or*
   `IOP_RETURN`; `linear_inst_starts_origin` (feeding the `RegionLinear` "not consumed" check) treats
   `IOP_GET_ARG`, `IOP_REGION_ALLOC`, `IOP_CALL`, `IOP_PHI`, or an owned field-transfer `FIELD_GET` as
   an origin. A grep-only read of `compiler/lower.vow` shows *every* `IOP_UPSILON` emission site is
   currently unspanned (not just the ones behind `lctx_emit_linear_consume_if_needed`), so a fixture
   may anchor on any of these, not necessarily the ones this plan's Slice 1/2 descriptions assume.
   Before writing the fix, determine the actual anchor for each of the 11 `linear_*` fixtures: add a
   temporary debug print in `region.vow` (e.g. in `linear_emit_consumed_error` print `consume.op`, and
   in `linear_emit_live_errors` print the origin `inst.op`), rebuild the self-hosted compiler, run it
   on each fixture, record the op kind, then remove the temporary print before committing. This turns
   "patch the Phi/Upsilon/Consume spans" from a guess into a checklist of exact call sites.

1. **`LinearTypeViolation` span (6 fixtures) — `linear_alias_option_payload_duplicate`,
   `linear_empty_variant_phi_duplicate`, `linear_enum_payload_duplicate`,
   `linear_match_payload_partial_duplicate`, `linear_match_payload_return_after_consume`,
   `linear_option_payload_duplicate`.**
   Red: run the per-fixture check above on `linear_option_payload_duplicate.vow` — self-hosted reports
   offset 0.
   Green: per Slice 0's findings, patch `lctx_set_inst_span` at the `IOP_LINEAR_CONSUME` emission
   (`lctx_emit_linear_consume_if_needed`, threading the consuming expression's span through its call
   sites) and/or the relevant `IOP_UPSILON`/`IOP_RETURN` emission sites, whichever each fixture
   actually anchors on. Re-run the check on all 6 until each reports a located span. Remove the 6
   entries from `SPANLESS_SELF_FIXTURES`.

2. **`RegionLinear` phi-leak span (5 fixtures) — `linear_for_mutation_phi_leak`,
   `linear_if_mutation_phi_leak`, `linear_loop_mutation_phi_leak`, `linear_match_mutation_phi_leak`,
   `linear_while_mutation_phi_leak`.**
   Red: run the per-fixture check on `linear_if_mutation_phi_leak.vow` — self-hosted reports offset 0
   for "not consumed before its region closes".
   Green: per Slice 0's findings, patch `lctx_set_inst_span` on the relevant origin instruction
   (most likely `IOP_PHI`, possibly `IOP_UPSILON` depending on what `linear_origins` walks back to)
   at the mutation-merge emission sites in `if`/`while`/`for`/`match` lowering. Use the span Rust's
   equivalent `vow-ir/src/lower/mod.rs` call site already uses at the matching
   `ctx.emit(Opcode::Phi/Upsilon, ..., span)` — grep that file for the matching lowering function and
   copy its `span` local rather than picking an anchor independently, so self-hosted doesn't newly
   diverge from Rust's anchor (which would swap this fixture from "dropped" into the 47-fixture
   anchor-mismatch pile this issue keeps out of scope). Re-run the check on all 5. Remove the 5
   entries from `SPANLESS_SELF_FIXTURES`.
   Note: `linear_region_branch_phi_leak.vow` exists in `tests/error/` but is *not* in
   `SPANLESS_SELF_FIXTURES` (its origin is a function-parameter `GetArg` inst, which already carries
   a span from parameter parsing) — re-run it too as a regression check, since it's the nearest
   already-passing sibling case.
   Slices 1 and 2 likely share a fix (Upsilon/Phi span patching in the same lowering code) — implement
   and verify them together if Slice 0 shows overlap, but still land as one commit per instruction-kind
   fixed (e.g. one commit for Upsilon spans, one for Phi spans) rather than one commit per fixture.

3. **`IoError` module-resolution span (2 fixtures) — `missing_module`,
   `missing_module_symbol_reference`.**
   Red: run the per-fixture check on `missing_module.vow` — self-hosted reports offset 0 for
   "cannot read ...".
   Green: add `uses_spans: Vec<i64>` to `Module` (`compiler/ast.vow`, plus its two literal sites),
   populate it in `parse_module_into`'s `use`-parsing loop (`compiler/parser.vow`), add the
   `importing_file` parameter to `load_frontend_deps` and use it as the diagnostic's `file` alongside
   the unpacked span (`compiler/frontend.vow`). Re-run the check on both fixtures. Remove both entries
   from `SPANLESS_SELF_FIXTURES`.

4. **`RegionLiteralMutation` span (2 fixtures) — `string_literal_field_mutation`,
   `string_literal_field_branch_overwrite`.**
   Red: run the per-fixture check on `string_literal_field_mutation.vow` — self-hosted reports offset
   0 for "literal-backed value cannot be mutated by ...".
   Green: flip `patches_span` to `true` for the `"push_byte"` row in `builtin_method_spec`
   (`compiler/lower.vow`). Re-run the check on both fixtures. Remove both entries from
   `SPANLESS_SELF_FIXTURES`. Before treating this as final, also run `$RUST build --no-verify` on both
   fixtures and read Rust's actual `span.offset`/`span.length` from its JSON output — if Rust anchors
   on `b.s` specifically (not the whole `.push_byte(33)` call), the `push_byte` flag flip still clears
   the "dropped" gate (these 2 fixtures aren't in `STRICT_SPAN_FIXTURES`) but adds a new anchor
   mismatch against Rust; note this explicitly in the PR description either way rather than silently
   picking whichever anchor was easiest.

5. **Full-suite regression sweep.**
   Run `scripts/full_test.sh` in full (not just the error-fixture subset) to confirm: no previously
   passing fixture (including the 47 anchor-mismatch, non-strict fixtures, and all
   `STRICT_SPAN_FIXTURES`) regresses now that more instructions carry real spans; `ops/catalogue-
   drift`, clippy, and the bootstrap triple-fixed-point check (already part of `full_test.sh`) stay
   green. Run the self-hosted `vow test` suite (`compiler/test_*.vow`) per the ~3 min/file budget
   noted in project memory.

Slice 0 gates slices 1–2. Slices 3 and 4 are independent of everything else and can be implemented
and verified in any order. Land each instruction-kind fix as its own small commit per the "surgical
changes" principle — do not squash all four categories into one diff.

## 4. Verification surface

None of these four fixes touch `requires`/`ensures`/`invariant` contracts, ESBMC-facing codegen, or
the C model. All four diagnostics (`LinearTypeViolation`, `RegionLinear`, `IoError`,
`RegionLiteralMutation`) are compile-time `CompileFailed` diagnostics produced before codegen; `ostart`/
`olen` on an `IrInst` are diagnostic-JSON metadata only (`span`'s `offset`/`length` in the structured
output), not consumed by `compiler/clif.vow`/`vow-clif-shim` codegen or embedded into emitted object
code. So:
- No new ESBMC properties to prove.
- No new `tests/run/*.vow` or `examples/*.vow` fixtures needed — the existing `tests/error/*.vow`
  fixtures already exercise every code path; this issue is purely about the `span` field of diagnostics
  already being emitted with the right `error_code`.
- The existing fixtures are sufficient oracles; adding new ones would duplicate coverage `scripts/
  parity.py` already provides.

One second-order effect worth checking, not fixing: `vow-verify`'s `Origin` metadata maps ESBMC
counterexamples back to source via the same kind of span. These four instruction kinds
(`LinearConsume`, mutation-`Phi`, `Upsilon`, the `push_byte` `Call`) are not verification-condition
carrying instructions in the ESBMC lowering path, so newly-real spans here should not perturb any
counterexample-to-source mapping — confirm this holds by watching for `vow verify` diffs in the
regression sweep (slice 5), not by adding new verification tests.

## 5. Risk areas

- **Binary fixed point.** `scripts/concat_vow.sh` + stage0/stage1/stage2 `sha256sum` comparison is the
  standard guard. Span fields are plain diagnostic metadata, not codegen input, so they should not
  perturb generated machine code — but this is exactly the kind of "should not" that the triple-build
  test verifies cheaply; run it once across all four changes together before opening the PR rather
  than trusting the reasoning alone.
- **`BTreeMap`/`HashMap`, stack-slot layout** — untouched; these changes don't touch `vow-clif-shim` or
  any Cranelift-facing code.
- **`parse → print → parse` idempotency** — the `Module.uses_spans` addition is a new AST field read
  only by `load_frontend_deps`; the canonical printer (`vow-syntax`/self-hosted printer, if one prints
  `use` declarations) must not start emitting span data into printed source. Since spans are never
  printed (only path segments are), this should be inert, but re-run the idempotency tests covering
  `use` declarations as part of slice 3's regression check.
- **`cargo clippy --all -- -D warnings`** — not applicable; no Rust files change in this plan; skip the
  cargo clippy gate for this PR's diff but still run it as part of the standard quality gate since CI
  runs it regardless of whether Rust files changed.
- **Diagnostic emitter robustness with a registered-but-mismatched or unregistered `file`.** Verified
  by reading `compiler/diag.vow`: both `diag_to_json` and `diag_to_human` guard
  `diag_ctx_lookup_source` with `if src.len() > 0` before computing `line`/`column`, and
  `diag_ctx_lookup_source` returns `""` (not a crash) for an unregistered file. So mislabeling a
  diagnostic's `file` degrades output (missing line/column) rather than corrupting or crashing it —
  this is why slice 3's `importing_file` threading is a correctness fix, not a safety one.
- **Regression risk on the 47 anchor-mismatch fixtures.** They're not in `STRICT_SPAN_FIXTURES`, so
  they only need *a* location, not a specific one — newly-patched spans in slices 1–2 could
  incidentally move some of those fixtures' self-hosted span from "also dropped" (currently invisible
  because it's a different diagnostic) to "located but different from Rust," which is fine (still
  passes `_span_errors`'s non-strict check), or from "located" to "located differently" if a Phi/
  Consume instruction happens to be involved in one of those 47 — the full-suite sweep (slice 5) is
  what catches this, not reasoning about it in advance.

## 6. Out of scope

- **`EXPR_FIELD()` span patching in `compiler/lower.vow`.** This would give the field-get target
  (`b.s`) its own span rather than relying on the `push_byte` call-span fallback, which would be a
  more "correct" anchor for `RegionLiteralMutation` and would likely also help some of the 47
  anchor-mismatch fixtures. Deliberately deferred: it's a broader, higher-blast-radius change (every
  field access in the compiler gets a new span, not just `push_byte`'s target) than this issue needs,
  and the issue's own scoping language explicitly separates "dropped" (this issue) from "anchor
  mismatch" (explicitly future work, "decide per check which anchor is right").
- **The 47 anchor-mismatch fixtures.** Not touched, not audited exhaustively — only checked
  incidentally via the slice 5 regression sweep. A separate follow-up issue should enumerate and fix
  these per-check, as the original issue recommends.
- **`push_str`'s `patches_span = false`.** Same dispatch table as `push_byte`, also `false`, but no
  `tests/error/*.vow` fixture in `SPANLESS_SELF_FIXTURES` currently exercises it through
  `RegionLiteralMutation`. Leaving it alone keeps this PR's diff minimal and targeted at the 15 listed
  fixtures; flip it in a follow-up if/when a fixture surfaces the same drop for `push_str`.
- **Auditing every other synthetic-instruction span gap in `compiler/lower.vow`.** `IOP_PHI`/
  `IOP_UPSILON`/`IOP_CONST_*` emissions elsewhere in the file that are not reachable by any of the 15
  fixtures (e.g., the value-producing if-expression `Phi` in `linear_region_branch_phi_leak.vow`,
  which already passes for an unrelated reason — see slice 2's note) are not touched.
- **Refactoring `lctx_emit`'s signature to accept a span parameter directly**, eliminating the
  emit-then-patch two-step (`lctx_emit` + `lctx_set_inst_span`) pattern. This would be a genuine
  simplification and would make "forgot to patch" bugs like this one structurally harder to write
  again, but it touches every `lctx_emit` call site in the file (100+) and is a pure refactor with no
  externally-visible behavior change beyond this bug fix — exactly the kind of change CLAUDE.md's
  "surgical changes" principle says to split out, not bundle into a bug-fix PR. Worth filing as a
  separate follow-up issue given how directly this bug traces back to that two-step pattern being easy
  to forget.
