#!/usr/bin/env python3
"""Performance harness: native verifier vs pinned ESBMC (epic #1398, issue #1420).

Times `vowc verify --backend esbmc` against `--backend native` over the gate
corpus (the non-stretch benchmark references, tests/verify*, and the float
fixtures inside them): median of five runs per backend, cache off, same
machine. Wall-clock is the process wall time; peak RSS is the larger of the
sampled sum over the whole process tree (driver plus solver children) and the
kernel's `ru_maxrss` for the run when that is above the harness's own peak.

The replacement gate (acceptance gate item 3) passes when, over the rows both
backends handle identically (class `match` in scripts/verify_diff.py terms):

  * the geometric mean of native/esbmc is <= 1.0 on wall-clock and on RSS,
  * no single fixture's native/esbmc ratio exceeds 1.25 on either metric,
  * native times out on no fixture where ESBMC finishes (checked over every
    row, comparable or not), and
  * at least one row is comparable.

Verdict parity is gate item 1 and belongs to scripts/verify_diff.py; here it
only decides which rows can be compared.

Output is one JSON document (stdout, or --output FILE) and a short human
summary on stderr. Exit code: 0 gate passed, 1 gate failed, 2 the harness
could not run (missing tool, wrong ESBMC version, corpus drift, no fixture).

Usage:
    scripts/verify_perf.py [--vowc build/vowc] [--filter NAME] [--runs N]
                           [--timeout S] [--verify-jobs N] [--output FILE]
"""

import argparse
import json
import math
import os
import platform
import re
import resource
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
from collections import Counter, namedtuple

import candidate_isolation
import measure_build_tree_rss
import verify_diff
import verify_eval

REPO_ROOT = verify_eval.REPO_ROOT
BENCHMARKS_DIR = os.path.join(REPO_ROOT, "benchmarks")

SCHEMA_VERSION = 1

RUNS = 5
GEOMEAN_MAX = 1.0
WORST_MAX = 1.25
MIN_COMPARABLE = 1
GATE_BENCHMARKS = 23

NON_GATING_GROUPS = ("verify-stress",)

ESBMC_PIN = "8.5"
BACKENDS = verify_diff.BACKENDS

# Shipped per-function default: 300s kernel watchdog plus the 30s BV phase.
DEFAULT_BUDGET_S = 330
SAMPLE_INTERVAL_S = 0.02

RATIO_TOLERANCE = 1e-9

Fixture = namedtuple("Fixture", "group name path truth")

NATIVE_DIR_TRUTH = {
    "pass": "Verified",
    "fail": "VerifyFailed",
    "skip": "Skipped",
    "unknown": None,
}

VERSION_RE = re.compile(r"\d+\.\d+(?:\.\d+)?")


def median(values):
    return statistics.median(values)


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


def evaluate_gate(rows, runs=RUNS, filtered=False):
    all_rows = rows
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


def benchmark_fixtures(manifest):
    out = []
    for entry in manifest["benchmarks"]:
        if entry["expected_status"] == "Stretch":
            continue
        path = os.path.join(BENCHMARKS_DIR, entry["path"], "reference.vow")
        out.append(
            Fixture(
                "benchmarks",
                os.path.basename(entry["path"]),
                path,
                entry["expected_status"],
            )
        )
    return out


def verify_eval_fixtures():
    out = []
    for sub, exp in verify_eval.collect(None):
        if exp.skip_reason:
            continue
        truth = "VerifyFailed" if exp.known_gap else exp.expected_status
        out.append(Fixture(sub, exp.name, exp.path, truth))
    return out


def native_fixtures():
    out = []
    for sub, truth in NATIVE_DIR_TRUTH.items():
        out += dir_fixtures(f"verify-native/{sub}", truth)
    return out


def dir_fixtures(group, truth):
    directory = os.path.join(REPO_ROOT, "tests", *group.split("/"))
    if not os.path.isdir(directory):
        return []
    return [
        Fixture(group, name[: -len(".vow")], os.path.join(directory, name), truth)
        for name in sorted(os.listdir(directory))
        if name.endswith(".vow")
    ]


def multi_fixtures():
    directory = os.path.join(REPO_ROOT, "tests", "verify-fail-multi")
    if not os.path.isdir(directory):
        return []
    return [
        Fixture(
            "verify-fail-multi",
            name,
            os.path.join(directory, name, "main.vow"),
            "VerifyFailed",
        )
        for name in sorted(os.listdir(directory))
        if os.path.isfile(os.path.join(directory, name, "main.vow"))
    ]


def collect_corpus(include_stress=False):
    with open(os.path.join(BENCHMARKS_DIR, "manifest.toml"), "rb") as fh:
        benchmarks = benchmark_fixtures(tomllib.load(fh))
    if len(benchmarks) != GATE_BENCHMARKS:
        raise ValueError(
            f"corpus drift: {len(benchmarks)} non-stretch benchmarks, "
            f"the gate is defined over {GATE_BENCHMARKS}"
        )
    corpus = benchmarks + verify_eval_fixtures() + multi_fixtures() + native_fixtures()
    missing = [fx.path for fx in corpus if not os.path.isfile(fx.path)]
    if missing:
        raise ValueError(f"corpus drift: fixture file missing: {missing[0]}")
    if include_stress:
        corpus += dir_fixtures("verify-stress", None)
    return corpus


def fixture_function_count(fx):
    if fx.group != "verify-fail-multi":
        return verify_diff.function_count(fx.path)
    directory = os.path.dirname(fx.path)
    return sum(
        verify_diff.function_count(os.path.join(directory, name))
        for name in sorted(os.listdir(directory))
        if name.endswith(".vow")
    )


def sample_tree_rss(pid, stop, peaks):
    while not stop.is_set():
        tree, own = measure_build_tree_rss.snapshot(pid)
        peaks["tree"] = max(peaks["tree"], tree)
        peaks["self"] = max(peaks["self"], own)
        stop.wait(SAMPLE_INTERVAL_S)


def parse_result(text, killed, status):
    if killed:
        return verify_diff.synthetic_failure("timeout")
    try:
        return json.loads(text.strip())
    except json.JSONDecodeError:
        pass
    if os.WIFSIGNALED(status):
        return verify_diff.synthetic_failure("crashed")
    return None


def run_once(vowc, backend, path, jobs, timeout, watchdog_s):
    """Run one `vowc verify`; return wall time, peak RSS and the parsed result.

    The child leads its own session so a watchdog kill also reaches the solver
    processes it spawned. Wall time ends the instant the child exits (blocking
    waitid), not at the next poll tick.
    """
    args = [vowc, "verify", "--no-cache", "--backend", backend]
    args += ["--verify-jobs", str(jobs)]
    if timeout is not None:
        args += ["--timeout", str(timeout)]
    args.append(path)
    with tempfile.TemporaryDirectory(prefix="verify-perf-") as scratch:
        cache = os.path.join(scratch, "cache")
        os.mkdir(cache)
        env = candidate_isolation.scrubbed_env()
        env["VOW_CACHE_DIR"] = cache
        out_path = os.path.join(scratch, "stdout")
        err_path = os.path.join(scratch, "stderr")
        with open(out_path, "wb") as out, open(err_path, "wb") as err:
            start = time.monotonic()
            proc = subprocess.Popen(
                args,
                stdout=out,
                stderr=err,
                cwd=REPO_ROOT,
                env=env,
                start_new_session=True,
            )
            measured = wait_measured(proc.pid, watchdog_s, start)
        proc.returncode = os.waitstatus_to_exitcode(measured["status"])
        with open(out_path, "r", encoding="utf-8", errors="replace") as fh:
            stdout = fh.read()
        with open(err_path, "r", encoding="utf-8", errors="replace") as fh:
            stderr = fh.read()
    result = parse_result(stdout, measured["killed"], measured["status"])
    if result is None:
        print(
            f"verify_perf: {backend} printed no JSON for {path}: {stderr.rstrip()}",
            file=sys.stderr,
        )
    timed_out = measured["killed"] or (
        result is not None and result.get("verify_status") == "timeout"
    )
    maxrss = measured["rusage"].ru_maxrss
    return {
        "wall_s": measured["wall_s"],
        "peak_rss_kb": max(measured["tree"], trusted_maxrss(maxrss)),
        "rss_self_kb": measured["self"],
        "rss_tree_kb": measured["tree"],
        "rss_maxrss_kb": maxrss,
        "result": result,
        "timed_out": timed_out,
        "exit_code": proc.returncode,
    }


def trusted_maxrss(maxrss_kb):
    """`ru_maxrss` of a reaped child, or 0 when it cannot be told from the floor.

    Linux seeds a child's high-water mark with its parent's at exec time, so a
    value at or below the harness's own peak says nothing about the child.
    """
    floor = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return maxrss_kb if maxrss_kb > floor else 0


def wait_measured(pid, watchdog_s, start):
    peaks = {"tree": 0, "self": 0}
    stop = threading.Event()
    killed = threading.Event()

    def kill_group():
        killed.set()
        try:
            os.killpg(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass

    sampler = threading.Thread(target=sample_tree_rss, args=(pid, stop, peaks))
    watchdog = threading.Timer(watchdog_s, kill_group)
    sampler.start()
    watchdog.start()
    try:
        os.waitid(os.P_PID, pid, os.WEXITED | os.WNOWAIT)
        wall_s = time.monotonic() - start
    except BaseException:
        kill_group()
        try:
            os.waitpid(pid, 0)
        except ChildProcessError:
            pass
        raise
    finally:
        watchdog.cancel()
        stop.set()
        sampler.join()
    _, status, rusage = os.wait4(pid, 0)
    return {
        "wall_s": wall_s,
        "status": status,
        "rusage": rusage,
        "killed": killed.is_set(),
        "tree": peaks["tree"],
        "self": peaks["self"],
    }


def summarise_backend(samples):
    verdicts = [verify_diff.verdict_of(s["result"]) for s in samples]
    common = Counter(verdicts).most_common(1)[0][0]
    result = samples[verdicts.index(common)]["result"] or {}
    timeout_runs = sum(1 for s in samples if s["timed_out"])
    return {
        "verdict": common,
        "verify_status": result.get("verify_status"),
        "status": result.get("status"),
        "counterexamples": verify_eval.actual_cex(result),
        "wall_s": median([s["wall_s"] for s in samples]),
        "peak_rss_kb": median([s["peak_rss_kb"] for s in samples]),
        "wall_samples": [s["wall_s"] for s in samples],
        "rss_samples": [s["peak_rss_kb"] for s in samples],
        "timeout_runs": timeout_runs,
        "timed_out": timeout_runs * 2 > len(samples),
        "unstable": len(set(verdicts)) > 1,
    }


def excluded_reason(gating, cls, esbmc, native):
    if not gating:
        return "non-gating"
    if esbmc["unstable"] or native["unstable"]:
        return "unstable"
    if esbmc["timed_out"] or native["timed_out"]:
        return "timeout"
    if cls != verify_diff.MATCH:
        return cls
    return None


def build_row(fx, samples):
    esbmc = summarise_backend(samples["esbmc"])
    native = summarise_backend(samples["native"])
    cls, detail = verify_diff.classify(
        fx.truth,
        esbmc["verdict"],
        native["verdict"],
        verify_diff.cex_keys(esbmc["counterexamples"]),
        verify_diff.cex_keys(native["counterexamples"]),
        native["verify_status"],
    )
    gating = fx.group not in NON_GATING_GROUPS
    excluded = excluded_reason(gating, cls, esbmc, native)
    row = {
        "fixture": f"{fx.group}/{fx.name}",
        "group": fx.group,
        "truth": fx.truth,
        "class": cls,
        "detail": detail,
        "gating": gating,
        "comparable": excluded is None,
        "excluded": excluded,
        "ratio_wall": None,
        "ratio_rss": None,
        "esbmc": esbmc,
        "native": native,
    }
    if excluded is None:
        row["ratio_wall"] = ratio(native["wall_s"], esbmc["wall_s"])
        row["ratio_rss"] = ratio(native["peak_rss_kb"], esbmc["peak_rss_kb"])
    return row


def measure_fixture(vowc, fx, runs, jobs, timeout):
    budget = DEFAULT_BUDGET_S if timeout is None else timeout
    watchdog_s = budget * fixture_function_count(fx) + verify_diff.WATCHDOG_SLACK

    def once(backend):
        return run_once(vowc, backend, fx.path, jobs, timeout, watchdog_s)

    for backend in BACKENDS:
        once(backend)
    samples = {backend: [] for backend in BACKENDS}
    for _ in range(runs):
        for backend in BACKENDS:
            samples[backend].append(once(backend))
    return build_row(fx, samples)


def parse_version(text):
    match = VERSION_RE.search(text or "")
    return match.group(0) if match else None


def probe_version(tool):
    try:
        proc = subprocess.run(
            [tool, "--version"], capture_output=True, text=True, timeout=30
        )
    except (OSError, subprocess.SubprocessError):
        return None
    return parse_version(proc.stdout + proc.stderr)


def preflight(vowc):
    problems = []
    if not os.path.isfile(vowc) or not os.access(vowc, os.X_OK):
        problems.append(f"vowc not found or not executable: {vowc}")
    for tool in ("esbmc", "bitwuzla"):
        if shutil.which(tool) is None:
            problems.append(f"`{tool}` is not on PATH")
    if shutil.which("esbmc") is not None:
        version = probe_version("esbmc")
        if version is None or ".".join(version.split(".")[:2]) != ESBMC_PIN:
            problems.append(
                f"esbmc {version or 'of unknown version'} found, "
                f"the gate is pinned to {ESBMC_PIN}"
            )
    return problems


def require_linux():
    return sys.platform.startswith("linux")


def build_report(args, rows, corpus):
    groups = Counter(fx.group for fx in corpus)
    return {
        "schema_version": SCHEMA_VERSION,
        "vowc": args.vowc,
        "esbmc_version": probe_version("esbmc"),
        "bitwuzla_version": probe_version("bitwuzla"),
        "host": {
            "platform": platform.platform(),
            "cpus": os.cpu_count(),
            "python": platform.python_version(),
        },
        "runs": args.runs,
        "budget": "default" if args.timeout is None else "explicit",
        "timeout_s": args.timeout,
        "verify_jobs": args.verify_jobs,
        "corpus": {
            "fixtures": len(rows),
            "groups": dict(groups),
            "include_stress": args.include_stress,
            "filter": args.filter,
        },
        "rows": rows,
        "gate": evaluate_gate(rows, args.runs, args.filter is not None),
    }


def exit_code(report):
    return 0 if report["gate"]["passed"] else 1


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
        verdict += " (provisional: fewer runs than the gate requires)"
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
    for r in report["rows"]:
        if not r["comparable"]:
            print(
                f"  [excluded: {r['excluded']}] {r['fixture']}"
                + (f": {r['detail']}" if r["detail"] else ""),
                file=stream,
            )


def parse_args(argv):
    ap = argparse.ArgumentParser(
        description="Performance harness: native verifier vs ESBMC (#1420)"
    )
    ap.add_argument(
        "--vowc",
        default=os.path.join(REPO_ROOT, "build", "vowc"),
        help="self-hosted vowc binary (default: build/vowc)",
    )
    ap.add_argument(
        "--filter",
        default=None,
        help="only fixtures whose `group/name` contains this (provisional result)",
    )
    ap.add_argument(
        "--runs",
        type=int,
        default=RUNS,
        help=f"timed runs per backend and fixture (default {RUNS}; fewer is provisional)",
    )
    ap.add_argument(
        "--timeout",
        type=int,
        default=None,
        help="explicit per-function budget in seconds for both backends "
        "(default: each backend's shipped default)",
    )
    ap.add_argument(
        "--verify-jobs",
        type=int,
        default=1,
        help="--verify-jobs passed to both backends (default 1)",
    )
    ap.add_argument(
        "--include-stress",
        action="store_true",
        help="also measure tests/verify-stress (non-gating, times out by design)",
    )
    ap.add_argument(
        "--output", default=None, help="write the JSON report here instead of stdout"
    )
    args = ap.parse_args(argv)
    args.vowc = os.path.abspath(args.vowc)
    if args.runs < 1:
        ap.error("--runs must be >= 1")
    if args.verify_jobs < 1:
        ap.error("--verify-jobs must be >= 1")
    if args.timeout is not None and args.timeout < 1:
        ap.error("--timeout must be >= 1")
    return args


def select(corpus, name_filter):
    if name_filter is None:
        return corpus
    return [fx for fx in corpus if name_filter in f"{fx.group}/{fx.name}"]


def main(argv=None):
    args = parse_args(argv)
    if not require_linux():
        print("verify_perf: peak RSS sampling needs Linux /proc", file=sys.stderr)
        return 2
    problems = preflight(args.vowc)
    if problems:
        for p in problems:
            print(f"verify_perf: {p}", file=sys.stderr)
        return 2

    try:
        corpus = select(collect_corpus(args.include_stress), args.filter)
        if not corpus:
            print("verify_perf: no fixtures selected", file=sys.stderr)
            return 2
        rows = [
            measure_fixture(args.vowc, fx, args.runs, args.verify_jobs, args.timeout)
            for fx in corpus
        ]
    except (OSError, ValueError) as err:
        print(f"verify_perf: harness failure: {err}", file=sys.stderr)
        return 2

    report = build_report(args, rows, corpus)
    text = json.dumps(report, indent=2) + "\n"
    if args.output:
        with open(args.output, "w", encoding="utf-8") as fh:
            fh.write(text)
    else:
        sys.stdout.write(text)
    print_summary(report, sys.stderr)
    return exit_code(report)


if __name__ == "__main__":
    sys.exit(main())
