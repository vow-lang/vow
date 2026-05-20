#!/usr/bin/env bash
set -euo pipefail

# Trigger the Release workflow on GitHub Actions.
# Usage: release.sh <rev|minor|major>

cd "$(dirname "$0")/.."

usage() {
    echo "Usage: $0 <rev|minor|major>" >&2
    exit 1
}

[ $# -eq 1 ] || usage

BUMP="$1"
case "$BUMP" in
    rev|minor|major) ;;
    *) echo "Error: bump type must be rev, minor, or major (got '$BUMP')" >&2; usage ;;
esac

command -v gh >/dev/null || { echo "Error: gh CLI not found. Install from https://cli.github.com/" >&2; exit 1; }
command -v git >/dev/null || { echo "Error: git not found." >&2; exit 1; }

# The workflow runs against main, so derive the preview from origin/main
# (not the local working tree, which may be ahead/behind).
git fetch --quiet origin main || echo "Warning: could not fetch origin/main; using cached ref" >&2
CURRENT=$(git show origin/main:vow/Cargo.toml 2>/dev/null | sed -n 's/^version[[:space:]]*=[[:space:]]*"\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)".*/\1/p' | head -1)
if [ -z "$CURRENT" ]; then
    echo "Error: could not extract version from origin/main:vow/Cargo.toml" >&2
    exit 1
fi
IFS='.' read -r MAJOR MINOR REV <<< "$CURRENT"
case "$BUMP" in
    major) NEXT="$((MAJOR + 1)).0.0" ;;
    minor) NEXT="${MAJOR}.$((MINOR + 1)).0" ;;
    rev)   NEXT="${MAJOR}.${MINOR}.$((REV + 1))" ;;
esac

echo "Current version: $CURRENT  (from origin/main)"
echo "Next version:    $NEXT  ($BUMP bump)"
echo "Triggers: .github/workflows/release.yml on main"
echo
read -r -p "Proceed? [y/N] " reply
case "$reply" in
    y|Y|yes|YES) ;;
    *) echo "Aborted." >&2; exit 1 ;;
esac

gh workflow run release.yml --ref main -f bump="$BUMP"

echo
echo "Workflow dispatched. Watch with:"
echo "  gh run list --workflow=release.yml --limit 1"
echo "  gh run watch"
