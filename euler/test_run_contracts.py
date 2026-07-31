import unittest
import sys
from pathlib import Path
from unittest.mock import MagicMock, patch

sys.modules.setdefault("anthropic", MagicMock())
sys.modules.setdefault("openai", MagicMock())

import run as euler_runner


SKELETON = """\
module EulerExample

fn solve(limit: i64) -> i64 vow {
  requires: limit >= 0,
  ensures: result >= 0
} {
  0
}

fn main() -> i32 [io] {
  print_i64(solve(10));
  0
}
"""


def _problem() -> euler_runner.EulerProblem:
    return euler_runner.EulerProblem(
        id="E000",
        euler_number=0,
        name="contract_fidelity",
        difficulty="easy",
        tags=[],
        unwind=10,
        answer=42,
        spec_md="Return the answer.",
        skeleton_vow=SKELETON,
    )


class EulerContractFidelityTests(unittest.TestCase):
    @patch("run.run_execute")
    @patch("run.run_verify")
    @patch("run.chat")
    def test_weakened_contract_is_rejected_before_verify_or_execute(
        self, mock_chat, mock_verify, mock_execute
    ):
        mock_chat.return_value = euler_runner.LLMResponse(
            content="""\
module EulerExample

fn solve(limit: i64) -> i64 vow {
  requires: true,
  ensures: true
} {
  42
}

fn main() -> i32 [io] {
  print_i64(42);
  0
}
""",
            input_tokens=10,
            output_tokens=20,
        )

        result = euler_runner.run_problem(
            _problem(),
            euler_runner.ModelConfig(provider="openai", model_id="test"),
            "system prompt",
            Path("/unused/vow"),
            max_cegis=1,
        )

        self.assertEqual(result.status, "contracts_weakened")
        self.assertIsNone(result.answer_correct)
        mock_verify.assert_not_called()
        mock_execute.assert_not_called()

    @patch("run.run_execute")
    @patch("run.run_verify")
    @patch("run.chat")
    def test_model_can_restore_contracts_before_euler_execution(
        self, mock_chat, mock_verify, mock_execute
    ):
        mock_chat.side_effect = [
            euler_runner.LLMResponse(
                content=SKELETON.replace(
                    "requires: limit >= 0", "requires: true"
                ),
                input_tokens=10,
                output_tokens=20,
            ),
            euler_runner.LLMResponse(
                content=SKELETON.replace("  0\n}", "  42\n}", 1),
                input_tokens=11,
                output_tokens=21,
            ),
        ]
        mock_verify.return_value = euler_runner.VerifyResult(
            status="Verified",
            raw_json='{"status":"Verified"}',
            parsed={"status": "Verified"},
            exit_code=0,
            timed_out=False,
        )
        mock_execute.return_value = (0, "42")

        result = euler_runner.run_problem(
            _problem(),
            euler_runner.ModelConfig(provider="openai", model_id="test"),
            "system prompt",
            Path("/unused/vow"),
            max_cegis=2,
        )

        self.assertEqual(result.status, "verified")
        self.assertTrue(result.answer_correct)
        self.assertEqual(result.iterations, 2)
        mock_verify.assert_called_once()
        mock_execute.assert_called_once()
        self.assertTrue(
            any(
                "contracts of `solve` changed" in message["content"]
                for message in mock_chat.call_args_list[1].args[2]
                if message["role"] == "user"
            )
        )


if __name__ == "__main__":
    unittest.main()
