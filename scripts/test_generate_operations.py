#!/usr/bin/env python3
"""Behavior tests for scripts/generate_operations.py.

The generator is the Operation Catalogue's single writer/checker for the
runtime-symbol/ABI/return-shape/doc projections it splices into both
compilers. These tests exercise its functions directly (the same seam
scripts/test_check_help_coverage.py uses for generate_help.py's siblings),
plus the real docs/spec/operations.json catalogue for the end-to-end cases.
"""

import json
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

sys.path.insert(0, str(REPO_ROOT / "scripts"))
import generate_operations as go  # noqa: E402


PRINT_OPS = [
    {
        "name": "print_str",
        "runtime_symbol": "__vow_string_print",
        "params": ["ptr"],
        "return": "unit",
        "doc_signature": "fn(s: String) -> ()",
        "effects": "[io]",
    },
    {
        "name": "print_i64",
        "runtime_symbol": "__vow_print_i64",
        "params": ["i64"],
        "return": "unit",
        "doc_signature": "fn(v: i64) -> ()",
        "effects": "[io]",
    },
    {
        "name": "print_u64",
        "runtime_symbol": "__vow_print_u64",
        "params": ["u64"],
        "return": "unit",
        "doc_signature": "fn(v: u64) -> ()",
        "effects": "[io]",
    },
]


class LoadCatalogueTest(unittest.TestCase):
    def _write(self, tmp_path: Path, ops: list) -> None:
        spec_dir = tmp_path / "docs" / "spec"
        spec_dir.mkdir(parents=True, exist_ok=True)
        (spec_dir / "operations.json").write_text(json.dumps({"operations": ops}))

    def test_real_catalogue_loads_and_matches_print_ops(self):
        ops = go.load_catalogue(REPO_ROOT)
        self.assertEqual(ops, PRINT_OPS)

    def test_missing_required_field_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            del bad["runtime_symbol"]
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("runtime_symbol", str(ctx.exception))
            self.assertIn("print_str", str(ctx.exception))

    def test_duplicate_name_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            dup = dict(PRINT_OPS[0])
            self._write(tmp, [PRINT_OPS[0], dup])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("print_str", str(ctx.exception))
            self.assertIn("duplicate", str(ctx.exception).lower())

    def test_unknown_return_token_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["return"] = "nonsense"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("nonsense", str(ctx.exception))

    def test_unknown_param_token_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["params"] = ["nonsense"]
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("nonsense", str(ctx.exception))

    def test_operations_not_a_list_raises_value_error(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            (tmp / "docs" / "spec").mkdir(parents=True)
            (tmp / "docs" / "spec" / "operations.json").write_text(
                json.dumps({"not_operations": []})
            )
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("operations", str(ctx.exception))

    def test_params_not_a_list_raises_value_error(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["params"] = "ptr"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("print_str", str(ctx.exception))
            self.assertIn("params", str(ctx.exception))


class GenRustIrBlockTest(unittest.TestCase):
    def test_matches_expected_rustfmt_canonical_text(self):
        expected = (
            "// GENERATE:OPERATIONS:START\n"
            "fn catalogue_builtin_to_runtime(name: &str) -> Option<(&'static str, Ty)> {\n"
            "    match name {\n"
            '        "print_str" => Some(("__vow_string_print", Ty::Unit)),\n'
            '        "print_i64" => Some(("__vow_print_i64", Ty::Unit)),\n'
            '        "print_u64" => Some(("__vow_print_u64", Ty::Unit)),\n'
            "        _ => None,\n"
            "    }\n"
            "}\n"
            "// GENERATE:OPERATIONS:END"
        )
        self.assertEqual(go.gen_rust_ir_block(PRINT_OPS), expected)


class GenCraneliftBlockTest(unittest.TestCase):
    def test_matches_expected_rustfmt_canonical_text(self):
        expected = (
            "// GENERATE:OPERATIONS:START\n"
            "fn catalogue_extern_sig(sym: &str, sig: &mut Signature) -> bool {\n"
            "    match sym {\n"
            '        "__vow_string_print" => {\n'
            "            sig.params.push(AbiParam::new(types::I64));\n"
            "            true\n"
            "        }\n"
            '        "__vow_print_i64" => {\n'
            "            sig.params.push(AbiParam::new(types::I64));\n"
            "            true\n"
            "        }\n"
            '        "__vow_print_u64" => {\n'
            "            sig.params.push(AbiParam::new(types::I64));\n"
            "            true\n"
            "        }\n"
            "        _ => false,\n"
            "    }\n"
            "}\n"
            "// GENERATE:OPERATIONS:END"
        )
        self.assertEqual(go.gen_cranelift_block(PRINT_OPS), expected)

    def test_non_unit_return_token_pushes_return_slot(self):
        # RETURN_TOKENS only defines "unit" today (clif_ret=None, no push).
        # Exercise the clif_ret plumbing itself so a future non-unit token
        # can't silently regress back to dropping the Cranelift return slot.
        go.RETURN_TOKENS["fake_i64"] = {
            "rust_ty": "Ty::I64",
            "ity_const": "ITY_I64()",
            "clif_ret": "types::I64",
        }
        try:
            op = dict(PRINT_OPS[1])
            op["return"] = "fake_i64"
            block = go.gen_cranelift_block([op])
        finally:
            del go.RETURN_TOKENS["fake_i64"]
        self.assertIn(
            "sig.returns.push(AbiParam::new(types::I64));\n            true", block
        )


class GenVowLowerBlockTest(unittest.TestCase):
    def test_matches_expected_text(self):
        expected = (
            "// GENERATE:OPERATIONS:START\n"
            "fn catalogue_builtin_to_extern(name: String) -> String {\n"
            '    if name == String::from("print_str") { return String::from("__vow_string_print"); }\n'
            '    if name == String::from("print_i64") { return String::from("__vow_print_i64"); }\n'
            '    if name == String::from("print_u64") { return String::from("__vow_print_u64"); }\n'
            '    return String::from("");\n'
            "}\n"
            "\n"
            "fn catalogue_builtin_ret_ty(name: String) -> i64 {\n"
            '    if name == String::from("print_str") { return ITY_UNIT(); }\n'
            '    if name == String::from("print_i64") { return ITY_UNIT(); }\n'
            '    if name == String::from("print_u64") { return ITY_UNIT(); }\n'
            "    return -1;\n"
            "}\n"
            "// GENERATE:OPERATIONS:END"
        )
        self.assertEqual(go.gen_vow_lower_block(PRINT_OPS), expected)


class GeneratorDeterminismTest(unittest.TestCase):
    def test_repeated_calls_are_byte_identical(self):
        self.assertEqual(
            go.gen_rust_ir_block(PRINT_OPS), go.gen_rust_ir_block(PRINT_OPS)
        )
        self.assertEqual(
            go.gen_cranelift_block(PRINT_OPS), go.gen_cranelift_block(PRINT_OPS)
        )
        self.assertEqual(
            go.gen_vow_lower_block(PRINT_OPS), go.gen_vow_lower_block(PRINT_OPS)
        )


class ReplaceBetweenMarkersTest(unittest.TestCase):
    def test_round_trip_preserves_surrounding_text(self):
        content = (
            "before\n"
            "// GENERATE:OPERATIONS:START\n"
            "old stuff\n"
            "// GENERATE:OPERATIONS:END\n"
            "after\n"
        )
        replacement = (
            "// GENERATE:OPERATIONS:START\nnew stuff\n// GENERATE:OPERATIONS:END"
        )
        result = go._replace_between_markers(
            content, go.MARKER_START, go.MARKER_END, replacement
        )
        self.assertEqual(
            result,
            "before\n// GENERATE:OPERATIONS:START\nnew stuff\n// GENERATE:OPERATIONS:END\nafter\n",
        )

    def test_missing_start_marker_raises(self):
        with self.assertRaises(ValueError) as ctx:
            go._replace_between_markers(
                "no markers here", go.MARKER_START, go.MARKER_END, "x"
            )
        self.assertIn(go.MARKER_START, str(ctx.exception))

    def test_missing_end_marker_raises(self):
        with self.assertRaises(ValueError) as ctx:
            go._replace_between_markers(
                f"{go.MARKER_START}\nno end", go.MARKER_START, go.MARKER_END, "x"
            )
        self.assertIn(go.MARKER_END, str(ctx.exception))

    def test_second_orphaned_marker_block_raises(self):
        # A bad merge leaving two marker pairs in one file must not silently
        # splice only the first and leave the second invisible to --check.
        content = (
            "before\n"
            "// GENERATE:OPERATIONS:START\nold 1\n// GENERATE:OPERATIONS:END\n"
            "middle\n"
            "// GENERATE:OPERATIONS:START\nold 2 (orphaned)\n// GENERATE:OPERATIONS:END\n"
            "after\n"
        )
        with self.assertRaises(ValueError) as ctx:
            go._replace_between_markers(
                content, go.MARKER_START, go.MARKER_END, "new stuff"
            )
        self.assertIn(go.MARKER_START, str(ctx.exception))


GRAMMAR_FIXTURE = """\
### Builtin Function Signatures

#### FFI Wrapper Intrinsics

| Function | Signature | Effects |
|---|---|---|
| `pin_to_root` | `fn(value: String) -> String` | `[]` |

#### Print / IO

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `print_str`      | `fn(s: String) -> ()`                      | `[io]`     |
| `print_i64`      | `fn(v: i64) -> ()`                         | `[io]`     |
| `print_u64`      | `fn(v: u64) -> ()`                         | `[io]`     |
| `eprintln_str`   | `fn(s: String) -> ()`                      | `[io]`     |

## Canonical Form

irrelevant trailing section.
"""


class SplitTableRowTest(unittest.TestCase):
    def test_strips_only_outer_pipe_cells(self):
        self.assertEqual(go._split_table_row("| foo | bar |"), ["foo", "bar"])

    def test_preserves_genuinely_empty_interior_cell(self):
        # A blank (unformatted) interior cell must survive splitting --
        # only the two empty strings produced by the row's own leading and
        # trailing "|" are dropped, never an interior column.
        self.assertEqual(go._split_table_row("| foo |  | bar |"), ["foo", "", "bar"])

    def test_escaped_pipe_is_not_a_column_separator(self):
        self.assertEqual(go._split_table_row(r"| a\|b | c |"), ["a|b", "c"])


class ExtractBuiltinSignaturesTableTest(unittest.TestCase):
    def test_sweeps_every_subsection(self):
        table = go.extract_builtin_signatures_table(GRAMMAR_FIXTURE)
        self.assertEqual(table["pin_to_root"], ("fn(value: String) -> String", "[]"))
        self.assertEqual(table["print_str"], ("fn(s: String) -> ()", "[io]"))
        self.assertEqual(table["print_i64"], ("fn(v: i64) -> ()", "[io]"))
        self.assertEqual(table["print_u64"], ("fn(v: u64) -> ()", "[io]"))

    def test_stops_at_next_level_2_heading(self):
        table = go.extract_builtin_signatures_table(GRAMMAR_FIXTURE)
        self.assertNotIn("irrelevant", " ".join(table.keys()))

    def test_real_grammar_md_contains_print_ops(self):
        grammar_text = (REPO_ROOT / "docs" / "spec" / "grammar.md").read_text()
        table = go.extract_builtin_signatures_table(grammar_text)
        for op in PRINT_OPS:
            self.assertEqual(table[op["name"]], (op["doc_signature"], op["effects"]))

    def test_duplicate_row_across_subsections_raises(self):
        # A stale second row for the same builtin (e.g. left in another
        # subsection by a bad merge) must not be silently overwritten.
        duplicated = GRAMMAR_FIXTURE.replace(
            "#### Print / IO",
            "#### Debug\n\n"
            "| Function    | Signature                 | Effects |\n"
            "|---|---|---|\n"
            "| `print_str` | `fn(s: String) -> BOGUS`  | `[STALE]` |\n\n"
            "#### Print / IO",
        )
        with self.assertRaises(ValueError) as ctx:
            go.extract_builtin_signatures_table(duplicated)
        self.assertIn("print_str", str(ctx.exception))


def _write_fixture_tree(
    tmp: Path, *, grammar_effects="[io]", main_vow_ok=True, skill_rs_ok=True
) -> None:
    (tmp / "docs" / "spec").mkdir(parents=True)
    (tmp / "docs" / "spec" / "grammar.md").write_text(
        GRAMMAR_FIXTURE.replace(
            "| `print_str`      | `fn(s: String) -> ()`                      | `[io]`     |",
            f"| `print_str`      | `fn(s: String) -> ()`                      | `{grammar_effects}`     |",
        )
    )
    (tmp / "compiler").mkdir(parents=True)
    main_vow_lines = [
        '    r.push_str(String::from("      \\"print_str\\": \\"fn(s: String) -> () [io]\\",\\n"));\n'
        if main_vow_ok
        else "    // no print_str doc line here\n",
        '    r.push_str(String::from("      \\"print_i64\\": \\"fn(v: i64) -> () [io]\\",\\n"));\n',
        '    r.push_str(String::from("      \\"print_u64\\": \\"fn(v: u64) -> () [io]\\",\\n"));\n',
    ]
    (tmp / "compiler" / "main.vow").write_text("".join(main_vow_lines))
    (tmp / "vow" / "src").mkdir(parents=True)
    skill_rs_lines = [
        '"print_str": "fn(s: String) -> () [io]",\n'
        if skill_rs_ok
        else "no doc line\n",
        '"print_i64": "fn(v: i64) -> () [io]",\n',
        '"print_u64": "fn(v: u64) -> () [io]",\n',
    ]
    (tmp / "vow" / "src" / "skill.rs").write_text("".join(skill_rs_lines))


class CheckDocFactsTest(unittest.TestCase):
    def test_clean_fixture_reports_no_mismatches(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_fixture_tree(tmp)
            mismatches = go.check_doc_facts(PRINT_OPS, tmp)
        self.assertEqual(mismatches, [])

    def test_grammar_effects_mismatch_is_caught(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_fixture_tree(tmp, grammar_effects="[]")
            mismatches = go.check_doc_facts(PRINT_OPS, tmp)
        matching = [m for m in mismatches if "print_str" in m and "grammar.md" in m]
        self.assertTrue(matching)
        # The mismatch message must carry the *observed* (wrong) value, not
        # just the fact that something disagreed -- otherwise this test would
        # pass even if check_doc_facts never actually read the table.
        self.assertIn("[]", matching[0])

    def test_grammar_missing_row_is_caught(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_fixture_tree(tmp)
            grammar_path = tmp / "docs" / "spec" / "grammar.md"
            lines = grammar_path.read_text().splitlines(keepends=True)
            grammar_path.write_text(
                "".join(line for line in lines if "print_str" not in line)
            )
            mismatches = go.check_doc_facts(PRINT_OPS, tmp)
        self.assertIn("grammar.md: no row found for 'print_str'", mismatches)

    def test_main_vow_missing_doc_line_is_caught(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_fixture_tree(tmp, main_vow_ok=False)
            mismatches = go.check_doc_facts(PRINT_OPS, tmp)
        self.assertTrue(any("print_str" in m and "main.vow" in m for m in mismatches))

    def test_skill_rs_missing_doc_line_is_caught(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_fixture_tree(tmp, skill_rs_ok=False)
            mismatches = go.check_doc_facts(PRINT_OPS, tmp)
        self.assertTrue(any("print_str" in m and "skill.rs" in m for m in mismatches))

    def test_real_repo_has_no_mismatches(self):
        mismatches = go.check_doc_facts(PRINT_OPS, REPO_ROOT)
        self.assertEqual(mismatches, [])


def _write_target_fixtures(tmp: Path) -> None:
    for rel, header in [
        ("vow-ir/src/lower/mod.rs", "fn vow_static_builtin_to_runtime() {}\n"),
        ("vow-codegen/src/cranelift_backend.rs", "fn make_extern_sig() {}\n"),
        ("vow-clif-shim/src/lib.rs", "fn make_extern_sig() {}\n"),
        ("compiler/lower.vow", "fn builtin_to_extern() {}\n"),
    ]:
        path = tmp / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            f"{header}// GENERATE:OPERATIONS:START\nplaceholder\n// GENERATE:OPERATIONS:END\n"
        )


class WriteAndCheckProjectionsTest(unittest.TestCase):
    def test_write_then_check_reports_clean(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            go.write_projections(PRINT_OPS, tmp)
            mismatches = go.check_projections(PRINT_OPS, tmp)
        self.assertEqual(mismatches, [])

    def test_write_actually_splices_generator_output(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            go.write_projections(PRINT_OPS, tmp)
            content = (tmp / "vow-ir/src/lower/mod.rs").read_text()
        self.assertIn("catalogue_builtin_to_runtime", content)
        self.assertIn("fn vow_static_builtin_to_runtime() {}", content)

    def test_hand_mutated_target_is_flagged_as_drift(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            go.write_projections(PRINT_OPS, tmp)
            mod_rs = tmp / "vow-ir/src/lower/mod.rs"
            mod_rs.write_text(
                mod_rs.read_text().replace("print_str", "print_str_TAMPERED")
            )
            mismatches = go.check_projections(PRINT_OPS, tmp)
        self.assertTrue(any("mod.rs" in m for m in mismatches))

    def test_missing_marker_in_target_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            (tmp / "compiler/lower.vow").write_text("no markers at all\n")
            with self.assertRaises(ValueError):
                go.check_projections(PRINT_OPS, tmp)


if __name__ == "__main__":
    unittest.main()
