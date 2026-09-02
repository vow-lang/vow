#!/usr/bin/env python3
"""Guards on where the bootstrap runs, now that it is off the pull-request path.

The three-stage bootstrap and its SHA-256 fixed-point check are the compiler's
central correctness claim: `build/vowc` compiles itself to a byte-identical
binary. Moving that off pull requests trades latency for a later signal, and the
trade is only sound while several things hold at once -- it still runs on every
push to `main` (so a break is attributed to one merge), it still runs nightly
(the backstop), it covers both platforms, and both legs still verify with ESBMC.
Drop any one and the guarantee quietly becomes something weaker than it
reads. These are cheap structural assertions, not a substitute for it running.

Deliberately parses with `re` rather than PyYAML. This module runs in
`build-and-test` before any dependency install, and PyYAML is not in the
standard library -- it is present on today's GitHub runner image by accident,
not by declaration, so importing it would make every code pull request depend on
an image detail. The workflows are uniformly formatted (top-level job keys at
exactly two spaces, bodies deeper), which is all the structure these assertions
need.

Also guards the scheduled full-test and equivalence workflows. That belongs
here rather than in separate modules because it is the same question -- which
workflow carries which equivalence tier -- under the same stdlib-only
constraint.
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
FULL_TEST_WORKFLOW = WORKFLOWS / "full-test.yml"
PROMOTED_FIXTURES_WORKFLOW = WORKFLOWS / "promoted-fixtures.yml"
RELEASE_WORKFLOW = WORKFLOWS / "release.yml"
FULL_TEST_SCRIPT = REPO_ROOT / "scripts" / "full_test.sh"

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
        self.assertIn("runs-on: macos-15", self.jobs["bootstrap-macos"])

    def test_runs_the_bootstrap_script_on_both_platforms(self) -> None:
        for name in ("bootstrap", "bootstrap-macos"):
            with self.subTest(job=name):
                self.assertIn("scripts/bootstrap.sh", self.jobs[name])

    def test_bootstrap_verifies_with_esbmc(self) -> None:
        # --stage3-no-verify halves wall time; Stages 1-2 still verify. A bare
        # --no-verify here would silently drop ESBMC from the whole pipeline.
        for name in ("bootstrap", "bootstrap-macos"):
            with self.subTest(job=name):
                job = self.jobs[name]
                self.assertIn("--stage3-no-verify", job)
                self.assertNotIn("bootstrap.sh --no-verify", job)
                self.assertIn("install-esbmc", job)

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


class ReleaseWorkflowTest(unittest.TestCase):
    def setUp(self) -> None:
        text = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        entry = re.compile(
            r"^\s{10}- os: (\S+)\n"
            r"\s{12}arch: (\S+)\n"
            r"\s{12}runner: (\S+)\n"
            r"\s{12}verify: (true|false)$",
            re.MULTILINE,
        )
        self.matrix = {
            (os_name, arch): {"runner": runner, "verify": verify}
            for os_name, arch, runner, verify in entry.findall(text)
        }

    def test_release_matrix_verifies_supported_platforms(self) -> None:
        self.assertEqual("true", self.matrix[("linux", "x86_64")]["verify"])
        self.assertEqual("true", self.matrix[("macos", "aarch64")]["verify"])
        self.assertEqual(
            {"runner": "macos-15-intel", "verify": "false"},
            self.matrix[("macos", "x86_64")],
        )


class FullTestWorkflowTest(unittest.TestCase):
    def setUp(self) -> None:
        self.text = FULL_TEST_WORKFLOW.read_text(encoding="utf-8")
        self.jobs = job_blocks(self.text)

    def test_runs_on_push_to_main_and_nightly(self) -> None:
        workflow_header = header(self.text)

        self.assertRegex(workflow_header, r"push:\s*\n\s*branches:\s*\[main\]")
        found = crons(self.text)
        self.assertTrue(found, "expected a scheduled run")
        for cron in found:
            with self.subTest(cron=cron):
                self.assertTrue(cron.endswith("* * *"), "expected a daily cron")

    def test_workflow_keeps_read_only_repository_permissions(self) -> None:
        workflow_header = header(self.text)

        self.assertRegex(workflow_header, r"permissions:\s*\n\s*contents:\s*read")
        self.assertNotRegex(workflow_header, r"contents:\s*write")

    def test_gated_on_the_docs_only_classifier(self) -> None:
        changes = self.jobs["changes"]
        full_test = self.jobs["full-test"]

        self.assertIn("fetch-depth: 0", changes)
        self.assertIn("code: ${{ steps.classify.outputs.code }}", changes)
        self.assertIn("python3 scripts/ci_docs_only.py", changes)
        self.assertIn("needs: changes", full_test)
        self.assertIn("if: needs.changes.outputs.code == 'true'", full_test)

    def test_runs_full_test_sh_with_required_toolchain(self) -> None:
        full_test = self.jobs["full-test"]

        self.assertIn("actions/checkout", full_test)
        self.assertIn("dtolnay/rust-toolchain", full_test)
        self.assertIn("Swatinem/rust-cache", full_test)
        self.assertIn("install-esbmc", full_test)
        self.assertIn("astral-sh/setup-uv", full_test)
        self.assertIn("scripts/full_test.sh", full_test)

    def test_enforces_a_minimum_passed_count(self) -> None:
        full_test = self.jobs["full-test"]

        self.assertIn("set -o pipefail", full_test)
        self.assertIn("tee /tmp/full_test.log", full_test)
        self.assertIn(r"grep -oP '\d+(?= passed)'", full_test)
        self.assertIn('test -n "$passed"', full_test)
        self.assertIn('[ "$passed" -ge 500 ]', full_test)

    def test_passed_count_grep_matches_full_test_sh_summary_format(self) -> None:
        # Ties the workflow's grep pattern to the summary line it actually
        # greps: a rename of "passed" in full_test.sh's print_summary would
        # silently disable the floor check while every string-matching test
        # above still passes.
        script = FULL_TEST_SCRIPT.read_text(encoding="utf-8")

        self.assertIn("${PASS} passed", script)


class PromotedFixturesWorkflowTest(unittest.TestCase):
    def setUp(self) -> None:
        self.text = PROMOTED_FIXTURES_WORKFLOW.read_text(encoding="utf-8")
        self.jobs = job_blocks(self.text)

    def test_runs_on_pull_requests_without_duplicate_main_pushes(self) -> None:
        workflow_header = header(self.text)

        self.assertRegex(
            workflow_header,
            r"pull_request:\s*\n\s*branches:\s*\[main\]",
        )
        self.assertNotRegex(workflow_header, r"(?m)^  push:")
        self.assertNotRegex(workflow_header, r"(?m)^  schedule:")

    def test_workflow_keeps_read_only_repository_permissions(self) -> None:
        workflow_header = header(self.text)

        self.assertRegex(workflow_header, r"permissions:\s*\n\s*contents:\s*read")
        self.assertNotRegex(workflow_header, r"contents:\s*write")

    def test_gated_on_the_docs_only_classifier(self) -> None:
        changes = self.jobs["changes"]
        promoted = self.jobs["promoted-fixtures"]

        self.assertIn("fetch-depth: 0", changes)
        self.assertIn("code: ${{ steps.classify.outputs.code }}", changes)
        self.assertIn("python3 scripts/ci_docs_only.py", changes)
        self.assertIn("needs: changes", promoted)
        self.assertIn("if: needs.changes.outputs.code == 'true'", promoted)

    def test_job_is_bounded_and_blocking(self) -> None:
        promoted = self.jobs["promoted-fixtures"]

        self.assertIn("timeout-minutes: 30", promoted)
        self.assertNotIn("continue-on-error", promoted)

    def test_runs_promoted_route_with_required_toolchain(self) -> None:
        promoted = self.jobs["promoted-fixtures"]

        self.assertIn("actions/checkout", promoted)
        self.assertIn("dtolnay/rust-toolchain", promoted)
        self.assertIn("Swatinem/rust-cache", promoted)
        self.assertIn("install-esbmc", promoted)
        self.assertNotIn("astral-sh/setup-uv", promoted)
        self.assertIn(
            "VOW_FULL_TEST_PROMOTED_ONLY=1 scripts/full_test.sh",
            promoted,
        )

    def test_enforces_a_minimum_passed_count(self) -> None:
        promoted = self.jobs["promoted-fixtures"]

        self.assertIn("set -o pipefail", promoted)
        self.assertIn("tee /tmp/promoted_fixtures.log", promoted)
        self.assertIn(r"grep -oP '\d+(?= passed)'", promoted)
        self.assertIn('test -n "$passed"', promoted)
        self.assertIn('[ "$passed" -ge 600 ]', promoted)


class FullTestPromotedGateTest(unittest.TestCase):
    def setUp(self) -> None:
        self.script = FULL_TEST_SCRIPT.read_text(encoding="utf-8")

    def test_compiler_setup_is_a_reusable_step(self) -> None:
        self.assertEqual(2, self.script.count("setup_compilers"))
        self.assertRegex(self.script, r"(?m)^setup_compilers\(\) \{$")

    def test_promoted_fixture_steps_are_shared_by_both_routes(self) -> None:
        for name in ("run_promoted_run_tests", "run_promoted_error_tests"):
            with self.subTest(function=name):
                self.assertEqual(3, self.script.count(name))
                self.assertEqual(
                    1,
                    len(re.findall(rf"(?m)^{name}\(\) \{{$", self.script)),
                )

    def test_promoted_only_route_stops_before_the_complete_suite(self) -> None:
        gate = re.search(
            r'if \[ "\$\{VOW_FULL_TEST_PROMOTED_ONLY:-0\}" = "1" \]; then\n'
            r"(?P<body>.*?)\nfi",
            self.script,
            re.DOTALL,
        )
        self.assertIsNotNone(gate)
        assert gate is not None

        section_zero = self.script.index('section_begin "Section 0: Setup"')
        setup_call = self.script.index("\nsetup_compilers\n", section_zero)
        section_zero_b = self.script.index("# ─── Section 0b")
        self.assertLess(setup_call, gate.start())
        self.assertLess(gate.end(), section_zero_b)

        body = gate.group("body")
        self.assertIn('section_begin "Section 4: Run Tests"', body)
        self.assertIn("run_promoted_run_tests", body)
        self.assertIn('section_begin "Section 7: Error Handling"', body)
        self.assertIn("run_promoted_error_tests", body)
        self.assertIn("print_summary || summary_status=$?", body)
        self.assertIn('exit "$summary_status"', body)


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
        self.assertIn(
            "promoted-fixtures",
            job_blocks(PROMOTED_FIXTURES_WORKFLOW.read_text()),
        )


if __name__ == "__main__":
    unittest.main()
