#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION=""
TARGET_OS=""
TARGET_ARCH=""
OUTPUT_DIR="release"

usage() {
    local code="${1:-1}"
    if [ "$code" -eq 0 ]; then
        echo "Usage: $0 --version <version> --os <linux|macos> --arch <x86_64|aarch64> [--output-dir <dir>]"
    else
        echo "Usage: $0 --version <version> --os <linux|macos> --arch <x86_64|aarch64> [--output-dir <dir>]" >&2
    fi
    exit "$code"
}

sha256_file() {
    local path="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$path" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$path" | awk '{print $1}'
    else
        echo "Error: neither sha256sum nor shasum is available" >&2
        exit 1
    fi
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || usage
            VERSION="$2"
            shift 2
            ;;
        --os)
            [ "$#" -ge 2 ] || usage
            TARGET_OS="$2"
            shift 2
            ;;
        --arch)
            [ "$#" -ge 2 ] || usage
            TARGET_ARCH="$2"
            shift 2
            ;;
        --output-dir)
            [ "$#" -ge 2 ] || usage
            OUTPUT_DIR="$2"
            shift 2
            ;;
        -h|--help)
            usage 0
            ;;
        *)
            echo "Unknown flag: $1" >&2
            usage
            ;;
    esac
done

[ -n "$VERSION" ] || usage
[ -n "$TARGET_OS" ] || usage
[ -n "$TARGET_ARCH" ] || usage

case "$TARGET_OS" in
    linux|macos) ;;
    *) echo "Error: unsupported OS '$TARGET_OS'" >&2; exit 1 ;;
esac

case "$TARGET_ARCH" in
    x86_64|aarch64) ;;
    *) echo "Error: unsupported architecture '$TARGET_ARCH'" >&2; exit 1 ;;
esac

VOWC="build/vowc"
RUNTIME_LIB="target/release/libvow_runtime.a"
SHIM_LIB="target/release/libvow_clif_shim.a"

if [ ! -x "$VOWC" ]; then
    echo "Error: missing executable $VOWC" >&2
    exit 1
fi
if [ ! -f "$RUNTIME_LIB" ]; then
    echo "Error: missing $RUNTIME_LIB" >&2
    exit 1
fi
if [ ! -f "$SHIM_LIB" ]; then
    echo "Error: missing $SHIM_LIB" >&2
    exit 1
fi

mkdir -p "$OUTPUT_DIR"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/vow-toolchain.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT INT TERM HUP

prefix="$tmp/vow-$VERSION"
scripts/install-toolchain.sh --prefix "$prefix" --skip-bootstrap >/dev/null

asset="vow-${TARGET_OS}-${TARGET_ARCH}.tar.gz"
tarball="$OUTPUT_DIR/$asset"
tar -czf "$tarball" -C "$tmp" "vow-$VERSION"

sha="$(sha256_file "$tarball")"
printf "%s  %s\n" "$sha" "$asset" > "$tarball.sha256"

echo "$tarball"
