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

    def test_rejects_module_signature_and_contract_changes(self):
        skeleton = """\
module Example

fn answer(x: i64) -> i64 vow {
  requires: x >= 0,
  ensures: result >= x
} {
  0
}
"""
        changed_sources = {
            "module changed: expected `Example`, found `Other`": skeleton.replace(
                "module Example", "module Other"
            ),
            "signature of `answer` changed": skeleton.replace(
                "answer(x: i64) -> i64", "answer(x: i32) -> i64"
            ),
            "contracts of `answer` changed": skeleton.replace(
                "requires: x >= 0", "requires: true"
            ),
            "skeleton function `answer` is missing": skeleton.replace(
                "fn answer", "fn renamed"
            ),
        }

        for expected_message, candidate in changed_sources.items():
            with self.subTest(expected_message):
                result = compare_skeleton(skeleton, candidate)
                self.assertFalse(result.matches)
                self.assertEqual(result.message, expected_message)

    def test_accepts_reordered_but_equivalent_clauses(self):
        skeleton = """\
module Example

fn answer(x: i64, y: i64) -> i64 vow {
  requires: x >= 0,
  requires: y >= 0,
  ensures: result >= x
} {
  0
}
"""
        candidate = skeleton.replace(
            "requires: x >= 0,\n  requires: y >= 0,",
            "requires: y >= 0,\n  requires: x >= 0,",
        )

        result = compare_skeleton(skeleton, candidate)

        self.assertTrue(result.matches, result.message)

    def test_rejects_contract_deletion(self):
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

fn answer(x: i64) -> i64 {
  x
}
"""

        result = compare_skeleton(skeleton, candidate)

        self.assertFalse(result.matches)
        self.assertEqual(result.message, "contracts of `answer` changed")


if __name__ == "__main__":
    unittest.main()
