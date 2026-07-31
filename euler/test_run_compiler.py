import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import MagicMock

sys.modules.setdefault("anthropic", MagicMock())
sys.modules.setdefault("openai", MagicMock())

from run import SELF_HOSTED_MEM_LIMIT, resolve_compiler


class ResolveCompilerTests(unittest.TestCase):
    def test_prefers_canonical_self_hosted_compiler(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            self_hosted = root / "build" / "vowc"
            legacy = root / "vowc"
            rust = root / "target" / "release" / "vow"
            for compiler in (self_hosted, legacy, rust):
                compiler.parent.mkdir(parents=True, exist_ok=True)
                compiler.touch()

            self.assertEqual(
                resolve_compiler(root),
                (self_hosted, SELF_HOSTED_MEM_LIMIT, True),
            )

    def test_ignores_legacy_self_hosted_compiler(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            legacy = root / "vowc"
            rust = root / "target" / "release" / "vow"
            for compiler in (legacy, rust):
                compiler.parent.mkdir(parents=True, exist_ok=True)
                compiler.touch()

            self.assertEqual(resolve_compiler(root), (rust, None, False))


if __name__ == "__main__":
    unittest.main()
