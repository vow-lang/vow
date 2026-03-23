"""Core CEGIS runner: one benchmark, one model."""

from __future__ import annotations

import re
import time
from dataclasses import dataclass, field
from pathlib import Path

from llm import LLMResponse, ModelConfig, chat
from manifest import BenchmarkInfo
from prompts import build_cegis_user_prompt, build_initial_user_prompt
from verifier import VerifyResult, run_verify


@dataclass
class BenchmarkResult:
    benchmark_id: str
    benchmark_name: str
    difficulty: str
    contract_fidelity: str
    status: str  # verified / verify_failed / compile_failed / timeout / max_iterations / empty_response
    iterations: int
    wall_clock_seconds: float
    failure_mode: str | None
    token_usage: dict[str, int]
    final_code: str
    raw_responses: list[str] = field(default_factory=list)
    verify_outputs: list[str] = field(default_factory=list)


def extract_vow_code(response: str) -> str | None:
    if not response.strip():
        return None

    # Try to extract from markdown code fence
    fence_pattern = re.compile(r"```(?:vow)?\s*\n(.*?)```", re.DOTALL)
    matches = fence_pattern.findall(response)
    if matches:
        # Use the longest match (likely the full file)
        code = max(matches, key=len).strip()
        if "module " in code or "fn " in code:
            return code

    # Try to find raw code starting with "module"
    lines = response.split("\n")
    start = None
    for i, line in enumerate(lines):
        if line.strip().startswith("module "):
            start = i
            break
    if start is not None:
        return "\n".join(lines[start:]).strip()

    # Last resort: if response contains fn declarations, use as-is
    if "fn " in response and ("module " in response or "vow {" in response):
        return response.strip()

    return None


def classify_failure(verify_result: VerifyResult) -> str:
    if verify_result.timed_out:
        return "esbmc_timeout"

    parsed = verify_result.parsed
    status = parsed.get("status", "")
    diagnostics = parsed.get("diagnostics", [])

    if status == "CompileFailed":
        for d in diagnostics:
            msg = d.get("message", "").lower()
            code = d.get("error_code", "")
            if "parse" in msg or "unexpected" in msg or code == "ParseError":
                return "syntax_error"
            if "effect" in msg:
                return "effect_violation"
        return "type_error"

    if status == "VerifyFailed":
        return "wrong_algorithm"

    return "unknown"


def run_benchmark(
    bench: BenchmarkInfo,
    model_config: ModelConfig,
    system_prompt: str,
    vow_binary: Path,
    verify_timeout: int = 120,
    memory_limit: int | None = None,
) -> BenchmarkResult:
    start = time.time()
    max_iters = bench.max_cegis_iterations
    messages: list[dict[str, str]] = []
    raw_responses: list[str] = []
    verify_outputs: list[str] = []
    previous_violations: list[str] = []
    total_input = 0
    total_output = 0
    final_code = ""

    # Initial prompt
    user_msg = build_initial_user_prompt(bench.spec_md, bench.skeleton_vow)
    messages.append({"role": "user", "content": user_msg})

    for iteration in range(1, max_iters + 1):
        # Call LLM
        resp: LLMResponse = chat(model_config, system_prompt, messages)
        raw_responses.append(resp.content)
        total_input += resp.input_tokens
        total_output += resp.output_tokens

        messages.append({"role": "assistant", "content": resp.content})

        # Extract code
        code = extract_vow_code(resp.content)
        if code is None:
            elapsed = time.time() - start
            return BenchmarkResult(
                benchmark_id=bench.id,
                benchmark_name=bench.name,
                difficulty=bench.difficulty,
                contract_fidelity=bench.contract_fidelity,
                status="empty_response",
                iterations=iteration,
                wall_clock_seconds=elapsed,
                failure_mode="empty_response",
                token_usage={"input_tokens": total_input, "output_tokens": total_output},
                final_code="",
                raw_responses=raw_responses,
                verify_outputs=verify_outputs,
            )

        final_code = code

        # Verify
        vr = run_verify(vow_binary, code, timeout=verify_timeout, memory_limit=memory_limit)
        verify_outputs.append(vr.raw_json)

        if vr.status == "Verified":
            elapsed = time.time() - start
            return BenchmarkResult(
                benchmark_id=bench.id,
                benchmark_name=bench.name,
                difficulty=bench.difficulty,
                contract_fidelity=bench.contract_fidelity,
                status="verified",
                iterations=iteration,
                wall_clock_seconds=elapsed,
                failure_mode=None,
                token_usage={"input_tokens": total_input, "output_tokens": total_output},
                final_code=final_code,
                raw_responses=raw_responses,
                verify_outputs=verify_outputs,
            )

        # Track violation summary for previous-attempt context
        violation_summary = ""
        if vr.parsed:
            ces = vr.parsed.get("counterexamples", [])
            if ces:
                ce0 = ces[0]
                violation_summary = f"violation: {ce0.get('violation', '?')}"
                vals = ce0.get("values", {})
                if vals:
                    val_str = ", ".join(f"{k}={v}" for k, v in vals.items())
                    violation_summary += f" ({val_str})"
            elif vr.parsed.get("status") == "CompileFailed":
                diags = vr.parsed.get("diagnostics", [])
                if diags:
                    violation_summary = f"compile error: {diags[0].get('message', '?')}"
        previous_violations.append(violation_summary)

        # Not verified — if iterations remain, send CEGIS feedback
        if iteration < max_iters:
            cegis_msg = build_cegis_user_prompt(
                vr.raw_json,
                iteration=iteration,
                previous_violations=previous_violations,
                parsed=vr.parsed,
            )
            messages.append({"role": "user", "content": cegis_msg})

    # Exhausted iterations
    elapsed = time.time() - start
    last_vr = run_verify(vow_binary, final_code, timeout=verify_timeout, memory_limit=memory_limit) if final_code else None
    failure_mode = classify_failure(last_vr) if last_vr else "empty_response"
    status_map = {
        "CompileFailed": "compile_failed",
        "VerifyFailed": "verify_failed",
        "Timeout": "timeout",
    }
    final_status = "max_iterations"
    if last_vr:
        final_status = status_map.get(last_vr.status, "max_iterations")

    return BenchmarkResult(
        benchmark_id=bench.id,
        benchmark_name=bench.name,
        difficulty=bench.difficulty,
        contract_fidelity=bench.contract_fidelity,
        status=final_status,
        iterations=max_iters,
        wall_clock_seconds=elapsed,
        failure_mode=failure_mode,
        token_usage={"input_tokens": total_input, "output_tokens": total_output},
        final_code=final_code,
        raw_responses=raw_responses,
        verify_outputs=verify_outputs,
    )
