"""The corpus sweep's gate: which rows count, the bounds, and the summary.

A row is like-for-like when both backends reach a deciding verdict (proven or
refuted) on the same functions. A fixture one side skips or leaves inconclusive
measures how fast each backend declines, not how fast each verifies, so it is
left out of the geomean and the worst-case bound and listed with both
backends' times instead (see docs/adr/2026-10-20-1848-corpus-sweep-like-for-like.md).
Rows both backends decide always gate, native-only cost included.
"""

import math

import verify_diff

RUNS = 5
GEOMEAN_MAX = 1.0
WORST_MAX = 1.25
MIN_COMPARABLE = 1
RATIO_TOLERANCE = 1e-9

DECIDED = (verify_diff.PROVEN, verify_diff.REFUTED)
UNDECIDED = "undecided"


def decided(esbmc, native):
    return esbmc["verdict"] in DECIDED and native["verdict"] in DECIDED


def excluded_reason(gating, cls, esbmc, native):
    if not gating:
        return "non-gating"
    if esbmc["unstable"] or native["unstable"]:
        return "unstable"
    if esbmc["timed_out"] or native["timed_out"]:
        return "timeout"
    if cls != verify_diff.MATCH:
        return cls
    return None if decided(esbmc, native) else UNDECIDED


def undecided_rows(rows):
    """Excluded rows where a backend did not decide, with both verdicts and times."""
    out = []
    for r in rows:
        esbmc, native = r["esbmc"], r["native"]
        if r["excluded"] in (None, "non-gating", "unstable", "timeout"):
            continue
        if decided(esbmc, native):
            continue
        ratio_wall = native["wall_s"] / esbmc["wall_s"] if esbmc["wall_s"] > 0 else None
        out.append(
            {
                "fixture": r["fixture"],
                "class": r["class"],
                "excluded": r["excluded"],
                "detail": r["detail"],
                "esbmc": {"verdict": esbmc["verdict"], "wall_s": esbmc["wall_s"]},
                "native": {"verdict": native["verdict"], "wall_s": native["wall_s"]},
                "ratio_wall": ratio_wall,
            }
        )
    return out


def geomean(values):
    if not values:
        raise ValueError("geometric mean of no values")
    if any(v <= 0 for v in values):
        raise ValueError("geometric mean needs positive values")
    return math.exp(math.fsum(math.log(v) for v in values) / len(values))


def ratio(native, esbmc):
    if native <= 0 or esbmc <= 0:
        raise ValueError(f"cannot form a ratio from {native!r} / {esbmc!r}")
    return native / esbmc


def timeout_buckets(rows):
    buckets = {"native_only": [], "esbmc_only": [], "both": []}
    for r in rows:
        native, esbmc = r["native"]["timed_out"], r["esbmc"]["timed_out"]
        if native and esbmc:
            buckets["both"].append(r["fixture"])
        elif native:
            buckets["native_only"].append(r["fixture"])
        elif esbmc:
            buckets["esbmc_only"].append(r["fixture"])
    return buckets


def metric_summary(comparable, key):
    if not comparable:
        return {"geomean": None, "worst": None}
    worst = max(comparable, key=lambda r: r[key])
    return {
        "geomean": geomean([r[key] for r in comparable]),
        "worst": {"fixture": worst["fixture"], "ratio": worst[key]},
    }


def evaluate_gate(all_rows, runs=RUNS, filtered=False):
    rows = [r for r in all_rows if r["gating"]]
    comparable = [r for r in rows if r["comparable"]]
    metrics = {
        "wall": metric_summary(comparable, "ratio_wall"),
        "rss": metric_summary(comparable, "ratio_rss"),
    }
    timeouts = timeout_buckets(rows)

    def geomean_ok(name):
        value = metrics[name]["geomean"]
        return value is not None and value <= GEOMEAN_MAX * (1 + RATIO_TOLERANCE)

    def worst_ok(name):
        worst = metrics[name]["worst"]
        return worst is not None and worst["ratio"] <= WORST_MAX * (1 + RATIO_TOLERANCE)

    criteria = {
        "enough_comparable": len(comparable) >= MIN_COMPARABLE,
        "geomean_wall": geomean_ok("wall"),
        "geomean_rss": geomean_ok("rss"),
        "worst_wall": worst_ok("wall"),
        "worst_rss": worst_ok("rss"),
        "no_new_timeouts": not timeouts["native_only"],
    }
    return {
        "passed": all(criteria.values()),
        "provisional": runs < RUNS or filtered,
        "criteria": criteria,
        "thresholds": {
            "geomean_max": GEOMEAN_MAX,
            "worst_max": WORST_MAX,
            "runs": RUNS,
            "min_comparable": MIN_COMPARABLE,
        },
        "comparable": len(comparable),
        "excluded": len(rows) - len(comparable),
        "non_gating": len(all_rows) - len(rows),
        "metrics": metrics,
        "timeouts": timeouts,
    }


def format_metric(name, metric):
    if metric["geomean"] is None:
        return f"{name} geomean n/a"
    worst = metric["worst"]
    return (
        f"{name} geomean {metric['geomean']:.3f}x, "
        f"worst {worst['ratio']:.3f}x ({worst['fixture']})"
    )


def print_summary(report, stream):
    gate = report["gate"]
    verdict = "PASS" if gate["passed"] else "FAIL"
    if gate["provisional"]:
        verdict += " (provisional: reduced runs or --filter)"
    print(f"verify_perf: gate {verdict}", file=stream)
    print(
        f"  {gate['comparable']} comparable, {gate['excluded']} excluded "
        f"of {len(report['rows'])} fixtures",
        file=stream,
    )
    for name in ("wall", "rss"):
        print("  " + format_metric(name, gate["metrics"][name]), file=stream)
    for kind, fixtures in gate["timeouts"].items():
        if fixtures:
            print(f"  timeouts {kind}: {', '.join(fixtures)}", file=stream)
    for criterion, ok in gate["criteria"].items():
        if not ok:
            print(f"  criterion failed: {criterion}", file=stream)
    listed = {u["fixture"]: u for u in report["undecided"]}
    for r in report["rows"]:
        if r["fixture"] in listed:
            u = listed[r["fixture"]]
            x = f"{u['ratio_wall']:.2f}x, " if u["ratio_wall"] is not None else ""
            print(
                f"  [not decided: {u['excluded']}] {r['fixture']}: "
                f"esbmc {u['esbmc']['verdict']} {u['esbmc']['wall_s'] * 1000:.1f} ms, "
                f"native {u['native']['verdict']} {u['native']['wall_s'] * 1000:.1f} ms "
                f"({x}not gated)" + (f": {u['detail']}" if u["detail"] else ""),
                file=stream,
            )
        elif not r["comparable"]:
            print(
                f"  [excluded: {r['excluded']}] {r['fixture']}"
                + (f": {r['detail']}" if r["detail"] else ""),
                file=stream,
            )
