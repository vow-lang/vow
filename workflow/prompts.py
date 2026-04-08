"""Prompt templates for the three-agent workflow: Coder, Analyst, Reviewer."""

from __future__ import annotations

from pathlib import Path

SKILL_FILES = [
    "index.md",
    "grammar.md",
    "contracts.md",
    "cli.md",
    "errors.md",
    "examples.md",
]


def load_vow_docs(root: Path) -> str:
    """Load Vow language documentation from docs/skill/."""
    skill_dir = root / "docs" / "skill"
    parts = []
    for name in SKILL_FILES:
        path = skill_dir / name
        if path.exists():
            parts.append(f"# {name}\n\n{path.read_text()}")
    return "\n\n---\n\n".join(parts)


# -- System prompts --

def coder_system_prompt(vow_docs: str) -> str:
    return f"""You are an expert Vow programmer. Vow is a verified programming language with contracts (requires, ensures, invariant). You write correct, verified Vow code.

Your role: Generate and iteratively improve Vow programs based on task descriptions and feedback from verification, property analysis, and code review.

Rules:
- Return ONLY the complete .vow file. No explanations, no markdown outside code fences.
- All functions must have appropriate contracts (requires, ensures).
- Loops must have invariants that help ESBMC verify the code.
- Use checked arithmetic (+!, -!, *!) for operations that should not overflow.
- Effects must be declared: io for printing, mem for heap allocation.
- Address verification failures first, then contract suggestions, then review issues.

{vow_docs}"""


def analyst_system_prompt(vow_docs: str) -> str:
    return f"""You are a property analyst for Vow programs. Vow is a verified programming language where contracts (requires, ensures, invariant) are checked by ESBMC, a bounded model checker.

Your role: Review Vow code and identify missing or weak contracts. Suggest specific property specifications that would strengthen verification coverage.

Response format — use these exact section headers:

VERDICT: APPROVE or NEEDS_WORK

SUGGESTIONS:
- Each suggestion on its own line, starting with a dash
- Be specific: name the function, the contract type (requires/ensures/invariant), and the predicate
- Example: "Add `ensures: result >= 0` to fn abs()"

REASONING:
- Brief explanation of why each suggestion matters for verification

{vow_docs}"""


def reviewer_system_prompt(vow_docs: str) -> str:
    return f"""You are a code reviewer for Vow programs. Vow is a verified programming language with contracts checked by ESBMC.

Your role: Review Vow code for correctness, quality, and anti-patterns. Check that the code correctly implements the task description and follows Vow idioms.

Response format — use these exact section headers:

VERDICT: APPROVE or NEEDS_WORK

ISSUES:
- Each issue on its own line, starting with a dash
- Classify severity: [critical], [major], [minor]
- Be specific about what's wrong and how to fix it
- Example: "[critical] fn divide() missing `requires: y != 0` — will fail verification"

SUMMARY:
- Brief overall assessment of code quality

{vow_docs}"""


# -- User prompts --

def coder_initial_prompt(task_description: str, context: str | None = None) -> str:
    parts = [
        "Write a Vow program that implements the following task:\n",
        task_description,
    ]
    if context:
        parts.append(f"\nAdditional context:\n{context}")
    parts.append(
        "\n\nReturn ONLY the complete .vow file with all necessary contracts "
        "(requires, ensures, invariant). Use a module name derived from the task."
    )
    return "\n".join(parts)


def coder_feedback_prompt(
    verify_feedback: str | None = None,
    analyst_feedback: str | None = None,
    reviewer_feedback: str | None = None,
) -> str:
    parts = ["Incorporate the following feedback and return the updated .vow file:\n"]

    if verify_feedback:
        parts.append(f"## Verification Feedback (HIGHEST PRIORITY)\n{verify_feedback}\n")
    if analyst_feedback:
        parts.append(f"## Property Analysis Feedback\n{analyst_feedback}\n")
    if reviewer_feedback:
        parts.append(f"## Code Review Feedback\n{reviewer_feedback}\n")

    parts.append(
        "Address verification failures first, then contract suggestions, then review issues. "
        "Return ONLY the complete updated .vow file."
    )
    return "\n".join(parts)


def analyst_review_prompt(code: str, task_description: str) -> str:
    return f"""Review this Vow code for missing or weak contracts.

## Task Description
{task_description}

## Code
```vow
{code}
```

Analyze each function and suggest any missing requires, ensures, or loop invariant clauses.
Use the VERDICT / SUGGESTIONS / REASONING format."""


def analyst_rereview_prompt(code: str, previous_suggestions: str) -> str:
    return f"""The code has been updated based on your previous suggestions. Re-review it.

## Previous Suggestions
{previous_suggestions}

## Updated Code
```vow
{code}
```

Check if your suggestions were addressed. Identify any remaining gaps.
Use the VERDICT / SUGGESTIONS / REASONING format."""


def reviewer_review_prompt(code: str, task_description: str) -> str:
    return f"""Review this Vow code for correctness and quality.

## Task Description
{task_description}

## Code
```vow
{code}
```

Check for: correct implementation of the task, proper error handling via contracts,
Vow idiom adherence, potential anti-patterns, and overall code quality.
Use the VERDICT / ISSUES / SUMMARY format."""


def reviewer_rereview_prompt(code: str, previous_issues: str) -> str:
    return f"""The code has been updated based on your previous review. Re-review it.

## Previous Issues
{previous_issues}

## Updated Code
```vow
{code}
```

Check if issues were addressed. Identify any remaining problems.
Use the VERDICT / ISSUES / SUMMARY format."""


def format_verify_feedback(parsed: dict) -> str:
    """Format verification result into feedback for the Coder."""
    status = parsed.get("status", "")

    if status == "CompileFailed":
        diagnostics = parsed.get("diagnostics", [])
        lines = ["**Compilation failed.** Fix these errors:\n"]
        for d in diagnostics:
            msg = d.get("message", "")
            code = d.get("error_code", "")
            hints = d.get("hints", [])
            lines.append(f"- [{code}] {msg}")
            for h in hints:
                lines.append(f"  Hint: {h}")
        return "\n".join(lines)

    if status == "VerifyFailed":
        counterexamples = parsed.get("counterexamples", [])
        func = parsed.get("function", "unknown")
        lines = [f"**Verification failed** on function `{func}`.\n"]
        for i, ce in enumerate(counterexamples):
            violation = ce.get("violation", "unknown")
            blame = ce.get("blame", "unknown")
            values = ce.get("values", {})
            vow_id = ce.get("vow_id", "?")
            val_str = ", ".join(f"{k}={v}" for k, v in values.items()) if values else "none"
            lines.append(f"Counterexample {i + 1}:")
            lines.append(f"  Violation: {violation} (vow_id={vow_id}, blame={blame})")
            lines.append(f"  Values: {val_str}")
        return "\n".join(lines)

    return f"Verification returned unexpected status: {status}"
