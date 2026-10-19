"""Operation catalogue generator tests."""

import tempfile
import unittest
from pathlib import Path

from opsgen_tests.common import REPO_ROOT, go
from opsgen_tests.fixtures_01 import PRINT_OPS
from opsgen_tests.fixtures_02 import (
    PRE_MIGRATION_C_MODEL_SYMBOLS,
    QUERY_UTILITY_OPS,
    VERIFIER_KNOWN_NAMES,
    _QUERY_UTILITY_ROWS,
    _heap_op,
    _write_catalogue,
)


class ArenaRoutingInvariantsTest(unittest.TestCase):
    def _problems(self, op, routes):
        return go.check_arena_routing([op], routes)

    def test_heap_result_without_arena_routing_is_reported_by_name(self):
        for ret in ("String", "Vec<i64>", "Option<u8>"):
            problems = self._problems(_heap_op(ret=ret), [])
            self.assertEqual(len(problems), 1, ret)
            self.assertIn("'heap_op' returns a heap value", problems[0])

    def test_scalar_result_may_omit_arena_routing(self):
        scalar = dict(_heap_op(), doc_signature="fn() -> i64", **{"return": "i64"})
        self.assertEqual(self._problems(scalar, []), [])

    def test_route_without_heap_fresh_is_reported(self):
        scalar = dict(_heap_op(), doc_signature="fn() -> i64", **{"return": "i64"})
        problems = self._problems(scalar, ["__vow_heap_op"])
        self.assertEqual(len(problems), 1)
        self.assertIn("has an arena route", problems[0])
        self.assertIn("'heap_op'", problems[0])

    def test_undeclared_heap_op_with_a_route_is_reported_once(self):
        problems = self._problems(_heap_op(), ["__vow_heap_op"])
        self.assertEqual(len(problems), 1)
        self.assertIn("returns a heap value but has no arena_routing", problems[0])

    def test_heap_fresh_needs_a_heap_result(self):
        scalar = dict(
            _heap_op(arena_routing="heap_fresh"),
            doc_signature="fn() -> i64",
            **{"return": "i64"},
        )
        problems = self._problems(scalar, ["__vow_heap_op"])
        self.assertEqual(len(problems), 1)
        self.assertIn("'heap_op' is heap_fresh", problems[0])
        self.assertIn("not a String, Vec or Option", problems[0])

    def test_heap_fresh_without_route_is_reported(self):
        problems = self._problems(_heap_op(arena_routing="heap_fresh"), [])
        self.assertEqual(len(problems), 1)
        self.assertIn("'heap_op' is heap_fresh", problems[0])
        self.assertIn("has no arena route", problems[0])

    def test_none_with_route_is_reported(self):
        problems = self._problems(_heap_op(arena_routing="none"), ["__vow_heap_op"])
        self.assertEqual(len(problems), 1)
        self.assertIn("'heap_op' is arena_routing 'none'", problems[0])

    def test_consistent_declarations_are_clean(self):
        self.assertEqual(
            self._problems(_heap_op(arena_routing="heap_fresh"), ["__vow_heap_op"]),
            [],
        )
        self.assertEqual(self._problems(_heap_op(arena_routing="none"), []), [])

    def test_none_on_a_heap_result_needs_a_reason(self):
        op = _heap_op(arena_routing="none")
        del op["arena_routing_reason"]
        problems = self._problems(op, [])
        self.assertEqual(len(problems), 1)
        self.assertIn("'heap_op' is arena_routing 'none'", problems[0])
        self.assertIn("arena_routing_reason", problems[0])

    def test_none_on_a_scalar_result_needs_no_reason(self):
        scalar = dict(_heap_op(), doc_signature="fn() -> i64", **{"return": "i64"})
        scalar["arena_routing"] = "none"
        self.assertEqual(self._problems(scalar, []), [])

    def test_reason_is_only_allowed_on_none(self):
        op = _heap_op(arena_routing="heap_fresh", arena_routing_reason="why")
        problems = self._problems(op, ["__vow_heap_op"])
        self.assertEqual(len(problems), 1)
        self.assertIn("is not arena_routing 'none'", problems[0])

    def test_blank_or_non_string_reason_is_reported(self):
        for reason in ("  ", 7):
            op = _heap_op(arena_routing="none", arena_routing_reason=reason)
            problems = self._problems(op, [])
            self.assertEqual(len(problems), 1, reason)
            self.assertIn("empty or non-string", problems[0])

    def test_real_catalogue_reasons_sit_only_on_heap_nones(self):
        for op in go.load_catalogue(REPO_ROOT):
            heap_none = (
                op.get("arena_routing") == "none" and go.result_shape(op) is not None
            )
            self.assertEqual("arena_routing_reason" in op, heap_none, op["name"])

    def test_route_without_a_catalogued_op_stays_legal(self):
        self.assertEqual(go.check_arena_routing([], ["__vow_map_get"]), [])

    def test_every_violation_across_the_catalogue_is_reported(self):
        ops = [
            _heap_op("a"),
            _heap_op("b", arena_routing="heap_fresh"),
            _heap_op("c", arena_routing="none"),
        ]
        self.assertEqual(len(go.check_arena_routing(ops, ["__vow_c"])), 3)

    def test_real_catalogue_declares_routing_exactly_where_a_route_exists(self):
        catalogue = go.load_catalogue(REPO_ROOT)
        routes = go.load_arena_routes(REPO_ROOT, catalogue)
        self.assertEqual(go.check_arena_routing(catalogue, routes), [])
        for op in catalogue:
            routed = op["runtime_symbol"] in routes
            self.assertEqual(
                op.get("arena_routing") == "heap_fresh", routed, op["name"]
            )
            if go.result_shape(op) is not None:
                self.assertIn("arena_routing", op, op["name"])

    def test_unrouted_heap_returns_are_explicit_nones(self):
        ops = {o["name"]: o for o in go.load_catalogue(REPO_ROOT)}
        self.assertEqual(ops["stdin_read_line"]["arena_routing"], "none")
        for name in (
            "fs_read",
            "fs_read_line",
            "fs_listdir",
            "args",
            "stdin_read",
            "proc_sample",
        ):
            self.assertEqual(ops[name]["arena_routing"], "heap_fresh", name)


class GenVowVcOpsBlockTest(unittest.TestCase):
    def test_only_known_symbols_land_in_the_lookup(self):
        ops = [
            dict(PRINT_OPS[0], verifier_model="known"),
            dict(PRINT_OPS[1], verifier_model="unmodeled"),
        ]
        block = go.gen_vow_vc_ops_block(ops, [])
        self.assertIn(PRINT_OPS[0]["runtime_symbol"], block)
        self.assertNotIn(PRINT_OPS[1]["runtime_symbol"], block)
        self.assertNotIn("catalogue_verifier_unmodeled", block)

    def test_no_known_symbols_still_yields_a_total_lookup(self):
        block = go.gen_vow_vc_ops_block(
            [dict(PRINT_OPS[0], verifier_model="unmodeled")], []
        )
        self.assertIn("fn catalogue_verifier_known(sym: String) -> bool", block)
        self.assertIn("return false;", block)
        self.assertNotIn("let n", block)


class ResultShapeTest(unittest.TestCase):
    def _op(self, doc_ret):
        return {"doc_signature": f"fn() -> {doc_ret}"}

    def test_shapes_follow_the_doc_return_type(self):
        self.assertEqual(go.result_shape(self._op("String")), ("string", None))
        self.assertEqual(go.result_shape(self._op("Vec<String>")), ("vec", None))
        self.assertEqual(go.result_shape(self._op("Option<u8>")), ("option", "u8"))
        for scalar in ("i64", "u64", "bool", "()", "!"):
            self.assertIsNone(go.result_shape(self._op(scalar)), scalar)

    def test_unknown_option_element_is_rejected_by_name(self):
        op = {
            "name": "bad",
            "runtime_symbol": "__vow_bad",
            "params": ["ptr"],
            "return": "ptr",
            "doc_signature": "fn(s: String) -> Option<f64>",
            "effects": "[]",
        }
        with tempfile.TemporaryDirectory() as d:
            _write_catalogue(Path(d), [op])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(Path(d))
        self.assertIn("Option element type 'f64'", str(ctx.exception))

    def test_wide_params_and_option_returns_are_valid_doc_types(self):
        self.assertEqual(go._doc_type_to_token("i128"), "i128")
        self.assertEqual(go._doc_type_to_token("u128"), "u128")
        self.assertEqual(go._doc_type_to_token("Option<i64>"), "ptr")
        self.assertEqual(go.PARAM_TOKENS["i128"], "types::I128")
        self.assertEqual(go.PARAM_TOKENS["u128"], "types::I128")

    def test_wide_param_reaches_the_cranelift_signature(self):
        op = next(o for o in QUERY_UTILITY_OPS if o["name"] == "int128_to_string")
        block = go.gen_cranelift_block([op], [])
        self.assertIn("sig.params.push(AbiParam::new(types::I128));", block)
        self.assertIn("sig.returns.push(AbiParam::new(types::I64));", block)

    def test_real_catalogue_tags_match_each_ops_runtime_shape(self):
        shapes = {o["name"]: go.result_shape(o) for o in go.load_catalogue(REPO_ROOT)}
        for name in ("hex_decode", "string_split", "vec_sort", "args", "fs_listdir"):
            self.assertEqual(shapes[name], ("vec", None), name)
        for name in (
            "hex_encode",
            "format_f64_bits",
            "proc_sample",
            "int_to_string",
            "uint_to_string",
            "int128_to_string",
            "uint128_to_string",
            "string_substr",
            "string_trim",
            "string_to_upper",
            "string_to_lower",
            "string_replace",
            "string_join",
            "fs_read",
            "stdin_read_line",
        ):
            self.assertEqual(shapes[name], ("string", None), name)
        for name in (
            "parse_i64",
            "parse_i8",
            "parse_i16",
            "parse_i32",
            "parse_i128",
            "parse_u8",
            "parse_u16",
            "parse_u32",
            "parse_u128",
        ):
            self.assertEqual(
                shapes[name], ("option", name.removeprefix("parse_")), name
            )
        for name in (
            "parse_f64_bits",
            "string_starts_with",
            "string_ends_with",
            "time_micros",
            "num_cpus",
            "gzip_write_file",
            "memory_peak_bytes",
        ):
            self.assertIsNone(shapes[name], name)


class QueryUtilityCatalogueTest(unittest.TestCase):
    def setUp(self):
        self.catalogue = go.load_catalogue(REPO_ROOT)
        self.ops = {o["name"]: o for o in self.catalogue}

    def test_every_query_and_utility_builtin_is_catalogued(self):
        for row in _QUERY_UTILITY_ROWS:
            op = self.ops[row[0]]
            self.assertEqual(op["runtime_symbol"], row[1], row[0])
            self.assertEqual(op["return"], row[3], row[0])

    def test_hand_written_survivors_are_not_catalogued(self):
        # `i64_to_string` shares its runtime symbol with `int_to_string`, the
        # literal-bytes lowering of `string_matches_literal_at` is a special
        # operation body, and there is no free `parse_u64`.
        for name in ("i64_to_string", "string_matches_literal_at", "parse_u64"):
            self.assertNotIn(name, self.ops)

    def test_native_verifier_models_only_the_string_slice(self):
        for row in _QUERY_UTILITY_ROWS:
            want = "known" if row[0] in VERIFIER_KNOWN_NAMES else "unmodeled"
            self.assertEqual(self.ops[row[0]]["verifier_model"], want, row[0])
        block = go.gen_vow_vc_ops_block(self.catalogue, [])
        self.assertIn("return false;", block)

    def test_c_model_symbols_equal_the_pre_migration_hand_list(self):
        routes = go.load_arena_routes(REPO_ROOT, self.catalogue)
        query_symbols = {r[1] for r in _QUERY_UTILITY_ROWS}
        query_symbols |= {go.arena_variant(r[1]) for r in _QUERY_UTILITY_ROWS}
        catalogued = {
            sym for sym in go.c_model_symbols(self.catalogue) if sym in query_symbols
        }
        self.assertEqual(catalogued, PRE_MIGRATION_C_MODEL_SYMBOLS)
        self.assertEqual(go.check_c_models(self.catalogue, routes), [])

    def test_effectful_query_ops_have_no_c_model(self):
        for name in ("time_unix", "num_cpus", "proc_sample", "gzip_write_file"):
            self.assertNotIn("c_model", self.ops[name], name)
