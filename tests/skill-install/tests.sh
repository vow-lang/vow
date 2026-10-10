#!/usr/bin/env bash
set -euo pipefail

# Skill installs are all-or-nothing in both compilers (issue #361): the tree is
# staged in a sibling directory and renamed into place, so a failure never
# leaves a SKILL.md that links to missing reference/, examples/ or schemas/
# files, and auto-install on `build` stays silent and never fails the build.
#
#   VOWC_BIN=build/vowc            bash tests/skill-install/tests.sh
#   VOWC_BIN=target/release/vow VOWC_KIND=rust bash tests/skill-install/tests.sh

VOWC_BIN="${VOWC_BIN:-build/vowc}"
VOWC_BIN=$(cd "$(dirname "$VOWC_BIN")" && pwd)/$(basename "$VOWC_BIN")
VOWC_KIND="${VOWC_KIND:-self}"
REPO_ROOT=$(pwd)
FIXTURE="$REPO_ROOT/examples/hello.vow"
MIRROR="$REPO_ROOT/skills/vow"
TMP_ROOT=$(mktemp -d)
cleanup() {
    chmod -R u+rwX "$TMP_ROOT" 2>/dev/null || true
    rm -rf "$TMP_ROOT"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }
expect() {
    if [ "$2" != "$3" ]; then fail "$1: expected '$3', got '$2'"; fi
}

new_project() {
    local dir
    dir=$(mktemp -d "$TMP_ROOT/proj.XXXXXX")
    mkdir -p "$dir/.git" "$dir/.claude"
    echo "$dir"
}

# Runs the compiler inside $1; sets rc, out and err for the caller.
run_in() {
    local dir="$1"; shift
    rc=0
    (cd "$dir" && "$VOWC_BIN" "$@") >"$TMP_ROOT/out" 2>"$TMP_ROOT/err" || rc=$?
    out=$(cat "$TMP_ROOT/out")
    err=$(cat "$TMP_ROOT/err")
}

# Byte equality is not asserted: the self-hosted driver embeds ASCII-only copies
# of some documents, so the tree is compared by file list and non-empty content.
tree_listing() { (cd "$1" && find . -type f | LC_ALL=C sort); }

assert_tree_at() {
    local label="$1" tree="$2" f
    expect "$label: file list" "$(tree_listing "$tree")" "$(tree_listing "$MIRROR")"
    while IFS= read -r f; do
        if [ ! -s "$tree/$f" ]; then fail "$label: $f is missing or empty"; fi
    done < <(tree_listing "$MIRROR")
}

assert_complete_tree() {
    assert_tree_at "$1" "$2/.claude/skills/vow"
}

assert_no_leftovers() {
    local label="$1" dir="$2" entries
    entries=$(ls -A "$dir/.claude/skills")
    expect "$label: .claude/skills entries" "$entries" "vow"
}

# fresh explicit install
p=$(new_project)
run_in "$p" skill install --local
expect "fresh install exit" "$rc" "0"
case "$err" in *"installed skill to"*".claude/skills/vow/SKILL.md"*) ;;
    *) fail "fresh install stderr lacks success line: $err" ;;
esac
assert_complete_tree "fresh install" "$p"
assert_no_leftovers "fresh install" "$p"

# re-install is idempotent
run_in "$p" skill install --local
expect "re-install exit" "$rc" "0"
assert_complete_tree "re-install" "$p"
assert_no_leftovers "re-install" "$p"

# stale and user-added files are replaced with the whole tree
mkdir -p "$p/.claude/skills/vow/reference"
echo stale >"$p/.claude/skills/vow/reference/stale.md"
run_in "$p" skill install --local
expect "replace exit" "$rc" "0"
assert_complete_tree "replace" "$p"
assert_no_leftovers "replace" "$p"

# #361 regression: a regular file named `reference` blocks the support tree.
# The old installer wrote SKILL.md first, failed on reference/, and left an
# orphan entrypoint that auto-install then never repaired.
p=$(new_project)
mkdir -p "$p/.claude/skills/vow"
echo blocker >"$p/.claude/skills/vow/reference"
run_in "$p" skill install --local
expect "blocked explicit exit" "$rc" "0"
assert_complete_tree "blocked explicit" "$p"
assert_no_leftovers "blocked explicit" "$p"

p=$(new_project)
mkdir -p "$p/.claude/skills/vow"
echo blocker >"$p/.claude/skills/vow/reference"
run_in "$p" build --no-verify "$FIXTURE" -o "$TMP_ROOT/hello"
expect "blocked auto-install build exit" "$rc" "0"
assert_complete_tree "blocked auto-install" "$p"

# a partial tree (entrypoint only) is repaired by an explicit install
p=$(new_project)
mkdir -p "$p/.claude/skills/vow"
echo old >"$p/.claude/skills/vow/SKILL.md"
run_in "$p" skill install --local
expect "partial repair exit" "$rc" "0"
assert_complete_tree "partial repair" "$p"

# unwritable .claude/skills: explicit install fails and keeps the old tree
if [ "$(id -u)" != "0" ]; then
    p=$(new_project)
    mkdir -p "$p/.claude/skills/vow/reference"
    echo old >"$p/.claude/skills/vow/SKILL.md"
    echo stale >"$p/.claude/skills/vow/reference/stale.md"
    chmod 555 "$p/.claude/skills"
    run_in "$p" skill install --local
    expect "unwritable exit" "$rc" "1"
    case "$err" in *"vow skill install: cannot"*) ;;
        *) fail "unwritable stderr lacks an install error: $err" ;;
    esac
    expect "unwritable keeps SKILL.md" "$(cat "$p/.claude/skills/vow/SKILL.md")" "old"
    expect "unwritable keeps stale.md" "$(cat "$p/.claude/skills/vow/reference/stale.md")" "stale"
    expect "unwritable entries" "$(ls -A "$p/.claude/skills")" "vow"
    chmod 755 "$p/.claude/skills"
fi

# auto-install on build: complete tree, no skill chatter, build succeeds
p=$(new_project)
run_in "$p" build --no-verify "$FIXTURE" -o "$TMP_ROOT/hello"
expect "auto-install build exit" "$rc" "0"
assert_complete_tree "auto-install" "$p"
assert_no_leftovers "auto-install" "$p"
case "$err" in *skill*) fail "auto-install is not silent: $err" ;; esac

# auto-install failure is silent and never fails the build
p=$(new_project)
echo "not a directory" >"$p/.claude/skills"
run_in "$p" build --no-verify "$FIXTURE" -o "$TMP_ROOT/hello"
expect "failed auto-install build exit" "$rc" "0"
case "$err" in *skill*) fail "failed auto-install is not silent: $err" ;; esac
expect "failed auto-install leaves skills file" "$(cat "$p/.claude/skills")" "not a directory"

# a symlinked skill directory is written in place and the link is kept
p=$(new_project)
mkdir -p "$p/dotfiles/vow" "$p/.claude/skills"
ln -s ../../dotfiles/vow "$p/.claude/skills/vow"
run_in "$p" skill install --local
expect "symlink exit" "$rc" "0"
if [ ! -L "$p/.claude/skills/vow" ]; then fail "symlinked skill directory was replaced"; fi
assert_tree_at "symlink target" "$p/dotfiles/vow"
assert_no_leftovers "symlink" "$p"

if [ "$failures" -ne 0 ]; then
    echo "$failures skill-install check(s) failed ($VOWC_KIND)" >&2
    exit 1
fi
echo "skill-install: all checks passed ($VOWC_KIND)"
