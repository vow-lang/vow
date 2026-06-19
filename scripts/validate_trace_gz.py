#!/usr/bin/env python3
"""Validate a gzipped Chrome Trace Event Format trace emitted by `--perfetto`.

Usage:
    validate_trace_gz.py <trace.json.gz> [--require name1,name2] [--expect-counters]

Checks the file gunzips, parses as JSON, has a non-empty `traceEvents` array,
and (optionally) contains the required `ph:"X"` span names and at least one
counter sample. Exits 0 on success, 1 with a message on any failure. Used by
scripts/full_test.sh to verify the self-hosted --perfetto tracer (issue #784).
"""

import gzip
import json
import sys


def fail(msg: str) -> "None":
    print(f"FAIL: {msg}", file=sys.stderr)
    sys.exit(1)


def main() -> "None":
    args = sys.argv[1:]
    if not args:
        fail("usage: validate_trace_gz.py <trace.json.gz> [--require a,b] [--expect-counters]")
    path = args[0]
    require = []
    expect_counters = False
    i = 1
    while i < len(args):
        if args[i] == "--require" and i + 1 < len(args):
            require = [s for s in args[i + 1].split(",") if s]
            i += 2
        elif args[i] == "--expect-counters":
            expect_counters = True
            i += 1
        else:
            fail(f"unknown argument: {args[i]}")

    try:
        with gzip.open(path, "rb") as f:
            doc = json.load(f)
    except OSError as e:
        fail(f"could not gunzip {path}: {e}")
    except json.JSONDecodeError as e:
        fail(f"trace is not valid JSON: {e}")

    events = doc.get("traceEvents")
    if not isinstance(events, list) or not events:
        fail("traceEvents missing or empty")

    span_names = {e.get("name") for e in events if e.get("ph") == "X"}
    for name in require:
        if name == "esbmc":
            if not any(str(e.get("name", "")).startswith("esbmc:") for e in events if e.get("ph") == "X"):
                fail("no esbmc:<fn> proof span present")
        elif name not in span_names:
            fail(f"required span '{name}' missing (have: {sorted(span_names)})")

    if expect_counters:
        if not any(e.get("ph") == "C" for e in events):
            fail("no counter (ph:C) samples present")

    print(f"OK: {len(events)} events; spans={sorted(span_names)}")


if __name__ == "__main__":
    main()
