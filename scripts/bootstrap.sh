#!/usr/bin/env bash
set -euo pipefail

BOLD="\033[1m"
GREEN="\033[32m"
RED="\033[31m"
RESET="\033[0m"

cd "$(dirname "$0")/.."

NO_VERIFY=""
SKIP_CARGO=false

usage() {
    echo "Usage: $0 [--no-verify] [--skip-cargo] [--help|-h]"
    echo ""
    echo "Bootstrap the self-hosted Vow compiler and verify the fixed point."
    echo ""
    echo "Stages:"
    echo "  0: cargo build --all --release      -> ./target/release/vow"
    echo "  1: Rust compiler builds self-hosted  -> ./vowc"
    echo "  2: Self-hosted rebuilds itself       -> ./vowc2"
    echo "  3: Second self-hosted rebuild        -> ./vowc3"
    echo "  Verify: sha256(vowc2) == sha256(vowc3)"
    echo ""
    echo "Options:"
    echo "  --no-verify   Skip ESBMC verification in Stage 1-3"
    echo "  --skip-cargo  Skip Stage 0 if Rust binary already built"
    echo "  -h, --help    Show this help"
    exit 0
}

for arg in "$@"; do
    case "$arg" in
        --no-verify)  NO_VERIFY="--no-verify" ;;
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

# ─── Stage 1: Rust compiler -> ./vowc ────────────────────────────────

printf "${BOLD}Stage 1:${RESET} Rust compiler -> ./vowc\n"
t0=$(date +%s)
if ! output=$(./target/release/vow build $NO_VERIFY compiler/main.vow -o ./vowc 2>&1); then
    printf "  ${RED}FAILED${RESET}\n%s\n" "$output"
    exit 1
fi
t1=$(date +%s)
printf "  done in %ds\n" $((t1 - t0))

# ─── Stage 2: Self-hosted compiler -> ./vowc2 ────────────────────────

printf "${BOLD}Stage 2:${RESET} ./vowc -> ./vowc2\n"
t0=$(date +%s)
if ! output=$(ulimit -v 2000000; ./vowc build $NO_VERIFY compiler/main.vow -o ./vowc2 2>&1); then
    printf "  ${RED}FAILED${RESET}\n%s\n" "$output"
    exit 1
fi
t1=$(date +%s)
printf "  done in %ds\n" $((t1 - t0))

# ─── Stage 3: Second self-hosted rebuild -> ./vowc3 ──────────────────

printf "${BOLD}Stage 3:${RESET} ./vowc2 -> ./vowc3\n"
t0=$(date +%s)
if ! output=$(ulimit -v 2000000; ./vowc2 build $NO_VERIFY compiler/main.vow -o ./vowc3 2>&1); then
    printf "  ${RED}FAILED${RESET}\n%s\n" "$output"
    exit 1
fi
t1=$(date +%s)
printf "  done in %ds\n" $((t1 - t0))

# ─── Verify: SHA-256 fixed point ─────────────────────────────────────

printf "${BOLD}Verify:${RESET}  SHA-256 fixed point (vowc2 == vowc3)\n"
sha_vowc2=$(sha256sum ./vowc2 | awk '{print $1}')
sha_vowc3=$(sha256sum ./vowc3 | awk '{print $1}')

if [ "$sha_vowc2" = "$sha_vowc3" ]; then
    printf "  ${GREEN}MATCH${RESET}  %s\n" "$sha_vowc2"
    rm -f ./vowc3
    printf "\n${GREEN}${BOLD}Bootstrap successful.${RESET} ./vowc is the self-hosted compiler.\n"
else
    printf "  ${RED}MISMATCH${RESET}\n"
    printf "  vowc2: %s\n" "$sha_vowc2"
    printf "  vowc3: %s\n" "$sha_vowc3"
    exit 1
fi
