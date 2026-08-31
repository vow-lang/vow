#!/usr/bin/env python3
"""Behavior tests for scripts/ci_docs_only.py.

The only dangerous answer this classifier can give is a false `docs-only`: it
skips the build, the verifier-evaluation corpus and the bootstrap fixed point,
so a wrong verdict lets a compiler regression onto main unopposed. These tests
therefore concentrate on the ways prose and code get confused -- a spec file
mistaken for documentation, a mixed changeset judged on its Markdown alone, and
an unresolvable commit range treated as evidence of anything at all.
"""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

import ci_docs_only

REPO_ROOT = Path(__file__).resolve().parent.parent


class IsProseTest(unittest.TestCase):
    def test_markdown_outside_the_spec_is_prose(self):
        for path in (
            "README.md",
            "CLAUDE.md",
            "docs/mutants.md",
            "docs/adr/0001-numeric-tower-narrow-ints.md",
            "benchmarks/easy/E01_absolute_value/spec.md",
            "euler/problems/E001_multiples_of_3_or_5/spec.md",
            "stdlib/math/README.md",
        ):
            with self.subTest(path=path):
                self.assertTrue(ci_docs_only.is_prose(path))

    def test_spec_markdown_is_code(self):
        # generate_help.py copies these into the compiler sources, and
        # check_help_coverage.py fails the suite when they drift.
        for path in (
            "docs/spec/grammar.md",
            "docs/spec/cli.md",
            "docs/spec/contracts.md",
            "docs/spec/index.md",
        ):
            with self.subTest(path=path):
                self.assertFalse(ci_docs_only.is_prose(path))

    def test_spec_schemas_are_code(self):
        # include_str!-ed by vow-diag and vow, so editing one changes a binary.
        self.assertFalse(
            ci_docs_only.is_prose("docs/spec/schemas/diagnostic.schema.json")
        )

    def test_the_generated_skill_mirror_is_code(self):
        # generate_help.py writes these, and a cargo test in vow/src/skill.rs
        # asserts they match the compiler-embedded skill byte for byte.
        for path in (
            "skills/vow/SKILL.md",
            "skills/vow/reference/grammar.md",
            "skills/vow/examples/examples.md",
        ):
            with self.subTest(path=path):
                self.assertFalse(ci_docs_only.is_prose(path))

    def test_non_markdown_is_code(self):
        for path in (
            "vow-types/src/env.rs",
            "compiler/lexer.vow",
            "scripts/full_test.sh",
            "scripts/ci_docs_only.py",
            ".github/workflows/ci.yml",
            "Cargo.toml",
            "docs/audits/screenshot.png",
        ):
            with self.subTest(path=path):
                self.assertFalse(ci_docs_only.is_prose(path))

    def test_a_markdown_suffix_is_not_matched_mid_path(self):
        self.assertFalse(ci_docs_only.is_prose("docs/notes.md/build.rs"))


class IsDocsOnlyTest(unittest.TestCase):
    def test_all_prose_is_docs_only(self):
        self.assertTrue(ci_docs_only.is_docs_only(["README.md", "docs/mutants.md"]))

    def test_one_code_path_defeats_the_whole_changeset(self):
        self.assertFalse(ci_docs_only.is_docs_only(["README.md", "vow/src/main.rs"]))

    def test_spec_edit_alongside_prose_is_not_docs_only(self):
        self.assertFalse(
            ci_docs_only.is_docs_only(["CHANGELOG.md", "docs/spec/grammar.md"])
        )

    def test_an_empty_changeset_is_not_docs_only(self):
        # Nothing to prove the change is inert, so the full suite runs.
        self.assertFalse(ci_docs_only.is_docs_only([]))

    def test_blank_lines_are_ignored_not_counted_as_prose(self):
        self.assertFalse(ci_docs_only.is_docs_only(["", ""]))


def git(repo, *args):
    subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True)


class ChangedPathsTest(unittest.TestCase):
    """Exercises the git plumbing: the three-dot range and rename handling."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)
        git(self.repo, "init", "-q", "-b", "main")
        git(self.repo, "config", "user.email", "t@example.com")
        git(self.repo, "config", "user.name", "t")
        (self.repo / "README.md").write_text("base\n")
        (self.repo / "lib.rs").write_text("fn main() {}\n")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-qm", "base")

        self.cwd = os.getcwd()
        os.chdir(self.repo)
        self.addCleanup(os.chdir, self.cwd)

    def head(self):
        return subprocess.run(
            ["git", "-C", str(self.repo), "rev-parse", "HEAD"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()

    def commit(self, message):
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-qm", message)
        return self.head()

    def test_prose_only_range_is_docs_only(self):
        base = self.head()
        (self.repo / "README.md").write_text("changed\n")
        head = self.commit("docs: tweak")

        paths = ci_docs_only.changed_paths(base, head)

        self.assertEqual(["README.md"], paths)
        self.assertTrue(ci_docs_only.is_docs_only(paths))

    def test_code_in_the_range_is_not_docs_only(self):
        base = self.head()
        (self.repo / "README.md").write_text("changed\n")
        self.commit("docs: tweak")
        (self.repo / "lib.rs").write_text("fn main() { }\n")
        head = self.commit("fix: tweak")

        paths = ci_docs_only.changed_paths(base, head)

        self.assertEqual(["README.md", "lib.rs"], sorted(paths))
        self.assertFalse(ci_docs_only.is_docs_only(paths))

    def test_a_rename_reports_both_endpoints(self):
        # --no-renames matters: with rename detection a code file moved to a .md
        # name would report only the destination and read as pure prose.
        base = self.head()
        git(self.repo, "mv", "lib.rs", "notes.md")
        head = self.commit("chore: move")

        paths = ci_docs_only.changed_paths(base, head)

        self.assertEqual(["lib.rs", "notes.md"], sorted(paths))
        self.assertFalse(ci_docs_only.is_docs_only(paths))

    def test_commits_on_the_base_branch_are_excluded(self):
        # A PR must be judged against its merge base, not against a base branch
        # tip that moved on after the PR was opened.
        base = self.head()
        (self.repo / "README.md").write_text("pr\n")
        head = self.commit("docs: pr change")
        git(self.repo, "checkout", "-q", base)
        (self.repo / "lib.rs").write_text("fn other() {}\n")
        moved_base = self.commit("fix: landed on base after the PR opened")

        paths = ci_docs_only.changed_paths(moved_base, head)

        self.assertEqual(["README.md"], paths)
        self.assertTrue(ci_docs_only.is_docs_only(paths))

    def test_a_missing_endpoint_refuses_to_classify(self):
        head = self.head()
        for base in ("", ci_docs_only.NULL_SHA):
            with self.subTest(base=base):
                self.assertIsNone(ci_docs_only.changed_paths(base, head))

    def test_an_unknown_revision_refuses_to_classify(self):
        self.assertIsNone(ci_docs_only.changed_paths("deadbeef" * 5, self.head()))


def tracked_markdown(*roots):
    """Tracked Markdown paths under the given repository roots.

    Args:
        roots: Repository-relative directories to list.

    Returns:
        list[str]: The tracked paths ending in `.md`.
    """
    out = subprocess.run(
        ["git", "-C", str(REPO_ROOT), "ls-files", "--", *roots],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    return [p for p in out if p.endswith(".md")]


class RepositoryTreeTest(unittest.TestCase):
    """Anchors the rule to the real tree, so a file added under a build-input
    root is covered without anyone remembering to extend a literal list."""

    def test_no_tracked_markdown_under_a_build_input_root_reads_as_prose(self):
        paths = tracked_markdown(*ci_docs_only.BUILD_INPUT_PREFIXES)

        self.assertTrue(paths, "expected tracked Markdown under the build-input roots")
        for path in paths:
            with self.subTest(path=path):
                self.assertFalse(ci_docs_only.is_prose(path))

    def test_the_skill_mirror_this_gate_must_protect_is_tracked(self):
        # If this file ever moves, BUILD_INPUT_PREFIXES has to move with it.
        self.assertIn("skills/vow/SKILL.md", tracked_markdown("skills"))

    def test_ordinary_repository_prose_still_reads_as_prose(self):
        # The gate is worthless if it classifies everything as code.
        paths = tracked_markdown("README.md", "CLAUDE.md", "docs/adr", "benchmarks")

        self.assertTrue(paths)
        for path in paths:
            with self.subTest(path=path):
                self.assertTrue(ci_docs_only.is_prose(path))


class MainTest(unittest.TestCase):
    def test_unresolvable_range_reports_code_true_and_exits_zero(self):
        proc = subprocess.run(
            ["python3", str(Path(__file__).with_name("ci_docs_only.py"))],
            capture_output=True,
            text=True,
        )

        self.assertEqual(0, proc.returncode)
        self.assertEqual("code=true", proc.stdout.strip())


if __name__ == "__main__":
    unittest.main()
