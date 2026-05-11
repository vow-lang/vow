import unittest

from prompts import curate_verify_output


class CegisPromptTests(unittest.TestCase):
    def test_requires_feedback_repairs_caller_instead_of_weakening_contract(self):
        prompt = curate_verify_output(
            {
                "status": "VerifyFailed",
                "function": "main",
                "counterexamples": [
                    {
                        "violation": "requires: n > 0",
                        "blame": "Caller",
                        "vow_id": 7,
                        "values": {"n": "0"},
                    }
                ],
            },
            iteration=1,
            previous_violations=[],
        )

        self.assertNotIn("Add bounds", prompt)
        self.assertNotIn("exclude this input", prompt)
        self.assertNotIn("correct the precondition", prompt)
        self.assertIn("fix the call site", prompt)
        self.assertIn("guard before the call", prompt)

    def test_requires_counterexample_includes_structured_caller_context(self):
        prompt = curate_verify_output(
            {
                "status": "VerifyFailed",
                "function": "main",
                "counterexamples": [
                    {
                        "violation": "requires: y != 0",
                        "blame": "Caller",
                        "vow_id": 3,
                        "values": {"y": "0"},
                        "source": {"file": "test.vow", "offset": 12, "length": 8},
                        "call_sites": [
                            {
                                "caller_function": "main",
                                "file": "test.vow",
                                "offset": 50,
                                "length": 15,
                            }
                        ],
                        "violating_args": [
                            {
                                "param": "y",
                                "value": "0",
                                "arg_offset": 59,
                                "arg_length": 1,
                            }
                        ],
                    }
                ],
            },
            iteration=1,
            previous_violations=[],
        )

        self.assertIn("test.vow@12+8", prompt)
        self.assertIn("main in test.vow@50+15", prompt)
        self.assertIn("y=0 at arg@59+1", prompt)
        self.assertIn("Use the caller context", prompt)

    def test_requires_context_hint_does_not_require_variable_values(self):
        prompt = curate_verify_output(
            {
                "status": "VerifyFailed",
                "function": "main",
                "counterexamples": [
                    {
                        "violation": "requires: y != 0",
                        "blame": "Caller",
                        "vow_id": 3,
                        "values": {},
                        "call_sites": [
                            {
                                "caller_function": "main",
                                "file": "test.vow",
                                "offset": 50,
                                "length": 15,
                            }
                        ],
                    }
                ],
            },
            iteration=1,
            previous_violations=[],
        )

        self.assertIn("precondition", prompt)
        self.assertIn("fix the call site", prompt)
        self.assertIn("Use the caller context", prompt)

    def test_ensures_feedback_still_points_to_algorithm_logic(self):
        prompt = curate_verify_output(
            {
                "status": "VerifyFailed",
                "function": "abs",
                "counterexamples": [
                    {
                        "violation": "ensures: result >= 0",
                        "blame": "Callee",
                        "vow_id": 9,
                        "values": {"x": "-5", "result": "-5"},
                    }
                ],
            },
            iteration=2,
            previous_violations=[],
        )

        self.assertIn("postcondition", prompt)
        self.assertIn("Check the algorithm logic", prompt)
        self.assertIn("x=-5, result=-5", prompt)


if __name__ == "__main__":
    unittest.main()
