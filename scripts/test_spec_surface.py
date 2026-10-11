#!/usr/bin/env python3
"""Spec-drift gate: docs/spec must name what the self-hosted driver accepts.

Three checks, stdlib only, no compiler needed:

* every flag a command's parser accepts (compiler/cli_flags.vow) is a row of
  that command's option table in docs/spec/cli.md;
* the contract-clause status vocabulary agrees between the contracts-result
  schema and the contracts `Status Values` table;
* every Skipped reason code the native verifier can emit
  (compiler/vc_ops.vow::vc_skip_code_valid) is a row of the errors.md
  `VerificationSkipped` table, the one canonical list.
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


class SkipCodeTest(unittest.TestCase):
    def test_every_native_skip_code_is_in_the_errors_table(self):
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
        self.assertGreaterEqual(len(names), 8)
        codes = [const[n] for n in names]
        errors = (SPEC / "errors.md").read_text()
        documented = set(
            extract_table_col(errors, "VerificationSkipped", heading_level=3)
        )
        self.assertFalse(set(codes) - documented, sorted(set(codes) - documented))


if __name__ == "__main__":
    unittest.main()
