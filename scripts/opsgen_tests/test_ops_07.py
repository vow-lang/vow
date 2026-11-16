"""Operation catalogue generator tests."""

import contextlib
import io
import re
import sys
import tempfile
import unittest
from pathlib import Path

from opsgen_tests.common import REPO_ROOT, go
from opsgen_tests.fixtures_01 import PRINT_OPS
from opsgen_tests.fixtures_02 import (
    _write_excluded_catalogue,
    _write_fixture_tree,
    _write_target_fixtures,
)
from opsgen_tests.fixtures_03 import EXCLUDED, _write_fact_fixtures


class OperationFactTablesTest(unittest.TestCase):
    def test_fact_targets_use_their_own_marker_pair(self):
        self.assertEqual(
            {str(p) for p, _ in go.FACT_TARGET_FILES},
            {"vow-types/src/env.rs", "vow-ir/src/lower/mod.rs"},
        )
        self.assertEqual(
            {str(p) for p, _, _ in go.FACT_WHOLE_TARGETS},
            {"compiler/tests/catalogue_facts_gen.vow"},
        )
        self.assertNotEqual(go.FACTS_MARKER_START, go.MARKER_START)
        self.assertNotEqual(go.FACTS_MARKER_END, go.MARKER_END)

    def test_generators_are_deterministic(self):
        for rel_path, gen_fn in go.FACT_TARGET_FILES:
            self.assertEqual(
                gen_fn(PRINT_OPS, EXCLUDED), gen_fn(PRINT_OPS, EXCLUDED), str(rel_path)
            )

    def test_rust_block_carries_every_fact_and_exclusion(self):
        block = go.gen_rust_facts_block(PRINT_OPS, EXCLUDED)
        self.assertIn(
            'CatalogueFact { name: "print_str", effects: "[io]", params: &["ptr"], '
            'ret: "unit", doc_params: &["String"], doc_ret: "()" },',
            block,
        )
        self.assertIn('ExcludedFact::Names(&["debug_str"]),', block)
        self.assertIn('ExcludedFact::Prefix("__vow_clif_"),', block)
        self.assertIn(
            'ExcludedFact::Contains { needle: "_to_", suffixes: &["_try", "_wrap"] },',
            block,
        )
        self.assertIn("#[rustfmt::skip]", block)
        self.assertTrue(block.startswith(go.FACTS_MARKER_START))
        self.assertTrue(block.endswith(go.FACTS_MARKER_END))

    def test_rust_block_indents_every_line_after_the_first(self):
        lines = go.gen_rust_facts_block(PRINT_OPS, EXCLUDED).splitlines()
        self.assertFalse(lines[0].startswith(" "))
        self.assertTrue(all(ln.startswith("    ") or not ln for ln in lines[1:]))

    def test_vow_block_carries_every_fact_and_exclusion(self):
        block = go.gen_vow_facts_block(PRINT_OPS, EXCLUDED)
        self.assertIn(
            'v.push(op_fact(String::from("print_str"), String::from("[io]"), '
            'String::from("ptr"), String::from("unit"), String::from("String"), '
            'String::from("()"), String::from("__vow_string_print"), '
            'String::from("none")));',
            block,
        )
        self.assertIn(
            'v.push(op_excluded(OPX_NAMES(), String::from("debug_str"), String::from("")));',
            block,
        )
        self.assertIn(
            'v.push(op_excluded(OPX_PREFIX(), String::from("__vow_clif_"), String::from("")));',
            block,
        )
        self.assertIn(
            'v.push(op_excluded(OPX_CONTAINS(), String::from("_to_"), '
            'String::from("_try,_wrap")));',
            block,
        )

    def test_multi_param_op_joins_tokens_and_empty_params_stay_empty(self):
        two = dict(PRINT_OPS[0], name="two", params=["ptr", "i64"])
        none = dict(PRINT_OPS[0], name="none", params=[])
        block = go.gen_vow_facts_block([two, none], [])
        self.assertIn('String::from("ptr,i64")', block)
        self.assertIn(
            'String::from("none"), String::from("[io]"), String::from("")', block
        )
        rust = go.gen_rust_facts_block([two, none], [])
        self.assertIn('params: &["ptr", "i64"]', rust)
        self.assertIn("params: &[]", rust)

    def test_doc_params_are_the_signature_parameter_types(self):
        join = dict(
            PRINT_OPS[0],
            name="join",
            params=["ptr", "ptr"],
            doc_signature="fn(parts: Vec<String>, sep: String) -> String",
        )
        self.assertEqual(go._doc_params(join), ["Vec<String>", "String"])
        self.assertEqual(go._doc_params(dict(join, doc_signature="fn() -> i64")), [])
        self.assertIn(
            'doc_params: &["Vec<String>", "String"]',
            go.gen_rust_facts_block([join], []),
        )
        self.assertIn(
            'String::from("Vec<String>|String")', go.gen_vow_facts_block([join], [])
        )

    def test_doc_return_is_the_signature_return_type(self):
        listing = dict(
            PRINT_OPS[0],
            doc_signature="fn(p: String) -> Vec<String>",
            **{"return": "ptr"},
        )
        self.assertIn('doc_ret: "Vec<String>"', go.gen_rust_facts_block([listing], []))

    def test_write_then_check_reports_clean(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_fact_fixtures(tmp)
            go.write_fact_tables(PRINT_OPS, tmp, EXCLUDED)
            self.assertEqual(go.check_fact_tables(PRINT_OPS, tmp, EXCLUDED), [])
            content = (tmp / "vow-types/src/env.rs").read_text()
        self.assertTrue(content.startswith("before\n"))
        self.assertTrue(content.endswith("after\n"))
        self.assertIn("const CATALOGUE_FACTS", content)

    def test_check_goes_stale_when_an_exclusion_changes(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_fact_fixtures(tmp)
            go.write_fact_tables(PRINT_OPS, tmp, EXCLUDED)
            changed = EXCLUDED + [{"names": ["debug_i64"], "reason": "r"}]
            mismatches = go.check_fact_tables(PRINT_OPS, tmp, changed)
        self.assertEqual(len(mismatches), 2)
        self.assertTrue(any("env.rs" in m for m in mismatches))
        self.assertTrue(any("catalogue_facts_gen.vow" in m for m in mismatches))

    def test_check_goes_stale_when_an_op_changes(self):
        # The lowering table carries only name, symbol, return and tag, so an
        # effects/params edit leaves vow-ir/src/lower/mod.rs fresh.
        for field, value, stale_files in [
            ("effects", "[read]", 2),
            ("params", ["i64"], 2),
            ("name", "print_renamed", 3),
        ]:
            with tempfile.TemporaryDirectory() as d:
                tmp = Path(d)
                _write_fact_fixtures(tmp)
                go.write_fact_tables(PRINT_OPS, tmp, EXCLUDED)
                edited = [dict(op) for op in PRINT_OPS]
                edited[0][field] = value
                mismatches = go.check_fact_tables(edited, tmp, EXCLUDED)
            self.assertEqual(len(mismatches), stale_files, field)

    def test_hand_edited_fact_table_is_flagged(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_fact_fixtures(tmp)
            go.write_fact_tables(PRINT_OPS, tmp, EXCLUDED)
            path = tmp / "compiler/tests/catalogue_facts_gen.vow"
            path.write_text(path.read_text().replace('"[io]"', '"[]"', 1))
            mismatches = go.check_fact_tables(PRINT_OPS, tmp, EXCLUDED)
        self.assertEqual(
            mismatches,
            ["compiler/tests/catalogue_facts_gen.vow: generated fact table is stale"],
        )

    def test_missing_marker_raises_instead_of_reporting_clean(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_fact_fixtures(tmp)
            (tmp / "vow-types/src/env.rs").write_text("no markers\n")
            with self.assertRaises(ValueError):
                go.check_fact_tables(PRINT_OPS, tmp, EXCLUDED)

    def test_fact_splice_leaves_an_operations_block_in_the_same_file_alone(self):
        content = (
            f"{go.MARKER_START}\nops\n{go.MARKER_END}\n"
            f"{go.FACTS_MARKER_START}\nold\n{go.FACTS_MARKER_END}\n"
        )
        spliced = go._replace_between_markers(
            content, go.FACTS_MARKER_START, go.FACTS_MARKER_END, "new"
        )
        self.assertIn(f"{go.MARKER_START}\nops\n{go.MARKER_END}", spliced)
        self.assertIn("new", spliced)
        self.assertNotIn("old", spliced)

    def test_result_tag_token_follows_the_documented_return_type(self):
        cases = {
            "fn(s: String) -> String": "string",
            "fn(s: String) -> Vec<String>": "vec",
            "fn(s: String) -> Option<u8>": "option:u8",
            "fn(s: String) -> i64": "none",
            "fn(s: String) -> ()": "none",
        }
        for signature, token in cases.items():
            op = dict(PRINT_OPS[0], doc_signature=signature)
            self.assertEqual(go.result_tag_token(op), token, signature)

    def test_lowering_block_carries_symbol_return_and_tag_for_every_op(self):
        listing = dict(
            PRINT_OPS[0],
            name="listing",
            runtime_symbol="__vow_listing",
            doc_signature="fn(s: String) -> Vec<String>",
        )
        listing["return"] = "ptr"
        block = go.gen_rust_lowering_facts_block(PRINT_OPS + [listing], EXCLUDED)
        self.assertIn(
            'LoweringFact { name: "print_str", symbol: "__vow_string_print", '
            'ret: "unit", tag: "none" },',
            block,
        )
        self.assertIn(
            'LoweringFact { name: "listing", symbol: "__vow_listing", '
            'ret: "ptr", tag: "vec" },',
            block,
        )
        self.assertIn("#[rustfmt::skip]", block)
        lines = block.splitlines()
        self.assertFalse(lines[0].startswith(" "))
        self.assertTrue(all(ln.startswith("    ") or not ln for ln in lines[1:]))

    def test_rust_and_self_hosted_lowering_facts_are_identical_for_every_op(self):
        rust_rows = re.findall(
            r'LoweringFact \{ name: "([^"]+)", symbol: "([^"]+)", '
            r'ret: "([^"]+)", tag: "([^"]+)" \}',
            (REPO_ROOT / "vow-ir/src/lower/mod.rs").read_text(),
        )
        vow_rows = re.findall(
            r'op_fact\(String::from\("([^"]+)"\), String::from\("[^"]*"\), '
            r'String::from\("[^"]*"\), String::from\("([^"]+)"\), '
            r'String::from\("[^"]*"\), String::from\("[^"]*"\), '
            r'String::from\("([^"]+)"\), String::from\("([^"]+)"\)\)\)',
            (REPO_ROOT / "compiler/tests/catalogue_facts_gen.vow").read_text(),
        )
        ops = go.load_catalogue(REPO_ROOT)
        want = [
            (op["name"], op["runtime_symbol"], op["return"], go.result_tag_token(op))
            for op in ops
        ]
        self.assertEqual(rust_rows, want)
        self.assertEqual(
            [(name, symbol, ret, tag) for name, ret, symbol, tag in vow_rows], want
        )

    def test_real_repo_fact_tables_are_fresh(self):
        ops = go.load_catalogue(REPO_ROOT)
        excluded = go.load_excluded(REPO_ROOT, ops)
        self.assertEqual(go.check_fact_tables(ops, REPO_ROOT, excluded), [])

    def _run_main(self, tmp: Path, *flags: str) -> tuple[str, int]:
        stdout = io.StringIO()
        old_argv = sys.argv
        sys.argv = ["generate_operations.py", *flags, "--repo-root", str(tmp)]
        code = 0
        try:
            with contextlib.redirect_stdout(stdout):
                go.main()
        except SystemExit as e:
            code = e.code
        finally:
            sys.argv = old_argv
        return stdout.getvalue(), code

    def test_main_writes_and_checks_fact_tables_end_to_end(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_fixture_tree(tmp)
            ops = [dict(op, verifier_model="unmodeled") for op in PRINT_OPS]
            _write_excluded_catalogue(tmp, EXCLUDED, ops)
            _write_target_fixtures(tmp)
            _write_fact_fixtures(tmp)

            written, code = self._run_main(tmp)
            self.assertEqual(code, 0)
            self.assertIn("Updated compiler/tests/catalogue_facts_gen.vow", written)
            checked, code = self._run_main(tmp, "--check")
            self.assertEqual((code, "OK:" in checked), (0, True), checked)

            facts = tmp / "compiler/tests/catalogue_facts_gen.vow"
            facts.write_text(facts.read_text().replace("print_u64", "print_u64_X"))
            stale, code = self._run_main(tmp, "--check")
        self.assertEqual(code, 1)
        self.assertIn(
            "compiler/tests/catalogue_facts_gen.vow: generated fact table is stale",
            stale,
        )


class CatalogueOutputsTest(unittest.TestCase):
    ANNOUNCED = [
        "vow-ir/src/lower/mod.rs",
        "vow-codegen/src/cranelift_backend.rs",
        "vow-clif-shim/src/lib.rs",
        "compiler/lower.vow",
        "vow-ir/src/region.rs",
        "vow-verify/src/c_emitter.rs",
        "compiler/c_emitter.vow",
        "vow-types/src/env.rs",
        "vow-ir/src/lower/mod.rs",
        "compiler/catalogue_ir_gen.vow",
        "compiler/catalogue_vc_ops_gen.vow",
        "compiler/tests/catalogue_facts_gen.vow",
    ]

    def _fixtures(self, root):
        _write_target_fixtures(root)
        _write_fact_fixtures(root)

    def _snapshot(self, root):
        return {
            p.relative_to(root).as_posix(): p.read_bytes()
            for p in root.rglob("*")
            if p.is_file()
        }

    def test_existing_families_preserve_shared_host_and_authored_text(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self._fixtures(root)
            go.write_projections(PRINT_OPS, root, [])
            go.write_fact_tables(PRINT_OPS, root, EXCLUDED)
            content = (root / "vow-ir/src/lower/mod.rs").read_text()
            self.assertTrue(
                content.startswith("fn vow_static_builtin_to_runtime() {}\n")
            )
            self.assertTrue(content.endswith("after\n"))
            self.assertIn("fn catalogue_builtin_to_runtime", content)
            self.assertIn("const LOWERING_FACTS", content)
            self.assertEqual(go.check_projections(PRINT_OPS, root, []), [])
            self.assertEqual(go.check_fact_tables(PRINT_OPS, root, EXCLUDED), [])

    def test_combined_write_matches_existing_bytes_order_and_idempotence(self):
        with tempfile.TemporaryDirectory() as d:
            legacy, combined = Path(d) / "legacy", Path(d) / "combined"
            self._fixtures(legacy)
            self._fixtures(combined)
            go.write_projections(PRINT_OPS, legacy, [])
            go.write_fact_tables(PRINT_OPS, legacy, EXCLUDED)
            paths = go.write_outputs(PRINT_OPS, combined, [], EXCLUDED)
            self.assertEqual([p.as_posix() for p in paths], self.ANNOUNCED)
            self.assertEqual(self._snapshot(combined), self._snapshot(legacy))
            self.assertEqual(go.check_outputs(PRINT_OPS, combined, [], EXCLUDED), [])
            go.write_outputs(PRINT_OPS, combined, [], EXCLUDED)
            self.assertEqual(self._snapshot(combined), self._snapshot(legacy))

    def test_check_reports_all_drift_in_execution_order_without_writing(self):
        expected = [f"{p}: spliced block is stale" for p in self.ANNOUNCED[:7]]
        expected += [f"{p}: generated file is stale" for p in self.ANNOUNCED[9:11]]
        expected += [f"{p}: spliced fact table is stale" for p in self.ANNOUNCED[7:9]]
        expected += [f"{self.ANNOUNCED[11]}: generated fact table is stale"]
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self._fixtures(root)
            before = self._snapshot(root)
            self.assertEqual(go.check_outputs(PRINT_OPS, root, [], EXCLUDED), expected)
            self.assertEqual(self._snapshot(root), before)
            self.assertFalse((root / "compiler/tests").exists())

    def test_missing_whole_file_is_reported_and_recreated(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self._fixtures(root)
            go.write_outputs(PRINT_OPS, root, [], EXCLUDED)
            path = root / "compiler/tests/catalogue_facts_gen.vow"
            before = path.read_bytes()
            path.unlink()
            path.parent.rmdir()
            self.assertEqual(
                go.check_outputs(PRINT_OPS, root, [], EXCLUDED),
                [
                    "compiler/tests/catalogue_facts_gen.vow: generated fact table is stale"
                ],
            )
            go.write_outputs(PRINT_OPS, root, [], EXCLUDED)
            self.assertEqual(path.read_bytes(), before)

    def test_bad_markers_raise_after_preserving_earlier_writes(self):
        corruptions = [
            ("no markers\n", "not found"),
            (go.MARKER_START + "\n", "not found"),
            (
                f"{go.MARKER_START}\n{go.MARKER_END}\n{go.MARKER_START}\n",
                "multiple",
            ),
        ]
        for content, message in corruptions:
            with self.subTest(content=content), tempfile.TemporaryDirectory() as d:
                root = Path(d)
                self._fixtures(root)
                (root / "compiler/lower.vow").write_text(content)
                facts_before = (root / "vow-types/src/env.rs").read_bytes()
                with self.assertRaisesRegex(ValueError, message):
                    go.write_outputs(PRINT_OPS, root, [], EXCLUDED)
                self.assertIn(
                    "catalogue_builtin_to_runtime",
                    (root / "vow-ir/src/lower/mod.rs").read_text(),
                )
                self.assertEqual(
                    (root / "vow-types/src/env.rs").read_bytes(), facts_before
                )
                self.assertFalse((root / "compiler/catalogue_ir_gen.vow").exists())
                before_check = self._snapshot(root)
                with self.assertRaisesRegex(ValueError, message):
                    go.check_outputs(PRINT_OPS, root, [], EXCLUDED)
                self.assertEqual(self._snapshot(root), before_check)

    def test_fact_failure_happens_after_runtime_whole_files_are_written(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self._fixtures(root)
            (root / "vow-types/src/env.rs").write_text("no markers\n")
            with self.assertRaisesRegex(ValueError, "OPERATION_FACTS:START.*not found"):
                go.write_outputs(PRINT_OPS, root, [], EXCLUDED)
            self.assertTrue((root / "compiler/catalogue_ir_gen.vow").exists())
            self.assertTrue((root / "compiler/catalogue_vc_ops_gen.vow").exists())
            self.assertFalse((root / "compiler/tests/catalogue_facts_gen.vow").exists())

    def test_missing_splice_host_raises_in_both_modes(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self._fixtures(root)
            (root / self.ANNOUNCED[0]).unlink()
            before = self._snapshot(root)
            for operation in (go.write_outputs, go.check_outputs):
                with self.assertRaises(FileNotFoundError):
                    operation(PRINT_OPS, root, [], EXCLUDED)
                self.assertEqual(self._snapshot(root), before)

    def test_whole_inventory_excludes_splices(self):
        self.assertEqual(go.whole_output_paths(), set(self.ANNOUNCED[9:]))
        self.assertEqual(go.generated_output_paths(), go.whole_output_paths())
