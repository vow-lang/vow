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

# ─── Section timing ────────────────────────────────────────────────
#
# Each `section_begin "Name"` records a wall-clock start; the next
# `section_begin` call (or the final summary) prints the elapsed time
# for the previous section. This makes slow sections visible without
# needing to retroactively bisect a 50-minute run.
SCRIPT_START=$(date +%s)
SECTION_NAME=""
SECTION_START=0

section_begin() {
    local name="$1"
    if [ -n "$SECTION_NAME" ]; then
        local now=$(date +%s)
        printf "  ${BOLD}>${RESET} %s done in %ds\n\n" "$SECTION_NAME" $((now - SECTION_START))
    fi
    SECTION_NAME="$name"
    SECTION_START=$(date +%s)
    echo -e "${BOLD}--- ${name} ---${RESET}"
}

section_finalize() {
    if [ -n "$SECTION_NAME" ]; then
        local now=$(date +%s)
        printf "  ${BOLD}>${RESET} %s done in %ds\n" "$SECTION_NAME" $((now - SECTION_START))
        SECTION_NAME=""
    fi
}

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

    # Counterexample JSON can blow past ARG_MAX (~128 KiB on Linux), so
    # write to temp files and pass paths instead of passing the JSON
    # itself as command-line arguments.
    local rust_f="$TMPDIR/cmp_rust_$$.json"
    local self_f="$TMPDIR/cmp_self_$$.json"
    printf '%s' "$rust_json" > "$rust_f"
    printf '%s' "$self_json" > "$self_f"

    local result
    if result=$(python3 -c "
import json, sys

rust_exit = int(sys.argv[1])
self_exit = int(sys.argv[2])
try:
    with open(sys.argv[3]) as f: r = json.load(f)
    with open(sys.argv[4]) as f: s = json.load(f)
except (json.JSONDecodeError, OSError) as e:
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
# A VerifyFailed with a non-empty verify_status is a 'soft' ESBMC outcome
# (timeout / unknown / error / tool_not_found) — ESBMC produced no
# counterexample by design, so the parity check must not require one.
rvs = r.get('verify_status') or ''
svs = s.get('verify_status') or ''
soft_fail = rs == 'VerifyFailed' and ss == 'VerifyFailed' and rvs and svs
if soft_fail:
    if rvs != svs:
        errors.append(f'verify_status: {rvs} vs {svs}')
    # For deterministic inputs the same function should trigger the soft fail on
    # both compilers; a divergence on which function was selected would otherwise
    # pass silently (verify_message is still skipped — ESBMC text is non-deterministic).
    rfn = r.get('function') or ''
    sfn = s.get('function') or ''
    if rfn != sfn:
        errors.append(f'function: {rfn} vs {sfn}')
elif rs == 'VerifyFailed' and ss == 'VerifyFailed':
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
" "$rust_exit" "$self_exit" "$rust_f" "$self_f" 2>&1); then
        pass "$label"
    else
        fail "$label" "$result"
    fi
    rm -f "$rust_f" "$self_f"
}

compare_runtime() {
    local label="$1" rust_bin="$2" self_bin="$3" stdin_file="${4:-}"

    if [ ! -x "$rust_bin" ] || [ ! -x "$self_bin" ]; then
        skip "$label" "binary not found"
        return
    fi

    local rust_out="" self_out="" rust_exit=0 self_exit=0
    if [ -n "$stdin_file" ]; then
        rust_out=$("$rust_bin" < "$stdin_file" 2>/dev/null) || rust_exit=$?
        self_out=$(run_self_bin "$self_bin" < "$stdin_file" 2>/dev/null) || self_exit=$?
    else
        rust_out=$("$rust_bin" </dev/null 2>/dev/null) || rust_exit=$?
        self_out=$(run_self_bin "$self_bin" </dev/null 2>/dev/null) || self_exit=$?
    fi

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

run_stdout_with_optional_stdin() {
    local bin="$1" stdin_file="${2:-}"
    if [ -n "$stdin_file" ]; then
        "$bin" < "$stdin_file" 2>/dev/null
    else
        "$bin" </dev/null 2>/dev/null
    fi
}

run_discard_with_optional_stdin() {
    local bin="$1" stdin_file="${2:-}"
    if [ -n "$stdin_file" ]; then
        "$bin" < "$stdin_file" >/dev/null 2>/dev/null
    else
        "$bin" </dev/null >/dev/null 2>/dev/null
    fi
}

compare_error() {
    local label="$1" rust_json="$2" self_json="$3" rust_exit="$4" self_exit="$5"

    local rust_f="$TMPDIR/cmp_err_rust_$$.json"
    local self_f="$TMPDIR/cmp_err_self_$$.json"
    printf '%s' "$rust_json" > "$rust_f"
    printf '%s' "$self_json" > "$self_f"

    local result
    if result=$(python3 -c "
import json, sys

rust_exit = int(sys.argv[1])
self_exit = int(sys.argv[2])
try:
    with open(sys.argv[3]) as f: r = json.load(f)
    with open(sys.argv[4]) as f: s = json.load(f)
except (json.JSONDecodeError, OSError) as e:
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
" "$rust_exit" "$self_exit" "$rust_f" "$self_f" 2>&1); then
        pass "$label"
    else
        fail "$label" "$result"
    fi
    rm -f "$rust_f" "$self_f"
}

echo -e "${BOLD}=== Phase 20.1: Full Test Suite ===${RESET}"
echo ""

section_begin "Section 0: Setup"
echo -e "${BOLD}Building Rust compiler...${RESET}"
cargo build --all --release 2>&1 | tail -1
echo -e "${BOLD}Building self-hosted compiler...${RESET}"
$RUST --no-verify compiler/main.vow -o "$TMPDIR/vowc_self" >/dev/null 2>/dev/null
SELF="$TMPDIR/vowc_self"

# ─── Section 0b: Concrete block-region parity ──────────────────────

section_begin "Section 0b: Concrete Block-Region Parity"
rust_ir="$TMPDIR/compiler_rust.ir"
self_ir="$TMPDIR/compiler_self.ir"
rust_ir_err="$TMPDIR/compiler_rust_ir.err"
self_ir_err="$TMPDIR/compiler_self_ir.err"
if "$RUST" build --no-verify --dump-ir compiler/main.vow >"$rust_ir" 2>"$rust_ir_err" \
    && run_self build --no-verify --dump-ir compiler/main.vow >"$self_ir" 2>"$self_ir_err"; then
    if python3 - "$rust_ir" "$self_ir" <<'PY'
import re
import sys

inst_re = re.compile(r'%([0-9]+) = RegionAlloc.*<region=block_([0-9]+)>')

def collect(path):
    out = {}
    func = None
    with open(path, encoding='utf-8') as fh:
        for line in fh:
            if line.startswith('fn '):
                func = line[3:].split('(', 1)[0].strip()
                continue
            match = inst_re.search(line)
            if match and func is not None:
                out[(func, int(match.group(1)))] = int(match.group(2))
    return out

rust = collect(sys.argv[1])
self_hosted = collect(sys.argv[2])
if rust == self_hosted:
    print(f'OK ({len(rust)} concrete block placements)')
    sys.exit(0)

missing = sorted(set(rust) - set(self_hosted))
extra = sorted(set(self_hosted) - set(rust))
mismatched = sorted(k for k in set(rust) & set(self_hosted) if rust[k] != self_hosted[k])
parts = []
if missing:
    parts.append('missing in self: ' + ', '.join(f'{f}%{i}->block_{rust[(f, i)]}' for f, i in missing[:8]))
if extra:
    parts.append('extra in self: ' + ', '.join(f'{f}%{i}->block_{self_hosted[(f, i)]}' for f, i in extra[:8]))
if mismatched:
    parts.append('mismatch: ' + ', '.join(
        f'{f}%{i}: rust block_{rust[(f, i)]} vs self block_{self_hosted[(f, i)]}'
        for f, i in mismatched[:8]
    ))
print('; '.join(parts))
sys.exit(1)
PY
    then
        pass "compiler/concrete-block-region-parity"
    else
        fail "compiler/concrete-block-region-parity" "concrete block placements differ"
    fi
else
    fail "compiler/concrete-block-region-parity" "failed to dump compiler IR"
fi

# ─── Section 1: Build --no-verify ─────────────────────────────────

section_begin "Section 1: Build --no-verify"
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

section_begin "Section 2: Verify"
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

# ─── Section 2b: Verifier C Preamble ──────────────────────────────

section_begin "Section 2b: Verifier C Preamble"

fake_esbmc_dir="$TMPDIR/fake-esbmc"
mkdir -p "$fake_esbmc_dir"
cat > "$fake_esbmc_dir/esbmc" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

capture_dir="${VOW_ESBMC_CAPTURE_DIR:?}"
mkdir -p "$capture_dir"

for arg in "$@"; do
    if [ -f "$arg" ] && [ "${arg##*.}" = "c" ]; then
        dest=$(mktemp "$capture_dir/esbmc.XXXXXX.c")
        cp "$arg" "$dest"
        break
    fi
done

echo "VERIFICATION SUCCESSFUL"
SH
chmod +x "$fake_esbmc_dir/esbmc"

u64_preamble_fixture="$TMPDIR/u64_nondet_preamble.vow"
cat > "$u64_preamble_fixture" <<'VOW'
module U64NondetPreamble

fn keep_u64(x: u64) -> u64 vow {
    ensures: result == x
} {
    x
}

fn main() -> i32 {
    0
}
VOW

check_unsigned_long_preamble_capture() {
    local label="$1" capture_dir="$2" json="$3" exit_code="$4"
    if grep -R -q 'extern unsigned long __VERIFIER_nondet_unsigned_long(void);' "$capture_dir" 2>/dev/null; then
        pass "$label"
    else
        local verify_status=""
        verify_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$json" 2>/dev/null) || verify_status=""
        fail "$label" "missing unsigned-long nondet extern in captured C (verify_status=${verify_status:-unknown}, exit=$exit_code)"
    fi
}

rust_capture="$TMPDIR/rust-esbmc-capture"
self_capture="$TMPDIR/self-esbmc-capture"
mkdir -p "$rust_capture" "$self_capture"

rust_json="" self_json="" rust_exit=0 self_exit=0
rust_json=$(PATH="$fake_esbmc_dir:$PATH" VOW_ESBMC_CAPTURE_DIR="$rust_capture" \
    "$RUST" verify --no-cache --verify-jobs 1 "$u64_preamble_fixture" 2>/dev/null) || rust_exit=$?
self_json=$(PATH="$fake_esbmc_dir:$PATH" VOW_ESBMC_CAPTURE_DIR="$self_capture" \
    run_self verify --no-cache --verify-jobs 1 "$u64_preamble_fixture" 2>/dev/null) || self_exit=$?

if [ -z "$rust_json" ]; then
    fail "verifier-preamble/rust" "empty output (exit=$rust_exit)"
else
    check_unsigned_long_preamble_capture "verifier-preamble/rust" "$rust_capture" "$rust_json" "$rust_exit"
fi

if [ -z "$self_json" ]; then
    fail "verifier-preamble/self" "empty output (exit=$self_exit)"
else
    check_unsigned_long_preamble_capture "verifier-preamble/self" "$self_capture" "$self_json" "$self_exit"
fi
echo ""

# ─── Section 3: Runtime Execution ──────────────────────────────────

section_begin "Section 3: Runtime Execution"
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

section_begin "Section 4: Run Tests"
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

    test_stdin=$(sed -n 's|^// TEST: stdin "\(.*\)"$|\1|p' "$vow_file" | head -1)
    test_stdin_file=$(sed -n 's|^// TEST: stdin-file \(.*\)$|\1|p' "$vow_file" | head -1)
    stdin_path=""
    if [ -n "$test_stdin_file" ]; then
        stdin_path="$(dirname "$vow_file")/$test_stdin_file"
        if [ ! -f "$stdin_path" ]; then
            fail "${name}/test-run" "stdin fixture not found: $stdin_path"
            continue
        fi
    elif [ -n "$test_stdin" ]; then
        stdin_path="$TMPDIR/stdin_${name}.txt"
        printf '%b' "$test_stdin" > "$stdin_path"
    fi

    # Compare runtime output between compilers
    compare_runtime "${name}/test-run" "$TMPDIR/test_rust_${name}" "$TMPDIR/test_self_${name}" "$stdin_path"

    # Validate against // TEST: stdout directive if present
    expected=$(sed -n 's|^// TEST: stdout "\(.*\)"$|\1|p' "$vow_file" | head -1)
    if [ -n "$expected" ]; then
        actual=$(run_stdout_with_optional_stdin "$TMPDIR/test_rust_${name}" "$stdin_path") || true
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
        run_discard_with_optional_stdin "$TMPDIR/test_rust_${name}" "$stdin_path" || actual_exit=$?
        if [ "$actual_exit" = "$expected_exit" ]; then
            pass "${name}/test-exit"
        else
            fail "${name}/test-exit" "expected exit $expected_exit got $actual_exit"
        fi
    fi
done
echo ""

# ─── Section 4b: Verify Tests (tests/verify/) ─────────────────────

section_begin "Section 4b: Verify Tests"
for vow_file in tests/verify/*.vow; do
    name=$(basename "$vow_file" .vow)

    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST verify "$vow_file" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self verify "$vow_file" 2>/dev/null) || self_exit=$?

    if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
        skip "${name}/verify-test" "empty output (rust=$rust_exit, self=$self_exit)"
        continue
    fi

    compare_json "${name}/verify-test" "$rust_json" "$self_json" "$rust_exit" "$self_exit"
    actual_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$rust_json" 2>/dev/null) || actual_status=""
    if [ -n "$actual_status" ] && [ "$actual_status" != "Verified" ]; then
        fail "${name}/verify-expected-pass" "expected Verified, got $actual_status"
    fi
done
echo ""

# ─── Section 4c: Verify-Fail Tests (tests/verify-fail/) ───────────

section_begin "Section 4c: Verify-Fail Tests"
for vow_file in tests/verify-fail/*.vow; do
    name=$(basename "$vow_file" .vow)

    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST verify "$vow_file" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self verify "$vow_file" 2>/dev/null) || self_exit=$?

    if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
        skip "${name}/verify-fail-test" "empty output (rust=$rust_exit, self=$self_exit)"
        continue
    fi

    compare_json "${name}/verify-fail-test" "$rust_json" "$self_json" "$rust_exit" "$self_exit"
    actual_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$rust_json" 2>/dev/null) || actual_status=""
    if [ -n "$actual_status" ] && [ "$actual_status" != "VerifyFailed" ]; then
        fail "${name}/verify-expected-fail" "expected VerifyFailed, got $actual_status"
    fi
done
echo ""

# ─── Section 4d: Verify-Skip Tests (tests/verify-skip/) ───────────
#
# Functions that exercise a non-modelable construct (e.g. nested-collection
# vec ops, issue #505) must be Skipped (fail-closed), not Verified. Such a
# file cannot live under tests/verify/ because Section 4b requires "Verified";
# here each file gives the offending function a contract so it becomes a real
# verification target, then we assert status == "Skipped" with Rust/self
# parity.

section_begin "Section 4d: Verify-Skip Tests"
for vow_file in tests/verify-skip/*.vow; do
    name=$(basename "$vow_file" .vow)

    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST verify "$vow_file" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self verify "$vow_file" 2>/dev/null) || self_exit=$?

    if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
        skip "${name}/verify-skip-test" "empty output (rust=$rust_exit, self=$self_exit)"
        continue
    fi

    compare_json "${name}/verify-skip-test" "$rust_json" "$self_json" "$rust_exit" "$self_exit"
    actual_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$rust_json" 2>/dev/null) || actual_status=""
    if [ -n "$actual_status" ] && [ "$actual_status" != "Skipped" ]; then
        fail "${name}/verify-expected-skip" "expected Skipped, got $actual_status"
    fi
done
echo ""

# ─── Section 4e: Verifier-Evaluation Suite (issue #334) ───────────
#
# The verifier's *acceptance* harness, distinct from the Rust/self parity
# checks above: it asserts each labelled program in tests/verify* against its
# ground-truth `// TEST:` directives (status + Caller/Callee blame + violated
# vow_id), runs a vacuity guard over the should-pass set, and surfaces
# false-accepts (SOUNDNESS) and false-rejects (PRECISION) under dedicated loud
# banners. Runs against the Rust verifier; Rust/self parity is covered by 4b-4d.

section_begin "Section 4e: Verifier-Evaluation Suite (#334)"
ve_out="$TMPDIR/verify_eval.out"
if python3 scripts/verify_eval.py --verifier "$RUST" --output-dir "$TMPDIR/verify-eval" >"$ve_out" 2>&1; then
    pass "verifier-eval/ground-truth"
else
    fail "verifier-eval/ground-truth" "ground-truth mismatch — see banners below"
fi
sed 's/^/    /' "$ve_out"
echo ""

# ─── Section 5: Debug Mode ─────────────────────────────────────────

section_begin "Section 5: Debug Mode"

# divide.vow: VowViolation at runtime
$RUST build --mode debug --no-verify examples/divide.vow -o "$TMPDIR/rust_divide_debug" >/dev/null 2>/dev/null
run_self build --mode debug --no-verify examples/divide.vow -o "$TMPDIR/self_divide_debug" >/dev/null 2>/dev/null

rust_exit=0 self_exit=0
"$TMPDIR/rust_divide_debug" </dev/null >"$TMPDIR/rust_dbg_out" 2>"$TMPDIR/rust_dbg_err" || rust_exit=$?
run_self_bin "$TMPDIR/self_divide_debug" </dev/null >"$TMPDIR/self_dbg_out" 2>"$TMPDIR/self_dbg_err" || self_exit=$?
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

section_begin "Section 5b: Profile Mode"

# Build profile_mode.vow with both compilers
$RUST build --mode profile --no-verify tests/run/profile_mode.vow -o "$TMPDIR/rust_profile_mode" >/dev/null 2>/dev/null
run_self build --mode profile --no-verify tests/run/profile_mode.vow -o "$TMPDIR/self_profile_mode" >/dev/null 2>/dev/null

# Run and capture stderr (profile report) and stdout (program output)
rust_prof_out=$("$TMPDIR/rust_profile_mode" </dev/null 2>"$TMPDIR/rust_prof_err") || true
self_prof_out=$(run_self_bin "$TMPDIR/self_profile_mode" </dev/null 2>"$TMPDIR/self_prof_err") || true

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

# ─── Section 5c: Sanitize Mode ────────────────────────────────────

section_begin "Section 5c: Sanitize Mode"

# sanitize_vec.vow: Vec operations with sanitize instrumentation
$RUST build --mode sanitize --no-verify tests/debug/sanitize_vec.vow -o "$TMPDIR/rust_sanitize_vec" >/dev/null 2>/dev/null
run_self build --mode sanitize --no-verify tests/debug/sanitize_vec.vow -o "$TMPDIR/self_sanitize_vec" >/dev/null 2>/dev/null
compare_runtime "sanitize_vec/sanitize" "$TMPDIR/rust_sanitize_vec" "$TMPDIR/self_sanitize_vec"

echo ""
# ─── Section 6: Multi-Module ───────────────────────────────────────

section_begin "Section 6: Multi-Module"

for multi in stack geometry bignum gc math heap; do
    case "$multi" in
        math) main_file="lib/math/main.vow" ;;
        heap) main_file="lib/heap/main.vow" ;;
        *)    main_file="examples/${multi}/main.vow" ;;
    esac
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

# ─── Section 6b: Multi-Module Fixtures (tests/multi/) ──────────────
# Discovers every tests/multi/<dir>/main.vow, builds it with both
# compilers (use-based module loading resolves siblings), checks
# rust/self parity, and validates its // TEST: directives. Covers the
# vmod_* serialization reject-path fixtures, which were otherwise built
# only by the concat bootstrap and never executed.

section_begin "Section 6b: Multi-Module Fixtures"

for dir in tests/multi/*/; do
    name=$(basename "$dir")
    main_file="${dir}main.vow"
    [ -f "$main_file" ] || continue
    printf "${BOLD}%s${RESET}\n" "$name"

    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST build --no-verify "$main_file" -o "$TMPDIR/rust_multi_${name}" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self build --no-verify "$main_file" -o "$TMPDIR/self_multi_${name}" 2>/dev/null) || self_exit=$?

    if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
        skip "${name}/build" "empty output (rust=$rust_exit, self=$self_exit)"
        continue
    fi
    compare_json "${name}/build" "$rust_json" "$self_json" "$rust_exit" "$self_exit"

    # rust/self runtime parity (exit code + stdout)
    compare_runtime "${name}/runtime" "$TMPDIR/rust_multi_${name}" "$TMPDIR/self_multi_${name}"

    # Validate // TEST: stdout directive (single-line) against the rust exe;
    # compare_runtime above guarantees the self exe matches.
    expected=$(sed -n 's|^// TEST: stdout "\(.*\)"$|\1|p' "$main_file" | head -1)
    if [ -n "$expected" ]; then
        actual=$(run_stdout_with_optional_stdin "$TMPDIR/rust_multi_${name}" "") || true
        expected_decoded=$(printf '%b' "$expected")
        if [ "$actual" = "$expected_decoded" ]; then
            pass "${name}/expected"
        else
            fail "${name}/expected" "expected '$expected' got '$(echo "$actual" | head -c 80)'"
        fi
    fi

    # Validate // TEST: exit directive (the vmod reject fixtures expect 1).
    expected_exit=$(sed -n 's|^// TEST: exit \([0-9]*\)$|\1|p' "$main_file" | head -1)
    if [ -n "$expected_exit" ]; then
        actual_exit=0
        run_discard_with_optional_stdin "$TMPDIR/rust_multi_${name}" "" || actual_exit=$?
        if [ "$actual_exit" = "$expected_exit" ]; then
            pass "${name}/exit"
        else
            fail "${name}/exit" "expected exit $expected_exit got $actual_exit"
        fi
    fi
done
echo ""

# ─── Section 7: Error Handling ─────────────────────────────────────

section_begin "Section 7: Error Handling"

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

section_begin "Section 8: Help Output"

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
if uv run python scripts/check_help_coverage.py docs/spec/grammar.md "$rust_help" 2>/dev/null; then
    pass "help/coverage-rust"
else
    fail "help/coverage-rust" "Rust --help missing grammar.md features"
fi

# help/coverage-self: cross-reference grammar.md → self-hosted --help
if uv run python scripts/check_help_coverage.py docs/spec/grammar.md "$self_help" 2>/dev/null; then
    pass "help/coverage-self"
else
    fail "help/coverage-self" "self-hosted --help missing grammar.md features"
fi

# help/skills-dir-drift: confirm skills/vow/ matches what generate_help.py
# would produce (so `npx skills add vow-lang/vow` keeps installing the live skill).
if uv run python scripts/generate_help.py --check >/dev/null 2>&1; then
    pass "help/skills-dir-drift"
else
    fail "help/skills-dir-drift" "skills/vow/ drifted from generated content; run 'uv run python scripts/generate_help.py'"
fi

# contract-quality/weak-gate: ratchet on static contract quality across the
# self-hosted compiler — fail if the weak/tautological contract count exceeds the
# committed baseline (#81). Static classification only (no ESBMC), so it is cheap.
# Capture the contracts JSON in its own step so a producer failure (parse error,
# missing binary, compiler crash) is reported as itself — with its stderr visible —
# instead of being masked as a baseline breach by the checker's empty-stdin exit.
contract_quality_json="$TMPDIR/contract_quality.json"
if ! run_self contracts compiler/main.vow >"$contract_quality_json"; then
    fail "contract-quality/weak-gate" "vow contracts compiler/main.vow failed (see stderr above); could not evaluate contract quality"
else
    # Distinguish the checker's exit codes: 0 = pass, 1 = baseline breach (a real
    # contract-quality regression), 2 = structural error (malformed JSON / missing
    # or non-integer counter — the checker's stderr above names the cause). A bare
    # else would mislabel a schema error as a baseline breach.
    quality_status=0
    uv run python scripts/check_contract_quality.py <"$contract_quality_json" || quality_status=$?
    if [ "$quality_status" -eq 0 ]; then
        pass "contract-quality/weak-gate"
    elif [ "$quality_status" -eq 1 ]; then
        fail "contract-quality/weak-gate" "weak/tautological contracts exceeded baseline; strengthen the new contract or adjust scripts/check_contract_quality.py with justification"
    else
        fail "contract-quality/weak-gate" "contract quality check could not run (malformed 'vow contracts' output / schema mismatch; see stderr above)"
    fi
fi
echo ""

# ─── Section 9: Bootstrap Triple Test ──────────────────────────────

section_begin "Section 9: Bootstrap Triple Test"

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

section_begin "Section 10: Build + Verify Default Mode"

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

# ─── Section 10b: Test Subcommand ───────────────────────────────────

section_begin "Section 10b: Test Subcommand"

# Run vowc test with both compilers on compiler/ directory
rust_test_json=$($RUST test compiler/ 2>/dev/null) || true
self_test_json=$(run_self test compiler/ 2>/dev/null) || true

if [ -z "$rust_test_json" ] || [ -z "$self_test_json" ]; then
    skip "test/subcommand" "empty output"
else
    # Check that both report TestsPassed
    rust_status=$(echo "$rust_test_json" | uv run python -c "import json,sys; print(json.load(sys.stdin)['status'])" 2>/dev/null) || rust_status=""
    self_status=$(echo "$self_test_json" | uv run python -c "import json,sys; print(json.load(sys.stdin)['status'])" 2>/dev/null) || self_status=""

    if [ "$rust_status" = "TestsPassed" ] && [ "$self_status" = "TestsPassed" ]; then
        pass "test/status"
    else
        fail "test/status" "rust=$rust_status self=$self_status"
    fi

    # Check counts match
    rust_total=$(echo "$rust_test_json" | uv run python -c "import json,sys; print(json.load(sys.stdin)['total'])" 2>/dev/null) || rust_total=""
    self_total=$(echo "$self_test_json" | uv run python -c "import json,sys; print(json.load(sys.stdin)['total'])" 2>/dev/null) || self_total=""

    if [ "$rust_total" = "$self_total" ] && [ -n "$rust_total" ] && [ "$rust_total" -gt 0 ]; then
        pass "test/counts"
    else
        fail "test/counts" "rust_total=$rust_total self_total=$self_total"
    fi

    # Check contract_density field exists
    rust_cd=$(echo "$rust_test_json" | uv run python -c "import json,sys; d=json.load(sys.stdin); print('ok' if 'contract_density' in d else 'missing')" 2>/dev/null) || rust_cd=""
    self_cd=$(echo "$self_test_json" | uv run python -c "import json,sys; d=json.load(sys.stdin); print('ok' if 'contract_density' in d else 'missing')" 2>/dev/null) || self_cd=""

    if [ "$rust_cd" = "ok" ] && [ "$self_cd" = "ok" ]; then
        pass "test/contract-density"
    else
        fail "test/contract-density" "rust=$rust_cd self=$self_cd"
    fi

    # Check --filter works
    rust_filter=$($RUST test compiler/ --filter arith 2>/dev/null) || true
    filter_total=$(echo "$rust_filter" | uv run python -c "import json,sys; print(json.load(sys.stdin)['total'])" 2>/dev/null) || filter_total=""

    if [ "$filter_total" = "1" ]; then
        pass "test/filter"
    else
        fail "test/filter" "expected 1 test with --filter arith, got $filter_total"
    fi
fi
echo ""

# ─── Section 11: Arena Primitive ESBMC Verification ────────────────

section_begin "Section 11: Arena Primitive Verification"
# Run under the same 2 GB virtual-memory cap as run_self so this also guards
# against a regression in the verify invocation: with the single-shot
# --unwind 5 --boolector command (#516) the harness peaks at ~0.5 GB, but
# --incremental-bmc / Bitwuzla blew past 2 GB and OOM-killed here (#546).
if command -v esbmc >/dev/null 2>&1; then
    if (ulimit -v 2000000; cd vow-runtime/verify && make verify) >"$TMPDIR/arena_verify.log" 2>&1; then
        pass "arena/esbmc"
    else
        fail "arena/esbmc" "$(tail -5 "$TMPDIR/arena_verify.log")"
    fi
else
    skip "arena/esbmc" "esbmc not on PATH"
fi
echo ""

# ─── Section 12: vowc mutants Smoke Test ────────────────────────────

section_begin "Section 12: vowc mutants Smoke Test"
if [ -f tests/mutants/tests.sh ]; then
    if (ulimit -v 2000000; VOWC_BIN="$SELF" bash tests/mutants/tests.sh) >"$TMPDIR/vowc-mutants-tests.log" 2>&1; then
        pass "vowc-mutants/tests"
    else
        fail "vowc-mutants/tests" "$(tail -10 "$TMPDIR/vowc-mutants-tests.log")"
    fi
else
    skip "vowc-mutants" "tests/mutants/tests.sh not present"
fi
echo ""

# ─── Section 13: vow complexity Parity ──────────────────────────────

section_begin "Section 13: vow complexity Parity"
for vow_file in tests/fixtures/complexity/*.vow; do
    [ -f "$vow_file" ] || continue
    name=$(basename "$vow_file" .vow)
    rust_json=$("$RUST" complexity "$vow_file" 2>/dev/null)
    self_json=$(run_self complexity "$vow_file" 2>/dev/null)
    golden="tests/fixtures/complexity/${name}.expected.json"
    if [ "$rust_json" != "$self_json" ]; then
        fail "complexity/${name}" "JSON differs between compilers"
    elif [ -f "$golden" ] && [ "$rust_json" != "$(cat "$golden")" ]; then
        fail "complexity/${name}" "output differs from golden ${name}.expected.json"
    else
        pass "complexity/${name} (byte-identical + golden)"
    fi
    # AST<->IR self-check: the AST decision-count cyclomatic and the IR
    # branch-count cyclomatic_ir are independent computations that must agree
    # on these clean-control-flow fixtures (cross-validates both).
    # `break_value` is exempt: break-with-value requires an unconditional `loop`,
    # which the AST counts as a decision but the branch-count cyclomatic_ir does
    # not (the documented AST<->IR divergence) — agreement cannot hold here by
    # construction. Byte-identity + golden above still cover this fixture.
    if [ "$name" = "break_value" ]; then
        skip "complexity/${name}/ast-ir-cyclomatic-agree" "loop divergence (expected)"
    elif echo "$rust_json" | python3 -c "import sys,json; d=json.load(sys.stdin); sys.exit(0 if all(f['structural']['cyclomatic']==f['structural']['cyclomatic_ir'] for f in d['files'][0]['functions']) else 1)" 2>/dev/null; then
        pass "complexity/${name}/ast-ir-cyclomatic-agree"
    else
        fail "complexity/${name}/ast-ir-cyclomatic-agree" "cyclomatic != cyclomatic_ir"
    fi
done
# Exit-code gating must agree across compilers (deep has cyclomatic 4).
# These commands exit nonzero by design, so capture the code with `|| code=$?`
# (a bare `cmd; code=$?` would trip `set -e` and abort the script here).
r_gate=0; "$RUST" complexity tests/fixtures/complexity/nested.vow --max-cyclomatic 1 >/dev/null 2>&1 || r_gate=$?
s_gate=0; run_self complexity tests/fixtures/complexity/nested.vow --max-cyclomatic 1 >/dev/null 2>&1 || s_gate=$?
if [ "$r_gate" = "$s_gate" ] && [ "$r_gate" != "0" ]; then
    pass "complexity/exit-gating (--max-cyclomatic 1 -> $r_gate, both)"
else
    fail "complexity/exit-gating" "rust=$r_gate self=$s_gate (expected equal, nonzero)"
fi
# A malformed --max-* value must fail closed (nonzero) in BOTH compilers, never
# silently disable the opt-in gate. Exact codes differ (clap=2, self-hosted=1).
r_bad=0; "$RUST" complexity tests/fixtures/complexity/params_basic.vow --max-score notanint >/dev/null 2>&1 || r_bad=$?
s_bad=0; run_self complexity tests/fixtures/complexity/params_basic.vow --max-score notanint >/dev/null 2>&1 || s_bad=$?
if [ "$r_bad" != "0" ] && [ "$s_bad" != "0" ]; then
    pass "complexity/gate-fail-closed (--max-score notanint -> rust=$r_bad self=$s_bad, both nonzero)"
else
    fail "complexity/gate-fail-closed" "rust=$r_bad self=$s_bad (expected both nonzero)"
fi
# A non-ASCII (UTF-8) source path must stay byte-identical across compilers: the
# JSON escaper must emit raw UTF-8 bytes, not mojibake (Rust byte-as-char bug).
utf8dir=$(mktemp -d)
cp tests/fixtures/complexity/params_basic.vow "$utf8dir/café.vow"
if diff -q <("$RUST" complexity "$utf8dir/café.vow" 2>/dev/null) <(run_self complexity "$utf8dir/café.vow" 2>/dev/null) >/dev/null; then
    pass "complexity/utf8-path-parity (café.vow byte-identical)"
else
    fail "complexity/utf8-path-parity" "non-ASCII path JSON diverges between compilers"
fi
rm -rf "$utf8dir"
echo ""

# ─── Section 6: Perfetto Trace (--perfetto, #784) ───────────────────
section_begin "Section 6: Perfetto Trace"
ptrace_dir=$(mktemp -d)
# (a) build --no-verify --perfetto: valid gz trace with frontend + codegen spans + counters.
if run_self build --no-verify --perfetto "$ptrace_dir/build.json.gz" examples/hello.vow -o "$ptrace_dir/hello" >/dev/null 2>&1 \
   && python3 scripts/validate_trace_gz.py "$ptrace_dir/build.json.gz" --require parse,codegen >/dev/null 2>&1; then
    pass "perfetto/build-trace (gz+json, parse+codegen spans)"
else
    fail "perfetto/build-trace" "missing or malformed trace from build --no-verify --perfetto"
fi
# (b) build --no-verify stdout JSON must be byte-identical with/without --perfetto (same -o).
boff=$(run_self build --no-verify examples/hello.vow -o "$ptrace_dir/h" 2>/dev/null)
bon=$(run_self build --no-verify --perfetto "$ptrace_dir/b.json.gz" examples/hello.vow -o "$ptrace_dir/h" 2>/dev/null)
if [ "$boff" = "$bon" ]; then
    pass "perfetto/build-stdout-parity (build JSON identical with/without --perfetto)"
else
    fail "perfetto/build-stdout-parity" "stdout build JSON changed with --perfetto"
fi
# (c) verify --perfetto: esbmc proof span present + stdout parity (needs ESBMC).
if command -v esbmc >/dev/null 2>&1; then
    if run_self verify --perfetto "$ptrace_dir/verify.json.gz" examples/divide.vow >/dev/null 2>&1 \
       && python3 scripts/validate_trace_gz.py "$ptrace_dir/verify.json.gz" --require parse,esbmc >/dev/null 2>&1; then
        pass "perfetto/verify-trace (esbmc proof span + flow)"
    else
        fail "perfetto/verify-trace" "missing or malformed trace from verify --perfetto"
    fi
    voff=$(run_self verify examples/divide.vow 2>/dev/null)
    von=$(run_self verify --perfetto "$ptrace_dir/parity.json.gz" examples/divide.vow 2>/dev/null)
    if [ "$voff" = "$von" ]; then
        pass "perfetto/verify-stdout-parity (verify JSON identical with/without --perfetto)"
    else
        fail "perfetto/verify-stdout-parity" "stdout verify JSON changed with --perfetto"
    fi
else
    skip "perfetto/verify-trace" "ESBMC not installed"
    skip "perfetto/verify-stdout-parity" "ESBMC not installed"
fi
rm -rf "$ptrace_dir"
echo ""

# ─── Summary ────────────────────────────────────────────────────────

section_finalize
echo ""

echo -e "${BOLD}=== Summary ===${RESET}"
SCRIPT_END=$(date +%s)
TOTAL=$((SCRIPT_END - SCRIPT_START))
echo -e "  ${GREEN}${PASS} passed${RESET}, ${RED}${FAIL} failed${RESET}, ${YELLOW}${SKIP} skipped${RESET} in ${TOTAL}s"

if [ ${#FAILURES[@]} -gt 0 ]; then
    echo ""
    echo -e "${RED}Failures:${RESET}"
    for f in "${FAILURES[@]}"; do
        echo "  - $f"
    done
fi

exit $(( FAIL > 0 ? 1 : 0 ))
