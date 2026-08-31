#!/usr/bin/env bash
set -euo pipefail

VOWC_BIN="${VOWC_BIN:-build/vowc}"
TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT
# Kill signals re-raise so the EXIT handler above still does the removal:
# EXIT alone does not fire on an untrapped SIGTERM, so a process-group kill
# would strand this scratch tree. See scripts/full_test.sh.
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

run_vowc() {
    (ulimit -v 2000000; "$VOWC_BIN" "$@")
}

FAKE_BIN="$TMP_ROOT/bin"
LOOKUP_COUNT="$TMP_ROOT/esbmc-lookup-count"
FIXTURE="$TMP_ROOT/two-contracts.vow"
mkdir -p "$FAKE_BIN"

cat > "$FAKE_BIN/sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -eq 2 ] && [ "$1" = "-c" ] && [ "$2" = "command -v esbmc" ]; then
    count=0
    if [ -f "${VOW_ESBMC_LOOKUP_COUNT:?}" ]; then
        read -r count < "$VOW_ESBMC_LOOKUP_COUNT"
    fi
    printf '%s\n' "$((count + 1))" > "$VOW_ESBMC_LOOKUP_COUNT"
fi

exec /bin/sh "$@"
SH

cat > "$FAKE_BIN/esbmc" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf 'VERIFICATION SUCCESSFUL\n'
SH

cat > "$FIXTURE" <<'VOW'
module TwoContracts

fn keep_i64(x: i64) -> i64 vow {
    ensures: result == x
} {
    x
}

fn keep_u64(x: u64) -> u64 vow {
    ensures: result == x
} {
    x
}

fn main() -> i32 {
    0
}
VOW

chmod +x "$FAKE_BIN/sh" "$FAKE_BIN/esbmc"

verify_json=""
verify_exit=0
verify_json=$(PATH="$FAKE_BIN:$PATH" VOW_ESBMC_LOOKUP_COUNT="$LOOKUP_COUNT" \
    run_vowc verify --no-cache --verify-jobs 1 "$FIXTURE" 2>"$TMP_ROOT/verify.stderr") || verify_exit=$?

if [ "$verify_exit" -ne 0 ]; then
    printf 'self-hosted verify failed with exit %s\n' "$verify_exit" >&2
    cat "$TMP_ROOT/verify.stderr" >&2
    exit 1
fi

verify_status=$(python3 -c 'import json, sys; print(json.loads(sys.argv[1]).get("status", ""))' "$verify_json")
if [ "$verify_status" != "Verified" ]; then
    printf 'self-hosted verify returned status %s\n' "$verify_status" >&2
    exit 1
fi

lookup_count=0
if [ -f "$LOOKUP_COUNT" ]; then
    read -r lookup_count < "$LOOKUP_COUNT"
fi
if [ "$lookup_count" -ne 1 ]; then
    printf 'expected one ESBMC PATH lookup, got %s\n' "$lookup_count" >&2
    exit 1
fi

printf 'esbmc path resolved once across multiple functions\n'
