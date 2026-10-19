#!/usr/bin/env bash
set -uo pipefail

# Cost of reading a pointer element back (#1825, ADR-2026-10-08-1422 addendum): each fixture is
# decided with the real solver, and its verdict and wall time must be the ones
# `// TEST: perf-expect <proven|unknown> <seconds>` names. The bound is generous (a loaded CI runner
# is several times slower than a workstation) and still far below what the shapes cost before the
# addresses of stored objects were one symbol and a constant apart and the chain of a stored element was
# exact: 32 s for the wide grid, the whole 300 s budget for the symbolic loop.
#
#   proven   Verified
#   unknown  VerifyFailed, verify_status unknown, no counterexample
#
# Usage: run.sh <vowc> [fixture.vow ...]

VOWC="$1"
shift
DIR="$(cd "$(dirname "$0")" && pwd)"
if [ "$#" -eq 0 ]; then set -- "$DIR"/*.vow; fi
failures=0

for f in "$@"; do
    name=$(basename "$f" .vow)
    want=$(sed -n 's|^// TEST: perf-expect \([a-z]*\) [0-9]*$|\1|p' "$f" | head -1)
    limit=$(sed -n 's|^// TEST: perf-expect [a-z]* \([0-9]*\)$|\1|p' "$f" | head -1)
    if [ -z "$want" ] || [ -z "$limit" ]; then
        echo "FAIL perf/$name: missing '// TEST: perf-expect <proven|unknown> <seconds>' directive"
        failures=$((failures + 1))
        continue
    fi
    start=$(date +%s.%N)
    json=$("$VOWC" verify --backend native --no-cache "$f" 2>/dev/null)
    end=$(date +%s.%N)
    problem=$(printf '%s' "$json" | python3 -c "
import json, sys
j = json.load(sys.stdin)
want, secs, limit = sys.argv[1], float(sys.argv[2]), float(sys.argv[3])
if want == 'proven':
    ok = j.get('status') == 'Verified'
else:
    ok = (j.get('status') == 'VerifyFailed' and j.get('verify_status') == 'unknown'
          and not (j.get('counterexamples') or []))
if not ok:
    print('expected %s, got %s/%s' % (want, j.get('status'), j.get('verify_status')))
elif secs > limit:
    print('took %.1f s, the bound is %d s' % (secs, limit))
" "$want" "$(python3 -c "print($end - $start)")" "$limit" 2>&1)
    if [ -n "$problem" ]; then
        echo "FAIL perf/$name: $problem"
        failures=$((failures + 1))
    else
        echo "PASS perf/$name"
    fi
done
[ "$failures" -eq 0 ]
