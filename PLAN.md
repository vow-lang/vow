# Plan: #1144 — non-contract counterexample `violation` text

## 1. Problem restated

When ESBMC trips a verifier-model guard that is not a user-authored `vow`
clause — a `Vec`/`String` bounds check, a collection-capacity guard, a shift
amount check, or an `unwrap()`-on-`None` guard — the counterexample carries
`blame: "none"` and no matching `VowEntry`. In that case Rust's
`build_structured_counterexample_with_module` (`vow/src/counterexample.rs:221-229`)
falls through to `ce.description`, which is just the first ESBMC output line
matching `"Counterexample"`/`"violation"`/`"FAILED"` (`vow-verify/src/esbmc.rs:171-175`)
— for a bounds failure this is the bare marker `[Counterexample]`, an internal
ESBMC token leaking into the agent-facing JSON. The self-hosted compiler
(`compiler/main.vow:543-632`) takes the equivalent path and simply leaves
`ce_violation` as `""`. Neither text tells an agent the actionable fact: which
check failed. Fixed empirically against real ESBMC 8.3.0 output (see §3,
Slice 1): the raw label the C emitter passes to `__ESBMC_assert(cond, "label")`
appears verbatim as its own trimmed line inside the `Violated property:` block,
in the same position `vow:N`/`arith:...` labels already occupy — so the fix is
a sibling to the existing `extract_vow_label`/`extract_arith_site` parsers, not
a new mechanism.

The C emitter's non-vow `__ESBMC_assert` labels form a **closed, enumerable
set of 8 literal strings**, independently written in `vow-verify/src/c_emitter.rs`
and `compiler/c_emitter.vow`:

| Label (as emitted)   | Rust emit site(s)                          | Self-hosted emit site(s)          |
|-----------------------|---------------------------------------------|-------------------------------------|
| `vec bounds`          | c_emitter.rs:1375,1396                      | c_emitter.vow:1827,1854             |
| `string bounds`       | c_emitter.rs:1515                           | c_emitter.vow:2084                  |
| `vec capacity`        | c_emitter.rs:1355,1363                      | c_emitter.vow:1803,1813             |
| `string capacity`     | c_emitter.rs:1483,1498                      | c_emitter.vow:2054,2065             |
| `hashmap capacity`    | c_emitter.rs:1735                           | c_emitter.vow:2331                  |
| `btreemap capacity`   | c_emitter.rs:1814                           | c_emitter.vow:2431                  |
| `integer shift count` | c_emitter.rs:1124                           | c_emitter.vow:1464                  |
| `unwrap-none`         | c_emitter.rs:1249                           | c_emitter.vow:1579                  |

The issue's acceptance criteria name 4 of these (`vec bounds`, `string bounds`,
`vec capacity`, `string capacity`) via the 3 reproduction fixtures, and phrase
the guarantee unconditionally ("Rust never emits a raw ESBMC line as
violation"). Mapping only 4 of the 8 while widening the "must not fall through"
assertion to cover the general `blame: "none"` path would leave `hashmap
capacity`/`btreemap capacity`/`integer shift count`/`unwrap-none` still able to
leak a raw line — a half-finished version of the exact mechanism this issue is
about. All 8 are mapped in this slice (§3, Slices 1 & 3); only the two that
already have `tests/verify-fail/` fixtures get end-to-end + parity-gate
coverage (§4).

**Confirmed empirically** (this session, ESBMC 8.3.0, `esbmc /tmp/vecbounds_test.c
--no-bounds-check --no-pointer-check --incremental-bmc --max-k-step 7 --64`
against `__ESBMC_assert(i < 3, "vec bounds")`):

```
Violated property:
  file vecbounds_test.c line 4 column 3 function main
  vec bounds
  i < 3
```

The label is a bare, trimmed, exact-match line — no decoration, no prefix —
immediately followed by the raw C condition text on its own line. This is the
same shape `vow:N` and `arith:...` labels already have in this position, so
`extract_vow_label`/`parse_vow_label` and `extract_arith_site`/`parse_arith_site`
are the direct precedent for the new parser, including the "return on first
exact match, scanning top to bottom" behavior — the condition-text line never
collides with a fixed English label like `"vec bounds"`.

## 2. Files to touch

**Rust (`vow-verify` — parsing) and `vow` (assembly):**
- `vow-verify/src/esbmc.rs` — add `extract_assert_label(output: &str) -> Option<&'static str>`, sibling to `extract_arith_site`/`extract_vow_label` (same file, same scan-the-`Violated property:`-block idiom). Pure function; no changes to `parse_esbmc_output` or the `Counterexample` struct.
- `vow-verify/src/lib.rs` — export `extract_assert_label` from the `pub use esbmc::{...}` list (line 10-16), alongside the already-exported `extract_arith_site`.
- `vow/src/counterexample.rs` — `build_structured_counterexample_with_module`'s `violation` computation (lines 221-229): add an `else if let Some(desc) = vow_verify::extract_assert_label(&ce.raw_output)` arm before the final fallback; the final fallback becomes a fixed, non-raw string instead of `ce.description.clone()`. Update the `use vow_verify::{...}` import (line 17) to add `extract_assert_label`.

**Self-hosted (`compiler/`):**
- `compiler/verifier.vow` — add `parse_assert_label(output: String) -> String`, sibling to `parse_arith_site`/`parse_vow_label` (same file, same scan idiom, `""` = "no match" per the existing `parsed_arith_site_none()` convention).
- `compiler/main.vow` — `build_ce_from_result`'s final `else` branch (lines 621-632, the plain vow-lookup loop): after the loop, if `ce_violation.len() == 0`, call `parse_assert_label(vr.raw_output)` and use it, falling back to the same fixed generic string Rust uses if that's also empty.

**Test/parity infrastructure:**
- `scripts/parity.py` — remove the `blame == "none"` exemption:
  - `_compare_counterexamples` (line 213): `for field in _counterexample_fields(fields, rust_cex, self_cex):` → `for field in fields:`.
  - Delete `_counterexample_fields` (lines 172-180) — no longer has a caller.
  - `compare_json`'s two explicit field tuples (lines 279, 283-286): add `"violation"` — `("function", "blame")` → `("function", "blame", "violation")`; `("function", "vow_id", "blame")` → `("function", "vow_id", "blame", "violation")`.
  - `COUNTEREXAMPLE_COMPARED_FIELDS` (line 67-71): drop `"violation"` from the exclusion tuple (`field not in ("source", VALUES_LABEL, "violation")` → `field not in ("source", VALUES_LABEL)`), so the schema-derived tuple used by the `vowc test` comparison path includes it automatically.
  - Update the stale comments this leaves behind (lines 56-62 doc comment on `COUNTEREXAMPLE_COMPARED_FIELDS`, the `#1144` reference at line 176-178).
- `scripts/test_parity.py`:
  - `test_unattributed_counterexample_violation_is_not_compared` (lines 192-201) — this test pins the exemption being removed; replace it with a test asserting `violation` **is** compared and reported on mismatch even when both sides are `blame: "none"` (rename accordingly, e.g. `test_none_blame_counterexample_violation_must_match`).
  - `test_the_compared_counterexample_fields_are_read_out_of_the_schema` (lines 826-834) — update the expected exclusion set from `{"source", "values", "violation"}` to `{"source", "values"}`.
- No changes needed to `scripts/equivalence.py` (confirmed: it only compares `(function, vow_id, blame)` identity tuples, never `violation` text — verified by reading `error_codes`/the `(function, vow_id, blame)` tuple builder around line 385-395) or to `scripts/verify_eval.py` (confirmed: its `counterexample-*` directives check `fn`/`blame`/`vow-id`, never `violation` text).

**Docs (`docs/spec/*.md` — required per repo convention, since the JSON output's `violation` semantics change for `blame: "none"`):**
- `docs/spec/schemas/counterexample.schema.json` — `violation`'s `description` (line 18-21) changes from `"Description of the violated contract clause"` to something covering both cases, e.g. `"Description of the violated contract clause, or of the internal check that failed when blame is \"none\""`.
- `docs/spec/cli.md` — one short addition near the existing `violation`/`blame` prose (around lines 344-349, after the caller-blame paragraph): a sentence stating that `blame: "none"` counterexamples (collection bounds/capacity, unwrap-on-None, shift-count guards) carry a real description of the failed check, not raw verifier output. Keep this to 1-2 sentences — do not restate the 8-label table in prose (that belongs in code comments, not the generated docs, per the advisor's regeneration-cost concern).
- After editing the two files above: `uv run python scripts/generate_help.py` (regenerates `vow/src/skill.rs` and the `// GENERATE:SKILL_*` blocks in `compiler/main.vow`), then `cargo build --release -p vow`, then `scripts/bootstrap.sh --skip-cargo`. `scripts/check_help_coverage.py` (part of `full_test.sh`) is the staleness gate if this is skipped.
- `docs/equivalence/README.md` — remove the now-stale bullet at line 220-236 ("Contract-backed counterexample `violation` text is already compared; only blame-`none` fallback text remains excluded pending #1144.") once the exemption is gone; fold the remaining content of that bullet (the `#1138` diagnostics-suppression point) so the paragraph still reads correctly without the removed sentence.

**Not touched:** `vow_id` handling anywhere. The existing audit note (`docs/audit-20260610/vow-analysis.md:605`) flags that a `blame: "none"` counterexample is reported with `vow_id: 0`, which can collide with a real vow 0 on the same function. That is a real, separate concern (it would need a reserved sentinel `vow_id` analogous to `UNSUPPORTED_OP_VOW_ID`/`CALLER_PRECONDITION_VOW_ID`, touching the `vow_id` field's parity comparison and JSON schema) — out of scope here; note it as a follow-up issue rather than bundling it into this fix.

## 3. TDD slices

1. **Rust: `extract_assert_label` unit tests, red then green** (`vow-verify/src/esbmc.rs`, new `#[cfg(test)]` cases near the existing `extract_arith_site`/`extract_vow_label` tests).
   - Table-driven test feeding a synthetic `Violated property:\n  file f.c line 1 column 1 function f\n  <label>\n  <cond>\n\nVERIFICATION FAILED` block for each of the 8 labels; assert the exact mapped description (see table in §1 for labels; descriptions below).
   - Test that a `vow:0` block and an `arith:add:0:0:0` block both return `None` (this extractor must not shadow the existing ones).
   - Test that an unrecognized bare label (e.g. `"some future guard"`) returns `None`.
   - Production code: add `extract_assert_label` (pure, no I/O) and export it from `vow-verify/src/lib.rs`.

2. **Rust: wire into `build_structured_counterexample_with_module`, red then green** (`vow/src/counterexample.rs`, alongside the existing `structured_counterexample_unsupported_op_sentinel` test).
   - New test `structured_counterexample_vec_bounds_maps_to_index_out_of_bounds`: construct a `Counterexample` with `vow_id: None`, `callee_precondition: None`, `description: "[Counterexample]"` (the raw marker, to prove it's not used), `raw_output` containing a `Violated property:` block with `vec bounds`. Assert `sce.violation == "index out of bounds"`, `sce.blame == "none"`, and — mirroring the existing `assert_ne!(sce.violation, "[Counterexample]", "must not fall through to raw ESBMC line")` pattern — assert `sce.violation != ce.description`.
   - New test `structured_counterexample_string_capacity_maps_to_capacity_message` (same shape, `string capacity` label) to prove the mapping is label-specific, not a single catch-all string.
   - New test `structured_counterexample_unrecognized_label_does_not_leak_raw_line`: `raw_output` with a `Violated property:` block containing neither a known label nor a `vow:`/`arith:` prefix (simulating a hypothetical future/unmapped assert); assert `sce.violation` is the fixed generic fallback string and, again, `!= ce.description`. This is the generalized version of the existing unsupported-op guard the issue asks to extend.
   - Production code: add the `else if let Some(desc) = vow_verify::extract_assert_label(&ce.raw_output)` arm; change the final fallback from `ce.description.clone()` to the fixed string `"internal verifier assertion failed"`.

3. **Self-hosted: `parse_assert_label` unit tests, red then green** (`compiler/tests/test_verifier.vow`, following the existing `check_*` / integer-return-code convention already used there — e.g. `check_memory_limit_classifier_ignores_echoed_memlimit_option`, `check_counterexample_value_names`).
   - New `check_parse_assert_label_maps_known_labels`: build the same 8 synthetic `Violated property:` blocks as Slice 1 and assert `parse_assert_label` returns the matching description string for each.
   - New `check_parse_assert_label_ignores_vow_and_arith`: a `vow:0` block and an `arith:add:0:0:0` block both return `""`.
   - New `check_parse_assert_label_unknown_label_returns_empty`: an unrecognized label returns `""`.
   - Wire the three into `main()`'s pass/fail chain (same pattern as the existing 5 checks).
   - Production code: add `parse_assert_label`/`describe_assert_label` to `compiler/verifier.vow`, mirroring `parse_arith_site`/`arith_cause_description`'s if/else-chain-on-`String` idiom (Vow has no string `match`).

4. **Self-hosted: wire into `build_ce_from_result`, red then green** — no isolated unit test is practical here (`build_ce_from_result` takes a full `IrFunction`/`IrModule`/`VerifyResult`, which is what the end-to-end fixtures in Slice 5 exercise); instead this slice is verified by Slice 5's fixture run. Production code: in `compiler/main.vow`'s final `else` branch, after the vow-lookup loop, add the `if ce_violation.len() == 0 { ... parse_assert_label(vr.raw_output) ... }` fallback with the same `"internal verifier assertion failed"` generic string as Rust.

5. **End-to-end parity, red then green** (`tests/verify-fail/off_by_one_bounds.vow`, `tests/verify-fail/string_push_str_overflow.vow`, already exist with `// TEST: counterexample-blame none` directives; no fixture changes needed).
   - Rebuild both compilers (`cargo build --release -p vow`, `scripts/bootstrap.sh --skip-cargo`).
   - Run `build/vowc verify tests/verify-fail/off_by_one_bounds.vow` and `./target/release/vow verify tests/verify-fail/off_by_one_bounds.vow`; confirm both report `"violation": "index out of bounds"`. Repeat for `string_push_str_overflow.vow`, expecting the `string capacity` mapping.
   - Manually run both compilers against `stdlib/gc/gc.vow` (`vow verify` / `vowc verify`) and diff the `violation` field of the resulting counterexample — this is **not** gated by any existing automated comparison (see §4 for why), so this is a one-off manual confirmation for the PR description, not a new permanent test.
   - Run `scripts/full_test.sh`'s section 4c (`tests/verify-fail/*.vow` loop, `scripts/parity.py`) to confirm the now-unconditional `violation` comparison is green for the whole `tests/verify-fail/` corpus, not just these two fixtures.

6. **Parity gate + its own tests, red then green** (`scripts/parity.py`, `scripts/test_parity.py`).
   - Update `test_unattributed_counterexample_violation_is_not_compared` → new assertion that a `blame: "none"` mismatch on `violation` **is** reported (red without the parity.py change, green after).
   - Update `test_the_compared_counterexample_fields_are_read_out_of_the_schema`'s expected exclusion set.
   - Apply the `scripts/parity.py` edit list from §2.
   - Run `python3 -m pytest scripts/test_parity.py` (or however the project invokes it — check for a `uv run` wrapper) to confirm green.

## 4. Verification surface

This issue does not touch contracts, IR, codegen, or the C model — it is pure
diagnostic-text plumbing downstream of ESBMC's own output. No new `vow`
clauses, no new C emitter assertions, no new properties for ESBMC to prove.
The 8 `__ESBMC_assert` call sites already exist in both `c_emitter.rs`/`c_emitter.vow`
today; this change only adds a **parser** for their labels, on the read side.

- **No new ESBMC properties.** The existing `vec bounds`/`string bounds`/`vec
  capacity`/`string capacity`/`hashmap capacity`/`btreemap capacity`/`integer
  shift count`/`unwrap-none` assertions are unmodified.
- **`tests/run/`/`examples/` fixtures:** none need to grow. The two existing
  `tests/verify-fail/` fixtures already exercise the two labels the issue's
  acceptance criteria gate automatically (`vec bounds` via `off_by_one_bounds.vow`,
  `string capacity` via `string_push_str_overflow.vow`); the other 6 labels get
  unit-level coverage only (Slices 1 & 3), since no existing fixture trips
  them and manufacturing one for each (e.g. a real hashmap-capacity or
  integer-shift-count counterexample) is not required by the issue and would
  be scope creep beyond a text-mapping fix.
- **`stdlib/gc/gc.vow` is not on any automated comparison path** today:
  `scripts/full_test.sh` never references it directly, and `scripts/equivalence.py`'s
  full-corpus sweep (which does walk `stdlib/`) only compares `(function,
  vow_id, blame)` identity tuples, never `violation` text. So criterion 3's
  third fixture is confirmed by the manual check in Slice 5, not by a gate —
  stated explicitly here rather than implied as covered.

## 5. Risk areas

- **Binary fixed point / self-hosted determinism:** `parse_assert_label` is a
  pure `String -> String` function with no iteration order over unordered
  collections (`HashMap`/`BTreeMap`) — it's a linear scan over `Vec<String>`
  lines and a fixed if/else chain. No `BTreeMap`-vs-`HashMap` risk, no
  `vow-clif-shim` stack-slot interaction (this code never reaches codegen —
  it runs in the verifier/CLI driver, host-side, not compiled into the
  self-hosted binary's own IR). Low risk to the stage-0/1/2 bootstrap triple
  test.
- **`parse -> print -> parse` idempotency:** not implicated — no AST/syntax
  changes.
- **`cargo clippy --all -- -D warnings`:** the new Rust `match` in
  `describe_assert_label` returns `Option<&'static str>`; must confirm clippy
  doesn't flag the `_ => return None` arm style inside a `Some(match {...})`
  wrapper (a `match` returning early from an arm is occasionally flagged by
  `clippy::match_wildcard_for_single_variants` lints on exhaustiveness-shaped
  matches, unlikely here since this isn't an enum match, but worth checking
  post-write) — trivial to restructure as a plain `match label { ... }`
  returning `Option<&'static str>` directly if clippy objects.
- **Verify cache correctness (confirmed safe, not just assumed):** `vow/src/cache.rs`'s
  `CachedFailure`/`CachedFailureRecord` round-trip `raw_output` byte-for-byte
  (`serialize_cached_result`/`parse_cached_result`, with an existing test
  pinning `assert_eq!(got.raw_output, ce.raw_output)`), and `to_counterexample()`
  reconstructs a full `Counterexample` including `raw_output`. Verified by
  reading the code this session — a cache hit and a cold run produce
  identical `extract_assert_label(&ce.raw_output)` input, so `violation` text
  cannot differ between warm and cold cache. This was the key risk the design
  (deriving the description lazily from `ce.raw_output` at
  structured-counterexample-build time, rather than adding a new field to
  `Counterexample`/`VerifyResult` and touching all ~22 Rust construction
  sites plus the cache schema) depended on, and it holds.
- **arith-overflow counterexamples do not reach this code path** (confirmed
  by reading `vow/src/verification.rs:147-193`): a `ce.arith_overflow.is_some()`
  counterexample is intercepted and re-run with arith obligations suppressed
  *before* `build_structured_counterexample_with_module` is ever called, so
  `extract_assert_label` never needs to special-case `arith:` labels — they
  structurally cannot appear in `ce.raw_output` at the point this function
  runs. No branch for arith needed in the new code.
- **Parity gate going unconditional widens what full_test.sh enforces** across
  the entire `tests/verify-fail/` corpus (not just the two named fixtures) —
  if any other existing fixture happens to have a `blame: "none"` counterexample
  whose two compilers currently diverge in some other way that was masked by
  the old exemption, `full_test.sh` will newly fail on it. Mitigated by
  running the full section-4c loop in Slice 5 rather than just the two named
  fixtures, and by both compilers implementing the exact same 8-entry mapping
  table (spelled out identically in this plan, per the advisor's point that
  Rust and Vow implement this independently and acceptance criterion 3 needs
  byte-equality) — but this is where an unexpected finding is most likely to
  surface during implementation.
- **Exact string equality between the two independently-written mapping
  tables.** Rust and self-hosted implement `describe_assert_label` from
  scratch in two languages with no shared constant (consistent with how
  `vow:`/`arith:` prefixes are already handled — duplicated literals, not a
  shared source of truth, per existing codebase convention). A typo in either
  table breaks parity silently until Slice 5/6 runs. Mitigate by copying the
  exact 8 mapped strings from this plan verbatim into both implementations:

  | Label                  | Mapped `violation` text                                     |
  |-------------------------|--------------------------------------------------------------|
  | `vec bounds`            | `index out of bounds`                                        |
  | `string bounds`         | `index out of bounds`                                        |
  | `vec capacity`          | `vec exceeded the verifier's internal capacity limit`        |
  | `string capacity`       | `string exceeded the verifier's internal capacity limit`     |
  | `hashmap capacity`      | `hashmap exceeded the verifier's internal capacity limit`    |
  | `btreemap capacity`     | `btreemap exceeded the verifier's internal capacity limit`   |
  | `integer shift count`   | `shift amount exceeds the operand's bit width`                |
  | `unwrap-none`           | `unwrap() called on None`                                     |
  | *(no label matched)*    | `internal verifier assertion failed` (generic fallback, both compilers) |

## 6. Out of scope

- **New reserved `vow_id` sentinel for `blame: "none"` counterexamples.** The
  audit-flagged `vow_id: 0` collision (a bounds/capacity trip reporting the
  same `vow_id` as a real vow 0 on the same function) is real but is a
  different field with different parity/schema implications; track as a
  separate follow-up issue rather than bundling it here.
- **Fixtures for the 6 unmapped-by-existing-fixture labels** (`hashmap
  capacity`, `btreemap capacity`, `integer shift count`, `unwrap-none`, plus
  end-to-end coverage beyond unit tests for `vec capacity` itself, which has
  no dedicated `tests/verify-fail/` fixture today either). Unit-level parser
  coverage is sufficient for a text-mapping fix; manufacturing real ESBMC
  repros for all 8 is a separate test-coverage improvement.
- **Making `scripts/equivalence.py` compare `violation` text.** It currently
  compares `(function, vow_id, blame)` identity only; extending it to also
  diff `violation` across the full stdlib/examples/benchmarks corpus is a
  meaningful scope increase (the full-corpus sweep is already flagged
  elsewhere as multi-hour) and isn't required by this issue's acceptance
  criteria, which only names `scripts/full_test.sh`'s `violation_aware_fields`
  gate (i.e., `scripts/parity.py`'s `_counterexample_fields`, its actual
  current name — the issue text's reference to `full_test.sh` is imprecise;
  `full_test.sh`'s own `compare_json` is a thin wrapper that shells out to
  `scripts/parity.py`, which is where the exemption actually lives).
- **Adding `stdlib/gc/gc.vow` (or a trimmed repro of it) to `tests/verify-fail/`**
  to get it onto the automated parity gate. Valuable, but a separate, additive
  test-coverage change from the text-mapping fix itself — do it in a follow-up
  if the manual check in Slice 5 reveals a real divergence worth permanently
  guarding against, rather than preemptively in this PR.
- **Refactoring the 8 literal label strings into shared constants** between
  each compiler's own emitter and its own parser (e.g. a `pub(crate) const
  LABEL_VEC_BOUNDS` in `c_emitter.rs` reused by `esbmc.rs`). Would reduce
  same-language drift risk, but is inconsistent with the existing convention
  (the `vow:`/`arith:` prefixes are already duplicated literals across
  `c_emitter.rs` and `esbmc.rs` with no shared constant) — not bundling a
  style change into a bug fix.
- **Rewording `ce.description`'s own heuristic** (`vow-verify/src/esbmc.rs:171-175`,
  "first line containing `Counterexample`/`violation`/`FAILED`"). Already
  identified in memory as a latent-but-inert bug (only caller destructures
  `Failed(_)` without using `.description` for JSON `violation` once this fix
  lands) — this PR removes the one path that surfaced it in agent-facing JSON,
  but does not touch the field itself or its remaining internal uses (e.g.
  log lines). Out of scope; do not rank it as newly high-impact.
