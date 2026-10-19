"""Operation catalogue generator tests."""

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

from opsgen_tests.common import REPO_ROOT, go
from opsgen_tests.fixtures_01 import PRINT_OPS
from opsgen_tests.fixtures_02 import _heap_op


class RealCatalogueProcessOpsTest(unittest.TestCase):
    """Fixture coverage for the process_* entries appended to the real
    catalogue: a non-unit (i64) return, a ptr return, and an
    arena_routing: heap_fresh entry, per issue #1273."""

    def test_process_run_has_i64_return(self):
        ops = go.load_catalogue(REPO_ROOT)
        op = next(o for o in ops if o["name"] == "process_run")
        self.assertEqual(op["return"], "i64")
        self.assertEqual(op["runtime_symbol"], "__vow_process_run")
        self.assertIsNone(op.get("arena_routing"))

    def test_process_get_stdout_has_ptr_return_and_heap_fresh_routing(self):
        ops = go.load_catalogue(REPO_ROOT)
        op = next(o for o in ops if o["name"] == "process_get_stdout")
        self.assertEqual(op["return"], "ptr")
        self.assertEqual(op["runtime_symbol"], "__vow_process_get_stdout")
        self.assertEqual(op["arena_routing"], "heap_fresh")

    def test_all_process_ops_present(self):
        ops = go.load_catalogue(REPO_ROOT)
        names = {o["name"] for o in ops}
        for name in [
            "process_exit",
            "process_run",
            "process_get_stdout",
            "process_get_stderr",
            "process_start",
            "process_start_capped",
            "process_wait",
            "process_wait_timeout",
            "process_poll_wait",
            "process_kill",
            "process_stdout_for",
            "process_stderr_for",
            "process_start_piped",
            "process_write_stdin",
            "process_close_stdin",
            "process_read_line",
            "process_read_status",
        ]:
            self.assertIn(name, names)

    def test_env_ops_present_with_expected_shape(self):
        ops = {o["name"]: o for o in go.load_catalogue(REPO_ROOT)}
        self.assertEqual(ops["fs_read_status"]["return"], "i64")
        self.assertEqual(ops["fs_read_status"]["params"], [])
        for name in ["getenv", "path_lookup", "mktemp_dir"]:
            self.assertEqual(ops[name]["return"], "ptr")
            self.assertEqual(ops[name]["params"], ["ptr"])
            self.assertEqual(ops[name]["arena_routing"], "heap_fresh")
            self.assertEqual(ops[name]["runtime_symbol"], f"__vow_{name}")
        self.assertNotIn("arena_routing", ops["fs_read_status"])

    def test_real_catalogue_projections_are_up_to_date(self):
        ops = go.load_catalogue(REPO_ROOT)
        routes = go.load_arena_routes(REPO_ROOT, ops)
        mismatches = go.check_projections(ops, REPO_ROOT, routes)
        self.assertEqual(mismatches, [])

    def test_real_catalogue_doc_facts_are_consistent(self):
        ops = go.load_catalogue(REPO_ROOT)
        mismatches = go.check_doc_facts(ops, REPO_ROOT)
        self.assertEqual(mismatches, [])


class MainCliTest(unittest.TestCase):
    """Exercises main() itself (the --check CLI entry point) rather than
    load_catalogue/check_projections/check_doc_facts directly, so the CLI
    plumbing (argv parsing, --repo-root threading) is proven end to end."""

    def _write_minimal_catalogue(self, tmp: Path, ops: list) -> None:
        spec_dir = tmp / "docs" / "spec"
        spec_dir.mkdir(parents=True, exist_ok=True)
        (spec_dir / "operations.json").write_text(json.dumps({"operations": ops}))

    def _run_main(self, argv: list[str]):
        old_argv = sys.argv
        sys.argv = ["generate_operations.py"] + argv
        try:
            return go.main()
        finally:
            sys.argv = old_argv

    def test_repo_root_flag_threads_through_check(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["effects"] = "[frobnicate]"
            self._write_minimal_catalogue(tmp, [bad])
            stderr = io.StringIO()
            with (
                self.assertRaises(SystemExit) as ctx,
                contextlib.redirect_stderr(stderr),
            ):
                self._run_main(["--check", "--repo-root", str(tmp)])
            self.assertEqual(ctx.exception.code, 1)
            self.assertIn("frobnicate", stderr.getvalue())

    def test_effectful_known_op_is_rejected_naming_it(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            self._write_minimal_catalogue(
                tmp, [dict(PRINT_OPS[2], verifier_model="known")]
            )
            stderr = io.StringIO()
            with (
                self.assertRaises(SystemExit) as ctx,
                contextlib.redirect_stderr(stderr),
            ):
                self._run_main(["--check", "--repo-root", str(tmp)])
            self.assertEqual(ctx.exception.code, 1)
            self.assertIn("'print_u64'", stderr.getvalue())
            self.assertIn("verifier_model", stderr.getvalue())

    def test_repo_root_flag_defaults_to_real_repo(self):
        # No --repo-root: main() must still validate the real, checked-in
        # catalogue exactly as the CI/local `--check` invocation does.
        self._run_main(["--check"])

    def test_bad_catalogue_exits_cleanly_without_traceback(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["effects"] = "[frobnicate]"
            self._write_minimal_catalogue(tmp, [bad])
            stdout, stderr = io.StringIO(), io.StringIO()
            with (
                self.assertRaises(SystemExit) as ctx,
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                self._run_main(["--check", "--repo-root", str(tmp)])
            self.assertEqual(ctx.exception.code, 1)
            combined = stdout.getvalue() + stderr.getvalue()
            self.assertNotIn("Traceback (most recent call last)", combined)
            self.assertIn("frobnicate", combined)

    def test_undecided_builtin_exits_cleanly_naming_verifier_model(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            self._write_minimal_catalogue(tmp, [dict(PRINT_OPS[0])])
            stdout, stderr = io.StringIO(), io.StringIO()
            with (
                self.assertRaises(SystemExit) as ctx,
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                self._run_main(["--check", "--repo-root", str(tmp)])
            self.assertEqual(ctx.exception.code, 1)
            combined = stdout.getvalue() + stderr.getvalue()
            self.assertNotIn("Traceback (most recent call last)", combined)
            self.assertIn("verifier_model", combined)

    def test_heap_result_without_arena_routing_exits_naming_the_op(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            op = _heap_op("undeclared", verifier_model="unmodeled", c_model="unmodeled")
            self._write_minimal_catalogue(tmp, [op])
            stdout, stderr = io.StringIO(), io.StringIO()
            with (
                self.assertRaises(SystemExit) as ctx,
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                self._run_main(["--check", "--repo-root", str(tmp)])
            self.assertEqual(ctx.exception.code, 1)
            combined = stdout.getvalue() + stderr.getvalue()
            self.assertNotIn("Traceback (most recent call last)", combined)
            self.assertIn("'undeclared' returns a heap value", combined)

    def test_malformed_json_exits_cleanly_without_traceback(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            spec_dir = tmp / "docs" / "spec"
            spec_dir.mkdir(parents=True, exist_ok=True)
            (spec_dir / "operations.json").write_text("{not valid json")
            stdout, stderr = io.StringIO(), io.StringIO()
            with (
                self.assertRaises(SystemExit) as ctx,
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                self._run_main(["--check", "--repo-root", str(tmp)])
            self.assertEqual(ctx.exception.code, 1)
            combined = stdout.getvalue() + stderr.getvalue()
            self.assertNotIn("Traceback (most recent call last)", combined)


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
            "\n"
            "fn catalogue_builtin_result_tag(name: &str) -> Option<BuiltinResultTag> {\n"
            "    match name {\n"
            "        _ => None,\n"
            "    }\n"
            "}\n"
            "// GENERATE:OPERATIONS:END"
        )
        self.assertEqual(go.gen_rust_ir_block(PRINT_OPS, []), expected)

    def test_i64_and_ptr_return_tokens_map_to_rust_ty(self):
        i64_op = dict(PRINT_OPS[1])
        i64_op["return"] = "i64"
        ptr_op = dict(PRINT_OPS[1])
        ptr_op["return"] = "ptr"
        block = go.gen_rust_ir_block([i64_op, ptr_op], [])
        self.assertIn(
            f'"{i64_op["name"]}" => Some(("{i64_op["runtime_symbol"]}", Ty::I64)),',
            block,
        )
        self.assertIn(
            f'"{ptr_op["name"]}" => Some(("{ptr_op["runtime_symbol"]}", Ty::Ptr)),',
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

    def test_string_return_is_tagged_string_heap(self):
        block = go.gen_rust_ir_block([self._op_returning("String")], [])
        self.assertIn('"op" => Some(BuiltinResultTag::StringHeap),', block)

    def test_vec_return_is_tagged_vec_heap(self):
        block = go.gen_rust_ir_block([self._op_returning("Vec<u8>")], [])
        self.assertIn('"op" => Some(BuiltinResultTag::VecHeap),', block)

    def test_option_return_is_tagged_with_its_element_type(self):
        for elem in go.OPTION_ELEMS:
            block = go.gen_rust_ir_block([self._op_returning(f"Option<{elem}>")], [])
            rust_ty = go.OPTION_ELEMS[elem]["rust_ty"]
            self.assertIn(
                f'"op" => Some(BuiltinResultTag::OptionOf({rust_ty})),', block, elem
            )

    def test_scalar_and_unit_returns_are_untagged(self):
        for doc_ret in ("i64", "u64", "bool", "()"):
            block = go.gen_rust_ir_block([self._op_returning(doc_ret)], [])
            self.assertNotIn(
                "BuiltinResultTag::", block.split("fn catalogue_builtin_result_tag")[1]
            )

    def test_tag_does_not_depend_on_arena_routing(self):
        plain = self._op_returning("String")
        routed = dict(plain, arena_routing="heap_fresh")
        none = dict(plain, arena_routing="none")
        self.assertEqual(
            go.gen_rust_ir_block([plain], []), go.gen_rust_ir_block([routed], [])
        )
        self.assertEqual(
            go.gen_rust_ir_block([plain], []), go.gen_rust_ir_block([none], [])
        )


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
        self.assertEqual(go.gen_cranelift_block(PRINT_OPS, []), expected)

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
            block = go.gen_cranelift_block([op], [])
        finally:
            del go.RETURN_TOKENS["fake_i64"]
        self.assertIn(
            "sig.returns.push(AbiParam::new(types::I64));\n            true", block
        )

    def test_i64_and_ptr_return_tokens_push_return_slot(self):
        i64_op = dict(PRINT_OPS[1])
        i64_op["return"] = "i64"
        ptr_op = dict(PRINT_OPS[1])
        ptr_op["return"] = "ptr"
        block = go.gen_cranelift_block([i64_op, ptr_op], [])
        self.assertEqual(
            block.count(
                "sig.returns.push(AbiParam::new(types::I64));\n            true"
            ),
            2,
        )
