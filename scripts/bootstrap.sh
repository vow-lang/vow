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
NO_VERIFY=false
VMEM_LIMIT_KB="${VOW_BOOTSTRAP_VMEM_KB:-0}"
if ! [[ "$VMEM_LIMIT_KB" =~ ^[0-9]+$ ]]; then
    echo "Error: VOW_BOOTSTRAP_VMEM_KB must be a non-negative integer (got: '$VMEM_LIMIT_KB')" >&2
    exit 1
fi

usage() {
    echo "Usage: $0 [--skip-cargo] [--no-verify] [--stage3-no-verify] [--help|-h]"
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
    echo "  --no-verify          Skip ESBMC verification at every stage. Useful on"
    echo "                       platforms where ESBMC is unavailable (e.g. macOS)."
    echo "                       Verification does not change codegen, so the"
    echo "                       SHA-256 fixed-point check remains meaningful."
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
        # `head -1` is safe: in the compact JSON, the top-level "status"
        # is emitted first; per-function VerificationSkipped diagnostics use
        # "error_code", not "status", so they cannot displace the top match.
        local status
        status=$(grep -aoE '"status":"[A-Za-z]+"' "$log" | head -1 \
                 | sed -E 's/.*"status":"(.*)"/\1/')
        if [ "$status" = "Skipped" ]; then
            printf "  note: overall Skipped — verifier cannot model these vowed functions (#397):\n" >&2
            # `[^}]*` between "error_code" and "message" assumes the
            # intervening diagnostic fields (severity, span, etc.) contain
            # no `}` characters. Current schema matches; revisit if a
            # nested object is ever added between those two fields.
            #
            # `|| printf …` is load-bearing: with `set -euo pipefail` (top
            # of script), grep exits 1 on no-match, the whole pipeline
            # exits 1, and the subshell would otherwise abort before the
            # `exit 0` below — silently turning a tolerated Skipped into a
            # bootstrap failure. The fallback also gives the user a hint to
            # consult the raw log on extractor breakage.
            grep -aoE '"VerificationSkipped"[^}]*"message":"[^"]+"' "$log" \
              | sed -E 's/.*"message":"([^"]+)".*/    - \1/' \
              | sort -u >&2 \
              || printf "    (could not extract function names — see full log above)\n" >&2
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

sha256_file() {
    local path="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$path" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$path" | awk '{print $1}'
    else
        echo "Error: neither sha256sum nor shasum is available" >&2
        exit 1
    fi
}

for arg in "$@"; do
    case "$arg" in
        --skip-cargo)        SKIP_CARGO=true ;;
        --no-verify)         NO_VERIFY=true ;;
        --stage3-no-verify)  STAGE3_NO_VERIFY=true ;;
        -h|--help)           usage ;;
        *)                   echo "Unknown flag: $arg"; usage ;;
    esac
done

stage12_build_flags="--verify-jobs 1"
stage3_build_flags="--verify-jobs 1"
if [ "$NO_VERIFY" = true ]; then
    stage12_build_flags="--no-verify"
    stage3_build_flags="--no-verify"
elif [ "$STAGE3_NO_VERIFY" = true ]; then
    stage3_build_flags="--no-verify"
fi

# Stage 1 runs the Rust compiler (well-behaved release binary) — no vmem cap.
# Stages 2+3 run the self-hosted compiler under VOW_BOOTSTRAP_VMEM_KB if set.
# With --no-verify ESBMC never runs, so the "Skipped" handling in
# run_verify_* is dead code but harmless. The Stage 1/2/3 call sites
# always invoke run_rust_stage / run_self_stage; these wrappers do the
# conditional dispatch (verify-aware vs plain logger), so the stage
# invocation code itself is identical in both modes — only the wrappers
# differ.
run_rust_stage() {
    local cmd="$1"
    if [ "$NO_VERIFY" = true ]; then
        run_logged "$cmd"
    else
        run_verify_logged "$cmd"
    fi
}

run_self_stage() {
    local cmd="$1"
    if [ "$NO_VERIFY" = true ]; then
        run_stage_cmd "$cmd"
    else
        run_verify_stage_cmd "$cmd"
    fi
}

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
if ! run_rust_stage "./target/release/vow build $stage12_build_flags compiler/main.vow -o build/vowc"; then
    printf "  ${RED}FAILED${RESET}\n"
    exit 1
fi
t1=$(date +%s)
printf "  done in %ds\n" $((t1 - t0))

# ─── Stage 2: Self-hosted compiler -> build/vowc2 ────────────────────

printf "${BOLD}Stage 2:${RESET} build/vowc -> build/vowc2\n"
t0=$(date +%s)
if ! run_self_stage "build/vowc build $stage12_build_flags compiler/main.vow -o build/vowc2"; then
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
if ! run_self_stage "build/vowc2 build $stage3_build_flags compiler/main.vow -o build/vowc3"; then
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
sha_vowc2=$(sha256_file build/vowc2)
sha_vowc3=$(sha256_file build/vowc3)

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
