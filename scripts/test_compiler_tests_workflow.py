#!/usr/bin/env python3
"""Guards on the pull-request run of the compiler unit tests.

bootstrap.yml runs `test compiler/` under both compilers only after a merge, so
compiler-tests.yml runs the same `scripts/compiler_test_parity.sh` on pull
requests that touch the compiler. Stdlib-only, like test_bootstrap_workflow.py.
"""

from __future__ import annotations

import unittest

from test_bootstrap_workflow import WORKFLOWS, header, job_blocks

COMPILER_TESTS_WORKFLOW = WORKFLOWS / "compiler-tests.yml"


class CompilerTestsWorkflowTest(unittest.TestCase):
    """The pull-request run of the compiler unit tests (#1903 process gap)."""

    def setUp(self) -> None:
        self.text = COMPILER_TESTS_WORKFLOW.read_text(encoding="utf-8")
        self.jobs = job_blocks(self.text)

    def test_runs_on_pull_requests_only(self) -> None:
        # bootstrap.yml already runs the suite on push to main and nightly.
        workflow_header = header(self.text)

        self.assertRegex(workflow_header, r"pull_request:\s*\n\s*branches:\s*\[main\]")
        self.assertNotIn("push:", workflow_header)
        self.assertNotIn("schedule:", workflow_header)

    def test_job_is_gated_on_the_compiler_classifier_output(self) -> None:
        job = self.jobs["compiler-tests"]

        self.assertIn("needs.changes.outputs.compiler == 'true'", job)
        self.assertIn("scripts/ci_docs_only.py", self.jobs["changes"])

    def test_reuses_the_bootstrap_and_the_shared_comparison_script(self) -> None:
        job = self.jobs["compiler-tests"]

        self.assertLess(
            job.index("scripts/bootstrap.sh"),
            job.index("scripts/compiler_test_parity.sh"),
        )
        self.assertNotIn("parity.py", job)

    def test_comparison_is_blocking_and_bounded(self) -> None:
        job = self.jobs["compiler-tests"]

        self.assertNotIn("continue-on-error", job)
        self.assertIn("timeout-minutes: 40", job)


if __name__ == "__main__":
    unittest.main()
