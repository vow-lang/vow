#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VOWC="${VOWC:-$ROOT_DIR/build/vowc}"
SIZE_MB="${SIZE_MB:-64}"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

if [[ ! -x "$VOWC" ]]; then
  echo "build/vowc not found; run scripts/bootstrap.sh first or set VOWC" >&2
  exit 1
fi

if [[ ! -x /usr/bin/time ]]; then
  echo "/usr/bin/time not available" >&2
  exit 1
fi

DATA="$WORK_DIR/large.ndjson"
STREAM_BIN="$WORK_DIR/streaming_count"
READ_BIN="$WORK_DIR/fs_read_count"

python3 - "$DATA" "$SIZE_MB" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
size_mb = int(sys.argv[2])
target = size_mb * 1024 * 1024
line = b'{"kind":"node","payload":"abcdefghijklmnopqrstuvwxyz0123456789"}\n'
written = 0
with path.open("wb") as f:
    while written < target:
        f.write(line)
        written += len(line)
PY

"$VOWC" build --no-verify "$ROOT_DIR/examples/streaming_file/streaming_count.vow" -o "$STREAM_BIN" >/dev/null
"$VOWC" build --no-verify "$ROOT_DIR/examples/streaming_file/fs_read_count.vow" -o "$READ_BIN" >/dev/null

/usr/bin/time -f "%M" -o "$WORK_DIR/streaming.rss" "$STREAM_BIN" "$DATA" > "$WORK_DIR/streaming.out"
/usr/bin/time -f "%M" -o "$WORK_DIR/fs_read.rss" "$READ_BIN" "$DATA" > "$WORK_DIR/fs_read.out"

if ! cmp -s "$WORK_DIR/streaming.out" "$WORK_DIR/fs_read.out"; then
  echo "streaming and fs_read counters disagreed" >&2
  diff -u "$WORK_DIR/streaming.out" "$WORK_DIR/fs_read.out" >&2 || true
  exit 1
fi

streaming_rss="$(cat "$WORK_DIR/streaming.rss")"
fs_read_rss="$(cat "$WORK_DIR/fs_read.rss")"

echo "data_mb=$SIZE_MB"
echo "streaming_peak_kb=$streaming_rss"
echo "fs_read_peak_kb=$fs_read_rss"

if (( streaming_rss >= fs_read_rss )); then
  echo "expected streaming reader to use less peak RSS than fs_read" >&2
  exit 1
fi
