#!/usr/bin/env bash
# Check the `// TEST:` directives of tests/verify-native fixtures against `vowc verify --backend
# native` (#1408). It runs on every pull request through .github/workflows/native-fixtures.yml, so a
# wrong directive in a new fixture fails its own PR. full_test.sh Section 4g, too large a file to
# edit under the file token budget, keeps its own copy of this loop; a unit test
# (scripts/test_check_native_fixtures.py) fails when the two read different directives.
#
#   pass/     Verified (exit 0); `warning` / `no-warning` directives
#   fail/     VerifyFailed (exit 1); `counterexample-{fn,vow-id,blame,violation,value}` and
#             `replay-skipped` directives, and `--replay-cex` must confirm every counterexample
#   unknown/  VerifyFailed with verify_status "unknown", no counterexample, and an
#             `unknown-reason` (default `unwinding assertion`) message
#   skip/     Skipped (exit 1) with the `skip-reason` code on every skipped function
#   contracts/ `vowc contracts --verify`: `contract-exit`, `contract-status`, `contract-trivial`
#
# Every fixture except skip/ must also report byte-identical JSON with VOW_VERIFY_NO_SLICE=1,
# with VOW_VERIFY_NO_INCREMENTAL=1, and on a cold and a warm result cache. pass/, fail/, unknown/
# and contracts/ need `bitwuzla` on PATH (skipped, not passed, without it); skip/ needs no solver.
#
# Usage: check_native_fixtures.sh [--dirs pass,fail,unknown,skip,contracts] [--filter SUBSTRING]
#                                 [--jobs N] [--require-solver] <vowc>
# Prints `PASS|FAIL|SKIP <name>[ — <reason>]` per fixture in directory then glob order, and exits
# 1 when any line is FAIL. With --require-solver a missing `bitwuzla` is a FAIL, not a SKIP. The
# `verify-native/cache-*` lines pin that the cold/warm comparison was not vacuous. VOW_ULIMIT_KB,
# when set, caps each vowc run's virtual memory (as in full_test.sh).
set -uo pipefail

if [ "${1:-}" = "--worker" ]; then
    shift
    WORKER=1
    VOWC="$1" OUT="$2" vow_file="$3"
else
    WORKER=0
fi

# ─── One fixture ────────────────────────────────────────────────────

run_vowc() {
    (if [ -n "${VOW_ULIMIT_KB:-}" ]; then ulimit -v "$VOW_ULIMIT_KB"; fi; "$VOWC" "$@")
}

json_field() {
    python3 -c "import json,sys; print(json.loads(sys.argv[1]).get(sys.argv[2],''))" "$1" "$2" 2>/dev/null
}

check_contracts_fixture() {
    native_json="" native_exit=0
    native_json=$(run_vowc contracts --verify --backend native --no-cache "$vow_file" 2>/dev/null) || native_exit=$?
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
}

check_status_fixture() {
    case "$native_dir" in
        pass) native_want_status="Verified"; native_want_exit=0 ;;
        fail|unknown) native_want_status="VerifyFailed"; native_want_exit=1 ;;
        *) native_want_status="Skipped"; native_want_exit=1 ;;
    esac
    native_json="" native_exit=0
    native_json=$(run_vowc verify --backend native --no-cache "$vow_file" 2>/dev/null) || native_exit=$?
    native_status=$(json_field "$native_json" status) || native_status=""
    if [ "$native_status" != "$native_want_status" ] || [ "$native_exit" -ne "$native_want_exit" ]; then
        native_errors+=("expected $native_want_status (exit $native_want_exit), got '$native_status' (exit $native_exit)")
        return
    fi
    case "$native_dir" in
        skip) check_skip_directives ;;
        unknown) check_unknown_directives ;;
        fail) check_fail_directives ;;
        pass) check_pass_directives ;;
    esac
    if [ "$native_dir" != "skip" ]; then check_stability; fi
}

check_skip_directives() {
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
}

# An open unwinding assertion (or, with `// TEST: unknown-reason <text>`, another named reason
# such as an opaque builtin result) is inconclusive: `unknown`, never a proof and never a
# counterexample.
check_unknown_directives() {
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
}

check_fail_directives() {
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
    # The harness writes temporary files next to the fixture under names that depend only on the
    # function and clause, so concurrent workers replay a private copy.
    native_replay_dir="$OUT/replay.$native_key"
    mkdir -p "$native_replay_dir"
    cp "$vow_file" "$native_replay_dir/"
    native_replay_json=$(run_vowc verify --backend native --no-cache --replay-cex "$native_replay_dir/$name.vow" 2>/dev/null) || true
    rm -rf "$native_replay_dir"
    native_replay=$(python3 -c "
import json, sys
j = json.loads(sys.argv[1])
cx = j.get('counterexamples') or []
bad = [c.get('function', '?') + '=' + str(c.get('replay', '')) for c in cx if c.get('replay') != 'confirmed']
print(','.join(bad) if bad else ('confirmed' if cx else 'none'))
" "$native_replay_json" 2>/dev/null) || native_replay=""
    # `// TEST: replay-skipped <reason>`: the self-hosted harness rebuilds only i64/u64/bool
    # scalars and struct, enum, Option and Result parameters of such scalars, and shift counts
    # are not runtime aborts, so a fixture outside that must say why its counterexample cannot
    # be confirmed.
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
}

# `// TEST: warning <Code>` / `// TEST: no-warning <Code>`: a proof may carry a warning (a
# reachable checked-arithmetic abort) or must not.
check_pass_directives() {
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
}

# Query slicing (#1424) and incremental solver sessions (#1428) must never change what the
# verifier reports: the run with each switched off (test-only knobs; one solver process per
# claim for the latter) is byte-identical. The result cache (#1429) must not either: a cold run
# on an empty cache and a warm run that replays it equal the --no-cache baseline in JSON and
# exit status.
check_stability() {
    for native_knob in VOW_VERIFY_NO_SLICE VOW_VERIFY_NO_INCREMENTAL; do
        native_alt_json="" native_alt_exit=0
        native_alt_json=$(export "$native_knob=1"; run_vowc verify --backend native --no-cache "$vow_file" 2>/dev/null) || native_alt_exit=$?
        if [ "$native_alt_json" != "$native_json" ] || [ "$native_alt_exit" -ne "$native_exit" ]; then
            native_errors+=("$native_knob=1 changed the result: runs with and without it differ")
        fi
    done
    native_cache_dir=$(mktemp -d "$OUT/native-cache.XXXXXX")
    for native_pass in cold warm; do
        native_cache_json="" native_cache_exit=0
        native_cache_json=$(VOW_CACHE_DIR="$native_cache_dir" run_vowc verify --backend native "$vow_file" 2>/dev/null) || native_cache_exit=$?
        if [ "$native_cache_json" != "$native_json" ] || [ "$native_cache_exit" -ne "$native_exit" ]; then
            native_errors+=("result cache changed the result: $native_pass run differs from --no-cache")
        fi
    done
    find "$native_cache_dir" -type f 2>/dev/null | wc -l | tr -d ' ' >"$OUT/$native_key.cache"
    rm -rf "$native_cache_dir"
}

run_worker() {
    native_dir=$(basename "$(dirname "$vow_file")")
    name=$(basename "$vow_file" .vow)
    native_key="${native_dir}__${name}"
    native_errors=()
    if [ "$native_dir" = "contracts" ]; then check_contracts_fixture; else check_status_fixture; fi
    {
        if [ ${#native_errors[@]} -eq 0 ]; then
            echo "PASS verify-native/${native_dir}/${name}"
        else
            native_joined=$(IFS='; '; echo "${native_errors[*]}")
            echo "FAIL verify-native/${native_dir}/${name} — ${native_joined//$'\n'/ }"
        fi
    } >"$OUT/$native_key.tmp"
    mv "$OUT/$native_key.tmp" "$OUT/$native_key.res"
}

if [ "$WORKER" -eq 1 ]; then
    run_worker
    exit 0
fi

# ─── Driver ─────────────────────────────────────────────────────────

dirs="pass,fail,unknown,skip,contracts"
filter=""
jobs=1
require_solver=0
vowc=""
while [ $# -gt 0 ]; do
    case "$1" in
        --dirs) dirs="${2:?--dirs needs a value}"; shift 2 ;;
        --filter) filter="${2:?--filter needs a value}"; shift 2 ;;
        --jobs) jobs="${2:?--jobs needs a value}"; shift 2 ;;
        --require-solver) require_solver=1; shift ;;
        -h|--help) sed -n '2,/^set -uo/p' "$0" | sed '$d;s/^# \{0,1\}//'; exit 0 ;;
        -*) echo "check_native_fixtures.sh: unknown option '$1'" >&2; exit 2 ;;
        *) vowc="$1"; shift ;;
    esac
done
if [ -z "$vowc" ] || [ ! -x "$vowc" ]; then
    echo "usage: check_native_fixtures.sh [--dirs LIST] [--filter SUBSTRING] [--jobs N] [--require-solver] <vowc>" >&2
    exit 2
fi
if ! [[ "$jobs" =~ ^[1-9][0-9]*$ ]]; then
    echo "check_native_fixtures.sh: --jobs must be a positive integer (got '$jobs')" >&2
    exit 2
fi
vowc=$(realpath "$vowc")
self=$(realpath "$0")
cd "$(dirname "$self")/.."

have_solver=0
command -v bitwuzla >/dev/null 2>&1 && have_solver=1

out=$(mktemp -d)
workers=""
cleanup() {
    if [ -n "$workers" ]; then kill "$workers" 2>/dev/null; fi
    rm -rf "$out"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

fixtures=()
empty_dirs=()
skipped_contracts=0
skipped_status=0
needs_cache_check=0
IFS=, read -r -a dir_list <<<"$dirs"
for native_dir in "${dir_list[@]}"; do
    dir_found=0
    for vow_file in tests/verify-native/"$native_dir"/*.vow; do
        [ -f "$vow_file" ] || continue
        dir_found=1
        case "$vow_file" in *"$filter"*) ;; *) continue ;; esac
        if [ "$native_dir" != "skip" ] && [ "$have_solver" -eq 0 ]; then
            if [ "$native_dir" = "contracts" ]; then skipped_contracts=1; else skipped_status=1; fi
            continue
        fi
        case "$native_dir" in skip|contracts) ;; *) needs_cache_check=1 ;; esac
        fixtures+=("$vow_file")
    done
    if [ "$dir_found" -eq 0 ]; then empty_dirs+=("$native_dir"); fi
done

fails=0
# skip/ fixtures contain only functions outside the subset, so they need no solver and keep
# guarding "never Verified" even without one. `--require-solver` (CI) turns a missing solver into
# a failure, so a broken install cannot make the checks vacuous.
for native_dir in ${empty_dirs[@]+"${empty_dirs[@]}"}; do
    echo "FAIL verify-native/$native_dir — no fixtures found in tests/verify-native/$native_dir"
    fails=$((fails + 1))
done
solver_missing() {
    if [ "$require_solver" -eq 1 ]; then
        echo "FAIL verify-native/$1 — bitwuzla not on PATH (--require-solver)"
        fails=$((fails + 1))
    else
        echo "SKIP verify-native/$1 — bitwuzla not on PATH"
    fi
}
if [ "$skipped_contracts" -eq 1 ]; then solver_missing "contracts fixtures"; fi
if [ "$skipped_status" -eq 1 ]; then solver_missing "pass+fail fixtures"; fi

if [ ${#fixtures[@]} -gt 0 ]; then
    printf '%s\0' "${fixtures[@]}" |
        VOW_ULIMIT_KB="${VOW_ULIMIT_KB:-}" xargs -0 -n1 -P "$jobs" bash "$self" --worker "$vowc" "$out" &
    workers=$!
    for vow_file in "${fixtures[@]}"; do
        native_dir=$(basename "$(dirname "$vow_file")")
        name=$(basename "$vow_file" .vow)
        result="$out/${native_dir}__${name}.res"
        while [ ! -e "$result" ]; do
            if ! kill -0 "$workers" 2>/dev/null && [ ! -e "$result" ]; then
                echo "FAIL verify-native/${native_dir}/${name} — the checker died before reporting"
                fails=$((fails + 1))
                continue 2
            fi
            sleep 0.2
        done
        cat "$result"
        case "$(cut -c1-4 "$result")" in FAIL) fails=$((fails + 1)) ;; esac
    done
    wait "$workers"
fi

# Non-vacuity of the cache comparison: a solver whose `--version` probe yields nothing silently
# disables the cache, which would make cold == warm trivially. With a real solver the corpus must
# have stored something.
if [ "$have_solver" -eq 1 ] && [ "$needs_cache_check" -eq 1 ]; then
    cache_total=$(cat "$out"/*.cache 2>/dev/null | awk '{ n += $1 } END { print n + 0 }')
    if [ -z "$(bitwuzla --version 2>/dev/null | tr -d '[:space:]')" ]; then
        echo "FAIL verify-native/cache-probe — bitwuzla --version printed nothing, so the result cache is disabled"
        fails=$((fails + 1))
    elif [ "${cache_total:-0}" -eq 0 ]; then
        echo "FAIL verify-native/cache-stored — no fixture stored a cache entry; the cold/warm comparison was vacuous"
        fails=$((fails + 1))
    else
        echo "PASS verify-native/cache-stored"
    fi
fi

[ "$fails" -eq 0 ]
