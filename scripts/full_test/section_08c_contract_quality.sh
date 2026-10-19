# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 8c: Contract Quality ─────────────────────────────────

section_begin "Section 8c: Contract Quality"

# contract-quality/weak-gate: ratchet on static contract quality across the
# self-hosted compiler — fail if the weak/tautological contract count exceeds the
# committed baseline (#81). Static classification only (no ESBMC), so it is cheap.
# Capture the contracts JSON in its own step so a producer failure (parse error,
# missing binary, compiler crash) is reported as itself — with its stderr visible —
# instead of being masked as a baseline breach by the checker's empty-stdin exit.
# One run per entry point: `vow contracts` follows `use` edges, so an entry point
# covers its own module graph and nothing else. `compiler/module_io.vow` is
# deliberately not in main.vow's `use` graph (.vmod parity infrastructure), so it
# had zero coverage until it was listed here. See the scope note in
# scripts/check_contract_quality.py for why the corpus dirs stay ungated.
for quality_entry in compiler/main.vow compiler/module_io.vow; do
    quality_case="contract-quality/weak-gate:$(basename "$quality_entry" .vow)"
    contract_quality_json="$TMPDIR/contract_quality_$(basename "$quality_entry" .vow).json"
    if ! run_self contracts "$quality_entry" >"$contract_quality_json"; then
        fail "$quality_case" "vow contracts $quality_entry failed (see stderr above); could not evaluate contract quality"
        continue
    fi
    # Distinguish the checker's exit codes: 0 = pass, 1 = baseline breach (a real
    # contract-quality regression), 2 = structural error (malformed JSON / missing
    # or non-integer counter — the checker's stderr above names the cause). A bare
    # else would mislabel a schema error as a baseline breach.
    quality_status=0
    uv run python scripts/check_contract_quality.py --label "$quality_entry" \
        <"$contract_quality_json" || quality_status=$?
    if [ "$quality_status" -eq 0 ]; then
        pass "$quality_case"
    elif [ "$quality_status" -eq 1 ]; then
        fail "$quality_case" "weak/tautological contracts exceeded baseline in $quality_entry; strengthen the new contract or adjust scripts/check_contract_quality.py with justification"
    else
        fail "$quality_case" "contract quality check could not run for $quality_entry (malformed 'vow contracts' output / schema mismatch; see stderr above)"
    fi
done

# contract-quality/parity: the ratchet above only ever runs $SELF, so
# vow/src/contract_quality.rs has no end-to-end coverage from it and the two
# classifiers can drift silently. Compare the (function, kind, quality,
# source.offset) tuples both compilers derive from one fixture, plus check
# that each offset actually lands on the clause keyword.
#
# `quality_fixture` only has vow-block `requires`/`ensures` clauses (no
# `invariant`, no parameter `where` refinements) — the keyword-prefix check
# below assumes that anchor rule (docs/spec/cli.md, docs/spec/contracts.md).
# A `where` refinement also reports kind "requires" but both compilers anchor
# it on the parameter name, not the keyword, so it would fail this check;
# keep such fixtures out of `quality_fixture` (their parity coverage lives in
# the separate `where-refinement/offset-parity` block below instead).
#
# `description` text is compared too (per clause, in declaration order): both
# printers must render casts, literals and operator nesting identically. The
# dedicated `contract-text/parity` case below pins that against an expected file.
quality_fixture="tests/fixtures/contracts/quality_shapes.vow"
rust_quality_json="$TMPDIR/quality_parity_rust.json"
self_quality_json="$TMPDIR/quality_parity_self.json"
if ! $RUST contracts "$quality_fixture" >"$rust_quality_json" 2>/dev/null \
    || ! run_self contracts "$quality_fixture" >"$self_quality_json" 2>/dev/null; then
    fail "contract-quality/parity" "vow contracts failed on $quality_fixture (rust or self-hosted)"
else
    parity_result=$(python3 -c "
import json, sys

def tuples(path):
    with open(path) as f:
        d = json.load(f)
    got = sorted(
        (c['function'], c['kind'], c['quality'], c['source']['offset'])
        for c in d['contracts']
    )
    return got, d['summary']['quality']

fixture_bytes = open(sys.argv[3], 'rb').read()

r_tuples, r_quality = tuples(sys.argv[1])
s_tuples, s_quality = tuples(sys.argv[2])
errors = []
if r_tuples != s_tuples:
    only_rust = [t for t in r_tuples if t not in s_tuples]
    only_self = [t for t in s_tuples if t not in r_tuples]
    errors.append(f'clause quality/offset differs: rust-only={only_rust} self-only={only_self}')
if r_quality != s_quality:
    errors.append(f'summary.quality differs: rust={r_quality} self={s_quality}')
def descriptions(path):
    with open(path) as f:
        return [c['description'] for c in json.load(f)['contracts']]
r_desc = descriptions(sys.argv[1])
s_desc = descriptions(sys.argv[2])
if r_desc != s_desc:
    errors.append(f'description text differs: rust={r_desc} self={s_desc}')
# Pin the absolute expectation too: parity alone would pass a regression that
# makes BOTH compilers classify every clause 'substantive'.
expected = {'weak': 6, 'tautological': 2, 'substantive': 7}
if r_quality != expected:
    errors.append(f'rust summary.quality {r_quality} != expected {expected}')
# Pin the anchor decision itself: the byte at offset must start the clause
# keyword (e.g. b'requires:'), not land inside the predicate expression.
for label, clause_tuples in (('rust', r_tuples), ('self-hosted', s_tuples)):
    for function, kind, quality, offset in clause_tuples:
        needle = (kind + ':').encode()
        if not fixture_bytes[offset:].startswith(needle):
            errors.append(
                f'{function}/{kind}: {label} offset {offset} does not start with {needle!r}'
            )
print('; '.join(errors) if errors else 'OK')
" "$rust_quality_json" "$self_quality_json" "$quality_fixture" 2>&1) || parity_result="checker error: $parity_result"
    if [ "$parity_result" = "OK" ]; then
        pass "contract-quality/parity"
    else
        fail "contract-quality/parity" "$parity_result"
    fi
fi
echo ""

# contract-text/parity: the contract text rendered into `description` (and
# from there into VowViolation.description and counterexample `violation`)
# must be byte-identical across compilers for every integer-literal form,
# cast, operator nesting, and compound expression (if/else, match, blocks,
# loops, constructors, type annotations). Both compilers are checked against
# the same expected file, one escaped description per clause in declaration
# order (`\` as `\\`, newline as `\n`); the Rust integration test
# vow-ir/tests/contract_text_forms.rs reads the same files.
# contract_text_postfix is the parse-parity table for parenthesised block-like
# expressions, cast chains and postfix operators; contract_text_postfix_canonical
# is its already-canonical twin sharing one expected file, so parse -> print ->
# parse is checked idempotent in both compilers.
for text_name in contract_text_forms contract_text_blocks contract_text_atoms contract_text_postfix contract_text_postfix_canonical; do
    text_fixture="tests/fixtures/contracts/${text_name}.vow"
    text_expected="tests/fixtures/contracts/${text_name%_canonical}.expected"
    for text_compiler in rust self; do
        text_json="$TMPDIR/${text_name}_${text_compiler}.json"
        text_ok=0
        if [ "$text_compiler" = "rust" ]; then
            $RUST contracts "$text_fixture" >"$text_json" 2>/dev/null || text_ok=1
        else
            run_self contracts "$text_fixture" >"$text_json" 2>/dev/null || text_ok=1
        fi
        if [ "$text_ok" -ne 0 ]; then
            fail "contract-text/parity:${text_name}:${text_compiler}" "vow contracts failed on $text_fixture"
            continue
        fi
        text_result=$(python3 -c "
import json, sys

def escape(text):
    return text.replace(chr(92), chr(92) * 2).replace(chr(10), chr(92) + 'n')

got = [escape(c['description']) for c in json.load(open(sys.argv[1]))['contracts']]
want = open(sys.argv[2]).read().splitlines()
if got == want:
    print('OK')
else:
    diffs = [(i, g, w) for i, (g, w) in enumerate(zip(got, want)) if g != w]
    print(f'{len(got)} clauses vs {len(want)} expected; first mismatches: {diffs[:3]}')
" "$text_json" "$text_expected" 2>&1) || text_result="checker error: $text_result"
        if [ "$text_result" = "OK" ]; then
            pass "contract-text/parity:${text_name}:${text_compiler}"
        else
            fail "contract-text/parity:${text_name}:${text_compiler}" "$text_result"
        fi
    done
done
echo ""

# where-refinement/offset-parity: a parameter's inline `where` refinement
# (e.g. `n: i64 where 0 <= n`) also reports kind "requires" in `vow contracts`
# JSON, but it is anchored on the parameter name's byte offset, not on the
# clause keyword that `contract-quality/parity` above checks for vow-block
# clauses. Cover that separately here with a fixture that has no vow-block
# clauses at all, so every "requires" entry it produces is a parameter
# refinement.
where_fixture="tests/fixtures/contracts/where_refinement_offsets.vow"
rust_where_json="$TMPDIR/where_parity_rust.json"
self_where_json="$TMPDIR/where_parity_self.json"
if ! $RUST contracts "$where_fixture" >"$rust_where_json" 2>/dev/null \
    || ! run_self contracts "$where_fixture" >"$self_where_json" 2>/dev/null; then
    fail "where-refinement/offset-parity" "vow contracts failed on $where_fixture (rust or self-hosted)"
else
    where_parity_result=$(python3 -c "
import json, re, sys

def param_entries(path):
    with open(path) as f:
        d = json.load(f)
    return sorted(
        (c['function'], c['source']['offset'], c['description'])
        for c in d['contracts']
        if '(where on parameter' in c['description']
    )

fixture_bytes = open(sys.argv[3], 'rb').read()

r_entries = param_entries(sys.argv[1])
s_entries = param_entries(sys.argv[2])
errors = []
# Pin the anchor decision itself: the byte at offset must start the
# parameter name followed by ':' (the 'name: type' position), not land
# inside the refinement predicate.
for label, entries in (('rust', r_entries), ('self-hosted', s_entries)):
    if len(entries) != 2:
        errors.append(f'{label}: expected 2 parameter-refinement requires entries, got {len(entries)}: {entries}')
    for function, offset, description in entries:
        m = re.search(r'\(where on parameter (\w+)\)', description)
        if not m:
            errors.append(f'{function}: {label} description missing parameter name: {description!r}')
            continue
        needle = (m.group(1) + ':').encode()
        if not fixture_bytes[offset:].startswith(needle):
            errors.append(
                f'{function}: {label} offset {offset} does not start with {needle!r}'
            )

r_tuples = [(f, o) for f, o, _ in r_entries]
s_tuples = [(f, o) for f, o, _ in s_entries]
if r_tuples != s_tuples:
    errors.append(f'(function, offset) differs: rust={r_tuples} self-hosted={s_tuples}')
print('; '.join(errors) if errors else 'OK')
" "$rust_where_json" "$self_where_json" "$where_fixture" 2>&1) || where_parity_result="checker error: $where_parity_result"
    if [ "$where_parity_result" = "OK" ]; then
        pass "where-refinement/offset-parity"
    else
        fail "where-refinement/offset-parity" "$where_parity_result"
    fi
fi
echo ""
