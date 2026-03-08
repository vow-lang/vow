#!/usr/bin/env bash
set -euo pipefail

# Run Semgrep static analysis on the codebase.
# Usage: ./scripts/semgrep-scan.sh [--ci]
#   --ci    Output SARIF for CI integration (default: text output)

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

CI_MODE=false
if [[ "${1:-}" == "--ci" ]]; then
    CI_MODE=true
fi

if ! command -v semgrep &>/dev/null; then
    echo "semgrep not found. Install with: pip install semgrep" >&2
    exit 1
fi

SEMGREP_ARGS=(
    --config auto
    --error
    --metrics off
)

if [[ "$CI_MODE" == true ]]; then
    SEMGREP_ARGS+=(--sarif --output semgrep-results.sarif)
else
    SEMGREP_ARGS+=(--text)
fi

echo "Running Semgrep scan..."
semgrep "${SEMGREP_ARGS[@]}" .
