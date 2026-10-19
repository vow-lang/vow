# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 4g: Native Verifier (tests/verify-native/, #1408) ────
#
# `vowc verify --backend native` exists only in the self-hosted compiler (epic
# #1398, ADR-2026-10-08-1421), so there is no Rust twin and no parity check:
# each fixture is asserted against its directory (pass/ -> Verified, fail/ ->
# VerifyFailed with the `counterexample-*` directives, skip/ -> Skipped, never
# Verified, unknown/ -> VerifyFailed with `verify_status: "unknown"`, no
# counterexample and an `unwinding assertion:` message (or the `// TEST: unknown-reason`
# text), never Verified; each
# fixture is also run with VOW_VERIFY_NO_SLICE=1 and VOW_VERIFY_NO_INCREMENTAL=1
# and must report byte-identical JSON). The
# wiring tier uses a fake Bitwuzla and always runs; the real-solver tier is
# skipped, not passed, when `bitwuzla` is not on PATH.
# tests/verify-native/ is deliberately outside Section 2c's explicit globs.

section_begin "Section 4g: Native Verifier (self-hosted only)"
native_wiring_log="$TMPDIR/verify-native-wiring.log"
if VOWC_BIN="$SELF" bash tests/verify-native/tests.sh >"$native_wiring_log" 2>&1; then
    pass "verify-native/wiring"
else
    fail "verify-native/wiring" "$(tail -20 "$native_wiring_log")"
fi

native_have_solver=0
native_cache_total=0
command -v bitwuzla >/dev/null 2>&1 && native_have_solver=1
if [ "$native_have_solver" -eq 0 ]; then
    skip "verify-native/pass+fail fixtures" "bitwuzla not on PATH"
fi
# skip/ fixtures contain only functions outside the subset, so they need no
# solver and keep guarding "never Verified" even without one.
for vow_file in tests/verify-native/pass/*.vow tests/verify-native/fail/*.vow tests/verify-native/unknown/*.vow tests/verify-native/skip/*.vow; do
    [ -f "$vow_file" ] || continue
    name=$(basename "$vow_file" .vow)
    native_dir=$(basename "$(dirname "$vow_file")")
    if [ "$native_dir" != "skip" ] && [ "$native_have_solver" -eq 0 ]; then
        continue
    fi
    case "$native_dir" in
        pass) native_want_status="Verified"; native_want_exit=0 ;;
        fail|unknown) native_want_status="VerifyFailed"; native_want_exit=1 ;;
        *) native_want_status="Skipped"; native_want_exit=1 ;;
    esac
    native_json="" native_exit=0
    native_json=$(run_self verify --backend native --no-cache "$vow_file" 2>/dev/null) || native_exit=$?
    native_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$native_json" 2>/dev/null) || native_status=""
    if [ "$native_status" != "$native_want_status" ] || [ "$native_exit" -ne "$native_want_exit" ]; then
        fail "verify-native/${native_dir}/${name}" "expected $native_want_status (exit $native_want_exit), got '$native_status' (exit $native_exit)"
        continue
    fi
    native_errors=()
    if [ "$native_dir" = "skip" ]; then
        native_reason=$(sed -n 's|^// TEST: skip-reason \(.*\)$|\1|p' "$vow_file" | head -1)
        if [ -z "$native_reason" ]; then
            native_errors+=("missing '// TEST: skip-reason <code>' directive")
        elif ! python3 -c "
import json, sys
msgs = [d.get('message', '') for d in json.loads(sys.argv[1]).get('diagnostics') or []]
skipped = [m for m in msgs if m.startswith('skipped verification of')]
sys.exit(0 if skipped and all(': ' + sys.argv[2] + ': ' in m for m in skipped) else 1)
" "$native_json" "$native_reason" 2>/dev/null; then
            native_errors+=("skip-reason: not every skipped-verification message carries code '$native_reason'")
        fi
    fi
    if [ "$native_dir" = "unknown" ]; then
        # An open unwinding assertion (or, with `// TEST: unknown-reason <text>`,
        # another named reason such as an opaque builtin result) is inconclusive:
        # `unknown`, never a proof and never a counterexample.
        native_unknown_reason=$(sed -n 's|^// TEST: unknown-reason \(.*\)$|\1|p' "$vow_file" | head -1)
        [ -n "$native_unknown_reason" ] || native_unknown_reason="unwinding assertion"
        if ! python3 -c "
import json, sys
j = json.loads(sys.argv[1])
ok = (j.get('verify_status') == 'unknown'
      and not (j.get('counterexamples') or [])
      and sys.argv[2] in (j.get('verify_message') or ''))
sys.exit(0 if ok else 1)
" "$native_json" "$native_unknown_reason" 2>/dev/null; then
            native_errors+=("expected verify_status unknown with a '$native_unknown_reason' message and no counterexample")
        fi
    fi
    if [ "$native_dir" = "fail" ]; then
        for native_field in fn vow-id blame violation; do
            native_expected=$(sed -n "s|^// TEST: counterexample-${native_field} \"\?\([^\"]*\)\"\?\$|\1|p" "$vow_file" | head -1)
            [ -n "$native_expected" ] || continue
            native_actual=$(python3 -c "
import json, sys
cx = json.loads(sys.argv[1]).get('counterexamples') or []
key = {'fn': 'function', 'vow-id': 'vow_id', 'blame': 'blame', 'violation': 'violation'}[sys.argv[2]]
print(cx[0].get(key, '') if cx else '')
" "$native_json" "$native_field" 2>/dev/null) || native_actual="<unparseable>"
            if [ "$native_actual" != "$native_expected" ]; then
                native_errors+=("counterexample-${native_field}: expected '$native_expected', got '$native_actual'")
            fi
        done
        while read -r native_name native_value; do
            [ -n "$native_name" ] || continue
            native_actual=$(python3 -c "
import json, sys
cx = json.loads(sys.argv[1]).get('counterexamples') or []
print((cx[0].get('values') or {}).get(sys.argv[2], '<absent>') if cx else '<no counterexample>')
" "$native_json" "$native_name" 2>/dev/null) || native_actual="<unparseable>"
            if [ "$native_actual" != "$native_value" ]; then
                native_errors+=("counterexample-value ${native_name}: expected '$native_value', got '$native_actual'")
            fi
        done < <(sed -n 's|^// TEST: counterexample-value \([^ ]*\) \(.*\)$|\1 \2|p' "$vow_file")
        native_replay_json=$(run_self verify --backend native --no-cache --replay-cex "$vow_file" 2>/dev/null) || true
        native_replay=$(python3 -c "
import json, sys
j = json.loads(sys.argv[1])
cx = j.get('counterexamples') or []
bad = [c.get('function', '?') + '=' + str(c.get('replay', '')) for c in cx if c.get('replay') != 'confirmed']
print(','.join(bad) if bad else ('confirmed' if cx else 'none'))
" "$native_replay_json" 2>/dev/null) || native_replay=""
        # `// TEST: replay-skipped <reason>`: the self-hosted harness rebuilds only
        # i64/u64/bool scalars and struct, enum, Option and Result parameters of
        # such scalars, and shift counts are not runtime aborts, so a fixture
        # outside that must say why its counterexample cannot be confirmed.
        native_replay_want="confirmed"
        if grep -q '^// TEST: replay-skipped ' "$vow_file"; then native_replay_want="skipped"; fi
        if [ "$native_replay_want" = "skipped" ] && [ -n "$native_replay" ]; then
            native_replay=$(python3 -c "
import json, sys
cx = json.loads(sys.argv[1]).get('counterexamples') or []
print('skipped' if cx and all(c.get('replay') == 'skipped' for c in cx) else 'not-skipped')
" "$native_replay_json" 2>/dev/null) || native_replay=""
        fi
        if [ "$native_replay" != "$native_replay_want" ]; then
            native_errors+=("--replay-cex: every counterexample must replay as $native_replay_want, got '$native_replay'")
        fi
    fi
    if [ "$native_dir" = "pass" ]; then
        # `// TEST: warning <Code>` / `// TEST: no-warning <Code>`: a proof may
        # carry a warning (a reachable checked-arithmetic abort) or must not.
        while read -r native_kind native_code; do
            [ -n "$native_kind" ] || continue
            native_count=$(python3 -c "
import json, sys
ds = json.loads(sys.argv[1]).get('diagnostics') or []
print(sum(1 for d in ds if d.get('error_code') == sys.argv[2]))
" "$native_json" "$native_code" 2>/dev/null) || native_count=-1
            if [ "$native_kind" = "warning" ] && [ "$native_count" -lt 1 ]; then
                native_errors+=("expected a $native_code diagnostic, got none")
            elif [ "$native_kind" = "no-warning" ] && [ "$native_count" -ne 0 ]; then
                native_errors+=("expected no $native_code diagnostic, got $native_count")
            fi
        done < <(sed -nE 's#^// TEST: (warning|no-warning) (.*)$#\1 \2#p' "$vow_file")
    fi
    if [ "$native_dir" != "skip" ]; then
        # Query slicing (#1424) and incremental solver sessions (#1428) must
        # never change what the verifier reports: the run with each switched off
        # (test-only knobs; one solver process per claim for the latter) is
        # byte-identical.
        for native_knob in VOW_VERIFY_NO_SLICE VOW_VERIFY_NO_INCREMENTAL; do
            native_alt_json="" native_alt_exit=0
            native_alt_json=$(export "$native_knob=1"; run_self verify --backend native --no-cache "$vow_file" 2>/dev/null) || native_alt_exit=$?
            if [ "$native_alt_json" != "$native_json" ] || [ "$native_alt_exit" -ne "$native_exit" ]; then
                native_errors+=("$native_knob=1 changed the result: runs with and without it differ")
            fi
        done
        # The result cache (#1429) must never change what the verifier reports:
        # a cold run on an empty cache and a warm run that replays it equal the
        # --no-cache baseline in JSON and exit status.
        native_cache_dir=$(mktemp -d "$TMPDIR/native-cache.XXXXXX")
        for native_pass in cold warm; do
            native_cache_json="" native_cache_exit=0
            native_cache_json=$(VOW_CACHE_DIR="$native_cache_dir" run_self verify --backend native "$vow_file" 2>/dev/null) || native_cache_exit=$?
            if [ "$native_cache_json" != "$native_json" ] || [ "$native_cache_exit" -ne "$native_exit" ]; then
                native_errors+=("result cache changed the result: $native_pass run differs from --no-cache")
            fi
        done
        native_cache_entries=$(find "$native_cache_dir" -type f 2>/dev/null | wc -l | tr -d ' ')
        native_cache_total=$((native_cache_total + native_cache_entries))
        rm -rf "$native_cache_dir"
    fi
    if [ ${#native_errors[@]} -eq 0 ]; then
        pass "verify-native/${native_dir}/${name}"
    else
        fail "verify-native/${native_dir}/${name}" "$(IFS='; '; echo "${native_errors[*]}")"
    fi
done
# Callee summaries (#1791): tests/verify-native/summary/run.sh decides each fixture with the
# inlining budget lowered so a proven callee is summarised, and with the default budget. A run
# that prints fewer lines than there are fixtures (a crash, a missing binary) fails.
if [ "$native_have_solver" -eq 0 ]; then
    skip "verify-native/summary fixtures" "bitwuzla not on PATH"
else
    summary_want=$(find tests/verify-native/summary -maxdepth 1 -name '*.vow' | wc -l)
    summary_got=0
    while IFS= read -r summary_line; do
        case "$summary_line" in
            PASS\ *) pass "verify-native/${summary_line#PASS }"; summary_got=$((summary_got + 1)) ;;
            FAIL\ *) fail "verify-native/${summary_line#FAIL }" "see tests/verify-native/summary/run.sh"; summary_got=$((summary_got + 1)) ;;
        esac
    done < <(bash tests/verify-native/summary/run.sh "$SELF" 2>/dev/null)
    if [ "$summary_got" -ne "$summary_want" ]; then
        fail "verify-native/summary" "run.sh reported $summary_got of $summary_want fixtures"
    fi
fi
# Proven callees whose claims are not decided again (#1803): tests/verify-native/trust/run.sh decides
# each fixture with the default size and with every caller waiting for its callees' proofs; both
# must give the verdict and counterexamples the fixture names.
if [ "$native_have_solver" -eq 0 ]; then
    skip "verify-native/trust fixtures" "bitwuzla not on PATH"
else
    trust_want=$(find tests/verify-native/trust -maxdepth 1 -name '*.vow' | wc -l)
    trust_got=0
    while IFS= read -r trust_line; do
        case "$trust_line" in
            PASS\ *) pass "verify-native/${trust_line#PASS }"; trust_got=$((trust_got + 1)) ;;
            FAIL\ *) fail "verify-native/${trust_line#FAIL }" "see tests/verify-native/trust/run.sh"; trust_got=$((trust_got + 1)) ;;
        esac
    done < <(bash tests/verify-native/trust/run.sh "$SELF" 2>/dev/null)
    if [ "$trust_got" -ne "$trust_want" ]; then
        fail "verify-native/trust" "run.sh reported $trust_got of $trust_want fixtures"
    fi
fi
# Every code of the closed `Skipped` list is pinned by a skip/ fixture (or an
# explicit exemption) and the list matches ADR-1430 section 4.
skip_codes_log="$TMPDIR/verify-native-skip-codes.log"
if python3 scripts/check_skip_codes.py >"$skip_codes_log" 2>&1; then
    pass "verify-native/skip-code-coverage"
else
    fail "verify-native/skip-code-coverage" "$(cat "$skip_codes_log")"
fi
# Non-vacuity of the cache comparison above: a solver whose `--version` probe
# yields nothing silently disables the cache, which would make cold == warm
# trivially. With a real solver the corpus must have stored something.
if [ "$native_have_solver" -eq 1 ]; then
    if [ -z "$(bitwuzla --version 2>/dev/null | tr -d '[:space:]')" ]; then
        fail "verify-native/cache-probe" "bitwuzla --version printed nothing, so the result cache is disabled"
    elif [ "$native_cache_total" -eq 0 ]; then
        fail "verify-native/cache-stored" "no fixture stored a cache entry; the cold/warm comparison was vacuous"
    else
        pass "verify-native/cache-stored"
    fi
fi


# `vowc contracts --verify --backend native` (#1427): each fixture in
# tests/verify-native/contracts/ pins the clause statuses and the weak-contract
# probes (`vacuous`, `trivially_satisfiable`) the native pool must report.
#   // TEST: contract-exit <n>
#   // TEST: contract-status <fn> <kind> <status>   (some clause has that status)
#   // TEST: contract-trivial <fn> <true|false>     (every `ensures` of fn)
# Every fixture needs the solver, so the tier is skipped without one.
if [ "$native_have_solver" -eq 0 ]; then
    skip "verify-native/contracts fixtures" "bitwuzla not on PATH"
else
    for vow_file in tests/verify-native/contracts/*.vow; do
        [ -f "$vow_file" ] || continue
        name=$(basename "$vow_file" .vow)
        native_json="" native_exit=0
        native_json=$(run_self contracts --verify --backend native --no-cache "$vow_file" 2>/dev/null) || native_exit=$?
        native_errors=()
        native_want_exit=$(sed -n 's|^// TEST: contract-exit \(.*\)$|\1|p' "$vow_file" | head -1)
        if [ -z "$native_want_exit" ]; then
            native_errors+=("missing '// TEST: contract-exit <n>' directive")
        elif [ "$native_exit" != "$native_want_exit" ]; then
            native_errors+=("exit: expected $native_want_exit, got $native_exit")
        fi
        while read -r native_fn native_kind native_want; do
            [ -n "$native_fn" ] || continue
            native_hit=$(python3 -c "
import json, sys
cs = json.loads(sys.argv[1])['contracts']
print(sum(1 for c in cs if c['function'] == sys.argv[2] and c['kind'] == sys.argv[3] and c['status'] == sys.argv[4]))
" "$native_json" "$native_fn" "$native_kind" "$native_want" 2>/dev/null) || native_hit=-1
            if [ "$native_hit" -lt 1 ]; then
                native_errors+=("contract-status ${native_fn} ${native_kind}: no clause is '$native_want'")
            fi
        done < <(sed -n 's|^// TEST: contract-status \(.*\)$|\1|p' "$vow_file")
        while read -r native_fn native_want; do
            [ -n "$native_fn" ] || continue
            native_bad=$(python3 -c "
import json, sys
cs = json.loads(sys.argv[1])['contracts']
want = sys.argv[3] == 'true'
rows = [c for c in cs if c['function'] == sys.argv[2] and c['kind'] == 'ensures']
print(len([c for c in rows if c['trivially_satisfiable'] != want]) if rows else -1)
" "$native_json" "$native_fn" "$native_want" 2>/dev/null) || native_bad=-1
            if [ "$native_bad" -ne 0 ]; then
                native_errors+=("contract-trivial ${native_fn}: expected every ensures clause to be $native_want")
            fi
        done < <(sed -n 's|^// TEST: contract-trivial \(.*\)$|\1|p' "$vow_file")
        if [ ${#native_errors[@]} -eq 0 ]; then
            pass "verify-native/contracts/${name}"
        else
            fail "verify-native/contracts/${name}" "$(IFS='; '; echo "${native_errors[*]}")"
        fi
    done
fi
echo ""
