#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

PREFIX="${HOME:?}/.local"
SKIP_BOOTSTRAP=false
WITH_RUST_COMPILER=false

usage() {
    echo "Usage: $0 [--prefix <path>] [--with-rust-compiler] [--skip-bootstrap] [--help|-h]"
    echo ""
    echo "Install the self-hosted Vow toolchain into a prefix."
    echo ""
    echo "Installs:"
    echo "  <prefix>/bin/vow                    (self-hosted compiler)"
    echo "  <prefix>/lib/vow/libvow_runtime.a"
    echo "  <prefix>/lib/vow/libvow_clif_shim.a"
    echo ""
    echo "With --with-rust-compiler, additionally installs:"
    echo "  <prefix>/bin/vowr                   (Rust bootstrap compiler)"
    echo ""
    echo "Options:"
    echo "  --prefix <path>        Installation prefix (default: \$HOME/.local)"
    echo "  --with-rust-compiler   Also install the Rust bootstrap compiler as 'vowr'"
    echo "  --skip-bootstrap       Require existing build artifacts; do not run bootstrap"
    echo "  -h, --help             Show this help"
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --prefix)
            if [ "$#" -lt 2 ]; then
                echo "Error: --prefix requires a path" >&2
                exit 1
            fi
            PREFIX="$2"
            shift 2
            ;;
        --with-rust-compiler)
            WITH_RUST_COMPILER=true
            shift
            ;;
        --skip-bootstrap)
            SKIP_BOOTSTRAP=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown flag: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

VOWC="build/vowc"
RUST_COMPILER="target/release/vow"
RUNTIME_LIB="target/release/libvow_runtime.a"
SHIM_LIB="target/release/libvow_clif_shim.a"

artifacts_ready() {
    [ -x "$VOWC" ] && [ -f "$RUNTIME_LIB" ] && [ -f "$SHIM_LIB" ] || return 1
    if [ "$WITH_RUST_COMPILER" = true ]; then
        [ -x "$RUST_COMPILER" ] || return 1
    fi
    return 0
}

if ! artifacts_ready; then
    if [ "$SKIP_BOOTSTRAP" = true ]; then
        echo "Error: missing toolchain artifacts." >&2
        echo "Run: scripts/bootstrap.sh" >&2
        exit 1
    fi
    scripts/bootstrap.sh
fi

if ! artifacts_ready; then
    echo "Error: bootstrap completed but required artifacts are still missing." >&2
    exit 1
fi

BIN_DIR="$PREFIX/bin"
LIB_DIR="$PREFIX/lib/vow"
mkdir -p "$BIN_DIR" "$LIB_DIR"

LEGACY_INSTALL=false
if [ -L "$BIN_DIR/vow" ] || [ -e "$BIN_DIR/vowc" ] || [ -L "$BIN_DIR/vowc" ]; then
    LEGACY_INSTALL=true
fi

rm -f "$BIN_DIR/vow" "$BIN_DIR/vowc"
install -m 0755 "$VOWC" "$BIN_DIR/vow"

install -m 0644 "$RUNTIME_LIB" "$LIB_DIR/libvow_runtime.a"
install -m 0644 "$SHIM_LIB" "$LIB_DIR/libvow_clif_shim.a"

if [ "$WITH_RUST_COMPILER" = true ]; then
    install -m 0755 "$RUST_COMPILER" "$BIN_DIR/vowr"
fi

echo "Installed Vow toolchain to $PREFIX"
echo "  vow  -> self-hosted compiler"
if [ "$WITH_RUST_COMPILER" = true ]; then
    echo "  vowr -> Rust bootstrap compiler"
fi
echo "Add $BIN_DIR to PATH if it is not already present."
if [ "$LEGACY_INSTALL" = true ]; then
    echo "Legacy in-place upgrade note: 'vow' is now the installed command; the legacy 'vowc' name is no longer installed. If this same shell behaves as though old command paths are cached, refresh command lookup (bash: hash -r; zsh: rehash)."
fi
