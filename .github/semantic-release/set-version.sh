#!/usr/bin/env bash
# Set the Vow workspace version explicitly across all member Cargo.toml files.
# Mirrors scripts/bump-version.sh's member discovery + portable sed, but takes an
# EXACT version (semantic-release computes the version; bump-version.sh only
# increments rev/minor/major and can't be told a specific version). Refreshes
# Cargo.lock so the release commit stays in sync. Invoked from the release
# workflow (build) and from @semantic-release/exec prepareCmd (publish).
set -euo pipefail
NEW_VERSION="${1:?usage: set-version.sh <X.Y.Z>}"
cd "$(git rev-parse --show-toplevel)"

CURRENT=$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)".*/\1/p' vow/Cargo.toml | head -1)
[ -n "$CURRENT" ] || { echo "Error: could not read current version from vow/Cargo.toml" >&2; exit 1; }

if [ "$CURRENT" = "$NEW_VERSION" ]; then
  echo "$NEW_VERSION"
  exit 0
fi

# Discover workspace members exactly as bump-version.sh does. Avoids `mapfile`
# (bash 4+ builtin) since macOS ships bash 3.2.
TOMLS=()
while IFS= read -r line; do
  TOMLS+=("$line")
done < <(sed -n '/^\[workspace\]/,/^\[/{ s/^[[:space:]]*"\(.*\)",\{0,1\}/\1\/Cargo.toml/p }' Cargo.toml)
[ ${#TOMLS[@]} -gt 0 ] || { echo "Error: no workspace members found in Cargo.toml" >&2; exit 1; }

ESCAPED_CURRENT="${CURRENT//./\\.}"
for toml in "${TOMLS[@]}"; do
  [ -f "$toml" ] || { echo "Warning: $toml not found, skipping" >&2; continue; }
  tmp="${toml}.tmp"
  sed "s/^version = \"${ESCAPED_CURRENT}\"/version = \"${NEW_VERSION}\"/" "$toml" > "$tmp"
  mv "$tmp" "$toml"
  updated=$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)".*/\1/p' "$toml" | head -1)
  [ "$updated" = "$NEW_VERSION" ] || { echo "Error: $toml is '$updated', expected '$NEW_VERSION'" >&2; exit 1; }
done

# Keep Cargo.lock in step with the bumped workspace versions.
cargo metadata --format-version 1 > /dev/null

echo "$NEW_VERSION"
