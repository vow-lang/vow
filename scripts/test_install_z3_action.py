#!/usr/bin/env python3
"""Guards the pin and platform contract of the repository's Z3 installer action.

Z3 is CI-only (#1431): it must be a checksum-pinned numbered release, installed
only by the workflows that cross-check the native verifier, never by a job that
ships a product.
"""

from pathlib import Path
import re
import unittest


REPO_ROOT = Path(__file__).resolve().parent.parent
ACTION = REPO_ROOT / ".github" / "actions" / "install-z3" / "action.yml"
WORKFLOWS = REPO_ROOT / ".github" / "workflows"


class InstallZ3ActionTest(unittest.TestCase):
    def setUp(self) -> None:
        self.text = ACTION.read_text(encoding="utf-8")

    def test_supported_platforms_get_checksum_verified_archives(self) -> None:
        self.assertIn("x64-glibc", self.text)
        self.assertIn("arm64-glibc", self.text)
        self.assertIn("arm64-osx", self.text)
        for input_name in ("sha256", "sha256-macos", "sha256-linux-arm64"):
            with self.subTest(input_name=input_name):
                self.assertRegex(
                    self.text,
                    rf"(?m)^  {input_name}:\n(?:    .*\n)*?    default: [0-9a-f]{{64}}$",
                )
        self.assertIn("sha256sum", self.text)
        self.assertIn("shasum -a 256", self.text)

    def test_version_is_a_numbered_release_not_the_nightly(self) -> None:
        self.assertRegex(
            self.text, r'(?m)^  version:\n(?:    .*\n)*?    default: "\d+\.\d+\.\d+"$'
        )
        self.assertNotIn("/Nightly/", self.text)
        self.assertIn("releases/download/z3-$Z3_VERSION/", self.text)

    def test_install_fails_at_install_time_when_binary_is_unusable(self) -> None:
        self.assertIn("z3 --version", self.text)
        self.assertRegex(self.text, r'\*"\$Z3_VERSION"\*')

    def test_only_the_cross_check_workflow_installs_z3(self) -> None:
        users = [
            workflow.name
            for workflow in sorted(WORKFLOWS.glob("*.yml"))
            if "install-z3" in workflow.read_text(encoding="utf-8")
        ]

        self.assertEqual(["native-oracle.yml"], users)

    def test_the_pin_lives_only_in_the_action(self) -> None:
        hashes = re.findall(r"(?m)^    default: ([0-9a-f]{64})$", self.text)
        self.assertEqual(3, len(hashes))

        for workflow in sorted(WORKFLOWS.glob("*.yml")):
            text = workflow.read_text(encoding="utf-8")
            with self.subTest(workflow=workflow.name):
                for needle in hashes:
                    self.assertNotIn(needle, text)
                self.assertNotRegex(text, r"install-z3\n\s+with:")


if __name__ == "__main__":
    unittest.main()
