#!/usr/bin/env bash
set -uo pipefail

# Proven callees whose claims are not decided again (#1803, ADR-2026-10-16-1800 addendum): each
# fixture is decided with the default size and with `VOW_VERIFY_TRUST_ROOT_MIN=1`, which makes
# every caller leave the claims of its callees to their proofs. Both runs must give the verdict
# `// TEST: trust-expect` names (`proven`: Verified, `failed`: VerifyFailed) and the same
# counterexamples.
#
# Usage: run.sh <vowc> [fixture.vow ...]

VOWC="$1"
shift
DIR="$(cd "$(dirname "$0")" && pwd)"
if [ "$#" -eq 0 ]; then set -- "$DIR"/*.vow; fi
failures=0

summary_of() {
    python3 -c "
import json, sys
j = json.loads(sys.argv[1])
cx = sorted((c.get('function', '?'), c.get('vow_id', -1)) for c in (j.get('counterexamples') or []))
print(j.get('status', ''), cx)" "$1" 2>/dev/null
}

for f in "$@"; do
    name=$(basename "$f" .vow)
    want=$(sed -n 's|^// TEST: trust-expect \(.*\)$|\1|p' "$f" | head -1)
    case "$want" in
        proven) status=Verified ;;
        failed) status=VerifyFailed ;;
        *) echo "FAIL trust/$name: missing or unknown '// TEST: trust-expect' directive"; failures=$((failures + 1)); continue ;;
    esac
    errors=()
    plain=$(summary_of "$("$VOWC" verify --backend native --no-cache "$f" 2>/dev/null)")
    forced=$(summary_of "$(VOW_VERIFY_TRUST_ROOT_MIN=1 "$VOWC" verify --backend native --no-cache "$f" 2>/dev/null)")
    case "$plain" in
        "$status"*) ;;
        *) errors+=("default run: expected $status, got '$plain'") ;;
    esac
    [ "$plain" = "$forced" ] || errors+=("trusted run differs from the default one: '$forced' against '$plain'")
    if [ ${#errors[@]} -eq 0 ]; then
        echo "PASS trust/$name"
    else
        echo "FAIL trust/$name: $(IFS='; '; echo "${errors[*]}")"
        failures=$((failures + 1))
    fi
done
[ "$failures" -eq 0 ]
