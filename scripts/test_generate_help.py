#!/usr/bin/env python3
"""Behavior tests for scripts/generate_help.py.

The `-o, --output` default reported by `--help` must come from the
docs/spec/cli.md option-table row, not from a string hardcoded in the
generator, so the help text cannot drift from the spec.
"""

import json
import re
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SPEC = REPO_ROOT / "docs" / "spec"

sys.path.insert(0, str(REPO_ROOT / "scripts"))
import generate_help  # noqa: E402


def _help_data(cli: str | None = None) -> dict:
    grammar = (SPEC / "grammar.md").read_text()
    cli = cli if cli is not None else (SPEC / "cli.md").read_text()
    contracts = (SPEC / "contracts.md").read_text()
    return generate_help.build_help_json(grammar, cli, contracts)


def _output_option(data: dict, command: str) -> dict:
    options = data["command_details"][command]["options"]
    matches = [o for o in options if o["form"] == "-o, --output <path>"]
    assert len(matches) == 1, matches
    return matches[0]


class OutputDefaultTest(unittest.TestCase):
    def test_build_output_default_is_build_stem(self):
        option = _output_option(_help_data(), "build")
        self.assertEqual(option["default"], "build/<stem>")
        self.assertTrue(option["description"].endswith("(default: build/<stem>)"))

    def test_build_output_default_tracks_cli_md(self):
        cli = (SPEC / "cli.md").read_text()
        row = "| `-o, --output`    | `build/<stem>` |"
        self.assertIn(row, cli)
        option = _output_option(
            _help_data(
                cli.replace(row, row.replace("build/<stem>", "out/<stem>.bin"), 1)
            ),
            "build",
        )
        self.assertEqual(option["default"], "out/<stem>.bin")
        self.assertTrue(option["description"].endswith("(default: out/<stem>.bin)"))

    def test_decl_output_default_is_source_vow_d(self):
        option = _output_option(_help_data(), "decl")
        self.assertEqual(option["default"], "<source>.vow.d")
        self.assertEqual(
            option["description"],
            "Output declaration file path (default: <source>.vow.d)",
        )


_LITERAL = re.compile(r'String::from\("((?:[^"\\]|\\.)*)"\)')
_ESCAPES = {"n": "\n", "\\": "\\", '"': '"'}


def _decode(escaped: str) -> str:
    return re.sub(r"\\(.)", lambda m: _ESCAPES[m.group(1)], escaped)


def _literals(source: str) -> list[str]:
    return [_decode(m.group(1)) for m in _LITERAL.finditer(source)]


def _function_bodies(source: str) -> dict[str, str]:
    bodies = {}
    for chunk in source.split("\nfn ")[1:]:
        name = chunk.split("(", 1)[0]
        bodies[name] = chunk
    return bodies


def _emit(text: str) -> str:
    return generate_help._vow_payload_fn("payload", text)


def _expected(text: str) -> str:
    return text.replace("\u2014", "--") + "\n"


class VowPayloadEmissionTest(unittest.TestCase):
    TEXTS = [
        "",
        "a",
        "a\n",
        "a\n\nb",
        'quote " and back\\slash',
        "dash \u2014 here\nsecond",
        "x" * 10_000,
        "\n".join(f'line {i} with "quotes" and \\' for i in range(100)),
    ]

    def test_round_trip_preserves_bytes(self):
        for text in self.TEXTS:
            with self.subTest(text=text[:30]):
                self.assertEqual("".join(_literals(_emit(text))), _expected(text))

    def test_chunks_respect_cap_and_never_split_lines(self):
        text = "\n".join(f"line {i} " + "y" * (i % 97) for i in range(2000))
        longest = max(len(line) for line in text.split("\n")) + 1
        limit = max(generate_help.VOW_LITERAL_CHUNK_BYTES, longest)
        literals = _literals(_emit(text))
        self.assertGreater(len(literals), 1)
        for lit in literals:
            self.assertLessEqual(len(lit.encode()), limit)
            self.assertTrue(lit.endswith("\n"))

    def test_overlong_line_is_its_own_chunk(self):
        long_line = "z" * (generate_help.VOW_LITERAL_CHUNK_BYTES * 2)
        literals = _literals(_emit(f"a\n{long_line}\nb"))
        self.assertIn(long_line + "\n", literals)

    def test_one_statement_per_source_line(self):
        lines = _emit("a\n" * 5000).split("\n")
        self.assertEqual(lines[0], "fn payload() -> String {")
        self.assertTrue(lines[1].startswith('    let r: String = String::from("'))
        self.assertEqual(lines[-2:], ["    r", "}"])
        for line in lines[2:-2]:
            self.assertTrue(line.startswith('    r.push_str(String::from("'), line)
            self.assertTrue(line.endswith('"));'), line)
        self.assertLess(len(lines), 100)

    def test_inject_vow_round_trips(self):
        json_str = (
            '{\n  "print_str": "fn(s: String) -> () [io]",\n  "k": "a \u2014 b"\n}'
        )
        human_str = "\n".join(f"human {i}" for i in range(1000))
        marked = (
            "// GENERATE:SKILL_JSON:START\nold\n// GENERATE:SKILL_JSON:END\n"
            "// GENERATE:SKILL_HUMAN:START\nold\n// GENERATE:SKILL_HUMAN:END\n"
        )
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "main.vow"
            path.write_text(marked)
            out = generate_help.inject_vow(path, json_str, human_str)
        bodies = _function_bodies(out)
        self.assertEqual("".join(_literals(bodies["skill_json"])), _expected(json_str))
        self.assertEqual(
            "".join(_literals(bodies["skill_human"])), _expected(human_str)
        )
        self.assertIn('\\"print_str\\": \\"fn(s: String) -> () [io]\\"', out)

    def test_inject_skill_vow_round_trips(self):
        entry = "entry\n" + "\n".join(f"e{i}" for i in range(900))
        bundle = "\n".join(f'b{i} "q"' for i in range(3000))
        support = {"refs/a.md": "alpha\n\nbeta", "refs/b.md": "gamma " * 2000}
        marked = "// GENERATE:SKILL_FULL:START\nold\n// GENERATE:SKILL_FULL:END\n"
        out = generate_help.inject_skill_vow(marked, entry, bundle, support)
        bodies = _function_bodies(out)
        self.assertEqual(
            "".join(_literals(bodies["skill_entrypoint"])), _expected(entry)
        )
        self.assertEqual("".join(_literals(bodies["skill_bundle"])), _expected(bundle))
        fns = [
            n
            for n in bodies
            if n.startswith("skill_support_content_")
            and n != "skill_support_content_index_guard"
        ]
        self.assertEqual(len(fns), 2)
        texts = ["".join(_literals(bodies[n])) for n in fns]
        self.assertCountEqual(texts, [_expected(t) for t in support.values()])

    def test_real_payload_chunks_never_exceed_longest_source_line(self):
        data = _help_data()
        texts = [
            json.dumps(data, indent=2),
            generate_help.build_help_human(data),
            generate_help.build_skill_entrypoint(),
            generate_help.build_skill_bundle(),
            *generate_help.build_skill_support_files().values(),
        ]
        statements = source_lines = 0
        for text in texts:
            expected = _expected(text)
            lines = expected.split("\n")
            limit = max(
                generate_help.VOW_LITERAL_CHUNK_BYTES,
                max(len(line.encode()) for line in lines) + 1,
            )
            lits = _literals(_emit(text))
            statements += len(lits)
            source_lines += len(lines)
            self.assertEqual("".join(lits), expected)
            for lit in lits:
                self.assertLessEqual(len(lit.encode()), limit)
        self.assertLess(statements, source_lines)


class SkillDoNotsTest(unittest.TestCase):
    def _section(self) -> str:
        entry = generate_help.build_skill_entrypoint()
        start = entry.index("## Do nots")
        end = entry.index("\n## ", start + 1)
        return entry[start:end]

    def test_section_sits_between_intro_and_live_toolchain(self):
        entry = generate_help.build_skill_entrypoint()
        self.assertIn("## Do nots", entry)
        self.assertLess(entry.index("# Vow"), entry.index("## Do nots"))
        self.assertLess(
            entry.index("## Do nots"), entry.index("## Installed toolchain (live)")
        )

    def test_section_covers_each_agent_bug_class(self):
        section = self._section()
        for needle in (
            "contract",
            "requires",
            "--no-verify",
            "Verified",
            "Unverified",
            "--help",
            "stdout",
            "--human",
        ):
            self.assertIn(needle, section)

    def test_every_rule_is_a_do_not_bullet(self):
        bullets = [ln for ln in self._section().splitlines() if ln.startswith("- ")]
        self.assertGreaterEqual(len(bullets), 5)
        for bullet in bullets:
            self.assertTrue(bullet.startswith("- Do not "), bullet)

    def test_section_names_only_flags_the_cli_spec_documents(self):
        cli = (SPEC / "cli.md").read_text()
        flags = set(re.findall(r"--[a-z][a-z-]*", self._section()))
        self.assertTrue(flags)
        for flag in flags:
            self.assertIn(flag, cli)

    def test_section_is_safe_for_generated_payloads(self):
        section = self._section()
        self.assertTrue(section.isascii())
        for forbidden in ('"', "\\", '"##'):
            self.assertNotIn(forbidden, section)
        self.assertEqual("".join(_literals(_emit(section))), _expected(section))


if __name__ == "__main__":
    unittest.main()
