#!/usr/bin/env bash
set -euo pipefail

BOLD="\033[1m"
GREEN="\033[32m"
RED="\033[31m"
RESET="\033[0m"

cd "$(dirname "$0")/.."
mkdir -p build

SKIP_CARGO=false
VMEM_LIMIT_KB="${VOW_BOOTSTRAP_VMEM_KB:-0}"

usage() {
    echo "Usage: $0 [--skip-cargo] [--help|-h]"
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
    echo "  --skip-cargo  Skip Stage 0 if Rust binary already built"
    echo "  -h, --help    Show this help"
    echo ""
    echo "Environment:"
    echo "  VOW_BOOTSTRAP_VMEM_KB  Optional per-stage virtual-memory cap in KB for"
    echo "                         self-hosted rebuilds (default: unlimited)"
    exit 0
}

run_logged() {
    local cmd="$1"
    local log
    log=$(mktemp)
    if bash -c "$cmd" >"$log" 2>&1; then
        rm -f "$log"
        return 0
    fi
    cat "$log"
    rm -f "$log"
    return 1
}

run_stage_cmd() {
    local cmd="$1"
    if [ "$VMEM_LIMIT_KB" -gt 0 ]; then
        cmd="ulimit -v $VMEM_LIMIT_KB; $cmd"
    fi
    run_logged "$cmd"
}

for arg in "$@"; do
    case "$arg" in
        --skip-cargo) SKIP_CARGO=true ;;
        -h|--help)    usage ;;
        *)            echo "Unknown flag: $arg"; usage ;;
    esac
done

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

printf "${BOLD}Stage 1:${RESET} Rust compiler -> build/vowc\n"
t0=$(date +%s)
if ! run_logged "./target/release/vow build compiler/main.vow -o build/vowc"; then
    printf "  ${RED}FAILED${RESET}\n"
    exit 1
fi
t1=$(date +%s)
printf "  done in %ds\n" $((t1 - t0))

# ─── Stage 2: Self-hosted compiler -> build/vowc2 ────────────────────

printf "${BOLD}Stage 2:${RESET} build/vowc -> build/vowc2\n"
t0=$(date +%s)
if ! run_stage_cmd "build/vowc build compiler/main.vow -o build/vowc2"; then
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
if ! run_stage_cmd "build/vowc2 build compiler/main.vow -o build/vowc3"; then
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
