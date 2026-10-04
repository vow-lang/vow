#!/usr/bin/env python3
"""End-to-end tests for the `vow test` / `vowc test` runner.

Runs every compiler binary it can find (the Rust `vow` and, when present, the
self-hosted `build/vowc`) against a small throwaway tree and checks the
behaviour the spec promises in docs/spec/cli.md: single-file module-root
inference, `--module-root`, `--filter`, `--jobs`, deterministic ordering and
the JSON shape. Binaries that are not built are skipped, so a bare
`cargo build --all` is enough to run the Rust half.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

ENTRY_KEYS = {
    "file",
    "name",
    "status",
    "exit_code",
    "stdout",
    "stderr",
    "duration_ms",
    "diagnostics",
    "counterexamples",
}

SOURCES = {
    "dep.vow": "module Dep\n\nfn d() -> i64 { 41 }\n",
    "tests/test_a.vow": (
        "module A\n\nuse dep\n\nfn main() -> i32 {\n"
        "    if d() == 41 { 0 } else { 1 }\n}\n"
    ),
    "tests/test_b.vow": "module B\n\nfn main() -> i32 { 0 }\n",
    "tests/test_c.vow": "module C\n\nuse nothing\n\nfn main() -> i32 { 0 }\n",
    "tests/test_fail.vow": "module Fail\n\nfn main() -> i32 { 3 }\n",
    "tests/sub/test_deep.vow": (
        "module Deep\n\nuse dep\n\nfn main() -> i32 {\n"
        "    if d() == 41 { 0 } else { 1 }\n}\n"
    ),
}


def binaries() -> list[tuple[str, Path]]:
    found: list[tuple[str, Path]] = []
    candidates = [
        ("rust", os.environ.get("VOW_BIN")),
        ("rust", str(REPO / "target/release/vow")),
        ("rust", str(REPO / "target/debug/vow")),
    ]
    for label, path in candidates:
        if path and Path(path).is_file():
            found.append((label, Path(path)))
            break
    selfhosted = os.environ.get("VOWC_BIN") or str(REPO / "build/vowc")
    if Path(selfhosted).is_file():
        found.append(("self", Path(selfhosted)))
    return found


class HarnessCase(unittest.TestCase):
    label = ""
    binary = Path()

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="vow-test-harness-")
        self.addCleanup(self.tmp.cleanup)
        self.work = Path(self.tmp.name)
        for rel, text in SOURCES.items():
            path = self.work / "proj" / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)

    def run_test(self, *args: str) -> tuple[int, dict]:
        proc = subprocess.run(
            [str(self.binary), "test", *args],
            cwd=self.work,
            capture_output=True,
            text=True,
            timeout=600,
        )
        try:
            return proc.returncode, json.loads(proc.stdout)
        except json.JSONDecodeError:
            self.fail(
                f"{self.label}: no JSON on stdout: {proc.stdout!r} {proc.stderr!r}"
            )

    def statuses(self, doc: dict) -> list[tuple[str, str]]:
        return [(Path(t["file"]).name, t["status"]) for t in doc["tests"]]


def make_cases(label: str, binary: Path) -> list[type[HarnessCase]]:
    class SingleFile(HarnessCase):
        def test_single_file_infers_the_module_root(self) -> None:
            code, doc = self.run_test("proj/tests/test_a.vow")
            self.assertEqual(0, code, doc)
            self.assertEqual("TestsPassed", doc["status"])
            self.assertEqual([("test_a.vow", "passed")], self.statuses(doc))

        def test_nested_single_file_climbs_several_levels(self) -> None:
            code, doc = self.run_test("proj/tests/sub/test_deep.vow")
            self.assertEqual(0, code, doc)
            self.assertEqual([("test_deep.vow", "passed")], self.statuses(doc))

        def test_explicit_module_root_wins(self) -> None:
            code, doc = self.run_test("proj/tests/test_a.vow", "--module-root", "proj")
            self.assertEqual(0, code, doc)
            self.assertEqual("TestsPassed", doc["status"])

        def test_a_wrong_explicit_root_is_not_second_guessed(self) -> None:
            code, doc = self.run_test(
                "proj/tests/test_a.vow", "--module-root", "proj/tests"
            )
            self.assertEqual(1, code)
            self.assertEqual([("test_a.vow", "compile_error")], self.statuses(doc))

        def test_unresolvable_use_is_a_compile_error_with_diagnostics(self) -> None:
            code, doc = self.run_test("proj/tests/test_c.vow")
            self.assertEqual(1, code)
            (entry,) = doc["tests"]
            self.assertEqual("compile_error", entry["status"])
            self.assertTrue(entry["diagnostics"], entry)

    class Directory(HarnessCase):
        def test_directory_scan_is_sorted_and_reports_every_status(self) -> None:
            code, doc = self.run_test("proj")
            self.assertEqual(1, code)
            self.assertEqual("TestsFailed", doc["status"])
            self.assertEqual(
                [
                    ("test_deep.vow", "passed"),
                    ("test_a.vow", "passed"),
                    ("test_b.vow", "passed"),
                    ("test_c.vow", "compile_error"),
                    ("test_fail.vow", "failed"),
                ],
                self.statuses(doc),
            )
            self.assertEqual(5, doc["total"])
            self.assertEqual(3, doc["passed"])
            self.assertEqual(2, doc["failed"])
            self.assertEqual(0, doc["skipped"])

        def test_entries_keep_the_documented_shape(self) -> None:
            _, doc = self.run_test("proj", "--filter", "test_fail")
            (entry,) = doc["tests"]
            self.assertEqual(ENTRY_KEYS, set(entry))
            self.assertEqual(3, entry["exit_code"])
            self.assertEqual("test_fail", entry["name"])
            self.assertIsInstance(entry["duration_ms"], int)
            self.assertEqual(
                {"functions_total", "functions_with_vows", "density_pct"},
                set(doc["contract_density"]),
            )

        def test_job_count_never_changes_the_report(self) -> None:
            reports = []
            for jobs in ("1", "2", "8"):
                code, doc = self.run_test("proj", "--jobs", jobs)
                self.assertEqual(1, code)
                reports.append(
                    (
                        self.statuses(doc),
                        [t.get("exit_code") for t in doc["tests"]],
                        doc["contract_density"],
                    )
                )
            self.assertEqual(reports[0], reports[1])
            self.assertEqual(reports[0], reports[2])

        def test_filter_selects_by_file_stem(self) -> None:
            code, doc = self.run_test("proj", "--filter", "test_b")
            self.assertEqual(0, code)
            self.assertEqual([("test_b.vow", "passed")], self.statuses(doc))

        def test_a_zero_job_count_is_rejected(self) -> None:
            proc = subprocess.run(
                [str(self.binary), "test", "proj", "--jobs", "0"],
                cwd=self.work,
                capture_output=True,
                text=True,
                timeout=60,
            )
            self.assertNotEqual(0, proc.returncode)
            self.assertIn("--jobs must be >= 1", proc.stderr)

    cases = []
    for base in (SingleFile, Directory):
        cls = type(
            f"{base.__name__}_{label}", (base,), {"label": label, "binary": binary}
        )
        cases.append(cls)
    return cases


def load_tests(loader, tests, pattern):
    suite = unittest.TestSuite()
    found = binaries()
    if not found:
        print(
            "test_vow_test_harness: no compiler binary built; skipping", file=sys.stderr
        )
    for label, binary in found:
        for cls in make_cases(label, binary):
            suite.addTests(loader.loadTestsFromTestCase(cls))
    return suite


if __name__ == "__main__":
    unittest.main()
