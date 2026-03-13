"""Generate markdown comparison reports from results."""

from __future__ import annotations

import json
from pathlib import Path


def load_results(results_dir: Path, run_id: str) -> dict[str, dict]:
    run_dir = results_dir / run_id
    models = {}
    for f in sorted(run_dir.glob("*.json")):
        with open(f) as fh:
            models[f.stem] = json.load(fh)
    return models


def generate_report(results_dir: Path, run_id: str) -> str:
    models = load_results(results_dir, run_id)
    if not models:
        return f"No results found for run_id: {run_id}"

    lines = [
        f"# Vericoding Benchmark Results — {run_id}",
        "",
        "## Comparison Table",
        "",
        "| Language/Model | Easy (15) | Medium (15) | Hard (6) | Total (36) | Rate |",
        "|----------------|-----------|-------------|----------|------------|------|",
        "| Dafny (paper)  | —         | —           | —        | —          | 82%  |",
    ]

    for model_name, data in models.items():
        summary = data.get("summary", {})
        by_diff = summary.get("by_difficulty", {})
        easy = by_diff.get("easy", {})
        medium = by_diff.get("medium", {})
        hard = by_diff.get("hard", {})
        total = summary.get("verified", 0)
        total_applicable = summary.get("total_applicable", 36)
        rate = summary.get("verification_rate", 0)
        compiler = data.get("compiler", "rust")
        compiler_label = f", {compiler}" if compiler != "rust" else ""

        lines.append(
            f"| Vow ({_short_name(model_name)}{compiler_label}) "
            f"| {easy.get('verified', 0)}/{easy.get('total', 15)} "
            f"| {medium.get('verified', 0)}/{medium.get('total', 15)} "
            f"| {hard.get('verified', 0)}/{hard.get('total', 6)} "
            f"| {total}/{total_applicable} "
            f"| {rate:.0%} |"
        )

    lines.extend([
        "| Verus/Rust (paper) | — | — | — | — | 44% |",
        "| Lean (paper)       | — | — | — | — | 27% |",
        "",
    ])

    # Per-model detail sections
    for model_name, data in models.items():
        lines.extend(_model_details(model_name, data))

    return "\n".join(lines)


def _short_name(model_id: str) -> str:
    parts = model_id.split("-")
    if "claude" in model_id:
        # claude-sonnet-4-20250514 -> Sonnet 4
        if len(parts) >= 3:
            return f"{parts[1].title()} {parts[2]}"
    return model_id


def _model_details(model_name: str, data: dict) -> list[str]:
    lines = [
        f"## {model_name}",
        "",
    ]

    summary = data.get("summary", {})
    results = data.get("results", [])

    # CEGIS stats
    verified_iters = [r["iterations"] for r in results if r["status"] == "verified"]
    if verified_iters:
        mean_iters = sum(verified_iters) / len(verified_iters)
        lines.append(f"**Mean CEGIS iterations (verified):** {mean_iters:.1f}")
    lines.append(f"**Total verified:** {summary.get('verified', 0)}/{summary.get('total_applicable', 36)}")
    lines.append("")

    # Failure modes
    failure_counts: dict[str, int] = {}
    for r in results:
        if r["status"] != "verified" and r.get("failure_mode"):
            fm = r["failure_mode"]
            failure_counts[fm] = failure_counts.get(fm, 0) + 1
    if failure_counts:
        lines.append("**Failure modes:**")
        for mode, count in sorted(failure_counts.items(), key=lambda x: -x[1]):
            lines.append(f"- {mode}: {count}")
        lines.append("")

    # Per-benchmark table
    lines.extend([
        "| ID | Name | Status | Iters | Time (s) | Failure |",
        "|----|------|--------|-------|----------|---------|",
    ])
    for r in results:
        status_icon = "pass" if r["status"] == "verified" else "FAIL"
        lines.append(
            f"| {r['benchmark_id']} | {r['benchmark_name']} "
            f"| {status_icon} | {r['iterations']} "
            f"| {r['wall_clock_seconds']:.1f} "
            f"| {r.get('failure_mode', '—') or '—'} |"
        )
    lines.append("")

    # Stretch results
    stretch = [r for r in data.get("stretch_results", [])]
    if stretch:
        lines.extend([
            "### Stretch benchmarks (not counted in rate)",
            "",
            "| ID | Name | Status | Iters |",
            "|----|------|--------|-------|",
        ])
        for r in stretch:
            lines.append(f"| {r['benchmark_id']} | {r['benchmark_name']} | {r['status']} | {r['iterations']} |")
        lines.append("")

    return lines
