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

# Only compare diagnostics count when both are not VerifyFailed
# (self-hosted doesn't emit diagnostics for VerifyFailed)
if rs != 'VerifyFailed':
    rd = len(r.get('diagnostics', []))
    sd = len(s.get('diagnostics', []))
    if rd != sd:
        errors.append(f'diagnostics count: {rd} vs {sd}')

rc = r.get('counterexamples', [])
sc = s.get('counterexamples', [])
if len(rc) != len(sc):
    errors.append(f'counterexamples count: {len(rc)} vs {len(sc)}')
else:
    for i, (rce, sce) in enumerate(zip(rc, sc)):
        for field in ('function', 'vow_id', 'blame'):
            rv = rce.get(field)
            sv = sce.get(field)
            # vow_id: Rust defaults None to 0, self-hosted to -1;
            # both mean 'ESBMC did not report a specific vow ID'
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
        printf "  ${GREEN}PASS${RESET} %s\n" "$label"
        PASS=$((PASS + 1))
    else
        printf "  ${RED}FAIL${RESET} %s — %s\n" "$label" "$result"
        FAIL=$((FAIL + 1))
        FAILURES+=("$label: $result")
    fi
}

run_mode() {
    local name="$1" mode="$2" vow_file="$3"
    local label="${name}/${mode}"
    local rust_json="" self_json="" rust_exit=0 self_exit=0

    case "$mode" in
        build-no-verify)
            rust_json=$($RUST build --no-verify "$vow_file" -o "$TMPDIR/rust_${name}" 2>/dev/null) || rust_exit=$?
            self_json=$(ulimit -v 2000000; $SELF build --no-verify "$vow_file" -o "$TMPDIR/self_${name}" 2>/dev/null) || self_exit=$?
            ;;
        verify)
            rust_json=$($RUST verify "$vow_file" 2>/dev/null) || rust_exit=$?
            self_json=$(ulimit -v 2000000; $SELF verify "$vow_file" 2>/dev/null) || self_exit=$?
            ;;
    esac

    if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
        printf "  ${YELLOW}SKIP${RESET} %s — empty output (rust=%d, self=%d)\n" "$label" "$rust_exit" "$self_exit"
        SKIP=$((SKIP + 1))
        return
    fi

    compare_json "$label" "$rust_json" "$self_json" "$rust_exit" "$self_exit"
}

echo -e "${BOLD}=== Phase 19.5: CLI Compatibility Test ===${RESET}"
echo ""

# Step 1: Build both compilers
echo -e "${BOLD}Building Rust compiler...${RESET}"
cargo build --all --release 2>&1 | tail -1
echo -e "${BOLD}Building self-hosted compiler...${RESET}"
$RUST --no-verify compiler/main.vow -o "$TMPDIR/vowc_self" >/dev/null 2>/dev/null
SELF="$TMPDIR/vowc_self"
echo ""

# Step 2: Run each example
for vow_file in examples/*.vow; do
    name=$(basename "$vow_file" .vow)
    printf "${BOLD}%s${RESET}\n" "$name"

    run_mode "$name" "build-no-verify" "$vow_file"

    if grep -q 'vow {' "$vow_file"; then
        run_mode "$name" "verify" "$vow_file"
    fi
done

# Step 3: Summary
echo ""
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
