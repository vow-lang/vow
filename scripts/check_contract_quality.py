#!/usr/bin/env python3
"""Ratchet gate on static contract quality across the self-hosted compiler (#81).

Reads the JSON of `vow contracts compiler/main.vow` on stdin and fails if the
`weak` or `tautological` contract count exceeds a committed baseline. This keeps
new hollow contracts (`ensures result >= 0` and friends) from creeping in. The
classification is static (no ESBMC), so this is cheap enough to run in CI.

The baselines are an upper bound the count must not exceed, not a target — lower
them whenever real hardening or the tag-family refactor (#81 PR-E) reduces the
weak count. They are intentionally not auto-derived: a human lowering the number
is the ratchet.

Usage:
    build/vowc contracts compiler/main.vow | scripts/check_contract_quality.py
"""

import json
import sys

# The count must not EXCEED these. Ratchet DOWN as contracts harden; never up.
WEAK_MAX = 408
TAUTOLOGICAL_MAX = 0

try:
    data = json.load(sys.stdin)
except json.JSONDecodeError as exc:
    print(f"check_contract_quality: invalid `vow contracts` JSON: {exc}", file=sys.stderr)
    sys.exit(2)

quality = data.get("summary", {}).get("quality")
if quality is None:
    print("check_contract_quality: missing summary.quality", file=sys.stderr)
    sys.exit(2)

# Fail closed: the two counters the ratchet compares are required by the
# contracts-result schema. Defaulting an absent counter to 0 would let a broken
# or mis-shaped `vow contracts` output sail through the gate (0 never exceeds a
# baseline), so demand the keys explicitly and demand they are integers.
missing = [k for k in ("weak", "tautological") if k not in quality]
if missing:
    print(
        "check_contract_quality: summary.quality missing required counter(s): "
        f"{', '.join(missing)} — refusing to evaluate (fail closed)",
        file=sys.stderr,
    )
    sys.exit(2)

weak = quality["weak"]
tautological = quality["tautological"]
if not isinstance(weak, int) or not isinstance(tautological, int):
    print(
        "check_contract_quality: summary.quality counters weak/tautological must "
        "be integers — refusing to evaluate (fail closed)",
        file=sys.stderr,
    )
    sys.exit(2)

substantive = quality.get("substantive", 0)
total = data.get("summary", {}).get("total", 0)

print(
    f"contract quality: weak={weak} (max {WEAK_MAX}), "
    f"tautological={tautological} (max {TAUTOLOGICAL_MAX}), "
    f"substantive={substantive}, total={total}"
)

failed = False
if weak > WEAK_MAX:
    print(
        f"FAIL: weak contracts {weak} exceeds baseline {WEAK_MAX} — "
        f"a new `ensures` only bounds result by a constant. Strengthen it or, if "
        f"intentional, raise the baseline with justification.",
        file=sys.stderr,
    )
    failed = True
if tautological > TAUTOLOGICAL_MAX:
    print(
        f"FAIL: tautological contracts {tautological} exceeds baseline "
        f"{TAUTOLOGICAL_MAX} — a clause says nothing about the program.",
        file=sys.stderr,
    )
    failed = True

sys.exit(1 if failed else 0)
