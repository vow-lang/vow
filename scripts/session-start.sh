#!/bin/bash
# SessionStart hook: validate development environment for Claude Code web sessions.
# Reports environment status so Claude knows what tools are available.

set -uo pipefail

cd "$CLAUDE_PROJECT_DIR" 2>/dev/null || cd "$(dirname "$0")/.."

status=0

# ─── Check Rust toolchain ──────────────────────────────────────────
if command -v cargo &>/dev/null; then
    rust_version=$(rustc --version 2>/dev/null || echo "unknown")
    echo "rust: ok ($rust_version)"
else
    echo "rust: MISSING (cargo not found — run 'curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh')"
    status=1
fi

# ─── Check self-hosted compiler ─────────────────────────────────────
if [ -x "./vowc" ]; then
    echo "vowc: ok (self-hosted compiler present)"
else
    echo "vowc: MISSING (run 'scripts/bootstrap.sh --no-verify' to build)"
    status=1
fi

# ─── Check Rust stage-0 binary ──────────────────────────────────────
if [ -x "./target/release/vow" ]; then
    echo "stage0: ok (./target/release/vow present)"
else
    echo "stage0: MISSING (run 'cargo build --all --release' or 'scripts/bootstrap.sh')"
fi

# ─── Check ESBMC ───────────────────────────────────────────────────
if command -v esbmc &>/dev/null; then
    esbmc_version=$(esbmc --version 2>/dev/null | head -1 || echo "unknown")
    echo "esbmc: ok ($esbmc_version)"
else
    echo "esbmc: NOT FOUND (verification will be unavailable — use --no-verify flags)"
fi

# ─── Check Python / uv (for benchmarks and scripts) ────────────────
if command -v uv &>/dev/null; then
    echo "uv: ok"
else
    echo "uv: NOT FOUND (benchmark runner unavailable)"
fi

# ─── Check jq (needed by fmt-hook) ─────────────────────────────────
if command -v jq &>/dev/null; then
    echo "jq: ok"
else
    echo "jq: MISSING (fmt-hook.sh will not work — install jq)"
    status=1
fi

# ─── Reminder ──────────────────────────────────────────────────────
echo ""
echo "reminder: Always use 'ulimit -v 2000000' before running ./vowc or any Vow-compiled binary."

exit $status
