#!/usr/bin/env python3
"""Byte-for-byte parity of the C each compiler hands to ESBMC.

`vow-verify/src/c_emitter.rs` and `compiler/c_emitter.vow` are specified to
emit identical C for the same IR. This module proves it on a fixture corpus: it
puts a fake `esbmc` first on PATH, runs `verify` with each compiler, and
compares what each one wrote to the ESBMC scratch file.

The fake `esbmc` only copies the C source it is given and reports success, so
the comparison needs no real solver and no network. Each ESBMC invocation is
keyed by the function its harness calls; a compiler may invoke ESBMC several
times for one function (solver portfolio, reachability or body-replace
variants), so the unit of comparison is the *set* of distinct C sources per
function. Both compilers must produce the same function names and the same
sets.
"""

import difflib
import os
import re
import stat
import subprocess
import tempfile
from pathlib import Path

SHIM = """#!/usr/bin/env bash
set -eu
for arg in "$@"; do
    if [ -f "$arg" ] && [ "${arg##*.}" = "c" ]; then
        dest=$(mktemp "$VOW_ESBMC_CAPTURE_DIR/esbmc.XXXXXX.c")
        cp "$arg" "$dest"
        break
    fi
done
echo "VERIFICATION SUCCESSFUL"
"""

HARNESS_CALL = re.compile(r"int main\(void\) \{ ([A-Za-z_][A-Za-z0-9_]*)\(")
VERIFY_TIMEOUT_SECS = 300


def install_shim(directory):
    """Write the capturing fake `esbmc` into `directory` and return its path."""
    path = Path(directory) / "esbmc"
    path.write_text(SHIM)
    path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return path


def harness_function(c_source):
    """Name of the function the ESBMC harness drives, or None."""
    found = HARNESS_CALL.search(c_source)
    return found.group(1) if found else None


def capture(binary, fixture, shim_dir, capture_dir):
    """Run `binary verify` on `fixture`; return {function: {distinct C source}}."""
    env = dict(os.environ)
    env["PATH"] = f"{shim_dir}{os.pathsep}{env.get('PATH', '')}"
    env["VOW_ESBMC_CAPTURE_DIR"] = str(capture_dir)
    subprocess.run(
        [binary, "verify", "--no-cache", "--verify-jobs", "1", str(fixture)],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=VERIFY_TIMEOUT_SECS,
        check=False,
    )
    captured = {}
    for path in sorted(Path(capture_dir).glob("esbmc.*.c")):
        source = path.read_text()
        key = harness_function(source) or "<no-harness>"
        captured.setdefault(key, set()).add(source)
    return captured


def diff_captures(rust, self_hosted):
    """Human-readable differences between two captures; empty when identical."""
    problems = []
    for name in sorted(set(rust) | set(self_hosted)):
        only = (
            "self-hosted"
            if name not in rust
            else "rust"
            if name not in self_hosted
            else None
        )
        if only:
            problems.append(f"{name}: C emitted only by the {only} compiler")
            continue
        rust_sources = sorted(rust[name])
        self_sources = sorted(self_hosted[name])
        if rust_sources == self_sources:
            continue
        for rust_src, self_src in zip(rust_sources, self_sources):
            if rust_src == self_src:
                continue
            diff = difflib.unified_diff(
                rust_src.splitlines(),
                self_src.splitlines(),
                "rust",
                "self-hosted",
                lineterm="",
                n=1,
            )
            problems.append(f"{name}:\n" + "\n".join(diff))
            break
        else:
            problems.append(
                f"{name}: rust emitted {len(rust_sources)} distinct C sources, "
                f"self-hosted {len(self_sources)}"
            )
    return problems


def compare_fixture(rust_bin, self_bin, fixture):
    """Problems for one fixture: [] when the C is byte-identical, None when
    neither compiler invoked ESBMC (nothing to compare)."""
    with tempfile.TemporaryDirectory(prefix="vow-parity-c-") as work:
        shim_dir = Path(work) / "bin"
        shim_dir.mkdir()
        install_shim(shim_dir)
        captures = []
        for label, binary in (("rust", rust_bin), ("self", self_bin)):
            capture_dir = Path(work) / label
            capture_dir.mkdir()
            captures.append(capture(binary, fixture, shim_dir, capture_dir))
    rust, self_hosted = captures
    if not rust and not self_hosted:
        return None
    return diff_captures(rust, self_hosted)


def main(args):
    """`parity.py c RUST_BIN SELF_BIN FIXTURE...`; exit 1 on any difference."""
    if len(args) < 3:
        print("usage: parity.py c RUST_BIN SELF_BIN FIXTURE...")
        return 2
    rust_bin, self_bin, *fixtures = args
    status = 0
    for fixture in fixtures:
        problems = compare_fixture(rust_bin, self_bin, fixture)
        if problems is None:
            print(f"SKIP {fixture}: neither compiler invoked ESBMC")
        elif problems:
            status = 1
            print(f"FAIL {fixture}")
            for problem in problems:
                print("  " + problem.replace("\n", "\n  "))
        else:
            print(f"OK {fixture}")
    return status
