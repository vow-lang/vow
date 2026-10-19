"""Operation catalogue generator tests."""

import tempfile
import unittest
from pathlib import Path

from opsgen_tests.common import REPO_ROOT, go
from opsgen_tests.fixtures_01 import PRINT_OPS
from opsgen_tests.fixtures_02 import (
    _eval_vow_symbol_set,
    _write_catalogue,
    _write_excluded_catalogue,
)
from opsgen_tests.fixtures_03 import _load_excluded_errors


class CModelTest(unittest.TestCase):
    def _op(self, name, model, effects="[]"):
        op = {
            "name": name,
            "runtime_symbol": f"__vow_{name}",
            "params": [],
            "return": "i64",
            "doc_signature": "fn() -> i64",
            "effects": effects,
        }
        if model is not None:
            op["c_model"] = model
        return op

    def test_symbols_are_base_and_optionally_arena_variant(self):
        ops = [
            self._op("a", "base"),
            self._op("b", "base_and_arena"),
            self._op("c", "unmodeled"),
            self._op("d", None),
        ]
        self.assertEqual(
            go.c_model_symbols(ops), ["__vow_a", "__vow_b", "__vow_b_in_arena"]
        )

    def test_rust_block_is_a_rustfmt_shaped_matches(self):
        ops = [self._op("a", "base"), self._op("b", "base_and_arena")]
        self.assertEqual(
            go.gen_rust_c_model_block(ops, ["__vow_b"]),
            "// GENERATE:OPERATIONS:START\n"
            "fn catalogue_c_model_known(name: &str) -> bool {\n"
            "    matches!(\n"
            "        name,\n"
            '        "__vow_a"\n'
            '            | "__vow_b"\n'
            '            | "__vow_b_in_arena"\n'
            "    )\n"
            "}\n"
            "// GENERATE:OPERATIONS:END",
        )

    def test_rust_block_with_one_symbol_stays_on_one_line(self):
        block = go.gen_rust_c_model_block([self._op("a", "base")], [])
        self.assertIn('    matches!(name, "__vow_a")\n', block)

    def test_vow_block_answers_exactly_the_c_model_symbols(self):
        ops = [self._op("a", "base"), self._op("b", "base_and_arena")]
        block = go.gen_vow_c_model_block(ops, ["__vow_b"])
        fn = block.split("\n", 1)[1].rsplit("\n", 1)[0]
        for sym in ("__vow_a", "__vow_b", "__vow_b_in_arena"):
            self.assertTrue(_eval_vow_symbol_set(fn, sym), sym)
        for sym in ("__vow_a_in_arena", "__vow_c", ""):
            self.assertFalse(_eval_vow_symbol_set(fn, sym), sym)

    def test_pure_builtin_without_c_model_is_reported_by_name(self):
        problems = go.check_c_models([self._op("a", None)], [])
        self.assertEqual(len(problems), 1)
        self.assertIn("'a'", problems[0])

    def test_effectful_builtin_without_c_model_is_clean(self):
        self.assertEqual(go.check_c_models([self._op("a", None, "[io]")], []), [])

    def test_effectful_builtin_cannot_be_known_to_the_c_model(self):
        problems = go.check_c_models([self._op("a", "base", "[io]")], [])
        self.assertEqual(len(problems), 1)
        self.assertIn("only knows pure builtins", problems[0])

    def test_arena_variant_needs_a_route(self):
        problems = go.check_c_models([self._op("a", "base_and_arena")], [])
        self.assertEqual(len(problems), 1)
        self.assertIn("no arena route", problems[0])
        self.assertEqual(
            go.check_c_models([self._op("a", "base_and_arena")], ["__vow_a"]), []
        )

    def test_unknown_c_model_value_is_rejected_by_the_loader(self):
        with tempfile.TemporaryDirectory() as d:
            _write_catalogue(Path(d), [dict(PRINT_OPS[0], c_model="maybe")])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(Path(d))
        self.assertIn("c_model", str(ctx.exception))

    def test_c_model_projection_targets_are_in_the_target_list(self):
        targets = {str(path) for path, _ in go.TARGET_FILES}
        self.assertIn("vow-verify/src/c_emitter.rs", targets)
        self.assertIn("compiler/c_emitter.vow", targets)

    def test_real_catalogue_has_a_decision_for_every_pure_builtin(self):
        ops = go.load_catalogue(REPO_ROOT)
        routes = go.load_arena_routes(REPO_ROOT, ops)
        self.assertEqual(go.check_c_models(ops, routes), [])


class LoadExcludedTest(unittest.TestCase):
    def _load(self, excluded):
        with tempfile.TemporaryDirectory() as d:
            _write_excluded_catalogue(Path(d), excluded)
            return go.load_excluded(Path(d), PRINT_OPS)

    def test_absent_section_excludes_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            _write_catalogue(Path(d), PRINT_OPS)
            self.assertEqual(go.load_excluded(Path(d), PRINT_OPS), [])

    def test_valid_entries_of_every_matcher_kind_are_returned_in_order(self):
        entries = [
            {"names": ["debug_str"], "reason": "r1"},
            {"prefix": "__vow_clif_", "reason": "r2"},
            {"contains": "_to_", "suffixes": ["_try", "_wrap"], "reason": "r3"},
        ]
        self.assertEqual(self._load(entries), entries)

    def test_missing_reason_is_reported_by_matcher(self):
        errors = _load_excluded_errors([{"names": ["debug_str"]}])
        self.assertEqual(len(errors), 1)
        self.assertIn("excluded entry #0", errors[0])
        self.assertIn("debug_str", errors[0])
        self.assertIn("no reason", errors[0])

    def test_blank_reason_is_rejected(self):
        errors = _load_excluded_errors([{"prefix": "__x_", "reason": "  "}])
        self.assertTrue(any("__x_" in e and "no reason" in e for e in errors))

    def test_name_both_catalogued_and_excluded_is_rejected_by_name(self):
        errors = _load_excluded_errors(
            [{"names": ["debug_str", "print_str"], "reason": "r"}]
        )
        self.assertEqual(len(errors), 1)
        self.assertIn("'print_str'", errors[0])
        self.assertIn("both catalogued and excluded", errors[0])

    def test_prefix_and_contains_matchers_cannot_swallow_a_catalogued_name(self):
        by_prefix = _load_excluded_errors([{"prefix": "print_", "reason": "r"}])
        self.assertTrue(any("'print_i64'" in e for e in by_prefix))
        by_contains = _load_excluded_errors(
            [{"contains": "print", "suffixes": ["_u64"], "reason": "r"}]
        )
        self.assertEqual(len(by_contains), 1)
        self.assertIn("'print_u64'", by_contains[0])

    def test_matcher_must_be_exactly_one(self):
        none = _load_excluded_errors([{"reason": "r"}])
        self.assertTrue(any("exactly one matcher" in e for e in none))
        both = _load_excluded_errors([{"names": ["a"], "prefix": "b_", "reason": "r"}])
        self.assertTrue(any("exactly one matcher" in e for e in both))

    def test_malformed_matchers_are_each_reported(self):
        errors = _load_excluded_errors(
            [
                {"names": [], "reason": "r"},
                {"names": ["bad name"], "reason": "r"},
                {"prefix": "", "reason": "r"},
                {"prefix": "a.*", "reason": "r"},
                {"contains": "_to_", "reason": "r"},
                {"contains": "_to_", "suffixes": [], "reason": "r"},
                {"contains": "_to_", "suffixes": ["_a b"], "reason": "r"},
                {"names": ["x"], "suffixes": ["_s"], "reason": "r"},
                {"names": ["y"], "reason": "r", "note": "stray"},
            ]
        )
        text = "\n".join(errors)
        for index in range(9):
            self.assertIn(f"excluded entry #{index}", text)
        self.assertIn("at least one name", text)
        self.assertIn("bad name", text)
        self.assertIn("malformed prefix", text)
        self.assertIn("non-empty 'suffixes'", text)
        self.assertIn("malformed suffix", text)
        self.assertIn("matcher is 'names'", text)
        self.assertIn("unknown field", text)

    def test_a_name_listed_twice_is_reported_with_both_entries(self):
        errors = _load_excluded_errors(
            [
                {"names": ["debug_str"], "reason": "r"},
                {"names": ["debug_str"], "reason": "r"},
            ]
        )
        self.assertEqual(len(errors), 1)
        self.assertIn("'debug_str'", errors[0])
        self.assertIn("#0", errors[0])
        self.assertIn("#1", errors[0])

    def test_non_list_section_and_non_object_entry_are_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            _write_excluded_catalogue(Path(d), {"names": ["a"]})
            with self.assertRaises(ValueError):
                go.load_excluded(Path(d), PRINT_OPS)
        errors = _load_excluded_errors(["debug_str"])
        self.assertIn("excluded entry #0 must be an object", errors)

    def test_real_catalogue_excluded_entries_are_all_valid(self):
        ops = go.load_catalogue(REPO_ROOT)
        excluded = go.load_excluded(REPO_ROOT, ops)
        self.assertTrue(excluded)
        for entry in excluded:
            self.assertTrue(entry["reason"].strip())


class ExcludedMatchesTest(unittest.TestCase):
    def test_names_match_exactly(self):
        entry = {"names": ["debug_str", "debug_i64"]}
        self.assertTrue(go.excluded_matches(entry, "debug_i64"))
        self.assertFalse(go.excluded_matches(entry, "debug_u64"))
        self.assertFalse(go.excluded_matches(entry, "debug_str_x"))

    def test_prefix_matches_only_at_the_start(self):
        entry = {"prefix": "__vow_clif_"}
        self.assertTrue(go.excluded_matches(entry, "__vow_clif_create"))
        self.assertFalse(go.excluded_matches(entry, "x__vow_clif_create"))

    def test_contains_needs_the_substring_and_a_listed_suffix(self):
        entry = {"contains": "_to_", "suffixes": ["_try", "_wrap", "_sat"]}
        self.assertTrue(go.excluded_matches(entry, "i64_to_u8_try"))
        self.assertTrue(go.excluded_matches(entry, "u128_to_i128_sat"))
        self.assertFalse(go.excluded_matches(entry, "int128_to_string"))
        self.assertFalse(go.excluded_matches(entry, "string_to_upper"))
        self.assertFalse(go.excluded_matches(entry, "i64_to"))

    def test_seeded_real_exclusions_cover_the_known_uncataloged_builtins(self):
        ops = go.load_catalogue(REPO_ROOT)
        excluded = go.load_excluded(REPO_ROOT, ops)
        covered = [
            "__vow_clif_fn_begin",
            "debug_str",
            "debug_i64",
            "debug_u64",
            "string_matches_literal_at",
            "i64_to_string",
            "i64_to_u8_try",
            "u128_to_i128_wrap",
            "add_sat_u8",
            "sub_sat_u8",
            "mul_sat_u8",
        ]
        for name in covered:
            self.assertTrue(any(go.excluded_matches(e, name) for e in excluded), name)
        for op in ops:
            self.assertFalse(
                any(go.excluded_matches(e, op["name"]) for e in excluded), op["name"]
            )
