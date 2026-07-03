#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

NOTE="Legacy in-place upgrade note: 'vow' is now the installed command; the legacy 'vowc' name is no longer installed. If this same shell behaves as though old command paths are cached, refresh command lookup (bash: hash -r; zsh: rehash)."

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

fail() {
    echo "FAIL: $1" >&2
    exit 1
}

assert_contains() {
    local haystack="$1"
    local needle="$2"
    local label="$3"

    if [[ "$haystack" != *"$needle"* ]]; then
        fail "$label: expected output to contain: $needle"
    fi
}

assert_not_contains() {
    local haystack="$1"
    local needle="$2"
    local label="$3"

    if [[ "$haystack" == *"$needle"* ]]; then
        fail "$label: expected output not to contain: $needle"
    fi
}

make_fake_repo() {
    local repo="$1"

    mkdir -p "$repo/scripts" "$repo/build" "$repo/target/release"
    cp scripts/install-toolchain.sh "$repo/scripts/install-toolchain.sh"
    chmod +x "$repo/scripts/install-toolchain.sh"

    printf '#!/usr/bin/env bash\nprintf "fake vowc\\n"\n' > "$repo/build/vowc"
    chmod +x "$repo/build/vowc"
    printf 'fake runtime lib\n' > "$repo/target/release/libvow_runtime.a"
    printf 'fake shim lib\n' > "$repo/target/release/libvow_clif_shim.a"
}

test_legacy_upgrade_prints_command_cache_note() {
    local repo="$TMPDIR/legacy_repo"
    local prefix="$TMPDIR/legacy_prefix"
    local output

    make_fake_repo "$repo"

    mkdir -p "$prefix/bin"
    printf '#!/usr/bin/env bash\nprintf "old vowc\\n"\n' > "$prefix/bin/vowc"
    chmod +x "$prefix/bin/vowc"
    ln -s vowc "$prefix/bin/vow"

    output=$(bash "$repo/scripts/install-toolchain.sh" --prefix "$prefix" --skip-bootstrap)

    [ ! -e "$prefix/bin/vowc" ] || fail "legacy upgrade should remove bin/vowc"
    [ -f "$prefix/bin/vow" ] || fail "legacy upgrade should install bin/vow as a regular file"
    [ ! -L "$prefix/bin/vow" ] || fail "legacy upgrade should replace bin/vow symlink"
    [ -x "$prefix/bin/vow" ] || fail "legacy upgrade should install executable bin/vow"
    assert_contains "$output" "Add $prefix/bin to PATH if it is not already present." "legacy upgrade PATH reminder"
    assert_contains "$output" "$NOTE" "legacy upgrade command-cache note"
}

test_fresh_and_current_installs_do_not_print_command_cache_note() {
    local repo="$TMPDIR/current_repo"
    local fresh_prefix="$TMPDIR/fresh_prefix"
    local current_prefix="$TMPDIR/current_prefix"
    local fresh_output
    local current_output

    make_fake_repo "$repo"

    fresh_output=$(bash "$repo/scripts/install-toolchain.sh" --prefix "$fresh_prefix" --skip-bootstrap)
    assert_contains "$fresh_output" "Add $fresh_prefix/bin to PATH if it is not already present." "fresh install PATH reminder"
    assert_not_contains "$fresh_output" "$NOTE" "fresh install command-cache note"

    mkdir -p "$current_prefix/bin"
    printf '#!/usr/bin/env bash\nprintf "current vow\\n"\n' > "$current_prefix/bin/vow"
    chmod +x "$current_prefix/bin/vow"

    current_output=$(bash "$repo/scripts/install-toolchain.sh" --prefix "$current_prefix" --skip-bootstrap)
    assert_contains "$current_output" "Add $current_prefix/bin to PATH if it is not already present." "current install PATH reminder"
    assert_not_contains "$current_output" "$NOTE" "current install command-cache note"
}

test_legacy_upgrade_prints_command_cache_note
test_fresh_and_current_installs_do_not_print_command_cache_note

echo "install-toolchain tests passed"
