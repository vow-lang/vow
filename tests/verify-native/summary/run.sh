#!/usr/bin/env bash
set -uo pipefail

# Callee summaries (#1791, ADR-2026-10-16-1800): each fixture is run twice with the
# real solver. With the test-only `VOW_VERIFY_INLINE_BUDGET` (`// TEST: summary-budget N`,
# default 60) the call tree is above the budget and a proven callee is summarised; the
# result must be the one `// TEST: summary-expect` names:
#
#   proven    Verified
#   unknown   VerifyFailed, verify_status unknown, "callee summary" message, no counterexample
#   skipped   Skipped, never Verified (the callee has no summary, or is not proven)
#
# Without the variable the function is inlined, and `// TEST: summary-baseline`
# (proven | failed | skipped; default: the same as the expectation) is the verdict of
# that run, which shows the fixture's outcome is the summary's doing.
#
# The summary run prints a `Wave` line on stderr for every round of callee proofs it starts: a
# `proven` or `unknown` result is the summary's doing, so it must have one. `// TEST: summary-no-waves`
# (with `// TEST: summary-trust-root-min N`, which lowers the size from which a callee is worth a
# proof) asserts the opposite: no callee of the function can be summarised, so no proof could change
# its skip and none is started.
#
# Run a third time with the summaries switched off (`VOW_VERIFY_NO_SUMMARIES=1`) at the lowered
# budget: the function must be `Skipped`, which shows the verdict above is the summary's doing.
# A fixture whose baseline is `failed` is also run with `--replay-cex` and every counterexample
# must be confirmed by a real run (`// TEST: replay-skipped` when the harness cannot rebuild its
# inputs), so a claim the summary run leaves open is false, not merely hard.
#
# Usage: run.sh <vowc> [fixture.vow ...]

VOWC="$1"
shift
DIR="$(cd "$(dirname "$0")" && pwd)"
if [ "$#" -eq 0 ]; then set -- "$DIR"/*.vow; fi
failures=0

status_of() {
    python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$1" 2>/dev/null
}

for f in "$@"; do
    name=$(basename "$f" .vow)
    want=$(sed -n 's|^// TEST: summary-expect \(.*\)$|\1|p' "$f" | head -1)
    base=$(sed -n 's|^// TEST: summary-baseline \(.*\)$|\1|p' "$f" | head -1)
    budget=$(sed -n 's|^// TEST: summary-budget \(.*\)$|\1|p' "$f" | head -1)
    [ -n "$budget" ] || budget=60
    root_min=$(sed -n 's|^// TEST: summary-trust-root-min \(.*\)$|\1|p' "$f" | head -1)
    [ -n "$base" ] || base="$want"
    errors=()
    err=$(mktemp)
    json=$(env ${root_min:+VOW_VERIFY_TRUST_ROOT_MIN="$root_min"} VOW_VERIFY_INLINE_BUDGET="$budget" "$VOWC" verify --backend native --no-cache "$f" 2>"$err")
    st=$(status_of "$json")
    grep -q '^Wave ' "$err" && waved=1 || waved=0
    rm -f "$err"
    if grep -q '^// TEST: summary-no-waves' "$f" && [ "$waved" = 1 ]; then
        errors+=("summary run: a proof wave ran, though no summary can change the skip")
    fi
    if { [ "$want" = proven ] || [ "$want" = unknown ]; } && [ "$waved" = 0 ]; then
        errors+=("summary run: no proof wave ran, though a summary decided the function")
    fi
    case "$want" in
        proven) [ "$st" = "Verified" ] || errors+=("summary run: expected Verified, got '$st'") ;;
        unknown)
            python3 -c "
import json, sys
j = json.loads(sys.argv[1])
ok = (j.get('verify_status') == 'unknown' and not (j.get('counterexamples') or [])
      and 'callee summary' in (j.get('verify_message') or ''))
sys.exit(0 if ok else 1)" "$json" 2>/dev/null || errors+=("summary run: expected unknown with a 'callee summary' message, got '$st'") ;;
        skipped) [ "$st" = "Skipped" ] || errors+=("summary run: expected Skipped, got '$st'") ;;
        *) errors+=("missing or unknown '// TEST: summary-expect' directive") ;;
    esac
    json=$("$VOWC" verify --backend native --no-cache "$f" 2>/dev/null)
    st=$(status_of "$json")
    case "$base" in
        proven) [ "$st" = "Verified" ] || errors+=("baseline run: expected Verified, got '$st'") ;;
        failed|unknown) [ "$st" = "VerifyFailed" ] || errors+=("baseline run: expected VerifyFailed, got '$st'") ;;
        skipped) [ "$st" = "Skipped" ] || errors+=("baseline run: expected Skipped, got '$st'") ;;
    esac
    json=$(VOW_VERIFY_NO_SUMMARIES=1 VOW_VERIFY_INLINE_BUDGET="$budget" "$VOWC" verify --backend native --no-cache "$f" 2>/dev/null)
    st=$(status_of "$json")
    [ "$st" = "Skipped" ] || errors+=("no-summaries run: expected Skipped, got '$st'")
    if [ "$base" = "failed" ]; then
        json=$("$VOWC" verify --backend native --no-cache --replay-cex "$f" 2>/dev/null)
        replay=$(python3 -c "
import json, sys
cx = json.loads(sys.argv[1]).get('counterexamples') or []
want = sys.argv[2]
bad = [c.get('function', '?') + '=' + str(c.get('replay', '')) for c in cx if c.get('replay') != want]
print(','.join(bad) if bad or not cx else 'ok')" "$json" "$( grep -q '^// TEST: replay-skipped' "$f" && echo skipped || echo confirmed )" 2>/dev/null)
        [ "$replay" = "ok" ] || errors+=("--replay-cex: every counterexample must replay as expected, got '$replay'")
    fi
    if [ ${#errors[@]} -eq 0 ]; then
        echo "PASS summary/$name"
    else
        echo "FAIL summary/$name: $(IFS='; '; echo "${errors[*]}")"
        failures=$((failures + 1))
    fi
done
[ "$failures" -eq 0 ]
