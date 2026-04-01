#!/usr/bin/env bash
set -euo pipefail

BOLD="\033[1m"
GREEN="\033[32m"
RED="\033[31m"
YELLOW="\033[33m"
RESET="\033[0m"

PASS=0
FAIL=0
SKIP=0
FAILURES=()

RUST="./target/release/vow"
SELF=""
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

# ─── Helpers ────────────────────────────────────────────────────────

run_self() {
    (ulimit -v 2000000; "$SELF" "$@")
}

run_self_bin() {
    local bin="$1"; shift
    (ulimit -v 2000000; "$bin" "$@")
}

pass() {
    printf "  ${GREEN}PASS${RESET} %s\n" "$1"
    PASS=$((PASS + 1))
}

fail() {
    printf "  ${RED}FAIL${RESET} %s — %s\n" "$1" "$2"
    FAIL=$((FAIL + 1))
    FAILURES+=("$1: $2")
}

skip() {
    printf "  ${YELLOW}SKIP${RESET} %s — %s\n" "$1" "$2"
    SKIP=$((SKIP + 1))
}

compare_json() {
    local label="$1" rust_json="$2" self_json="$3" rust_exit="$4" self_exit="$5"

    local result
    if result=$(python3 -c "
import json, sys

rust_exit = int(sys.argv[1])
self_exit = int(sys.argv[2])
try:
    r = json.loads(sys.argv[3])
    s = json.loads(sys.argv[4])
except json.JSONDecodeError as e:
    print(f'FAIL: JSON parse error: {e}')
    sys.exit(1)

errors = []

if rust_exit != self_exit:
    errors.append(f'exit code: {rust_exit} vs {self_exit}')

rs = r.get('status', '')
ss = s.get('status', '')
if rs != ss:
    errors.append(f'status: {rs} vs {ss}')

if rs != 'VerifyFailed':
    rd = len(r.get('diagnostics', []))
    sd = len(s.get('diagnostics', []))
    if rd != sd:
        errors.append(f'diagnostics count: {rd} vs {sd}')

rc = r.get('counterexamples', [])
sc = s.get('counterexamples', [])
if rs == 'VerifyFailed' and ss == 'VerifyFailed':
    if len(rc) == 0:
        errors.append('rust has no counterexamples for VerifyFailed')
    if len(sc) == 0:
        errors.append('self has no counterexamples for VerifyFailed')
    if rc and sc:
        for field in ('function', 'blame'):
            rv = rc[0].get(field)
            sv = sc[0].get(field)
            if rv != sv:
                errors.append(f'counterexample[0].{field}: {rv} vs {sv}')
else:
    if len(rc) != len(sc):
        errors.append(f'counterexamples count: {len(rc)} vs {len(sc)}')
    else:
        for i, (rce, sce) in enumerate(zip(rc, sc)):
            for field in ('function', 'vow_id', 'blame'):
                rv = rce.get(field)
                sv = sce.get(field)
                if field == 'vow_id' and (rv in (0, -1, None) and sv in (0, -1, None)):
                    continue
                if rv != sv:
                    errors.append(f'counterexample[{i}].{field}: {rv} vs {sv}')

if errors:
    print('FAIL: ' + '; '.join(errors))
    sys.exit(1)
else:
    print('OK')
    sys.exit(0)
" "$rust_exit" "$self_exit" "$rust_json" "$self_json" 2>&1); then
        pass "$label"
    else
        fail "$label" "$result"
    fi
}

compare_runtime() {
    local label="$1" rust_bin="$2" self_bin="$3"

    if [ ! -x "$rust_bin" ] || [ ! -x "$self_bin" ]; then
        skip "$label" "binary not found"
        return
    fi

    local rust_out="" self_out="" rust_exit=0 self_exit=0
    rust_out=$("$rust_bin" 2>/dev/null) || rust_exit=$?
    self_out=$(run_self_bin "$self_bin" 2>/dev/null) || self_exit=$?

    local errors=()
    if [ "$rust_exit" != "$self_exit" ]; then
        errors+=("exit: $rust_exit vs $self_exit")
    fi
    if [ "$rust_out" != "$self_out" ]; then
        errors+=("stdout differs")
    fi

    if [ ${#errors[@]} -eq 0 ]; then
        pass "$label"
    else
        fail "$label" "$(IFS='; '; echo "${errors[*]}")"
    fi
}

compare_error() {
    local label="$1" rust_json="$2" self_json="$3" rust_exit="$4" self_exit="$5"

    local result
    if result=$(python3 -c "
import json, sys

rust_exit = int(sys.argv[1])
self_exit = int(sys.argv[2])
try:
    r = json.loads(sys.argv[3])
    s = json.loads(sys.argv[4])
except json.JSONDecodeError as e:
    print(f'FAIL: JSON parse error: {e}')
    sys.exit(1)

errors = []
if rust_exit == 0:
    errors.append('rust exited 0, expected failure')
if self_exit == 0:
    errors.append('self exited 0, expected failure')
for name, j in [('rust', r), ('self', s)]:
    if j.get('status') != 'CompileFailed':
        errors.append(f'{name} status={j.get(\"status\")}, expected CompileFailed')
    if len(j.get('diagnostics', [])) < 1:
        errors.append(f'{name} has no diagnostics')

if errors:
    print('FAIL: ' + '; '.join(errors))
    sys.exit(1)
else:
    print('OK')
    sys.exit(0)
" "$rust_exit" "$self_exit" "$rust_json" "$self_json" 2>&1); then
        pass "$label"
    else
        fail "$label" "$result"
    fi
}

# ─── Section 0: Setup ───────────────────────────────────────────────

echo -e "${BOLD}=== Phase 20.1: Full Test Suite ===${RESET}"
echo ""

echo -e "${BOLD}Building Rust compiler...${RESET}"
cargo build --all --release 2>&1 | tail -1
echo -e "${BOLD}Building self-hosted compiler...${RESET}"
$RUST --no-verify compiler/main.vow -o "$TMPDIR/vowc_self" >/dev/null 2>/dev/null
SELF="$TMPDIR/vowc_self"
echo ""

# ─── Section 1: Build --no-verify ─────────────────────────────────

echo -e "${BOLD}--- Section 1: Build --no-verify ---${RESET}"
for vow_file in examples/*.vow; do
    name=$(basename "$vow_file" .vow)

    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST build --no-verify "$vow_file" -o "$TMPDIR/rust_${name}" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self build --no-verify "$vow_file" -o "$TMPDIR/self_${name}" 2>/dev/null) || self_exit=$?

    if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
        skip "${name}/build-no-verify" "empty output (rust=$rust_exit, self=$self_exit)"
        continue
    fi

    compare_json "${name}/build-no-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit"

    # Save JSON for Section 3 (runtime execution)
    echo "$rust_json" > "$TMPDIR/rust_${name}.json"
    echo "$self_json" > "$TMPDIR/self_${name}.json"
done
echo ""

# ─── Section 2: Verify ─────────────────────────────────────────────

echo -e "${BOLD}--- Section 2: Verify ---${RESET}"
for vow_file in examples/*.vow; do
    name=$(basename "$vow_file" .vow)
    if ! grep -q 'vow {' "$vow_file"; then
        continue
    fi

    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST verify "$vow_file" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self verify "$vow_file" 2>/dev/null) || self_exit=$?

    if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
        skip "${name}/verify" "empty output (rust=$rust_exit, self=$self_exit)"
        continue
    fi

    compare_json "${name}/verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit"
done
echo ""

# ─── Section 3: Runtime Execution ──────────────────────────────────

echo -e "${BOLD}--- Section 3: Runtime Execution ---${RESET}"
for vow_file in examples/*.vow; do
    name=$(basename "$vow_file" .vow)
    if [ "$name" = "divide" ]; then
        skip "${name}/runtime" "division by zero UB in release mode"
        continue
    fi

    # Check if build produced executables (from Section 1 JSON)
    rust_exe=$(python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('executable') or '')" < "$TMPDIR/rust_${name}.json" 2>/dev/null) || rust_exe=""
    self_exe=$(python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('executable') or '')" < "$TMPDIR/self_${name}.json" 2>/dev/null) || self_exe=""

    if [ -z "$rust_exe" ] && [ -z "$self_exe" ]; then
        skip "${name}/runtime" "no executable (library module)"
        continue
    fi
    if [ -z "$rust_exe" ] || [ -z "$self_exe" ]; then
        fail "${name}/runtime" "executable mismatch: rust='${rust_exe:-null}' self='${self_exe:-null}'"
        continue
    fi

    compare_runtime "${name}/runtime" "$TMPDIR/rust_${name}" "$TMPDIR/self_${name}"
done
echo ""

# ─── Section 4: Run Tests (tests/run/) ────────────────────────────

echo -e "${BOLD}--- Section 4: Run Tests ---${RESET}"
for vow_file in tests/run/*.vow; do
    name=$(basename "$vow_file" .vow)

    # Build with both compilers
    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST build --no-verify "$vow_file" -o "$TMPDIR/test_rust_${name}" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self build --no-verify "$vow_file" -o "$TMPDIR/test_self_${name}" 2>/dev/null) || self_exit=$?

    if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
        skip "${name}/test-build" "empty output (rust=$rust_exit, self=$self_exit)"
        continue
    fi

    compare_json "${name}/test-build" "$rust_json" "$self_json" "$rust_exit" "$self_exit"

    # Extract executables
    rust_exe=$(python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('executable') or '')" <<< "$rust_json" 2>/dev/null) || rust_exe=""
    self_exe=$(python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('executable') or '')" <<< "$self_json" 2>/dev/null) || self_exe=""

    if [ -z "$rust_exe" ] && [ -z "$self_exe" ]; then
        skip "${name}/test-run" "no executable"
        continue
    fi
    if [ -z "$rust_exe" ] || [ -z "$self_exe" ]; then
        fail "${name}/test-run" "executable mismatch: rust='${rust_exe:-null}' self='${self_exe:-null}'"
        continue
    fi

    # Compare runtime output between compilers
    compare_runtime "${name}/test-run" "$TMPDIR/test_rust_${name}" "$TMPDIR/test_self_${name}"

    # Validate against // TEST: stdout directive if present
    expected=$(sed -n 's|^// TEST: stdout "\(.*\)"$|\1|p' "$vow_file" | head -1)
    if [ -n "$expected" ]; then
        actual=$("$TMPDIR/test_rust_${name}" 2>/dev/null) || true
        # Interpret \n escapes in expected string
        expected_decoded=$(printf '%b' "$expected")
        if [ "$actual" = "$expected_decoded" ]; then
            pass "${name}/test-expected"
        else
            fail "${name}/test-expected" "expected '$expected' got '$(echo "$actual" | head -c 80)'"
        fi
    fi

    # Validate against // TEST: exit directive if present
    expected_exit=$(sed -n 's|^// TEST: exit \([0-9]*\)$|\1|p' "$vow_file" | head -1)
    if [ -n "$expected_exit" ]; then
        actual_exit=0
        "$TMPDIR/test_rust_${name}" >/dev/null 2>/dev/null || actual_exit=$?
        if [ "$actual_exit" = "$expected_exit" ]; then
            pass "${name}/test-exit"
        else
            fail "${name}/test-exit" "expected exit $expected_exit got $actual_exit"
        fi
    fi
done
echo ""

# ─── Section 5: Debug Mode ─────────────────────────────────────────

echo -e "${BOLD}--- Section 5: Debug Mode ---${RESET}"

# divide.vow: VowViolation at runtime
$RUST build --mode debug --no-verify examples/divide.vow -o "$TMPDIR/rust_divide_debug" >/dev/null 2>/dev/null
run_self build --mode debug --no-verify examples/divide.vow -o "$TMPDIR/self_divide_debug" >/dev/null 2>/dev/null

rust_exit=0 self_exit=0
"$TMPDIR/rust_divide_debug" >"$TMPDIR/rust_dbg_out" 2>"$TMPDIR/rust_dbg_err" || rust_exit=$?
run_self_bin "$TMPDIR/self_divide_debug" >"$TMPDIR/self_dbg_out" 2>"$TMPDIR/self_dbg_err" || self_exit=$?
rust_err=$(cat "$TMPDIR/rust_dbg_err")
self_err=$(cat "$TMPDIR/self_dbg_err")

errors=()
if [ "$rust_exit" -ne 1 ]; then errors+=("rust exit=$rust_exit, expected 1"); fi
if [ "$self_exit" -ne 1 ]; then errors+=("self exit=$self_exit, expected 1"); fi
for pattern in VowViolation Caller "y != 0"; do
    if ! echo "$rust_err" | grep -q "$pattern"; then errors+=("rust stderr missing '$pattern'"); fi
    if ! echo "$self_err" | grep -q "$pattern"; then errors+=("self stderr missing '$pattern'"); fi
done
if [ ${#errors[@]} -eq 0 ]; then
    pass "divide/debug-violation"
else
    fail "divide/debug-violation" "$(IFS='; '; echo "${errors[*]}")"
fi

# callee_blame, clamp, hello: contracts pass (or none), compare runtime
for name in callee_blame clamp hello; do
    $RUST build --mode debug --no-verify "examples/${name}.vow" -o "$TMPDIR/rust_${name}_debug" >/dev/null 2>/dev/null
    run_self build --mode debug --no-verify "examples/${name}.vow" -o "$TMPDIR/self_${name}_debug" >/dev/null 2>/dev/null
    compare_runtime "${name}/debug" "$TMPDIR/rust_${name}_debug" "$TMPDIR/self_${name}_debug"
done
echo ""

# ─── Section 5b: Profile Mode ─────────────────────────────────────

echo -e "${BOLD}--- Section 5b: Profile Mode ---${RESET}"

# Build profile_mode.vow with both compilers
$RUST build --mode profile --no-verify tests/run/profile_mode.vow -o "$TMPDIR/rust_profile_mode" >/dev/null 2>/dev/null
run_self build --mode profile --no-verify tests/run/profile_mode.vow -o "$TMPDIR/self_profile_mode" >/dev/null 2>/dev/null

# Run and capture stderr (profile report) and stdout (program output)
rust_prof_out=$("$TMPDIR/rust_profile_mode" 2>"$TMPDIR/rust_prof_err") || true
self_prof_out=$(run_self_bin "$TMPDIR/self_profile_mode" 2>"$TMPDIR/self_prof_err") || true

errors=()
# Verify stdout matches expected output
if [ "$rust_prof_out" != "5" ]; then errors+=("rust stdout='$rust_prof_out', expected '5'"); fi
if [ "$self_prof_out" != "5" ]; then errors+=("self stdout='$self_prof_out', expected '5'"); fi
# Verify profile report structure in stderr
for compiler in rust self; do
    errfile="$TMPDIR/${compiler}_prof_err"
    if ! grep -q "vow profile report" "$errfile"; then errors+=("${compiler} stderr missing 'vow profile report'"); fi
    if ! grep -q "total calls: 5" "$errfile"; then errors+=("${compiler} stderr missing 'total calls: 5'"); fi
    if ! grep -q "unique functions: 2" "$errfile"; then errors+=("${compiler} stderr missing 'unique functions: 2'"); fi
    # helper called 4 times (4/5 = 80.0%)
    if ! grep -qE "helper\s+4\s" "$errfile"; then errors+=("${compiler} stderr: helper not called 4 times"); fi
    # main called 1 time
    if ! grep -qE "main\s+1\s" "$errfile"; then errors+=("${compiler} stderr: main not called 1 time"); fi
    # helper should appear before main (sorted by count descending)
    helper_line=$(grep -n "helper" "$errfile" | head -1 | cut -d: -f1)
    main_line=$(grep -n "main" "$errfile" | grep -v "vow_main" | tail -1 | cut -d: -f1)
    if [ -n "$helper_line" ] && [ -n "$main_line" ] && [ "$helper_line" -gt "$main_line" ]; then
        errors+=("${compiler} stderr: helper should appear before main (sorted by count)")
    fi
done
if [ ${#errors[@]} -eq 0 ]; then
    pass "profile_mode/profile"
else
    fail "profile_mode/profile" "$(IFS='; '; echo "${errors[*]}")"
fi
echo ""

# ─── Section 6: Multi-Module ───────────────────────────────────────

echo -e "${BOLD}--- Section 6: Multi-Module ---${RESET}"

for multi in stack geometry; do
    main_file="examples/${multi}/main.vow"
    printf "${BOLD}%s${RESET}\n" "$multi"

    # build --no-verify
    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST build --no-verify "$main_file" -o "$TMPDIR/rust_${multi}_main" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self build --no-verify "$main_file" -o "$TMPDIR/self_${multi}_main" 2>/dev/null) || self_exit=$?

    if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
        skip "${multi}/build-no-verify" "empty output (rust=$rust_exit, self=$self_exit)"
    else
        compare_json "${multi}/build-no-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit"
    fi

    # verify
    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST verify "$main_file" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self verify "$main_file" 2>/dev/null) || self_exit=$?

    if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
        skip "${multi}/verify" "empty output (rust=$rust_exit, self=$self_exit)"
    else
        compare_json "${multi}/verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit"
    fi

    # runtime execution
    compare_runtime "${multi}/runtime" "$TMPDIR/rust_${multi}_main" "$TMPDIR/self_${multi}_main"
done
echo ""

# ─── Section 7: Error Handling ─────────────────────────────────────

echo -e "${BOLD}--- Section 7: Error Handling ---${RESET}"

cat > "$TMPDIR/parse_error.vow" <<'EOF'
module M 123
EOF

cat > "$TMPDIR/type_error.vow" <<'EOF'
module Bad
fn f() -> i32 { true }
EOF

cat > "$TMPDIR/missing_module.vow" <<'EOF'
module Main
use nonexistent
fn main() -> i32 { 0 }
EOF

cat > "$TMPDIR/const_type_mismatch.vow" <<'EOF'
module Bad
const BAD: bool = 42;
fn main() -> i32 { 0 }
EOF

for fixture in parse_error type_error missing_module const_type_mismatch; do
    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST build --no-verify "$TMPDIR/${fixture}.vow" -o "$TMPDIR/rust_${fixture}" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self build --no-verify "$TMPDIR/${fixture}.vow" -o "$TMPDIR/self_${fixture}" 2>/dev/null) || self_exit=$?

    if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
        skip "${fixture}/error" "empty output (rust=$rust_exit, self=$self_exit)"
        continue
    fi

    compare_error "${fixture}/error" "$rust_json" "$self_json" "$rust_exit" "$self_exit"
done
echo ""

# ─── Section 8: Help Output ────────────────────────────────────────

echo -e "${BOLD}--- Section 8: Help Output ---${RESET}"

# --help → valid JSON with "tool" key
rust_help=$($RUST --help 2>/dev/null) || true
self_help=$(run_self --help 2>/dev/null) || true

help_ok=true
for name_src in "rust:$rust_help" "self:$self_help"; do
    src="${name_src%%:*}"
    json="${name_src#*:}"
    if ! python3 -c "
import json, sys
try:
    d = json.loads(sys.argv[1])
    assert 'tool' in d, 'missing tool key'
except Exception as e:
    print(str(e)); sys.exit(1)
" "$json" 2>/dev/null; then
        help_ok=false
    fi
done
if $help_ok; then
    pass "help/json"
else
    fail "help/json" "JSON help missing 'tool' key or invalid JSON"
fi

# --help --human → text containing USAGE
rust_human=$($RUST --help --human 2>/dev/null) || true
self_human=$(run_self --help --human 2>/dev/null) || true

if echo "$rust_human" | grep -q "USAGE" && echo "$self_human" | grep -q "USAGE"; then
    pass "help/human"
else
    fail "help/human" "human help output missing USAGE"
fi

# help/coverage-rust: cross-reference grammar.md → Rust --help
if uv run python scripts/check_help_coverage.py docs/skill/grammar.md "$rust_help" 2>/dev/null; then
    pass "help/coverage-rust"
else
    fail "help/coverage-rust" "Rust --help missing grammar.md features"
fi

# help/coverage-self: cross-reference grammar.md → self-hosted --help
if uv run python scripts/check_help_coverage.py docs/skill/grammar.md "$self_help" 2>/dev/null; then
    pass "help/coverage-self"
else
    fail "help/coverage-self" "self-hosted --help missing grammar.md features"
fi
echo ""

# ─── Section 9: Bootstrap Triple Test ──────────────────────────────

echo -e "${BOLD}--- Section 9: Bootstrap Triple Test ---${RESET}"

scripts/concat_vow.sh clif > "$TMPDIR/compiler_clif.vow"

# Stage 0: Rust compiler → Binary A
$RUST --no-verify "$TMPDIR/compiler_clif.vow" -o "$TMPDIR/compiler_a" >/dev/null 2>/dev/null

# Stage 1: A → B
run_self_bin "$TMPDIR/compiler_a" -o "$TMPDIR/compiler_b" "$TMPDIR/compiler_clif.vow" >/dev/null 2>/dev/null

# Stage 2: B → C
run_self_bin "$TMPDIR/compiler_b" -o "$TMPDIR/compiler_c" "$TMPDIR/compiler_clif.vow" >/dev/null 2>/dev/null

hash_b=$(sha256sum "$TMPDIR/compiler_b" | awk '{print $1}')
hash_c=$(sha256sum "$TMPDIR/compiler_c" | awk '{print $1}')

if [ "$hash_b" = "$hash_c" ]; then
    pass "bootstrap/triple-test"
else
    fail "bootstrap/triple-test" "sha256 mismatch: B=$hash_b C=$hash_c"
fi
echo ""

# ─── Section 10: Build + Verify Default Mode ───────────────────────

echo -e "${BOLD}--- Section 10: Build + Verify Default Mode ---${RESET}"

for name in clamp max callee_blame cegis_broken; do
    vow_file="examples/${name}.vow"

    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST build "$vow_file" -o "$TMPDIR/rust_bv_${name}" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self build "$vow_file" -o "$TMPDIR/self_bv_${name}" 2>/dev/null) || self_exit=$?

    if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
        skip "${name}/build-verify" "empty output (rust=$rust_exit, self=$self_exit)"
        continue
    fi

    compare_json "${name}/build-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit"
done
echo ""

# ─── Summary ────────────────────────────────────────────────────────

echo -e "${BOLD}=== Summary ===${RESET}"
echo -e "  ${GREEN}${PASS} passed${RESET}, ${RED}${FAIL} failed${RESET}, ${YELLOW}${SKIP} skipped${RESET}"

if [ ${#FAILURES[@]} -gt 0 ]; then
    echo ""
    echo -e "${RED}Failures:${RESET}"
    for f in "${FAILURES[@]}"; do
        echo "  - $f"
    done
fi

exit $(( FAIL > 0 ? 1 : 0 ))
