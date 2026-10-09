#!/usr/bin/env python3
"""Compare `vowc verify` verdicts of the ESBMC and native backends.

Runs both backends over the given fixtures and reports every fixture the native
backend decides (anything but Skipped) whose status differs from ESBMC's. A
fixture the native backend skips is not compared: it makes no claim. A missing
JSON result or a timeout counts as a mismatch. Known, documented divergences are
listed in KNOWN_DIVERGENCES with the reason and are excused only while both
statuses match the recorded pair.

    python3 scripts/native_vs_esbmc.py build/vowc tests/verify/max.vow ...

Exit status is 1 when an undocumented mismatch is found, 2 on a usage error.
"""

import json
import subprocess
import sys

MIN_DIV = "ESBMC's model does not check MIN / -1 for `/`"
# fixture name -> (esbmc status, native status, reason). A divergence is only
# excused while both statuses are exactly the recorded pair.
KNOWN_DIVERGENCES = {
    "signed_div_min.vow": ("Verified", "VerifyFailed", MIN_DIV),
    "io_caller_unchecked_note.vow": ("Verified", "VerifyFailed", MIN_DIV),
    "where_divide.vow": ("Verified", "VerifyFailed", MIN_DIV),
    "modulo_safe.vow": (
        "Verified",
        "VerifyFailed",
        "straight-line x - (x / m) * m: the native solver run does not finish within the budget (pre-existing, no branches)",
    ),
}


def run(vowc, fixture, extra):
    try:
        proc = subprocess.run(
            [vowc, "verify", "--no-cache", *extra, fixture],
            capture_output=True,
            text=True,
            timeout=900,
        )
    except subprocess.TimeoutExpired:
        return "<timeout>"
    for line in reversed(proc.stdout.splitlines()):
        line = line.strip()
        if line.startswith("{"):
            try:
                return json.loads(line).get("status", "")
            except json.JSONDecodeError:
                break
    return "<no json>"


def main(argv):
    if len(argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    vowc, fixtures = argv[1], argv[2:]
    compared = skipped = 0
    mismatches = []
    for fixture in fixtures:
        native = run(vowc, fixture, ["--backend", "native"])
        if native == "Skipped":
            skipped += 1
            continue
        esbmc = run(vowc, fixture, [])
        compared += 1
        broken = native.startswith("<") or esbmc.startswith("<")
        if native != esbmc or broken:
            name = fixture.rsplit("/", 1)[-1]
            known = KNOWN_DIVERGENCES.get(name)
            note = known[2] if known and known[:2] == (esbmc, native) else None
            mismatches.append((fixture, esbmc, native, note))
            tag = "known divergence" if note else "MISMATCH"
            print(
                f"{tag}: {fixture}: esbmc={esbmc} native={native}"
                + (f" ({note})" if note else "")
            )
    print(
        f"compared {compared}, native-skipped {skipped}, mismatches {len(mismatches)}"
    )
    return 1 if any(note is None for *_, note in mismatches) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
