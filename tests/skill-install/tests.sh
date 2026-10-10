#!/usr/bin/env bash
set -euo pipefail

# Migration contract for existing Vow skill installs (issue #358). Both
# compilers must leave an existing .claude/skills/vow/SKILL.md untouched on
# build (auto-install), and rewrite it plus the split-layout support files on
# an explicit `skill install --local`.
#
#   VOWC_BIN=build/vowc            bash tests/skill-install/tests.sh
#   VOWC_BIN=target/release/vow bash tests/skill-install/tests.sh

VOWC_BIN="${VOWC_BIN:-build/vowc}"
VOWC_BIN=$(cd "$(dirname "$VOWC_BIN")" && pwd)/$(basename "$VOWC_BIN")
FIXTURE=$(cd "$(dirname "$0")/../.." && pwd)/examples/hello.vow
TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }
expect() {
    if [ "$2" != "$3" ]; then fail "$1: expected '$3', got '$2'"; fi
}
expect_file() {
    if [ ! -f "$2" ]; then fail "$1: missing $2"; fi
}
expect_absent() {
    if [ -e "$2" ]; then fail "$1: unexpected $2"; fi
}

# Auto-install runs before the frontend and linker in both drivers, so only
# file state is asserted, never the build's exit code.
build_in() {
    (cd "$1" && "$VOWC_BIN" build --no-verify -o out "$FIXTURE" >/dev/null 2>&1) || true
}

new_project() {
    local dir="$TMP_ROOT/$1"
    mkdir -p "$dir/.claude" "$dir/.git"
    echo "$dir"
}

MONOLITHIC="monolithic skill content"

# 1. A monolithic SKILL.md survives a build and gains no support files.
proj=$(new_project monolithic)
mkdir -p "$proj/.claude/skills/vow"
printf '%s' "$MONOLITHIC" > "$proj/.claude/skills/vow/SKILL.md"
build_in "$proj"
expect "auto-install keeps monolithic SKILL.md" \
    "$(cat "$proj/.claude/skills/vow/SKILL.md")" "$MONOLITHIC"
for sub in reference examples schemas; do
    expect_absent "auto-install adds no $sub/" "$proj/.claude/skills/vow/$sub"
done

# 2. Explicit install migrates it to the split layout.
(cd "$proj" && "$VOWC_BIN" skill install --local >/dev/null 2>&1) \
    || fail "skill install --local exited non-zero"
case "$(head -2 "$proj/.claude/skills/vow/SKILL.md")" in
    *"name: vow"*) ;;
    *) fail "SKILL.md was not rewritten with the split-layout entrypoint" ;;
esac
expect_file "install writes cli reference" "$proj/.claude/skills/vow/reference/cli.md"
expect_file "install writes examples" "$proj/.claude/skills/vow/examples/examples.md"
expect_file "install writes schemas" \
    "$proj/.claude/skills/vow/schemas/build-result.schema.json"

# 3. Explicit install repairs missing support files and keeps foreign files.
rm -rf "$proj/.claude/skills/vow/reference"
printf 'mine' > "$proj/.claude/skills/vow/notes.md"
(cd "$proj" && "$VOWC_BIN" skill install --local >/dev/null 2>&1) \
    || fail "repair install exited non-zero"
expect_file "install restores reference/" "$proj/.claude/skills/vow/reference/cli.md"
expect "install keeps foreign file" "$(cat "$proj/.claude/skills/vow/notes.md")" "mine"

# 4. A pre-rename vow-toolchain directory is neither detected nor modified,
#    and a fresh auto-install writes the full split tree next to it.
proj=$(new_project legacy-dir)
mkdir -p "$proj/.claude/skills/vow-toolchain"
printf '%s' "$MONOLITHIC" > "$proj/.claude/skills/vow-toolchain/SKILL.md"
build_in "$proj"
expect "legacy dir untouched" \
    "$(cat "$proj/.claude/skills/vow-toolchain/SKILL.md")" "$MONOLITHIC"
expect_file "auto-install creates skills/vow" "$proj/.claude/skills/vow/SKILL.md"
expect_file "auto-install writes cli reference" "$proj/.claude/skills/vow/reference/cli.md"

if [ "$failures" -ne 0 ]; then
    echo "$failures check(s) failed" >&2
    exit 1
fi
echo "skill-install: all checks passed"
