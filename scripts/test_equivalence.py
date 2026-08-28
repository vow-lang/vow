#!/usr/bin/env python3
"""Behavior tests for scripts/equivalence.py.

The runner's whole value is that a green run means something, so these tests
concentrate on the ways a differential harness silently goes vacuous: a
divergence that is not reported, a skip that is counted as coverage, or a
nondeterministic program mistaken for a miscompile.
"""

import tempfile
import unittest
from pathlib import Path

import equivalence


def result(
    status=None, executable=None, diagnostics=None, exit_code=0, stderr="", parsed=True
):
    j = None
    if parsed:
        j = {
            "status": status,
            "executable": executable,
            "diagnostics": diagnostics or [],
        }
    return {
        "timeout": False,
        "exit": exit_code,
        "stdout": "",
        "stderr": stderr,
        "json": j,
    }


class CompareBuildTest(unittest.TestCase):
    def test_accept_reject_divergence_is_reported(self):
        rust = result(status="CompileFailed")
        slf = result(status="Unverified", executable="/tmp/a.out")

        div = equivalence.compare_build(rust, slf)

        self.assertEqual(1, len(div))
        self.assertEqual("accept_reject", div[0]["observable"])

    def test_both_accepting_is_not_a_divergence(self):
        rust = result(status="Unverified", executable="/tmp/a")
        slf = result(status="Unverified", executable="/tmp/b")

        self.assertEqual([], equivalence.compare_build(rust, slf))

    def test_differing_error_codes_when_both_reject(self):
        rust = result(
            status="CompileFailed", diagnostics=[{"error_code": "TypeMismatch"}]
        )
        slf = result(
            status="CompileFailed", diagnostics=[{"error_code": "UnexpectedToken"}]
        )

        div = equivalence.compare_build(rust, slf)

        self.assertEqual(1, len(div))
        self.assertEqual("error_code", div[0]["observable"])

    def test_same_error_codes_in_different_order_agree(self):
        rust = result(
            status="CompileFailed",
            diagnostics=[{"error_code": "A"}, {"error_code": "B"}],
        )
        slf = result(
            status="CompileFailed",
            diagnostics=[{"error_code": "B"}, {"error_code": "A"}],
        )

        self.assertEqual([], equivalence.compare_build(rust, slf))

    def test_accept_reject_divergence_suppresses_error_code_noise(self):
        # When one side accepted, the other's diagnostics are not comparable;
        # reporting both would double-count one underlying bug.
        rust = result(
            status="CompileFailed", diagnostics=[{"error_code": "TypeMismatch"}]
        )
        slf = result(status="Unverified", executable="/tmp/a")

        div = equivalence.compare_build(rust, slf)

        self.assertEqual(["accept_reject"], [d["observable"] for d in div])


class FailClosedTest(unittest.TestCase):
    def test_panic_in_stderr_is_a_finding(self):
        res = result(
            status="CompileFailed", stderr="thread 'main' panicked at src/lib.rs:1"
        )

        div = equivalence.check_fail_closed("rust", res)

        self.assertEqual(1, len(div))
        self.assertEqual("fail_closed", div[0]["observable"])

    def test_todo_macro_is_a_finding(self):
        res = result(stderr="not yet implemented")

        self.assertEqual(1, len(equivalence.check_fail_closed("rust", res)))

    def test_signal_death_is_a_finding(self):
        res = result(exit_code=-11)

        div = equivalence.check_fail_closed("self-hosted", res)

        self.assertIn("signal 11", div[0]["detail"])

    def test_clean_rejection_is_not_a_finding(self):
        res = result(
            status="CompileFailed",
            exit_code=1,
            diagnostics=[{"error_code": "TypeMismatch"}],
        )

        self.assertEqual([], equivalence.check_fail_closed("rust", res))


class DirectiveTest(unittest.TestCase):
    def test_skip_directive_is_honoured(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "t.vow"
            p.write_text('// TEST: skip "needs network"\nmodule T\n')

            self.assertEqual("needs network", equivalence.read_directives(p)["skip"])

    def test_inline_stdin_escapes_are_decoded(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "t.vow"
            p.write_text('// TEST: stdin "a\\nb"\nmodule T\n')

            self.assertEqual(
                b"a\nb", equivalence.stdin_bytes(equivalence.read_directives(p))
            )

    def test_absent_stdin_is_empty(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "t.vow"
            p.write_text("module T\n")

            self.assertEqual(
                b"", equivalence.stdin_bytes(equivalence.read_directives(p))
            )


class ExpectedSignalTest(unittest.TestCase):
    def test_declared_trap_exit_becomes_an_expected_signal(self):
        # `// TEST: exit 132` documents a checked-overflow trap (128 + SIGILL);
        # the runner must not report the feature working as a fail_closed bug.
        self.assertEqual(4, equivalence.expected_signal({"expected_exit": 132}))

    def test_declared_abort_exit_becomes_an_expected_signal(self):
        self.assertEqual(6, equivalence.expected_signal({"expected_exit": 134}))

    def test_normal_exit_declares_no_signal(self):
        self.assertIsNone(equivalence.expected_signal({"expected_exit": 0}))

    def test_absent_directive_declares_no_signal(self):
        self.assertIsNone(equivalence.expected_signal({"expected_exit": None}))

    def test_exit_directive_is_parsed(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "t.vow"
            p.write_text("// TEST: exit 132\nmodule T\n")

            self.assertEqual(132, equivalence.read_directives(p)["expected_exit"])


class SignalClassificationTest(unittest.TestCase):
    """Signals both compilers agree on: a trap is the language working."""

    def test_deliberate_traps_are_classified(self):
        # SIGILL from checked-overflow, SIGABRT from a debug vow violation.
        self.assertIn(4, equivalence.TRAP_SIGNALS)
        self.assertIn(6, equivalence.TRAP_SIGNALS)

    def test_memory_unsafety_signals_are_never_traps(self):
        # #905 makes "no input produces a SIGSEGV binary" an invariant that
        # holds regardless of whether the two compilers agree.
        self.assertIn(11, equivalence.UNSAFE_SIGNALS)
        self.assertNotIn(11, equivalence.TRAP_SIGNALS)
        self.assertFalse(
            set(equivalence.TRAP_SIGNALS) & set(equivalence.UNSAFE_SIGNALS)
        )


class CorpusTest(unittest.TestCase):
    def test_corpus_is_sorted_and_deduplicated(self):
        # Shard k of n must mean the same file set on every run, so ordering is
        # part of the contract, not an implementation detail.
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            for name in ("c.vow", "a.vow", "b.vow"):
                (root / name).write_text("module M\n")

            got = equivalence.collect_corpus([root, root], exclude=[])

            self.assertEqual(["a.vow", "b.vow", "c.vow"], [p.name for p in got])

    def test_exclude_filters_by_substring(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "keep.vow").write_text("module M\n")
            (root / "drop_me.vow").write_text("module M\n")

            got = equivalence.collect_corpus([root], exclude=["drop_me"])

            self.assertEqual(["keep.vow"], [p.name for p in got])

    def test_shard_partitions_the_corpus_exactly(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            for i in range(10):
                (root / f"f{i}.vow").write_text("module M\n")
            corpus = equivalence.collect_corpus([root], exclude=[])

            shards = [[f for i, f in enumerate(corpus) if i % 3 == k] for k in range(3)]

            self.assertEqual(
                sorted(corpus, key=str), sorted((f for s in shards for f in s), key=str)
            )


class ReconcileTest(unittest.TestCase):
    """A ledger that suppresses real findings is worse than no ledger."""

    def test_untracked_divergence_is_new(self):
        recs = [{"file": "a.vow", "divergences": [{"observable": "runtime"}]}]

        new, known, fixed = equivalence.reconcile(recs, {})

        self.assertEqual(["a.vow"], [r["file"] for r in new])
        self.assertEqual([], known)
        self.assertEqual([], fixed)

    def test_tracked_divergence_is_known_not_new(self):
        recs = [{"file": "a.vow", "divergences": [{"observable": "error_code"}]}]
        ledger = {"a.vow": {"status": "expected", "issue": 588}}

        new, known, fixed = equivalence.reconcile(recs, ledger)

        self.assertEqual([], new)
        self.assertEqual(["a.vow"], [r["file"] for r in known])

    def test_tracked_divergence_that_stopped_is_reported_fixed(self):
        # Mirrors verify_eval.py's GAP_FIXED: a welcome change must force the
        # ledger to be updated rather than silently drifting out of date.
        recs = [{"file": "a.vow", "divergences": []}]
        ledger = {"a.vow": {"status": "open", "issue": 1087}}

        new, known, fixed = equivalence.reconcile(recs, ledger)

        self.assertEqual(["a.vow"], fixed)

    def test_clean_untracked_file_is_silent(self):
        recs = [{"file": "a.vow", "divergences": []}]

        new, known, fixed = equivalence.reconcile(recs, {})

        self.assertEqual(([], [], []), (new, known, fixed))

    def test_already_fixed_ledger_entry_does_not_re_report(self):
        # status 'fixed' is retained so a REAPPEARANCE reads as a regression;
        # it must not itself be re-reported as newly fixed on every run.
        recs = [{"file": "a.vow", "divergences": []}]
        ledger = {"a.vow": {"status": "fixed", "issue": 1087}}

        new, known, fixed = equivalence.reconcile(recs, ledger)

        self.assertEqual([], fixed)

    def test_reappearance_of_a_fixed_entry_is_known_not_new(self):
        recs = [{"file": "a.vow", "divergences": [{"observable": "runtime"}]}]
        ledger = {"a.vow": {"status": "fixed", "issue": 1087}}

        new, known, fixed = equivalence.reconcile(recs, ledger)

        self.assertEqual(["a.vow"], [r["file"] for r in known])


class LedgerLoadTest(unittest.TestCase):
    def test_missing_ledger_is_empty_not_fatal(self):
        self.assertEqual({}, equivalence.load_ledger("/nonexistent/ledger.json"))

    def test_corrupt_ledger_is_empty_not_fatal(self):
        # Fail open to "everything is new": a broken ledger must never
        # suppress findings.
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "ledger.json"
            p.write_text("{not json")

            self.assertEqual({}, equivalence.load_ledger(p))

    def test_real_repo_ledger_loads(self):
        ledger = equivalence.load_ledger()

        self.assertIn("benchmarks/medium/M13_gcd/reference.vow", ledger)


if __name__ == "__main__":
    unittest.main()
