#!/usr/bin/env python3
"""Behavior tests for scripts/verify_diff_explain.py and its use by verify_diff."""

import os
import tempfile
import unittest

import verify_diff as vd
import verify_diff_explain as ex

REAL = ("first", "Callee", 1)
ARTEFACT = ("first", "None", 4294967293)
OTHER = ("first", "Caller", 7)
ARTEFACTS = {ARTEFACT: "ESBMC reads a Vec field afresh at each use"}


def fixture(body):
    d = tempfile.mkdtemp()
    path = os.path.join(d, "f.vow")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(body)
    return path


class ArtefactOfARefutedFixtureTest(unittest.TestCase):
    def classify(self, esbmc_keys, native_keys, truth="VerifyFailed", artifacts=None):
        return vd.classify(
            truth,
            vd.REFUTED,
            vd.REFUTED,
            frozenset(esbmc_keys),
            frozenset(native_keys),
            model_artifacts=ARTEFACTS if artifacts is None else artifacts,
        )

    def test_a_declared_extra_esbmc_counterexample_is_more_precise(self):
        cls, detail = self.classify({ARTEFACT}, {REAL})
        self.assertEqual(vd.MORE_PRECISE, cls)
        self.assertIn("ESBMC reads a Vec field afresh", detail)

    def test_without_the_directive_the_row_stays_weaker(self):
        cls, detail = self.classify({ARTEFACT}, {REAL}, artifacts={})
        self.assertEqual(vd.WEAKER, cls)
        self.assertIn("lacks ESBMC counterexample", detail)

    def test_an_undeclared_missing_counterexample_stays_weaker(self):
        self.assertEqual(vd.WEAKER, self.classify({ARTEFACT, OTHER}, {REAL})[0])

    def test_an_artefact_in_a_function_native_does_not_refute_stays_weaker(self):
        self.assertEqual(
            vd.WEAKER, self.classify({ARTEFACT}, {("other_fn", "Callee", 1)})[0]
        )

    def test_a_declared_counterexample_does_not_excuse_another_one(self):
        self.assertEqual(vd.WEAKER, self.classify({OTHER}, {REAL})[0])

    def test_a_correctly_labelled_program_is_not_excused(self):
        self.assertEqual(vd.WEAKER, self.classify({ARTEFACT}, {REAL}, "Verified")[0])

    def test_matching_counterexamples_still_match(self):
        self.assertEqual(vd.MATCH, self.classify({REAL}, {REAL})[0])

    def test_a_declared_counterexample_both_report_is_a_match(self):
        self.assertEqual(vd.MATCH, self.classify({ARTEFACT}, {ARTEFACT})[0])

    def test_extra_native_counterexample_is_more_precise(self):
        self.assertEqual(vd.MORE_PRECISE, self.classify({REAL}, {REAL, OTHER})[0])


class NativeGapDirectiveTest(unittest.TestCase):
    GAP = '// TEST: native-gap "map phi merge is not modelled" #1849\n'

    def test_parses_reason_and_issue(self):
        path = fixture("// TEST: category x\n" + self.GAP)
        self.assertEqual(("map phi merge is not modelled", 1849), ex.native_gap(path))

    def test_no_directive_is_none(self):
        self.assertIsNone(ex.native_gap(fixture("fn main() {}\n")))
        self.assertIsNone(ex.native_gap("/nonexistent/f.vow"))

    def test_a_malformed_directive_is_an_error(self):
        for line in (
            "// TEST: native-gap\n",
            '// TEST: native-gap "why"\n',
            "// TEST: native-gap #12\n",
            '// TEST: native-gap "why" #abc\n',
            '// TEST: native-gap "" #12\n',
            '// TEST: native-gap "why" 12\n',
        ):
            with self.assertRaises(ValueError, msg=line):
                ex.native_gap(fixture(line))

    def test_a_second_directive_is_an_error(self):
        with self.assertRaises(ValueError):
            ex.native_gap(fixture(self.GAP + self.GAP))

    def test_an_indented_directive_is_read(self):
        self.assertEqual(
            ("why", 3), ex.native_gap(fixture('  // TEST: native-gap "why" #3\n'))
        )

    def test_every_directive_of_the_corpus_is_well_formed(self):
        root = os.path.join(vd.REPO_ROOT, "tests")
        found = 0
        for base, _, names in os.walk(root):
            for name in names:
                if name.endswith(".vow") and ex.native_gap(os.path.join(base, name)):
                    found += 1
        self.assertGreater(found, 0)

    def test_the_directive_name_is_a_word(self):
        self.assertIsNone(ex.native_gap(fixture('// TEST: native-gaps "why" #1\n')))


class TrackTest(unittest.TestCase):
    def setUp(self):
        self.documented = fixture(NativeGapDirectiveTest.GAP)
        self.bare = fixture("fn main() {}\n")

    def test_a_documented_less_conclusive_row_is_tracked(self):
        for native in (vd.SKIPPED, vd.INCONCLUSIVE):
            cls, detail = ex.track(
                vd.WEAKER, "native is skipped", native, None, self.documented
            )
            self.assertEqual(vd.TRACKED, cls)
            self.assertIn("#1849", detail)
            self.assertIn("native is skipped", detail)

    def test_an_undocumented_row_stays_weaker(self):
        self.assertEqual(
            (vd.WEAKER, "d"), ex.track(vd.WEAKER, "d", vd.SKIPPED, None, self.bare)
        )

    def test_only_weaker_rows_are_tracked(self):
        for cls in (vd.MATCH, vd.MORE_PRECISE, vd.SOUNDNESS, vd.HARNESS):
            self.assertEqual(
                (cls, "d"), ex.track(cls, "d", vd.SKIPPED, None, self.documented)
            )

    def test_a_native_refutation_is_never_a_documented_gap(self):
        for native in (vd.REFUTED, vd.PROVEN, vd.ERROR):
            self.assertEqual(
                (vd.WEAKER, "d"),
                ex.track(vd.WEAKER, "d", native, None, self.documented),
            )

    def test_a_crash_a_panic_an_error_and_a_timeout_are_never_a_documented_gap(self):
        for status in ("crashed", "panicked", "error", "timeout"):
            self.assertEqual(
                (vd.WEAKER, "d"),
                ex.track(vd.WEAKER, "d", vd.INCONCLUSIVE, status, self.documented),
            )

    def test_an_unknown_verdict_is_a_documented_gap(self):
        cls, _ = ex.track(vd.WEAKER, "d", vd.INCONCLUSIVE, "unknown", self.documented)
        self.assertEqual(vd.TRACKED, cls)

    def test_classify_outcomes_tracks_a_documented_skip(self):
        def outcome(verdict):
            return {
                "verdict": verdict,
                "keys": frozenset(),
                "verify_status": None,
                "capacity_only": False,
                "skipped": frozenset(),
            }

        for path, want in ((self.documented, vd.TRACKED), (self.bare, vd.WEAKER)):
            cls, _ = vd.classify_outcomes(
                "Skipped", path, outcome(vd.PROVEN), outcome(vd.SKIPPED)
            )
            self.assertEqual(want, cls)

    def test_a_tracked_row_is_reported_but_does_not_fail_the_gate(self):
        row = {"class": vd.TRACKED}
        self.assertIn(vd.TRACKED, vd.CLASSES)
        self.assertNotIn(vd.TRACKED, vd.FAILING_CLASSES)
        self.assertEqual(1, vd.build_report("v", [row])["summary"][vd.TRACKED])


if __name__ == "__main__":
    unittest.main()
