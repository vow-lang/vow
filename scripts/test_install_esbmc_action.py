#!/usr/bin/env python3
"""Guards the platform contract of the repository's ESBMC installer action."""

from pathlib import Path
import unittest


REPO_ROOT = Path(__file__).resolve().parent.parent
ACTION = REPO_ROOT / ".github" / "actions" / "install-esbmc" / "action.yml"


class InstallEsbmcActionTest(unittest.TestCase):
    def test_supported_platforms_get_checksum_verified_archives(self) -> None:
        text = ACTION.read_text(encoding="utf-8")

        self.assertIn("esbmc-linux.zip", text)
        self.assertIn("esbmc-macos.zip", text)
        self.assertRegex(
            text,
            r"(?m)^  sha256-macos:\n(?:    .*\n)*?    default: [0-9a-f]{64}$",
        )
        self.assertNotIn("ubuntu-24.04", text)
        self.assertIn("sha256sum", text)
        self.assertIn("shasum -a 256", text)


if __name__ == "__main__":
    unittest.main()
