#!/usr/bin/env bash
# Weak-contract gate (#81): run scripts/check_contract_quality.py over the contracts of each
# entry point `vow contracts` is asked about. full_test.sh Section 8c calls this script too, and
# .github/workflows/native-fixtures.yml runs it on every pull request (#1795 reached `main` without it).
# One run per entry point: `vow contracts` follows `use` edges, so an entry point covers its own
# module graph and nothing else. `compiler/module_io.vow` is deliberately not in main.vow's `use`
# graph (.vmod parity infrastructure), so it is listed on its own. See the scope note in
# check_contract_quality.py for why the corpus dirs stay ungated.
#
# Usage: check_contract_quality.sh <vowc>
# Prints `PASS|FAIL contract-quality/weak-gate:<entry> [— <reason>]` per entry point and exits 1
# when any line is FAIL. VOW_ULIMIT_KB, when set, caps the `vowc contracts` runs' virtual memory.
set -uo pipefail

if [ $# -ne 1 ] || [ ! -x "$1" ]; then
    echo "usage: check_contract_quality.sh <vowc>" >&2
    exit 2
fi
vowc=$(realpath "$1")
cd "$(dirname "$0")/.."

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

fails=0
for entry in compiler/main.vow compiler/module_io.vow; do
    case_name="contract-quality/weak-gate:$(basename "$entry" .vow)"
    json="$tmp/$(basename "$entry" .vow).json"
    # Capture the contracts JSON in its own step so a producer failure (parse error, missing
    # binary, compiler crash) is reported as itself, with its stderr visible, instead of being
    # masked as a baseline breach by the checker's empty-stdin exit.
    if ! (if [ -n "${VOW_ULIMIT_KB:-}" ]; then ulimit -v "$VOW_ULIMIT_KB"; fi; "$vowc" contracts "$entry") >"$json"; then
        echo "FAIL $case_name — vow contracts $entry failed (see stderr above); could not evaluate contract quality"
        fails=$((fails + 1))
        continue
    fi
    # Exit codes of the checker: 0 = pass, 1 = baseline breach (a real contract-quality
    # regression), 2 = structural error (malformed JSON / missing or non-integer counter; the
    # checker's stderr names the cause). A bare else would mislabel a schema error as a breach.
    status=0
    python3 scripts/check_contract_quality.py --label "$entry" <"$json" || status=$?
    if [ "$status" -eq 0 ]; then
        echo "PASS $case_name"
    elif [ "$status" -eq 1 ]; then
        echo "FAIL $case_name — weak/tautological contracts exceeded baseline in $entry; strengthen the new contract or adjust scripts/check_contract_quality.py with justification"
        fails=$((fails + 1))
    else
        echo "FAIL $case_name — contract quality check could not run for $entry (malformed 'vow contracts' output / schema mismatch; see stderr above)"
        fails=$((fails + 1))
    fi
done

[ "$fails" -eq 0 ]
