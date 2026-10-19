"""Operation catalogue generator tests."""

import tempfile
import unittest
from pathlib import Path

from opsgen_tests.common import REPO_ROOT, go
from opsgen_tests.fixtures_01 import PRINT_OPS
from opsgen_tests.fixtures_02 import (
    GRAMMAR_FIXTURE,
    KNOWN_OPS,
    ROUTES,
    _write_fixture_tree,
    _write_target_fixtures,
)


class ReturnTokensRenderTest(unittest.TestCase):
    def _op(self, return_token: str) -> dict:
        op = dict(PRINT_OPS[1])  # single i64 param, easiest to reuse
        op["name"] = f"op_{return_token}"
        op["runtime_symbol"] = f"__vow_op_{return_token}"
        op["return"] = return_token
        return op

    def test_i64_return_token_renders(self):
        op = self._op("i64")
        self.assertIn("Ty::I64", go.gen_rust_ir_block([op], []))
        cranelift = go.gen_cranelift_block([op], [])
        self.assertIn(
            "sig.returns.push(AbiParam::new(types::I64));\n            true", cranelift
        )
        self.assertIn("ITY_I64()", go.gen_vow_lower_block([op], []))

    def test_ptr_return_token_renders(self):
        op = self._op("ptr")
        self.assertIn("Ty::Ptr", go.gen_rust_ir_block([op], []))
        cranelift = go.gen_cranelift_block([op], [])
        self.assertIn(
            "sig.returns.push(AbiParam::new(types::I64));\n            true", cranelift
        )
        self.assertIn("ITY_PTR()", go.gen_vow_lower_block([op], []))

    def test_bool_return_token_renders_as_i64_in_cranelift(self):
        # stdin_ready is the one operation using this token: the Vow surface
        # type is bool, but Cranelift has no dedicated bool return type, so
        # the ABI must still push types::I64 -- never a narrower I8/bool slot.
        op = self._op("bool")
        self.assertIn("Ty::Bool", go.gen_rust_ir_block([op], []))
        cranelift = go.gen_cranelift_block([op], [])
        self.assertIn(
            "sig.returns.push(AbiParam::new(types::I64));\n            true", cranelift
        )
        self.assertIn("ITY_BOOL()", go.gen_vow_lower_block([op], []))


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
            "\n"
            "fn catalogue_builtin_result_tag(name: String) -> i64 {\n"
            "    return BRT_NONE();\n"
            "}\n"
            "// GENERATE:OPERATIONS:END"
        )
        self.assertEqual(go.gen_vow_lower_block(PRINT_OPS, []), expected)

    def test_i64_and_ptr_return_tokens_map_to_ity_const(self):
        i64_op = dict(PRINT_OPS[1])
        i64_op["return"] = "i64"
        ptr_op = dict(PRINT_OPS[1])
        ptr_op["return"] = "ptr"
        block = go.gen_vow_lower_block([i64_op, ptr_op], [])
        self.assertIn(
            f'if name == String::from("{i64_op["name"]}") {{ return ITY_I64(); }}',
            block,
        )
        self.assertIn(
            f'if name == String::from("{ptr_op["name"]}") {{ return ITY_PTR(); }}',
            block,
        )

    def _op_returning(self, doc_ret, name="op"):
        return {
            "name": name,
            "runtime_symbol": f"__vow_{name}",
            "params": [],
            "return": "ptr",
            "doc_signature": f"fn() -> {doc_ret}",
            "effects": "[]",
        }

    def test_string_and_vec_returns_are_tagged(self):
        string_block = go.gen_vow_lower_block([self._op_returning("String")], [])
        vec_block = go.gen_vow_lower_block([self._op_returning("Vec<String>")], [])
        self.assertIn(
            'if name == String::from("op") { return BRT_STRING(); }', string_block
        )
        self.assertIn('if name == String::from("op") { return BRT_VEC(); }', vec_block)

    def test_option_return_is_tagged_with_its_element_type(self):
        for elem in go.OPTION_ELEMS:
            block = go.gen_vow_lower_block([self._op_returning(f"Option<{elem}>")], [])
            ity = go.OPTION_ELEMS[elem]["ity_const"]
            self.assertIn(
                f'if name == String::from("op") {{ return {ity}; }}',
                block.split("fn catalogue_builtin_result_tag")[1],
                elem,
            )

    def test_scalar_return_is_untagged(self):
        block = go.gen_vow_lower_block([self._op_returning("i64")], [])
        self.assertEqual(
            block.split("fn catalogue_builtin_result_tag")[1].count("return BRT_"), 1
        )
        self.assertIn("return BRT_NONE();", block)


class GeneratorDeterminismTest(unittest.TestCase):
    def test_repeated_calls_are_byte_identical(self):
        for rel_path, gen_fn in go.TARGET_FILES:
            self.assertEqual(
                gen_fn(PRINT_OPS, ROUTES), gen_fn(PRINT_OPS, ROUTES), str(rel_path)
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

    def test_real_grammar_md_contains_known_ops(self):
        grammar_text = (REPO_ROOT / "docs" / "spec" / "grammar.md").read_text()
        table = go.extract_builtin_signatures_table(grammar_text)
        for op in KNOWN_OPS:
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
        mismatches = go.check_doc_facts(KNOWN_OPS, REPO_ROOT)
        self.assertEqual(mismatches, [])


class WriteAndCheckProjectionsTest(unittest.TestCase):
    def test_write_then_check_reports_clean(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            go.write_projections(PRINT_OPS, tmp, [])
            mismatches = go.check_projections(PRINT_OPS, tmp, [])
        self.assertEqual(mismatches, [])

    def test_write_actually_splices_generator_output(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            go.write_projections(PRINT_OPS, tmp, [])
            content = (tmp / "vow-ir/src/lower/mod.rs").read_text()
        self.assertIn("catalogue_builtin_to_runtime", content)
        self.assertIn("fn vow_static_builtin_to_runtime() {}", content)

    def test_hand_mutated_target_is_flagged_as_drift(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            go.write_projections(PRINT_OPS, tmp, [])
            mod_rs = tmp / "vow-ir/src/lower/mod.rs"
            mod_rs.write_text(
                mod_rs.read_text().replace("print_str", "print_str_TAMPERED")
            )
            mismatches = go.check_projections(PRINT_OPS, tmp, [])
        self.assertTrue(any("mod.rs" in m for m in mismatches))

    def test_missing_marker_in_target_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            (tmp / "compiler/lower.vow").write_text("no markers at all\n")
            with self.assertRaises(ValueError):
                go.check_projections(PRINT_OPS, tmp, [])

    def test_check_projections_reports_every_stale_target(self):
        # A single catalogue edit (e.g. hand-editing operations.json and
        # forgetting to regenerate) must invalidate every language
        # projection's check in one --check run, not just one of them.
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            go.write_projections(PRINT_OPS, tmp, [])
            edited_ops = [dict(op) for op in PRINT_OPS]
            edited_ops[0] = dict(edited_ops[0])
            edited_ops[0]["runtime_symbol"] = "__vow_string_print_v2"
            mismatches = go.check_projections(edited_ops, tmp, [])
        stale_targets = " ".join(mismatches)
        self.assertIn("mod.rs", stale_targets)
        self.assertIn("lower.vow", stale_targets)
