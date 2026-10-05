#!/usr/bin/env python3
"""Check the `bench/memory` programs against their recorded RSS bounds.

`bench/memory/run.sh` is the developer-facing harness: it needs GNU
`/usr/bin/time -v` and only drives the Rust stage-0 compiler. This script is the
CI-facing counterpart. It builds every `bench/memory/programs/*.vow` with the
compiler given on the command line (the Rust compiler or the self-hosted
`vowc`), runs the executable, and fails if its peak resident set exceeds the
`// BENCH: max-rss-kb N` annotation on the first line of the source. Running it
once per compiler is what pins a region-inference or allocation regression in
*both* compilers: a per-call leak makes the peak grow with the loop count, far
past the 4 MiB cushion the bounds carry.

The peak is read from `/proc/<pid>/status` (`VmHWM`) while the program runs, so
no external `time` binary is needed and the number belongs to the program alone
(a `wait4` `ru_maxrss` would include the resident size of the forking parent).
Linux only; on other platforms the script reports a skip and exits 0.

Usage:
  scripts/check_memory_bounds.py --compiler ./target/release/vow [--filter NAME]
  scripts/check_memory_bounds.py --compiler build/vowc
"""

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
PROGRAM_DIR = REPO_ROOT / "bench" / "memory" / "programs"
BENCH_RE = re.compile(r"^// BENCH: max-rss-kb ([0-9]+)$")
RUN_TIMEOUT_SECS = 120


def read_bound(source: Path) -> int | None:
    """Return the `// BENCH: max-rss-kb N` bound on the first line, if any."""
    with source.open(encoding="utf-8") as handle:
        match = BENCH_RE.match(handle.readline().strip())
    return int(match.group(1)) if match else None


def vm_hwm_kb(pid: int, executable: Path) -> int | None:
    """Peak RSS of `pid` in KiB, once it is running `executable`.

    Between fork and exec the child still maps a copy of its parent's address
    space, so its `VmHWM` is the parent's. Only a process whose `exe` is the
    program counts.
    """
    try:
        if os.path.realpath(os.readlink(f"/proc/{pid}/exe")) != str(
            executable.resolve()
        ):
            return None
        with open(f"/proc/{pid}/status", encoding="ascii") as status:
            for line in status:
                if line.startswith("VmHWM:"):
                    return int(line.split()[1])
    except (OSError, ValueError):
        return None
    return None


def run_and_measure(executable: Path) -> tuple[int, int]:
    """Run `executable`; return (exit code, peak RSS in KiB sampled while it ran)."""
    with tempfile.TemporaryFile() as out:
        proc = subprocess.Popen(
            [str(executable)], stdout=out, stderr=subprocess.DEVNULL
        )
        peak = 0
        deadline_hit = False
        started = time.monotonic()
        while proc.poll() is None:
            sample = vm_hwm_kb(proc.pid, executable)
            if sample is not None:
                peak = max(peak, sample)
            if time.monotonic() - started > RUN_TIMEOUT_SECS:
                proc.kill()
                deadline_hit = True
                break
        proc.wait()
        return (-1 if deadline_hit else proc.returncode), peak


def build(compiler: Path, source: Path, output: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [
            str(compiler),
            "build",
            "--no-verify",
            "--no-cache",
            str(source),
            "-o",
            str(output),
        ],
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
        check=False,
    )


def check_program(compiler: Path, source: Path, work: Path) -> dict:
    name = source.stem
    bound = read_bound(source)
    if bound is None:
        return {"program": name, "status": "missing_annotation"}
    binary = work / name
    built = build(compiler, source, binary)
    if built.returncode != 0 or not binary.exists():
        return {
            "program": name,
            "status": "build_failed",
            "detail": (built.stdout + built.stderr)[-400:],
        }
    try:
        exit_code, peak = run_and_measure(binary)
    finally:
        binary.unlink(missing_ok=True)
    result = {
        "program": name,
        "bound_kb": bound,
        "max_rss_kb": peak,
        "run_exit": exit_code,
    }
    if exit_code != 0:
        result["status"] = "run_failed"
    elif peak == 0:
        result["status"] = "unsampled"
    elif peak > bound:
        result["status"] = "rss_exceeded"
    else:
        result["status"] = "pass"
    return result


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--compiler", required=True, help="Rust `vow` or self-hosted `vowc`"
    )
    parser.add_argument(
        "--filter", default="", help="only programs whose name contains this"
    )
    args = parser.parse_args(argv)

    if not sys.platform.startswith("linux"):
        print("SKIP: /proc-based RSS sampling needs Linux")
        return 0
    compiler = Path(args.compiler)
    if not compiler.exists():
        print(f"error: compiler not found: {compiler}", file=sys.stderr)
        return 2

    sources = sorted(p for p in PROGRAM_DIR.glob("*.vow") if args.filter in p.stem)
    if not sources:
        print("error: no bench/memory programs matched", file=sys.stderr)
        return 2

    failed = 0
    with tempfile.TemporaryDirectory(prefix="vow_memory_bounds_") as tmp:
        for source in sources:
            result = check_program(compiler, source, Path(tmp))
            print(json.dumps(result, sort_keys=True))
            if result["status"] != "pass":
                failed += 1
    print(f"{len(sources) - failed}/{len(sources)} memory bounds held")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
