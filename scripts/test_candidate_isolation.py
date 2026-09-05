#!/usr/bin/env python3
"""Behavior tests for scripts/candidate_isolation.py."""

import os
import unittest
from pathlib import Path
from unittest import mock

import candidate_isolation


class ScrubbedEnvTest(unittest.TestCase):
    def test_strips_credential_shaped_variables_keeps_the_rest(self):
        source = {
            "ANTHROPIC_API_KEY": "sk-ant-x",
            "OPENAI_API_KEY": "sk-oai-x",
            "GITHUB_TOKEN": "ghp_x",
            "MY_APP_TOKEN": "tok_x",
            "PATH": "/usr/bin:/bin",
            "VOW_CACHE_DIR": "/tmp/cache",
            "HOME": "/home/user",
        }

        result = candidate_isolation.scrubbed_env(source)

        for key in (
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "GITHUB_TOKEN",
            "MY_APP_TOKEN",
        ):
            self.assertNotIn(key, result)
        self.assertEqual(result["PATH"], "/usr/bin:/bin")
        self.assertEqual(result["VOW_CACHE_DIR"], "/tmp/cache")
        self.assertEqual(result["HOME"], "/home/user")

    def test_defaults_to_os_environ(self):
        with mock.patch.dict(
            os.environ,
            {"ANTHROPIC_API_KEY": "sk-ant-x", "PATH": "/usr/bin"},
            clear=True,
        ):
            result = candidate_isolation.scrubbed_env()

        self.assertNotIn("ANTHROPIC_API_KEY", result)
        self.assertEqual(result["PATH"], "/usr/bin")


class DisposableWorkdirTest(unittest.TestCase):
    def test_yields_a_fresh_directory_removed_on_exit(self):
        with candidate_isolation.disposable_workdir() as d:
            path = Path(d)
            self.assertTrue(path.is_dir())

        self.assertFalse(path.exists())


if __name__ == "__main__":
    unittest.main()
