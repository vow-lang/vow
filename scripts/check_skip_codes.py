#!/usr/bin/env python3
"""Check the native verifier's `Skipped` reason codes against their sources.

The closed list lives in compiler/vc_ops.vow and in the table of section 4 of
ADR-2026-10-08-1430. Every code must be pinned by a `// TEST: skip-reason <code>`
fixture in tests/verify-native/skip/, or be in EXEMPT with the unit test that
covers it.

Usage:
    python3 scripts/check_skip_codes.py

Exits 0 when consistent, 1 with the differences listed.
"""

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
VC_OPS = REPO_ROOT / "compiler" / "vc_ops.vow"
IR_DOMINANCE = REPO_ROOT / "compiler" / "ir_dominance.vow"
ADR = (
    REPO_ROOT
    / "docs"
    / "adr"
    / "2026-10-08-1430-native-verifier-cli-and-status-surface.md"
)
FIXTURES = REPO_ROOT / "tests" / "verify-native" / "skip"

# No source program produces it (the lowerer is fixed, #1407); it is a defensive gate.
EXEMPT = {
    "ir-non-dominating-read": "compiler/tests/test_vc_gate.vow check_non_dominating_read"
}

_LITERAL = r'fn {name}\(\) -> String \{{ String::from\("([a-z-]+)"\) \}}'


def source_codes(vc_ops: str, ir_dominance: str) -> set[str]:
    codes = set()
    for m in re.finditer(r"fn (VC_SKIP_\w+)\(\) -> String \{ (.*?) \}", vc_ops):
        body = m.group(2)
        literal = re.fullmatch(r'String::from\("([a-z-]+)"\)', body)
        if literal:
            codes.add(literal.group(1))
            continue
        callee = re.fullmatch(r"(\w+)\(\)", body)
        target = (
            re.search(_LITERAL.format(name=callee.group(1)), ir_dominance)
            if callee
            else None
        )
        if not target:
            raise SystemExit(f"{m.group(1)}: cannot resolve the code from `{body}`")
        codes.add(target.group(1))
    return codes


def adr_codes(adr: str) -> set[str]:
    heading = "### 4. Closed list of `Skipped` reasons"
    if heading not in adr:
        raise SystemExit(f"ADR-1430 has no `{heading}` section")
    section = adr.split(heading, 1)[1].split("\n### ", 1)[0]
    return set(re.findall(r"^\| `([a-z-]+)` \|", section, re.MULTILINE))


def fixture_codes(fixtures: Path) -> set[str]:
    codes = set()
    for path in fixtures.glob("*.vow"):
        codes.update(
            re.findall(r"^// TEST: skip-reason (\S+)$", path.read_text(), re.MULTILINE)
        )
    return codes


def problems(src: set[str], adr: set[str], covered: set[str]) -> list[str]:
    out = []
    for code in sorted(src - adr):
        out.append(
            f"`{code}` is in compiler/vc_ops.vow but not in the ADR-1430 section 4 table"
        )
    for code in sorted(adr - src):
        out.append(
            f"`{code}` is in the ADR-1430 section 4 table but not in compiler/vc_ops.vow"
        )
    for code in sorted(src - covered - set(EXEMPT)):
        out.append(
            f"`{code}` has no tests/verify-native/skip/ fixture with `// TEST: skip-reason {code}`"
        )
    for code in sorted(covered - src):
        out.append(
            f"a tests/verify-native/skip/ fixture pins `{code}`, which is not in compiler/vc_ops.vow"
        )
    for code in sorted(set(EXEMPT) & covered):
        out.append(f"`{code}` now has a fixture; drop it from EXEMPT")
    for code in sorted(set(EXEMPT) - src):
        out.append(f"EXEMPT lists `{code}`, which is not an emitted code")
    return out


def main() -> int:
    found = problems(
        source_codes(VC_OPS.read_text(), IR_DOMINANCE.read_text()),
        adr_codes(ADR.read_text()),
        fixture_codes(FIXTURES),
    )
    for line in found:
        print(line, file=sys.stderr)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
