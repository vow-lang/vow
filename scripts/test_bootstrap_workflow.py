#!/usr/bin/env python3
"""Guards on where the bootstrap runs, now that it is off the pull-request path.

The three-stage bootstrap and its SHA-256 fixed-point check are the compiler's
central correctness claim: `build/vowc` compiles itself to a byte-identical
binary. Moving that off pull requests trades latency for a later signal, and the
trade is only sound while several things hold at once -- it still runs on every
push to `main` (so a break is attributed to one merge), it still runs nightly
(the backstop), it covers both platforms, and the Linux leg still verifies with
ESBMC. Drop any one and the guarantee quietly becomes something weaker than it
reads. These are cheap structural assertions, not a substitute for it running.

Deliberately parses with `re` rather than PyYAML. This module runs in
`build-and-test` before any dependency install, and PyYAML is not in the
standard library -- it is present on today's GitHub runner image by accident,
not by declaration, so importing it would make every code pull request depend on
an image detail. The workflows are uniformly formatted (top-level job keys at
exactly two spaces, bodies deeper), which is all the structure these assertions
need.

Also guards `equivalence.yml`'s read-only permissions and ledger-proposal
wiring. That belongs here rather than in its own module because it is the same
question -- which workflow carries which equivalence tier -- under the same
stdlib-only constraint.
"""

from __future__ import annotations

from pathlib import Path
import re
import unittest

REPO_ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
BOOTSTRAP_WORKFLOW = WORKFLOWS / "bootstrap.yml"
CI_WORKFLOW = WORKFLOWS / "ci.yml"
EQUIVALENCE_WORKFLOW = WORKFLOWS / "equivalence.yml"

# A top-level job key: exactly two spaces, a name, a colon, end of line.
JOB_KEY = re.compile(r"^  ([A-Za-z0-9_-]+):[ \t]*$", re.MULTILINE)


def header(text):
    """Everything above `jobs:` — triggers, permissions, and top-level keys."""
    return text[: text.index("\njobs:\n")]


def job_blocks(text):
    """Split a workflow's `jobs:` section into one text block per job.

    Args:
        text: The full workflow file contents.

    Returns:
        dict[str, str]: Job name to the raw text of that job's body.
    """
    jobs_at = text.index("\njobs:\n")
    body = text[jobs_at:]
    matches = list(JOB_KEY.finditer(body))
    blocks = {}
    for n, match in enumerate(matches):
        end = matches[n + 1].start() if n + 1 < len(matches) else len(body)
        blocks[match.group(1)] = body[match.end() : end]
    return blocks


def crons(text):
    """Every cron expression a workflow schedules.

    Args:
        text: The full workflow file contents.

    Returns:
        list[str]: The cron expressions, in file order.
    """
    return re.findall(r"^\s*-\s*cron:\s*[\"']([^\"']+)[\"']", text, re.MULTILINE)


class BootstrapWorkflowTest(unittest.TestCase):
    def setUp(self) -> None:
        self.text = BOOTSTRAP_WORKFLOW.read_text(encoding="utf-8")
        self.jobs = job_blocks(self.text)

    def test_covers_both_platforms(self) -> None:
        self.assertIn("runs-on: ubuntu-latest", self.jobs["bootstrap"])
        self.assertIn("runs-on: macos-latest", self.jobs["bootstrap-macos"])

    def test_runs_the_bootstrap_script_on_both_platforms(self) -> None:
        for name in ("bootstrap", "bootstrap-macos"):
            with self.subTest(job=name):
                self.assertIn("scripts/bootstrap.sh", self.jobs[name])

    def test_linux_bootstrap_verifies_with_esbmc(self) -> None:
        # --stage3-no-verify halves wall time; Stages 1-2 still verify. A bare
        # --no-verify here would silently drop ESBMC from the whole pipeline.
        linux = self.jobs["bootstrap"]

        self.assertIn("--stage3-no-verify", linux)
        self.assertNotIn("bootstrap.sh --no-verify", linux)
        self.assertIn("install-esbmc", linux)

    def compiler_test_step(self) -> str:
        """Just the tier-1 comparison step.

        Job-wide assertions are worthless here: `ulimit` and `timeout-minutes`
        both already appear elsewhere in this job, so a job-scoped `assertIn`
        stays green even if the step is deleted outright.
        """
        linux = self.jobs["bootstrap"]
        start = linux.index("equivalence tier 1")
        return linux[start : linux.index("- name:", start)]

    def test_linux_compares_the_compiler_test_suite_after_bootstrap(self) -> None:
        linux = self.jobs["bootstrap"]
        step = self.compiler_test_step()

        rust_test = "target/release/vow test compiler/"
        self_test = "build/vowc test compiler/"
        comparator = "scripts/parity.py test"
        for command in (rust_test, self_test, comparator):
            with self.subTest(command=command):
                self.assertIn(command, step)
        self.assertLess(linux.index("scripts/bootstrap.sh"), linux.index(self_test))

    def test_the_address_space_cap_covers_only_the_self_hosted_binary(self) -> None:
        # Capping the Rust compiler or python3 as well would turn a memory
        # limit into a spurious parity failure. full_test.sh's run_self scopes
        # it the same way.
        step = self.compiler_test_step()

        self.assertIn("( ulimit -v 2000000; build/vowc test compiler/ )", step)
        self.assertNotRegex(step, r"^\s+ulimit -v \d+$")

    def test_linux_compiler_test_comparison_is_blocking(self) -> None:
        # No `continue-on-error`, and a step-level bound so a #1171 overrun
        # fails this step rather than starving the steps after it.
        step = self.compiler_test_step()

        self.assertNotIn("continue-on-error", step)
        self.assertIn("timeout-minutes:", step)

    def test_runs_on_every_push_to_main(self) -> None:
        # The nightly cron alone would attribute a self-hosting break to a day
        # of commits rather than to the merge that caused it.
        workflow_header = header(self.text)

        self.assertRegex(workflow_header, r"push:\s*\n\s*branches:\s*\[main\]")

    def test_runs_nightly_as_a_backstop(self) -> None:
        found = crons(self.text)

        self.assertTrue(found, "expected a scheduled run")
        for cron in found:
            with self.subTest(cron=cron):
                self.assertTrue(cron.endswith("* * *"), "expected a daily cron")

    def test_nightly_does_not_collide_with_the_other_scheduled_workflows(self) -> None:
        # Contending for runners makes every nightly slower and flakier.
        schedules = {}
        for path in sorted(WORKFLOWS.glob("*.yml")):
            for cron in crons(path.read_text(encoding="utf-8")):
                schedules.setdefault(cron, []).append(path.name)

        collisions = {c: names for c, names in schedules.items() if len(names) > 1}
        self.assertEqual({}, collisions)


class CiWorkflowTest(unittest.TestCase):
    def setUp(self) -> None:
        self.text = CI_WORKFLOW.read_text(encoding="utf-8")

    def test_pull_request_ci_no_longer_bootstraps(self) -> None:
        # The point of the move. A bootstrap job reappearing here puts ~900s
        # back on the pull-request path.
        self.assertNotIn("bootstrap.sh", self.text)

    def test_pull_requests_still_compile_the_self_hosted_compiler(self) -> None:
        # What survives on the pull-request path: the concatenated build. It is
        # weaker than a fixed point, but it still fails on a change that stops
        # the compiler compiling itself at all.
        jobs = job_blocks(self.text)
        for name in ("build-and-test", "build-and-test-macos"):
            with self.subTest(job=name):
                self.assertIn("concat_vow.sh", jobs[name])


class EquivalenceWorkflowTest(unittest.TestCase):
    def setUp(self) -> None:
        self.text = EQUIVALENCE_WORKFLOW.read_text(encoding="utf-8")

    def test_sweep_emits_a_ledger_update_proposal(self) -> None:
        self.assertIn("--emit-ledger-update", self.text)

    def test_uploaded_artifact_contains_the_proposal_directory(self) -> None:
        self.assertIn("--output-dir equivalence.out", self.text)
        self.assertIn("path: equivalence.out", self.text)

    def test_workflow_keeps_read_only_repository_permissions(self) -> None:
        workflow_header = header(self.text)

        self.assertRegex(workflow_header, r"permissions:\s*\n\s*contents:\s*read")
        self.assertNotRegex(workflow_header, r"contents:\s*write")


class ParserTest(unittest.TestCase):
    """The regex parsing is load-bearing for every assertion above."""

    def test_job_blocks_splits_on_top_level_keys_only(self) -> None:
        text = "name: X\njobs:\n  alpha:\n    runs-on: a\n    steps:\n      - run: x\n  beta:\n    runs-on: b\n"
        blocks = job_blocks(text)

        self.assertEqual(["alpha", "beta"], sorted(blocks))
        self.assertIn("runs-on: a", blocks["alpha"])
        self.assertNotIn("runs-on: b", blocks["alpha"])
        self.assertNotIn("steps", blocks["beta"])

    def test_crons_reads_both_quote_styles(self) -> None:
        self.assertEqual(
            ["53 3 * * *", "17 2 * * *"],
            crons(
                "on:\n  schedule:\n    - cron: \"53 3 * * *\"\n    - cron: '17 2 * * *'\n"
            ),
        )

    def test_the_workflows_actually_parse(self) -> None:
        # A formatting change that defeated the splitter would otherwise make
        # every assertion above vacuously pass.
        self.assertIn("bootstrap", job_blocks(BOOTSTRAP_WORKFLOW.read_text()))
        self.assertIn("build-and-test", job_blocks(CI_WORKFLOW.read_text()))


if __name__ == "__main__":
    unittest.main()
