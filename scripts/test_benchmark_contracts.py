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


VEC_SUM_SKELETON = """\
module VecSum

fn make_vec(n: i64) -> Vec<i64> vow {
  requires: n >= 0
} {
  let v: Vec<i64> = Vec::new();
  let mut i: i64 = 0;
  while i < n vow {
    invariant: i >= 0,
    invariant: i <= n
  } {
    v.push(i);
    i = i + 1;
  }
  v
}

fn vec_sum(v: Vec<i64>) -> i64 {
  0
}
"""


class NestedVowClauseTests(unittest.TestCase):
    def test_rejects_deleted_loop_invariant(self):
        candidate = VEC_SUM_SKELETON.replace(
            "while i < n vow {\n    invariant: i >= 0,\n    invariant: i <= n\n  } {",
            "while i < n {",
        )

        result = compare_skeleton(VEC_SUM_SKELETON, candidate)

        self.assertFalse(result.matches)
        self.assertEqual(result.message, "nested contracts of `make_vec` changed")

    def test_rejects_weakened_loop_invariant(self):
        candidate = VEC_SUM_SKELETON.replace(
            "invariant: i >= 0,\n    invariant: i <= n", "invariant: true"
        )

        result = compare_skeleton(VEC_SUM_SKELETON, candidate)

        self.assertFalse(result.matches)
        self.assertEqual(result.message, "nested contracts of `make_vec` changed")

    def test_accepts_reordered_loop_invariants(self):
        candidate = VEC_SUM_SKELETON.replace(
            "invariant: i >= 0,\n    invariant: i <= n",
            "invariant: i <= n,\n    invariant: i >= 0",
        )

        result = compare_skeleton(VEC_SUM_SKELETON, candidate)

        self.assertTrue(result.matches, result.message)

    def test_accepts_invariant_preserved_alongside_an_added_loop(self):
        candidate = VEC_SUM_SKELETON.replace(
            "fn vec_sum(v: Vec<i64>) -> i64 {\n  0\n}",
            "fn vec_sum(v: Vec<i64>) -> i64 {\n"
            "  let mut total: i64 = 0;\n"
            "  let mut i: i64 = 0;\n"
            "  while i < v.len() vow {\n"
            "    invariant: total >= 0\n"
            "  } {\n"
            "    total = total + v[i];\n"
            "    i = i + 1;\n"
            "  }\n"
            "  total\n"
            "}",
        )

        result = compare_skeleton(VEC_SUM_SKELETON, candidate)

        self.assertTrue(result.matches, result.message)


EXTERN_SKELETON = """\
module ExternExample

extern "C" {
  fn write(fd: i32, ptr: i64, len: i64) -> i64;
}

fn main() -> i32 [io] {
  0
}
"""


class ExternBlockTests(unittest.TestCase):
    def test_accepts_extern_block_unchanged(self):
        candidate = EXTERN_SKELETON

        result = compare_skeleton(EXTERN_SKELETON, candidate)

        self.assertTrue(result.matches, result.message)

    def test_rejects_changed_extern_signature(self):
        candidate = EXTERN_SKELETON.replace(
            "fn write(fd: i32, ptr: i64, len: i64) -> i64;",
            "fn write(fd: i64, ptr: i64, len: i64) -> i64;",
        )

        result = compare_skeleton(EXTERN_SKELETON, candidate)

        self.assertFalse(result.matches)
        self.assertEqual(result.message, "signature of `write` changed")

    def test_rejects_changed_extern_contract(self):
        skeleton = EXTERN_SKELETON.replace(
            'extern "C" {',
            'extern "C" {\n  vow {\n    requires: fd >= 0\n  }',
        )
        candidate = skeleton.replace("requires: fd >= 0", "requires: true")

        result = compare_skeleton(skeleton, candidate)

        self.assertFalse(result.matches)
        self.assertEqual(result.message, "contracts of `write` changed")


if __name__ == "__main__":
    unittest.main()
