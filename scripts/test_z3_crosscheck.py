#!/usr/bin/env python3
"""Behavior tests for scripts/z3_crosscheck.py (#1431).

The cross-check re-runs every query the native backend sends to Bitwuzla under
Z3. These tests use fake solvers and a fake `vowc`: a query file holds
"<bitwuzla verdict> <z3 verdict>" so each scenario is one line of text.
"""

import json
import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

import z3_crosscheck


def executable(directory, name, body):
    path = Path(directory) / name
    path.write_text("#!/usr/bin/env bash\n" + body)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


FAKE_BITWUZLA = """\
for arg in "$@"; do last="$arg"; done
case "$last" in
  --version) echo "Bitwuzla 0.9.1"; exit 0 ;;
esac
echo "argv: $*" >&2
read -r verdict _ < "$last"
echo "$verdict"
exit "${FAKE_BITWUZLA_EXIT:-0}"
"""

FAKE_Z3 = """\
for arg in "$@"; do last="$arg"; done
case "$last" in
  --version|-version) echo "Z3 version 5.1.0 - 64 bit"; exit 0 ;;
esac
read -r _ verdict < "$last"
case "$verdict" in
  error) echo '(error "line 1 column 1: unexpected")' ;;
  sleep) sleep 30 ;;
  *) echo "$verdict" ;;
esac
"""

# One query per line of the fixture, handed to whichever `bitwuzla` is first on
# PATH exactly as the product does: `--bv-output-format 2 <file>`.
FAKE_VOWC = """\
fixture="${@: -1}"
n=0
while IFS= read -r line; do
  case "$line" in
    "// query: "*)
      n=$((n + 1))
      q="$(mktemp -d)/query.smt2"
      printf '%s\\n' "${line#// query: }" > "$q"
      bitwuzla --bv-output-format 2 "$q" >/dev/null 2>&1
      rm -rf "$(dirname "$q")"
      ;;
    "// session")
      printf '(check-sat)\\n' | bitwuzla --bv-output-format 2 >/dev/null 2>&1
      ;;
  esac
done < "$fixture"
exit "${FAKE_VOWC_EXIT:-0}"
"""


class WorkTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.work = Path(self._tmp.name)
        self.bin = self.work / "bin"
        self.bin.mkdir()
        self.bitwuzla = executable(self.bin, "bitwuzla", FAKE_BITWUZLA)
        self.z3 = executable(self.bin, "z3", FAKE_Z3)
        self.vowc = executable(self.bin, "vowc", FAKE_VOWC)

    def fixture(self, name, *queries, sessions=0):
        path = self.work / "corpus" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        lines = [f"// query: {q}" for q in queries] + ["// session"] * sessions
        path.write_text("\n".join(lines) + "\n")
        return path


class ParseVerdictTest(unittest.TestCase):
    def test_first_non_empty_line_decides(self):
        self.assertEqual("sat", z3_crosscheck.parse_verdict("\n sat \n((x #b1))\n"))
        self.assertEqual("unsat", z3_crosscheck.parse_verdict("unsat\n"))
        self.assertEqual("unknown", z3_crosscheck.parse_verdict("unknown\n"))

    def test_z3_timeout_line_is_a_timeout(self):
        self.assertEqual("timeout", z3_crosscheck.parse_verdict("timeout\n"))

    def test_anything_else_is_an_error_never_unsat(self):
        for text in ("", '(error "x")\n', "satisfiable\n", "UNSAT\n"):
            with self.subTest(text=text):
                self.assertEqual("error", z3_crosscheck.parse_verdict(text))


class ClassifyTest(unittest.TestCase):
    def test_agreement(self):
        for v in ("sat", "unsat"):
            self.assertEqual("agree", z3_crosscheck.classify(v, v))

    def test_bitwuzla_unsat_z3_sat_is_a_false_proof(self):
        self.assertEqual("SOUNDNESS", z3_crosscheck.classify("unsat", "sat"))

    def test_bitwuzla_sat_z3_unsat_is_a_spurious_counterexample(self):
        self.assertEqual("PRECISION", z3_crosscheck.classify("sat", "unsat"))

    def test_unknown_or_timeout_on_either_side_is_inconclusive(self):
        for b, z in (
            ("unknown", "sat"),
            ("unsat", "unknown"),
            ("timeout", "unsat"),
            ("sat", "timeout"),
            ("unknown", "unknown"),
        ):
            with self.subTest(b=b, z=z):
                self.assertEqual("inconclusive", z3_crosscheck.classify(b, z))

    def test_a_z3_error_is_the_harness_failing(self):
        for b in ("sat", "unsat", "unknown"):
            with self.subTest(b=b):
                self.assertEqual("harness", z3_crosscheck.classify(b, "error"))

    def test_a_bitwuzla_error_is_inconclusive_not_a_verdict(self):
        self.assertEqual("inconclusive", z3_crosscheck.classify("error", "sat"))


class SummarizeTest(unittest.TestCase):
    def counts(self, **kw):
        base = {k: 0 for k in z3_crosscheck.CLASSES}
        base.update(kw)
        return base

    def test_clean_run_is_zero(self):
        self.assertEqual(0, z3_crosscheck.exit_status(self.counts(agree=5), 1))

    def test_a_disagreement_is_one(self):
        self.assertEqual(
            1, z3_crosscheck.exit_status(self.counts(agree=5, SOUNDNESS=1), 1)
        )
        self.assertEqual(
            1, z3_crosscheck.exit_status(self.counts(agree=5, PRECISION=1), 1)
        )

    def test_too_few_agreed_queries_cannot_read_as_clean(self):
        self.assertEqual(2, z3_crosscheck.exit_status(self.counts(agree=2), 3))
        self.assertEqual(2, z3_crosscheck.exit_status(self.counts(), 1))

    def test_harness_failure_outranks_a_disagreement(self):
        self.assertEqual(
            2,
            z3_crosscheck.exit_status(self.counts(agree=5, harness=1, SOUNDNESS=1), 1),
        )


class ShimTest(WorkTest):
    def setUp(self):
        super().setUp()
        self.shim_dir = self.work / "shim"
        self.shim_dir.mkdir()
        self.capture = self.work / "capture"
        self.capture.mkdir()
        z3_crosscheck.install_shim(self.shim_dir)
        self.env = dict(os.environ)
        self.env["VOW_QUERY_CAPTURE_DIR"] = str(self.capture)
        self.env["VOW_REAL_BITWUZLA"] = str(self.bitwuzla)

    def shim(self, *args, stdin=None):
        return subprocess.run(
            [str(self.shim_dir / "bitwuzla"), *args],
            env=self.env,
            input=stdin,
            capture_output=True,
            text=True,
        )

    def test_a_query_file_is_copied_then_the_real_solver_answers_unchanged(self):
        q = self.work / "query.smt2"
        q.write_text("unsat sat\n")

        proc = self.shim("--bv-output-format", "2", str(q))

        self.assertEqual("unsat\n", proc.stdout)
        self.assertEqual(0, proc.returncode)
        self.assertIn("argv: --bv-output-format 2 " + str(q), proc.stderr)
        copies = sorted(self.capture.glob("q.*.smt2"))
        self.assertEqual(1, len(copies))
        self.assertEqual("unsat sat\n", copies[0].read_text())

    def test_the_real_exit_code_is_the_products_exit_code(self):
        q = self.work / "query.smt2"
        q.write_text("sat sat\n")
        self.env["FAKE_BITWUZLA_EXIT"] = "7"

        self.assertEqual(7, self.shim(str(q)).returncode)

    def test_version_passes_through_and_is_not_a_session(self):
        proc = self.shim("--version")

        self.assertEqual("Bitwuzla 0.9.1\n", proc.stdout)
        self.assertFalse((self.capture / "sessions.log").exists())

    def test_a_stdin_session_is_logged_not_captured(self):
        self.shim("--bv-output-format", "2", stdin="(check-sat)\n")

        log = (self.capture / "sessions.log").read_text()
        self.assertIn("--bv-output-format 2", log)
        self.assertEqual([], list(self.capture.glob("q.*.smt2")))

    def test_without_a_real_solver_it_fails_loudly_instead_of_recursing(self):
        del self.env["VOW_REAL_BITWUZLA"]

        proc = self.shim("--version")

        self.assertEqual(127, proc.returncode)
        self.assertIn("VOW_REAL_BITWUZLA", proc.stderr)


class RunSolverTest(WorkTest):
    def test_bitwuzla_is_run_with_the_products_bit_vector_format(self):
        q = self.work / "q.smt2"
        q.write_text("unsat sat\n")

        verdict = z3_crosscheck.run_bitwuzla(str(self.bitwuzla), q, 5)

        self.assertEqual("unsat", verdict)

    def test_z3_verdicts(self):
        for text, expected in (
            ("unsat sat\n", "sat"),
            ("sat unsat\n", "unsat"),
            ("sat unknown\n", "unknown"),
            ("sat error\n", "error"),
        ):
            with self.subTest(text=text):
                q = self.work / "q.smt2"
                q.write_text(text)
                self.assertEqual(expected, z3_crosscheck.run_z3(str(self.z3), q, 5))

    def test_z3_past_the_deadline_is_a_timeout(self):
        q = self.work / "q.smt2"
        q.write_text("sat sleep\n")

        self.assertEqual("timeout", z3_crosscheck.run_z3(str(self.z3), q, 1))

    def test_a_missing_solver_is_an_error_not_a_crash(self):
        q = self.work / "q.smt2"
        q.write_text("sat sat\n")

        self.assertEqual("error", z3_crosscheck.run_z3(str(self.work / "nope"), q, 1))


class MainTest(WorkTest):
    def run_main(self, *roots, extra=()):
        out = self.work / "out"
        argv = [
            *map(str, roots),
            "--vowc",
            str(self.vowc),
            "--bitwuzla",
            str(self.bitwuzla),
            "--z3",
            str(self.z3),
            "--output-dir",
            str(out),
            "--timeout",
            "20",
            *extra,
        ]
        status = z3_crosscheck.main(argv)
        results = out / "results.json"
        return status, json.loads(results.read_text()) if results.exists() else None

    def test_all_queries_agreeing_is_clean(self):
        self.fixture("a.vow", "unsat unsat", "sat sat")

        status, results = self.run_main(self.work / "corpus")

        self.assertEqual(0, status)
        self.assertEqual(2, results["counts"]["agree"])
        self.assertEqual([], results["disagreements"])

    def test_a_disagreement_fails_and_the_query_is_saved_as_a_repro(self):
        self.fixture("a.vow", "unsat unsat", "unsat sat")

        status, results = self.run_main(self.work / "corpus")

        self.assertEqual(1, status)
        self.assertEqual(1, results["counts"]["SOUNDNESS"])
        (found,) = results["disagreements"]
        self.assertEqual("SOUNDNESS", found["class"])
        self.assertTrue(found["fixture"].endswith("a.vow"))
        saved = (self.work / "out" / found["query"]).read_text()
        self.assertEqual("unsat sat\n", saved)

    def test_inconclusive_queries_are_counted_not_failed(self):
        self.fixture("a.vow", "unsat unsat", "unsat unknown")

        status, results = self.run_main(self.work / "corpus")

        self.assertEqual(0, status)
        self.assertEqual(1, results["counts"]["inconclusive"])

    def test_a_z3_error_is_a_harness_failure(self):
        self.fixture("a.vow", "unsat unsat", "sat error")

        status, _ = self.run_main(self.work / "corpus")

        self.assertEqual(2, status)

    def test_no_captured_queries_is_never_clean(self):
        self.fixture("a.vow")

        status, results = self.run_main(self.work / "corpus")

        self.assertEqual(2, status)
        self.assertEqual(0, results["queries"])

    def test_a_session_invocation_is_a_harness_failure(self):
        self.fixture("a.vow", "unsat unsat", sessions=1)

        status, results = self.run_main(self.work / "corpus")

        self.assertEqual(2, status)
        self.assertEqual(1, results["sessions"])

    def test_missing_z3_is_a_harness_failure(self):
        self.fixture("a.vow", "unsat unsat")
        out = self.work / "out"

        status = z3_crosscheck.main(
            [
                str(self.work / "corpus"),
                "--vowc",
                str(self.vowc),
                "--bitwuzla",
                str(self.bitwuzla),
                "--z3",
                str(self.work / "no-z3"),
                "--output-dir",
                str(out),
            ]
        )

        self.assertEqual(2, status)

    def test_skip_directive_fixtures_are_not_run(self):
        path = self.fixture("a.vow", "unsat unsat")
        skipped = self.work / "corpus" / "b.vow"
        skipped.write_text('// TEST: skip "x"\n// query: sat unsat\n')

        status, results = self.run_main(self.work / "corpus")

        self.assertEqual(0, status)
        self.assertEqual(1, results["counts"]["agree"])
        self.assertTrue(path.exists())

    def test_shard_partitions_the_corpus(self):
        self.fixture("a.vow", "unsat unsat")
        self.fixture("b.vow", "sat sat")

        _, first = self.run_main(
            self.work / "corpus", extra=("--shard", "0/2", "--min-agreed", "1")
        )
        _, second = self.run_main(
            self.work / "corpus", extra=("--shard", "1/2", "--min-agreed", "1")
        )

        self.assertEqual(1, first["counts"]["agree"])
        self.assertEqual(1, second["counts"]["agree"])

    def test_extra_root_is_swept_alongside_explicit_roots(self):
        self.fixture("a.vow", "unsat unsat")
        extra = self.work / "generated"
        extra.mkdir()
        (extra / "g.vow").write_text("// query: sat sat\n")

        status, results = self.run_main(
            self.work / "corpus", extra=("--extra-root", str(extra))
        )

        self.assertEqual(0, status)
        self.assertEqual(2, results["counts"]["agree"])

    def test_min_agreed_floor(self):
        self.fixture("a.vow", "unsat unsat")

        status, _ = self.run_main(self.work / "corpus", extra=("--min-agreed", "5"))

        self.assertEqual(2, status)

    def test_a_vowc_that_does_not_run_is_a_harness_failure(self):
        self.fixture("a.vow", "unsat unsat")
        self.vowc.unlink()

        status, _ = self.run_main(self.work / "corpus")

        self.assertEqual(2, status)


if __name__ == "__main__":
    unittest.main()
