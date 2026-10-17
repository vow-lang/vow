#!/usr/bin/env python3
"""Spec-drift gate: docs/spec must name what the self-hosted driver accepts.

Three checks, stdlib only, no compiler needed:

* every flag a command's parser accepts (compiler/cli_flags.vow) is a row of
  that command's option table in docs/spec/cli.md;
* the contract-clause status vocabulary agrees between the contracts-result
  schema and the contracts `Status Values` table;
* every Skipped reason code the native verifier can emit
  (compiler/vc_ops.vow::vc_skip_code_valid) is a row of the errors.md
  `VerificationSkipped` table, the one canonical list;
* that code list equals the closed list of ADR-2026-10-08-1430 section 4, and
  every code is covered by a tests/verify-native/skip fixture with a matching
  `// TEST: skip-reason` directive, or by an explicit unit-test exemption.
"""

import json
import re
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "scripts"))
from generate_help import extract_table_col  # noqa: E402

SPEC = REPO_ROOT / "docs" / "spec"
CLI_FLAGS = REPO_ROOT / "compiler" / "cli_flags.vow"
VC_OPS = REPO_ROOT / "compiler" / "vc_ops.vow"
ADR_1430 = (
    REPO_ROOT
    / "docs"
    / "adr"
    / "2026-10-08-1430-native-verifier-cli-and-status-surface.md"
)
SKIP_FIXTURES = REPO_ROOT / "tests" / "verify-native" / "skip"
# A code no source program can produce is covered by a unit test instead of a
# fixture; the entry names the test file that must still mention the code.
UNIT_COVERED_CODES = {
    "ir-non-dominating-read": REPO_ROOT / "compiler" / "tests" / "test_vc_gate.vow",
    "unmodeled-builtin": REPO_ROOT / "compiler" / "tests" / "test_vc_gate.vow",
}

GLOBAL_FLAGS = {"--help", "--human"}
INTERNAL_PREFIX = "--worker"
COMMANDS = {
    "build": "`vow build` (default)",
    "verify": "`vow verify`",
    "test": "`vow test`",
    "contracts": "`vow contracts`",
    "decl": "`vow decl`",
}


def _functions(src: str) -> dict[str, str]:
    heads = list(re.finditer(r"^fn (\w+)\(", src, re.M))
    out = {}
    for i, m in enumerate(heads):
        end = heads[i + 1].start() if i + 1 < len(heads) else len(src)
        out[m.group(1)] = src[m.end() : end]
    return out


def _accepted_flags(fns: dict[str, str], name: str, seen=None) -> set[str]:
    seen = seen if seen is not None else set()
    if name in seen or name not in fns:
        return set()
    seen.add(name)
    body = fns[name]
    flags = set(re.findall(r'"(--?[A-Za-z][\w-]*)"', body))
    for callee in re.findall(r"\b(cf_\w+)\(", body):
        flags |= _accepted_flags(fns, callee, seen)
    return flags


def _documented_flags(cli: str, heading: str) -> set[str]:
    flag = re.compile(r"(?<![\w-])(--?[A-Za-z][\w-]*)")
    cells = extract_table_col(cli, heading, heading_level=3)
    return {f for cell in cells for f in flag.findall(cell)}


class FlagTablesTest(unittest.TestCase):
    def test_every_accepted_flag_has_a_row_in_its_command_table(self):
        fns = _functions(CLI_FLAGS.read_text())
        cli = (SPEC / "cli.md").read_text()
        for command, heading in COMMANDS.items():
            accepted = {
                f
                for f in _accepted_flags(fns, f"cf_kind_{command}")
                if f not in GLOBAL_FLAGS and not f.startswith(INTERNAL_PREFIX)
            }
            documented = _documented_flags(cli, heading)
            self.assertTrue(accepted or command == "decl", command)
            self.assertFalse(
                accepted - documented,
                f"`vow {command}` accepts flags missing from docs/spec/cli.md: "
                f"{sorted(accepted - documented)}",
            )


class ContractStatusTest(unittest.TestCase):
    def test_schema_statuses_match_the_status_table(self):
        schema = json.loads(
            (SPEC / "schemas" / "contracts-result.schema.json").read_text()
        )
        enum = set(
            schema["properties"]["contracts"]["items"]["properties"]["status"]["enum"]
        )
        cli = (SPEC / "cli.md").read_text()
        contracts_json = cli[cli.index("## Contracts Output JSON") :]
        rows = set(extract_table_col(contracts_json, "Status Values", heading_level=3))
        self.assertEqual(enum, rows)


def _native_skip_codes() -> list[str]:
    src = VC_OPS.read_text()
    const = dict(
        re.findall(
            r'fn (VC_SKIP_\w+)\(\) -> String \{ String::from\("([^"]+)"\) \}', src
        )
    )
    dominance = (REPO_ROOT / "compiler" / "ir_dominance.vow").read_text()
    const["VC_SKIP_NON_DOMINATING_READ"] = re.search(
        r'fn SKIP_IR_NON_DOMINATING_READ\(\) -> String \{ String::from\("([^"]+)"\)',
        dominance,
    ).group(1)
    valid = _functions(src)["vc_skip_code_valid"]
    names = re.findall(r"\b(VC_SKIP_\w+)\(\)", valid)
    return [const[n] for n in names]


def _adr_skip_codes() -> list[str]:
    adr = ADR_1430.read_text()
    start = adr.index("### 4. Closed list of `Skipped` reasons")
    codes = []
    for line in adr[start:].splitlines()[1:]:
        m = re.match(r"\| `([a-z][a-z-]*)` \|", line)
        if m:
            codes.append(m.group(1))
        elif codes and not line.startswith("|"):
            break
    return codes


class SkipCodeTest(unittest.TestCase):
    def test_every_native_skip_code_is_in_the_errors_table(self):
        codes = _native_skip_codes()
        self.assertGreaterEqual(len(codes), 8)
        errors = (SPEC / "errors.md").read_text()
        documented = set(
            extract_table_col(errors, "VerificationSkipped", heading_level=3)
        )
        self.assertFalse(set(codes) - documented, sorted(set(codes) - documented))

    def test_native_code_list_matches_adr_1430_section_4(self):
        adr = _adr_skip_codes()
        self.assertGreaterEqual(len(adr), 8)
        self.assertEqual(len(adr), len(set(adr)))
        self.assertEqual(sorted(adr), sorted(_native_skip_codes()))

    def test_every_skip_code_has_a_fixture_or_a_unit_test(self):
        covered = {}
        for fixture in sorted(SKIP_FIXTURES.glob("*.vow")):
            m = re.search(r"^// TEST: skip-reason (\S+)$", fixture.read_text(), re.M)
            self.assertIsNotNone(m, f"{fixture.name}: no skip-reason directive")
            covered.setdefault(m.group(1), []).append(fixture.name)
        codes = set(_native_skip_codes())
        self.assertFalse(set(covered) - codes, sorted(set(covered) - codes))
        for code in sorted(codes):
            if code in UNIT_COVERED_CODES:
                self.assertNotIn(
                    code, covered, f"{code} now has a fixture; drop the exemption"
                )
                unit = UNIT_COVERED_CODES[code].read_text()
                self.assertIn(code, unit, f"{code} is exempt but its unit test is gone")
            else:
                self.assertIn(code, covered, f"no skip fixture for {code}")


if __name__ == "__main__":
    unittest.main()
