#!/usr/bin/env bash
set -euo pipefail

# Bump the Vow workspace version across all Cargo.toml files.
# Usage: bump-version.sh <rev|minor|major>
# Prints the new version to stdout on success.

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

# Read current version from the main crate using toml-compatible extraction
CURRENT=$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)".*/\1/p' vow/Cargo.toml | head -1)
if [ -z "$CURRENT" ]; then
    echo "Error: could not extract version from vow/Cargo.toml" >&2
    exit 1
fi

IFS='.' read -r MAJOR MINOR REV <<< "$CURRENT"

case "$BUMP" in
    major) MAJOR=$((MAJOR + 1)); MINOR=0; REV=0 ;;
    minor) MINOR=$((MINOR + 1)); REV=0 ;;
    rev)   REV=$((REV + 1)) ;;
esac

NEW_VERSION="${MAJOR}.${MINOR}.${REV}"

# Discover workspace member Cargo.toml files dynamically.
# Assumes multi-line members array (one per line), which cargo fmt enforces.
# Uses only bash builtins (no sed/awk) since GNU sed's `addr1,addr2{ cmds }`
# block syntax isn't portable to macOS's BSD sed, and `mapfile` isn't
# available on macOS's bash 3.2 either.
TOMLS=()
in_workspace=0
while IFS= read -r line; do
    if [[ "$line" =~ ^\[workspace\] ]]; then
        in_workspace=1
        continue
    fi
    if [ "$in_workspace" -eq 1 ] && [[ "$line" =~ ^\[ ]]; then
        break
    fi
    if [ "$in_workspace" -eq 1 ] && [[ "$line" =~ ^[[:space:]]*\"([^\"]+)\" ]]; then
        TOMLS+=("${BASH_REMATCH[1]}/Cargo.toml")
    fi
done < Cargo.toml
if [ ${#TOMLS[@]} -eq 0 ]; then
    echo "Error: no workspace members found in Cargo.toml" >&2
    exit 1
fi

ESCAPED_CURRENT="${CURRENT//./\\.}"
FAILED=0

for toml in "${TOMLS[@]}"; do
    if [ ! -f "$toml" ]; then
        echo "Warning: $toml not found, skipping" >&2
        continue
    fi

    # Portable sed: write to temp file then move (works on both GNU and BSD)
    tmp="${toml}.tmp"
    sed "s/^version = \"${ESCAPED_CURRENT}\"/version = \"${NEW_VERSION}\"/" "$toml" > "$tmp"
    mv "$tmp" "$toml"

    # Validate the version was actually updated
    UPDATED=$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)".*/\1/p' "$toml" | head -1)
    if [ "$UPDATED" != "$NEW_VERSION" ]; then
        echo "Error: $toml version is '$UPDATED', expected '$NEW_VERSION'" >&2
        FAILED=1
    fi
done

if [ "$FAILED" -ne 0 ]; then
    echo "Error: not all crate versions were updated" >&2
    exit 1
fi

echo "$NEW_VERSION"
