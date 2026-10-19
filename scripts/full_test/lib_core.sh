# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

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

# Optional per-process virtual-memory cap (KB) for self-hosted runs. Unset or
# empty means unlimited. The limit is inherited by ESBMC children and is per
# process, not aggregate; a host-level cap (e.g. a cgroup) bounds the total.
apply_vmem_limit() {
    if [ -n "${VOW_ULIMIT_KB:-}" ]; then ulimit -v "$VOW_ULIMIT_KB"; fi
}

run_self() {
    (apply_vmem_limit; "$SELF" "$@")
}

run_self_bin() {
    local bin="$1"; shift
    (apply_vmem_limit; "$bin" "$@")
}

setup_compilers() {
    if [ "${VOW_FULL_TEST_SKIP_CARGO:-0}" = "1" ]; then
        if [ ! -x "$RUST" ]; then
            echo "VOW_FULL_TEST_SKIP_CARGO=1 but $RUST is not executable; build it first (cargo build --all --release) or unset VOW_FULL_TEST_SKIP_CARGO." >&2
            exit 1
        fi
        echo -e "${BOLD}Skipping Rust compiler rebuild (VOW_FULL_TEST_SKIP_CARGO=1)${RESET}"
    else
        echo -e "${BOLD}Building Rust compiler...${RESET}"
        cargo build --all --release 2>&1 | tail -1
    fi
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

# Run a check script that prints `PASS|FAIL|SKIP <name>[ — <reason>]` lines (the same scripts
# pull requests run) and record each line as one result. A script that crashes, exits above 1,
# exits 1 without a FAIL line, or prints no verdict at all is itself a failure, so it cannot pass
# vacuously. VOW_ULIMIT_KB is passed on explicitly so an unexported value still caps its runs.
run_shared_check() {
    local label="$1" line rest name reason rc=-1 seen=0 saw_fail=0
    shift
    while IFS= read -r line; do
        case "$line" in
            "@@rc "*) rc="${line#@@rc }"; continue ;;
            PASS\ *|FAIL\ *|SKIP\ *) ;;
            *) continue ;;
        esac
        seen=$((seen + 1))
        rest="${line#* }"
        name="${rest%% — *}"
        reason=""
        if [ "$name" != "$rest" ]; then reason="${rest#* — }"; fi
        case "$line" in
            PASS\ *) pass "$name" ;;
            FAIL\ *) fail "$name" "$reason"; saw_fail=1 ;;
            SKIP\ *) skip "$name" "$reason" ;;
        esac
    done < <(VOW_ULIMIT_KB="${VOW_ULIMIT_KB:-}" "$@" && printf '\n@@rc 0\n' || printf '\n@@rc %s\n' "$?")
    if [ "$rc" -gt 1 ] || [ "$rc" -lt 0 ] || [ "$seen" -eq 0 ] || { [ "$rc" -eq 1 ] && [ "$saw_fail" -eq 0 ]; }; then
        fail "$label" "$1 exited with status $rc, $seen verdict lines, and no FAIL verdict"
    fi
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

# Whole-document comparison for `vow verify` output: every diagnostic and every
# counterexample field must match, modulo the documented allow-list in
# scripts/parity.py (FULL_JSON_ALLOWLIST).
compare_full_json() {
    run_parity full-json "$@"
}

# Structured runtime aborts (one JSON object per stderr line) of the same
# program built by each compiler. Takes the two stderr files, not JSON text.
compare_runtime_json() {
    local label="$1" rust_err="$2" self_err="$3" rust_exit="$4" self_exit="$5"
    local result
    if result=$(python3 scripts/parity.py runtime-json "$rust_err" "$self_err" "$rust_exit" "$self_exit" 2>&1); then
        pass "$label"
    else
        fail "$label" "$result"
    fi
}

# A fixture may carry `// TEST: known-divergence <issue> "<why>"` to document a
# tracked Rust-vs-self-hosted runtime divergence (docs/equivalence/README.md).
# Such a fixture is committed deliberately: it pins a real miscompile so the
# suite regression-guards the eventual fix. It reports as a loud SKIP rather
# than a FAIL, and — mirroring verify_eval.py's GAP_FIXED — becomes a hard
# FAIL once the two compilers agree again, so the directive must be removed in
# the same PR that fixes the bug instead of silently going stale.
