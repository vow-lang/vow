#!/usr/bin/env python3
"""Behavior tests for scripts/equivalence.py.

The runner's whole value is that a green run means something, so these tests
concentrate on the ways a differential harness silently goes vacuous: a
divergence that is not reported, a skip that is counted as coverage, or a
nondeterministic program mistaken for a miscompile.
"""

import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

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


def ledger_document(corpus=None):
    return {
        "schema_version": 1,
        "updated": "2026-08-31",
        "pairs": {
            "lexer": {
                "rust": "vow-syntax/src/lexer.rs",
                "self_hosted": "compiler/lexer.vow",
                "content_hash": "abc",
                "last_reviewed": "never",
                "outcome": "clean",
                "confirmed_issues": [],
            }
        },
        "corpus": {} if corpus is None else corpus,
    }


LEDGER_SCHEMA = json.loads(
    (equivalence.REPO_ROOT / "docs" / "equivalence" / "ledger.schema.json").read_text()
)
CORPUS_ENTRY_SCHEMA = LEDGER_SCHEMA["properties"]["corpus"]["additionalProperties"]


def assert_valid_ledger_document(test_case, document):
    """Assert the schema rules relied on by stdlib-only workflow tests.

    A deliberate subset of `ledger.schema.json`, not a replacement for it: no
    jsonschema dependency is available here. The enums, property names, and
    required keys are read back out of the schema rather than restated, so the
    parts most likely to drift stay derived; anything asserted literally below
    must be updated alongside the schema.
    """
    test_case.assertEqual(1, document.get("schema_version"))
    test_case.assertIsInstance(document.get("pairs"), dict)
    corpus = document.get("corpus")
    test_case.assertIsInstance(corpus, dict)

    allowed = set(LEDGER_SCHEMA["$defs"]["observableName"]["enum"])
    allowed_keys = set(CORPUS_ENTRY_SCHEMA["properties"])
    statuses = CORPUS_ENTRY_SCHEMA["properties"]["status"]["enum"]
    for path, entry in corpus.items():
        with test_case.subTest(path=path):
            # The schema sets `additionalProperties: false`, so a proposal that
            # invented a field would be rejected on commit rather than here.
            test_case.assertLessEqual(set(entry), allowed_keys)
            for key in CORPUS_ENTRY_SCHEMA["required"]:
                test_case.assertTrue(entry.get(key), f"missing {key}")
            test_case.assertIn(entry.get("status"), statuses)
            declared = equivalence.tracked_observables(entry)
            test_case.assertLessEqual(declared, allowed)
            if entry.get("status") == "expected":
                test_case.assertTrue(entry.get("note"), "missing note")
                test_case.assertIsInstance(entry.get("issue"), int)
            expected_observables = entry.get("expected_observables")
            if expected_observables:
                # `expected_observables ⊆ observable` is something JSON Schema
                # cannot express (2020-12 has no cross-property subset
                # keyword), so this helper is the only place it is checked.
                test_case.assertLessEqual(set(expected_observables), declared)
                test_case.assertIn(entry.get("status"), ("open", "fixed"))
                test_case.assertTrue(entry.get("note"), "missing note")
                test_case.assertIsInstance(entry.get("issue"), int)
            if entry.get("status") in ("open", "expected") and "error_code" in declared:
                rust_codes = entry.get("rust_error_codes")
                self_codes = entry.get("self_hosted_error_codes")
                test_case.assertIsInstance(rust_codes, list)
                test_case.assertIsInstance(self_codes, list)
                test_case.assertEqual(sorted(rust_codes), rust_codes)
                test_case.assertEqual(sorted(self_codes), self_codes)


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


class CompilerExitCodeTest(unittest.TestCase):
    """docs/spec/cli.md pins an exit code per outcome; agreement is not enough."""

    def test_same_verdict_but_differing_process_exit_is_a_divergence(self):
        rust = result(status="CompileFailed", exit_code=1)
        slf = result(status="CompileFailed", exit_code=0)

        div = equivalence.compare_build(rust, slf)

        self.assertEqual(["exit_code"], [d["observable"] for d in div])

    def test_same_executable_parity_but_differing_status_diverges(self):
        # Unverified vs Verified: both exit 0, both carry an executable, but
        # they are distinct CLI outcomes.
        rust = result(status="Unverified", executable="/tmp/a")
        slf = result(status="Verified", executable="/tmp/b")

        div = equivalence.compare_build(rust, slf)

        self.assertEqual(["accept_reject"], [d["observable"] for d in div])
        self.assertIn("status differs", div[0]["detail"])

    def test_matching_process_exit_is_not_a_divergence(self):
        rust = result(status="Unverified", executable="/tmp/a", exit_code=0)
        slf = result(status="Unverified", executable="/tmp/b", exit_code=0)

        self.assertEqual([], equivalence.compare_build(rust, slf))

    def test_accept_reject_divergence_does_not_also_report_exit_code(self):
        # The exit difference is a consequence of the disagreement already
        # reported, not independent information.
        rust = result(status="CompileFailed", exit_code=1)
        slf = result(status="Unverified", executable="/tmp/a", exit_code=0)

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

    def test_no_directives_never_reads_the_candidate_for_directives(self):
        # A candidate a model wrote is not a fixture: it must not be able to
        # excuse itself from the comparison that judges it.
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "candidate.vow"
            p.write_text('// TEST: skip "nothing to see"\nmodule T\n')
            out = Path(d) / "out"

            honoured = equivalence.check_file(p, "rust", "self", out, 1)
            with mock.patch.object(
                equivalence, "read_directives", side_effect=AssertionError("consulted")
            ):
                # Compilation fails on the fake binaries; reaching that at all
                # proves the skip was not taken and directives were not read.
                with self.assertRaises(FileNotFoundError):
                    equivalence.check_file(
                        p, "rust", "self", out, 1, honour_directives=False
                    )

        self.assertEqual("directive: nothing to see", honoured["skipped"])

    def test_verify_only_survives_no_directives(self):
        # Which comparison runs is the harness's choice; suppressing the
        # candidate's directives must not also suppress that choice.
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "candidate.vow"
            p.write_text("module T\n")
            out = Path(d) / "out"
            seen = {}

            def fake_compare_verify(r, s):
                seen["ran"] = True
                return []

            compiled = {
                "timeout": False,
                "exit": 0,
                "stdout": "",
                "stderr": "",
                "json": {"status": "Verified", "diagnostics": []},
            }
            with (
                mock.patch.object(equivalence, "run_compiler", return_value=compiled),
                mock.patch.object(
                    equivalence, "compare_verify", side_effect=fake_compare_verify
                ),
            ):
                equivalence.check_file(
                    p,
                    "rust",
                    "self",
                    out,
                    1,
                    honour_directives=False,
                    verify_only=True,
                )

        self.assertTrue(seen.get("ran"), "the verify comparison never ran")

    def test_the_neutral_record_matches_a_file_declaring_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "t.vow"
            p.write_text("module T\n")

            self.assertEqual(equivalence.read_directives(p), equivalence.NO_DIRECTIVES)


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


def binary_result(stdout=b"", exit_code=0, timeout=False):
    return {"timeout": timeout, "exit": exit_code, "stdout": stdout}


class CompareRuntimeTest(unittest.TestCase):
    """A timeout that DISTINGUISHES the two binaries is a finding, not a skip."""

    def run_with(self, results, **kwargs):
        with mock.patch.object(equivalence, "run_binary", side_effect=results):
            return equivalence.compare_runtime("rust", "self", b"", 30, **kwargs)

    def test_one_sided_self_hosted_timeout_is_a_divergence(self):
        # A codegen regression turning a terminating program into an infinite
        # loop must not be able to leave the sweep green as a mere skip.
        div, why = self.run_with(
            [
                binary_result(b"ok"),
                binary_result(b"ok"),
                binary_result(timeout=True, exit_code=None),
            ]
        )

        self.assertIsNone(why)
        self.assertEqual(["runtime"], [d["observable"] for d in div])
        self.assertIn("timed out", div[0]["detail"])

    def test_one_sided_rust_timeout_is_also_a_divergence(self):
        # Symmetric with the self-hosted case: one implementation hanging where
        # the other terminates distinguishes them, in either direction.
        div, why = self.run_with(
            [
                binary_result(timeout=True, exit_code=None),
                binary_result(timeout=True, exit_code=None),
                binary_result(b"ok"),
            ]
        )

        self.assertIsNone(why)
        self.assertEqual(["runtime"], [d["observable"] for d in div])
        self.assertIn("rust binary timed out", div[0]["detail"])

    def test_both_sides_timing_out_is_inconclusive(self):
        div, why = self.run_with(
            [
                binary_result(timeout=True, exit_code=None),
                binary_result(timeout=True, exit_code=None),
                binary_result(timeout=True, exit_code=None),
            ]
        )

        self.assertEqual([], div)
        self.assertEqual("runtime-timeout", why)

    def test_nondeterministic_rust_is_skipped(self):
        div, why = self.run_with(
            [
                binary_result(b"one"),
                binary_result(b"two"),
            ]
        )

        self.assertEqual([], div)
        self.assertEqual("nondeterministic", why)

    def test_declared_segv_is_still_a_fail_closed_finding(self):
        # A fixture may carry `// TEST: exit 139`, but #905's "no input
        # produces a SIGSEGV binary" invariant is not something a fixture can
        # declare away.
        div, why = self.run_with(
            [
                binary_result(b"", exit_code=-11),
                binary_result(b"", exit_code=-11),
                binary_result(b"", exit_code=-11),
            ],
            expect_signal=11,
        )

        self.assertIsNone(why)
        self.assertEqual(["fail_closed", "fail_closed"], [d["observable"] for d in div])
        self.assertTrue(all("memory unsafety" in d["detail"] for d in div))

    def test_declared_trap_signal_is_still_suppressed(self):
        # SIGILL from a checked-arithmetic overflow is the feature working.
        div, why = self.run_with(
            [
                binary_result(b"", exit_code=-4),
                binary_result(b"", exit_code=-4),
                binary_result(b"", exit_code=-4),
            ],
            expect_signal=4,
        )

        self.assertEqual(([], None), (div, why))

    def test_undeclared_unclassified_signal_is_a_finding(self):
        div, _why = self.run_with(
            [
                binary_result(b"", exit_code=-9),
                binary_result(b"", exit_code=-9),
                binary_result(b"", exit_code=-9),
            ]
        )

        self.assertEqual(["fail_closed"], [d["observable"] for d in div])

    def test_an_exit_difference_is_never_labelled_runtime(self):
        # A `runtime` ledger entry documents a wrong-stdout gap; exit parity is
        # tracked separately so that entry cannot also hide a wrong exit
        # status. 134 is the reserved runtime-abort exit (errors.md), but an
        # ordinary nonzero exit must be separated just as strictly.
        div, _why = self.run_with(
            [
                binary_result(b"ok"),
                binary_result(b"ok"),
                binary_result(b"ok", exit_code=134),
            ]
        )

        # Same stdout, so the exit difference is the only finding — and it must
        # not be labelled `runtime`, or a `runtime` ledger entry would hide it.
        self.assertEqual(["runtime_exit"], [d["observable"] for d in div])

    def test_an_ordinary_exit_difference_is_also_runtime_exit(self):
        # A normal nonzero exit is ordinary Vow behaviour, so singling out
        # aborts would have left this half suppressible.
        div, _why = self.run_with(
            [
                binary_result(b"ok"),
                binary_result(b"ok"),
                binary_result(b"ok", exit_code=1),
            ]
        )

        self.assertEqual(["runtime_exit"], [d["observable"] for d in div])

    def test_a_one_sided_crash_is_reported_once(self):
        # The crash explains the exit difference, so it yields one finding —
        # check_fail_closed's "one bug, one finding" rule.
        div, _why = self.run_with(
            [
                binary_result(b"ok"),
                binary_result(b"ok"),
                binary_result(b"ok", exit_code=-11),
            ]
        )

        self.assertEqual(["fail_closed"], [d["observable"] for d in div])
        self.assertIn("self-hosted binary died on SIGSEGV", div[0]["detail"])

    def test_agreeing_binaries_produce_no_divergence(self):
        div, why = self.run_with(
            [
                binary_result(b"ok"),
                binary_result(b"ok"),
                binary_result(b"ok"),
            ]
        )

        self.assertEqual(([], None), (div, why))


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


class VerifyOnlyTest(unittest.TestCase):
    """`// TEST: verify-only` fixtures must actually reach the verifier."""

    def run_check(self, rust_res, self_res):
        """Drive check_file over a verify-only fixture.

        Args:
            rust_res: run_compiler result for the Rust compiler.
            self_res: run_compiler result for the self-hosted compiler.

        Returns:
            tuple: (record, list of argv lists the stub was called with).
        """
        seen = []
        with tempfile.TemporaryDirectory() as d:
            outdir = Path(d) / "out"
            outdir.mkdir()
            vow = Path(d) / "lib.vow"
            vow.write_text("// TEST: verify-only\nfn f(x: i64) -> i64 { return x; }\n")

            pending = [rust_res, self_res]

            def fake_run_compiler(binary, args, timeout, limit_memory):
                seen.append(args)
                return pending.pop(0)

            with mock.patch.object(
                equivalence, "run_compiler", side_effect=fake_run_compiler
            ):
                rec = equivalence.check_file(vow, "rust", "self", outdir, 5)
        return rec, seen

    def test_verify_only_dispatches_to_the_verifier(self):
        ok = result(status="Verified")

        _rec, seen = self.run_check(ok, ok)

        self.assertTrue(all(a[0] == "verify" for a in seen))
        self.assertTrue(all("-o" not in a for a in seen))

    def test_agreeing_verdicts_count_as_compared_not_skipped(self):
        # Previously these files were recorded as "both rejected (no runtime
        # check)", understating coverage on top of never verifying.
        ok = result(status="Verified")

        rec, _ = self.run_check(ok, ok)

        self.assertEqual([], rec["divergences"])
        self.assertIsNone(rec["skipped"])

    def test_differing_verdicts_are_a_divergence(self):
        rec, _ = self.run_check(result(status="Verified"), result(status="Unverified"))

        self.assertEqual(
            ["verify_status"], [d["observable"] for d in rec["divergences"]]
        )


class MissingStdinFileTest(unittest.TestCase):
    """A fixture whose declared stdin is absent must not read as a pass."""

    def test_missing_stdin_file_raises(self):
        with self.assertRaises(equivalence.MissingStdinFile):
            equivalence.stdin_bytes(
                {"stdin_file": "/nonexistent/input.txt", "stdin": None}
            )

    def test_declared_inline_stdin_still_works(self):
        self.assertEqual(
            b"hi", equivalence.stdin_bytes({"stdin_file": None, "stdin": "hi"})
        )

    def test_present_stdin_file_is_read(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "in.txt"
            f.write_bytes(b"payload")

            self.assertEqual(
                b"payload",
                equivalence.stdin_bytes({"stdin_file": str(f), "stdin": None}),
            )


class CompilerTimeoutTest(unittest.TestCase):
    """One compiler hanging where the other completes is a finding."""

    def run_check(self, rust_res, self_res):
        """Drive check_file with two stubbed compiler results.

        Args:
            rust_res: run_compiler result for the Rust compiler.
            self_res: run_compiler result for the self-hosted compiler.

        Returns:
            dict: The check_file record.
        """
        with tempfile.TemporaryDirectory() as d:
            outdir = Path(d) / "out"
            outdir.mkdir()
            vow = Path(d) / "case.vow"
            vow.write_text("fn main() -> i64 { return 0; }\n")
            pending = [rust_res, self_res]
            with mock.patch.object(
                equivalence, "run_compiler", side_effect=lambda *a, **k: pending.pop(0)
            ):
                return equivalence.check_file(vow, "rust", "self", outdir, 5)

    def timed_out(self):
        return {
            "timeout": True,
            "exit": None,
            "stdout": "",
            "stderr": "",
            "json": None,
        }

    def test_one_sided_compiler_timeout_is_a_divergence(self):
        rec = self.run_check(
            result(status="Unverified", executable="/tmp/a"), self.timed_out()
        )

        self.assertEqual(["fail_closed"], [d["observable"] for d in rec["divergences"]])
        self.assertIn("self-hosted compiler timed out", rec["divergences"][0]["detail"])

    def test_agreeing_rejections_count_as_a_completed_comparison(self):
        # Every applicable build observable was compared; the absence of a
        # runtime phase does not make this an unexamined file.
        rejected = result(
            status="CompileFailed",
            exit_code=1,
            diagnostics=[{"error_code": "TypeMismatch"}],
        )

        rec = self.run_check(rejected, rejected)

        self.assertEqual([], rec["divergences"])
        self.assertIsNone(rec["skipped"])

    def test_both_compilers_timing_out_is_a_skip(self):
        rec = self.run_check(self.timed_out(), self.timed_out())

        self.assertEqual([], rec["divergences"])
        self.assertEqual("compile timeout (both)", rec["skipped"])


class CompareVerifyTest(unittest.TestCase):
    def test_matching_verdicts_are_not_a_divergence(self):
        ok = result(status="Verified")

        self.assertEqual([], equivalence.compare_verify(ok, ok))

    def test_shared_verify_failed_with_different_backends_diverges(self):
        # cli.md: VerifyFailed covers both a real counterexample and a soft
        # backend failure, and both commonly carry exit 1 and no diagnostics.
        rust = result(status="VerifyFailed", exit_code=1)
        rust["json"]["counterexamples"] = [{"function": "f"}]
        slf = result(status="VerifyFailed", exit_code=1)
        slf["json"]["verify_status"] = "timeout"
        slf["json"]["counterexamples"] = []

        div = equivalence.compare_verify(rust, slf)

        self.assertEqual(["verify_status"], [d["observable"] for d in div])

    def test_same_counterexample_count_different_identity_diverges(self):
        rust = result(status="VerifyFailed", exit_code=1)
        rust["json"]["counterexamples"] = [
            {"function": "f", "vow_id": 1, "blame": "Callee"}
        ]
        slf = result(status="VerifyFailed", exit_code=1)
        slf["json"]["counterexamples"] = [
            {"function": "g", "vow_id": 2, "blame": "Caller"}
        ]

        div = equivalence.compare_verify(rust, slf)

        self.assertEqual(["verify_status"], [d["observable"] for d in div])

    def test_identical_counterexamples_agree(self):
        cex = [{"function": "f", "vow_id": 1, "blame": "Callee"}]
        rust = result(status="VerifyFailed", exit_code=1)
        rust["json"]["counterexamples"] = list(cex)
        slf = result(status="VerifyFailed", exit_code=1)
        slf["json"]["counterexamples"] = list(cex)

        self.assertEqual([], equivalence.compare_verify(rust, slf))

    def test_differing_diagnostics_are_reported(self):
        rust = result(status="Unverified", diagnostics=[{"error_code": "A"}])
        slf = result(status="Unverified", diagnostics=[{"error_code": "B"}])

        div = equivalence.compare_verify(rust, slf)

        self.assertEqual(["error_code"], [d["observable"] for d in div])


class CheckFileCleanupTest(unittest.TestCase):
    """Partial binaries must not survive the early-return paths."""

    def run_check(self, compiler_results):
        """Drive check_file with a stubbed compiler that writes its -o target.

        Args:
            compiler_results: One run_compiler return value per invocation.

        Returns:
            list: Leftover file names in the output directory.
        """
        with tempfile.TemporaryDirectory() as d:
            outdir = Path(d) / "out"
            outdir.mkdir()
            vow = Path(d) / "case.vow"
            vow.write_text("fn main() -> i64 { return 0; }\n")

            pending = list(compiler_results)

            def fake_run_compiler(binary, args, timeout, limit_memory):
                target = Path(args[args.index("-o") + 1])
                target.write_bytes(b"partial")
                target.with_suffix(".o").write_bytes(b"partial")
                return pending.pop(0)

            with mock.patch.object(
                equivalence, "run_compiler", side_effect=fake_run_compiler
            ):
                equivalence.check_file(vow, "rust", "self", outdir, 5)

            return sorted(f.name for f in outdir.iterdir())

    def test_compile_timeout_leaves_nothing_behind(self):
        timed_out = {
            "timeout": True,
            "exit": None,
            "stdout": "",
            "stderr": "",
            "json": None,
        }

        self.assertEqual([], self.run_check([timed_out, timed_out]))

    def test_unparseable_json_leaves_nothing_behind(self):
        garbage = result(parsed=False, exit_code=1)

        self.assertEqual([], self.run_check([garbage, garbage]))


class ReconcileTest(unittest.TestCase):
    """A ledger that suppresses real findings is worse than no ledger."""

    def test_untracked_divergence_is_new(self):
        recs = [{"file": "a.vow", "divergences": [{"observable": "runtime"}]}]

        new, known, fixed = equivalence.reconcile(recs, {})

        self.assertEqual(["a.vow"], [r["file"] for r in new])
        self.assertEqual([], known)
        self.assertEqual([], fixed)

    def test_tracked_divergence_is_known_not_new(self):
        divergence = {
            "observable": "error_code",
            "rust": ["A"],
            "self_hosted": ["B"],
        }
        recs = [{"file": "a.vow", "divergences": [divergence]}]
        ledger = {
            "a.vow": {
                "status": "expected",
                "observable": "error_code",
                "issue": 588,
                "rust_error_codes": ["A"],
                "self_hosted_error_codes": ["B"],
            }
        }

        new, known, _fixed = equivalence.reconcile(recs, ledger)

        self.assertEqual([], new)
        self.assertEqual(["a.vow"], [r["file"] for r in known])

    def test_changed_error_code_payload_is_new_and_the_old_gap_is_fixed(self):
        recs = [
            {
                "file": "a.vow",
                "divergences": [
                    {
                        "observable": "error_code",
                        "rust": ["LinearTypeViolation"],
                        "self_hosted": ["TypeMismatch"],
                    }
                ],
            }
        ]
        ledger = {
            "a.vow": {
                "status": "expected",
                "observable": "error_code",
                "issue": 588,
                "rust_error_codes": ["LinearTypeViolation"],
                "self_hosted_error_codes": ["RegionLinear"],
            }
        }

        new, known, fixed = equivalence.reconcile(recs, ledger)

        self.assertEqual(["a.vow"], [record["file"] for record in new])
        self.assertEqual([], known)
        self.assertEqual([{"file": "a.vow", "observables": ["error_code"]}], fixed)

    def test_tracked_divergence_that_stopped_is_reported_fixed(self):
        # Mirrors verify_eval.py's GAP_FIXED: a welcome change must force the
        # ledger to be updated rather than silently drifting out of date.
        recs = [{"file": "a.vow", "divergences": []}]
        ledger = {"a.vow": {"status": "open", "observable": "runtime", "issue": 1087}}

        _new, _known, fixed = equivalence.reconcile(recs, ledger)

        self.assertEqual([{"file": "a.vow", "observables": ["runtime"]}], fixed)

    def test_clean_untracked_file_is_silent(self):
        recs = [{"file": "a.vow", "divergences": []}]

        new, known, fixed = equivalence.reconcile(recs, {})

        self.assertEqual(([], [], []), (new, known, fixed))

    def test_a_skipped_file_is_never_reported_fixed(self):
        # A compile timeout on a loaded CI runner carries no divergences, but
        # it is not evidence the tracked gap closed. Reporting it as fixed
        # would fail the run and demand a ledger edit over infra flakiness.
        recs = [
            {
                "file": "a.vow",
                "divergences": [],
                "skipped": "compile timeout (self-hosted)",
            }
        ]
        ledger = {"a.vow": {"status": "open", "observable": "runtime", "issue": 1087}}

        new, known, fixed = equivalence.reconcile(recs, ledger)

        self.assertEqual(([], [], []), (new, known, fixed))

    def test_a_directive_skipped_file_is_never_reported_fixed(self):
        recs = [
            {"file": "a.vow", "divergences": [], "skipped": "directive: needs stdin"}
        ]
        ledger = {
            "a.vow": {
                "status": "expected",
                "observable": "error_code",
                "note": "n",
                "issue": 588,
            }
        }

        _new, _known, fixed = equivalence.reconcile(recs, ledger)

        self.assertEqual([], fixed)

    def test_already_fixed_ledger_entry_does_not_re_report(self):
        # status 'fixed' is retained so a REAPPEARANCE reads as a regression;
        # it must not itself be re-reported as newly fixed on every run.
        recs = [{"file": "a.vow", "divergences": []}]
        ledger = {"a.vow": {"status": "fixed", "issue": 1087}}

        _new, _known, fixed = equivalence.reconcile(recs, ledger)

        self.assertEqual([], fixed)

    def test_reappearance_of_a_fixed_entry_is_a_regression(self):
        # The schema retains `fixed` entries precisely so a reappearance reads
        # as a regression; folding it into `known` would let the run pass.
        recs = [{"file": "a.vow", "divergences": [{"observable": "runtime"}]}]
        ledger = {"a.vow": {"status": "fixed", "observable": "runtime", "issue": 1087}}

        new, known, fixed = equivalence.reconcile(recs, ledger)

        self.assertEqual(["a.vow"], [r["file"] for r in new])
        self.assertEqual([], known)
        self.assertEqual([], fixed)

    def test_untracked_observable_on_a_tracked_file_is_new(self):
        # The suppression the ledger exists to prevent, one level down: a file
        # tracked for an error_code gap that ALSO starts dying on a signal has
        # produced a genuinely new finding.
        recs = [
            {
                "file": "a.vow",
                "divergences": [
                    {
                        "observable": "error_code",
                        "rust": ["A"],
                        "self_hosted": ["B"],
                    },
                    {"observable": "fail_closed"},
                ],
            }
        ]
        ledger = {
            "a.vow": {
                "status": "expected",
                "observable": "error_code",
                "issue": 588,
                "rust_error_codes": ["A"],
                "self_hosted_error_codes": ["B"],
            }
        }

        new, known, fixed = equivalence.reconcile(recs, ledger)

        self.assertEqual(["a.vow"], [r["file"] for r in new])
        self.assertEqual([{"observable": "fail_closed"}], new[0]["divergences"])
        self.assertEqual(
            [
                {
                    "observable": "error_code",
                    "rust": ["A"],
                    "self_hosted": ["B"],
                }
            ],
            known[0]["divergences"],
        )
        self.assertEqual([], fixed)

    def test_tracked_observable_gone_is_fixed_even_when_another_appears(self):
        # The tracked gap stopped reproducing; that must still force a ledger
        # update even though the file is not clean.
        recs = [{"file": "a.vow", "divergences": [{"observable": "runtime"}]}]
        ledger = {"a.vow": {"status": "open", "observable": "error_code", "issue": 588}}

        new, known, fixed = equivalence.reconcile(recs, ledger)

        self.assertEqual(["a.vow"], [r["file"] for r in new])
        self.assertEqual([], known)
        self.assertEqual([{"file": "a.vow", "observables": ["error_code"]}], fixed)

    def test_one_stale_half_of_a_multi_observable_entry_is_reported(self):
        # Only `runtime` still reproduces; the tracked `error_code` half is
        # stale and would otherwise keep suppressing its next recurrence.
        recs = [{"file": "a.vow", "divergences": [{"observable": "runtime"}]}]
        ledger = {"a.vow": {"status": "open", "observable": ["error_code", "runtime"]}}

        new, known, fixed = equivalence.reconcile(recs, ledger)

        self.assertEqual([], new)
        self.assertEqual(["a.vow"], [r["file"] for r in known])
        self.assertEqual([{"file": "a.vow", "observables": ["error_code"]}], fixed)

    def test_entry_may_track_several_observables(self):
        recs = [
            {
                "file": "a.vow",
                "divergences": [
                    {
                        "observable": "error_code",
                        "rust": ["A"],
                        "self_hosted": ["B"],
                    },
                    {"observable": "runtime"},
                ],
            }
        ]
        ledger = {
            "a.vow": {
                "status": "open",
                "observable": ["error_code", "runtime"],
                "rust_error_codes": ["A"],
                "self_hosted_error_codes": ["B"],
            }
        }

        new, known, fixed = equivalence.reconcile(recs, ledger)

        self.assertEqual([], new)
        self.assertEqual(["a.vow"], [r["file"] for r in known])
        self.assertEqual([], fixed)


class ProposeLedgerTest(unittest.TestCase):
    def test_new_divergence_adds_an_open_entry(self):
        document = ledger_document()
        new = [
            {
                "file": "z.vow",
                "divergences": [
                    {"observable": "runtime"},
                    {"observable": "runtime_exit"},
                ],
            }
        ]

        proposed = equivalence.propose_ledger(document, new, [], "2026-09-01")

        self.assertEqual(
            {
                "first_seen": "2026-09-01",
                "observable": ["runtime", "runtime_exit"],
                "status": "open",
            },
            proposed["corpus"]["z.vow"],
        )
        assert_valid_ledger_document(self, proposed)

    def test_new_error_code_divergence_pins_sorted_multisets(self):
        new = [
            {
                "file": "a.vow",
                "divergences": [
                    {
                        "observable": "error_code",
                        "rust": ["Z", "A"],
                        "self_hosted": ["Y", "B"],
                    }
                ],
            }
        ]

        proposed = equivalence.propose_ledger(ledger_document(), new, [], "2026-09-01")
        entry = proposed["corpus"]["a.vow"]

        self.assertEqual(["A", "Z"], entry["rust_error_codes"])
        self.assertEqual(["B", "Y"], entry["self_hosted_error_codes"])
        assert_valid_ledger_document(self, proposed)

    def test_fully_fixed_entry_keeps_its_history(self):
        original = {
            "first_seen": "2026-08-25",
            "observable": "runtime",
            "status": "open",
            "note": "wrong output",
            "issue": 1087,
            "fixture": "tests/run/reproducer.vow",
        }
        fixed = [{"file": "a.vow", "observables": ["runtime"]}]

        proposed = equivalence.propose_ledger(
            ledger_document({"a.vow": original}), [], fixed, "2026-09-01"
        )

        self.assertEqual({**original, "status": "fixed"}, proposed["corpus"]["a.vow"])
        assert_valid_ledger_document(self, proposed)

    def test_fully_fixing_a_mixed_entry_keeps_expected_observables_as_history(self):
        # A full fix retains every field verbatim (test_fully_fixed_entry_
        # keeps_its_history above) so a reappearance reads as a regression.
        # expected_observables is no exception, even though its own `status:
        # open` requirement can no longer hold once the whole entry is fixed.
        original = {
            "first_seen": "2026-08-25",
            "observable": ["error_code", "runtime"],
            "status": "open",
            "expected_observables": ["error_code"],
            "note": "intentional diagnostic wording difference",
            "issue": 588,
            "rust_error_codes": ["A"],
            "self_hosted_error_codes": ["B"],
        }
        fixed = [{"file": "a.vow", "observables": ["error_code", "runtime"]}]

        proposed = equivalence.propose_ledger(
            ledger_document({"a.vow": original}), [], fixed, "2026-09-01"
        )

        self.assertEqual({**original, "status": "fixed"}, proposed["corpus"]["a.vow"])
        assert_valid_ledger_document(self, proposed)

    def test_partially_fixed_entry_keeps_only_the_live_observable(self):
        original = {
            "first_seen": "2026-08-25",
            "observable": ["error_code", "runtime"],
            "status": "open",
            "rust_error_codes": ["A"],
            "self_hosted_error_codes": ["B"],
        }
        fixed = [{"file": "a.vow", "observables": ["error_code"]}]

        proposed = equivalence.propose_ledger(
            ledger_document({"a.vow": original}), [], fixed, "2026-09-01"
        )

        self.assertEqual(
            {
                "first_seen": "2026-08-25",
                "observable": "runtime",
                "status": "open",
            },
            proposed["corpus"]["a.vow"],
        )
        assert_valid_ledger_document(self, proposed)

    def test_partial_fix_prunes_the_fixed_observable_from_expected_observables(self):
        # The `expected` half stopped reproducing; the still-open `runtime`
        # half never was expected, so it must not carry a stale reference to
        # an observable that no longer appears in `observable`.
        original = {
            "first_seen": "2026-08-25",
            "observable": ["error_code", "runtime"],
            "status": "open",
            "expected_observables": ["error_code"],
            "note": "intentional diagnostic wording difference",
            "issue": 588,
        }
        fixed = [{"file": "a.vow", "observables": ["error_code"]}]

        proposed = equivalence.propose_ledger(
            ledger_document({"a.vow": original}), [], fixed, "2026-09-01"
        )

        entry = proposed["corpus"]["a.vow"]
        self.assertEqual("runtime", entry["observable"])
        self.assertNotIn("expected_observables", entry)
        assert_valid_ledger_document(self, proposed)

    def test_new_observable_extends_an_existing_entry(self):
        original = {
            "first_seen": "2026-08-25",
            "observable": "runtime",
            "status": "open",
            "note": "wrong output",
            "issue": 1087,
        }
        new = [
            {
                "file": "a.vow",
                "divergences": [{"observable": "runtime_exit"}],
            }
        ]

        proposed = equivalence.propose_ledger(
            ledger_document({"a.vow": original}), new, [], "2026-09-01"
        )

        self.assertEqual(
            {
                **original,
                "observable": ["runtime", "runtime_exit"],
            },
            proposed["corpus"]["a.vow"],
        )
        assert_valid_ledger_document(self, proposed)

    def test_extending_an_expected_entry_preserves_its_classification(self):
        # `expected` means a human already reviewed and blessed this specific
        # asymmetry. A new, unrelated observable must still force `open` (the
        # new finding is unreviewed), but the prior review must not vanish.
        original = {
            "first_seen": "2026-08-25",
            "observable": "error_code",
            "status": "expected",
            "note": "intentional diagnostic wording difference",
            "issue": 588,
            "rust_error_codes": ["A"],
            "self_hosted_error_codes": ["B"],
        }
        new = [
            {
                "file": "a.vow",
                "divergences": [{"observable": "runtime"}],
            }
        ]

        proposed = equivalence.propose_ledger(
            ledger_document({"a.vow": original}), new, [], "2026-09-01"
        )

        entry = proposed["corpus"]["a.vow"]
        self.assertEqual(["error_code", "runtime"], entry["observable"])
        self.assertEqual("open", entry["status"])
        self.assertEqual(["error_code"], entry["expected_observables"])
        self.assertEqual("intentional diagnostic wording difference", entry["note"])
        self.assertEqual(588, entry["issue"])
        self.assertEqual(["A"], entry["rust_error_codes"])
        self.assertEqual(["B"], entry["self_hosted_error_codes"])
        assert_valid_ledger_document(self, proposed)

    def test_further_extending_a_mixed_entry_keeps_its_prior_expected_observables(self):
        original = {
            "first_seen": "2026-08-25",
            "observable": ["error_code", "runtime"],
            "status": "open",
            "expected_observables": ["error_code"],
            "note": "intentional diagnostic wording difference",
            "issue": 588,
            "rust_error_codes": ["A"],
            "self_hosted_error_codes": ["B"],
        }
        new = [
            {
                "file": "a.vow",
                "divergences": [{"observable": "exit_code"}],
            }
        ]

        proposed = equivalence.propose_ledger(
            ledger_document({"a.vow": original}), new, [], "2026-09-01"
        )

        entry = proposed["corpus"]["a.vow"]
        self.assertEqual(["error_code", "exit_code", "runtime"], entry["observable"])
        self.assertEqual(["error_code"], entry["expected_observables"])
        assert_valid_ledger_document(self, proposed)

    def test_reopening_a_fixed_entry_drops_its_stale_expected_observables(self):
        original = {
            "first_seen": "2026-08-25",
            "observable": ["error_code", "runtime"],
            "status": "fixed",
            "expected_observables": ["error_code"],
            "note": "intentional diagnostic wording difference",
            "issue": 588,
        }
        new = [
            {
                "file": "a.vow",
                "divergences": [{"observable": "exit_code"}],
            }
        ]

        proposed = equivalence.propose_ledger(
            ledger_document({"a.vow": original}), new, [], "2026-09-01"
        )

        entry = proposed["corpus"]["a.vow"]
        self.assertEqual("exit_code", entry["observable"])
        self.assertNotIn("expected_observables", entry)
        assert_valid_ledger_document(self, proposed)

    def test_untouched_data_round_trips_and_corpus_keys_are_sorted(self):
        untouched = {
            "first_seen": "2026-08-25",
            "observable": "runtime",
            "status": "fixed",
            "note": "history",
            "issue": 1,
        }
        document = ledger_document({"z.vow": untouched})
        pairs = json.loads(json.dumps(document["pairs"]))
        new = [
            {
                "file": "a.vow",
                "divergences": [{"observable": "runtime_exit"}],
            }
        ]

        proposed = equivalence.propose_ledger(document, new, [], "2026-09-01")

        self.assertEqual(pairs, proposed["pairs"])
        self.assertEqual(untouched, proposed["corpus"]["z.vow"])
        self.assertEqual(["a.vow", "z.vow"], list(proposed["corpus"]))
        assert_valid_ledger_document(self, proposed)

    def test_clean_run_changes_only_the_update_date(self):
        document = ledger_document(
            {
                "a.vow": {
                    "first_seen": "2026-08-25",
                    "observable": "runtime",
                    "status": "fixed",
                }
            }
        )
        expected = json.loads(json.dumps(document))
        expected["updated"] = "2026-09-01"

        proposed = equivalence.propose_ledger(document, [], [], "2026-09-01")

        self.assertEqual(expected, proposed)
        assert_valid_ledger_document(self, proposed)

    def test_a_fixed_observable_is_not_reopened_by_an_unrelated_new_one(self):
        # Applying a proposal that re-listed `runtime` would make the next run
        # suppress a genuine runtime regression as `known` — precisely what
        # reconcile's per-observable bookkeeping exists to prevent.
        document = ledger_document(
            {
                "a.vow": {
                    "first_seen": "2026-08-25",
                    "observable": "runtime",
                    "status": "open",
                }
            }
        )

        proposed = equivalence.propose_ledger(
            document,
            [{"file": "a.vow", "divergences": [{"observable": "exit_code"}]}],
            [{"file": "a.vow", "observables": ["runtime"]}],
            "2026-09-01",
        )

        entry = proposed["corpus"]["a.vow"]
        self.assertEqual("exit_code", entry["observable"])
        self.assertEqual("open", entry["status"])
        assert_valid_ledger_document(self, proposed)

    def test_an_already_fixed_observable_is_not_reopened_by_a_new_one(self):
        # An entry fixed by an EARLIER sweep never appears in `fixed`, so the
        # disappearance pass cannot drop its observable. Carrying it into this
        # reopen would claim a runtime gap nothing measured this run.
        document = ledger_document(
            {
                "a.vow": {
                    "first_seen": "2026-08-25",
                    "observable": "runtime",
                    "status": "fixed",
                }
            }
        )

        proposed = equivalence.propose_ledger(
            document,
            [{"file": "a.vow", "divergences": [{"observable": "exit_code"}]}],
            [],
            "2026-09-01",
        )

        entry = proposed["corpus"]["a.vow"]
        self.assertEqual("exit_code", entry["observable"])
        self.assertEqual("open", entry["status"])
        assert_valid_ledger_document(self, proposed)

    def test_reopening_a_fixed_entry_drops_its_stale_error_code_multisets(self):
        # The pinned multisets exist to stop a DIFFERENT diagnostic regression
        # inheriting the exception. Retained past a reopen on another
        # observable they would do exactly that.
        document = ledger_document(
            {
                "a.vow": {
                    "first_seen": "2026-08-25",
                    "observable": "error_code",
                    "status": "fixed",
                    "rust_error_codes": ["TypeMismatch"],
                    "self_hosted_error_codes": ["UnknownName"],
                }
            }
        )

        proposed = equivalence.propose_ledger(
            document,
            [{"file": "a.vow", "divergences": [{"observable": "runtime"}]}],
            [],
            "2026-09-01",
        )

        entry = proposed["corpus"]["a.vow"]
        self.assertEqual("runtime", entry["observable"])
        self.assertNotIn("rust_error_codes", entry)
        self.assertNotIn("self_hosted_error_codes", entry)
        assert_valid_ledger_document(self, proposed)

    def test_an_open_error_code_entry_keeps_its_multisets_when_a_gap_is_added(self):
        # The error_code divergence is still live and still suppressing, so the
        # schema still requires both pinned multisets on the reopened entry.
        document = ledger_document(
            {
                "a.vow": {
                    "first_seen": "2026-08-25",
                    "observable": "error_code",
                    "status": "open",
                    "rust_error_codes": ["TypeMismatch"],
                    "self_hosted_error_codes": ["UnknownName"],
                }
            }
        )

        proposed = equivalence.propose_ledger(
            document,
            [{"file": "a.vow", "divergences": [{"observable": "runtime"}]}],
            [],
            "2026-09-01",
        )

        entry = proposed["corpus"]["a.vow"]
        self.assertEqual(["error_code", "runtime"], entry["observable"])
        self.assertEqual(["TypeMismatch"], entry["rust_error_codes"])
        self.assertEqual(["UnknownName"], entry["self_hosted_error_codes"])
        assert_valid_ledger_document(self, proposed)

    def test_error_code_payload_change_drops_the_stale_expected_observables(self):
        # reconcile() reports a changed error_code payload as BOTH fixed (the
        # old codes) and new (the replacement codes) in the same run — see
        # propose_ledger's module comment. The previously-reviewed
        # classification applied to the old codes specifically, so it must
        # not survive onto the unreviewed replacement.
        original = {
            "first_seen": "2026-08-25",
            "observable": ["error_code", "runtime"],
            "status": "open",
            "expected_observables": ["error_code"],
            "note": "intentional diagnostic wording difference",
            "issue": 588,
            "rust_error_codes": ["A"],
            "self_hosted_error_codes": ["B"],
        }
        fixed = [{"file": "a.vow", "observables": ["error_code"]}]
        new = [
            {
                "file": "a.vow",
                "divergences": [
                    {"observable": "error_code", "rust": ["Z"], "self_hosted": ["Y"]}
                ],
            }
        ]

        proposed = equivalence.propose_ledger(
            ledger_document({"a.vow": original}), new, fixed, "2026-09-01"
        )

        entry = proposed["corpus"]["a.vow"]
        self.assertEqual(["error_code", "runtime"], entry["observable"])
        self.assertNotIn("expected_observables", entry)
        self.assertEqual(["Z"], entry["rust_error_codes"])
        self.assertEqual(["Y"], entry["self_hosted_error_codes"])
        assert_valid_ledger_document(self, proposed)


class EmitLedgerUpdateCliTest(unittest.TestCase):
    def run_sweep(self, root, min_compared="0", extra=(), emit=True):
        """Run main() over an empty corpus, returning (exit_code, output dir)."""
        rust = root / "rust"
        self_hosted = root / "self"
        rust.write_bytes(b"rust")
        self_hosted.write_bytes(b"self")
        ledger = root / "ledger.json"
        ledger.write_text(json.dumps(ledger_document()))
        output = root / "out"
        argv = [
            "equivalence.py",
            "--rust",
            str(rust),
            "--self",
            str(self_hosted),
            "--ledger",
            str(ledger),
            "--output-dir",
            str(output),
            "--min-compared",
            min_compared,
            *(["--emit-ledger-update"] if emit else []),
            *extra,
        ]

        with (
            mock.patch("sys.argv", argv),
            mock.patch.object(equivalence, "collect_corpus", return_value=[]),
        ):
            return equivalence.main(), output

    def test_flag_writes_a_schema_valid_proposal_in_the_output_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            exit_code, output = self.run_sweep(Path(directory))

            proposal = json.loads((output / "ledger.proposed.json").read_text())
            self.assertEqual(0, exit_code)
            assert_valid_ledger_document(self, proposal)

    def test_a_run_below_the_coverage_floor_proposes_nothing(self):
        # A shard that measured too little to be meaningful must not ship a
        # proposal that reads as applicable; its `updated` stamp would claim a
        # sweep that never happened.
        with tempfile.TemporaryDirectory() as directory:
            exit_code, output = self.run_sweep(Path(directory), min_compared="20")

            self.assertEqual(2, exit_code)
            self.assertFalse((output / "ledger.proposed.json").exists())

    def test_a_below_floor_run_clears_an_earlier_proposal(self):
        # A reused --output-dir must not keep a proposal from a different
        # sweep: the summary says "none" while the directory an operator
        # applies, or the workflow uploads, still holds the stale file.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            _exit, output = self.run_sweep(root)
            self.assertTrue((output / "ledger.proposed.json").exists())

            exit_code, output = self.run_sweep(root, min_compared="20")

            self.assertEqual(2, exit_code)
            self.assertFalse((output / "ledger.proposed.json").exists())

    def test_a_run_without_the_flag_clears_an_earlier_proposal(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            _exit, output = self.run_sweep(root)
            stale = output / "ledger.proposed.json"
            self.assertTrue(stale.exists())

            self.run_sweep(root, emit=False)

            self.assertFalse(stale.exists())

    def test_a_below_floor_run_does_not_advertise_a_proposal_path(self):
        # The "NO LONGER DIVERGING" block prints where the proposal landed.
        # Pointing an operator at a path nothing wrote — and that this run just
        # deleted — contradicts the "none" line printed just above it.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with mock.patch.object(
                equivalence,
                "reconcile",
                return_value=([], [], [{"file": "a.vow", "observables": ["runtime"]}]),
            ):
                with mock.patch("sys.stdout", new_callable=io.StringIO) as out:
                    self.run_sweep(root, min_compared="20")

        printed = out.getvalue()
        self.assertIn("NO LONGER DIVERGING", printed)
        self.assertNotIn("proposed update:", printed)

    def test_a_crashing_sweep_leaves_no_stale_completion_sentinel(self):
        # equivalence.yml reads results.json's presence as proof the sweep
        # completed, so a previous run's copy turns a crash into a divergence
        # verdict. Clearing it up front is the only placement that survives a
        # crash inside the sweep itself.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            _exit, output = self.run_sweep(root)
            self.assertTrue((output / "results.json").exists())

            with mock.patch.object(
                equivalence, "reconcile", side_effect=RuntimeError("boom")
            ):
                with self.assertRaises(RuntimeError):
                    self.run_sweep(root)

            self.assertFalse((output / "results.json").exists())

    def test_results_json_is_written_last(self):
        # equivalence.yml treats results.json's presence as proof the sweep
        # completed, so a crash while proposing must not leave it behind.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with mock.patch.object(
                equivalence, "propose_ledger", side_effect=RuntimeError("boom")
            ):
                with self.assertRaises(RuntimeError):
                    self.run_sweep(root)

            self.assertFalse((root / "out" / "results.json").exists())

    def test_the_update_stamp_is_caller_supplied(self):
        # ledger.schema.json requires `updated` to be stamped by the caller so
        # a re-run of the same sweep reproduces the same proposal.
        with tempfile.TemporaryDirectory() as directory:
            _exit, output = self.run_sweep(
                Path(directory), extra=("--today", "2026-01-02")
            )

            proposal = json.loads((output / "ledger.proposed.json").read_text())
            self.assertEqual("2026-01-02", proposal["updated"])


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

    def test_real_repo_ledger_satisfies_schema_constraints(self):
        document = json.loads(Path(equivalence.LEDGER_PATH).read_text())

        assert_valid_ledger_document(self, document)


if __name__ == "__main__":
    unittest.main()
