#!/usr/bin/env python3
"""Behavior tests for scripts/pair_review.py.

The confirmation gate is the reason this harness is trustworthy, so the tests
concentrate on it: an unconfirmed claim must never be counted as a finding, and
a pair must never be reported as reviewed when it was skipped or truncated.
"""

import io
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

import pair_review


class PairSpecTest(unittest.TestCase):
    def test_every_declared_pair_exists_on_disk(self):
        # A typo'd path would silently review nothing.
        for name, (rust_paths, self_path) in pair_review.PAIRS.items():
            with self.subTest(pair=name):
                self.assertTrue((pair_review.REPO_ROOT / self_path).exists(), self_path)
                for spec in rust_paths:
                    self.assertTrue((pair_review.REPO_ROOT / spec).exists(), spec)

    def test_pair_hash_changes_when_self_hosted_side_changes(self):
        rust_paths, self_path = pair_review.PAIRS["lexer"]
        before = pair_review.hash_pair(rust_paths, self_path)
        p = pair_review.REPO_ROOT / self_path
        original = p.read_bytes()
        try:
            p.write_bytes(original + b"\n// touched\n")
            after = pair_review.hash_pair(rust_paths, self_path)
        finally:
            p.write_bytes(original)

        self.assertNotEqual(before, after)

    def test_pair_hash_is_stable_across_calls(self):
        rust_paths, self_path = pair_review.PAIRS["c_emitter"]

        self.assertEqual(
            pair_review.hash_pair(rust_paths, self_path),
            pair_review.hash_pair(rust_paths, self_path),
        )

    def test_directory_pair_hashes_every_rust_file(self):
        # `parser` and `lower` are directories on the Rust side; a hash that
        # only covered mod.rs would miss a change in expr.rs.
        rust_paths, self_path = pair_review.PAIRS["parser"]
        target = pair_review.REPO_ROOT / "vow-syntax/src/parser/expr.rs"
        before = pair_review.hash_pair(rust_paths, self_path)
        original = target.read_bytes()
        try:
            target.write_bytes(original + b"\n// touched\n")
            after = pair_review.hash_pair(rust_paths, self_path)
        finally:
            target.write_bytes(original)

        self.assertNotEqual(before, after)


class SplitUnitsTest(unittest.TestCase):
    def test_units_reassemble_each_file_byte_for_byte(self):
        cases = [
            ("compiler/lower.vow", pair_review.VOW_FN),
            ("compiler/c_emitter.vow", pair_review.VOW_FN),
            ("vow-ir/src/lower/mod.rs", pair_review.RUST_FN),
            ("vow-types/src/check.rs", pair_review.RUST_FN),
        ]
        for relative, pattern in cases:
            with self.subTest(path=relative):
                text = (pair_review.REPO_ROOT / relative).read_text()
                preamble, units = pair_review.split_units(text, pattern)
                self.assertEqual(text, preamble + "".join(u.text for u in units))

    def test_vow_split_finds_every_top_level_function(self):
        cases = [("compiler/lower.vow", 135), ("compiler/lexer.vow", 14)]
        for relative, expected in cases:
            with self.subTest(path=relative):
                text = (pair_review.REPO_ROOT / relative).read_text()
                _, units = pair_review.split_units(text, pair_review.VOW_FN)
                self.assertEqual(expected, len(units))

    def test_rust_split_finds_free_functions_and_impl_methods(self):
        text = (pair_review.REPO_ROOT / "vow-ir/src/lower/mod.rs").read_text()
        _, units = pair_review.split_units(text, pair_review.RUST_FN)

        names = [unit.name for unit in units]
        self.assertIn("lower_expr", names)
        self.assertIn("merge_inst_ty", names)

    def test_preamble_holds_leading_declarations(self):
        text = (pair_review.REPO_ROOT / "compiler/lower.vow").read_text()
        preamble, _ = pair_review.split_units(text, pair_review.VOW_FN)

        self.assertTrue(preamble)
        self.assertNotIn("\nfn ", preamble)


class ChunkPlanTest(unittest.TestCase):
    def test_related_matches_receiver_prefix_convention(self):
        self.assertTrue(pair_review.related("lctx_merge_inst_ty", "merge_inst_ty"))
        self.assertTrue(pair_review.related("lower_expr", "lower_expr"))
        self.assertFalse(pair_review.related("lower_expr", "lower_stmt"))

    def test_every_real_lower_unit_lands_in_exactly_one_chunk(self):
        preambles, rust_units, self_units = pair_review.load_pair_units("lower")
        chunks = pair_review.plan_chunks(rust_units, self_units, 120_000, preambles)

        self.assertEqual(len(rust_units), sum(len(c.rust_units) for c in chunks))
        self.assertEqual(len(self_units), sum(len(c.self_units) for c in chunks))
        self.assertEqual(
            sorted((u.source, u.name, u.text) for u in rust_units),
            sorted((u.source, u.name, u.text) for c in chunks for u in c.rust_units),
        )
        self.assertEqual(
            sorted((u.source, u.name, u.text) for u in self_units),
            sorted((u.source, u.name, u.text) for c in chunks for u in c.self_units),
        )

    def test_unmatched_rust_unit_gets_its_own_chunk(self):
        rust_units = [pair_review.Unit("rust_only", "fn rust_only() {}\n", "a.rs")]
        self_units = [pair_review.Unit("vow_only", "fn vow_only() {}\n", "a.vow")]

        chunks = pair_review.plan_chunks(rust_units, self_units, 10_000)

        self.assertEqual(["rust_only"], [u.name for c in chunks for u in c.rust_units])

    def test_chunks_respect_rendered_byte_budget(self):
        preambles, rust_units, self_units = pair_review.load_pair_units("lower")
        chunks = pair_review.plan_chunks(rust_units, self_units, 40_000, preambles)

        for index, chunk in enumerate(chunks, 1):
            with self.subTest(chunk=index):
                rendered = pair_review.render_chunk(
                    chunk, preambles, index, len(chunks)
                )
                self.assertTrue(
                    len(rendered.encode()) <= 40_000 or chunk.oversize_units,
                    len(rendered.encode()),
                )

    def test_oversize_unit_is_reported_and_preserved(self):
        text = "fn huge() {\n" + ("x" * 200_000) + "\n}\n"
        units = [pair_review.Unit("huge", text, "huge.vow")]

        chunks = pair_review.plan_chunks([], units, 50_000)

        self.assertEqual(1, len(chunks))
        self.assertEqual(["huge.vow:huge"], chunks[0].oversize_units)
        self.assertIn(text, pair_review.render_chunk(chunks[0], None, 1, 1))

    def test_lower_pair_chunk_count_is_bounded(self):
        preambles, rust_units, self_units = pair_review.load_pair_units("lower")

        chunks = pair_review.plan_chunks(rust_units, self_units, 120_000, preambles)

        self.assertLessEqual(len(chunks), 12)


class RenderChunkTest(unittest.TestCase):
    def test_both_sides_are_labelled(self):
        chunk = pair_review.Chunk(
            rust_units=[pair_review.Unit("r", "fn r() {}\n", "r.rs")],
            self_units=[pair_review.Unit("v", "fn v() {}\n", "v.vow")],
        )
        body = pair_review.render_chunk(chunk, None, 1, 1)

        self.assertIn("=== RUST:", body)
        self.assertIn("=== SELF-HOSTED:", body)


class ReviewReportTest(unittest.TestCase):
    def run_dry(self, *extra):
        with tempfile.TemporaryDirectory() as directory:
            output = io.StringIO()
            with redirect_stdout(output):
                status = pair_review.main(
                    [
                        "--dry-run",
                        "--all",
                        "--output-dir",
                        directory,
                        *extra,
                    ]
                )
            report = json.loads((Path(directory) / "results.json").read_text())
            return status, output.getvalue(), report

    def test_dry_run_emits_all_five_chunk_plans_without_model_calls(self):
        with mock.patch.dict("sys.modules", {"llm": None}):
            status, _, report = self.run_dry()

        self.assertEqual(0, status)
        self.assertEqual(set(pair_review.PAIRS), {p["pair"] for p in report["pairs"]})
        self.assertTrue(all(p["plan"]["chunks"] for p in report["pairs"]))
        self.assertEqual([], report["reviewed"])

    def test_coverage_is_one_when_nothing_is_deferred(self):
        _, _, report = self.run_dry()

        self.assertTrue(all(p["coverage"] == 1.0 for p in report["pairs"]))
        self.assertTrue(all(not p["truncated"] for p in report["pairs"]))

    def test_deferred_chunks_are_reported_and_coverage_drops(self):
        _, output, report = self.run_dry(
            "--pair", "lower", "--max-chunks-per-pair", "2"
        )

        result = report["pairs"][0]
        self.assertTrue(result["chunks_deferred"])
        self.assertLess(result["coverage"], 1.0)
        self.assertIn("deferred", output)

    @staticmethod
    def two_chunk_sources():
        units = [
            pair_review.Unit("one", "fn one() {\n" + "x" * 500 + "\n}\n", "x.vow"),
            pair_review.Unit("two", "fn two() {\n" + "y" * 500 + "\n}\n", "x.vow"),
        ]
        return pair_review.Preambles(), [], units

    def test_findings_carry_their_chunk_index(self):
        replies = iter(
            [
                SimpleNamespace(
                    content=json.dumps(
                        {"findings": [{"claim": "first", "program": "module M\n"}]}
                    ),
                    input_tokens=10,
                    output_tokens=5,
                ),
                SimpleNamespace(
                    content=json.dumps(
                        {"findings": [{"claim": "second", "program": "module M\n"}]}
                    ),
                    input_tokens=10,
                    output_tokens=5,
                ),
            ]
        )
        fake_llm = SimpleNamespace(
            make_config=lambda model: model,
            chat=lambda *_: next(replies),
        )
        with mock.patch.object(
            pair_review, "load_pair_units", return_value=self.two_chunk_sources()
        ):
            result = pair_review.review_pair(
                "lexer",
                "model",
                "rust",
                "self",
                600,
                1,
                llm_module=fake_llm,
                confirm_fn=lambda *_: ("refuted", "agreed"),
            )

        self.assertEqual([1, 2], [f["chunk_index"] for f in result["findings"]])

    def test_unparseable_chunk_does_not_lose_sibling_findings(self):
        replies = iter(
            [
                SimpleNamespace(content="not json", input_tokens=1, output_tokens=1),
                SimpleNamespace(
                    content=json.dumps(
                        {"findings": [{"claim": "kept", "program": ""}]}
                    ),
                    input_tokens=1,
                    output_tokens=1,
                ),
            ]
        )
        fake_llm = SimpleNamespace(
            make_config=lambda model: model,
            chat=lambda *_: next(replies),
        )
        with mock.patch.object(
            pair_review, "load_pair_units", return_value=self.two_chunk_sources()
        ):
            result = pair_review.review_pair(
                "lexer",
                "model",
                "rust",
                "self",
                600,
                1,
                llm_module=fake_llm,
            )

        self.assertEqual(1, len(result["errors"]))
        self.assertEqual("kept", result["findings"][0]["claim"])


class SystemPromptTest(unittest.TestCase):
    def test_prompt_demands_a_module_header(self):
        # Without this the model emits header-less programs that the Rust
        # compiler rejects outright, and every candidate comes back as an
        # accept/reject divergence for the wrong reason.
        self.assertIn("module M", pair_review.SYSTEM)

    def test_prompt_says_only_error_codes_are_compared(self):
        self.assertIn("error CODE", pair_review.SYSTEM)

    def test_prompt_allows_an_empty_answer(self):
        # A model pushed to produce findings will invent them.
        self.assertIn("empty findings list", pair_review.SYSTEM)


if __name__ == "__main__":
    unittest.main()
