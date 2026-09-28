#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd -P)"

# Unlimited unless VOW_ULIMIT_KB (KB) is set.
if [ -n "${VOW_ULIMIT_KB:-}" ]; then ulimit -v "$VOW_ULIMIT_KB"; fi
exec make -C "$REPO_ROOT/vow-runtime/verify" verify
