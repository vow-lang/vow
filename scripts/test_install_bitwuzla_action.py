#!/usr/bin/env python3
"""Guards the pin and platform contract of the repository's Bitwuzla installer action."""

from pathlib import Path
import unittest


REPO_ROOT = Path(__file__).resolve().parent.parent
ACTION = REPO_ROOT / ".github" / "actions" / "install-bitwuzla" / "action.yml"


class InstallBitwuzlaActionTest(unittest.TestCase):
    def test_supported_platforms_get_checksum_verified_archives(self) -> None:
        text = ACTION.read_text(encoding="utf-8")

        self.assertIn("Bitwuzla-Linux-x86_64-static.zip", text)
        self.assertIn("Bitwuzla-Linux-arm64-static.zip", text)
        self.assertIn("Bitwuzla-macOS-arm64-static.zip", text)
        for input_name in ("sha256", "sha256-macos", "sha256-linux-arm64"):
            with self.subTest(input_name=input_name):
                self.assertRegex(
                    text,
                    rf"(?m)^  {input_name}:\n(?:    .*\n)*?    default: [0-9a-f]{{64}}$",
                )
        self.assertIn("sha256sum", text)
        self.assertIn("shasum -a 256", text)

    def test_version_is_a_release_tag_not_a_rolling_one(self) -> None:
        text = ACTION.read_text(encoding="utf-8")

        self.assertRegex(
            text, r'(?m)^  version:\n(?:    .*\n)*?    default: "\d+\.\d+\.\d+"$'
        )
        self.assertNotIn("latest", text.replace("latest release", ""))

    def test_install_fails_at_install_time_when_binary_is_unusable(self) -> None:
        text = ACTION.read_text(encoding="utf-8")

        self.assertIn("bitwuzla --version", text)

    def test_macos_installs_the_dylibs_the_binary_links(self) -> None:
        text = ACTION.read_text(encoding="utf-8")

        self.assertRegex(
            text, r"if: runner\.os == 'macOS'\n(?:.*\n)*?.*brew install gmp mpfr"
        )


if __name__ == "__main__":
    unittest.main()
