#!/usr/bin/env python3
"""Behavior tests for scripts/pair_review.py.

The confirmation gate is the reason this harness is trustworthy, so the tests
concentrate on it: an unconfirmed claim must never be counted as a finding, and
a pair must never be reported as reviewed when it was skipped or only partly
reviewed.
"""

import contextlib
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


def usable_llm():
    """A provider double that satisfies `main`'s `--model` preflight."""
    return SimpleNamespace(make_config=lambda model: model)


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
    def test_units_and_preamble_partition_each_file_byte_for_byte(self):
        """Units tile the file in order; the preamble is exactly the rest."""
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
                cursor, residue = 0, []
                for unit in units:
                    start = text.find(unit.text, cursor)
                    self.assertGreaterEqual(start, cursor, unit.label)
                    residue.append(text[cursor:start])
                    cursor = start + len(unit.text)
                residue.append(text[cursor:])
                self.assertEqual(preamble, "".join(residue))

    def test_container_declarations_reach_every_chunk(self):
        # A method chunk with no `struct`/`impl` in front of it hands the model
        # a receiver it cannot see the fields of.
        preambles, _, _ = pair_review.load_pair_units("checker")
        context = "".join(text for _, text in preambles.rust)

        self.assertIn("pub struct Checker<'e>", context)
        self.assertIn("impl<'e> Checker<'e> {", context)

    def test_a_function_keeps_its_own_doc_comment(self):
        text = (
            "use std::fmt;\n\n"
            "/// Doc for one.\n"
            "#[inline]\n"
            "pub fn one() -> u32 { 1 }\n\n"
            "pub struct Held { field: u32 }\n\n"
            "/// Doc for two.\n"
            "fn two() -> u32 { 2 }\n"
        )
        preamble, units = pair_review.split_units(text, pair_review.RUST_FN)

        self.assertEqual(["one", "two"], [u.name for u in units])
        self.assertIn("/// Doc for one.", units[0].text)
        self.assertIn("#[inline]", units[0].text)
        self.assertIn("/// Doc for two.", units[1].text)
        self.assertIn("pub struct Held", preamble)
        self.assertNotIn("pub struct Held", units[0].text)

    def test_a_vow_contract_block_does_not_end_the_function(self):
        # `fn f() -> T vow { requires: ... } { body }` has two brace groups.
        # Stopping at the first leaves the contract as the unit and files the
        # body elsewhere -- the whole implementation missing from the review.
        text = (pair_review.REPO_ROOT / "compiler/lexer.vow").read_text()
        preamble, units = pair_review.split_units(text, pair_review.VOW_FN)
        contracted = next(u for u in units if u.name == "is_whitespace")

        self.assertIn("requires:", contracted.text)
        self.assertIn("b == 32 || b == 9 || b == 10 || b == 13", contracted.text)
        self.assertTrue(contracted.text.rstrip().endswith("}"))
        self.assertNotIn("b == 32", preamble)

    def test_a_signature_brace_is_not_the_body(self):
        cases = [
            "fn g(x: [u8; 4]) -> i32 { 1 }\n",
            "fn f() -> [u8; const { 1 }] { [0] }\n",
            "fn t(&self) -> u32;\n",
        ]
        for source in cases:
            with self.subTest(source=source.strip()):
                _, units = pair_review.split_units(source, pair_review.RUST_FN)

                self.assertEqual([source.strip()], [u.text.strip() for u in units])

    def test_an_item_the_scanner_cannot_end_is_loud(self):
        # A const-generic `Bar<{ N }>` needs a real parser. Truncating silently
        # would leave a named unit that still matches and still counts covered.
        with self.assertRaises(pair_review.UnsupportedItem):
            pair_review.split_units(
                "fn h() -> Bar<{ 1 }> { Bar }\n", pair_review.RUST_FN
            )

    def test_vow_split_finds_every_top_level_function(self):
        # Counted against the file rather than a literal: these modules gain
        # functions regularly, and a hardcoded total fails on someone else's
        # commit while proving nothing about the split.
        for relative in ("compiler/lower.vow", "compiler/lexer.vow"):
            with self.subTest(path=relative):
                text = (pair_review.REPO_ROOT / relative).read_text()
                declared = [m.group(1) for m in pair_review.VOW_FN.finditer(text)]
                _, units = pair_review.split_units(text, pair_review.VOW_FN)

                self.assertGreater(len(declared), 10, "file has no functions to find")
                self.assertEqual(declared, [unit.name for unit in units])

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

    def test_related_matches_a_trailing_qualifier(self):
        # The two compilers also disagree by a suffix one side adds.
        self.assertTrue(pair_review.related("lower_requires", "lower_requires_clauses"))
        self.assertTrue(pair_review.related("check_expr_inner", "check_expr"))
        self.assertFalse(pair_review.related("lower_requires", "lower_requireses"))

    def test_a_loose_match_never_strands_an_exact_one(self):
        # `lower_expr` matches `lower_expr_inner` by qualifier, but
        # `lower_expr_inner` matches it by name -- and the name must win, or
        # the exact counterpart is reviewed alone.
        names = ("lower_expr", "lower_expr_inner")
        rust = [pair_review.Unit(n, f"fn {n}() {{}}\n", "a.rs") for n in names]
        selves = [pair_review.Unit(n, f"fn {n}() {{}}\n", "a.vow") for n in names]

        groups = dict(
            (tuple(s.name for s in gs), tuple(r.name for r in gr))
            for gr, gs in pair_review.match_units(rust, selves)
        )

        self.assertEqual(("lower_expr",), groups[("lower_expr",)])
        self.assertEqual(("lower_expr_inner",), groups[("lower_expr_inner",)])

    def test_unmatched_units_are_named_and_lower_matched_coverage(self):
        # A chunk with both sides present still leaves a function uncompared
        # when nothing in it is that function's counterpart.
        chunk = pair_review.Chunk(
            rust_units=[
                pair_review.Unit("check_expr", "fn check_expr() {}\n", "a.rs"),
                pair_review.Unit(
                    "suggest_similar", "fn suggest_similar() {}\n", "a.rs"
                ),
            ],
            self_units=[
                pair_review.Unit("check_expr", "fn check_expr() {}\n", "b.vow")
            ],
        )

        self.assertEqual(1.0, pair_review._paired_coverage([chunk]))
        self.assertEqual(
            ["a.rs:suggest_similar"],
            [u.label for u in pair_review._unmatched([chunk])],
        )
        self.assertLess(pair_review._matched_coverage([chunk]), 1.0)

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
        # Signal 9, not 11: `compare_runtime` only writes the `both ...` shape
        # for a signal outside UNSAFE_SIGNALS/TRAP_SIGNALS, and SIGSEGV(11) is
        # routed to the two per-side details instead.
        divergences = [
            {
                "observable": "fail_closed",
                "detail": "both binaries died on unclassified signal 9",
            }
        ]

        self.assertTrue(pair_review._agreed_by_crashing(divergences))

    def test_a_both_detail_never_swallows_one_sided_evidence(self):
        # No producer mixes the two today, but the gate must not be the thing
        # standing between a real one-sided divergence and `confirmed`.
        divergences = [
            {
                "observable": "fail_closed",
                "detail": "both binaries died on unclassified signal 9",
            },
            {
                "observable": "fail_closed",
                "detail": "rust compiler panicked: 'panicked at'",
            },
        ]

        self.assertFalse(pair_review._agreed_by_crashing(divergences))

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


class CandidateDirectiveTest(unittest.TestCase):
    """A candidate the model wrote must not be able to steer its own judge."""

    @staticmethod
    def written_candidate(program):
        """The text `confirm` hands to scripts/equivalence.py."""
        seen = {}

        def fake_run(argv, **kwargs):
            seen["text"] = Path(argv[2]).read_text()
            raise StopIteration

        with mock.patch.object(pair_review.subprocess, "run", fake_run):
            with contextlib.suppress(StopIteration):
                pair_review.confirm(program, "rust", "self", 1)
        return seen["text"]

    @staticmethod
    def candidate_argv(program):
        """The argv `confirm` hands to scripts/equivalence.py."""
        seen = {}

        def fake_run(argv, **kwargs):
            seen["argv"] = argv
            raise StopIteration

        with (
            mock.patch.object(pair_review.subprocess, "run", fake_run),
            contextlib.suppress(StopIteration),
        ):
            pair_review.confirm(program, "rust", "self", 1)
        return seen["argv"]

    def test_the_c_emitter_pair_is_judged_on_the_verify_path_too(self):
        # The C emitters only run under `verify`; comparing `build` alone
        # refutes every claim about them without either one executing.
        calls = []

        def fake_confirm(program, rust, self_bin, timeout, verify_only=False):
            calls.append(verify_only)
            return "refuted", "agreed"

        with mock.patch.object(pair_review, "confirm", fake_confirm):
            pair_review.confirm_both_paths("module M\n", "rust", "self", 1)

        self.assertEqual([False, True], calls)

    def test_a_verify_path_confirmation_is_not_lost_to_the_build_path(self):
        with mock.patch.object(
            pair_review,
            "confirm",
            side_effect=[("refuted", "agreed"), ("confirmed", "status differs")],
        ):
            verdict, detail, unjudged = pair_review.confirm_both_paths(
                "module M\n", "rust", "self", 1
            )

        self.assertEqual("confirmed", verdict)
        self.assertIn("status differs", detail)
        self.assertIsNone(unjudged)

    def test_a_failed_verify_gate_is_reported_beside_a_build_finding(self):
        # The build path never executes either C emitter, so a build-only
        # divergence must not let the pair read as reviewed when the path that
        # does exercise them failed to run.
        with mock.patch.object(
            pair_review,
            "confirm",
            side_effect=[("confirmed", "exit differs"), ("error", "compile timeout")],
        ):
            verdict, detail, unjudged = pair_review.confirm_both_paths(
                "module M\n", "rust", "self", 1
            )

        self.assertEqual("confirmed", verdict)
        self.assertIn("exit differs", detail)
        self.assertIn("verify path did not run", unjudged)

    def test_a_raising_gate_costs_one_claim_not_the_run(self):
        llm = fake_llm(
            json.dumps(
                {
                    "findings": [
                        {"claim": "first", "program": "module M\n"},
                        {"claim": "second", "program": "module M\n"},
                    ]
                }
            )
        )

        def gate(*_):
            if not getattr(gate, "raised", False):
                gate.raised = True
                raise FileNotFoundError("target/release/vow")
            return "refuted", "agreed"

        with mock.patch.object(
            pair_review,
            "load_pair_units",
            return_value=ReviewReportTest.two_chunk_sources(),
        ):
            result = pair_review.review_pair(
                "lexer",
                "model",
                "rust",
                "self",
                600,
                1,
                max_chunks=1,
                llm_module=llm,
                confirm_fn=gate,
            )

        self.assertEqual(["first", "second"], [f["claim"] for f in result["findings"]])
        self.assertIn("gate raised", result["errors"][0]["error"])
        self.assertFalse(pair_review.reviewed_completely(result))

    def test_the_runner_is_told_to_ignore_directives(self):
        self.assertIn("--no-directives", self.candidate_argv("module M\n"))

    def test_the_judged_program_is_the_reported_one(self):
        # Byte-identical, directives included. Rewriting the text would make the
        # verdict describe a program the report does not contain.
        program = (
            "module M\n"
            '// TEST: skip "nothing to see"\n'
            "// TEST: stdin-file absent.txt\n"
            "fn main() -> i32 [io] { 0 }\n"
        )

        self.assertEqual(program, self.written_candidate(program))

    def test_a_directive_inside_a_string_is_program_data(self):
        # Vow strings admit literal newlines, so this line is a value, not a
        # comment -- deleting it would change what the program means.
        program = (
            'module M\nfn main() -> i32 [io] {\n  print("a\n// TEST: x");\n  0\n}\n'
        )

        self.assertEqual(program, self.written_candidate(program))

    def test_a_skipped_record_is_unjudged_not_a_hypothesis(self):
        # Whatever made the runner skip -- a dual compile timeout,
        # nondeterminism -- the observable the claim is about was never
        # compared, so the run must not read as complete.
        def fake_run(argv, **kwargs):
            outdir = Path(argv[argv.index("--output-dir") + 1])
            outdir.mkdir(parents=True, exist_ok=True)
            (outdir / "results.json").write_text(
                json.dumps(
                    {"records": [{"divergences": [], "skipped": "nondeterministic"}]}
                )
            )
            return SimpleNamespace(returncode=0, stdout="", stderr="")

        with mock.patch.object(pair_review.subprocess, "run", fake_run):
            verdict, detail = pair_review.confirm("module M\n", "rust", "self", 1)

        self.assertEqual("error", verdict)
        self.assertEqual("nondeterministic", detail)

    def test_an_agreed_comparison_is_still_refuted(self):
        def fake_run(argv, **kwargs):
            outdir = Path(argv[argv.index("--output-dir") + 1])
            outdir.mkdir(parents=True, exist_ok=True)
            (outdir / "results.json").write_text(
                json.dumps({"records": [{"divergences": [], "skipped": None}]})
            )
            return SimpleNamespace(returncode=0, stdout="", stderr="")

        with mock.patch.object(pair_review.subprocess, "run", fake_run):
            verdict, _ = pair_review.confirm("module M\n", "rust", "self", 1)

        self.assertEqual("refuted", verdict)


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

    def test_a_cap_lowers_coverage_but_not_paired_coverage(self):
        # Every checker chunk carries both sides, so a cap defers bytes without
        # showing any of them one-sided. Folding deferral into this metric would
        # print the one-sided warning about chunks nobody was shown.
        _, output, report = self.run_dry(
            "--pair", "checker", "--max-chunks-per-pair", "1"
        )

        result = report["pairs"][0]
        self.assertLess(result["coverage"], 1.0)
        self.assertEqual(1.0, result["paired_coverage"])
        self.assertNotIn("unpaired", output)

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

    def test_an_unrunnable_gate_is_an_error_not_a_hypothesis(self):
        # `confirm` returns `error` when equivalence.py could not judge the
        # program at all. Filed as a plain hypothesis it would leave the run
        # looking complete, stamp the ledger, and skip the pair next month.
        llm = fake_llm(
            json.dumps({"findings": [{"claim": "unjudged", "program": "module M\n"}]})
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
                max_chunks=1,
                llm_module=llm,
                confirm_fn=lambda *_: ("error", "runner examined no file"),
            )

        self.assertEqual("inconclusive", result["findings"][0]["verdict"])
        self.assertEqual(1, len(result["errors"]))
        self.assertIn("claim not judged", result["errors"][0]["error"])
        self.assertFalse(pair_review.reviewed_completely(result))

    def test_a_finding_without_a_program_is_also_unjudged(self):
        # The gate never ran on this claim either, so the same rule applies:
        # visible as a hypothesis, but the run is not complete.
        llm = fake_llm(json.dumps({"findings": [{"claim": "no program"}]}))
        with mock.patch.object(
            pair_review, "load_pair_units", return_value=self.two_chunk_sources()
        ):
            result = pair_review.review_pair(
                "lexer", "model", "rust", "self", 600, 1, max_chunks=1, llm_module=llm
            )

        self.assertEqual("inconclusive", result["findings"][0]["verdict"])
        self.assertIn("no program supplied", result["errors"][0]["error"])
        self.assertFalse(pair_review.reviewed_completely(result))

    def test_reviewed_completely_matches_the_ledger_gate(self):
        complete = {"coverage": 1.0, "chunks_deferred": [], "errors": []}

        self.assertTrue(pair_review.reviewed_completely(complete))
        self.assertFalse(
            pair_review.reviewed_completely({**complete, "chunks_deferred": [3]})
        )
        self.assertFalse(
            pair_review.reviewed_completely({**complete, "errors": [{"error": "x"}]})
        )
        self.assertFalse(pair_review.reviewed_completely({**complete, "coverage": 0.9}))

    def test_unparseable_chunk_does_not_lose_sibling_findings(self):
        llm = fake_llm(
            "not json",
            json.dumps({"findings": [{"claim": "kept", "program": "module M\n"}]}),
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
            # An unjudged claim must not retire the pair for a month.
            self.result(
                errors=[{"chunk_index": 1, "error": "confirmation gate did not run"}]
            ),
        ]
        for result in cases:
            with self.subTest(result=result):
                shutil.copyfile(pair_review.LEDGER, self.ledger_path)
                self.write(result)
                written = json.loads(self.ledger_path.read_text())
                self.assertEqual(
                    self.original["pairs"]["lexer"], written["pairs"]["lexer"]
                )

    def test_unmatched_coverage_alone_never_blocks_a_stamp(self):
        # `matched_coverage` measures asymmetry between the two implementations,
        # which no re-run can raise; gating on it would block every pair forever.
        self.write(self.result(matched_coverage=0.46, paired_coverage=0.96))

        written = json.loads(self.ledger_path.read_text())
        self.assertEqual("2026-09-01", written["pairs"]["lexer"]["last_reviewed"])

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
            mock.patch.dict("sys.modules", {"llm": usable_llm()}),
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

    def test_a_ledger_failure_still_writes_the_run_results(self):
        # results.json holds every finding from every model call this run made.
        # A ledger problem must not discard it, nor exit 1 -- the code that
        # means "confirmed findings".
        compiler = Path(self.directory.name) / "compiler"
        compiler.touch()
        outdir = Path(self.directory.name) / "ledger-blew-up"
        clean = self.result(
            plan={"chunk_bytes": 100, "chunks": []},
            chunks_reviewed=[1],
            input_tokens=0,
            output_tokens=0,
        )
        with (
            mock.patch.dict("sys.modules", {"llm": usable_llm()}),
            mock.patch.object(pair_review, "LEDGER", self.ledger_path),
            mock.patch.object(pair_review, "review_pair", return_value=clean),
            mock.patch.object(
                pair_review, "write_ledger", side_effect=ValueError("no pair entry")
            ),
            redirect_stdout(io.StringIO()),
            redirect_stderr(io.StringIO()),
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
                    str(outdir),
                    "--update-ledger",
                    "--date",
                    "2026-09-01",
                ]
            )

        report = json.loads((outdir / "results.json").read_text())
        self.assertEqual(2, status)
        self.assertEqual([], report["ledger_updated"])
        self.assertIn(
            "ledger writeback failed", report["pairs"][0]["errors"][0]["error"]
        )

    def test_a_failed_run_leaves_no_stale_results(self):
        # The documented output directory is a dated one that gets reused.
        outdir = Path(self.directory.name) / "reused"
        outdir.mkdir()
        (outdir / "results.json").write_text('{"confirmed": 7}\n')
        missing = Path(self.directory.name) / "not-a-compiler"
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            status = pair_review.main(
                [
                    "--all",
                    "--pair",
                    "lexer",
                    "--rust",
                    str(missing),
                    "--self",
                    str(missing),
                    "--output-dir",
                    str(outdir),
                ]
            )

        self.assertEqual(2, status)
        self.assertFalse((outdir / "results.json").exists())

    def test_an_unusable_model_exits_two(self):
        # Escaping as a traceback would exit 1 -- the code that means
        # "confirmed divergences" -- so a typo would report findings.
        compiler = Path(self.directory.name) / "compiler"
        compiler.touch()
        stub = SimpleNamespace(
            make_config=mock.Mock(side_effect=ValueError("Cannot infer provider"))
        )
        stderr = io.StringIO()
        with (
            mock.patch.dict("sys.modules", {"llm": stub}),
            redirect_stdout(io.StringIO()),
            redirect_stderr(stderr),
        ):
            status = pair_review.main(
                [
                    "--all",
                    "--pair",
                    "lexer",
                    "--model",
                    "nonesuch-9",
                    "--rust",
                    str(compiler),
                    "--self",
                    str(compiler),
                    "--output-dir",
                    str(Path(self.directory.name) / "bad-model"),
                ]
            )

        self.assertEqual(2, status)
        self.assertIn("nonesuch-9", stderr.getvalue())

    def test_an_errored_run_exits_two(self):
        # Exit 1 means "confirmed findings". A run whose chunks errored or left
        # a claim unjudged is not a verdict at all, so it takes the same
        # incomplete-run code scripts/equivalence.py uses for its coverage floor
        # -- exiting 0 would let a caller reading only the status take a run
        # that measured nothing for a clean bill of health.
        errored = self.result(
            plan={"chunk_bytes": 100, "chunks": []},
            chunks_reviewed=[1],
            input_tokens=0,
            output_tokens=0,
            errors=[{"chunk_index": 1, "error": "claim not judged: no program"}],
        )
        compiler = Path(self.directory.name) / "compiler"
        compiler.touch()
        with (
            mock.patch.dict("sys.modules", {"llm": usable_llm()}),
            mock.patch.object(pair_review, "LEDGER", self.ledger_path),
            mock.patch.object(pair_review, "review_pair", return_value=errored),
            redirect_stdout(io.StringIO()),
            redirect_stderr(io.StringIO()),
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
                    str(Path(self.directory.name) / "errored-out"),
                ]
            )

        self.assertEqual(2, status)


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
            # `skipped` is the gate failing to run, not a judgement on the
            # program -- it must not read as an ordinary hypothesis.
            ("skipped", "error"),
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
