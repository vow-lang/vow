#!/usr/bin/env bash
set -euo pipefail

# Vow self-hosted compiler test suite
# Usage: tests/run_tests.sh [--no-bootstrap] [--no-verify] [--filter <pattern>]

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
VOWC="$ROOT_DIR/build/vowc"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

# Counters
PASS=0
FAIL=0
SKIP=0
FAILURES=()

# Flags
NO_BOOTSTRAP=false
NO_VERIFY=false
FILTER=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-bootstrap) NO_BOOTSTRAP=true; shift ;;
    --no-verify)    NO_VERIFY=true; shift ;;
    --filter)       FILTER="$2"; shift 2 ;;
    *) echo "Unknown flag: $1"; exit 1 ;;
  esac
done

# --- Helpers ---

run_vowc() {
  ( ulimit -v 2000000; "$VOWC" "$@" )
}

run_vowc_with_tmpdir() {
  local tmp_root="$1"
  shift
  ( export TMPDIR="$tmp_root"; ulimit -v 2000000; "$VOWC" "$@" )
}

run_bin() {
  ( ulimit -v 2000000; "$@" )
}

pass() {
  local name="$1"
  PASS=$((PASS + 1))
  echo -e "  ${GREEN}PASS${RESET} $name"
}

fail() {
  local name="$1"
  local reason="$2"
  FAIL=$((FAIL + 1))
  FAILURES+=("$name: $reason")
  echo -e "  ${RED}FAIL${RESET} $name — $reason"
}

skip() {
  local name="$1"
  local reason="$2"
  SKIP=$((SKIP + 1))
  echo -e "  ${YELLOW}SKIP${RESET} $name — $reason"
}

json_field() {
  local json="$1"
  local field="$2"
  python3 -c "
import json, sys
try:
    d = json.loads(sys.stdin.read())
except Exception:
    print('')
    sys.exit(0)
print(d.get('$field', '') if isinstance(d, dict) else '')
" <<< "$json"
}

json_path_field() {
  local json="$1"
  local path="$2"
  JSON_PATH="$path" python3 -c "
import json
import os
import sys

cur = json.loads(sys.stdin.read())
for part in os.environ['JSON_PATH'].split('.'):
    if isinstance(cur, dict):
        cur = cur.get(part, '')
    else:
        cur = ''
        break
if isinstance(cur, (dict, list)):
    print(json.dumps(cur))
else:
    print(cur)
" <<< "$json"
}

json_cx_field() {
  local json="$1"
  local field="$2"
  python3 -c "
import json,sys
d=json.loads(sys.stdin.read())
cx=d.get('counterexamples',[])
if cx: print(cx[0].get('$field',''))
else: print('')
" <<< "$json"
}

count_verify_scratch_dirs() {
  local tmp_root="$1"
  find "$tmp_root" -maxdepth 1 -type d -name 'vow-verify.*' | wc -l | tr -d '[:space:]'
}

parse_annotations() {
  local file="$1"
  # Reset annotation vars
  TEST_EXIT=""
  TEST_STDOUT=""
  TEST_STDERR=""
  TEST_STATUS=""
  TEST_CX_FN=""
  TEST_CX_BLAME=""
  TEST_ERROR_CODE=""
  TEST_SKIP=""
  TEST_STDIN=""
  TEST_STDIN_FILE=""
  TEST_BUILD_JSON=""

  while IFS= read -r line; do
    if [[ "$line" =~ ^//\ TEST:\ exit\ ([0-9]+) ]]; then
      TEST_EXIT="${BASH_REMATCH[1]}"
    elif [[ "$line" =~ ^//\ TEST:\ stdout\ \"(.*)\" ]]; then
      TEST_STDOUT="${BASH_REMATCH[1]}"
    elif [[ "$line" =~ ^//\ TEST:\ stdout-file\ (.+) ]]; then
      local stdout_file
      stdout_file="$(dirname "$file")/${BASH_REMATCH[1]}"
      TEST_STDOUT="$(cat "$stdout_file")"
    elif [[ "$line" =~ ^//\ TEST:\ stderr\ \"(.*)\" ]]; then
      TEST_STDERR="${BASH_REMATCH[1]}"
    elif [[ "$line" =~ ^//\ TEST:\ status\ (.+) ]]; then
      TEST_STATUS="${BASH_REMATCH[1]}"
    elif [[ "$line" =~ ^//\ TEST:\ counterexample-fn\ \"(.+)\" ]]; then
      TEST_CX_FN="${BASH_REMATCH[1]}"
    elif [[ "$line" =~ ^//\ TEST:\ counterexample-blame\ (.+) ]]; then
      TEST_CX_BLAME="${BASH_REMATCH[1]}"
    elif [[ "$line" =~ ^//\ TEST:\ error-code\ (.+) ]]; then
      TEST_ERROR_CODE="${BASH_REMATCH[1]}"
    elif [[ "$line" =~ ^//\ TEST:\ stdin\ \"(.*)\" ]]; then
      TEST_STDIN="${BASH_REMATCH[1]}"
    elif [[ "$line" =~ ^//\ TEST:\ stdin-file\ (.+) ]]; then
      TEST_STDIN_FILE="$(dirname "$file")/${BASH_REMATCH[1]}"
    elif [[ "$line" =~ ^//\ TEST:\ skip\ \"(.+)\" ]]; then
      TEST_SKIP="${BASH_REMATCH[1]}"
    elif [[ "$line" =~ ^//\ TEST:\ build-json\ (.+) ]]; then
      # Only enforced by Phase 5 (error/) — annotation is parsed elsewhere but not checked.
      TEST_BUILD_JSON="${BASH_REMATCH[1]}"
    elif [[ ! "$line" =~ ^// ]]; then
      break
    fi
  done < "$file"
}

matches_filter() {
  local name="$1"
  if [[ -z "$FILTER" ]]; then
    return 0
  fi
  [[ "$name" == *"$FILTER"* ]]
}

section_header() {
  echo -e "\n${BOLD}${CYAN}=== $1 ===${RESET}"
}

# --- Phase 0: Bootstrap gate ---

if [[ "$NO_BOOTSTRAP" == false ]]; then
  section_header "Phase 0: Bootstrap gate"

  if [[ ! -x "$VOWC" ]]; then
    echo -e "${RED}FATAL: build/vowc not found${RESET}"
    exit 1
  fi

  CONCAT="$ROOT_DIR/scripts/concat_vow.sh"
  COMPILER_VOW="$TMPDIR/compiler_all.vow"
  STAGE2="$TMPDIR/stage2"
  STAGE3="$TMPDIR/stage3"

  echo "  Concatenating compiler sources..."
  "$CONCAT" clif > "$COMPILER_VOW"

  echo "  Stage 1: build/vowc → stage2..."
  run_vowc build --no-verify "$COMPILER_VOW" -o "$STAGE2" > /dev/null 2>&1

  echo "  Stage 2: stage2 → stage3..."
  run_bin "$STAGE2" build --no-verify "$COMPILER_VOW" -o "$STAGE3" > /dev/null 2>&1

  HASH2="$(sha256sum "$STAGE2" | cut -d' ' -f1)"
  HASH3="$(sha256sum "$STAGE3" | cut -d' ' -f1)"

  if [[ "$HASH2" == "$HASH3" ]]; then
    echo -e "  ${GREEN}PASS${RESET} Bootstrap fixed point (SHA-256 match)"
  else
    echo -e "  ${RED}FATAL: Bootstrap fixed point FAILED${RESET}"
    echo "  stage2: $HASH2"
    echo "  stage3: $HASH3"
    exit 1
  fi
else
  echo -e "\n${YELLOW}Skipping bootstrap gate (--no-bootstrap)${RESET}"
fi

# --- Phase 1: run/ tests ---

section_header "Phase 1: run/ (build + execute)"

for f in "$SCRIPT_DIR"/run/*.vow; do
  [[ -f "$f" ]] || continue
  name="$(basename "$f" .vow)"
  matches_filter "$name" || continue

  parse_annotations "$f"
  if [[ -n "$TEST_SKIP" ]]; then
    skip "$name" "$TEST_SKIP"
    continue
  fi

  # Defaults for run/
  local_exit="${TEST_EXIT:-0}"

  # Build
  out="$TMPDIR/$name"
  build_json="$(run_vowc build --no-verify "$f" -o "$out" 2>/dev/null)" || true
  build_status="$(json_field "$build_json" "status")"

  if [[ "$build_status" != "Unverified" ]]; then
    fail "$name" "build failed: status=$build_status"
    continue
  fi

  # Run (pipe stdin if TEST_STDIN is set)
  # Disable pipefail in the stdin subshell so we capture the binary's exit
  # code, not printf's SIGPIPE (141) when the binary exits early.
  set +e
  if [[ -n "$TEST_STDIN_FILE" ]]; then
    actual_stdout="$(run_bin "$out" < "$TEST_STDIN_FILE" 2>/dev/null)"
  elif [[ -n "$TEST_STDIN" ]]; then
    actual_stdout="$(set +o pipefail; printf '%b' "$TEST_STDIN" | run_bin "$out" 2>/dev/null)"
  else
    actual_stdout="$(run_bin "$out" 2>/dev/null)"
  fi
  actual_exit=$?
  set -e

  # Check exit code
  if [[ "$actual_exit" -ne "$local_exit" ]]; then
    fail "$name" "exit $actual_exit (expected $local_exit)"
    continue
  fi

  # Check stdout (handle \n escapes in annotation)
  if [[ -n "$TEST_STDOUT" ]]; then
    expected_stdout="$(echo -e "$TEST_STDOUT")"
    if [[ "$actual_stdout" != "$expected_stdout" ]]; then
      fail "$name" "stdout mismatch"
      echo "    expected: $(echo "$expected_stdout" | head -3)"
      echo "    actual:   $(echo "$actual_stdout" | head -3)"
      continue
    fi
  fi

  pass "$name"
done

# --- Phase 2: verify/ tests ---

if [[ "$NO_VERIFY" == true ]]; then
  echo -e "\n${YELLOW}Skipping verify/ tests (--no-verify)${RESET}"
else
  section_header "Phase 2: verify/ (contracts verified)"

  for f in "$SCRIPT_DIR"/verify/*.vow; do
    [[ -f "$f" ]] || continue
    name="$(basename "$f" .vow)"
    matches_filter "$name" || continue

    parse_annotations "$f"
    if [[ -n "$TEST_SKIP" ]]; then
      skip "$name" "$TEST_SKIP"
      continue
    fi

    expected_status="${TEST_STATUS:-Verified}"

    set +e
    verify_json="$(run_vowc verify "$f" 2>/dev/null)"
    verify_exit=$?
    set -e

    actual_status="$(json_field "$verify_json" "status")"
    if [[ "$actual_status" != "$expected_status" ]]; then
      fail "$name" "status=$actual_status (expected $expected_status)"
      continue
    fi

    pass "$name"
  done

  name="verify_tmp_cleanup"
  if matches_filter "$name"; then
    scratch_tmp="$TMPDIR/${name}"
    mkdir -p "$scratch_tmp"
    before_count="$(count_verify_scratch_dirs "$scratch_tmp")"
    set +e
    verify_json="$(run_vowc_with_tmpdir "$scratch_tmp" verify --verify-jobs 2 "$SCRIPT_DIR/verify/verify_jobs_pool.vow" 2>/dev/null)"
    verify_exit=$?
    set -e
    after_count="$(count_verify_scratch_dirs "$scratch_tmp")"
    actual_status="$(json_field "$verify_json" "status")"
    if [[ "$verify_exit" -ne 0 ]]; then
      fail "$name" "exit $verify_exit"
    elif [[ "$actual_status" != "Verified" ]]; then
      fail "$name" "status=$actual_status (expected Verified)"
    elif [[ "$after_count" != "$before_count" ]]; then
      fail "$name" "scratch dirs grew from $before_count to $after_count"
    else
      pass "$name"
    fi
  fi

  name="contracts_tmp_cleanup"
  if matches_filter "$name"; then
    scratch_tmp="$TMPDIR/${name}"
    mkdir -p "$scratch_tmp"
    before_count="$(count_verify_scratch_dirs "$scratch_tmp")"
    set +e
    contracts_json="$(run_vowc_with_tmpdir "$scratch_tmp" contracts --verify "$SCRIPT_DIR/verify/verify_jobs_pool.vow" 2>/dev/null)"
    contracts_exit=$?
    set -e
    after_count="$(count_verify_scratch_dirs "$scratch_tmp")"
    contracts_proven="$(python3 -c "
import json, sys
try:
    d = json.loads(sys.stdin.read())
except Exception:
    print('parse_error'); sys.exit(0)
ss = [e.get('status', '') for e in d.get('contracts', [])]
print('ok' if any(s in ('proven', 'proven-ir') for s in ss) else 'no_proven')
" <<< "$contracts_json")"
    if [[ "$contracts_exit" -ne 0 ]]; then
      fail "$name" "exit $contracts_exit"
    elif [[ "$contracts_proven" != "ok" ]]; then
      fail "$name" "no proven contracts (got $contracts_proven)"
    elif [[ "$after_count" != "$before_count" ]]; then
      fail "$name" "scratch dirs grew from $before_count to $after_count"
    else
      pass "$name"
    fi
  fi

  name="contracts_per_clause_precise"
  if matches_filter "$name"; then
    # PR-A (#81): ESBMC --multi-property gives each ensures clause an individual
    # verdict, so a passing clause stays `proven` even when a sibling clause
    # fails. Before the fix the function-level FAILED collapsed the passing
    # clause to `unknown`. (The requires is modeled as an assumption, not an
    # asserted property, so it has no individual verdict and is not checked here
    # — same as the Rust contracts_verify_per_clause_precise_status test.)
    src="$TMPDIR/${name}.vow"
    cat > "$src" <<'VOWEOF'
module M
fn f(x: i64) -> i64 vow {
  requires: x >= 0,
  ensures: result == x,
  ensures: result == x + 1
} { x }
fn main() -> i32 [io] { 0 }
VOWEOF
    set +e
    cpc_json="$(run_vowc contracts --verify "$src" 2>/dev/null)"
    cpc_exit=$?
    set -e
    cpc_check="$(python3 -c "
import json, sys
try:
    d = json.loads(sys.stdin.read())
except Exception:
    print('parse_error'); sys.exit(0)
st = {}
for e in d.get('contracts', []):
    st[e.get('description', '')] = e.get('status', '')
passing = st.get('ensures result == x', '?')
failing = st.get('ensures result == x + 1', '?')
if passing == 'proven' and failing == 'failed':
    print('ok')
else:
    print('pass=%s fail=%s' % (passing, failing))
" <<< "$cpc_json")"
    if [[ "$cpc_exit" -ne 1 ]]; then
      fail "$name" "exit $cpc_exit (expected 1 — fail-closed on the failing clause)"
    elif [[ "$cpc_check" != "ok" ]]; then
      fail "$name" "per-clause status mismatch ($cpc_check)"
    else
      pass "$name"
    fi
  fi

  name="contracts_vacuity_detected"
  if matches_filter "$name"; then
    # PR-B (#81): a function whose `requires` are contradictory makes every
    # `ensures` pass vacuously. The `--error-label vow_reach` probe finds the
    # post-requires point unreachable and marks the whole contract `vacuous`
    # (fail-closed, exit 1). Mirrors the Rust contracts_verify_detects_vacuous.
    src="$TMPDIR/${name}.vow"
    cat > "$src" <<'VOWEOF'
module M
fn f(x: i64) -> i64 vow {
  requires: x > 10,
  requires: x < 5,
  ensures: result == x
} { x }
fn main() -> i32 [io] { 0 }
VOWEOF
    set +e
    vac_json="$(run_vowc contracts --verify "$src" 2>/dev/null)"
    vac_exit=$?
    set -e
    vac_check="$(python3 -c "
import json, sys
try:
    d = json.loads(sys.stdin.read())
except Exception:
    print('parse_error'); sys.exit(0)
ss = [e.get('status', '') for e in d.get('contracts', [])]
if ss and all(s == 'vacuous' for s in ss) and d.get('summary', {}).get('vacuous') == 3:
    print('ok')
else:
    print('statuses=%s vacuous=%s' % (ss, d.get('summary', {}).get('vacuous')))
" <<< "$vac_json")"
    if [[ "$vac_exit" -ne 1 ]]; then
      fail "$name" "exit $vac_exit (expected 1 — fail-closed on vacuous)"
    elif [[ "$vac_check" != "ok" ]]; then
      fail "$name" "vacuity not detected ($vac_check)"
    else
      pass "$name"
    fi
  fi

  name="contracts_weakness_trivially_satisfiable"
  if matches_filter "$name"; then
    # PR-C (#81): a weak postcondition (`result >= 0`) is satisfied by a trivial
    # `return 0` body, so the body-replace probe flags it trivially_satisfiable;
    # a tight one (`result == x + 1`) is not. Informational (no exit-code change).
    # Mirrors the Rust contracts_verify_flags_trivially_satisfiable_ensures test.
    src="$TMPDIR/${name}.vow"
    cat > "$src" <<'VOWEOF'
module M
fn weak(x: i64) -> i64 vow {
  requires: x >= 0,
  ensures: result >= 0
} { x + 1 }
fn tight(x: i64) -> i64 vow {
  ensures: result == x + 1
} { x + 1 }
fn main() -> i32 [io] { 0 }
VOWEOF
    set +e
    wk_json="$(run_vowc contracts --verify "$src" 2>/dev/null)"
    set -e
    wk_check="$(python3 -c "
import json, sys
try:
    d = json.loads(sys.stdin.read())
except Exception:
    print('parse_error'); sys.exit(0)
t = {}
for e in d.get('contracts', []):
    if e.get('kind') == 'ensures':
        t[e.get('function', '')] = e.get('trivially_satisfiable')
if t.get('weak') is True and t.get('tight') is False and d.get('summary', {}).get('trivially_satisfiable') == 1:
    print('ok')
else:
    print('weak=%s tight=%s n=%s' % (t.get('weak'), t.get('tight'), d.get('summary', {}).get('trivially_satisfiable')))
" <<< "$wk_json")"
    if [[ "$wk_check" != "ok" ]]; then
      fail "$name" "weakness probe mismatch ($wk_check)"
    else
      pass "$name"
    fi
  fi

  name="contracts_suffix_len_unknown_clause_set_membership"
  if matches_filter "$name"; then
    set +e
    suffix_contracts_json="$(run_vowc contracts "$ROOT_DIR/compiler/main.vow" 2>/dev/null)"
    suffix_contracts_exit=$?
    set -e
    suffix_check="$(python3 -c "
import json, sys

suffixes = [
    'i8', 'i16', 'i32', 'i64', 'i128',
    'u8', 'u16', 'u32', 'u64', 'u128',
    'usize', 'isize',
]

try:
    d = json.loads(sys.stdin.read())
except Exception:
    print('parse_error'); sys.exit(0)

descriptions = [
    e.get('description', '')
    for e in d.get('contracts', [])
    if e.get('function', '') == 'suffix_len' and e.get('kind', '') == 'ensures'
]
text = '\n'.join(descriptions)
missing = [s for s in suffixes if ('suffix == tok_suffix_%s()' % s) not in text]
has_range = (
    'suffix >= tok_suffix_i8()' in text
    or 'suffix <= tok_suffix_isize()' in text
)

if not descriptions:
    print('missing_suffix_len')
elif has_range:
    print('range_guard_present')
elif missing:
    print('missing_membership=' + ','.join(missing))
else:
    print('ok')
" <<< "$suffix_contracts_json")"
    if [[ "$suffix_contracts_exit" -ne 0 ]]; then
      fail "$name" "exit $suffix_contracts_exit"
    elif [[ "$suffix_check" != "ok" ]]; then
      fail "$name" "suffix_len unknown-suffix clause mismatch ($suffix_check)"
    else
      pass "$name"
    fi
  fi
fi

# --- Phase 3: verify-fail/ tests ---

if [[ "$NO_VERIFY" == true ]]; then
  echo -e "\n${YELLOW}Skipping verify-fail/ tests (--no-verify)${RESET}"
else
  section_header "Phase 3: verify-fail/ (expected verification failures)"

  for f in "$SCRIPT_DIR"/verify-fail/*.vow; do
    [[ -f "$f" ]] || continue
    name="$(basename "$f" .vow)"
    matches_filter "$name" || continue

    parse_annotations "$f"
    if [[ -n "$TEST_SKIP" ]]; then
      skip "$name" "$TEST_SKIP"
      continue
    fi

    expected_status="${TEST_STATUS:-VerifyFailed}"

    set +e
    verify_json="$(run_vowc verify "$f" 2>/dev/null)"
    verify_exit=$?
    set -e

    actual_status="$(json_field "$verify_json" "status")"
    if [[ "$actual_status" != "$expected_status" ]]; then
      fail "$name" "status=$actual_status (expected $expected_status)"
      continue
    fi

    # Check counterexample fields if annotated
    if [[ -n "$TEST_CX_FN" ]]; then
      actual_fn="$(json_cx_field "$verify_json" "function")"
      if [[ "$actual_fn" != "$TEST_CX_FN" ]]; then
        fail "$name" "counterexample fn=$actual_fn (expected $TEST_CX_FN)"
        continue
      fi
    fi
    if [[ -n "$TEST_CX_BLAME" ]]; then
      actual_blame="$(json_cx_field "$verify_json" "blame")"
      if [[ "$actual_blame" != "$TEST_CX_BLAME" ]]; then
        fail "$name" "counterexample blame=$actual_blame (expected $TEST_CX_BLAME)"
        continue
      fi
    fi

    pass "$name"
  done
fi

# --- Phase 4: debug/ tests ---

section_header "Phase 4: debug/ (runtime vow checks)"

for f in "$SCRIPT_DIR"/debug/*.vow; do
  [[ -f "$f" ]] || continue
  name="$(basename "$f" .vow)"
  matches_filter "$name" || continue

  parse_annotations "$f"
  if [[ -n "$TEST_SKIP" ]]; then
    skip "$name" "$TEST_SKIP"
    continue
  fi

  local_exit="${TEST_EXIT:-1}"

  # Build in debug mode
  out="$TMPDIR/debug_$name"
  build_json="$(run_vowc build --mode debug --no-verify "$f" -o "$out" 2>/dev/null)" || true
  build_status="$(json_field "$build_json" "status")"

  if [[ "$build_status" != "Unverified" ]]; then
    fail "$name" "debug build failed: status=$build_status"
    continue
  fi

  # Run and capture stderr
  set +e
  actual_stdout="$(run_bin "$out" 2>"$TMPDIR/debug_${name}_stderr")"
  actual_exit=$?
  set -e
  actual_stderr="$(cat "$TMPDIR/debug_${name}_stderr")"

  # Check exit code
  if [[ "$actual_exit" -ne "$local_exit" ]]; then
    fail "$name" "exit $actual_exit (expected $local_exit)"
    continue
  fi

  # Check stderr substring
  if [[ -n "$TEST_STDERR" ]]; then
    expected_stderr="$(echo -e "$TEST_STDERR")"
    if [[ "$actual_stderr" != *"$expected_stderr"* ]]; then
      fail "$name" "stderr missing: $expected_stderr"
      echo "    actual stderr: $(echo "$actual_stderr" | head -3)"
      continue
    fi
  fi

  pass "$name"
done

# --- Phase 5: error/ tests ---

section_header "Phase 5: error/ (compilation failures)"

for f in "$SCRIPT_DIR"/error/*.vow; do
  [[ -f "$f" ]] || continue
  name="$(basename "$f" .vow)"
  matches_filter "$name" || continue

  parse_annotations "$f"
  if [[ -n "$TEST_SKIP" ]]; then
    skip "$name" "$TEST_SKIP"
    continue
  fi

  expected_status="${TEST_STATUS:-CompileFailed}"
  local_exit="${TEST_EXIT:-1}"

  set +e
  build_json="$(run_vowc build --no-verify "$f" 2>/dev/null)"
  build_exit=$?
  set -e

  # Check exit code
  if [[ "$build_exit" -ne "$local_exit" ]]; then
    fail "$name" "exit $build_exit (expected $local_exit)"
    continue
  fi

  # Check status
  actual_status="$(json_field "$build_json" "status")"
  if [[ "$actual_status" != "$expected_status" ]]; then
    fail "$name" "status=$actual_status (expected $expected_status)"
    continue
  fi

  if [[ -n "$TEST_ERROR_CODE" ]]; then
    # Scan all error-severity diagnostics, not just the first — a compiler
    # may legitimately emit several errors for a single bug (e.g. AST
    # backstop + region pass), and the harness should treat the test as
    # passing if the expected code is anywhere in the output.
    has_expected="$(EXPECTED="$TEST_ERROR_CODE" python3 -c "
import json, sys, os
expected = os.environ['EXPECTED']
try:
    d = json.loads(sys.stdin.read())
except Exception:
    print('false'); sys.exit(0)
xs = d.get('diagnostics', []) or []
codes = [x.get('error_code', '') for x in xs if x.get('severity', 'error') == 'error']
print('true' if expected in codes else 'false')
print('|'.join(codes))
" <<< "$build_json")"
    found="$(echo "$has_expected" | head -1)"
    actual_codes="$(echo "$has_expected" | tail -1)"
    if [[ "$found" != "true" ]]; then
      fail "$name" "error_code=$actual_codes (expected $TEST_ERROR_CODE)"
      continue
    fi
  fi

  # Check stderr substring if annotated
  if [[ -n "$TEST_STDERR" ]]; then
    # Re-run to capture stderr
    set +e
    run_vowc build --no-verify "$f" 2>"$TMPDIR/error_${name}_stderr" >/dev/null
    set -e
    actual_stderr="$(cat "$TMPDIR/error_${name}_stderr")"
    expected_stderr="$(echo -e "$TEST_STDERR")"
    if [[ "$actual_stderr" != *"$expected_stderr"* ]]; then
      fail "$name" "stderr missing: $expected_stderr"
      continue
    fi
  fi

  # build-json: Python expr; d=full JSON, xs=diagnostics array, x=xs[0] or {}.
  if [[ -n "$TEST_BUILD_JSON" ]]; then
    json_check="$(EXPR="$TEST_BUILD_JSON" python3 -c "
import json, sys, os
expr = os.environ['EXPR']
try:
    d = json.loads(sys.stdin.read())
except Exception as e:
    print('false'); print(f'json parse: {e}'); sys.exit(0)
xs = d.get('diagnostics', []) or []
x = xs[0] if xs else {}
try:
    ok = bool(eval(expr))
except Exception as e:
    print('false'); print(f'expr error: {e}'); sys.exit(0)
print('true' if ok else 'false')
print(json.dumps(x))
" <<< "$build_json")"
    json_pass="$(echo "$json_check" | head -1)"
    json_detail="$(echo "$json_check" | tail -1)"
    if [[ "$json_pass" != "true" ]]; then
      fail "$name" "build-json failed: $TEST_BUILD_JSON (x=$json_detail)"
      continue
    fi
  fi

  pass "$name"
done

# --- Phase 6: multi/ tests ---

section_header "Phase 6: multi/ (multi-module)"

for d in "$SCRIPT_DIR"/multi/*/; do
  [[ -d "$d" ]] || continue
  main_file="$d/main.vow"
  [[ -f "$main_file" ]] || continue
  name="$(basename "$d")"
  matches_filter "$name" || continue

  parse_annotations "$main_file"
  if [[ -n "$TEST_SKIP" ]]; then
    skip "$name" "$TEST_SKIP"
    continue
  fi

  local_exit="${TEST_EXIT:-0}"

  # Build
  out="$TMPDIR/multi_$name"
  build_json="$(run_vowc build --no-verify "$main_file" -o "$out" 2>/dev/null)" || true
  build_status="$(json_field "$build_json" "status")"

  if [[ "$build_status" != "Unverified" ]]; then
    fail "$name" "build failed: status=$build_status"
    continue
  fi

  # Run
  set +e
  actual_stdout="$(run_bin "$out" 2>/dev/null)"
  actual_exit=$?
  set -e

  # Check exit code
  if [[ "$actual_exit" -ne "$local_exit" ]]; then
    fail "$name" "exit $actual_exit (expected $local_exit)"
    continue
  fi

  # Check stdout
  if [[ -n "$TEST_STDOUT" ]]; then
    expected_stdout="$(echo -e "$TEST_STDOUT")"
    if [[ "$actual_stdout" != "$expected_stdout" ]]; then
      fail "$name" "stdout mismatch"
      echo "    expected: $(echo "$expected_stdout" | head -3)"
      echo "    actual:   $(echo "$actual_stdout" | head -3)"
      continue
    fi
  fi

  pass "$name"
done

# --- Summary ---

section_header "Phase 7: frontend/ (staged boundary regressions)"

stack_main="$SCRIPT_DIR/multi/stack/main.vow"
geometry_main="$SCRIPT_DIR/multi/geometry/main.vow"

name="frontend_stack_contracts"
if matches_filter "$name"; then
  set +e
  contracts_json="$(run_vowc contracts "$stack_main" 2>/dev/null)"
  contracts_exit=$?
  set -e
  if [[ "$contracts_exit" -ne 0 ]]; then
    fail "$name" "exit $contracts_exit"
  else
    contracts_total="$(json_path_field "$contracts_json" "summary.total")"
    if [[ "$contracts_total" != "2" ]]; then
      fail "$name" "summary.total=$contracts_total (expected 2)"
    else
      pass "$name"
    fi
  fi
fi

name="frontend_stack_test"
if matches_filter "$name"; then
  set +e
  test_json="$(run_vowc test "$stack_main" 2>/dev/null)"
  test_exit=$?
  set -e
  if [[ "$test_exit" -ne 0 ]]; then
    fail "$name" "exit $test_exit"
  else
    test_status="$(json_field "$test_json" "status")"
    test_passed="$(json_field "$test_json" "passed")"
    if [[ "$test_status" != "TestsPassed" ]]; then
      fail "$name" "status=$test_status (expected TestsPassed)"
    elif [[ "$test_passed" != "1" ]]; then
      fail "$name" "passed=$test_passed (expected 1)"
    else
      pass "$name"
    fi
  fi
fi

name="frontend_geometry_verify"
if [[ "$NO_VERIFY" == true ]]; then
  if matches_filter "$name"; then
    echo -e "  ${YELLOW}SKIP${RESET} frontend_geometry_verify — --no-verify"
    SKIP=$((SKIP + 1))
  fi
else
  if matches_filter "$name"; then
    set +e
    verify_json="$(run_vowc verify "$geometry_main" 2>/dev/null)"
    verify_exit=$?
    set -e
    if [[ "$verify_exit" -ne 0 ]]; then
      fail "$name" "exit $verify_exit"
    else
      verify_status="$(json_field "$verify_json" "status")"
      if [[ "$verify_status" != "Verified" ]]; then
        fail "$name" "status=$verify_status (expected Verified)"
      else
        pass "$name"
      fi
    fi
  fi
fi

echo ""
echo -e "${BOLD}=== Summary ===${RESET}"
TOTAL=$((PASS + FAIL + SKIP))
echo -e "  ${GREEN}$PASS passed${RESET}, ${RED}$FAIL failed${RESET}, ${YELLOW}$SKIP skipped${RESET} ($TOTAL total)"

if [[ ${#FAILURES[@]} -gt 0 ]]; then
  echo ""
  echo -e "${RED}Failures:${RESET}"
  for f in "${FAILURES[@]}"; do
    echo "  - $f"
  done
  exit 1
fi

exit 0
