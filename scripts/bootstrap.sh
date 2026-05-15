#!/usr/bin/env bash
set -euo pipefail

BOLD="\033[1m"
GREEN="\033[32m"
RED="\033[31m"
RESET="\033[0m"

cd "$(dirname "$0")/.."
mkdir -p build

SKIP_CARGO=false
STAGE3_NO_VERIFY=false
VMEM_LIMIT_KB="${VOW_BOOTSTRAP_VMEM_KB:-0}"
if ! [[ "$VMEM_LIMIT_KB" =~ ^[0-9]+$ ]]; then
    echo "Error: VOW_BOOTSTRAP_VMEM_KB must be a non-negative integer (got: '$VMEM_LIMIT_KB')" >&2
    exit 1
fi

usage() {
    echo "Usage: $0 [--skip-cargo] [--stage3-no-verify] [--help|-h]"
    echo ""
    echo "Bootstrap the self-hosted Vow compiler and verify the fixed point."
    echo ""
    echo "Stages:"
    echo "  0: cargo build --all --release      -> ./target/release/vow"
    echo "  1: Rust compiler builds self-hosted  -> build/vowc"
    echo "  2: Self-hosted rebuilds itself       -> build/vowc2"
    echo "  3: Second self-hosted rebuild        -> build/vowc3"
    echo "  Verify: sha256(vowc2) == sha256(vowc3)"
    echo ""
    echo "Options:"
    echo "  --skip-cargo         Skip Stage 0 if Rust binary already built"
    echo "  --stage3-no-verify   Skip ESBMC verification on Stage 3 only (Stages 1-2"
    echo "                       still verify). Verification does not change codegen,"
    echo "                       so the SHA-256 fixed-point check remains meaningful."
    echo "  -h, --help           Show this help"
    echo ""
    echo "Environment:"
    echo "  VOW_BOOTSTRAP_VMEM_KB  Optional per-stage virtual-memory cap in KB for"
    echo "                         self-hosted rebuilds (default: unlimited)"
    exit 0
}

run_logged() {
    local cmd="$1"
    (
        log=$(mktemp)
        trap 'rm -f "$log"' EXIT INT TERM HUP
        if bash -c "$cmd" >"$log" 2>&1; then
            exit 0
        fi
        cat "$log"
        exit 1
    )
}

# Like run_logged but accepts `vow build`'s `Skipped` overall status as success.
# Skipped happens when a vowed function's body uses an opcode the verifier
# cannot model (currently RegionAlloc, see #397). ESBMC still runs over every
# modelable function; only the unverifiable-by-design ones are skipped. The
# script surfaces a one-line note per skipped function so the warning is not
# silently buried.
run_verify_logged() {
    local cmd="$1"
    (
        log=$(mktemp)
        trap 'rm -f "$log"' EXIT INT TERM HUP
        if bash -c "$cmd" >"$log" 2>&1; then
            exit 0
        fi
        local status
        status=$(grep -aoE '"status":"[A-Za-z]+"' "$log" | head -1 \
                 | sed -E 's/.*"status":"(.*)"/\1/')
        if [ "$status" = "Skipped" ]; then
            printf "  note: overall Skipped — verifier cannot model these vowed functions (#397):\n" >&2
            grep -aoE '"VerificationSkipped"[^}]*"message":"[^"]+"' "$log" \
              | sed -E 's/.*"message":"([^"]+)".*/    - \1/' \
              | sort -u >&2
            exit 0
        fi
        cat "$log"
        exit 1
    )
}

run_stage_cmd() {
    local cmd="$1"
    if [ "$VMEM_LIMIT_KB" -gt 0 ]; then
        cmd="ulimit -v $VMEM_LIMIT_KB && $cmd"
    fi
    run_logged "$cmd"
}

run_verify_stage_cmd() {
    local cmd="$1"
    if [ "$VMEM_LIMIT_KB" -gt 0 ]; then
        cmd="ulimit -v $VMEM_LIMIT_KB && $cmd"
    fi
    run_verify_logged "$cmd"
}

for arg in "$@"; do
    case "$arg" in
        --skip-cargo)        SKIP_CARGO=true ;;
        --stage3-no-verify)  STAGE3_NO_VERIFY=true ;;
        -h|--help)           usage ;;
        *)                   echo "Unknown flag: $arg"; usage ;;
    esac
done

stage3_build_flags="--verify-jobs 1"
if [ "$STAGE3_NO_VERIFY" = true ]; then
    stage3_build_flags="--no-verify"
fi

# ─── Stage 0: Build Rust compiler ────────────────────────────────────

if [ "$SKIP_CARGO" = true ]; then
    printf "${BOLD}Stage 0:${RESET} skipped (--skip-cargo)\n"
else
    printf "${BOLD}Stage 0:${RESET} cargo build --all --release\n"
    t0=$(date +%s)
    cargo build --all --release
    t1=$(date +%s)
    printf "  done in %ds\n" $((t1 - t0))
fi

# ─── Stage 1: Rust compiler -> build/vowc ────────────────────────────

# --verify-jobs 1 keeps bootstrap memory bounded: at most one ESBMC process
# runs at a time across the ~45 vowed compiler functions, so peak RSS is
# codegen + one ESBMC instead of codegen + N-fold ESBMC fan-out. See #175.
printf "${BOLD}Stage 1:${RESET} Rust compiler -> build/vowc\n"
t0=$(date +%s)
if ! run_verify_logged "./target/release/vow build --verify-jobs 1 compiler/main.vow -o build/vowc"; then
    printf "  ${RED}FAILED${RESET}\n"
    exit 1
fi
t1=$(date +%s)
printf "  done in %ds\n" $((t1 - t0))

# ─── Stage 2: Self-hosted compiler -> build/vowc2 ────────────────────

printf "${BOLD}Stage 2:${RESET} build/vowc -> build/vowc2\n"
t0=$(date +%s)
if ! run_verify_stage_cmd "build/vowc build --verify-jobs 1 compiler/main.vow -o build/vowc2"; then
    printf "  ${RED}FAILED${RESET}\n"
    if [ "$VMEM_LIMIT_KB" -gt 0 ]; then
        printf "  Hint: rerun with a higher VOW_BOOTSTRAP_VMEM_KB or unset it for no cap.\n"
    fi
    exit 1
fi
t1=$(date +%s)
printf "  done in %ds\n" $((t1 - t0))

# ─── Stage 3: Second self-hosted rebuild -> build/vowc3 ──────────────

printf "${BOLD}Stage 3:${RESET} build/vowc2 -> build/vowc3\n"
t0=$(date +%s)
if ! run_verify_stage_cmd "build/vowc2 build $stage3_build_flags compiler/main.vow -o build/vowc3"; then
    printf "  ${RED}FAILED${RESET}\n"
    if [ "$VMEM_LIMIT_KB" -gt 0 ]; then
        printf "  Hint: rerun with a higher VOW_BOOTSTRAP_VMEM_KB or unset it for no cap.\n"
    fi
    exit 1
fi
t1=$(date +%s)
printf "  done in %ds\n" $((t1 - t0))

# ─── Verify: SHA-256 fixed point ─────────────────────────────────────

printf "${BOLD}Verify:${RESET}  SHA-256 fixed point (vowc2 == vowc3)\n"
sha_vowc2=$(sha256sum build/vowc2 | awk '{print $1}')
sha_vowc3=$(sha256sum build/vowc3 | awk '{print $1}')

if [ "$sha_vowc2" = "$sha_vowc3" ]; then
    printf "  ${GREEN}MATCH${RESET}  %s\n" "$sha_vowc2"
    mv build/vowc2 build/vowc
    rm -f build/vowc3
    printf "\n${GREEN}${BOLD}Bootstrap successful.${RESET} build/vowc is the self-hosted compiler.\n"
else
    printf "  ${RED}MISMATCH${RESET}\n"
    printf "  vowc2: %s\n" "$sha_vowc2"
    printf "  vowc3: %s\n" "$sha_vowc3"
    exit 1
fi

