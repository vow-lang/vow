#!/usr/bin/env python3
"""Behavior tests for scripts/generate_operations.py.

The generator is the Operation Catalogue's single writer/checker for the
runtime-symbol/ABI/return-shape/doc projections it splices into both
compilers. The tests live in scripts/opsgen_tests/ (split to fit the
authored-file budget); this file runs them all, so `python3
scripts/test_generate_operations.py` keeps working.
"""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from opsgen_tests.test_ops_01 import *  # noqa: E402, F403
from opsgen_tests.test_ops_02 import *  # noqa: E402, F403
from opsgen_tests.test_ops_03 import *  # noqa: E402, F403
from opsgen_tests.test_ops_04 import *  # noqa: E402, F403
from opsgen_tests.test_ops_05 import *  # noqa: E402, F403
from opsgen_tests.test_ops_06 import *  # noqa: E402, F403
from opsgen_tests.test_ops_07 import *  # noqa: E402, F403

if __name__ == "__main__":
    unittest.main()
