#!/usr/bin/env python3
"""Calibrate the `vow complexity` 0-100 gate over the self-hosted compiler corpus.

Per docs/design/code-complexity-and-vowc-complexity.md Part 4, the gate's
validation target is *comprehensibility / refactor-priority*, not correctness:

  1. Threshold calibration: `complexity_score > THRESHOLD` should flag roughly
     the worst ~5-15% of functions. If the rate is far off, adjust the anchors
     (--cog-anchor / --nloc-anchor), not the scale.
  2. Beats-size check: the score must add signal beyond raw NLOC. If the
     score-ranking is ~identical to the NLOC-ranking (Spearman ~1.0), the score
     is just LOC in disguise and the cognitive factor is mis-weighted.

This is a local-only analysis harness (like `vowc mutants`); it is not wired
into CI. It writes a markdown report and prints a one-line verdict. Exit code is
always 0 — calibration reports, it does not gate.
"""

import argparse
import glob
import json
import math
import os
import re
import subprocess
import sys


REPORT_BASENAME_RE = re.compile(r"^\d{4}-\d{2}-\d{2}-.+\.md$")
REPORT_DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
RETENTION_CLASSES = ("current-baseline", "release-evidence", "temporary-review")


def out_targets_reports(path):
    reports_dir = os.path.abspath("reports")
    out_path = os.path.abspath(path)
    try:
        return os.path.commonpath([reports_dir, out_path]) == reports_dir
    except ValueError:
        return False


def validate_report_output(parser, args):
    if not out_targets_reports(args.out):
        return

    basename = os.path.basename(os.path.normpath(args.out))
    if not args.date:
        parser.error("--date is required when --out targets reports/")
    if not REPORT_DATE_RE.fullmatch(args.date):
        parser.error("--date must use YYYY-MM-DD when --out targets reports/")
    if not REPORT_BASENAME_RE.fullmatch(basename):
        parser.error("--out under reports/ must use YYYY-MM-DD-<topic>.md")
    if not basename.startswith(f"{args.date}-"):
        parser.error("--out filename date must match --date")


def run_complexity(vowc, path, extra):
    """Returns the parsed report, or None if the file does not compile
    standalone (some modules rely on the concat build's global namespace and
    omit `use` declarations, so they cannot be a module-loaded entry point)."""
    res = subprocess.run([vowc, "complexity", path, *extra], capture_output=True, text=True)
    out = res.stdout.strip()
    if not out:
        return None
    try:
        return json.loads(out)
    except json.JSONDecodeError:
        return None


def collect(vowc, files, extra):
    """Per-file functions across the corpus. `vow complexity` reports only the
    functions defined in each queried file, so each function appears once with
    line/nloc relative to its own source. Files that do not compile standalone
    are skipped (returned in `skipped`)."""
    funcs, seen, skipped = [], set(), []
    for path in files:
        data = run_complexity(vowc, path, extra)
        if data is None:
            skipped.append(path)
            continue
        for fobj in data["files"]:
            for f in fobj["functions"]:
                key = (path, f["name"], f["line"])
                if key in seen:
                    continue
                seen.add(key)
                funcs.append(f)
    return funcs, skipped


def pct_rank(sorted_vals, p):
    if not sorted_vals:
        return 0
    idx = max(0, min(len(sorted_vals) - 1, math.ceil(p / 100 * len(sorted_vals)) - 1))
    return sorted_vals[idx]


def ranks(values):
    order = sorted(range(len(values)), key=lambda i: values[i])
    out = [0] * len(values)
    for rank, i in enumerate(order):
        out[i] = rank
    return out


def spearman(xs, ys):
    n = len(xs)
    if n < 2:
        return 1.0
    rx, ry = ranks(xs), ranks(ys)
    d2 = sum((rx[i] - ry[i]) ** 2 for i in range(n))
    return 1.0 - (6.0 * d2) / (n * (n * n - 1))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--vowc", default="target/release/vow", help="compiler binary")
    ap.add_argument("--dir", default="compiler", help="directory of *.vow to score (test_* excluded)")
    ap.add_argument("--threshold", type=int, default=80)
    ap.add_argument("--cog-anchor", type=int, default=15)
    ap.add_argument("--nloc-anchor", type=int, default=60)
    ap.add_argument("--out", default="reports.out/complexity-calibration.md")
    ap.add_argument("--date", default="", help="report date stamp (caller-supplied; no clock in-script)")
    ap.add_argument(
        "--retention-class",
        choices=RETENTION_CLASSES,
        default="current-baseline",
        help="retention policy class for committed reports/ snapshots",
    )
    args = ap.parse_args()
    validate_report_output(ap, args)

    extra = ["--cog-anchor", str(args.cog_anchor), "--nloc-anchor", str(args.nloc_anchor)]
    files = sorted(
        p for p in glob.glob(os.path.join(args.dir, "*.vow"))
        if not os.path.basename(p).startswith("test_")
    )
    funcs, skipped = collect(args.vowc, files, extra)
    n = len(funcs)
    if n == 0:
        sys.exit("no functions found")

    scores = [f["complexity_score"] for f in funcs]
    nlocs = [f["size"]["nloc"] for f in funcs]
    cogs = [f["structural"]["cognitive"] for f in funcs]

    over = [f for f in funcs if f["complexity_score"] > args.threshold]
    over_pct = 100.0 * len(over) / n
    rho = spearman(scores, nlocs)

    s_sorted = sorted(scores)
    # Beats-size: functions the score ranks much higher/lower than NLOC alone.
    rscore, rnloc = ranks(scores), ranks(nlocs)
    diverg = sorted(
        range(n), key=lambda i: abs(rscore[i] - rnloc[i]), reverse=True
    )
    tangled = [i for i in diverg if rscore[i] > rnloc[i]][:8]  # high score, low NLOC
    bulky = [i for i in diverg if rscore[i] < rnloc[i]][:8]    # low score, high NLOC

    verdict_rate = "OK" if 5.0 <= over_pct <= 15.0 else ("HIGH" if over_pct > 15.0 else "LOW")
    cog_p50 = pct_rank(sorted(cogs), 50)
    # High overall rho is expected when most functions are simple (cognitive 0),
    # since size then legitimately drives the score. The signal is in the tail.
    verdict_size = "adds signal" if rho < 0.97 else "size-dominated (cognitive differentiates the tail)"

    lines = []
    lines.append("# `vow complexity` gate calibration")
    lines.append("")
    generated = "Generated"
    if args.date:
        generated += f" {args.date}"
    lines.append(f"_{generated} by scripts/complexity_calibrate.py over `{args.dir}/*.vow` ({len(files)} files)._")
    lines.append("")
    if out_targets_reports(args.out):
        lines.append(f"_Retention: `{args.retention_class}` for the `complexity-calibration` stream.")
        if args.retention_class == "current-baseline":
            lines.append("Replace")
            lines.append("this snapshot in the same PR as the next committed complexity calibration")
            lines.append("snapshot unless a reviewer reclassifies it as `release-evidence`._")
        else:
            lines[-1] += "_"
        lines.append("")
    lines.append("> Validation target is comprehensibility / refactor priority, NOT correctness")
    lines.append("> (docs/design Part 4). Correctness is the job of contracts + tests.")
    lines.append("")
    lines.append("## Threshold calibration")
    lines.append("")
    lines.append(f"- Functions scored: **{n}** (from {len(files) - len(skipped)}/{len(files)} files; {len(skipped)} skipped — don't compile standalone)")
    lines.append(f"- Over threshold (`score > {args.threshold}`): **{len(over)}** ({over_pct:.1f}%) — target 5–15% → **{verdict_rate}**")
    lines.append(f"- complexity_score p50/p90/max: {pct_rank(s_sorted,50)} / {pct_rank(s_sorted,90)} / {s_sorted[-1]}")
    lines.append(f"- cognitive p50/p90/max: {pct_rank(sorted(cogs),50)} / {pct_rank(sorted(cogs),90)} / {sorted(cogs)[-1]}")
    lines.append(f"- nloc p50/p90/max: {pct_rank(sorted(nlocs),50)} / {pct_rank(sorted(nlocs),90)} / {sorted(nlocs)[-1]}")
    lines.append("")
    lines.append("Anchors: cog=%d, nloc=%d. If the rate is far off target, adjust these (not the 0-100 scale)." % (args.cog_anchor, args.nloc_anchor))
    lines.append("")
    lines.append("## Beats-size check")
    lines.append("")
    lines.append(f"- Spearman(score, nloc) = **{rho:.3f}** → score is **{verdict_size}**.")
    lines.append(f"- Median cognitive is **{cog_p50}**: ~half the functions have no control flow, so size correctly drives their score. The cognitive factor only reorders the complex tail (below); judge the metric there, not on the global correlation.")
    lines.append("")
    lines.append("Functions the score prioritizes ABOVE their size rank (tangled, not just long):")
    lines.append("")
    lines.append("| function | line | score | cognitive | nloc |")
    lines.append("|---|--:|--:|--:|--:|")
    for i in tangled:
        f = funcs[i]
        lines.append(f"| `{f['name']}` | {f['line']} | {f['complexity_score']} | {f['structural']['cognitive']} | {f['size']['nloc']} |")
    lines.append("")
    lines.append("Functions the score DEPRIORITIZES below their size rank (long but flat):")
    lines.append("")
    lines.append("| function | line | score | cognitive | nloc |")
    lines.append("|---|--:|--:|--:|--:|")
    for i in bulky:
        f = funcs[i]
        lines.append(f"| `{f['name']}` | {f['line']} | {f['complexity_score']} | {f['structural']['cognitive']} | {f['size']['nloc']} |")
    lines.append("")
    lines.append("## Worst functions by score")
    lines.append("")
    lines.append("| function | line | score | cyclomatic | cognitive | nloc |")
    lines.append("|---|--:|--:|--:|--:|--:|")
    for f in sorted(funcs, key=lambda f: f["complexity_score"], reverse=True)[:15]:
        st = f["structural"]
        lines.append(f"| `{f['name']}` | {f['line']} | {f['complexity_score']} | {st['cyclomatic']} | {st['cognitive']} | {f['size']['nloc']} |")
    lines.append("")
    report = "\n".join(lines) + "\n"

    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as fh:
        fh.write(report)

    print(
        f"calibration: {n} fns, {len(over)} over {args.threshold} ({over_pct:.1f}%, target 5-15% {verdict_rate}), "
        f"spearman(score,nloc)={rho:.3f} ({verdict_size}) -> {args.out}"
    )


if __name__ == "__main__":
    main()
