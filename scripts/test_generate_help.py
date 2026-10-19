#!/usr/bin/env python3
"""Behavior tests for scripts/generate_help.py.

The `-o, --output` default reported by `--help` must come from the
docs/spec/cli.md option-table row, not from a string hardcoded in the
generator, so the help text cannot drift from the spec.
"""

import json
import re
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SPEC = REPO_ROOT / "docs" / "spec"

sys.path.insert(0, str(REPO_ROOT / "scripts"))
import generate_help  # noqa: E402
from helpgen import shards  # noqa: E402


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


def _raw_strings(source: str) -> list[str]:
    return [m.group(2) for m in re.finditer(r'r(#+)"(.*?)"\1', source, re.S)]


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

    def test_help_module_round_trips(self):
        json_str = (
            '{\n  "print_str": "fn(s: String) -> () [io]",\n  "k": "a \u2014 b"\n}'
        )
        human_str = "\n".join(f"human {i}" for i in range(1000))
        files = generate_help.vow_help_files(json_str, human_str)
        self.assertEqual(list(files), ["compiler/help_text_gen.vow"])
        out = files["compiler/help_text_gen.vow"]
        self.assertTrue(out.startswith("// GENERATED by scripts/generate_help.py"))
        self.assertIn("module HelpTextGen\n", out)
        bodies = _function_bodies(out)
        self.assertEqual("".join(_literals(bodies["skill_json"])), _expected(json_str))
        self.assertEqual(
            "".join(_literals(bodies["skill_human"])), _expected(human_str)
        )
        self.assertIn('\\"print_str\\": \\"fn(s: String) -> () [io]\\"', out)
        for marker in ("SKILL_JSON", "SKILL_HUMAN"):
            self.assertIn(f"// GENERATE:{marker}:START\n", out)
            self.assertIn(f"// GENERATE:{marker}:END\n", out)

    def test_skill_modules_round_trip_across_shards(self):
        entry = "entry\n" + "\n".join(f"e{i}" for i in range(900))
        bundle = "\n".join(f'b{i} "q" \u2014 {i}' for i in range(40_000))
        support = {"refs/a.md": "alpha\n\nbeta", "refs/b.md": "gamma " * 2000}
        files = generate_help.vow_skill_files(entry, bundle, support)
        bodies = {}
        for path, text in files.items():
            self.assertTrue(text.startswith("// GENERATED by scripts/"), path)
            self.assertLess(len(text.encode()), 450_000, path)
            bodies.update(_function_bodies(text))
        parts = sorted(n for n in bodies if n.startswith("skill_bundle_part_"))
        self.assertGreater(len(parts), 1)
        self.assertEqual(
            "".join("".join(_literals(bodies[n])) for n in parts), _expected(bundle)
        )
        for name in parts[1:]:
            self.assertIn(f"r.push_str({name}());", bodies["skill_bundle"])
        self.assertEqual(
            "".join(_literals(bodies["skill_entrypoint"])), _expected(entry)
        )
        fns = [
            n
            for n in bodies
            if n.startswith("skill_support_content_")
            and n != "skill_support_content_index_guard"
        ]
        self.assertEqual(len(fns), 2)
        texts = ["".join(_literals(bodies[n])) for n in fns]
        self.assertCountEqual(texts, [_expected(t) for t in support.values()])
        text_module = files["compiler/skill_text_gen.vow"]
        for module in (p[9:-4] for p in files if p != "compiler/skill_text_gen.vow"):
            self.assertIn(f"use {module}\n", text_module)

    def test_rust_fragments_round_trip(self):
        json_str = '{"k": "a \u2014 b", "q": "\\"x\\""}'
        human_str = "human\n" * 10
        helped = generate_help.rust_help_files(json_str, human_str)
        text = helped["vow/src/skill_gen/help_text.rs"]
        self.assertTrue(text.startswith("// GENERATED by scripts/generate_help.py"))
        self.assertEqual(_raw_strings(text), [json_str, human_str])

        entry = "entry \u2014 text"
        bundle = "\n".join(f'b{i} "#q"' for i in range(30_000))
        support = {"refs/a.md": 'alpha "## beta', "schemas/b.json": "{}\n"}
        files = generate_help.rust_skill_files(entry, bundle, support)
        parts = sorted(p for p in files if "/bundle_" in p)
        rejoined = "\n".join(raw for p in parts for raw in _raw_strings(files[p]))
        self.assertEqual(rejoined, bundle)
        docs = [
            raw
            for p in sorted(files)
            if "/docs_" in p
            for raw in _raw_strings(files[p])
        ]
        self.assertEqual(docs, list(support.values()))
        index = files["vow/src/skill_gen/skill_text.rs"]
        self.assertEqual(_raw_strings(index), [entry])
        for name in (p.rsplit("/", 1)[1] for p in files if p.endswith(("_01.rs"))):
            self.assertIn(f'include!("{name}");', index)
        self.assertIn('("schemas/b.json", SKILL_DOC_SCHEMAS_B_JSON),', index)
        part_count = sum(files[p].count("fn skill_bundle_part_") for p in parts)
        self.assertGreater(part_count, 1)
        self.assertEqual(index.count("out.push('\\n');"), part_count - 1)

    def test_shards_split_by_source_size(self):
        big = {f"refs/{i}.md": "word " * 30_000 for i in range(5)}
        files = generate_help.vow_skill_files("e", "b", big)
        docs = [p for p in files if "skill_docs_" in p]
        self.assertGreater(len(docs), 1)
        for path in docs:
            self.assertLessEqual(len(files[path].encode()), 260_000, path)

    def test_split_text_rejoins_exactly(self):
        text = "\n".join(f"line {i}" for i in range(5000))
        for limit in (50, 1000, 10**9):
            pieces = shards.split_text(text, limit)
            self.assertEqual("\n".join(pieces), text)
            if limit < 10**9:
                self.assertGreater(len(pieces), 1)
            for piece in pieces:
                self.assertLessEqual(len(piece.encode()), limit + len("line 4999"))

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


class VerificationSurfaceTest(unittest.TestCase):
    def test_esbmc_defaults_stay_and_native_defaults_are_added(self):
        defaults = _help_data()["verification_defaults"]
        self.assertEqual(defaults["strategy"], "incremental-bmc")
        self.assertEqual(defaults["max_k_step"], 50)
        native = defaults["native"]
        self.assertIn("k-induction", native["strategy"])
        self.assertEqual(native["tuning_flags"], ["--timeout"])

    def test_human_help_labels_defaults_per_backend(self):
        human = generate_help.build_help_human(_help_data())
        block = human[human.index("VERIFICATION DEFAULTS") :]
        self.assertIn("ESBMC backend", block)
        self.assertIn("Native backend", block)

    def test_no_text_calls_every_counterexample_an_esbmc_one(self):
        data = _help_data()
        for text in (
            json.dumps(data),
            generate_help.build_help_human(data),
            generate_help.SKILL_FRONTMATTER,
        ):
            self.assertNotIn("ESBMC counterexample", text)

    def test_build_help_lists_backend_flag(self):
        options = _help_data()["command_details"]["build"]["options"]
        self.assertIn("--backend <esbmc|native>", [o["form"] for o in options])


class SkillDoNotsTest(unittest.TestCase):
    def _section(self) -> str:
        entry = generate_help.build_skill_entrypoint()
        start = entry.index("## Do nots")
        end = entry.index("\n## ", start + 1)
        return entry[start:end]

    def test_section_sits_between_intro_and_live_toolchain(self):
        entry = generate_help.build_skill_entrypoint()
        self.assertLess(entry.index("# Vow"), entry.index("## Do nots"))
        self.assertLess(
            entry.index("## Do nots"), entry.index("## Installed toolchain (live)")
        )

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

    def test_section_cannot_terminate_the_rust_raw_string(self):
        self.assertNotIn('"##', self._section())


if __name__ == "__main__":
    unittest.main()
