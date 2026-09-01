#!/usr/bin/env python3
"""Behavior tests for scripts/pair_review.py.

The confirmation gate is the reason this harness is trustworthy, so the tests
concentrate on it: an unconfirmed claim must never be counted as a finding, and
a pair must never be reported as reviewed when it was skipped or only partly
reviewed.
"""

import io
import json
import re
import shutil
import tempfile
import unittest
from contextlib import nullcontext, redirect_stderr, redirect_stdout
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

import equivalence
import pair_review


def fake_llm(*contents):
    """A provider double that replays one canned response per chunk."""
    replies = iter(
        [
            SimpleNamespace(content=content, input_tokens=1, output_tokens=1)
            for content in contents
        ]
    )
    return SimpleNamespace(
        make_config=lambda model: model, chat=lambda *_: next(replies)
    )


def stamp_ledger(directory, pair):
    """Write a ledger marking `pair` reviewed at its current content hash."""
    rust_paths, self_path = pair_review.PAIRS[pair]
    path = Path(directory) / "ledger.json"
    entry = {
        "rust": rust_paths[0],
        "self_hosted": self_path,
        "content_hash": pair_review.hash_pair(rust_paths, self_path),
        "last_reviewed": "2026-08-01",
        "outcome": "clean",
    }
    path.write_text(json.dumps({"pairs": {pair: entry}}))
    return path


def run_dry(*argv, ledger=None):
    """Drive `main` through a dry run and return (status, stdout, results.json).

    `ledger` stamps that pair as already reviewed at its current content hash,
    which is the only input that makes a run skip a pair.
    """
    with tempfile.TemporaryDirectory() as directory:
        stamped = (
            mock.patch.object(pair_review, "LEDGER", stamp_ledger(directory, ledger))
            if ledger
            else nullcontext()
        )
        output = Path(directory) / "out"
        stdout = io.StringIO()
        with stamped, redirect_stdout(stdout):
            status = pair_review.main([*argv, "--dry-run", "--output-dir", str(output)])
        report = json.loads((output / "results.json").read_text())
        return status, stdout.getvalue(), report


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

    def test_matched_group_stays_whole_even_when_oversize(self):
        body = "x" * 80_000
        rust = [pair_review.Unit("lower_expr", f"fn lower_expr() {{{body}}}\n", "a.rs")]
        self_ = [
            pair_review.Unit("lower_expr", f"fn lower_expr() {{{body}}}\n", "a.vow")
        ]

        chunks = pair_review.plan_chunks(rust, self_, 50_000)

        self.assertEqual(1, len(chunks))
        self.assertEqual(["lower_expr"], [u.name for u in chunks[0].rust_units])
        self.assertEqual(["lower_expr"], [u.name for u in chunks[0].self_units])
        self.assertEqual(
            ["a.rs:lower_expr", "a.vow:lower_expr"], chunks[0].oversize_units
        )

    def test_real_lower_expr_counterparts_share_a_chunk(self):
        preambles, rust_units, self_units = pair_review.load_pair_units("lower")
        chunks = pair_review.plan_chunks(rust_units, self_units, 120_000, preambles)

        holding = [
            c
            for c in chunks
            if any(u.name == "lower_expr" for u in (*c.rust_units, *c.self_units))
        ]
        self.assertEqual(1, len(holding))
        self.assertTrue(any(u.name == "lower_expr" for u in holding[0].rust_units))
        self.assertTrue(any(u.name == "lower_expr" for u in holding[0].self_units))

    def test_lower_pair_chunk_count_is_bounded(self):
        preambles, rust_units, self_units = pair_review.load_pair_units("lower")

        chunks = pair_review.plan_chunks(rust_units, self_units, 120_000, preambles)

        self.assertLessEqual(len(chunks), 12)


class StripCfgTestTest(unittest.TestCase):
    def test_implementation_after_a_test_item_survives(self):
        # vow-ir/src/lower/mod.rs has `#[cfg(test)] fn ...` followed by real
        # lowering code; cutting at the first marker would drop it silently.
        text = (pair_review.REPO_ROOT / "vow-ir/src/lower/mod.rs").read_text()
        names = {
            m.group(1)
            for m in pair_review.RUST_FN.finditer(pair_review.strip_cfg_test(text))
        }

        self.assertIn("lower_module_with_pattern_aggregates", names)
        self.assertNotIn("lower_function", names)

    def test_raw_string_braces_do_not_end_a_test_module(self):
        # The test modules embed Vow programs whose braces sit at column 0.
        text = (
            "fn keep() {}\n"
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    #[test]\n"
            "    fn drop_me() {\n"
            '        let s = r#"\nstruct S {\n    id: i64,\n}\n"#;\n'
            "    }\n"
            "}\n"
            "fn also_keep() {}\n"
        )

        stripped = pair_review.strip_cfg_test(text)

        self.assertNotIn("#[test]", stripped)
        self.assertNotIn("drop_me", stripped)
        self.assertIn("fn keep()", stripped)
        self.assertIn("fn also_keep()", stripped)

    def test_declared_rust_sides_ship_no_test_functions(self):
        for name in pair_review.PAIRS:
            with self.subTest(pair=name):
                _, rust_units, _ = pair_review.load_pair_units(name)
                joined = "".join(unit.text for unit in rust_units)
                self.assertNotIn("#[test]", joined)
                self.assertNotIn("#[cfg(test)]", joined)

    def test_lexer_pairing_is_no_longer_captured_by_a_test_helper(self):
        _, rust_units, _ = pair_review.load_pair_units("lexer")
        names = [unit.name for unit in rust_units]

        self.assertIn("tokenize", names)
        self.assertNotIn("lex", names)


class FailClosedGateTest(unittest.TestCase):
    @staticmethod
    def panic(side):
        return equivalence.check_fail_closed(
            side, {"stderr": "thread 'main' panicked at src/x.rs", "exit": 101}
        )

    def test_both_compilers_crashing_is_not_a_divergence(self):
        divergences = self.panic("rust") + self.panic("self-hosted")

        self.assertTrue(pair_review._agreed_by_crashing(divergences))

    def test_one_compiler_crashing_still_counts(self):
        self.assertFalse(pair_review._agreed_by_crashing(self.panic("rust")))

    def test_a_real_observable_is_never_masked(self):
        divergences = self.panic("rust") + self.panic("self-hosted")
        divergences.append({"observable": "error_code", "detail": "E0001 vs E0002"})

        self.assertFalse(pair_review._agreed_by_crashing(divergences))

    def test_shared_signal_death_is_agreement(self):
        divergences = [
            {
                "observable": "fail_closed",
                "detail": "both binaries died on unclassified signal 11",
            }
        ]

        self.assertTrue(pair_review._agreed_by_crashing(divergences))

    def test_shared_missing_json_is_agreement(self):
        divergences = [
            {
                "observable": "fail_closed",
                "detail": f"{side} emitted no parseable JSON (exit 1)",
            }
            for side in ("rust", "self-hosted")
        ]

        self.assertTrue(pair_review._agreed_by_crashing(divergences))

    def test_shared_binary_memory_unsafety_is_agreement(self):
        divergences = [
            {
                "observable": "fail_closed",
                "detail": (
                    f"{side} binary died on SIGSEGV (11) "
                    "\u2014 memory unsafety, not a trap"
                ),
            }
            for side in ("rust", "self-hosted")
        ]

        self.assertTrue(pair_review._agreed_by_crashing(divergences))

    def test_one_sided_timeout_is_still_a_divergence(self):
        divergences = [
            {
                "observable": "fail_closed",
                "detail": "rust compiler timed out after 120s; self-hosted completed",
            }
        ]

        self.assertFalse(pair_review._agreed_by_crashing(divergences))

    def test_every_equivalence_fail_closed_shape_names_a_side(self):
        """The gate reads details equivalence.py writes; keep the two in step."""
        source = Path(equivalence.__file__).read_text()
        shapes = re.findall(
            r'"observable": "fail_closed",\s*"detail": \(?\s*(?:f?")(.*?)"',
            source,
            re.DOTALL,
        )
        self.assertGreaterEqual(len(shapes), 5)
        for shape in shapes:
            detail = shape.replace("{name}", "rust").replace("{hung}", "rust")
            self.assertTrue(
                pair_review.FAIL_CLOSED_SIDE.match(detail)
                or detail.startswith("both "),
                f"unrecognised fail_closed shape: {detail!r}",
            )


class RenderChunkTest(unittest.TestCase):
    def test_both_sides_are_labelled(self):
        chunk = pair_review.Chunk(
            rust_units=[pair_review.Unit("r", "fn r() {}\n", "r.rs")],
            self_units=[pair_review.Unit("v", "fn v() {}\n", "v.vow")],
        )
        body = pair_review.render_chunk(chunk, None, 1, 1)

        self.assertIn("=== RUST:", body)
        self.assertIn("=== SELF-HOSTED:", body)

    def test_summed_parts_equal_the_rendered_byte_count(self):
        # plan_chunks budgets chunks by summing part sizes instead of rendering
        # them. If that arithmetic drifts from the renderer, the planner ships
        # prompts over the model's context budget.
        preambles, rust_units, self_units = pair_review.load_pair_units("lexer")
        chunks = pair_review.plan_chunks(rust_units, self_units, 40_000, preambles)

        for index, chunk in enumerate(chunks, 1):
            with self.subTest(chunk=index):
                parts = pair_review._chunk_parts(chunk, preambles, index, len(chunks))
                summed = sum(len(part.encode()) for part in parts) + len(
                    pair_review.JOIN.encode()
                ) * (len(parts) - 1)
                rendered = pair_review.render_chunk(
                    chunk, preambles, index, len(chunks)
                )
                self.assertEqual(len(rendered.encode()), summed)


class ReviewReportTest(unittest.TestCase):
    def run_dry(self, *extra):
        return run_dry("--all", *extra)

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

    def test_paired_coverage_is_one_when_every_chunk_has_both_sides(self):
        _, _, report = self.run_dry("--pair", "checker")

        self.assertEqual(1.0, report["pairs"][0]["paired_coverage"])

    def test_single_sided_chunks_lower_paired_coverage_and_are_reported(self):
        body = "x" * 40_000
        rust = [pair_review.Unit("only_rust", f"fn only_rust() {{{body}}}\n", "a.rs")]
        self_ = [pair_review.Unit("only_vow", f"fn only_vow() {{{body}}}\n", "a.vow")]
        with mock.patch.object(
            pair_review,
            "load_pair_units",
            return_value=(pair_review.Preambles(), rust, self_),
        ):
            _, output, report = self.run_dry(
                "--pair", "lexer", "--chunk-bytes", "45000"
            )

        self.assertEqual(0.0, report["pairs"][0]["paired_coverage"])
        self.assertIn("unpaired", output)

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
        llm = fake_llm(
            json.dumps({"findings": [{"claim": "first", "program": "module M\n"}]}),
            json.dumps({"findings": [{"claim": "second", "program": "module M\n"}]}),
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
                llm_module=llm,
                confirm_fn=lambda *_: ("refuted", "agreed"),
            )

        self.assertEqual([1, 2], [f["chunk_index"] for f in result["findings"]])

    def test_unparseable_chunk_does_not_lose_sibling_findings(self):
        llm = fake_llm(
            "not json",
            json.dumps({"findings": [{"claim": "kept", "program": ""}]}),
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
                llm_module=llm,
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

    def test_concurrent_triage_edit_is_not_clobbered(self):
        # A review runs for minutes; the ledger it loaded at startup is stale by
        # the time it writes.
        stale = json.loads(self.ledger_path.read_text())
        live = json.loads(self.ledger_path.read_text())
        live["pairs"]["parser"]["confirmed_issues"] = [4242]
        self.ledger_path.write_text(json.dumps(live))

        pair_review.write_ledger(stale, [self.result()], "2026-09-01", self.ledger_path)

        written = json.loads(self.ledger_path.read_text())
        self.assertEqual([4242], written["pairs"]["parser"]["confirmed_issues"])
        self.assertEqual("2026-09-01", written["pairs"]["lexer"]["last_reviewed"])

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

    def test_soundness_pair_gate_checks_both_compilers(self):
        with mock.patch.object(
            pair_review,
            "confirm_soundness",
            side_effect=[
                ("refuted", "no Rust violation"),
                ("confirmed", "self-hosted false proof"),
            ],
        ) as gate:
            verdict, detail = pair_review.confirm_soundness_pair(
                "module M\n", "rust-vow", "self-vow", 7
            )

        self.assertEqual("confirmed", verdict)
        self.assertIn("rust: refuted", detail)
        self.assertIn("self-hosted: confirmed", detail)
        self.assertEqual(
            [
                mock.call("module M\n", "rust-vow", 7),
                mock.call("module M\n", "self-vow", 7),
            ],
            gate.call_args_list,
        )

    def test_soundness_ignores_the_equivalence_ledger(self):
        # Soundness runs never stamp the ledger, so an equivalence stamp must
        # not make them skip -- that would exit 0 having asked nothing.
        _, _, report = run_dry("--mode", "soundness", ledger="c_emitter")

        self.assertEqual(["c_emitter"], report["planned"])
        self.assertEqual([], report["skipped_unchanged"])

    def test_equivalence_still_skips_an_unchanged_stamped_pair(self):
        _, _, report = run_dry("--pair", "lexer", ledger="lexer")

        self.assertEqual(["lexer"], report["skipped_unchanged"])
        self.assertEqual([], report["planned"])

    def test_soundness_rejects_a_pair_it_does_not_cover(self):
        with (
            self.assertRaises(SystemExit),
            redirect_stdout(io.StringIO()),
            redirect_stderr(io.StringIO()),
        ):
            pair_review.main(["--mode", "soundness", "--pair", "lower", "--dry-run"])

    def test_soundness_mode_defaults_to_c_emitter_pair(self):
        status, _, report = run_dry("--mode", "soundness", "--all")

        self.assertEqual(0, status)
        self.assertEqual("soundness", report["mode"])
        self.assertEqual(["c_emitter"], report["planned"])
        self.assertEqual("soundness", report["pairs"][0]["mode"])


if __name__ == "__main__":
    unittest.main()
