# Plan: sync `docs/spec` and the embedded help/skill with the native verifier (#1430, P3 of epic #1398)

## Goal
Make every verification statement in `docs/spec/*.md`, the schemas' descriptions, the hard-coded
strings in `scripts/generate_help.py`, and the regenerated `--help`/skill copies match what the code
does today with **both** backends, per ADR-2026-10-08-1430 (surface) and ADR-2026-10-08-1422
(semantics). Docs-only: no compiler, verifier, or CLI behaviour changes.

## Assumptions
(decided without an operator; recorded on the PR)
- **Document both backends; do not flip or delete anything.** The default is still `esbmc`
  (`compiler/main.vow` tests only `get_flag_arg(argv, "--backend") == "native"`; embedded help
  says `default: esbmc`). The default flip is P5 and ESBMC/`proven-ir`/`ModelCapacityAssumed`/
  removed-flag rejection are P6 (ADR-1430 sections 3 and 5, #1418 addendum). ADR-1422 line 216
  ("rewrite after native can be the default") conflicts with ADR-1430 section 5 (P3); resolution:
  native semantics are the model, ESBMC-only text is kept and labelled "ESBMC backend", and the
  spec never claims something the compiler does not do. Alternative (wait for P5) rejected: the
  issue is `ready-for-agent` and its blockers (#1427, #1401, #1400) are closed.
- **Spec describes behaviour, not ADR intent, where they differ.** Divergences found, all stay
  documented as current behaviour with the ADR target named; no code change is bundled:
  - Floats are `unsupported-opcode` natively (`compiler/vc_ops.vow:69-75`, no FloatingPoint
    theory); ADR-1422 rule 5 is a target, so contracts.md describes it as "specified, not yet
    modelled".
  - `reason_code` JSON field is not emitted (the code only appears in the message
    ``skipped verification of `f`: <code>: <detail>``); schemas must not declare it yet.
  - `build`/bare form accept `--backend esbmc` silently and reject `native`
    (`backend_flag_error`, `compiler/main.vow:4130-4165`, exit 1) although ADR-1430 section 1
    lists `build` as a native-capable command. The build flag table has no `--backend` row yet;
    the plan adds one (values `esbmc`|`native`, default `esbmc`, native = usage error).
  - `--perfetto` under native emits the frontend/`codegen`/`link` spans and one `verification`
    span (`compiler/main.vow:1906, 2014`), not per-claim worker/Bitwuzla spans or per-worker RSS;
    cli.md:77 "works as for ESBMC" is wrong and is corrected.
  - `--no-cache` is accepted and a no-op natively; Bitwuzla is not hash-pinned (ADR P4 items).
  - Native flag rejection text is `error: --max-k-step is not supported with --backend native`
    (also `--solver`, `--encoding`), exit 1 (unknown flags are exit 2); it does not say "the
    bound is now internal" as ADR-1430 asks. Spec states the actual text.
  - `vf_legacy_fields` (`compiler/main.vow:1413`, called from the shared `verify_finish`, 2040)
    builds `esbmc error: <message>` for `verify_status: "error"` under BOTH backends. Spec says
    the legacy `counterexample` string is `esbmc error: <message>` whichever backend ran (known
    misnomer, follow-up issue, not bundled).
  - `--timeout 0` natively is a zero budget (no solver spawn; statically decided claims still
    resolve), not a group kill; cli.md:77 already says so, keep. ESBMC wording stays
    "immediate watchdog kill".
  - `--perfetto` is parseable only on `build` and `verify` (exit 2 elsewhere); `--no-verify` only
    on `build`; `--replay-cex` only on `build`/`verify`.
  - Native uncontracted-caller notes: `report_unchecked_calls` still picks callers with ESBMC's
    `non_modelable_reason`, and `demote_undecided_caller` (`main.vow:1195`) is ESBMC-only, so a
    native timeout/unknown on an uncontracted caller is a hard `VerifyFailed`, not a Note. Spec
    scopes the "Note form" and the `Verified`-row sentence to the ESBMC backend (ADR-1430 "open
    gap" stays open).
  - `test --verify --backend native` skip is a fixed-message JSON diagnostic
    (`contract verification skipped: function not modelable`, no code/detail); verify cli.md:83
    ("also reported as a `VerificationSkipped` warning") against `main.vow:2377-2410` and
    correct the wording/severity.
  - Native `contracts` summary still counts the string `proven-ir` as proven (`main.vow:4062`);
    never produced.
- **Spec-drift gate added (small, separable).** `check_help_coverage.py` only checks grammar.md;
  nothing checks flags/statuses. A narrow stdlib-only unit test (a)-(c) is the "spec matches
  behaviour" evidence (Slice 6). If it grows, ship the docs edits without it.
- **Evidence status.** All three explorer reports were received; behavioural claims were
  re-verified by grep against `compiler/main.vow`, `compiler/vc_ops.vow`, `compiler/cli_flags.vow`
  (backend_flag_error text, single `verification` perfetto span, float opcodes unsupported,
  `esbmc error:` path). Claims not exercised at runtime (no `build/vowc` here) are re-checked in
  Slice 7 before landing.
- Out-of-spec docs (`docs/verifier-discipline.md`, `docs/design/verifier-model-bounds.md`,
  `docs/verifier-eval.md`) are not edited; their "superseded by" note belongs to P5.

## Key Files
| File | Role | Lines of Interest |
|------|------|-------------------|
| `docs/spec/cli.md` | flag tables, status tables, native sections, replay, contracts fields | 22-34, 48-56, 62-77, 85, 103-114, 154-163, 214, 320, 346-350, 447, 476, 488-491, 495-515, 571-613, 730 |
| `docs/spec/contracts.md` | pipeline, ESBMC config, collection models, loops, counterexamples, 128-bit | 3, 23-127, 141, 158-162, 355-486, 505-514, 542 |
| `docs/spec/errors.md` | VerificationSkipped (add closed reason-code table), EsbmcNotFound, VerifierAssertionUnattributed, ModelCapacityAssumed, wording | 333, 957, 1069-1130, 1162-1173, 1379-1435, 1478-1506, 1541-1595 |
| `docs/spec/grammar.md` | float/128-bit verification notes, checked arithmetic, loops | 150-157, 177-179, 375-376, 398-414, 483-485, 614, 774-782, 803-809, 818, 862, 1032-1036 |
| `docs/spec/index.md`, `examples.md`, `contracts-methodology.md`, `stdlib.md` | ESBMC wording, wrong `Unverified` meaning | index.md:3,20; examples.md:38,140,188; methodology 5,30,132,288-290,319-321,367,379; stdlib 9,17,26,79-92,356 |
| `docs/spec/schemas/*.json` | description strings only (no field/enum changes) | build-result:12,38; counterexample:5,16; test-result:50,67; contracts-result:60 |
| `docs/adr/2026-10-08-1430-native-verifier-cli-and-status-surface.md` | one addendum recording the divergences above (what P3 documented) | end of file |
| `scripts/generate_help.py` | hard-coded ESBMC strings in help JSON/human text and skill frontmatter | 38-40, 155, 316, 475, 549, 758, 837-842, 965-969 |
| `scripts/test_generate_help.py` | pins generator output; update with the strings | `SkillDoNotsTest` ~222, `test_build_output_default_tracks_cli_md` |
| `scripts/test_spec_surface.py` [new] + `.github/workflows/ci.yml` (~l.100) + `scripts/full_test.sh` (~l.2084) | drift gate | see Slice 6 |
| Generated, never hand-edited: `vow/src/skill.rs`, `compiler/main.vow` (GENERATE markers), `skills/vow/**` | output of `generate_help.py` | |

## Steps

### 1. Reconcile cli.md (ordered, one commit)
- Flag tables (build 18-32, verify 46-56, contracts 101-110, test ~150-163): reword generic
  `ESBMC` to "verifier"/"verification" in `--no-verify`, `--verify`, `--verify-jobs`, `--replay-cex`,
  `--no-cache`, compile-object-cache paragraph (34). Keep `--max-k-step`, `--solver`, `--encoding`
  and the 30 s `--timeout` special case, each prefixed "ESBMC backend only; usage error under
  `--backend native`; removed at P6" (ADR-1430 section 2). Keep `--backend` default `esbmc`.
  Description cells must stay non-empty, `|` escaped as `\|`, and keep the `(default: ...)`
  parsing behaviour in mind (`normalize_option`, generate_help.py:95-165).
- Build table (18-32): add a `--backend <esbmc\|native>` row (default `esbmc`; `native` is a usage
  error on `build` and the bare form: `--backend native is only supported by vowc verify, ...`).
  Slice 6(a) is then green for build as well.
- `--perfetto` (32, 56, 77): state the per-backend spans truthfully (ESBMC: per-function proof
  spans + RSS per process; native: phase + one `verification` span; only `build`/`verify`).
- Two `### Status Values` headings exist (build status ~342, contracts clause ~595): keep both
  heading texts unchanged (`extract_table` and anchors depend on them).
- Native section 62-77: add `reserved-verifier-symbol` to the code list (defined, not yet emitted);
  fix `--perfetto` sentence; add the rejection message text; keep "not hash-checked yet",
  "reason_code not emitted yet", `--no-cache` no-op; keep ESBMC comparison clauses (removal is P6).
- Status tables (346-350, 488-491, 600-609): "ESBMC" -> "the verifier" where backend-neutral;
  `Skipped` row gets the closed reason list pointing to errors.md and notes the 128-bit text is
  ESBMC-only; `unknown`/`unwinding assertion:`/`memory limit exceeded`/`tool_not_found` (Bitwuzla
  missing; for contracts clauses it is clause `error`) rows; `proven` row backend-neutral with the
  never-weakened-retry rule; `proven-ir` kept, marked ESBMC-only/never produced natively;
  `vacuous` row names both probe mechanisms.
- 214, 320, 447, 476, 730: backend-neutral wording; fix 730 (missing tool is `VerifyFailed`/`error`,
  not `Unverified`); 476 add `index out of bounds`.
- 112/114: keep `proven`/`proven-ir` exit rule; make "ESBMC not installed" backend-generic.

### 2. contracts.md rewrite per ADR-1422 "Contracts.md rewrite plan"
- Keep: "Semantic Contracts Are Backend-Independent" (reword two ESBMC mentions), blame model,
  Integer/Vec/String/HashMap patterns, anti-patterns.
- Replace "Verification Pipeline" and "ESBMC Configuration" with a per-backend table: native =
  IR -> SMT-LIB -> Bitwuzla in `verify-worker`s, staged codegen-then-verify (ADR 1719),
  BMC + invariant induction + automatic k-induction, mandatory unwinding assertion, internal
  bounds 2..64, `--timeout` the only knob, `unknown` with `unwinding assertion:`; ESBMC = existing
  text under "ESBMC backend".
- "Collection Models": add the native array + symbolic-length model (no caps, only
  bool/<=64-bit integer `Vec`, String/maps Skipped); keep the capacity table and
  `ModelCapacityAssumed` text labelled ESBMC-only.
- Edit "Non-Inductive Loop Invariant", "Unbound Loop Iterations", "Interpreting Counterexamples"
  (step-case failure is `unknown`, never `failed`; unattributed vow id `4294967293`),
  "Counterexample Replay", u64/128-bit sections (`BitVec w`; `--encoding ir` sentence ESBMC-only).
- Add "Floats" (IEEE RNE, canonical NaN, `RemF*` Skipped `float-rem-unsupported`; **status: not
  yet modelled, floats are `unsupported-opcode` today**) and "Known limits" (NaN payload,
  String parse/format opacity, callee-inlining cost, aggregates).

### 3. errors.md
- `VerificationSkipped`: replace "C model cannot represent" meaning; add the closed reason-code
  table (8 codes, when, lifetime, `detail` free-form, "not Skipped" mapping: timeout, unknown,
  `tool_not_found`, `panicked`, failed); state `reason_code` field is additive and not yet emitted;
  keep note form and wrong/right snippets (Fix text must stay valid under both backends).
- `EsbmcNotFound`: keep, add "native: Bitwuzla missing is `verify_status: tool_not_found`".
- `VowRequiresViolated`/`VowEnsuresViolated`/`VowInvariantViolated`/`VerifierAssertionUnattributed`:
  "Verification (ESBMC)" -> "Verification"; add native violation texts.
- `ModelCapacityAssumed`: add "never emitted under `--backend native`"; entry stays until P6.
- `ShiftCountOutOfRange`, BTreeMap key: neutral wording.

### 4. grammar.md, index.md, examples.md, methodology, stdlib
- grammar.md: 128-bit (150, 155, 177-179, 398-414) = ESBMC `Skipped`, native models scalars and
  aggregate fields (#1421), `Vec<i128>` Skipped under both; floats (156-157, 1032-1036) = "native:
  unsupported-opcode today; IEEE per ADR when modelled"; fix false claims at 803-809 (checked
  arithmetic IS modelled natively) and 818/862 (`loop` verified natively); 774-782 verify
  against `tests/verify-native` fixtures before editing.
- index.md:3 and :20 (`Unverified` = `--no-verify`/`--dump-ir`, not "ESBMC not available").
- examples.md, methodology: "ESBMC" -> "the verifier" except where the text is ESBMC-specific
  (`--error-label vow_reach`, `__ESBMC_assume`: label "ESBMC backend", add native probe sentence
  from cli.md:86). stdlib.md measurements: label "(ESBMC backend, measured against 8.3.0)"; do not
  invent native results.
- Schemas: description strings only; **no enum/field edits** (`proven-ir`, `ModelCapacityAssumed`
  stay until P6; `reason_code` not declared until emitted).

### 5. Generator strings, regenerate, rebuild
- `scripts/generate_help.py`: keep `verification_defaults.strategy`/`max_k_step` byte-identical
  (agent-facing `--help` surface; ESBMC is still the default) and ADD a sibling `native` key
  (BMC + invariant/k-induction, no bound flag, `--timeout` only knob); qualify the human
  `VERIFICATION DEFAULTS` block per backend. Grep `tests/`, `bench/`, `scripts/*.py` for
  `verification_defaults` consumers first (none found in `tests/`, `bench/`, `scripts/*.py` apart
  from the generator); `ESBMC counterexample` -> "verifier counterexample"; skill frontmatter
  `ESBMC counterexamples` -> "verifier counterexamples" (965, 969); `--max-k-step` default cast
  stays for the ESBMC backend. Update `scripts/test_generate_help.py` first (Slice 5).
- Run in order: `uv run python scripts/generate_help.py` (rewrites `vow/src/skill.rs`,
  `compiler/main.vow` markers, `skills/vow/**`); `cargo build --release -p vow`;
  `scripts/bootstrap.sh --skip-cargo --no-cache` (builds `build/vowc`, which does not exist yet);
  `generate_help.py --check`; `check_help_coverage.py docs/spec/grammar.md "$(build/vowc --help)"`
  and with `./target/release/vow --help`.

### 6. Spec-drift unit test [new] `scripts/test_spec_surface.py`
Asserts (stdlib-only, fast, no compiler, separable from the docs edits): (a) every `--flag`
accepted by `compiler/cli_flags.vow` (`cf_is_*`, excluding the internal `--worker-*` set) appears
in a `docs/spec/cli.md` flag table row of a command that accepts it (build table gains
`--backend`); (b) every `status` enum value in
`docs/spec/schemas/contracts-result.schema.json` appears in the `### Status Values` table that
follows "Contracts Output JSON" (scope by that parent heading, not the first `### Status Values`
which is the build-status table) and vice versa; (c) every code in `compiler/vc_ops.vow::vc_skip_code_valid` appears in both
cli.md and errors.md. Wire into `ci.yml` next to the other `scripts/test_*.py` steps.

## TDD slices (ordered red -> green)
1. `scripts/test_spec_surface.py` (c): write failing test that `reserved-verifier-symbol` is in
   cli.md and errors.md -> add the errors.md table and cli.md code. (Red proves the gate bites.)
2. Same file (a): `--worker-*`/internal flags excluded explicitly; red until the build table
   gains its `--backend` row -> green with Step 1 edits.
3. Same file (b): statuses; green by Step 1 table edits (no enum change).
4. `scripts/test_generate_help.py`: new assertions that the help JSON keeps `strategy` and
   `max_k_step` unchanged (additive only) and gains a sibling native key, and that no help
   text says "ESBMC counterexample" -> edit generator (Step 5).
5. `generate_help.py --check` red after any spec edit until `generate_help.py` is rerun (that is
   the existing regeneration gate: commit its output).
6. Docs prose edits (Steps 1-4) are verified by the above plus `scripts/test_build_docs_site.py`
   (anchors such as `errors.md#modelcapacityassumed`, `#verificationskipped` must still resolve).

## Verification surface
- Runtime spot-check of the `--perfetto` correction: after bootstrap run
  `build/vowc verify --backend native --perfetto $TMPDIR/t.json.gz` on one
  `tests/verify-native/pass/*.vow`, gunzip, list span names; record in the PR.
- No ESBMC/Bitwuzla property changes; no `tests/run/`, `tests/verify*/` or `examples/` fixtures grow.
  Spot-check every new factual sentence by running it against `build/vowc` on an existing
  `tests/verify-native/**` fixture (`--backend native` usage errors, `unwinding assertion:` message,
  `memory limit exceeded`, `tool_not_found` with `PATH` stripped).
- Gates to run before pushing: `python3 scripts/test_generate_help.py`,
  `python3 scripts/test_check_help_coverage.py`, `python3 scripts/test_build_docs_site.py`,
  `python3 scripts/test_spec_surface.py`, `python3 scripts/generate_help.py --check`,
  `python3 scripts/generate_operations.py --check`, `cargo test -p vow checked_in_skills_vow_matches_install_output`,
  `cargo test -p vow-diag` (error-code/schema enum test), `scripts/bootstrap.sh --skip-cargo --no-cache`
  (record head SHA and `scripts/seed.toml` pin if present), `build/vowc test compiler/` smoke is
  unnecessary (generated strings only) but `scripts/full_test.sh` Section `help/*` runs it.

## Risks
- **Binary fixed point / stage determinism**: `compiler/main.vow` GENERATE blocks change, so
  bootstrap must be re-run; chunked string literals are deterministic, but a stray unescaped
  quote breaks compile. Mitigation: only edit via `generate_help.py`; bootstrap before PR.
- **Both-compilers rule**: `vow/src/skill.rs` and `compiler/main.vow` are both regenerated by the
  same script in one commit, so no drift; no semantic code changes, hence no Rust/Vow twin work.
- **Verifier C parity** (`c_emitter.rs`/`c_emitter.vow`): untouched.
- **cli.md table parsing**: a row with an empty cell or an unescaped `|` silently drops/garbles
  an option in the help JSON; ESBMC-only labels added to a description must still end the
  cell sensibly because `(default: X)` is appended unless it ends with `)`. Diff the regenerated
  help JSON options list before/after.
- **Truthfulness**: easiest failure is writing ADR intent as fact (floats IEEE, `reason_code`,
  hash pin, `build` accepting `--backend`). Every new sentence needs a code or fixture citation
  in the PR description.
- **Anchors/links**: website build rewrites relative links; renamed headings break
  `contracts.md#...` anchors. Keep heading text of existing sections stable (add, don't rename).
- **`skills/vow/**` mirrors the spec byte-for-byte**; hand-edits are overwritten.
- **Environment**: no `build/vowc` exists; bootstrap needs `cargo build --release -p vow` first
  (~5 min bootstrap); run in foreground with explicit timeouts and background+poll per the
  wall-clock memory. Concurrent bootstrap can flake cargo tests.
- **Codecov**: regenerated `compiler/main.vow` chunks appear as changed lines; #1617/#1618
  regenerated the same blocks and merged, confirm `.vow` exclusion in `codecov.yml` before
  worrying about `codecov/patch`.
- **Seed pin**: record `scripts/seed.toml` pin with the head SHA when claiming "green locally".

## Out of scope
- Flipping the default backend (P5), removing ESBMC/flags/`proven-ir`/`ModelCapacityAssumed` (P6),
  emitting/declaring `reason_code` in schemas, FloatingPoint modelling, Bitwuzla hash pin,
  native result cache, native per-worker perfetto spans, changing `esbmc error:` string.
- Edits to `docs/verifier-discipline.md`, `docs/design/verifier-model-bounds.md`,
  `docs/verifier-eval.md`, mutation-testing docs, or any compiler/verifier source.
- File follow-up issues for the code gaps listed under Assumptions (perfetto spans, `esbmc error:`
  prefix, `build` accepting `--backend`, `reason_code` emission).
