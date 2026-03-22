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

    # Compute dynamic column sizes from first model's data
    first_data = next(iter(models.values()))
    first_summary = first_data.get("summary", {})
    by_diff = first_summary.get("by_difficulty", {})
    easy_total = by_diff.get("easy", {}).get("total", 0)
    medium_total = by_diff.get("medium", {}).get("total", 0)
    hard_total = by_diff.get("hard", {}).get("total", 0)
    grand_total = first_summary.get("total_applicable", easy_total + medium_total + hard_total)

    lines = [
        f"# Vericoding Benchmark Results — {run_id}",
        "",
        "## Comparison Table",
        "",
        f"| Language/Model | Easy ({easy_total}) | Medium ({medium_total}) | Hard ({hard_total}) | Total ({grand_total}) | Rate |",
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
        total_applicable = summary.get("total_applicable", grand_total)
        rate = summary.get("verification_rate", 0)
        compiler = data.get("compiler", "rust")
        compiler_label = f", {compiler}" if compiler != "rust" else ""

        lines.append(
            f"| Vow ({_short_name(model_name)}{compiler_label}) "
            f"| {easy.get('verified', 0)}/{easy.get('total', easy_total)} "
            f"| {medium.get('verified', 0)}/{medium.get('total', medium_total)} "
            f"| {hard.get('verified', 0)}/{hard.get('total', hard_total)} "
            f"| {total}/{total_applicable} "
            f"| {rate:.0%} |"
        )

    lines.extend([
        "| Verus/Rust (paper) | — | — | — | — | 44% |",
        "| Lean (paper)       | — | — | — | — | 27% |",
        "",
    ])

    # HumanEval fidelity table (if any HE benchmarks present)
    _add_fidelity_table(lines, models)

    # Per-model detail sections
    for model_name, data in models.items():
        lines.extend(_model_details(model_name, data))

    return "\n".join(lines)


def _add_fidelity_table(lines: list[str], models: dict[str, dict]) -> None:
    has_he = False
    for data in models.values():
        for r in data.get("results", []):
            if r["benchmark_id"].startswith("HE"):
                has_he = True
                break
        if has_he:
            break
    if not has_he:
        return

    lines.extend([
        "## HumanEval Contract Fidelity",
        "",
        "| Model | HE-All | HE-Exact | HE-Partial | HE-Weak |",
        "|-------|--------|----------|------------|---------|",
    ])

    for model_name, data in models.items():
        results = data.get("results", [])
        he = [r for r in results if r["benchmark_id"].startswith("HE")]
        if not he:
            continue

        by_fid: dict[str, dict] = {}
        for r in he:
            fid = r.get("contract_fidelity", "n/a")
            if fid not in by_fid:
                by_fid[fid] = {"total": 0, "verified": 0}
            by_fid[fid]["total"] += 1
            if r["status"] == "verified":
                by_fid[fid]["verified"] += 1

        he_total = len(he)
        he_verified = sum(1 for r in he if r["status"] == "verified")
        exact = by_fid.get("exact", {"total": 0, "verified": 0})
        partial = by_fid.get("partial", {"total": 0, "verified": 0})
        weak = by_fid.get("weak", {"total": 0, "verified": 0})

        compiler = data.get("compiler", "rust")
        compiler_label = f", {compiler}" if compiler != "rust" else ""

        lines.append(
            f"| Vow ({_short_name(model_name)}{compiler_label}) "
            f"| {he_verified}/{he_total} "
            f"| {exact['verified']}/{exact['total']} "
            f"| {partial['verified']}/{partial['total']} "
            f"| {weak['verified']}/{weak['total']} |"
        )

    lines.append("")


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
    lines.append(f"**Total verified:** {summary.get('verified', 0)}/{summary.get('total_applicable', len(results))}")
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
        "| ID | Name | Fidelity | Status | Iters | Time (s) | Failure |",
        "|----|------|----------|--------|-------|----------|---------|",
    ])
    for r in results:
        status_icon = "pass" if r["status"] == "verified" else "FAIL"
        fidelity = r.get("contract_fidelity", "n/a")
        lines.append(
            f"| {r['benchmark_id']} | {r['benchmark_name']} "
            f"| {fidelity} "
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
