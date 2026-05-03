#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

PREFIX="${HOME:?}/.local"
SKIP_BOOTSTRAP=false

usage() {
    echo "Usage: $0 [--prefix <path>] [--skip-bootstrap] [--help|-h]"
    echo ""
    echo "Install the self-hosted Vow toolchain into a prefix."
    echo ""
    echo "Installs:"
    echo "  <prefix>/bin/vowc"
    echo "  <prefix>/bin/vow"
    echo "  <prefix>/lib/vow/libvow_runtime.a"
    echo "  <prefix>/lib/vow/libvow_clif_shim.a"
    echo ""
    echo "Options:"
    echo "  --prefix <path>    Installation prefix (default: \$HOME/.local)"
    echo "  --skip-bootstrap   Require existing build artifacts; do not run bootstrap"
    echo "  -h, --help         Show this help"
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
RUNTIME_LIB="target/release/libvow_runtime.a"
SHIM_LIB="target/release/libvow_clif_shim.a"

artifacts_ready() {
    [ -x "$VOWC" ] && [ -f "$RUNTIME_LIB" ] && [ -f "$SHIM_LIB" ]
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

install -m 0755 "$VOWC" "$BIN_DIR/vowc"
if ln -sfn "vowc" "$BIN_DIR/vow" 2>/dev/null; then
    :
else
    install -m 0755 "$VOWC" "$BIN_DIR/vow"
fi

install -m 0644 "$RUNTIME_LIB" "$LIB_DIR/libvow_runtime.a"
install -m 0644 "$SHIM_LIB" "$LIB_DIR/libvow_clif_shim.a"

echo "Installed Vow toolchain to $PREFIX"
echo "Add $BIN_DIR to PATH if it is not already present."
