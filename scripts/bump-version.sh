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

# Read current version from the main crate
CURRENT=$(grep '^version' vow/Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
IFS='.' read -r MAJOR MINOR REV <<< "$CURRENT"

case "$BUMP" in
    major) MAJOR=$((MAJOR + 1)); MINOR=0; REV=0 ;;
    minor) MINOR=$((MINOR + 1)); REV=0 ;;
    rev)   REV=$((REV + 1)) ;;
esac

NEW_VERSION="${MAJOR}.${MINOR}.${REV}"

# Update all workspace Cargo.toml files
for toml in \
    vow/Cargo.toml \
    vow-syntax/Cargo.toml \
    vow-types/Cargo.toml \
    vow-ir/Cargo.toml \
    vow-codegen/Cargo.toml \
    vow-verify/Cargo.toml \
    vow-diag/Cargo.toml \
    vow-runtime/Cargo.toml \
    vow-clif-shim/Cargo.toml
do
    sed -i "s/^version = \"$CURRENT\"/version = \"$NEW_VERSION\"/" "$toml"
done

echo "$NEW_VERSION"
