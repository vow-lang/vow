#!/usr/bin/env python3
"""Guards on where the bootstrap runs, now that it is off the pull-request path.

The three-stage bootstrap and its SHA-256 fixed-point check are the compiler's
central correctness claim: `build/vowc` compiles itself to a byte-identical
binary. Moving that off pull requests trades latency for a later signal, and the
trade is only sound while three things hold at once -- it still runs on every
push to `main` (so a break is attributed to one merge), it still runs nightly
(the backstop), and it covers both platforms. Drop any one and the guarantee
quietly becomes something weaker than it reads.

These are cheap structural assertions, not a substitute for the workflow running.
"""

from __future__ import annotations

from pathlib import Path
import unittest

REPO_ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
BOOTSTRAP_WORKFLOW = WORKFLOWS / "bootstrap.yml"
CI_WORKFLOW = WORKFLOWS / "ci.yml"


def load(path: Path):
    """Parse one workflow file.

    Args:
        path: Path to a workflow YAML file.

    Returns:
        dict: The parsed workflow.
    """
    import yaml

    return yaml.safe_load(path.read_text(encoding="utf-8"))


class BootstrapWorkflowTest(unittest.TestCase):
    def setUp(self) -> None:
        self.workflow = load(BOOTSTRAP_WORKFLOW)
        # `on:` is the YAML 1.1 boolean True, not the string "on".
        self.triggers = self.workflow[True]
        self.jobs = self.workflow["jobs"]

    def test_covers_both_platforms(self) -> None:
        runners = {
            name: job["runs-on"]
            for name, job in self.jobs.items()
            if name.startswith("bootstrap")
        }

        self.assertEqual(
            {"bootstrap": "ubuntu-latest", "bootstrap-macos": "macos-latest"},
            runners,
        )

    def test_runs_the_bootstrap_script_on_both_platforms(self) -> None:
        for name in ("bootstrap", "bootstrap-macos"):
            with self.subTest(job=name):
                runs = " ".join(
                    step.get("run", "") for step in self.jobs[name]["steps"]
                )
                self.assertIn("scripts/bootstrap.sh", runs)

    def test_linux_bootstrap_verifies_with_esbmc(self) -> None:
        # --stage3-no-verify halves wall time; Stages 1-2 still verify, and
        # --no-verify here would silently drop ESBMC from the whole pipeline.
        steps = self.jobs["bootstrap"]["steps"]
        runs = " ".join(step.get("run", "") for step in steps)

        self.assertIn("--stage3-no-verify", runs)
        self.assertNotIn("--no-verify ", runs)
        self.assertTrue(
            any("install-esbmc" in str(step.get("uses", "")) for step in steps)
        )

    def test_runs_on_every_push_to_main(self) -> None:
        # The nightly cron alone would attribute a self-hosting break to a day
        # of commits rather than to the merge that caused it.
        self.assertEqual(["main"], self.triggers["push"]["branches"])

    def test_runs_nightly_as_a_backstop(self) -> None:
        crons = [entry["cron"] for entry in self.triggers["schedule"]]

        self.assertTrue(crons)
        for cron in crons:
            with self.subTest(cron=cron):
                self.assertTrue(cron.endswith("* * *"), "expected a daily cron")

    def test_nightly_does_not_collide_with_the_other_scheduled_workflows(self) -> None:
        # Contending for runners makes every nightly slower and flakier.
        schedules = {}
        for path in WORKFLOWS.glob("*.yml"):
            triggers = load(path).get(True) or {}
            for entry in triggers.get("schedule") or []:
                schedules.setdefault(entry["cron"], []).append(path.name)

        collisions = {c: names for c, names in schedules.items() if len(names) > 1}
        self.assertEqual({}, collisions)


class CiWorkflowTest(unittest.TestCase):
    def test_pull_request_ci_no_longer_bootstraps(self) -> None:
        # The point of the move. A bootstrap job reappearing here puts ~900s
        # back on the pull-request path.
        self.assertNotIn("bootstrap.sh", CI_WORKFLOW.read_text(encoding="utf-8"))

    def test_pull_requests_still_compile_the_self_hosted_compiler(self) -> None:
        # What survives on the pull-request path: the concatenated build. It is
        # weaker than a fixed point, but it still fails on a change that stops
        # the compiler compiling itself at all.
        jobs = load(CI_WORKFLOW)["jobs"]
        for name in ("build-and-test", "build-and-test-macos"):
            with self.subTest(job=name):
                runs = " ".join(step.get("run", "") for step in jobs[name]["steps"])
                self.assertIn("concat_vow.sh", runs)


if __name__ == "__main__":
    unittest.main()
