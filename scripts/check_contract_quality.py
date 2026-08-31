#!/usr/bin/env python3
"""Ratchet gate on static contract quality across the self-hosted compiler (#81).

Reads the JSON of one `vow contracts <entry>` run on stdin and fails if the
`weak` or `tautological` contract count exceeds a committed baseline. This keeps
new hollow contracts (`ensures result >= 0` and friends) from creeping in. The
classification is static (no ESBMC), so this is cheap enough to run in CI.

The baselines are an upper bound the count must not exceed, not a target — lower
them whenever real hardening or the tag-family refactor (#81 PR-E) reduces the
weak count. They are intentionally not auto-derived: a human lowering the number
is the ratchet.

SCOPE — which entry points the gate reads, and why not the rest.

`vow contracts` follows `use` edges, so one entry point covers its whole module
graph but nothing outside it. `scripts/full_test.sh` therefore runs this checker
once per entry point, passing `--label` so a breach names the entry it came from:

  compiler/main.vow       the compiler's own `use` graph
  compiler/module_io.vow  deliberately-unwired .vmod parity infrastructure that
                          is NOT in main.vow's `use` graph, so it had zero
                          coverage until it was added here. It must not drift.

`tests/`, `stdlib/`, `examples/` and `benchmarks/` are deliberately NOT gated.
They are the corpus — the *input* to contract cleanup — and they legitimately
carry weak and tautological clauses today. A baseline aggregated over hundreds
of files is a number nobody can ratchet down meaningfully. Corpus-side cleanup
is enforced instead by the `TautologicalComparison` type-checker error, which is
a hard compile failure per site rather than a count, plus reviewer discipline.

Usage:
    build/vowc contracts compiler/main.vow \
        | scripts/check_contract_quality.py --label compiler/main.vow
"""

import argparse
import json
import sys

# The count must not EXCEED these. Ratchet DOWN as contracts harden; never up.
# 408 -> 11 once #81 PR-E removed the meaningless `ensures result >= 0` from the
# tag-constant families (IOP_*, ITY_*, EXPR_*, …). 11 -> 0 once the remaining
# parametric bit-packers (region_pack/kind/val, span_pack, item_kind,
# marker_caller_store, suffix_len) were hardened with exact
# functional / enumerated postconditions (#81). The baseline is now 0: no weak
# contract may enter the self-hosted compiler.
# The gate covers `compiler/main.vow` and `compiler/module_io.vow`; both measured
# 0 weak / 0 tautological when the second entry point was added, so the baseline
# stays 0 for every entry.
WEAK_MAX = 0
TAUTOLOGICAL_MAX = 0

parser = argparse.ArgumentParser(add_help=True)
parser.add_argument(
    "--label",
    default="",
    help="entry point this document came from, echoed in output so a breach "
    "names which entry point regressed",
)
args = parser.parse_args()
where = f" [{args.label}]" if args.label else ""

try:
    data = json.load(sys.stdin)
except json.JSONDecodeError as exc:
    print(
        f"check_contract_quality{where}: invalid `vow contracts` JSON: {exc}",
        file=sys.stderr,
    )
    sys.exit(2)

summary = data.get("summary")
if not isinstance(summary, dict):
    print(f"check_contract_quality{where}: missing summary object", file=sys.stderr)
    sys.exit(2)

quality = summary.get("quality")
if not isinstance(quality, dict):
    print(f"check_contract_quality{where}: missing summary.quality", file=sys.stderr)
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
            f"check_contract_quality{where}: missing {label} — refusing to evaluate (fail closed)",
            file=sys.stderr,
        )
        sys.exit(2)
    if isinstance(container[key], bool) or not isinstance(container[key], int):
        print(
            f"check_contract_quality{where}: {label} must be an integer — refusing to evaluate (fail closed)",
            file=sys.stderr,
        )
        sys.exit(2)

weak = quality["weak"]
tautological = quality["tautological"]
substantive = quality["substantive"]
total = summary["total"]

print(
    f"contract quality{where}: weak={weak} (max {WEAK_MAX}), "
    f"tautological={tautological} (max {TAUTOLOGICAL_MAX}), "
    f"substantive={substantive}, total={total}"
)

failed = False
if weak > WEAK_MAX:
    print(
        f"FAIL{where}: weak contracts {weak} exceeds baseline {WEAK_MAX} — "
        f"a new `ensures` only bounds result by a constant. Strengthen it or, if "
        f"intentional, raise the baseline with justification.",
        file=sys.stderr,
    )
    failed = True
if tautological > TAUTOLOGICAL_MAX:
    print(
        f"FAIL{where}: tautological contracts {tautological} exceeds baseline "
        f"{TAUTOLOGICAL_MAX} — a clause says nothing about the program.",
        file=sys.stderr,
    )
    failed = True

sys.exit(1 if failed else 0)
