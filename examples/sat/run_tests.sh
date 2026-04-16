#!/usr/bin/env bash
set -euo pipefail

SAT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SAT_DIR/../.." && pwd)"
LOCAL_DIR="$SAT_DIR/.local"
BIN="$LOCAL_DIR/sat-test"
TMP_ROOT="${TMPDIR:-/dev/shm}"

if [[ ! -d "$TMP_ROOT" ]]; then
  TMP_ROOT="/tmp"
fi

mkdir -p "$LOCAL_DIR"

zsh -lc "ulimit -v 2000000; TMPDIR='$TMP_ROOT' '$REPO_ROOT/build/vowc' build --no-verify '$SAT_DIR/main.vow' -o '$BIN'" >/dev/null

failures=0

expect_eq() {
  local actual="$1"
  local expected="$2"
  local label="$3"
  if [[ "$actual" != "$expected" ]]; then
    printf 'FAIL %s\nexpected:\n%s\nactual:\n%s\n' "$label" "$expected" "$actual" >&2
    failures=$((failures + 1))
  fi
}

expect_contains() {
  local haystack="$1"
  local needle="$2"
  local label="$3"
  if [[ "$haystack" != *"$needle"* ]]; then
    printf 'FAIL %s\nmissing substring: %s\nactual:\n%s\n' "$label" "$needle" "$haystack" >&2
    failures=$((failures + 1))
  fi
}

run_file_case() {
  local name="$1"
  local file="$2"
  local expected_code="$3"
  local expected_stdout="$4"
  local expected_stderr_substr="$5"
  local stdout_file stderr_file code stdout_text stderr_text
  stdout_file="$(mktemp "$TMP_ROOT/sat-test-${name}.stdout.XXXXXX")"
  stderr_file="$(mktemp "$TMP_ROOT/sat-test-${name}.stderr.XXXXXX")"
  set +e
  "$BIN" "$file" >"$stdout_file" 2>"$stderr_file"
  code=$?
  set -e
  stdout_text="$(cat "$stdout_file")"
  stderr_text="$(cat "$stderr_file")"
  rm -f "$stdout_file" "$stderr_file"
  expect_eq "$code" "$expected_code" "$name exit"
  expect_eq "$stdout_text" "$expected_stdout" "$name stdout"
  if [[ -n "$expected_stderr_substr" ]]; then
    expect_contains "$stderr_text" "$expected_stderr_substr" "$name stderr"
  else
    expect_eq "$stderr_text" "" "$name stderr"
  fi
}

run_stdin_case() {
  local name="$1"
  local file="$2"
  local expected_code="$3"
  local expected_stdout="$4"
  local expected_stderr_substr="$5"
  local stdout_file stderr_file code stdout_text stderr_text
  stdout_file="$(mktemp "$TMP_ROOT/sat-test-${name}.stdout.XXXXXX")"
  stderr_file="$(mktemp "$TMP_ROOT/sat-test-${name}.stderr.XXXXXX")"
  set +e
  cat "$file" | "$BIN" >"$stdout_file" 2>"$stderr_file"
  code=$?
  set -e
  stdout_text="$(cat "$stdout_file")"
  stderr_text="$(cat "$stderr_file")"
  rm -f "$stdout_file" "$stderr_file"
  expect_eq "$code" "$expected_code" "$name exit"
  expect_eq "$stdout_text" "$expected_stdout" "$name stdout"
  if [[ -n "$expected_stderr_substr" ]]; then
    expect_contains "$stderr_text" "$expected_stderr_substr" "$name stderr"
  else
    expect_eq "$stderr_text" "" "$name stderr"
  fi
}

run_stats_case() {
  local file="$1"
  local stdout_file stderr_file code stdout_text stderr_text
  stdout_file="$(mktemp "$TMP_ROOT/sat-test-stats.stdout.XXXXXX")"
  stderr_file="$(mktemp "$TMP_ROOT/sat-test-stats.stderr.XXXXXX")"
  set +e
  "$BIN" --stats "$file" >"$stdout_file" 2>"$stderr_file"
  code=$?
  set -e
  stdout_text="$(cat "$stdout_file")"
  stderr_text="$(cat "$stderr_file")"
  rm -f "$stdout_file" "$stderr_file"
  expect_eq "$code" "10" "stats exit"
  expect_eq "$stdout_text" $'SAT\nv 1 0' "stats stdout"
  expect_contains "$stderr_text" "vars=1" "stats vars"
  expect_contains "$stderr_text" "clauses=1" "stats clauses"
  expect_contains "$stderr_text" "decisions=0" "stats decisions"
}

run_pure_stats_case() {
  local file="$1"
  local stdout_file stderr_file code stdout_text stderr_text
  stdout_file="$(mktemp "$TMP_ROOT/sat-test-pure.stdout.XXXXXX")"
  stderr_file="$(mktemp "$TMP_ROOT/sat-test-pure.stderr.XXXXXX")"
  set +e
  "$BIN" --stats "$file" >"$stdout_file" 2>"$stderr_file"
  code=$?
  set -e
  stdout_text="$(cat "$stdout_file")"
  stderr_text="$(cat "$stderr_file")"
  rm -f "$stdout_file" "$stderr_file"
  expect_eq "$code" "10" "pure exit"
  expect_eq "$stdout_text" $'SAT\nv 1 2 3 0' "pure stdout"
  expect_contains "$stderr_text" "decisions=0" "pure decisions"
}

run_sat_validate_case() {
  local name="$1"
  local file="$2"
  local stdout_file stderr_file code
  stdout_file="$(mktemp "$TMP_ROOT/sat-test-${name}.stdout.XXXXXX")"
  stderr_file="$(mktemp "$TMP_ROOT/sat-test-${name}.stderr.XXXXXX")"
  set +e
  "$BIN" "$file" >"$stdout_file" 2>"$stderr_file"
  code=$?
  set -e
  expect_eq "$code" "10" "$name exit"
  expect_eq "$(head -n 1 "$stdout_file")" "SAT" "$name status"
  python - "$file" "$stdout_file" <<'PY'
import sys
from pathlib import Path

cnf_path = Path(sys.argv[1])
out_path = Path(sys.argv[2])

assignment = {}
with out_path.open() as handle:
    for raw in handle:
        line = raw.strip()
        if not line.startswith("v "):
            continue
        for tok in line.split()[1:]:
            lit = int(tok)
            if lit == 0:
                break
            assignment[abs(lit)] = lit > 0

num_vars = 0
with cnf_path.open() as handle:
    for raw in handle:
        line = raw.strip()
        if not line or line.startswith("c"):
            continue
        if line.startswith("p "):
            num_vars = int(line.split()[2])
            continue
        clause = []
        for tok in line.split():
            lit = int(tok)
            if lit == 0:
                if not any((assignment.get(abs(l), True) if l > 0 else not assignment.get(abs(l), True)) for l in clause):
                    raise SystemExit(1)
                clause = []
            else:
                clause.append(lit)

for var in range(1, num_vars + 1):
    assignment.setdefault(var, True)
PY
  local py_code=$?
  rm -f "$stdout_file" "$stderr_file"
  expect_eq "$py_code" "0" "$name assignment"
}

run_help_case() {
  local stdout_file stderr_file code stdout_text stderr_text
  stdout_file="$(mktemp "$TMP_ROOT/sat-test-help.stdout.XXXXXX")"
  stderr_file="$(mktemp "$TMP_ROOT/sat-test-help.stderr.XXXXXX")"
  set +e
  "$BIN" --help >"$stdout_file" 2>"$stderr_file"
  code=$?
  set -e
  stdout_text="$(cat "$stdout_file")"
  stderr_text="$(cat "$stderr_file")"
  rm -f "$stdout_file" "$stderr_file"
  expect_eq "$code" "0" "help exit"
  expect_contains "$stdout_text" "usage: sat" "help stdout"
  expect_eq "$stderr_text" "" "help stderr"
}

run_stdin_case "sat-simple-stdin" \
  "$SAT_DIR/tests/sat_simple.cnf" \
  "10" \
  $'SAT\nv 1 0' \
  ""

run_file_case "sat-simple-file" \
  "$SAT_DIR/tests/sat_simple.cnf" \
  "10" \
  $'SAT\nv 1 0' \
  ""

run_file_case "sat-many-vars" \
  "$SAT_DIR/tests/sat_many_vars.cnf" \
  "10" \
  $'SAT\nv 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 \nv 21 0' \
  ""

run_file_case "unsat-unit" \
  "$SAT_DIR/tests/unsat_unit.cnf" \
  "20" \
  "UNSAT" \
  ""

run_file_case "unsat-learned" \
  "$SAT_DIR/tests/unsat_learned.cnf" \
  "20" \
  "UNSAT" \
  ""

run_sat_validate_case "sat-conflict-regression" \
  "$SAT_DIR/tests/sat_conflict_regression.cnf"

run_sat_validate_case "sat-watch-bucket-regression" \
  "$SAT_DIR/tests/sat_watch_bucket_regression.cnf"

run_file_case "missing-header" \
  "$SAT_DIR/tests/malformed_missing_header.cnf" \
  "1" \
  "" \
  "encountered clause tokens before header"

run_file_case "out-of-range" \
  "$SAT_DIR/tests/malformed_out_of_range.cnf" \
  "1" \
  "" \
  "literal out of declared variable range"

run_file_case "missing-zero" \
  "$SAT_DIR/tests/malformed_missing_zero.cnf" \
  "1" \
  "" \
  "unterminated final clause"

run_file_case "clause-count" \
  "$SAT_DIR/tests/malformed_clause_count.cnf" \
  "1" \
  "" \
  "clause count mismatch"

run_stats_case "$SAT_DIR/tests/sat_simple.cnf"
run_pure_stats_case "$SAT_DIR/tests/sat_pure_root.cnf"
run_help_case

if [[ "$failures" -ne 0 ]]; then
  printf 'sat demo tests failed: %s\n' "$failures" >&2
  exit 1
fi

printf 'sat demo tests passed\n'
