#!/usr/bin/env python3
"""Triage 162 HumanEval-Dafny tasks for Vow translatability."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path


# Already-translated benchmarks (skip these in triage)
ALREADY_TRANSLATED = {3, 5, 9, 13, 25, 31, 41, 42, 49, 60}


def extract_he_number(source_id: str) -> int:
    """Extract HumanEval number from source-id like 'humaneval_042'."""
    parts = source_id.split("_")
    return int(parts[1][:3])


def classify_task(task: dict) -> tuple[str, str]:
    """Classify a task as translatable/maybe/skip with reason."""
    combined = (
        task.get("vc-spec", "")
        + "\n"
        + task.get("vc-helpers", "")
        + "\n"
        + task.get("vc-preamble", "")
        + "\n"
        + task.get("vc-code", "")
    )

    # Check for unsupported types/features
    checks = [
        ("real", r"\breal\b", "uses real numbers"),
        ("seq<real>", r"seq<real>", "uses seq<real>"),
        ("seq<string>", r"seq<string>", "uses seq<string>"),
        ("seq<seq<", r"seq<seq<", "uses nested sequences"),
        ("set", r"\bset<", "uses set type"),
        ("multiset", r"\bmultiset<", "uses multiset type"),
        ("map", r"\bmap<", "uses map type"),
        ("multi_return", r"returns\s*\([^)]*,", "uses multiple return values"),
    ]

    blockers = []
    for name, pattern, reason in checks:
        if re.search(pattern, combined):
            blockers.append((name, reason))

    if blockers:
        reasons = "; ".join(r for _, r in blockers)
        # String/char tasks are "maybe" (Vow has String but limited verification)
        maybe_only = all(
            n in ("string", "char")
            for n, _ in blockers
        )
        if maybe_only:
            return "maybe", reasons
        return "skip", reasons

    # Check for string/char usage (not in the blockers above)
    has_string = bool(re.search(r"\bstring\b", combined))
    has_char = bool(re.search(r"\bchar\b", combined))
    if has_string or has_char:
        reasons = []
        if has_string:
            reasons.append("uses string type")
        if has_char:
            reasons.append("uses char type")
        return "maybe", "; ".join(reasons)

    return "translatable", "int/bool/seq<int> types only"


def main() -> None:
    jsonl_path = Path("/tmp/dafny_tasks.jsonl")
    if not jsonl_path.exists():
        print("Error: /tmp/dafny_tasks.jsonl not found", file=sys.stderr)
        print("Download it first:", file=sys.stderr)
        print("  curl -sL 'https://raw.githubusercontent.com/Beneficial-AI-Foundation/vericoding-benchmark/main/jsonl/dafny_tasks.jsonl' -o /tmp/dafny_tasks.jsonl", file=sys.stderr)
        sys.exit(1)

    with open(jsonl_path) as f:
        all_tasks = [json.loads(line) for line in f]

    he_tasks = [t for t in all_tasks if t["source"] == "humaneval"]
    print(f"Total HumanEval tasks: {len(he_tasks)}")

    results: list[dict] = []
    counts = {"translatable": 0, "maybe": 0, "skip": 0, "already_done": 0}

    for task in he_tasks:
        he_num = extract_he_number(task["source-id"])
        task_id = task["id"]

        if he_num in ALREADY_TRANSLATED:
            counts["already_done"] += 1
            results.append({
                "dafny_id": task_id,
                "humaneval_num": he_num,
                "source_id": task["source-id"],
                "status": "already_done",
                "reason": f"already translated as HE{he_num:03d}",
            })
            continue

        status, reason = classify_task(task)
        counts[status] += 1

        spec_lines = task["vc-spec"].strip().split("\n")
        sig = spec_lines[0].strip() if spec_lines else ""

        results.append({
            "dafny_id": task_id,
            "humaneval_num": he_num,
            "source_id": task["source-id"],
            "status": status,
            "reason": reason,
            "dafny_signature": sig,
        })

    # Print summary
    print(f"\nTriage results:")
    print(f"  Already translated: {counts['already_done']}")
    print(f"  Translatable:       {counts['translatable']}")
    print(f"  Maybe:              {counts['maybe']}")
    print(f"  Skip:               {counts['skip']}")
    print(f"  Total:              {sum(counts.values())}")

    # Write TOML output
    root = Path(__file__).resolve().parent.parent
    out_path = root / "benchmarks" / "humaneval" / "triage.toml"
    out_path.parent.mkdir(parents=True, exist_ok=True)

    lines = [
        "[triage]",
        f"total_humaneval = {len(he_tasks)}",
        f"already_translated = {counts['already_done']}",
        f"translatable = {counts['translatable']}",
        f"maybe = {counts['maybe']}",
        f"skip = {counts['skip']}",
        "",
    ]

    for r in sorted(results, key=lambda x: x["humaneval_num"]):
        lines.append(f'[[tasks]]')
        lines.append(f'dafny_id = "{r["dafny_id"]}"')
        lines.append(f'humaneval_num = {r["humaneval_num"]}')
        lines.append(f'source_id = "{r["source_id"]}"')
        lines.append(f'status = "{r["status"]}"')
        lines.append(f'reason = "{r["reason"]}"')
        if "dafny_signature" in r:
            sig = r["dafny_signature"].replace('"', '\\"')
            lines.append(f'dafny_signature = "{sig}"')
        lines.append("")

    out_path.write_text("\n".join(lines))
    print(f"\nWritten to {out_path}")

    # Print translatable tasks
    print("\n--- Translatable tasks ---")
    for r in sorted(results, key=lambda x: x["humaneval_num"]):
        if r["status"] == "translatable":
            sig = r.get("dafny_signature", "")[:80]
            print(f"  HE{r['humaneval_num']:03d} ({r['dafny_id']}): {sig}")


if __name__ == "__main__":
    main()
