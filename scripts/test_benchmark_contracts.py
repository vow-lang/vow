import unittest

from benchmark_contracts import compare_skeleton


class SkeletonFidelityTests(unittest.TestCase):
    def test_accepts_implementation_changes_that_preserve_the_skeleton(self):
        skeleton = """\
module Example

fn answer(x: i64) -> i64 vow {
  requires: x >= 0,
  ensures: result >= x
} {
  0
}
"""
        candidate = """\
module Example

// A helper is allowed because it does not alter a skeleton declaration.
fn helper(x: i64) -> i64 {
  x + 1
}

fn answer(x: i64) -> i64 vow {
  requires: x >= 0,
  ensures: result >= x,
} {
  helper(x)
}
"""

        result = compare_skeleton(skeleton, candidate)

        self.assertTrue(result.matches, result.message)


if __name__ == "__main__":
    unittest.main()
