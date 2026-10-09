#!/usr/bin/env python3
"""Compare `vowc verify` verdicts of the ESBMC and native backends.

Runs both backends over the given fixtures and reports every fixture the native
backend decides (anything but Skipped) whose status differs from ESBMC's. A
fixture the native backend skips is not compared: it makes no claim. Known,
documented divergences are listed in KNOWN_DIVERGENCES with the reason.

    python3 scripts/native_vs_esbmc.py build/vowc tests/verify/max.vow ...

Exit status is 1 when an undocumented mismatch is found, 2 on a usage error.
"""

import json
import subprocess
import sys

KNOWN_DIVERGENCES = {
    "signed_div_min.vow": "ESBMC's model does not check MIN / -1 for `/`",
    "io_caller_unchecked_note.vow": "ESBMC's model does not check MIN / -1 for `/`",
    "where_divide.vow": "ESBMC's model does not check MIN / -1 for `/`",
    "modulo_safe.vow": "straight-line x - (x / m) * m: the native solver run does not finish within the budget (pre-existing, no branches)",
}


def run(vowc, fixture, extra):
    proc = subprocess.run(
        [vowc, "verify", "--no-cache", *extra, fixture],
        capture_output=True,
        text=True,
        timeout=900,
    )
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
        if native != esbmc:
            name = fixture.rsplit("/", 1)[-1]
            note = KNOWN_DIVERGENCES.get(name)
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
