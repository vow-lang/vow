#!/usr/bin/env python3
"""Behavior tests for scripts/pair_review.py.

The confirmation gate is the reason this harness is trustworthy, so the tests
concentrate on it: an unconfirmed claim must never be counted as a finding, and
a pair must never be reported as reviewed when it was skipped or truncated.
"""

import io
import json
import shutil
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


class LedgerWritebackTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.ledger_path = Path(self.directory.name) / "ledger.json"
        shutil.copyfile(pair_review.LEDGER, self.ledger_path)
        self.original = json.loads(self.ledger_path.read_text())

    @staticmethod
    def result(findings=None, **overrides):
        result = {
            "pair": "lexer",
            "content_hash": "f" * 64,
            "coverage": 1.0,
            "chunks_deferred": [],
            "errors": [],
            "findings": findings or [],
        }
        result.update(overrides)
        return result

    def write(self, result):
        ledger = json.loads(self.ledger_path.read_text())
        return pair_review.write_ledger(
            ledger, [result], "2026-09-01", self.ledger_path
        )

    def test_writeback_stamps_hash_date_and_clean_outcome(self):
        self.write(self.result())

        written = json.loads(self.ledger_path.read_text())
        entry = written["pairs"]["lexer"]
        self.assertEqual("f" * 64, entry["content_hash"])
        self.assertEqual("2026-09-01", entry["last_reviewed"])
        self.assertEqual("clean", entry["outcome"])
        self.assertEqual("2026-09-01", written["updated"])

    def test_outcome_reflects_strongest_verdict(self):
        cases = [
            ([{"verdict": "confirmed"}], "confirmed"),
            ([{"verdict": "inconclusive"}], "hypotheses"),
            ([{"verdict": "refuted"}], "clean"),
        ]
        for findings, expected in cases:
            with self.subTest(expected=expected):
                shutil.copyfile(pair_review.LEDGER, self.ledger_path)
                self.write(self.result(findings))
                written = json.loads(self.ledger_path.read_text())
                self.assertEqual(expected, written["pairs"]["lexer"]["outcome"])

    def test_partially_reviewed_pair_is_not_stamped(self):
        cases = [
            self.result(chunks_deferred=[2]),
            self.result(errors=[{"chunk_index": 1, "error": "bad JSON"}]),
            self.result(coverage=0.5),
        ]
        for result in cases:
            with self.subTest(result=result):
                shutil.copyfile(pair_review.LEDGER, self.ledger_path)
                self.write(result)
                written = json.loads(self.ledger_path.read_text())
                self.assertEqual(
                    self.original["pairs"]["lexer"], written["pairs"]["lexer"]
                )

    def test_corpus_and_untouched_pairs_survive(self):
        self.write(self.result())

        written = json.loads(self.ledger_path.read_text())
        self.assertEqual(self.original["corpus"], written["corpus"])
        self.assertEqual(self.original["pairs"]["parser"], written["pairs"]["parser"])

    def test_written_entry_matches_schema_key_set(self):
        self.write(self.result())

        schema = json.loads(
            (pair_review.REPO_ROOT / "docs/equivalence/ledger.schema.json").read_text()
        )
        pair_schema = schema["properties"]["pairs"]["additionalProperties"]
        entry = json.loads(self.ledger_path.read_text())["pairs"]["lexer"]
        self.assertLessEqual(set(pair_schema["required"]), set(entry))
        self.assertLessEqual(set(entry), set(pair_schema["properties"]))

    def test_writeback_is_off_by_default(self):
        before = self.ledger_path.read_bytes()
        fake_result = self.result(
            truncated=False,
            plan={"chunk_bytes": 100, "chunks": []},
            chunks_reviewed=[],
            input_tokens=0,
            output_tokens=0,
        )
        compiler = Path(self.directory.name) / "compiler"
        compiler.touch()
        output = Path(self.directory.name) / "output"
        with (
            mock.patch.object(pair_review, "LEDGER", self.ledger_path),
            mock.patch.object(pair_review, "review_pair", return_value=fake_result),
            redirect_stdout(io.StringIO()),
        ):
            status = pair_review.main(
                [
                    "--all",
                    "--pair",
                    "lexer",
                    "--rust",
                    str(compiler),
                    "--self",
                    str(compiler),
                    "--output-dir",
                    str(output),
                ]
            )

        self.assertEqual(0, status)
        self.assertEqual(before, self.ledger_path.read_bytes())


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


class SoundnessModeTest(unittest.TestCase):
    def test_soundness_prompt_asks_about_assume_narrowing(self):
        self.assertIn("__ESBMC_assume", pair_review.SYSTEM_SOUNDNESS)
        self.assertIn("proves", pair_review.SYSTEM_SOUNDNESS)
        self.assertIn("permits", pair_review.SYSTEM_SOUNDNESS)

    def test_soundness_prompt_demands_module_header_and_permits_empty_answer(self):
        self.assertIn("module M", pair_review.SYSTEM_SOUNDNESS)
        self.assertIn("empty findings list", pair_review.SYSTEM_SOUNDNESS)

    def test_soundness_verdict_mapping(self):
        cases = [
            ("SOUNDNESS", "confirmed"),
            ("ok", "refuted"),
            ("not-applicable", "inconclusive"),
            ("skipped", "inconclusive"),
        ]
        for runner_verdict, expected in cases:
            with (
                self.subTest(runner_verdict=runner_verdict),
                mock.patch.object(
                    pair_review,
                    "check_soundness",
                    return_value={
                        "verdict": runner_verdict,
                        "detail": "runner detail",
                    },
                ),
            ):
                verdict, detail = pair_review.confirm_soundness("module M\n", "vow", 1)
                self.assertEqual(expected, verdict)
                self.assertEqual("runner detail", detail)

    def test_soundness_mode_defaults_to_c_emitter_pair(self):
        with tempfile.TemporaryDirectory() as directory:
            with redirect_stdout(io.StringIO()):
                status = pair_review.main(
                    [
                        "--mode",
                        "soundness",
                        "--dry-run",
                        "--all",
                        "--output-dir",
                        directory,
                    ]
                )
            report = json.loads((Path(directory) / "results.json").read_text())

        self.assertEqual(0, status)
        self.assertEqual("soundness", report["mode"])
        self.assertEqual(["c_emitter"], report["planned"])
        self.assertEqual("soundness", report["pairs"][0]["mode"])


if __name__ == "__main__":
    unittest.main()
