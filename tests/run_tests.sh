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
  python3 -c "import json,sys; d=json.loads(sys.stdin.read()); print(d.get('$field',''))" <<< "$json"
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

parse_annotations() {
  local file="$1"
  # Reset annotation vars
  TEST_EXIT=""
  TEST_STDOUT=""
  TEST_STDERR=""
  TEST_STATUS=""
  TEST_CX_FN=""
  TEST_CX_BLAME=""
  TEST_SKIP=""
  TEST_STDIN=""
  TEST_STDIN_FILE=""

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
    elif [[ "$line" =~ ^//\ TEST:\ stdin\ \"(.*)\" ]]; then
      TEST_STDIN="${BASH_REMATCH[1]}"
    elif [[ "$line" =~ ^//\ TEST:\ stdin-file\ (.+) ]]; then
      TEST_STDIN_FILE="$(dirname "$file")/${BASH_REMATCH[1]}"
    elif [[ "$line" =~ ^//\ TEST:\ skip\ \"(.+)\" ]]; then
      TEST_SKIP="${BASH_REMATCH[1]}"
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
