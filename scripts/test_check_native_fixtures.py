"""full_test.sh Section 4g and scripts/check_native_fixtures.sh must read the same directives.

Section 4g of full_test.sh is too large a file to edit under the file token budget, so it keeps
its own copy of the per-fixture loop that scripts/check_native_fixtures.sh (run on every pull
request by .github/workflows/native-fixtures.yml) implements. This test fails when a `// TEST:`
directive is read by one and not the other, so the two cannot drift apart silently.
"""

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DIRECTIVE = re.compile(r"// TEST: ([a-z][a-z-]*[a-z])")


def directives(text):
    return set(DIRECTIVE.findall(text))


def section_4g():
    text = (REPO_ROOT / "scripts" / "full_test.sh").read_text()
    start = text.index("Section 4g: Native Verifier (tests/verify-native/")
    end = text.index("Section 4h: Unknown CLI flags")
    return text[start:end]


class NativeFixtureDirectiveParityTest(unittest.TestCase):
    def test_section_4g_and_the_pull_request_script_read_the_same_directives(self):
        script = (REPO_ROOT / "scripts" / "check_native_fixtures.sh").read_text()
        in_full_test = directives(section_4g())
        in_script = directives(script)
        self.assertTrue(in_script, "no directive found in check_native_fixtures.sh")
        self.assertEqual(
            sorted(in_full_test),
            sorted(in_script),
            "full_test.sh Section 4g and scripts/check_native_fixtures.sh disagree on the "
            "// TEST: directives they read; update both",
        )


if __name__ == "__main__":
    unittest.main()
