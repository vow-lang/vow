import unittest
from pathlib import Path
from unittest.mock import patch

from llm import LLMResponse, ModelConfig
from manifest import BenchmarkInfo
from runner import run_benchmark
from verifier import VerifyResult


SKELETON = """\
module Example

fn answer(x: i64) -> i64 vow {
  requires: x >= 0,
  ensures: result >= x
} {
  0
}
"""


def _benchmark(max_iterations: int = 1) -> BenchmarkInfo:
    return BenchmarkInfo(
        id="T01",
        name="contract fidelity",
        difficulty="easy",
        path="easy/T01_contract_fidelity",
        expected_status="Verified",
        max_cegis_iterations=max_iterations,
        tags=[],
        contract_fidelity="high",
        spec_md="Return x.",
        skeleton_vow=SKELETON,
        reference_vow=SKELETON,
    )


class BenchmarkContractFidelityTests(unittest.TestCase):
    @patch("runner.run_verify")
    @patch("runner.chat")
    def test_deleted_contract_is_rejected_before_verification(
        self, mock_chat, mock_verify
    ):
        mock_chat.return_value = LLMResponse(
            content="""\
module Example

fn answer(x: i64) -> i64 {
  x
}
""",
            input_tokens=10,
            output_tokens=20,
        )

        result = run_benchmark(
            _benchmark(),
            ModelConfig(provider="openai", model_id="test"),
            "system prompt",
            Path("/unused/vow"),
        )

        self.assertEqual(result.status, "contracts_weakened")
        self.assertEqual(result.failure_mode, "contracts_weakened")
        mock_verify.assert_not_called()

    @patch("runner.run_verify")
    @patch("runner.chat")
    def test_model_can_restore_contracts_on_the_next_iteration(
        self, mock_chat, mock_verify
    ):
        weakened = LLMResponse(
            content="""\
module Example

fn answer(x: i64) -> i64 vow {
  requires: true,
  ensures: true
} {
  x
}
""",
            input_tokens=10,
            output_tokens=20,
        )
        restored = LLMResponse(
            content=SKELETON.replace("  0\n}", "  x\n}"),
            input_tokens=11,
            output_tokens=21,
        )
        mock_chat.side_effect = [weakened, restored]
        mock_verify.return_value = VerifyResult(
            status="Verified",
            raw_json='{"status":"Verified"}',
            parsed={"status": "Verified"},
            exit_code=0,
            timed_out=False,
        )

        result = run_benchmark(
            _benchmark(max_iterations=2),
            ModelConfig(provider="openai", model_id="test"),
            "system prompt",
            Path("/unused/vow"),
        )

        self.assertEqual(result.status, "verified")
        self.assertEqual(result.iterations, 2)
        mock_verify.assert_called_once()
        self.assertTrue(
            any(
                "contracts of `answer` changed" in message["content"]
                for message in mock_chat.call_args_list[1].args[2]
                if message["role"] == "user"
            )
        )


if __name__ == "__main__":
    unittest.main()
