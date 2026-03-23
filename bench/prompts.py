"""System and user prompt construction."""

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


def build_system_prompt(root: Path) -> str:
    skill_dir = root / "docs" / "skill"
    parts = []
    for name in SKILL_FILES:
        path = skill_dir / name
        parts.append(f"# {name}\n\n{path.read_text()}")
    return "\n\n---\n\n".join(parts)


def build_initial_user_prompt(spec_md: str, skeleton_vow: str) -> str:
    return f"""Below is a specification for a Vow function and a skeleton .vow file with the contract already written but the body incomplete.

Fill in the function bodies so that all contracts verify. Return ONLY the complete .vow file, no explanation.

## Specification

{spec_md}

## Skeleton

```vow
{skeleton_vow}
```

Return the complete .vow file with function bodies filled in. Do not change the module name, function signatures, or contracts."""


def _classify_violation(violation: str) -> str:
    """Classify a violation string as requires/ensures/invariant."""
    v = violation.lower()
    if "require" in v:
        return "requires"
    if "ensure" in v:
        return "ensures"
    if "invariant" in v:
        return "invariant"
    return "contract"


def _format_values(values: dict[str, str]) -> str:
    """Format variable bindings into a readable string."""
    if not values:
        return "(no concrete values available)"
    return ", ".join(f"{k}={v}" for k, v in values.items())


def curate_verify_output(
    parsed_json: dict,
    iteration: int,
    previous_violations: list[str],
) -> str:
    """Transform raw verify JSON into a curated CEGIS feedback message."""
    status = parsed_json.get("status", "")
    parts: list[str] = []

    if status == "CompileFailed":
        diagnostics = parsed_json.get("diagnostics", [])
        parts.append("**Compilation failed.** Fix the following errors:\n")
        for d in diagnostics:
            msg = d.get("message", "")
            code = d.get("error_code", "")
            hints = d.get("hints", [])
            parts.append(f"- [{code}] {msg}")
            for h in hints:
                parts.append(f"  Hint: {h}")
        parts.append(
            "\nFix the implementation so it compiles and all contracts verify. "
            "Return ONLY the complete updated .vow file, no explanation."
        )
        return "\n".join(parts)

    counterexamples = parsed_json.get("counterexamples", [])
    func = parsed_json.get("function", "unknown")

    if not counterexamples:
        parts.append(f"Verification failed on function `{func}` but no counterexample was produced.")
        parts.append(f"\nRaw output:\n```json\n{_json_compact(parsed_json)}\n```")
    else:
        parts.append(f"**Verification failed** on function `{func}` (CEGIS iteration {iteration}).\n")
        for i, ce in enumerate(counterexamples):
            violation = ce.get("violation", "unknown")
            blame = ce.get("blame", "unknown")
            values = ce.get("values", {})
            vow_id = ce.get("vow_id", "?")
            vtype = _classify_violation(violation)

            parts.append(f"**Counterexample {i + 1}:**")
            parts.append(f"- Violation: `{vtype}: {violation}` (vow_id={vow_id}, blame={blame})")
            parts.append(f"- Variable values at failure: {_format_values(values)}")

            exec_path = ce.get("execution_path", [])
            if exec_path:
                blocks = [str(step.get("block_id", "?")) for step in exec_path]
                parts.append(f"- Execution path (blocks visited): {' → '.join(blocks)}")

            branch_decisions = ce.get("branch_decisions", [])
            if branch_decisions:
                decisions = [f"branch@{bd.get('condition_offset', '?')}→{bd.get('taken', '?')}"
                             for bd in branch_decisions]
                parts.append(f"- Branch decisions: {', '.join(decisions)}")
            parts.append("")

    if previous_violations:
        parts.append("**Previous failed attempts:**")
        for j, pv in enumerate(previous_violations):
            parts.append(f"- Iteration {j + 1}: {pv}")
        parts.append("")
        parts.append("Do NOT repeat the same approach. Try a different invariant or algorithm.")
        parts.append("")

    # Generate targeted hints based on violation type
    if counterexamples:
        ce0 = counterexamples[0]
        violation = ce0.get("violation", "")
        values = ce0.get("values", {})
        vtype = _classify_violation(violation)
        if vtype == "invariant" and values:
            var_hints = []
            for k, v in values.items():
                var_hints.append(f"`{k}`={v}")
            parts.append(
                f"**Hint:** The loop invariant `{violation}` was falsified with "
                f"{', '.join(var_hints)}. Tighten the invariant or strengthen the loop guard."
            )
        elif vtype == "requires" and values:
            parts.append(
                f"**Hint:** The precondition `{violation}` was violated by the caller. "
                f"Add bounds to the function's `requires` clause to exclude this input."
            )
        elif vtype == "ensures" and values:
            parts.append(
                f"**Hint:** The postcondition `{violation}` was not satisfied. "
                f"Check the algorithm logic for the case where "
                f"{_format_values(values)}."
            )

    parts.append(
        "\nFix the implementation so all contracts verify. "
        "Return ONLY the complete updated .vow file, no explanation."
    )
    return "\n".join(parts)


def _json_compact(d: dict) -> str:
    """Compact JSON for fallback display."""
    import json
    return json.dumps(d, indent=2)


def build_cegis_user_prompt(
    verify_output: str,
    iteration: int = 1,
    previous_violations: list[str] | None = None,
    parsed: dict | None = None,
) -> str:
    """Build a curated CEGIS feedback prompt.

    Falls back to raw JSON if parsed data is not available.
    """
    if parsed is not None:
        return curate_verify_output(parsed, iteration, previous_violations or [])

    # Fallback: raw JSON (backwards compatible)
    return f"""Verification failed. Here is the full JSON output from `vow verify`:

```json
{verify_output}
```

Fix the implementation so all contracts verify. Return ONLY the complete updated .vow file, no explanation."""
