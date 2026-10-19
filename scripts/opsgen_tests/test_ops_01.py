"""Operation catalogue generator tests."""

import json
import tempfile
import unittest
from pathlib import Path

from opsgen_tests.common import REPO_ROOT, go
from opsgen_tests.fixtures_01 import PRINT_OPS
from opsgen_tests.fixtures_02 import KNOWN_OPS, _without_skip_reason


class LoadCatalogueTest(unittest.TestCase):
    def _write(self, tmp_path: Path, ops: list) -> None:
        spec_dir = tmp_path / "docs" / "spec"
        spec_dir.mkdir(parents=True, exist_ok=True)
        (spec_dir / "operations.json").write_text(json.dumps({"operations": ops}))

    def test_real_catalogue_loads_and_matches_known_ops(self):
        ops = go.load_catalogue(REPO_ROOT)
        self.assertEqual(_without_skip_reason(ops), KNOWN_OPS)

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

    def test_duplicate_runtime_symbol_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            other = dict(PRINT_OPS[1])
            other["runtime_symbol"] = PRINT_OPS[0]["runtime_symbol"]
            self._write(tmp, [PRINT_OPS[0], other])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn(PRINT_OPS[0]["runtime_symbol"], str(ctx.exception))
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

    def test_effects_unknown_token_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["effects"] = "[frobnicate]"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("frobnicate", str(ctx.exception))

    def test_effects_malformed_bracket_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["effects"] = "io"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("effects", str(ctx.exception))

    def test_effects_trailing_comma_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["effects"] = "[io,]"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("effects", str(ctx.exception))

    def test_effects_unsorted_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["effects"] = "[write, read]"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("effects", str(ctx.exception))

    def test_effects_all_observed_real_values_parse_clean(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            ops = []
            for i, eff in enumerate(["[]", "[io]", "[read]", "[write]"]):
                op = dict(PRINT_OPS[0])
                op["name"] = f"op_{i}"
                op["runtime_symbol"] = f"__vow_op_{i}"
                op["effects"] = eff
                ops.append(op)
            self._write(tmp, ops)
            go.load_catalogue(tmp)  # must not raise

    def test_effects_multi_token_sorted_parses_clean(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            op = dict(PRINT_OPS[0])
            op["effects"] = "[read, write]"
            self._write(tmp, [op])
            go.load_catalogue(tmp)  # must not raise

    def test_doc_signature_arity_mismatch_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            self.assertEqual(bad["params"], ["ptr"])
            bad["doc_signature"] = "fn() -> ()"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("print_str", str(ctx.exception))
            self.assertIn("doc_signature", str(ctx.exception))

    def test_doc_signature_type_mismatch_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            self.assertEqual(bad["params"], ["ptr"])
            bad["doc_signature"] = "fn(s: i64) -> ()"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("print_str", str(ctx.exception))
            self.assertIn("doc_signature", str(ctx.exception))

    def test_doc_signature_return_type_mismatch_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            self.assertEqual(bad["return"], "unit")
            bad["doc_signature"] = "fn(s: String) -> i64"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("print_str", str(ctx.exception))
            self.assertIn("doc_signature", str(ctx.exception))

    def test_doc_signature_missing_arrow_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["doc_signature"] = "fn(s: String)"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("print_str", str(ctx.exception))
            self.assertIn("doc_signature", str(ctx.exception))

    def test_doc_signature_vec_return_matches_ptr_token(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            op = dict(PRINT_OPS[0])
            op["params"] = []
            op["return"] = "ptr"
            op["doc_signature"] = "fn() -> Vec<String>"
            self._write(tmp, [op])
            go.load_catalogue(tmp)  # must not raise

    def test_doc_signature_bool_return_matches_bool_token(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            op = dict(PRINT_OPS[0])
            op["params"] = []
            op["return"] = "bool"
            op["doc_signature"] = "fn() -> bool"
            self._write(tmp, [op])
            go.load_catalogue(tmp)  # must not raise

    def test_unknown_verifier_model_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["verifier_model"] = "nonsense"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("nonsense", str(ctx.exception))
            self.assertIn("verifier_model", str(ctx.exception))

    def test_known_verifier_model_values_parse_clean(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            ops = []
            for i, val in enumerate(["known", "unmodeled"]):
                op = dict(PRINT_OPS[0])
                op["name"] = f"op_{i}"
                op["runtime_symbol"] = f"__vow_op_{i}"
                op["verifier_model"] = val
                ops.append(op)
            self._write(tmp, ops)
            go.load_catalogue(tmp)  # must not raise

    def test_unknown_arena_routing_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["arena_routing"] = "nonsense"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("nonsense", str(ctx.exception))
            self.assertIn("arena_routing", str(ctx.exception))

    def test_known_arena_routing_values_parse_clean(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            ops = []
            for i, val in enumerate(["none", "heap_fresh"]):
                op = dict(PRINT_OPS[0])
                op["name"] = f"op_{i}"
                op["runtime_symbol"] = f"__vow_op_{i}"
                op["arena_routing"] = val
                ops.append(op)
            self._write(tmp, ops)
            go.load_catalogue(tmp)  # must not raise

    def test_arena_routing_is_optional(self):
        # PRINT_OPS carries no arena_routing field at all -- it must not be
        # required, since only heap-returning ops need to set it.
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            self._write(tmp, [PRINT_OPS[0]])
            go.load_catalogue(tmp)  # must not raise

    def test_unexpected_field_name_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["bogus_field"] = "whatever"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("bogus_field", str(ctx.exception))

    def test_misspelled_optional_field_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["verifer_model"] = "known"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("verifer_model", str(ctx.exception))

    def test_multiple_violations_all_reported(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            dup_name = dict(PRINT_OPS[0])
            bad_effects = dict(PRINT_OPS[1])
            bad_effects["effects"] = "[frobnicate]"
            bad_arity = dict(PRINT_OPS[2])
            bad_arity["doc_signature"] = "fn() -> ()"
            self._write(tmp, [PRINT_OPS[0], dup_name, bad_effects, bad_arity])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            message = str(ctx.exception)
            self.assertIn("duplicate operation name 'print_str'", message)
            self.assertIn("frobnicate", message)
            self.assertIn("doc_signature", message)
            self.assertIn("print_u64", message)

    def test_missing_field_does_not_crash_dependent_checks(self):
        # A missing 'params' must not reach the doc_signature arity check
        # and raise a raw TypeError -- it must stop at the clean, collected
        # "missing required field" message for that entry.
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            del bad["params"]
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("missing required field 'params'", str(ctx.exception))
