"""Three specialized agents: Coder, Analyst, Reviewer."""

from __future__ import annotations

import re
from dataclasses import dataclass

from llm import AgentLLM
from prompts import (
    analyst_review_prompt,
    analyst_rereview_prompt,
    coder_feedback_prompt,
    coder_initial_prompt,
    reviewer_review_prompt,
    reviewer_rereview_prompt,
)


@dataclass
class ParsedResponse:
    verdict: str  # "APPROVE" or "NEEDS_WORK"
    suggestions: list[str]
    issues: list[str]
    reasoning: str
    raw: str


def parse_structured_response(text: str) -> ParsedResponse:
    """Parse VERDICT / SUGGESTIONS / ISSUES / REASONING / SUMMARY sections."""
    verdict = "NEEDS_WORK"
    suggestions: list[str] = []
    issues: list[str] = []
    reasoning = ""

    # Extract VERDICT
    m = re.search(r"VERDICT:\s*(APPROVE|NEEDS_WORK)", text, re.IGNORECASE)
    if m:
        verdict = m.group(1).upper()

    # Extract SUGGESTIONS section
    m = re.search(r"SUGGESTIONS:\s*\n(.*?)(?=\n[A-Z]+:|$)", text, re.DOTALL | re.IGNORECASE)
    if m:
        for line in m.group(1).strip().split("\n"):
            line = line.strip()
            if line.startswith("-"):
                suggestions.append(line[1:].strip())

    # Extract ISSUES section
    m = re.search(r"ISSUES:\s*\n(.*?)(?=\n[A-Z]+:|$)", text, re.DOTALL | re.IGNORECASE)
    if m:
        for line in m.group(1).strip().split("\n"):
            line = line.strip()
            if line.startswith("-"):
                issues.append(line[1:].strip())

    # Extract REASONING or SUMMARY
    for header in ("REASONING", "SUMMARY"):
        m = re.search(rf"{header}:\s*\n(.*?)(?=\n[A-Z]+:|$)", text, re.DOTALL | re.IGNORECASE)
        if m:
            reasoning = m.group(1).strip()
            break

    return ParsedResponse(
        verdict=verdict,
        suggestions=suggestions,
        issues=issues,
        reasoning=reasoning,
        raw=text,
    )


def extract_vow_code(response: str) -> str | None:
    """Extract Vow source code from an LLM response."""
    if not response.strip():
        return None

    # Prefer vow-tagged fences over untagged ones
    vow_pattern = re.compile(r"```vow\s*\n(.*?)```", re.DOTALL | re.IGNORECASE)
    vow_matches = vow_pattern.findall(response)
    if vow_matches:
        code = max(vow_matches, key=len).strip()
        if "module " in code or "fn " in code:
            return code

    # Fall back to any fenced block
    fence_pattern = re.compile(r"```\s*\n(.*?)```", re.DOTALL)
    matches = fence_pattern.findall(response)
    if matches:
        code = max(matches, key=len).strip()
        if "module " in code or "fn " in code:
            return code

    lines = response.split("\n")
    for i, line in enumerate(lines):
        if line.strip().startswith("module "):
            return "\n".join(lines[i:]).strip()

    if "fn " in response and "module " in response:
        return response.strip()

    return None


@dataclass
class CoderAgent:
    """Generates and iteratively improves Vow code."""

    llm: AgentLLM

    def generate_initial(self, task_description: str, context: str | None = None) -> str:
        prompt = coder_initial_prompt(task_description, context)
        resp = self.llm.chat(prompt)
        return resp.content

    def incorporate_feedback(
        self,
        verify_feedback: str | None = None,
        analyst_feedback: str | None = None,
        reviewer_feedback: str | None = None,
    ) -> str:
        prompt = coder_feedback_prompt(verify_feedback, analyst_feedback, reviewer_feedback)
        resp = self.llm.chat(prompt)
        return resp.content


@dataclass
class AnalystAgent:
    """Reviews code for missing contracts and suggests property specifications."""

    llm: AgentLLM
    last_suggestions: str = ""

    def review(self, code: str, task_description: str) -> ParsedResponse:
        prompt = analyst_review_prompt(code, task_description)
        resp = self.llm.chat(prompt)
        parsed = parse_structured_response(resp.content)
        self.last_suggestions = resp.content
        return parsed

    def rereview(self, code: str) -> ParsedResponse:
        prompt = analyst_rereview_prompt(code, self.last_suggestions)
        resp = self.llm.chat(prompt)
        parsed = parse_structured_response(resp.content)
        self.last_suggestions = resp.content
        return parsed


@dataclass
class ReviewerAgent:
    """Performs quality and correctness review, flags anti-patterns."""

    llm: AgentLLM
    last_issues: str = ""

    def review(self, code: str, task_description: str) -> ParsedResponse:
        prompt = reviewer_review_prompt(code, task_description)
        resp = self.llm.chat(prompt)
        parsed = parse_structured_response(resp.content)
        self.last_issues = resp.content
        return parsed

    def rereview(self, code: str) -> ParsedResponse:
        prompt = reviewer_rereview_prompt(code, self.last_issues)
        resp = self.llm.chat(prompt)
        parsed = parse_structured_response(resp.content)
        self.last_issues = resp.content
        return parsed
