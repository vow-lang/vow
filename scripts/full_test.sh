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
# EXIT alone does not fire on an untrapped SIGTERM/SIGINT/SIGHUP: bash dies
# immediately and this whole scratch tree survives. It routinely reaches several
# GB, and on hosts where /tmp is a tmpfs that is abandoned RAM. Re-raise rather
# than cleaning up inline -- a bare `trap 'rm -rf "$TMPDIR"' TERM` would delete
# the scratch dir and then let the script keep running against it, because a
# bash signal handler resumes execution unless it exits.
trap 'rm -rf "$TMPDIR"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

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

setup_compilers() {
    echo -e "${BOLD}Building Rust compiler...${RESET}"
    cargo build --all --release 2>&1 | tail -1
    echo -e "${BOLD}Building self-hosted compiler...${RESET}"
    $RUST --no-verify compiler/main.vow -o "$TMPDIR/vowc_self" >/dev/null 2>/dev/null
    SELF="$TMPDIR/vowc_self"
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

# Run one scripts/parity.py comparator. The optional fixture path lets the
# comparator find a fixture's known-divergence suppression (a `// TEST:`
# directive, or docs/equivalence/ledger.json), which it reports as `SKIP:`.
run_parity() {
    local mode="$1" label="$2" rust_json="$3" self_json="$4" rust_exit="$5" self_exit="$6" fixture_path="${7:-}"

    # Counterexample JSON can blow past ARG_MAX (~128 KiB on Linux), so
    # write to temp files and pass paths instead of passing the JSON
    # itself as command-line arguments.
    local rust_f="$TMPDIR/cmp_${mode}_rust_$$.json"
    local self_f="$TMPDIR/cmp_${mode}_self_$$.json"
    printf '%s' "$rust_json" > "$rust_f"
    printf '%s' "$self_json" > "$self_f"

    local result
    if result=$(python3 scripts/parity.py "$mode" "$rust_f" "$self_f" "$rust_exit" "$self_exit" "$fixture_path" 2>&1); then
        if [[ "$result" == SKIP:* ]]; then
            skip "$label" "${result#SKIP: }"
        else
            pass "$label"
        fi
    else
        fail "$label" "$result"
    fi
    rm -f "$rust_f" "$self_f"
}

compare_json() {
    run_parity json "$@"
}

# A fixture may carry `// TEST: known-divergence <issue> "<why>"` to document a
# tracked Rust-vs-self-hosted runtime divergence (docs/equivalence/README.md).
# Such a fixture is committed deliberately: it pins a real miscompile so the
# suite regression-guards the eventual fix. It reports as a loud SKIP rather
# than a FAIL, and — mirroring verify_eval.py's GAP_FIXED — becomes a hard
# FAIL once the two compilers agree again, so the directive must be removed in
# the same PR that fixes the bug instead of silently going stale.
compare_runtime() {
    local label="$1" rust_bin="$2" self_bin="$3" stdin_file="${4:-}" vow_file="${5:-}"

    if [ ! -x "$rust_bin" ] || [ ! -x "$self_bin" ]; then
        skip "$label" "binary not found"
        return
    fi

    local known_div=""
    if [ -n "$vow_file" ]; then
        known_div=$(sed -n 's|^// TEST: known-divergence \([0-9]*\) "\(.*\)"$|#\1: \2|p' "$vow_file" | head -1)
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

    # The directive documents a stdout miscompile and nothing else. An exit-code
    # change — a crash, or a signal death such as 139 — is never what it covers,
    # so it must still FAIL on a fixture carrying the directive. Suppressing the
    # whole errors array would let a documented stdout gap hide a new crash.
    if [ -n "$known_div" ]; then
        local uncovered="" stdout_diverged=0 e
        for e in ${errors[@]+"${errors[@]}"}; do
            if [ "$e" = "stdout differs" ]; then
                stdout_diverged=1
            else
                uncovered="${uncovered:+$uncovered; }$e"
            fi
        done
        if [ -n "$uncovered" ]; then
            fail "$label" "known-divergence ($known_div) does not cover: $uncovered"
        elif [ "$stdout_diverged" -eq 0 ]; then
            fail "$label" "known-divergence ($known_div) no longer reproduces — remove the directive and update docs/equivalence/ledger.json"
        else
            skip "$label" "known divergence ($known_div)"
        fi
        return
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
    run_parity error "$@"
}

run_promoted_run_tests() {
    section_begin "Section 4: Run Tests"
    for vow_file in tests/run/*.vow; do
        name=$(basename "$vow_file" .vow)

        skip_reason=$(sed -n 's|^// TEST: skip "\(.*\)"$|\1|p' "$vow_file" | head -1)
        if [ -n "$skip_reason" ]; then
            skip "${name}/test-build" "$skip_reason"
            continue
        fi

        if grep -q '^// TEST: verify-only$' "$vow_file"; then
            rust_json="" self_json="" rust_exit=0 self_exit=0
            rust_json=$($RUST verify "$vow_file" 2>/dev/null) || rust_exit=$?
            self_json=$(run_self verify "$vow_file" 2>/dev/null) || self_exit=$?

            if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
                skip "${name}/test-verify" "empty output (rust=$rust_exit, self=$self_exit)"
            else
                compare_json "${name}/test-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"
                # Parity alone would pass a regression that makes BOTH compilers
                # reject the fixture, so pin the absolute expectation too (as
                # Section 4b does for tests/verify/).
                actual_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$rust_json" 2>/dev/null) || actual_status=""
                if [ "$actual_status" != "Verified" ]; then
                    fail "${name}/test-verify-expected-pass" "expected Verified, got ${actual_status:-<none>}"
                fi
            fi
            continue
        fi

        # Build with both compilers
        rust_json="" self_json="" rust_exit=0 self_exit=0
        rust_json=$($RUST build --no-verify "$vow_file" -o "$TMPDIR/test_rust_${name}" 2>/dev/null) || rust_exit=$?
        self_json=$(run_self build --no-verify "$vow_file" -o "$TMPDIR/test_self_${name}" 2>/dev/null) || self_exit=$?

        if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
            skip "${name}/test-build" "empty output (rust=$rust_exit, self=$self_exit)"
            continue
        fi

        compare_json "${name}/test-build" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"

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
        compare_runtime "${name}/test-run" "$TMPDIR/test_rust_${name}" "$TMPDIR/test_self_${name}" "$stdin_path" "$vow_file"

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
}

run_promoted_error_tests() {
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

    for fixture_path in \
        "$TMPDIR/parse_error.vow" \
        "$TMPDIR/type_error.vow" \
        "$TMPDIR/missing_module.vow" \
        "$TMPDIR/const_type_mismatch.vow" \
        tests/error/*.vow; do
        fixture=$(basename "$fixture_path" .vow)
        rust_json="" self_json="" rust_exit=0 self_exit=0
        rust_json=$($RUST build --no-verify "$fixture_path" -o "$TMPDIR/rust_${fixture}" 2>/dev/null) || rust_exit=$?
        self_json=$(run_self build --no-verify "$fixture_path" -o "$TMPDIR/self_${fixture}" 2>/dev/null) || self_exit=$?

        if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
            skip "${fixture}/error" "empty output (rust=$rust_exit, self=$self_exit)"
            continue
        fi

        compare_error "${fixture}/error" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$fixture_path"
    done
}

bootstrap_stage_failure() {
    local stage="$1"
    local status="$2"
    local stderr_log="$3"
    local stderr_tail="<no stderr>"

    if [ -s "$stderr_log" ]; then
        stderr_tail=$(tail -20 "$stderr_log")
    fi
    fail "bootstrap/triple-test" "$stage failed with exit code $status; stderr (last 20 lines): $stderr_tail"
}

run_bootstrap_triple() {
    local rust="$1"
    local concat="$2"
    local status=0
    local stderr_log="$TMPDIR/bootstrap_concat.stderr"

    "$concat" clif > "$TMPDIR/compiler_clif.vow" 2>"$stderr_log" || status=$?
    if [ "$status" -ne 0 ]; then
        bootstrap_stage_failure "concat" "$status" "$stderr_log"
        return 0
    fi

    # Stage 0: Rust compiler → Binary A
    status=0
    stderr_log="$TMPDIR/bootstrap_stage0.stderr"
    "$rust" --no-verify "$TMPDIR/compiler_clif.vow" -o "$TMPDIR/compiler_a" >/dev/null 2>"$stderr_log" || status=$?
    if [ "$status" -ne 0 ]; then
        bootstrap_stage_failure "Stage 0" "$status" "$stderr_log"
        return 0
    fi
    if [ ! -x "$TMPDIR/compiler_a" ]; then
        fail "bootstrap/triple-test" "Stage 0 exited with code 0 but did not produce executable compiler_a"
        return 0
    fi

    # Stage 1: A → B
    status=0
    stderr_log="$TMPDIR/bootstrap_stage1.stderr"
    run_self_bin "$TMPDIR/compiler_a" -o "$TMPDIR/compiler_b" "$TMPDIR/compiler_clif.vow" >/dev/null 2>"$stderr_log" || status=$?
    if [ "$status" -ne 0 ]; then
        bootstrap_stage_failure "Stage 1" "$status" "$stderr_log"
        return 0
    fi
    if [ ! -x "$TMPDIR/compiler_b" ]; then
        fail "bootstrap/triple-test" "Stage 1 exited with code 0 but did not produce executable compiler_b"
        return 0
    fi

    # Stage 2: B → C
    status=0
    stderr_log="$TMPDIR/bootstrap_stage2.stderr"
    run_self_bin "$TMPDIR/compiler_b" -o "$TMPDIR/compiler_c" "$TMPDIR/compiler_clif.vow" >/dev/null 2>"$stderr_log" || status=$?
    if [ "$status" -ne 0 ]; then
        bootstrap_stage_failure "Stage 2" "$status" "$stderr_log"
        return 0
    fi
    if [ ! -x "$TMPDIR/compiler_c" ]; then
        fail "bootstrap/triple-test" "Stage 2 exited with code 0 but did not produce executable compiler_c"
        return 0
    fi

    local hash_b=""
    local hash_c=""
    status=0
    stderr_log="$TMPDIR/bootstrap_hash_b.stderr"
    hash_b=$({ sha256sum "$TMPDIR/compiler_b" | awk '{print $1}'; } 2>"$stderr_log") || status=$?
    if [ "$status" -ne 0 ]; then
        bootstrap_stage_failure "Hash B" "$status" "$stderr_log"
        return 0
    fi
    if [ -z "$hash_b" ]; then
        fail "bootstrap/triple-test" "Hash B exited with code 0 but produced no digest"
        return 0
    fi

    status=0
    stderr_log="$TMPDIR/bootstrap_hash_c.stderr"
    hash_c=$({ sha256sum "$TMPDIR/compiler_c" | awk '{print $1}'; } 2>"$stderr_log") || status=$?
    if [ "$status" -ne 0 ]; then
        bootstrap_stage_failure "Hash C" "$status" "$stderr_log"
        return 0
    fi
    if [ -z "$hash_c" ]; then
        fail "bootstrap/triple-test" "Hash C exited with code 0 but produced no digest"
        return 0
    fi

    if [ "$hash_b" = "$hash_c" ]; then
        pass "bootstrap/triple-test"
    else
        fail "bootstrap/triple-test" "sha256 mismatch: B=$hash_b C=$hash_c"
    fi
}

print_summary() {
    section_finalize
    echo ""

    echo -e "${BOLD}=== Summary ===${RESET}"
    local script_end
    local total
    script_end=$(date +%s)
    total=$((script_end - SCRIPT_START))
    echo -e "  ${GREEN}${PASS} passed${RESET}, ${RED}${FAIL} failed${RESET}, ${YELLOW}${SKIP} skipped${RESET} in ${total}s"

    if [ ${#FAILURES[@]} -gt 0 ]; then
        echo ""
        echo -e "${RED}Failures:${RESET}"
        local failure
        for failure in "${FAILURES[@]}"; do
            echo "  - $failure"
        done
    fi

    return $(( FAIL > 0 ? 1 : 0 ))
}

# The boundary test recursively invokes this script with fake VOW_FULL_TEST_RUST/VOW_FULL_TEST_CONCAT tools to isolate bootstrap failure handling.
if [ "${VOW_FULL_TEST_BOOTSTRAP_ONLY:-0}" = "1" ]; then
    echo -e "${BOLD}=== Phase 20.1: Full Test Suite ===${RESET}"
    echo ""
    section_begin "Section 9: Bootstrap Triple Test"
    run_bootstrap_triple "${VOW_FULL_TEST_RUST:?}" "${VOW_FULL_TEST_CONCAT:?}"
    echo ""
    summary_status=0
    print_summary || summary_status=$?
    exit "$summary_status"
fi

echo -e "${BOLD}=== Phase 20.1: Full Test Suite ===${RESET}"
echo ""

section_begin "Section 0: Setup"
setup_compilers

if [ "${VOW_FULL_TEST_PROMOTED_ONLY:-0}" = "1" ]; then
    run_promoted_run_tests
    echo ""
    run_promoted_error_tests
    echo ""
    summary_status=0
    print_summary || summary_status=$?
    exit "$summary_status"
fi

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

    compare_json "${name}/build-no-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"

    # Save JSON for Section 3 (runtime execution)
    echo "$rust_json" > "$TMPDIR/rust_${name}.json"
    echo "$self_json" > "$TMPDIR/self_${name}.json"
done

missing_parent_fixture="examples/hello.vow"
rust_missing_parent_output="$TMPDIR/rust_missing_output_parent/out"
self_missing_parent_output="$TMPDIR/self_missing_output_parent/out"
rust_json="" self_json="" rust_exit=0 self_exit=0
rust_json=$($RUST build --no-verify "$missing_parent_fixture" -o "$rust_missing_parent_output" 2>/dev/null) || rust_exit=$?
self_json=$(run_self build --no-verify "$missing_parent_fixture" -o "$self_missing_parent_output" 2>/dev/null) || self_exit=$?

if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
    fail "build-no-verify/missing-output-parent" "empty output (rust=$rust_exit, self=$self_exit)"
else
    compare_json "build-no-verify/missing-output-parent" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$missing_parent_fixture"
fi

for name_output in "rust:$rust_missing_parent_output" "self:$self_missing_parent_output"; do
    compiler="${name_output%%:*}"
    output="${name_output#*:}"
    if [ -x "$output" ]; then
        pass "build-no-verify/${compiler}-creates-output-parent"
    else
        fail "build-no-verify/${compiler}-creates-output-parent" "missing executable: $output"
    fi
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

    compare_json "${name}/verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"
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

# The vowed u64 parameter is modeled with __VERIFIER_nondet_unsigned_long(), exercising its preamble declaration.
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

if VOWC_BIN="$SELF" bash tests/esbmc-path-cache/tests.sh >"$TMPDIR/esbmc-path-cache.log" 2>&1; then
    pass "verifier/esbmc-path-cache"
else
    fail "verifier/esbmc-path-cache" "$(tail -10 "$TMPDIR/esbmc-path-cache.log")"
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

run_promoted_run_tests
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

    compare_json "${name}/verify-test" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"
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

    compare_json "${name}/verify-fail-test" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"
    actual_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$rust_json" 2>/dev/null) || actual_status=""
    if [ -n "$actual_status" ] && [ "$actual_status" != "VerifyFailed" ]; then
        fail "${name}/verify-expected-fail" "expected VerifyFailed, got $actual_status"
    fi
done
echo ""

name="verify_jobs_counterexample_suppresses_later_soft_meta"
fixture="tests/verify-fail/verify_jobs_ce_before_soft.vow"
errors=()
for mode in verify build legacy; do
    self_json="" self_exit=0
    case "$mode" in
        verify)
            self_json=$(run_self verify --verify-jobs 2 "$fixture" 2>/dev/null) || self_exit=$?
            ;;
        build)
            self_json=$(run_self build --verify-jobs 2 "$fixture" -o "$TMPDIR/ce_before_soft" 2>/dev/null) || self_exit=$?
            ;;
        legacy)
            self_json=$(run_self --verify --verify-jobs 2 "$fixture" 2>/dev/null) || self_exit=$?
            ;;
    esac
    actual_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$self_json" 2>/dev/null) || actual_status=""
    verify_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('verify_status',''))" "$self_json" 2>/dev/null) || verify_status=""
    ce_function=$(python3 -c "import json,sys; d=json.loads(sys.argv[1]); c=d.get('counterexamples') or []; print(c[0].get('function','') if c else '')" "$self_json" 2>/dev/null) || ce_function=""
    if [ "$self_exit" -eq 0 ]; then
        errors+=("$mode exited 0")
    elif [ "$actual_status" != "VerifyFailed" ]; then
        errors+=("$mode status=$actual_status")
    elif [ "$ce_function" != "early_bad" ]; then
        errors+=("$mode counterexample function=$ce_function")
    elif [ -n "$verify_status" ]; then
        errors+=("$mode verify_status=$verify_status")
    fi
done
if [ ${#errors[@]} -eq 0 ]; then
    pass "$name"
else
    fail "$name" "$(IFS='; '; echo "${errors[*]}")"
fi
echo ""

name="verify_jobs_reports_first_hard_failure"
fixture="tests/verify-fail/verify_jobs_multi_hard_failure.vow"
errors=()
for mode in verify build; do
    rust_json="" self_json="" rust_exit=0 self_exit=0
    case "$mode" in
        verify)
            rust_json=$("$RUST" verify --no-cache --verify-jobs 3 "$fixture" 2>/dev/null) || rust_exit=$?
            self_json=$(run_self verify --no-cache --verify-jobs 3 "$fixture" 2>/dev/null) || self_exit=$?
            ;;
        build)
            rust_json=$("$RUST" build --no-cache --verify-jobs 3 "$fixture" -o "$TMPDIR/multi_hard_rust" 2>/dev/null) || rust_exit=$?
            self_json=$(run_self build --no-cache --verify-jobs 3 "$fixture" -o "$TMPDIR/multi_hard_self" 2>/dev/null) || self_exit=$?
            ;;
    esac

    compare_json "$name/$mode" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$fixture"
    for compiler in rust self; do
        if [ "$compiler" = "rust" ]; then
            result_json="$rust_json"
            result_exit="$rust_exit"
        else
            result_json="$self_json"
            result_exit="$self_exit"
        fi
        actual_status=$(python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$result_json" 2>/dev/null) || actual_status=""
        ce_count=$(python3 -c "import json,sys; print(len(json.loads(sys.argv[1]).get('counterexamples') or []))" "$result_json" 2>/dev/null) || ce_count=""
        ce_function=$(python3 -c "import json,sys; c=json.loads(sys.argv[1]).get('counterexamples') or []; print(c[0].get('function','') if c else '')" "$result_json" 2>/dev/null) || ce_function=""
        if [ "$result_exit" -eq 0 ]; then
            errors+=("$mode/$compiler exited 0")
        elif [ "$actual_status" != "VerifyFailed" ]; then
            errors+=("$mode/$compiler status=$actual_status")
        elif [ "$ce_count" != "1" ]; then
            errors+=("$mode/$compiler counterexamples=$ce_count")
        elif [ "$ce_function" != "first_bad" ]; then
            errors+=("$mode/$compiler counterexample function=$ce_function")
        fi
    done
done
if [ ${#errors[@]} -eq 0 ]; then
    pass "$name/policy"
else
    fail "$name/policy" "$(IFS='; '; echo "${errors[*]}")"
fi
echo ""

# ─── Checked-arithmetic abort model (#585) ────────────────────────
#
# Sections 4b/4c already hold Rust/self parity and the Verified/VerifyFailed
# verdicts for the two committed fixtures. What they cannot express is the
# property the issue is actually about: that `+!` and `+` are *distinguishable*,
# and that the reachable-abort warning appears exactly where the abort is
# reachable. Both compilers are checked, so a self-hosted regression cannot hide
# behind the Rust verdict.

section_begin "Checked-arithmetic abort model (#585)"

arith_dir="$TMPDIR/arith585"
mkdir -p "$arith_dir"

# Same contract, same body, one operator apart. Under the old model these two
# produced byte-identical counterexamples.
cat > "$arith_dir/wrapping.vow" <<'VOW'
module ArithWrapping
fn last_index(n: u64) -> u64 vow { ensures: result <= n } { n - 1 }
fn main() -> i32 [io] { print_i64(0); 0 }
VOW
cat > "$arith_dir/checked.vow" <<'VOW'
module ArithChecked
fn last_index(n: u64) -> u64 vow { ensures: result <= n } { n -! 1 }
fn main() -> i32 [io] { print_i64(0); 0 }
VOW

arith_status() {
    python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$1" 2>/dev/null || echo ""
}
# Count ArithOverflowReachable warnings naming a given function.
arith_warns() {
    python3 -c "
import json, sys
d = json.loads(sys.argv[1])
fn = sys.argv[2]
n = sum(1 for g in d.get('diagnostics', [])
        if g.get('error_code') == 'ArithOverflowReachable'
        and ('\`' + fn + '\`') in g.get('message', ''))
print(n)
" "$1" "$2" 2>/dev/null || echo "-1"
}

for compiler in rust self; do
    errors=()

    # 1. Wrapping still wraps: the counterexample is real and must be reported.
    if [ "$compiler" = rust ]; then
        j=$($RUST verify "$arith_dir/wrapping.vow" 2>/dev/null) || true
    else
        j=$(run_self verify "$arith_dir/wrapping.vow" 2>/dev/null) || true
    fi
    [ "$(arith_status "$j")" = "VerifyFailed" ] || errors+=("wrapping '-' should fail, got $(arith_status "$j")")

    # 2. Checked is distinguishable: the abort prunes the wrapped value, so the
    #    postcondition holds on every returning execution and the reachable
    #    abort is reported as a warning instead.
    if [ "$compiler" = rust ]; then
        j=$($RUST verify "$arith_dir/checked.vow" 2>/dev/null) || true
    else
        j=$(run_self verify "$arith_dir/checked.vow" 2>/dev/null) || true
    fi
    [ "$(arith_status "$j")" = "Verified" ] || errors+=("checked '-!' should verify, got $(arith_status "$j")")
    [ "$(arith_warns "$j" last_index)" = "1" ] || errors+=("checked '-!' should warn once, got $(arith_warns "$j" last_index)")

    # 3. The warning is precise, not blanket: in the committed fixture only the
    #    two functions whose abort is genuinely reachable are named, and a
    #    `requires` that rules the abort out silences it.
    if [ "$compiler" = rust ]; then
        j=$($RUST verify tests/verify/checked_arith_abort_modelled.vow 2>/dev/null) || true
    else
        j=$(run_self verify tests/verify/checked_arith_abort_modelled.vow 2>/dev/null) || true
    fi
    [ "$(arith_status "$j")" = "Verified" ] || errors+=("fixture should verify, got $(arith_status "$j")")
    for fn in twice scale doomed; do
        [ "$(arith_warns "$j" "$fn")" = "1" ] || errors+=("$fn should warn, got $(arith_warns "$j" "$fn")")
    done
    for fn in last_index halve rem_pos; do
        [ "$(arith_warns "$j" "$fn")" = "0" ] || errors+=("$fn abort is ruled out by requires; must not warn")
    done

    # 4. A site inside a co-emitted callee is attributed to the callee, not to
    #    the verify target that pulled it in, and the two targets that both
    #    reach it report it once.
    if [ "$compiler" = rust ]; then
        j=$($RUST verify tests/verify/checked_arith_callee_attribution.vow 2>/dev/null) || true
    else
        j=$(run_self verify tests/verify/checked_arith_callee_attribution.vow 2>/dev/null) || true
    fi
    [ "$(arith_status "$j")" = "Verified" ] || errors+=("attribution fixture status $(arith_status "$j")")
    [ "$(arith_warns "$j" helper)" = "1" ] || errors+=("site must be attributed to helper exactly once, got $(arith_warns "$j" helper)")
    [ "$(arith_warns "$j" caller)" = "0" ] || errors+=("caller has no checked arithmetic and must not be named")

    if [ ${#errors[@]} -eq 0 ]; then
        pass "checked_arith_abort_model/$compiler"
    else
        fail "checked_arith_abort_model/$compiler" "$(IFS='; '; echo "${errors[*]}")"
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

    compare_json "${name}/verify-skip-test" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"
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

# divide.vow: VowViolation at runtime — aborts with the reserved runtime-abort
# exit code 134, not a plain 1 (#877)
$RUST build --mode debug --no-verify examples/divide.vow -o "$TMPDIR/rust_divide_debug" >/dev/null 2>/dev/null
run_self build --mode debug --no-verify examples/divide.vow -o "$TMPDIR/self_divide_debug" >/dev/null 2>/dev/null

rust_exit=0 self_exit=0
"$TMPDIR/rust_divide_debug" </dev/null >"$TMPDIR/rust_dbg_out" 2>"$TMPDIR/rust_dbg_err" || rust_exit=$?
run_self_bin "$TMPDIR/self_divide_debug" </dev/null >"$TMPDIR/self_dbg_out" 2>"$TMPDIR/self_dbg_err" || self_exit=$?
rust_err=$(cat "$TMPDIR/rust_dbg_err")
self_err=$(cat "$TMPDIR/self_dbg_err")

errors=()
if [ "$rust_exit" -ne 134 ]; then errors+=("rust exit=$rust_exit, expected 134"); fi
if [ "$self_exit" -ne 134 ]; then errors+=("self exit=$self_exit, expected 134"); fi
for pattern in VowViolation Caller "y != 0"; do
    if ! echo "$rust_err" | grep -q "$pattern"; then errors+=("rust stderr missing '$pattern'"); fi
    if ! echo "$self_err" | grep -q "$pattern"; then errors+=("self stderr missing '$pattern'"); fi
done
if [ ${#errors[@]} -eq 0 ]; then
    pass "divide/debug-violation"
else
    fail "divide/debug-violation" "$(IFS='; '; echo "${errors[*]}")"
fi

# u8_requires_violation.vow: a captured u8 free variable must report its real
# value in the VowViolation payload, not 0 (numeric-tower u8 first-class fix).
$RUST build --mode debug --no-verify tests/debug/u8_requires_violation.vow -o "$TMPDIR/rust_u8_violation_debug" >/dev/null 2>/dev/null
run_self build --mode debug --no-verify tests/debug/u8_requires_violation.vow -o "$TMPDIR/self_u8_violation_debug" >/dev/null 2>/dev/null

rust_exit=0 self_exit=0
"$TMPDIR/rust_u8_violation_debug" </dev/null >"$TMPDIR/rust_u8_dbg_out" 2>"$TMPDIR/rust_u8_dbg_err" || rust_exit=$?
run_self_bin "$TMPDIR/self_u8_violation_debug" </dev/null >"$TMPDIR/self_u8_dbg_out" 2>"$TMPDIR/self_u8_dbg_err" || self_exit=$?
rust_err=$(cat "$TMPDIR/rust_u8_dbg_err")
self_err=$(cat "$TMPDIR/self_u8_dbg_err")

errors=()
if [ "$rust_exit" -ne 134 ]; then errors+=("rust exit=$rust_exit, expected 134"); fi
if [ "$self_exit" -ne 134 ]; then errors+=("self exit=$self_exit, expected 134"); fi
for pattern in VowViolation Caller '"x":5'; do
    if ! echo "$rust_err" | grep -qF "$pattern"; then errors+=("rust stderr missing '$pattern'"); fi
    if ! echo "$self_err" | grep -qF "$pattern"; then errors+=("self stderr missing '$pattern'"); fi
done
if [ ${#errors[@]} -eq 0 ]; then
    pass "u8_requires_violation/debug-violation"
else
    fail "u8_requires_violation/debug-violation" "$(IFS='; '; echo "${errors[*]}")"
fi

# i128_requires_violation.vow: both compilers must preserve both limbs of a
# captured i128 value in the runtime VowViolation payload (#1077).
$RUST build --mode debug --no-verify tests/debug/i128_requires_violation.vow -o "$TMPDIR/rust_i128_violation_debug" >/dev/null 2>/dev/null
run_self build --mode debug --no-verify tests/debug/i128_requires_violation.vow -o "$TMPDIR/self_i128_violation_debug" >/dev/null 2>/dev/null

rust_exit=0 self_exit=0
"$TMPDIR/rust_i128_violation_debug" </dev/null >"$TMPDIR/rust_i128_dbg_out" 2>"$TMPDIR/rust_i128_dbg_err" || rust_exit=$?
run_self_bin "$TMPDIR/self_i128_violation_debug" </dev/null >"$TMPDIR/self_i128_dbg_out" 2>"$TMPDIR/self_i128_dbg_err" || self_exit=$?
rust_err=$(cat "$TMPDIR/rust_i128_dbg_err")
self_err=$(cat "$TMPDIR/self_i128_dbg_err")

errors=()
if [ "$rust_exit" -ne 134 ]; then errors+=("rust exit=$rust_exit, expected 134"); fi
if [ "$self_exit" -ne 134 ]; then errors+=("self exit=$self_exit, expected 134"); fi
for pattern in VowViolation Caller '"x":3154393236604333326336'; do
    if ! echo "$rust_err" | grep -qF "$pattern"; then errors+=("rust stderr missing '$pattern'"); fi
    if ! echo "$self_err" | grep -qF "$pattern"; then errors+=("self stderr missing '$pattern'"); fi
done
if [ ${#errors[@]} -eq 0 ]; then
    pass "i128_requires_violation/debug-violation"
else
    fail "i128_requires_violation/debug-violation" "$(IFS='; '; echo "${errors[*]}")"
fi

# cast_in_contract_violation.vow: the contract text carried into the
# VowViolation payload must render the cast's real target type on both
# compilers, not a placeholder (#1113 Half B).
$RUST build --mode debug --no-verify tests/debug/cast_in_contract_violation.vow -o "$TMPDIR/rust_cast_violation_debug" >/dev/null 2>/dev/null
run_self build --mode debug --no-verify tests/debug/cast_in_contract_violation.vow -o "$TMPDIR/self_cast_violation_debug" >/dev/null 2>/dev/null

rust_exit=0 self_exit=0
"$TMPDIR/rust_cast_violation_debug" </dev/null >"$TMPDIR/rust_cast_dbg_out" 2>"$TMPDIR/rust_cast_dbg_err" || rust_exit=$?
run_self_bin "$TMPDIR/self_cast_violation_debug" </dev/null >"$TMPDIR/self_cast_dbg_out" 2>"$TMPDIR/self_cast_dbg_err" || self_exit=$?
rust_err=$(cat "$TMPDIR/rust_cast_dbg_err")
self_err=$(cat "$TMPDIR/self_cast_dbg_err")

errors=()
if [ "$rust_exit" -ne 134 ]; then errors+=("rust exit=$rust_exit, expected 134"); fi
if [ "$self_exit" -ne 134 ]; then errors+=("self exit=$self_exit, expected 134"); fi
for pattern in VowViolation Caller "as u64"; do
    if ! echo "$rust_err" | grep -qF "$pattern"; then errors+=("rust stderr missing '$pattern'"); fi
    if ! echo "$self_err" | grep -qF "$pattern"; then errors+=("self stderr missing '$pattern'"); fi
done
for pattern in "as <type>"; do
    if echo "$rust_err" | grep -qF "$pattern"; then errors+=("rust stderr has placeholder '$pattern'"); fi
    if echo "$self_err" | grep -qF "$pattern"; then errors+=("self stderr has placeholder '$pattern'"); fi
done
if [ ${#errors[@]} -eq 0 ]; then
    pass "cast_in_contract_violation/debug-violation"
else
    fail "cast_in_contract_violation/debug-violation" "$(IFS='; '; echo "${errors[*]}")"
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
    main_file="stdlib/${multi}/main.vow"
    printf "${BOLD}%s${RESET}\n" "$multi"

    # build --no-verify
    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST build --no-verify "$main_file" -o "$TMPDIR/rust_${multi}_main" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self build --no-verify "$main_file" -o "$TMPDIR/self_${multi}_main" 2>/dev/null) || self_exit=$?

    if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
        skip "${multi}/build-no-verify" "empty output (rust=$rust_exit, self=$self_exit)"
    else
        compare_json "${multi}/build-no-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$main_file"
    fi

    # verify
    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST verify "$main_file" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self verify "$main_file" 2>/dev/null) || self_exit=$?

    if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
        skip "${multi}/verify" "empty output (rust=$rust_exit, self=$self_exit)"
    else
        compare_json "${multi}/verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$main_file"
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
    compare_json "${name}/build" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$main_file"

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

    # Validate // TEST: exit directive (the vmod reject fixtures expect 134,
    # the reserved runtime-abort code: they reject via an index-out-of-bounds
    # trap, which now exits 134 rather than colliding with a plain 1 — see #877).
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

run_promoted_error_tests
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

if [[ "$rust_human" == *USAGE* ]] && [[ "$self_human" == *USAGE* ]]; then
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
echo ""

# ─── Section 8b: Tooling Scripts ──────────────────────────────────

section_begin "Section 8b: Tooling Scripts"

install_toolchain_log="$TMPDIR/install_toolchain_tests.log"
if bash tests/install_toolchain/tests.sh >"$install_toolchain_log" 2>&1; then
    pass "install-toolchain/smoke"
else
    fail "install-toolchain/smoke" "$(tail -20 "$install_toolchain_log")"
fi

bootstrap_log="$TMPDIR/bootstrap_tests.log"
if bash tests/bootstrap/tests.sh >"$bootstrap_log" 2>&1; then
    pass "bootstrap/smoke"
else
    fail "bootstrap/smoke" "$(tail -20 "$bootstrap_log")"
fi

release_log="$TMPDIR/release_tests.log"
if python3 tests/test_release.py >"$release_log" 2>&1; then
    pass "release/smoke"
else
    fail "release/smoke" "$(tail -20 "$release_log")"
fi

measure_bootstrap_rss_probe="$TMPDIR/measure_bootstrap_rss_time_probe.log"
if /usr/bin/time -v -o "$measure_bootstrap_rss_probe" true >/dev/null 2>&1 \
    && grep -q "Maximum resident set size" "$measure_bootstrap_rss_probe"; then
    measure_bootstrap_rss_log="$TMPDIR/measure_bootstrap_rss_tests.log"
    if bash tests/measure_bootstrap_rss/tests.sh >"$measure_bootstrap_rss_log" 2>&1; then
        pass "measure-bootstrap-rss/smoke"
    else
        fail "measure-bootstrap-rss/smoke" "$(tail -20 "$measure_bootstrap_rss_log")"
    fi
else
    skip "measure-bootstrap-rss/smoke" "requires GNU-compatible /usr/bin/time -v"
fi
echo ""

# ─── Section 8c: Contract Quality ─────────────────────────────────

section_begin "Section 8c: Contract Quality"

# contract-quality/weak-gate: ratchet on static contract quality across the
# self-hosted compiler — fail if the weak/tautological contract count exceeds the
# committed baseline (#81). Static classification only (no ESBMC), so it is cheap.
# Capture the contracts JSON in its own step so a producer failure (parse error,
# missing binary, compiler crash) is reported as itself — with its stderr visible —
# instead of being masked as a baseline breach by the checker's empty-stdin exit.
# One run per entry point: `vow contracts` follows `use` edges, so an entry point
# covers its own module graph and nothing else. `compiler/module_io.vow` is
# deliberately not in main.vow's `use` graph (.vmod parity infrastructure), so it
# had zero coverage until it was listed here. See the scope note in
# scripts/check_contract_quality.py for why the corpus dirs stay ungated.
for quality_entry in compiler/main.vow compiler/module_io.vow; do
    quality_case="contract-quality/weak-gate:$(basename "$quality_entry" .vow)"
    contract_quality_json="$TMPDIR/contract_quality_$(basename "$quality_entry" .vow).json"
    if ! run_self contracts "$quality_entry" >"$contract_quality_json"; then
        fail "$quality_case" "vow contracts $quality_entry failed (see stderr above); could not evaluate contract quality"
        continue
    fi
    # Distinguish the checker's exit codes: 0 = pass, 1 = baseline breach (a real
    # contract-quality regression), 2 = structural error (malformed JSON / missing
    # or non-integer counter — the checker's stderr above names the cause). A bare
    # else would mislabel a schema error as a baseline breach.
    quality_status=0
    uv run python scripts/check_contract_quality.py --label "$quality_entry" \
        <"$contract_quality_json" || quality_status=$?
    if [ "$quality_status" -eq 0 ]; then
        pass "$quality_case"
    elif [ "$quality_status" -eq 1 ]; then
        fail "$quality_case" "weak/tautological contracts exceeded baseline in $quality_entry; strengthen the new contract or adjust scripts/check_contract_quality.py with justification"
    else
        fail "$quality_case" "contract quality check could not run for $quality_entry (malformed 'vow contracts' output / schema mismatch; see stderr above)"
    fi
done

# contract-quality/parity: the ratchet above only ever runs $SELF, so
# vow/src/contract_quality.rs has no end-to-end coverage from it and the two
# classifiers can drift silently. Compare the (function, kind, quality) triples
# both compilers derive from one fixture.
#
# Scoped to those three fields on purpose — two pre-existing divergences in the
# published contracts schema would otherwise mask a real quality regression:
#   `description`   renders a cast as ` as <type>` in the self-hosted printer
#                   (compiler/lower.vow) but ` as i64` in the Rust one — #1113.
#   `source.offset` is always 0 in the self-hosted output — #1135.
# Widen this case to a full compare_json once both are fixed.
quality_fixture="tests/fixtures/contracts/quality_shapes.vow"
rust_quality_json="$TMPDIR/quality_parity_rust.json"
self_quality_json="$TMPDIR/quality_parity_self.json"
if ! $RUST contracts "$quality_fixture" >"$rust_quality_json" 2>/dev/null \
    || ! run_self contracts "$quality_fixture" >"$self_quality_json" 2>/dev/null; then
    fail "contract-quality/parity" "vow contracts failed on $quality_fixture (rust or self-hosted)"
else
    parity_result=$(python3 -c "
import json, sys

def triples(path):
    with open(path) as f:
        d = json.load(f)
    got = sorted(
        (c['function'], c['kind'], c['quality']) for c in d['contracts']
    )
    return got, d['summary']['quality']

r_triples, r_quality = triples(sys.argv[1])
s_triples, s_quality = triples(sys.argv[2])
errors = []
if r_triples != s_triples:
    only_rust = [t for t in r_triples if t not in s_triples]
    only_self = [t for t in s_triples if t not in r_triples]
    errors.append(f'clause quality differs: rust-only={only_rust} self-only={only_self}')
if r_quality != s_quality:
    errors.append(f'summary.quality differs: rust={r_quality} self={s_quality}')
# Pin the absolute expectation too: parity alone would pass a regression that
# makes BOTH compilers classify every clause 'substantive'.
expected = {'weak': 6, 'tautological': 2, 'substantive': 7}
if r_quality != expected:
    errors.append(f'rust summary.quality {r_quality} != expected {expected}')
print('; '.join(errors) if errors else 'OK')
" "$rust_quality_json" "$self_quality_json" 2>&1) || parity_result="checker error: $parity_result"
    if [ "$parity_result" = "OK" ]; then
        pass "contract-quality/parity"
    else
        fail "contract-quality/parity" "$parity_result"
    fi
fi
echo ""

# ─── Section 9: Bootstrap Triple Test ──────────────────────────────

section_begin "Section 9: Bootstrap Triple Test"

bootstrap_harness_log="$TMPDIR/full_test_bootstrap_tests.log"
if bash tests/full_test_bootstrap/tests.sh >"$bootstrap_harness_log" 2>&1; then
    pass "bootstrap/harness-tests"
else
    fail "bootstrap/harness-tests" "$(tail -20 "$bootstrap_harness_log")"
fi

run_bootstrap_triple "$RUST" "scripts/concat_vow.sh"
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

    compare_json "${name}/build-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"
done
echo ""

# ─── Section 10b: Test Subcommand ───────────────────────────────────

section_begin "Section 10b: Test Subcommand"

# Run vowc test with both compilers on compiler/ directory.
rust_test_exit=0 self_test_exit=0
rust_test_json=$($RUST test compiler/ 2>/dev/null) || rust_test_exit=$?
self_test_json=$(run_self test compiler/ 2>/dev/null) || self_test_exit=$?

if [ -z "$rust_test_json" ] || [ -z "$self_test_json" ]; then
    skip "test/subcommand" "empty output"
else
    run_parity test "test/parity" "$rust_test_json" "$self_test_json" "$rust_test_exit" "$self_test_exit"

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

    # test_complexity_io must not depend on the caller's working directory.
    repo_root=$(pwd -P)
    temp_cwd="$TMPDIR/test_complexity_io_cwd"
    mkdir -p "$temp_cwd"
    rust_abs="$repo_root/target/release/vow"
    rust_cwd_json=$(cd "$temp_cwd" && "$rust_abs" test "$repo_root/compiler" --filter test_complexity_io 2>/dev/null) || true
    self_cwd_json=$(cd "$temp_cwd" && run_self test "$repo_root/compiler" --filter test_complexity_io 2>/dev/null) || true
    rust_cwd_status=$(echo "$rust_cwd_json" | uv run python -c "import json,sys; print(json.load(sys.stdin)['status'])" 2>/dev/null) || rust_cwd_status=""
    self_cwd_status=$(echo "$self_cwd_json" | uv run python -c "import json,sys; print(json.load(sys.stdin)['status'])" 2>/dev/null) || self_cwd_status=""

    if [ "$rust_cwd_status" = "TestsPassed" ] && [ "$self_cwd_status" = "TestsPassed" ]; then
        pass "test/complexity-io-path-independent"
    else
        fail "test/complexity-io-path-independent" "rust=$rust_cwd_status self=$self_cwd_status"
    fi
fi
echo ""

# ─── Section 11: Arena Primitive ESBMC Verification ────────────────

section_begin "Section 11: Arena Primitive Verification"
default_solver_command=$(env -u SOLVER_FLAGS make -s -C vow-runtime/verify -n verify)
override_solver_command=$(make -s -C vow-runtime/verify -n \
    SOLVER_FLAGS="--z3 --incremental-bmc" verify)
if [[ "$default_solver_command" == *"--64 --boolector"* \
    && "$override_solver_command" == *"--64 --z3 --incremental-bmc"* \
    && "$override_solver_command" != *"--boolector"* \
    && "$override_solver_command" != *'"--z3 --incremental-bmc"'* ]]; then
    pass "arena/solver-flags"
else
    fail "arena/solver-flags" \
        "default=$default_solver_command; override=$override_solver_command"
fi

# The shared runner applies the same 2 GB virtual-memory cap as run_self, so
# this also guards against a regression in the verify invocation. With the
# single-shot --unwind 5 --boolector command (#516) the harness stays below
# the cap, but --incremental-bmc / Bitwuzla blew past it (#546).
if command -v esbmc >/dev/null 2>&1; then
    if scripts/verify_arena.sh >"$TMPDIR/arena_verify.log" 2>&1; then
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
    rust_exit=0
    self_exit=0
    rust_json=$("$RUST" complexity "$vow_file" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self complexity "$vow_file" 2>/dev/null) || self_exit=$?
    if [ "$rust_exit" != "0" ] || [ "$self_exit" != "0" ]; then
        fail "complexity/${name}" "rust_exit=${rust_exit} self_exit=${self_exit} (expected both 0)"
        continue
    fi
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
    elif [ "$name" = "predicate_control_flow" ]; then
        skip "complexity/${name}/ast-ir-cyclomatic-agree" "contract predicate control-flow lowers to IR branches outside the body AST count"
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
utf8dir=$(mktemp -d "$TMPDIR/utf8.XXXXXX")
cp tests/fixtures/complexity/params_basic.vow "$utf8dir/café.vow"
if diff -q <("$RUST" complexity "$utf8dir/café.vow" 2>/dev/null) <(run_self complexity "$utf8dir/café.vow" 2>/dev/null) >/dev/null; then
    pass "complexity/utf8-path-parity (café.vow byte-identical)"
else
    fail "complexity/utf8-path-parity" "non-ASCII path JSON diverges between compilers"
fi
rm -rf "$utf8dir"
# JSON named-control escapes in source paths must be byte-identical and must
# round-trip through a parser. Backspace/form-feed are control bytes that remain
# shell-log friendly but distinguish `\b`/`\f` from generic `?` escaping.
escape_dir=$(mktemp -d "$TMPDIR/escape.XXXXXX")
escape_name=$'quote"backslash\\backspace\bformfeed\f.vow'
escape_path="$escape_dir/$escape_name"
cp tests/fixtures/complexity/params_basic.vow "$escape_path"
rust_escape_json=$("$RUST" complexity "$escape_path" 2>/dev/null)
self_escape_json=$(run_self complexity "$escape_path" 2>/dev/null)
if [ "$rust_escape_json" = "$self_escape_json" ] && COMPLEXITY_EXPECT_PATH="$escape_path" python3 -c 'import json, os, sys; d=json.load(sys.stdin); sys.exit(0 if d["files"][0]["file"] == os.environ["COMPLEXITY_EXPECT_PATH"] else 1)' <<< "$rust_escape_json"; then
    pass "complexity/control-path-parity (quote/backslash/backspace/form-feed round-trip)"
else
    fail "complexity/control-path-parity" "escaped control-byte path did not stay byte-identical and round-trip"
fi
rm -rf "$escape_dir"
# Missing source file must produce a command-specific error in BOTH compilers and
# exit nonzero. Regression: the self-hosted compiler previously printed a generic
# `usage: vowc <command> ...` line for every subcommand (confusing `vow complexity` UX).
r_nosrc=0; "$RUST" complexity >/dev/null 2>"$TMPDIR/r_nosrc.err" || r_nosrc=$?
s_nosrc=0; run_self complexity >/dev/null 2>"$TMPDIR/s_nosrc.err" || s_nosrc=$?
if [ "$r_nosrc" != "0" ] && [ "$s_nosrc" != "0" ] \
   && grep -q "vow complexity: source file required" "$TMPDIR/r_nosrc.err" \
   && grep -q "vow complexity: source file required" "$TMPDIR/s_nosrc.err"; then
    pass "complexity/no-source-message (command-specific, both nonzero)"
else
    fail "complexity/no-source-message" "rust=$r_nosrc self=$s_nosrc (expected nonzero + 'vow complexity: source file required')"
fi
# The same per-command message must hold for the other three subcommands this PR
# fixed through the shared frontend_path_from_argv change, so a regression in any
# of them is caught — not just complexity.
for subcmd in build verify contracts; do
    r_sub=0; "$RUST" "$subcmd" >/dev/null 2>"$TMPDIR/r_${subcmd}_nosrc.err" || r_sub=$?
    s_sub=0; run_self "$subcmd" >/dev/null 2>"$TMPDIR/s_${subcmd}_nosrc.err" || s_sub=$?
    if [ "$r_sub" != "0" ] && [ "$s_sub" != "0" ] \
       && grep -q "vow ${subcmd}: source file required" "$TMPDIR/r_${subcmd}_nosrc.err" \
       && grep -q "vow ${subcmd}: source file required" "$TMPDIR/s_${subcmd}_nosrc.err"; then
        pass "${subcmd}/no-source-message (command-specific, both nonzero)"
    else
        fail "${subcmd}/no-source-message" "rust=$r_sub self=$s_sub (expected nonzero + 'vow ${subcmd}: source file required')"
    fi
done
# Legacy flag-only invocations also have no source path, but they bypass
# frontend_path_from_argv. Keep that fallback aligned with Rust instead of
# regressing to the old generic `usage: main ...` text.
r_legacy=0; "$RUST" --no-verify >/dev/null 2>"$TMPDIR/r_legacy_nosrc.err" || r_legacy=$?
s_legacy=0; run_self --no-verify >/dev/null 2>"$TMPDIR/s_legacy_nosrc.err" || s_legacy=$?
if [ "$r_legacy" != "0" ] && [ "$s_legacy" != "0" ] \
   && grep -q "vow: source file required (try --help or use a subcommand)" "$TMPDIR/r_legacy_nosrc.err" \
   && grep -q "vow: source file required (try --help or use a subcommand)" "$TMPDIR/s_legacy_nosrc.err" \
   && ! grep -q "usage: main" "$TMPDIR/r_legacy_nosrc.err" \
   && ! grep -q "usage: main" "$TMPDIR/s_legacy_nosrc.err"; then
    pass "legacy/no-source-message (fallback-specific, both nonzero)"
else
    fail "legacy/no-source-message" "rust=$r_legacy self=$s_legacy (expected nonzero + legacy source-required message, no 'usage: main')"
fi
# `--help --human` must document the complexity command in BOTH compilers. The JSON
# --help always listed it; the legacy human help previously omitted it.
if grep -q "vow complexity \[OPTIONS\]" <<< "$rust_human" \
   && grep -q "vow complexity \[OPTIONS\]" <<< "$self_human"; then
    pass "complexity/human-help-documented (both compilers)"
else
    fail "complexity/human-help-documented" "vow complexity missing from --help --human"
fi
echo ""

# ─── Section 6: Perfetto Trace (--perfetto, #784) ───────────────────
section_begin "Section 6: Perfetto Trace"
ptrace_dir=$(mktemp -d "$TMPDIR/ptrace.XXXXXX")
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

summary_status=0
print_summary || summary_status=$?
exit "$summary_status"
