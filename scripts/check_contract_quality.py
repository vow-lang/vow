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
# 408 -> 11 once #81 PR-E removed the meaningless `ensures result >= 0` from the
# tag-constant families (IOP_*, ITY_*, EXPR_*, …). The remaining 11 are real
# parametric functions (region/span bit-packers and friends) that want a proper
# round-trip or enumerated postcondition — ratchet down as those are hardened.
WEAK_MAX = 11
TAUTOLOGICAL_MAX = 0

try:
    data = json.load(sys.stdin)
except json.JSONDecodeError as exc:
    print(f"check_contract_quality: invalid `vow contracts` JSON: {exc}", file=sys.stderr)
    sys.exit(2)

summary = data.get("summary")
if not isinstance(summary, dict):
    print("check_contract_quality: missing summary object", file=sys.stderr)
    sys.exit(2)

quality = summary.get("quality")
if not isinstance(quality, dict):
    print("check_contract_quality: missing summary.quality", file=sys.stderr)
    sys.exit(2)

# Fail closed: every counter the contracts-result schema requires must be present
# and a real integer. Defaulting an absent counter to 0 would let a broken or
# mis-shaped `vow contracts` output sail through the gate (0 never exceeds a
# baseline); bool is an int subclass, so reject it too (True == 1 would slip past).
required_int_fields = (
    ("summary.quality.weak", quality, "weak"),
    ("summary.quality.tautological", quality, "tautological"),
    ("summary.quality.substantive", quality, "substantive"),
    ("summary.total", summary, "total"),
)
for label, container, key in required_int_fields:
    if key not in container:
        print(
            f"check_contract_quality: missing {label} — refusing to evaluate (fail closed)",
            file=sys.stderr,
        )
        sys.exit(2)
    if isinstance(container[key], bool) or not isinstance(container[key], int):
        print(
            f"check_contract_quality: {label} must be an integer — refusing to evaluate (fail closed)",
            file=sys.stderr,
        )
        sys.exit(2)

weak = quality["weak"]
tautological = quality["tautological"]
substantive = quality["substantive"]
total = summary["total"]

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
