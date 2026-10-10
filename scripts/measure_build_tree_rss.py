#!/usr/bin/env python3
"""Measure the peak summed RSS of a command and all of its descendants.

`/usr/bin/time -v` and `ru_maxrss` report the largest *single* process, so
they cannot show the effect of staging codegen before ESBMC (#179): the
driver and its ESBMC children are separate processes whose resident sets
add up only in the tree sum this script samples from /proc.

Usage:
    python3 scripts/measure_build_tree_rss.py [--interval-ms N] -- CMD [ARG...]

Prints one JSON object: peak_tree_kb, peak_self_kb (the command's own pid),
peak_at_s (seconds after start), wall_s, exit_code. Linux only.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time


def read_status(pid: int) -> tuple[int, int] | None:
    """Return (ppid, rss_kb) for a live pid, or None if it has gone."""
    try:
        with open(f"/proc/{pid}/status", encoding="ascii", errors="replace") as f:
            ppid = 0
            rss = 0
            for line in f:
                if line.startswith("PPid:"):
                    ppid = int(line.split()[1])
                elif line.startswith("VmRSS:"):
                    rss = int(line.split()[1])
    except OSError:
        return None
    return ppid, rss


def snapshot(root: int) -> tuple[int, int]:
    """Return (tree_rss_kb, root_rss_kb) over the root and its descendants."""
    procs: dict[int, tuple[int, int]] = {}
    for entry in os.listdir("/proc"):
        if entry.isdigit():
            status = read_status(int(entry))
            if status is not None:
                procs[int(entry)] = status
    members = {root}
    grew = True
    while grew:
        grew = False
        for pid, (ppid, _) in procs.items():
            if ppid in members and pid not in members:
                members.add(pid)
                grew = True
    tree = sum(procs[p][1] for p in members if p in procs)
    own = procs[root][1] if root in procs else 0
    return tree, own


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--interval-ms", type=int, default=20)
    parser.add_argument("cmd", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    cmd = args.cmd[1:] if args.cmd and args.cmd[0] == "--" else args.cmd
    if not cmd:
        parser.error("missing command")

    start = time.monotonic()
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    peak_tree = peak_self = 0
    peak_at = 0.0
    while proc.poll() is None:
        tree, own = snapshot(proc.pid)
        if tree > peak_tree:
            peak_tree, peak_at = tree, time.monotonic() - start
        peak_self = max(peak_self, own)
        time.sleep(args.interval_ms / 1000)
    wall = time.monotonic() - start
    print(
        json.dumps(
            {
                "peak_tree_kb": peak_tree,
                "peak_self_kb": peak_self,
                "peak_at_s": round(peak_at, 3),
                "wall_s": round(wall, 3),
                "exit_code": proc.returncode,
            }
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
