#!/usr/bin/env bash
# Measure peak RSS of build/vowc compiling compiler/main.vow.
# Used as baseline + post-refactor gate for issue #178 (driver lifetimes).
#
# Default: 3 samples of `--no-verify` compilation (the cleanest signal for
# driver-side memory; ESBMC subprocesses dominate verify-mode RSS and are a
# separate concern tracked in #175 / #179).
#
# Env:
#   VOW_RSS_SAMPLES         (default 3)  samples per configuration
#   VOW_RSS_INCLUDE_VERIFY  (default 0)  also measure --verify-jobs 1 (slow)
#   VOW_RSS_OUT             (default build/bootstrap_rss.json)
#
# Output: writes JSON to $VOW_RSS_OUT and a one-line summary to stdout.

set -euo pipefail

cd "$(dirname "$0")/.."

SAMPLES="${VOW_RSS_SAMPLES:-3}"
INCLUDE_VERIFY="${VOW_RSS_INCLUDE_VERIFY:-0}"
OUT="${VOW_RSS_OUT:-build/bootstrap_rss.json}"

if ! [[ "$SAMPLES" =~ ^[0-9]+$ ]] || [ "$SAMPLES" -lt 1 ]; then
    echo "error: VOW_RSS_SAMPLES must be a positive integer (got: '$SAMPLES')" >&2
    exit 1
fi

# Capability probe: /usr/bin/time must support -v and produce a "Maximum
# resident set size" line. Mirrors bench/memory/run.sh:225-233.
probe=$(mktemp)
trap 'rm -f "$probe"' EXIT
if ! /usr/bin/time -v -o "$probe" true >/dev/null 2>&1; then
    echo "error: /usr/bin/time must support -v" >&2
    exit 1
fi
if ! grep -q "Maximum resident set size" "$probe"; then
    echo "error: /usr/bin/time -v output did not include 'Maximum resident set size'" >&2
    exit 1
fi

if [ ! -x build/vowc ]; then
    echo "error: build/vowc not found; run scripts/bootstrap.sh first" >&2
    exit 1
fi

# Mirrors bench/memory/run.sh:131-134.
parse_max_rss() {
    awk -F': ' '/Maximum resident set size/ { print $2; exit }' "$1"
}

# Integer median over positional args.
median() {
    printf '%s\n' "$@" | sort -n | awk '
        { a[NR]=$1 }
        END {
            n=NR
            if (n%2==1) print a[(n+1)/2]
            else print int((a[n/2] + a[n/2+1])/2)
        }
    '
}

# Run one configuration N times and echo the median peak RSS (kbytes).
# Args: label, then the full vowc command (with placeholder output handled here).
measure_one() {
    local label="$1"; shift
    local samples=() i tmplog tmpout rss
    for ((i=1; i<=SAMPLES; i++)); do
        tmplog=$(mktemp)
        tmpout=$(mktemp "/tmp/vowc_rss_${label}_XXXX")
        if ! /usr/bin/time -v -o "$tmplog" "$@" -o "$tmpout" >/dev/null 2>&1; then
            echo "--- /usr/bin/time output ($label sample $i) ---" >&2
            cat "$tmplog" >&2
            echo "error: measurement failed for $label" >&2
            rm -f "$tmplog" "$tmpout"
            return 1
        fi
        rss=$(parse_max_rss "$tmplog")
        if [ -z "$rss" ]; then
            cat "$tmplog" >&2
            echo "error: could not extract max RSS for $label" >&2
            rm -f "$tmplog" "$tmpout"
            return 1
        fi
        samples+=("$rss")
        rm -f "$tmplog" "$tmpout"
    done
    echo "  samples ($label): ${samples[*]}" >&2
    median "${samples[@]}"
}

mkdir -p build

echo "Measuring build/vowc build --no-verify compiler/main.vow ($SAMPLES samples)..." >&2
nv_rss=$(measure_one no_verify build/vowc build --no-verify compiler/main.vow)
echo "  median peak RSS (kbytes): $nv_rss" >&2

verify_block='"stage2_default_kb": null'
if [ "$INCLUDE_VERIFY" = "1" ]; then
    echo "Measuring build/vowc build --verify-jobs 1 compiler/main.vow ($SAMPLES samples)..." >&2
    v_rss=$(measure_one verify_j1 build/vowc build --verify-jobs 1 compiler/main.vow)
    echo "  median peak RSS (kbytes): $v_rss" >&2
    verify_block="\"stage2_default_kb\": $v_rss"
fi

cat > "$OUT" <<EOF
{
    "stage2_no_verify_kb": $nv_rss,
    $verify_block,
    "samples": $SAMPLES
}
EOF

echo "Wrote $OUT"
echo "  stage2_no_verify_kb = $nv_rss"
if [ "$INCLUDE_VERIFY" = "1" ]; then
    echo "  stage2_default_kb   = $v_rss"
fi
