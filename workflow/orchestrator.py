"""Main workflow engine: three-agent convergence loop."""

from __future__ import annotations

import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

from agents import (
    AnalystAgent,
    CoderAgent,
    ReviewerAgent,
    extract_vow_code,
)
from llm import AgentLLM, make_config
from prompts import (
    analyst_system_prompt,
    coder_system_prompt,
    format_verify_feedback,
    load_vow_docs,
    reviewer_system_prompt,
)
from verifier import run_compile, run_verify


@dataclass
class RoundResult:
    round_num: int
    code: str
    compile_ok: bool
    verify_status: str | None  # None if compilation failed
    analyst_verdict: str | None
    analyst_suggestions: list[str]
    reviewer_verdict: str | None
    reviewer_issues: list[str]
    converged: bool


@dataclass
class WorkflowResult:
    task_description: str
    final_code: str
    status: str  # "converged", "max_rounds", "compile_failed", "empty_response"
    total_rounds: int
    wall_clock_seconds: float
    rounds: list[RoundResult]
    token_usage: dict[str, dict[str, int]]  # per-agent token usage


EventCallback = Callable[[str, dict], None]


def _try_compile_fix(
    coder: CoderAgent,
    code: str,
    vow_binary: Path,
    memory_limit: int | None,
    max_attempts: int,
    compile_timeout: int,
    on_event: EventCallback | None,
) -> tuple[str, bool]:
    """Attempt to fix compilation errors by feeding them back to the Coder."""
    for attempt in range(max_attempts):
        cr = run_compile(vow_binary, code, timeout=compile_timeout, memory_limit=memory_limit)
        if cr.success:
            return code, True

        if on_event:
            on_event("compile_fix", {"attempt": attempt + 1, "error": cr.parsed})

        feedback = format_verify_feedback({"status": "CompileFailed", **cr.parsed})
        response = coder.incorporate_feedback(verify_feedback=feedback)
        new_code = extract_vow_code(response)
        if new_code is None:
            return code, False
        code = new_code

    # Final check
    cr = run_compile(vow_binary, code, timeout=compile_timeout, memory_limit=memory_limit)
    return code, cr.success


def run_workflow(
    task_description: str,
    root: Path,
    vow_binary: Path,
    config: dict,
    context: str | None = None,
    on_event: EventCallback | None = None,
) -> WorkflowResult:
    """Run the multi-agent convergence loop.

    Args:
        task_description: What the Vow program should do.
        root: Repository root (for loading docs).
        vow_binary: Path to vowc binary.
        config: Parsed config.toml dict.
        context: Optional additional context for the Coder.
        on_event: Optional callback for progress events.
    """
    start = time.time()

    max_rounds = config.get("defaults", {}).get("max_rounds", 5)
    compile_fix_attempts = config.get("defaults", {}).get("compile_fix_attempts", 3)
    verify_timeout = config.get("defaults", {}).get("verify_timeout", 120)
    memory_limit_kb = config.get("defaults", {}).get("memory_limit", 2_000_000)
    memory_limit = memory_limit_kb * 1024

    vow_docs = load_vow_docs(root)

    # Create agents
    agent_configs = config.get("agents", {})

    def _make_agent_llm(role: str, system_fn) -> AgentLLM:
        ac = agent_configs.get(role, {})
        mc = make_config(
            ac.get("model", "claude-sonnet-4-20250514"),
            max_tokens=ac.get("max_tokens", 8192),
            temperature=ac.get("temperature", 0.0),
        )
        return AgentLLM(config=mc, system_prompt=system_fn(vow_docs))

    coder = CoderAgent(llm=_make_agent_llm("coder", coder_system_prompt))
    analyst = AnalystAgent(llm=_make_agent_llm("analyst", analyst_system_prompt))
    reviewer = ReviewerAgent(llm=_make_agent_llm("reviewer", reviewer_system_prompt))

    if on_event:
        on_event("start", {"task": task_description, "max_rounds": max_rounds})

    # Phase 1: Initial code generation
    if on_event:
        on_event("coder_initial", {})

    response = coder.generate_initial(task_description, context)
    code = extract_vow_code(response)
    if code is None:
        elapsed = time.time() - start
        return WorkflowResult(
            task_description=task_description,
            final_code="",
            status="empty_response",
            total_rounds=0,
            wall_clock_seconds=elapsed,
            rounds=[],
            token_usage=_collect_usage(coder, analyst, reviewer),
        )

    rounds: list[RoundResult] = []

    for round_num in range(1, max_rounds + 1):
        if on_event:
            on_event("round_start", {"round": round_num})

        # Pre-round compilation fix
        code, compile_ok = _try_compile_fix(
            coder, code, vow_binary, memory_limit,
            compile_fix_attempts, verify_timeout, on_event,
        )

        if not compile_ok:
            rounds.append(RoundResult(
                round_num=round_num, code=code, compile_ok=False,
                verify_status=None, analyst_verdict=None,
                analyst_suggestions=[], reviewer_verdict=None,
                reviewer_issues=[], converged=False,
            ))
            if on_event:
                on_event("round_end", {"round": round_num, "compile_ok": False})
            continue

        # Verification
        if on_event:
            on_event("verify", {"round": round_num})

        vr = run_verify(vow_binary, code, timeout=verify_timeout, memory_limit=memory_limit)
        verify_status = vr.status

        # Analysis
        if on_event:
            on_event("analyst", {"round": round_num})

        if round_num == 1:
            analyst_result = analyst.review(code, task_description)
        else:
            analyst_result = analyst.rereview(code)

        # Review
        if on_event:
            on_event("reviewer", {"round": round_num})

        if round_num == 1:
            reviewer_result = reviewer.review(code, task_description)
        else:
            reviewer_result = reviewer.rereview(code)

        # Check convergence
        converged = (
            verify_status == "Verified"
            and analyst_result.verdict == "APPROVE"
            and reviewer_result.verdict == "APPROVE"
        )

        round_result = RoundResult(
            round_num=round_num,
            code=code,
            compile_ok=True,
            verify_status=verify_status,
            analyst_verdict=analyst_result.verdict,
            analyst_suggestions=analyst_result.suggestions,
            reviewer_verdict=reviewer_result.verdict,
            reviewer_issues=reviewer_result.issues,
            converged=converged,
        )
        rounds.append(round_result)

        if on_event:
            on_event("round_end", {
                "round": round_num,
                "compile_ok": True,
                "verify_status": verify_status,
                "analyst_verdict": analyst_result.verdict,
                "reviewer_verdict": reviewer_result.verdict,
                "converged": converged,
            })

        if converged:
            elapsed = time.time() - start
            return WorkflowResult(
                task_description=task_description,
                final_code=code,
                status="converged",
                total_rounds=round_num,
                wall_clock_seconds=elapsed,
                rounds=rounds,
                token_usage=_collect_usage(coder, analyst, reviewer),
            )

        # Build feedback for next round (prioritized)
        verify_feedback = None
        if verify_status != "Verified":
            verify_feedback = format_verify_feedback(vr.parsed)

        analyst_feedback = None
        if analyst_result.verdict == "NEEDS_WORK" and analyst_result.suggestions:
            analyst_feedback = "\n".join(f"- {s}" for s in analyst_result.suggestions)

        reviewer_feedback = None
        if reviewer_result.verdict == "NEEDS_WORK" and reviewer_result.issues:
            reviewer_feedback = "\n".join(f"- {i}" for i in reviewer_result.issues)

        if on_event:
            on_event("coder_feedback", {"round": round_num})

        response = coder.incorporate_feedback(
            verify_feedback=verify_feedback,
            analyst_feedback=analyst_feedback,
            reviewer_feedback=reviewer_feedback,
        )
        new_code = extract_vow_code(response)
        if new_code is not None:
            code = new_code

    # Exhausted rounds
    elapsed = time.time() - start
    last_compiled = rounds[-1].compile_ok if rounds else False
    status = "max_rounds" if last_compiled else "compile_failed"

    return WorkflowResult(
        task_description=task_description,
        final_code=code,
        status=status,
        total_rounds=max_rounds,
        wall_clock_seconds=elapsed,
        rounds=rounds,
        token_usage=_collect_usage(coder, analyst, reviewer),
    )


def _collect_usage(
    coder: CoderAgent,
    analyst: AnalystAgent,
    reviewer: ReviewerAgent,
) -> dict[str, dict[str, int]]:
    return {
        "coder": coder.llm.usage.to_dict(),
        "analyst": analyst.llm.usage.to_dict(),
        "reviewer": reviewer.llm.usage.to_dict(),
    }
