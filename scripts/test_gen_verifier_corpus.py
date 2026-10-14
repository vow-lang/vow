#!/usr/bin/env python3
"""Behavior tests for scripts/gen_verifier_corpus.py (#1473).

The generator is the input half of the verifier-versus-runtime differential, so
what it must guarantee is checked here without a verifier: it is reproducible
from its seed, both classes are emitted with the directives the runner reads,
and every program a debug-mode run depends on actually calls what it generates.
Whether a program's runtime matches its class is checked against the real
runtime by `verifier_runtime.check_witness`, not here.
"""

import contextlib
import io
import random
import re
import tempfile
import unittest
import unittest.mock
from pathlib import Path

import gen_corpus
import gen_verifier_corpus as gen

REPO_ROOT = Path(__file__).resolve().parent.parent


def run_cli(*argv):
    gen_corpus.sys.argv = ["gen_corpus.py", *argv]
    with contextlib.redirect_stdout(io.StringIO()):
        return gen_corpus.main()


def corpus(seed, count=200):
    rng = random.Random(seed)
    return [gen.gen_program(rng, i) for i in range(count)]


def class_of(text):
    return re.search(r"^// TEST: gen-class (\S+) (\S+) (\S+)$", text, re.M).groups()


class DeterminismTest(unittest.TestCase):
    def test_same_seed_same_corpus(self):
        self.assertEqual(corpus(1473), corpus(1473))

    def test_different_seed_different_corpus(self):
        self.assertNotEqual(corpus(1473), corpus(1474))

    def test_cli_names_files_by_content_and_is_reproducible(self):
        with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory() as b:
            for out in (a, b):
                self.assertEqual(
                    0,
                    run_cli(
                        "--axis",
                        "verifier",
                        "--seed",
                        "5",
                        "--count",
                        "12",
                        "--output-dir",
                        out,
                    ),
                )
            self.assertEqual(
                sorted(p.name for p in Path(a).iterdir()),
                sorted(p.name for p in Path(b).iterdir()),
            )
            self.assertEqual(12, len(list(Path(a).iterdir())))

    def test_regenerating_keeps_files_it_did_not_generate(self):
        with tempfile.TemporaryDirectory() as out:
            kept = Path(out) / "gen_handwritten.vow"
            kept.write_text("fn main() -> i64 { return 0; }\n")

            run_cli("--axis", "verifier", "--seed", "1", "--output-dir", out)

            self.assertTrue(kept.exists())

    def test_regenerating_replaces_the_previous_corpus(self):
        with tempfile.TemporaryDirectory() as out:
            for seed in ("1", "2"):
                run_cli(
                    "--axis",
                    "verifier",
                    "--seed",
                    seed,
                    "--count",
                    "5",
                    "--output-dir",
                    out,
                )
            self.assertEqual(5, len(list(Path(out).glob("gen_*.vow"))))


class ClassTest(unittest.TestCase):
    def setUp(self):
        self.programs = corpus(1473)

    def test_both_classes_are_generated(self):
        classes = {class_of(p)[0] for p in self.programs}

        self.assertEqual({gen.CLASS_BUG, gen.CLASS_CONTROL}, classes)

    def test_every_shape_appears_in_both_classes(self):
        seen = {(class_of(p)[1], class_of(p)[0]) for p in corpus(1473, 600)}

        for shape in gen.SHAPES:
            for cls in (gen.CLASS_BUG, gen.CLASS_CONTROL):
                with self.subTest(shape=shape, cls=cls):
                    self.assertIn((shape, cls), seen)

    def test_both_regimes_are_generated(self):
        regimes = {class_of(p)[2] for p in corpus(1473, 600)}

        self.assertEqual({gen.WITHIN, gen.BEYOND}, regimes)

    def test_controls_are_marked_expect_verified_and_bugs_are_not(self):
        for text in self.programs:
            marked = "// TEST: expect-verified" in text
            self.assertEqual(class_of(text)[0] == gen.CLASS_CONTROL, marked)

    def test_main_calls_the_generated_function_and_prints(self):
        # A `main` that never calls the contracted function gives the runtime no
        # say, which is the weakness of gen_corpus.py that this axis exists for.
        for text in self.programs:
            fn = re.search(r"^fn (\w+)\(.*\) ->", text, re.M).group(1)
            main = text[text.index("fn main()") :]
            self.assertIn(f"{fn}(", main)
            self.assertRegex(main, r"print_(u|i)64\(")

    def test_every_program_is_contracted(self):
        for text in self.programs:
            self.assertIn("ensures:", text)

    def test_loops_carry_an_invariant(self):
        for text in self.programs:
            self.assertEqual(
                len(re.findall(r"while .* vow \{", text)), text.count("invariant:")
            )

    def test_bug_programs_exceed_a_boundary(self):
        # The point of the class: the program is longer than some boundary a
        # bounded checker is likely to assume.
        for text in corpus(3, 300):
            if class_of(text)[0] != gen.CLASS_BUG:
                continue
            numbers = [int(n) for n in re.findall(r"\b(\d{1,4})\b", text)]
            self.assertTrue(any(n >= min(gen.BOUNDARIES) for n in numbers), text)


class RegimeTest(unittest.TestCase):
    def test_regime_follows_the_longest_loop(self):
        for text in corpus(1473, 300):
            _, _, regime = class_of(text)
            trips = [int(n) for n in re.findall(r"while \w+ < (\d+) vow", text)]
            if trips:
                expected = gen.BEYOND if max(trips) > gen.UNWIND_BOUND else gen.WITHIN
                self.assertEqual(expected, regime, text)

    def test_unwind_bound_matches_the_native_verifier(self):
        source = (REPO_ROOT / "compiler" / "vc_native.vow").read_text()
        body = source[source.index("fn vc_unwind_bounds()") :]
        pushes = re.findall(r"b\.push\((\d+)\);", body[: body.index("\n}\n")])

        self.assertEqual(gen.UNWIND_BOUND, int(pushes[-1]))


class CollectionBoundaryTest(unittest.TestCase):
    def test_esbmc_vec_cap_matches_the_emitter_it_mirrors(self):
        emitter = REPO_ROOT / "vow-verify" / "src" / "c_emitter.rs"
        if not emitter.exists():
            self.skipTest("the C emitter is gone (epic #1398, P6)")
        cap = re.search(r"VEC_MODEL_CAP: usize = (\d+);", emitter.read_text())

        self.assertEqual(gen.ESBMC_VEC_CAP, int(cap.group(1)))

    def test_boundaries_straddle_every_known_capacity(self):
        for edge in (gen.ESBMC_VEC_CAP, gen.UNWIND_BOUND):
            for n in (edge - 1, edge, edge + 1):
                with self.subTest(edge=edge, n=n):
                    self.assertIn(n, gen.BOUNDARIES)

    def test_the_native_verifier_declares_no_collection_capacity(self):
        # AC1 of #1473 asks for behaviour beyond the length the verifier
        # *assumes*. The native model has none (a Vec is an array plus a
        # symbolic length), so the boundaries above are the ESBMC capacities and
        # the unwind bound; if a capacity is ever added to compiler/vc_*.vow this
        # fails, and the generator must straddle it too.
        capacity = re.compile(r"(vec|string|map)[_a-z]*(cap|capacity|max_len)\b", re.I)
        for path in sorted((REPO_ROOT / "compiler").glob("vc_*.vow")):
            for line in path.read_text().splitlines():
                if line.lstrip().startswith("//"):
                    continue
                with self.subTest(path=path.name, line=line):
                    self.assertIsNone(capacity.search(line))


class KnownDisagreementTest(unittest.TestCase):
    def test_the_shipped_ledger_is_valid(self):
        gen.validate_known(gen.KNOWN_DISAGREEMENTS)

    def test_an_entry_needs_an_issue_number(self):
        for bad in ("", "1645", "#", "TBD"):
            with self.subTest(issue=bad):
                with self.assertRaises(ValueError):
                    gen.validate_known(
                        {("vec_len_cap", gen.CLASS_CONTROL, gen.BEYOND): bad}
                    )

    def test_an_entry_must_name_a_real_shape_class_and_regime(self):
        for key in (
            ("nope", gen.CLASS_CONTROL, gen.BEYOND),
            ("vec_len_cap", "other", gen.BEYOND),
            ("vec_len_cap", gen.CLASS_CONTROL, "elsewhere"),
        ):
            with self.subTest(key=key):
                with self.assertRaises(ValueError):
                    gen.validate_known({key: "#1"})

    def test_a_known_control_is_stamped_with_its_issue(self):
        known = {("vec_len_cap", gen.CLASS_CONTROL, gen.BEYOND): "#99"}
        rng = random.Random(0)
        stamped = [gen.gen_program(rng, i, known) for i in range(300)]
        hits = [
            t
            for t in stamped
            if class_of(t) == (gen.CLASS_CONTROL, "vec_len_cap", gen.BEYOND)
        ]

        self.assertTrue(hits)
        for text in hits:
            self.assertIn("// TEST: known-control-gap #99", text)
        for text in stamped:
            if text not in hits:
                self.assertNotIn("known-control-gap", text)

    def test_a_known_bug_entry_becomes_a_soundness_gap_directive(self):
        lines = gen.directives(
            "vec_len_cap",
            gen.CLASS_BUG,
            gen.WITHIN,
            {("vec_len_cap", gen.CLASS_BUG, gen.WITHIN): "#7"},
        )

        self.assertIn("// TEST: known-soundness-gap #7", lines)

    def test_replay_skips_are_issue_backed(self):
        for (shape, cls), why in gen.REPLAY_SKIPS.items():
            self.assertIn(shape, gen.SHAPES)
            self.assertRegex(why, r"#\d+")

    def test_a_replay_skip_trails_the_file(self):
        skips = {("vec_param_clamp", gen.CLASS_BUG): "example limit (#1)"}
        rng = random.Random(2)
        with unittest.mock.patch.dict(gen.REPLAY_SKIPS, skips):
            for i in range(300):
                text = gen.gen_program(rng, i)
                cls, shape = class_of(text)[0], class_of(text)[1]
                if (shape, cls) in skips:
                    self.assertEqual(
                        "// TEST: replay-skipped example limit (#1)",
                        text.splitlines()[-1],
                    )
                    return
        self.fail("no program exercised a replay skip")

    def test_no_shape_skips_its_replay_today(self):
        rng = random.Random(2)
        for i in range(100):
            self.assertNotIn("replay-skipped", gen.gen_program(rng, i))


if __name__ == "__main__":
    unittest.main()
