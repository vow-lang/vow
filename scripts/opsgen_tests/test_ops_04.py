"""Operation catalogue generator tests."""

import json
import tempfile
import unittest
from pathlib import Path

from opsgen_tests.common import REPO_ROOT, go
from opsgen_tests.fixtures_01 import HASH_OPS, PRINT_OPS
from opsgen_tests.fixtures_02 import (
    KNOWN_OPS,
    ROUTES,
    _eval_vow_symbol_set,
    _reasoned,
    _write_target_fixtures,
)


class LoadArenaRoutesTest(unittest.TestCase):
    def _write(self, tmp: Path, routes, ops=None) -> None:
        spec_dir = tmp / "docs" / "spec"
        spec_dir.mkdir(parents=True, exist_ok=True)
        body = {"operations": ops or [], "arena_routes": routes}
        (spec_dir / "operations.json").write_text(json.dumps(body))

    def _errors(self, routes, ops=None) -> str:
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            self._write(tmp, routes, ops)
            with self.assertRaises(ValueError) as ctx:
                go.load_arena_routes(tmp, ops or [])
            return str(ctx.exception)

    def test_real_catalogue_routes_load(self):
        ops = go.load_catalogue(REPO_ROOT)
        routes = go.load_arena_routes(REPO_ROOT, ops)
        self.assertIn("__vow_vec_sort", routes)
        self.assertIn("__vow_btreemap_insert", routes)
        self.assertIn("__vow_btreemap_new", routes)
        self.assertIn("__vow_i64_to_u8_try", routes)

    def test_missing_section_means_no_routes(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            spec_dir = tmp / "docs" / "spec"
            spec_dir.mkdir(parents=True)
            (spec_dir / "operations.json").write_text(json.dumps({"operations": []}))
            self.assertEqual(go.load_arena_routes(tmp, []), [])

    def test_valid_routes_load_unchanged(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            self._write(tmp, ROUTES)
            self.assertEqual(go.load_arena_routes(tmp, []), ROUTES)

    def test_duplicate_symbol_raises(self):
        self.assertIn(
            "'__vow_map_get' appears more than once",
            self._errors([ROUTES[0], ROUTES[0]]),
        )

    def test_variant_of_another_route_cannot_also_be_a_base(self):
        self.assertIn(
            "appears more than once",
            self._errors([ROUTES[0], "__vow_map_get_in_arena"]),
        )

    def test_malformed_symbol_raises(self):
        self.assertIn("is not a symbol", self._errors(["map_get"]))

    def test_non_string_route_raises(self):
        self.assertIn("is not a symbol", self._errors([{"runtime_symbol": "x"}]))

    def test_non_list_section_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            self._write(tmp, {"not": "a list"})
            with self.assertRaises(ValueError):
                go.load_arena_routes(tmp, [])


class CheckRuntimeExportsTest(unittest.TestCase):
    def _check(self, runtime_text: str) -> list[str]:
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            (tmp / "vow-runtime" / "src").mkdir(parents=True)
            (tmp / "vow-runtime" / "src" / "lib.rs").write_text(runtime_text)
            return go.check_runtime_exports(ROUTES, tmp)

    def test_all_symbols_present_is_clean(self):
        text = " ".join(sym for r in ROUTES for sym in (r, go.arena_variant(r)))
        self.assertEqual(self._check(text), [])

    def test_base_is_not_satisfied_by_its_variant(self):
        text = (
            " ".join(go.arena_variant(r) for r in ROUTES)
            + " __vow_string_parse_i64_opt"
        )
        self.assertEqual(
            self._check(text),
            ["vow-runtime/src: does not define '__vow_map_get'"],
        )

    def test_missing_variant_is_reported(self):
        text = " ".join(ROUTES)
        self.assertEqual(len(self._check(text)), 2)


class VowSymbolSetFnTest(unittest.TestCase):
    def _real_routes(self):
        return go.load_arena_routes(REPO_ROOT, go.load_catalogue(REPO_ROOT))

    def test_real_routes_match_exactly_their_symbols(self):
        routes = self._real_routes()
        base_fn = go._vow_symbol_set_fn("f", routes)
        for sym in routes:
            self.assertTrue(_eval_vow_symbol_set(base_fn, sym), sym)
            self.assertFalse(_eval_vow_symbol_set(base_fn, go.arena_variant(sym)), sym)

    def test_near_misses_are_rejected(self):
        routes = self._real_routes()
        base_fn = go._vow_symbol_set_fn("f", routes)
        known = set(routes)
        for sym in routes:
            near = {sym[:-1], sym + "x", ""}
            near |= {sym[:i] + "#" + sym[i + 1 :] for i in range(len(sym))}
            for candidate in near - known:
                self.assertFalse(_eval_vow_symbol_set(base_fn, candidate), candidate)

    def test_unrelated_runtime_symbols_are_rejected(self):
        base_fn = go._vow_symbol_set_fn("f", ["__vow_map_get", "__vow_vec_sort"])
        for sym in ["__vow_map_new", "__vow_vec_push", "__vow_string_print", "main"]:
            self.assertFalse(_eval_vow_symbol_set(base_fn, sym), sym)

    def test_each_symbol_is_spelled_once_and_length_is_read_first(self):
        routes = self._real_routes()
        source = go._vow_symbol_set_fn("f", routes)
        self.assertEqual(source.count("String::from("), len(routes))
        self.assertIn("let n: u64 = sym.len();", source)

    def test_empty_set_matches_nothing(self):
        fn = go._vow_symbol_set_fn("f", [])
        self.assertFalse(_eval_vow_symbol_set(fn, "__vow_map_get"))


class GenFreshRustBlocksTest(unittest.TestCase):
    def test_region_block_is_the_public_table(self):
        block = go.gen_region_block(PRINT_OPS, ROUTES)
        self.assertIn("pub const FRESH_ARENA_VARIANTS", block)
        self.assertIn('("__vow_map_get", "__vow_map_get_in_arena")', block)
        self.assertNotIn("fn ", block)

    def test_shim_block_keeps_the_table_private(self):
        block = go.gen_shim_block(PRINT_OPS, ROUTES)
        self.assertIn("fn catalogue_extern_sig(", block)
        self.assertIn("\nconst FRESH_ARENA_VARIANTS", block)
        self.assertNotIn("cfg(test)", block)
        self.assertNotIn("pub ", block)


class FreshRouteProjectionsTest(unittest.TestCase):
    def test_route_edit_makes_exactly_the_route_targets_stale(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            go.write_projections(PRINT_OPS, tmp, ROUTES)
            self.assertEqual(go.check_projections(PRINT_OPS, tmp, ROUTES), [])
            mismatches = go.check_projections(PRINT_OPS, tmp, ROUTES[:1])
        stale = " ".join(mismatches)
        self.assertIn("region.rs", stale)
        self.assertIn("ir.vow", stale)
        self.assertIn("vow-clif-shim", stale)
        self.assertNotIn("lower.vow", stale)
        self.assertNotIn("cranelift_backend.rs", stale)
        self.assertNotIn("vc_ops.vow", stale)


class VerifierModelDriftTest(unittest.TestCase):
    def test_entry_without_verifier_model_is_reported_by_name(self):
        ops = [dict(PRINT_OPS[0]), dict(PRINT_OPS[1], verifier_model="unmodeled")]
        problems = go.check_verifier_models(ops)
        self.assertEqual(len(problems), 1)
        self.assertIn(PRINT_OPS[0]["name"], problems[0])

    def test_explicit_unmodeled_and_known_are_clean(self):
        ops = [
            dict(PRINT_OPS[0], verifier_model="unmodeled"),
            dict(HASH_OPS[0], verifier_model="known"),
        ]
        self.assertEqual(go.check_verifier_models(ops), [])

    def test_real_catalogue_has_a_decision_for_every_builtin(self):
        self.assertEqual(go.check_verifier_models(go.load_catalogue(REPO_ROOT)), [])

    def test_pure_unmodeled_entry_needs_a_skip_reason(self):
        problems = go.check_verifier_models(
            [dict(HASH_OPS[0], verifier_model="unmodeled")]
        )
        self.assertEqual(len(problems), 1)
        self.assertIn("'hash_u64'", problems[0])
        self.assertIn("verifier_skip_reason", problems[0])

    def test_pure_unmodeled_entry_with_a_reason_is_clean(self):
        ops = [
            dict(
                HASH_OPS[0],
                verifier_model="unmodeled",
                verifier_skip_reason="a fold over every byte",
            )
        ]
        self.assertEqual(go.check_verifier_models(ops), [])

    def test_effectful_unmodeled_entry_needs_no_reason(self):
        ops = [dict(PRINT_OPS[0], verifier_model="unmodeled")]
        self.assertEqual(go.check_verifier_models(ops), [])

    def test_blank_or_non_string_reason_is_rejected(self):
        for bad in ("", "   ", 7):
            with self.subTest(reason=bad):
                problems = go.check_verifier_models(
                    [
                        dict(
                            HASH_OPS[0],
                            verifier_model="unmodeled",
                            verifier_skip_reason=bad,
                        )
                    ]
                )
                self.assertEqual(len(problems), 1)
                self.assertIn("empty or non-string", problems[0])

    def test_reason_on_a_modelled_or_effectful_entry_is_rejected(self):
        for op in (
            dict(HASH_OPS[0], verifier_model="known"),
            dict(PRINT_OPS[0], verifier_model="unmodeled"),
        ):
            with self.subTest(op=op["name"]):
                problems = go.check_verifier_models(
                    [dict(op, verifier_skip_reason="because")]
                )
                self.assertEqual(len(problems), 1)
                self.assertIn("not a pure 'unmodeled' op", problems[0])

    def test_real_catalogue_explains_every_pure_unmodeled_builtin(self):
        for op in go.load_catalogue(REPO_ROOT):
            if op["effects"] == "[]" and op["verifier_model"] == "unmodeled":
                with self.subTest(op=op["name"]):
                    self.assertTrue(op["verifier_skip_reason"].strip())

    def test_effectful_builtin_cannot_be_known_to_the_native_verifier(self):
        problems = go.check_verifier_models(
            [dict(PRINT_OPS[2], verifier_model="known")]
        )
        self.assertEqual(len(problems), 1)
        self.assertIn("'print_u64'", problems[0])
        self.assertIn("only models pure builtins", problems[0])

    def test_only_the_effectful_known_entry_is_reported(self):
        ops = [
            dict(HASH_OPS[0], verifier_model="known"),
            dict(PRINT_OPS[0], verifier_model="unmodeled"),
            dict(PRINT_OPS[1], verifier_model="known"),
        ]
        problems = go.check_verifier_models(ops)
        self.assertEqual(len(problems), 1)
        self.assertIn("'print_i64'", problems[0])

    def test_every_effect_token_blocks_known(self):
        for effects in (
            "[read]",
            "[write]",
            "[io]",
            "[panic]",
            "[unsafe]",
            "[io, read]",
        ):
            with self.subTest(effects=effects):
                op = dict(HASH_OPS[0], effects=effects, verifier_model="known")
                self.assertEqual(len(go.check_verifier_models([op])), 1)

    def test_real_effectful_builtins_are_never_known(self):
        effectful = [op for op in go.load_catalogue(REPO_ROOT) if op["effects"] != "[]"]
        self.assertTrue(effectful)
        for op in effectful:
            with self.subTest(op=op["name"]):
                self.assertNotEqual(op.get("verifier_model"), "known")
                problems = go.check_verifier_models([dict(op, verifier_model="known")])
                self.assertEqual(len(problems), 1)
                self.assertIn(f"'{op['name']}'", problems[0])

    def test_named_effectful_families_are_unmodeled_in_the_real_catalogue(self):
        by_name = {op["name"]: op for op in go.load_catalogue(REPO_ROOT)}
        for name in (
            "print_str",
            "print_i64",
            "print_u64",
            "fs_read",
            "process_run",
            "getenv",
            "mktemp_dir",
            "path_lookup",
        ):
            with self.subTest(op=name):
                self.assertIn(name, by_name)
                self.assertNotEqual(by_name[name]["effects"], "[]")
                self.assertEqual(by_name[name]["verifier_model"], "unmodeled")

    def test_real_pure_hash_builtins_say_what_the_native_verifier_models(self):
        by_name = {op["name"]: op for op in go.load_catalogue(REPO_ROOT)}
        for name, model in (("hash_u64", "known"), ("hash_str", "known")):
            with self.subTest(op=name):
                self.assertEqual(by_name[name]["effects"], "[]")
                self.assertEqual(by_name[name]["verifier_model"], model)

    def test_synthetic_pure_known_op_is_the_only_symbol_generated(self):
        pure = {
            "name": "pure_probe",
            "runtime_symbol": "__vow_pure_probe",
            "params": ["i64"],
            "return": "i64",
            "doc_signature": "fn(x: i64) -> i64",
            "effects": "[]",
            "verifier_model": "known",
            "c_model": "unmodeled",
        }
        ops = [_reasoned(dict(op, verifier_model="unmodeled")) for op in KNOWN_OPS]
        ops.append(pure)
        self.assertEqual(go.check_verifier_models(ops), [])
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            go.write_projections(ops, tmp, [])
            spliced = (tmp / "compiler" / "vc_ops.vow").read_text()
        block = spliced.split("fn vc_op_model() {}\n", 1)[1]
        fn = block.split("\n", 1)[1].rsplit("\n", 1)[0]
        self.assertTrue(_eval_vow_symbol_set(fn, "__vow_pure_probe"))
        for op in KNOWN_OPS:
            with self.subTest(op=op["name"]):
                self.assertFalse(_eval_vow_symbol_set(fn, op["runtime_symbol"]))
