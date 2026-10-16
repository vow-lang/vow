#!/usr/bin/env python3
"""Whole-compiler native verification (epic #1398, issue #1432).

Runs `vowc verify` over every source of the self-hosted compiler, the real-world
workload for the native verifier, and holds the native backend to three claims:

  1. Verdicts. Per verified function, native is at least as conclusive as ESBMC
     (`scripts/verify_diff.py` classes, ground truth `Verified`: the compiler is
     expected to satisfy its own contracts). A function native skips is allowed
     only when `scripts/native_compiler_skips.toml` lists it with the issue that
     tracks the gap, and an entry for a function that is no longer skipped is
     itself a failure, so the list only shrinks.
  2. Cost. Wall-clock and peak RSS against ESBMC with the gate of
     `scripts/verify_perf.py` (geometric mean <= 1.0, worst root <= 1.25,
     native times out nowhere ESBMC finishes), median of `--runs` runs, cache off.
     The comparison is like for like: it covers the functions both backends
     decide the same way (proven by both, or refuted by both). A function only
     one backend decides (native proves what ESBMC skips, ESBMC proves what
     native skips, either side undecided) is left out of both sides, and the
     report names each with the reason and the time it took. The time of a
     function is the gap between the end of the previous function's progress
     lines and the end of its own on the driver's stderr, and its memory the
     peak of the process tree in that gap, so the attribution needs
     `--verify-jobs 1`; with more jobs a root whose decided sets differ is left
     out of the cost gate. Everything else (front end, start-up, the function
     reported first) stays in both sides. The whole-root numbers are still
     reported next to the like-for-like ones.
  3. Stages. `--native-only` verifies with each of several `vowc` binaries (the
     bootstrap's stage 1 and stage 2) and requires identical outcomes, so a
     native verdict never depends on which stage compiled the verifier.

Roots are `compiler/main.vow` plus every top-level module of `compiler/` that
`main.vow` does not reach through `use` (orphans such as `module_io.vow` and the
`test_*.vow` programs), so every compiler source is under a verify run.
`compiler/tests/` is not a root: those programs only re-load the modules above.

Per-function verdicts come from the driver's progress lines on stderr
(`  <fn>: PROVEN|SKIPPED|UNKNOWN|TIMEOUT|FAILED|ERROR|PANICKED`); the JSON result
carries the overall status, the skipped functions and the counterexamples. Both
are read, and a function the progress lines do not mention is `absent`.

Exit code: 0 every criterion holds, 1 a criterion failed, 2 the harness could
not run (missing binary or solver, malformed skip list, no root).

Usage:
    scripts/verify_compiler.py [--vowc build/vowc] [--runs N] [--timeout S]
                               [--verify-jobs N] [--no-perf-gate] [--output FILE]
    scripts/verify_compiler.py --native-only --vowc build/vowc --vowc build/vowc2
"""

import argparse
import json
import os
import re
import shutil
import statistics
import sys
import tomllib
from collections import Counter

import verify_diff
import verify_eval
import verify_perf

REPO_ROOT = verify_eval.REPO_ROOT
COMPILER_DIR = os.path.join(REPO_ROOT, "compiler")
DEFAULT_SKIPS = os.path.join(REPO_ROOT, "scripts", "native_compiler_skips.toml")

SCHEMA_VERSION = 1

# One whole-compiler run is long; this bounds a run that ignores `--timeout`.
DEFAULT_WATCHDOG_S = 7200

# A root this quick on both backends measures process start-up, not verification.
MIN_GATED_WALL_S = 1.0

NATIVE_SKIP_CODES = (
    "function-has-effects",
    "recursion-unsupported",
    "float-rem-unsupported",
    "ir-non-dominating-read",
    "unmodeled-builtin",
    "unsupported-opcode",
    "non-modelable-callee",
    "reserved-verifier-symbol",
    "inlining-too-large",
)

USE_RE = re.compile(r"^use\s+(\w+)\s*$", re.MULTILINE)
PROGRESS_RE = re.compile(
    r"^  ([\w.:]+): (PROVEN|SKIPPED|UNKNOWN|TIMEOUT|FAILED|ERROR|PANICKED)\b",
    re.MULTILINE,
)
SITE_NOTE = "(checked arithmetic can abort)"
VERIFYING_RE = re.compile(r"^Verifying ([\w.:]+)\.\.\.$")
SKIPPED_RE = re.compile(r"^skipped verification of `([^`]+)`: (.*)$", re.DOTALL)

PROGRESS_VERDICT = {
    "PROVEN": verify_diff.PROVEN,
    "SKIPPED": verify_diff.SKIPPED,
    "UNKNOWN": verify_diff.INCONCLUSIVE,
    "TIMEOUT": verify_diff.INCONCLUSIVE,
    "FAILED": verify_diff.REFUTED,
    "ERROR": verify_diff.INCONCLUSIVE,
    "PANICKED": verify_diff.INCONCLUSIVE,
}
ABSENT = "absent"

CONCLUSIVE = verify_diff.CONCLUSIVE

CAPACITY_REASON = "the counterexample is ESBMC's vec/string capacity assertion"

# Progress words that make native `weaker` through `verify_diff.classify`.
SOFT_PROGRESS = {"ERROR": "error", "PANICKED": "panicked"}


def module_uses(path):
    with open(path, "r", encoding="utf-8") as fh:
        return set(USE_RE.findall(fh.read()))


def discover_roots(compiler_dir=COMPILER_DIR):
    """`main.vow` first, then the top-level modules `main.vow` does not reach."""
    modules = {
        f[: -len(".vow")] for f in os.listdir(compiler_dir) if f.endswith(".vow")
    }
    if "main" not in modules:
        raise ValueError(f"no main.vow under {compiler_dir}")
    seen, stack = set(), ["main"]
    while stack:
        name = stack.pop()
        if name in seen:
            continue
        seen.add(name)
        stack += sorted(
            module_uses(os.path.join(compiler_dir, name + ".vow")) & modules
        )
    orphans = sorted(modules - seen)
    return [os.path.join(compiler_dir, f"{m}.vow") for m in ["main"] + orphans]


def load_skips(path):
    """`{function: {code, issue, reason}}` from the skip list; ValueError if malformed."""
    with open(path, "rb") as fh:
        entries = tomllib.load(fh).get("skip", [])
    skips = {}
    for entry in entries:
        name = entry.get("function")
        unknown = set(entry) - {"function", "code", "issue", "reason"}
        if unknown:
            raise ValueError(f"skip entry {name!r}: unknown field(s) {sorted(unknown)}")
        if not isinstance(name, str) or not name:
            raise ValueError(f"skip entry without a function name: {entry!r}")
        if name in skips:
            raise ValueError(f"skip entry {name!r} is listed twice")
        if entry.get("code") not in NATIVE_SKIP_CODES:
            raise ValueError(
                f"skip entry {name!r}: code {entry.get('code')!r} is not in the closed list"
            )
        if not isinstance(entry.get("issue"), int) or isinstance(
            entry.get("issue"), bool
        ):
            raise ValueError(
                f"skip entry {name!r}: `issue` must be a tracking issue number"
            )
        if not str(entry.get("reason", "")).strip():
            raise ValueError(f"skip entry {name!r}: `reason` is required")
        skips[name] = {k: entry[k] for k in ("code", "issue", "reason")}
    return skips


def progress_verdicts(stderr, blocks=False):
    """`{function: progress word}` from the driver's stderr; the last line per function wins.

    A function proven with reachable checked-arithmetic aborts prints no
    `<fn>: PROVEN` line: each site prints `<owner>: PROVEN (checked arithmetic
    can abort)` under the name of the function that owns the site, which is the
    verified function or a callee. With `blocks` (the lines are in the order of a
    sequential run) such a line is read as the verdict of the `Verifying <fn>...`
    block it sits in, and a site line of a callee is not a verdict on the callee.
    """
    if not blocks:
        return {m.group(1): m.group(2) for m in PROGRESS_RE.finditer(stderr or "")}
    verdicts, current = {}, None
    for line in (stderr or "").splitlines():
        opened = VERIFYING_RE.match(line)
        if opened:
            current = opened.group(1)
            continue
        m = PROGRESS_RE.match(line)
        if not m:
            continue
        if SITE_NOTE in line:
            if current is not None:
                verdicts[current] = "PROVEN"
        else:
            verdicts[m.group(1)] = m.group(2)
    return verdicts


def skipped_functions(result):
    """`{function: reason text}` of the `VerificationSkipped` diagnostics naming a function."""
    out = {}
    for d in (result or {}).get("diagnostics") or []:
        if d.get("error_code") != "VerificationSkipped":
            continue
        m = SKIPPED_RE.match(d.get("message", ""))
        if m:
            out[m.group(1)] = m.group(2)
    return out


def skip_code(reason):
    """The closed-list code a native skip reason starts with, or None (an ESBMC free text)."""
    head = reason.split(":", 1)[0].strip()
    return head if head in NATIVE_SKIP_CODES else None


def capacity_functions(result):
    """Functions whose every counterexample is ESBMC's own model-capacity assertion."""
    by_function = {}
    for c in (result or {}).get("counterexamples") or []:
        by_function.setdefault(c.get("function"), []).append(c)
    return sorted(
        name
        for name, cexes in by_function.items()
        if verify_diff.capacity_only({"counterexamples": cexes})
    )


def attribute(events, rss_series, wall_s):
    """Time and memory per verified function of one run.

    `events` are `(seconds, stderr line)`, `rss_series` `(seconds, kB)`. Each
    `Verifying <fn>...` line opens a block that runs to the next one; the block
    ends at its last progress line (the native driver prints both together when
    a worker's answer is ready, ESBMC's driver prints the first at the start and
    the others at the end). A function's time is the gap from the end of the
    previous block to the end of its own, its memory the largest sample in that
    gap. The first block has no gap of its own: what precedes it, and the
    function reported first, are common start-up. `prelude_peak_kb` covers the
    time up to the first block's end and `tail_peak_kb` what follows the last.
    """
    blocks = []
    for t, line in events or ():
        opened = VERIFYING_RE.match(line.rstrip("\n"))
        if opened:
            blocks.append([opened.group(1), t])
        elif blocks and PROGRESS_RE.match(line):
            blocks[-1][1] = max(blocks[-1][1], t)
    gaps, peaks = {}, {}

    def peak_in(lo, hi):
        return max((kb for t, kb in rss_series or () if lo < t <= hi), default=0)

    previous = None
    for name, end in blocks:
        gaps[name] = gaps.get(name, 0.0) + (
            0.0 if previous is None else max(0.0, end - previous)
        )
        if previous is not None:
            peaks[name] = max(peaks.get(name, 0), peak_in(previous, end))
        previous = max(end, previous) if previous is not None else end
    return {
        "wall_s": wall_s,
        "gap_s": gaps,
        "peak_kb": peaks,
        "prelude_peak_kb": peak_in(-1.0, blocks[0][1] if blocks else float("inf")),
        "tail_peak_kb": peak_in(previous, float("inf")) if blocks else 0,
    }


def summarise(sample_set, blocks=False):
    """One backend's summary over its timed samples (median cost, modal outcome).

    `blocks` says the runs were sequential (`--verify-jobs 1`), so a progress
    line belongs to the `Verifying` line before it."""
    summary = verify_perf.summarise_backend(sample_set)
    if all("events" in s for s in sample_set):
        summary["attribution"] = [
            attribute(s["events"], s.get("rss_series"), s["wall_s"]) for s in sample_set
        ]
    modal = sample_set[
        [verify_diff.verdict_of(s["result"]) for s in sample_set].index(
            summary["verdict"]
        )
    ]
    result = modal["result"] or {}
    summary["functions"] = progress_verdicts(modal["stderr"], blocks)
    summary["skipped"] = skipped_functions(result)
    summary["capacity_functions"] = capacity_functions(result)
    summary["diagnostic_counts"] = dict(
        Counter(d.get("error_code") for d in result.get("diagnostics") or [])
    )
    return summary


def function_verdict(progress_word):
    if progress_word is None:
        return ABSENT
    return PROGRESS_VERDICT[progress_word]


def function_rows(esbmc, native, skips):
    """Per-function comparison rows; `explained` marks a gap the skip list tracks."""
    rows = []
    for name in sorted(set(esbmc["functions"]) | set(native["functions"])):
        e_word, n_word = esbmc["functions"].get(name), native["functions"].get(name)
        e, n = function_verdict(e_word), function_verdict(n_word)
        cls, detail = verify_diff.classify(
            "Verified",
            verify_diff.INCONCLUSIVE if e == ABSENT else e,
            verify_diff.INCONCLUSIVE if n == ABSENT else n,
            native_status=SOFT_PROGRESS.get(n_word),
            capacity_reason=CAPACITY_REASON,
            esbmc_capacity_only=name in esbmc.get("capacity_functions", ()),
        )
        explained = skips.get(name) if n == verify_diff.SKIPPED else None
        rows.append(
            {
                "function": name,
                "esbmc": e,
                "native": n,
                "class": cls,
                "detail": detail,
                "explained": explained,
            }
        )
    return rows


def unexplained(rows):
    return [
        r
        for r in rows
        if r["class"] in verify_diff.FAILING_CLASSES and not r["explained"]
    ]


def native_problems(native, skips):
    """Why one native run is not acceptable on its own, independent of ESBMC."""
    problems = []
    status = native["status"]
    if status not in ("Verified", "Skipped"):
        problems.append(f"overall status is {status}")
    if native["counterexamples"]:
        problems.append(f"{len(native['counterexamples'])} counterexample(s)")
    if native["verify_status"]:
        problems.append(f"verify_status {native['verify_status']}")
    for name, reason in sorted(native["skipped"].items()):
        entry = skips.get(name)
        if entry is None:
            problems.append(f"`{name}` is skipped ({reason}) and not in the skip list")
        elif skip_code(reason) != entry["code"]:
            problems.append(
                f"`{name}` is skipped as `{skip_code(reason)}`, the skip list says `{entry['code']}`"
            )
    if native["timed_out"]:
        problems.append("the run timed out")
    return problems


def stale_skips(runs_native, skips):
    """Skip-list entries no root skips any more: the gap closed, the entry must go."""
    still = set()
    for native in runs_native:
        still |= set(native["skipped"])
    return sorted(set(skips) - still)


def measure(vowc, backend, path, jobs, timeout, watchdog_s, runs):
    def once():
        return verify_perf.run_once(
            vowc, backend, path, jobs, timeout, watchdog_s, timeline=True
        )

    if runs > 1:
        once()
    return summarise([once() for _ in range(runs)], blocks=jobs == 1)


def left_out_reason(e, n, native_skip):
    """Why a function is not in the like-for-like cost comparison."""
    if e == ABSENT and n == ABSENT:
        return (
            "no verdict line of its own (the sites of its checked arithmetic are "
            "reported under their owner)"
        )
    if n == verify_diff.SKIPPED and e in CONCLUSIVE:
        return f"native skipped it ({native_skip or 'no reason'}), esbmc {e}"
    if e == verify_diff.SKIPPED and n in CONCLUSIVE:
        return f"esbmc skipped it, native {n}"
    if e in CONCLUSIVE and n in CONCLUSIVE:
        return f"backends disagree: esbmc {e}, native {n}"
    if e in CONCLUSIVE or n in CONCLUSIVE:
        return f"one side undecided: esbmc {e}, native {n}"
    return f"neither backend decides it: esbmc {e}, native {n}"


def cost_functions(rows, esbmc, native):
    """`(compared, left_out)`: the functions both backends decide the same way,
    and every other function the driver verified or reported, with the reason and
    the median time each backend spent on it."""
    verdicts = {r["function"]: (r["esbmc"], r["native"]) for r in rows}
    timed = {
        name
        for summary in (esbmc, native)
        for a in summary.get("attribution", ())
        for name in a["gap_s"]
    }
    compared, left_out = [], []
    for name in sorted(set(verdicts) | timed):
        e, n = verdicts.get(name, (ABSENT, ABSENT))
        if e == n and e in CONCLUSIVE:
            compared.append(name)
            continue
        if e == ABSENT and n == ABSENT and name not in timed:
            continue
        reason = native["skipped"].get(name, "")
        entry = {
            "function": name,
            "esbmc": e,
            "native": n,
            "reason": left_out_reason(e, n, skip_code(reason) or reason.split(":")[0]),
        }
        for side, summary in (("esbmc_s", esbmc), ("native_s", native)):
            gaps = [
                a["gap_s"][name]
                for a in summary.get("attribution", ())
                if name in a["gap_s"]
            ]
            entry[side] = statistics.median(gaps) if gaps else None
        left_out.append(entry)
    return compared, left_out


def like_for_like(summary, compared):
    """`(wall_s, peak_rss_kb)` medians of one backend over the compared
    functions: the run less the gap of every function not in `compared`, and the
    tree peak outside those gaps (the whole-run peak when no sample fell there).
    None when the run carries no attribution."""
    samples = summary.get("attribution")
    if not samples:
        return None
    keep = set(compared)
    walls, peaks = [], []
    for a in samples:
        walls.append(
            a["wall_s"] - sum(g for fn, g in a["gap_s"].items() if fn not in keep)
        )
        peaks.append(
            max(
                [a["prelude_peak_kb"], a["tail_peak_kb"]]
                + [kb for fn, kb in a["peak_kb"].items() if fn in keep]
            )
        )
    return statistics.median(walls), statistics.median(peaks) or summary["peak_rss_kb"]


def cost_basis(rows, esbmc, native, jobs):
    """What the cost ratios of one root compare, and the numbers they use.

    `whole-root` when no function left out was verified by either backend (the
    whole run is then the like-for-like figure; a function both skip costs
    nothing); `like-for-like` when some was and the run could say what it cost;
    `unattributable` when it cannot (`--verify-jobs` above 1, or no timeline)."""
    compared, left_out = cost_functions(rows, esbmc, native)
    basis = {
        "mode": "whole-root",
        "compared": compared,
        "left_out": left_out,
        "whole_root": {
            "esbmc_wall_s": esbmc["wall_s"],
            "native_wall_s": native["wall_s"],
            "esbmc_peak_rss_kb": esbmc["peak_rss_kb"],
            "native_peak_rss_kb": native["peak_rss_kb"],
        },
        "esbmc_wall_s": esbmc["wall_s"],
        "native_wall_s": native["wall_s"],
        "esbmc_peak_rss_kb": esbmc["peak_rss_kb"],
        "native_peak_rss_kb": native["peak_rss_kb"],
    }
    costly = [
        x
        for x in left_out
        if x["esbmc"] not in (verify_diff.SKIPPED, ABSENT)
        or x["native"] not in (verify_diff.SKIPPED, ABSENT)
        or x["esbmc_s"] is not None
        or x["native_s"] is not None
    ]
    if not costly:
        return basis
    e_cost = like_for_like(esbmc, compared) if jobs == 1 else None
    n_cost = like_for_like(native, compared) if jobs == 1 else None
    if e_cost is None or n_cost is None:
        basis["mode"] = "unattributable"
        return basis
    basis["mode"] = "like-for-like"
    basis["esbmc_wall_s"], basis["esbmc_peak_rss_kb"] = e_cost
    basis["native_wall_s"], basis["native_peak_rss_kb"] = n_cost
    return basis


def timed_row(root, esbmc, native, skips, jobs=1):
    """The report row of one root: both backends' summaries, ratios and verdict rows."""
    rows = function_rows(esbmc, native, skips)
    basis = cost_basis(rows, esbmc, native, jobs)
    timed_out = esbmc["timed_out"] or native["timed_out"]
    excluded = None
    if timed_out:
        excluded = "timeout"
    elif basis["mode"] == "unattributable":
        excluded = "unattributable"
    elif not basis["compared"] and basis["left_out"]:
        excluded = "no-common-functions"
    elif max(basis["esbmc_wall_s"], basis["native_wall_s"]) < MIN_GATED_WALL_S:
        excluded = "below-floor"
    gated = excluded is None
    return {
        "fixture": os.path.relpath(root, REPO_ROOT),
        "gating": True,
        "comparable": gated,
        "excluded": excluded,
        "ratio_wall": verify_perf.ratio(basis["native_wall_s"], basis["esbmc_wall_s"])
        if gated
        else None,
        "ratio_rss": verify_perf.ratio(
            basis["native_peak_rss_kb"], basis["esbmc_peak_rss_kb"]
        )
        if gated
        else None,
        "cost_basis": basis,
        "esbmc": esbmc,
        "native": native,
        "functions": rows,
        "native_problems": native_problems(native, skips),
    }


def compare_criteria(rows, skips, perf_gate, runs, pinned=True, all_roots=True):
    """Gate over the comparison rows."""
    gate = verify_perf.evaluate_gate(rows, runs)
    verdict_failures = [
        f"{row['fixture']}: {r['function']}: {r['class']} ({r['detail']})"
        for row in rows
        for r in unexplained(row["functions"])
    ]
    problems = [
        f"{row['fixture']}: {p}" for row in rows for p in row["native_problems"]
    ]
    stale = stale_skips([row["native"] for row in rows], skips) if all_roots else []
    criteria = {
        "verdict_parity": not verdict_failures,
        "native_acceptable": not problems,
        "skip_list_fresh": not stale,
    }
    perf = {k: v for k, v in gate["criteria"].items() if k != "enough_comparable"}
    perf_ok = all(perf.values())
    if perf_gate:
        criteria.update(perf)
    return {
        "passed": all(criteria.values()),
        "perf_gated": perf_gate,
        "perf_passed": perf_ok,
        "provisional": gate["provisional"] or not pinned,
        "criteria": criteria,
        "verdict_failures": verdict_failures,
        "native_problems": problems,
        "stale_skips": stale,
        "metrics": gate["metrics"],
        "timeouts": gate["timeouts"],
        "comparable": gate["comparable"],
        "excluded": gate["excluded"],
    }


def fingerprint(native):
    """What two stages of the verifier must agree on, bit for bit."""
    return {
        "status": native["status"],
        "verify_status": native["verify_status"],
        "functions": native["functions"],
        "skipped": native["skipped"],
        "counterexamples": sorted(
            (c["fn"], c["blame"], c["vow_id"]) for c in native["counterexamples"]
        ),
    }


def stage_criteria(by_stage, skips, all_roots=True):
    """Gate over native-only runs: `by_stage` is `[(vowc, [native summary per root])]`."""
    problems, disagreements = [], []
    reference = by_stage[0]
    for vowc, natives in by_stage:
        for native in natives:
            problems += [f"{vowc}: {p}" for p in native_problems(native, skips)]
    for vowc, natives in by_stage[1:]:
        for index, native in enumerate(natives):
            if fingerprint(native) != fingerprint(reference[1][index]):
                disagreements.append(
                    f"{vowc} differs from {reference[0]} on root #{index}"
                )
    stale = (
        stale_skips([n for _, natives in by_stage for n in natives], skips)
        if all_roots
        else []
    )
    criteria = {
        "native_acceptable": not problems,
        "stages_agree": not disagreements,
        "skip_list_fresh": not stale,
    }
    return {
        "passed": all(criteria.values()),
        "criteria": criteria,
        "native_problems": problems,
        "stage_disagreements": disagreements,
        "stale_skips": stale,
    }


def esbmc_pinned():
    version = verify_perf.probe_version("esbmc")
    return (
        version is not None
        and ".".join(version.split(".")[:2]) == verify_perf.ESBMC_PIN
    )


def preflight(vowcs, need_esbmc, allow_esbmc_version=False):
    problems = []
    for vowc in vowcs:
        if not os.path.isfile(vowc) or not os.access(vowc, os.X_OK):
            problems.append(f"vowc not found or not executable: {vowc}")
    tools = ["bitwuzla"] + (["esbmc"] if need_esbmc else [])
    problems += [f"`{t}` is not on PATH" for t in tools if shutil.which(t) is None]
    if need_esbmc and shutil.which("esbmc") is not None and not allow_esbmc_version:
        if not esbmc_pinned():
            version = verify_perf.probe_version("esbmc")
            problems.append(
                f"esbmc {version or 'of unknown version'} found, "
                f"the gate is pinned to {verify_perf.ESBMC_PIN}"
            )
    return problems


def run_compare(args, roots, skips):
    rows = []
    for root in roots:
        esbmc = measure(
            args.vowc[0],
            "esbmc",
            root,
            args.verify_jobs,
            args.timeout,
            args.watchdog,
            args.runs,
        )
        native = measure(
            args.vowc[0],
            "native",
            root,
            args.verify_jobs,
            args.timeout,
            args.watchdog,
            args.runs,
        )
        rows.append(timed_row(root, esbmc, native, skips, args.verify_jobs))
    report = {
        "mode": "compare",
        "rows": rows,
        "gate": compare_criteria(
            rows,
            skips,
            not args.no_perf_gate,
            args.runs,
            esbmc_pinned(),
            not args.root,
        ),
    }
    return report


def run_native_only(args, roots, skips):
    by_stage = []
    for vowc in args.vowc:
        natives = [
            measure(
                vowc, "native", root, args.verify_jobs, args.timeout, args.watchdog, 1
            )
            for root in roots
        ]
        by_stage.append((vowc, natives))
    return {
        "mode": "native-only",
        "stages": [
            {
                "vowc": vowc,
                "roots": [
                    fingerprint(n)
                    | {"wall_s": n["wall_s"], "peak_rss_kb": n["peak_rss_kb"]}
                    for n in natives
                ],
            }
            for vowc, natives in by_stage
        ],
        "gate": stage_criteria(by_stage, skips, not args.root),
    }


def print_summary(report, stream):
    gate = report["gate"]
    print(
        f"verify_compiler: {report['mode']} {'PASS' if gate['passed'] else 'FAIL'}",
        file=stream,
    )
    for name, ok in gate["criteria"].items():
        print(f"  {'ok  ' if ok else 'FAIL'} {name}", file=stream)
    for key in ("verdict_failures", "native_problems", "stage_disagreements"):
        for line in gate.get(key, []):
            print(f"  {key}: {line}", file=stream)
    for name in gate["stale_skips"]:
        print(f"  stale skip-list entry (no longer skipped): {name}", file=stream)
    if report["mode"] == "compare":
        for metric in ("wall", "rss"):
            print(
                "  " + verify_perf.format_metric(metric, gate["metrics"][metric]),
                file=stream,
            )
        for row in report["rows"]:
            basis = row["cost_basis"]
            if basis["mode"] == "whole-root":
                continue
            print(
                f"  {row['fixture']}: cost {basis['mode']}, "
                f"{len(basis['compared'])} compared, {len(basis['left_out'])} left out"
                + (f" ({row['excluded']})" if row["excluded"] else ""),
                file=stream,
            )
            for entry in basis["left_out"]:
                print(
                    f"    left out {entry['function']}: {entry['reason']}", file=stream
                )
        if not gate["perf_gated"]:
            print(
                f"  cost criteria informational (perf gate off, passed={gate['perf_passed']})",
                file=stream,
            )


def parse_args(argv):
    ap = argparse.ArgumentParser(
        description="Whole-compiler native verification (#1432)"
    )
    ap.add_argument(
        "--vowc",
        action="append",
        default=None,
        help="self-hosted vowc binary (default build/vowc); repeat with --native-only",
    )
    ap.add_argument(
        "--native-only",
        action="store_true",
        help="skip ESBMC; compare the given vowc binaries",
    )
    ap.add_argument(
        "--runs",
        type=int,
        default=verify_perf.RUNS,
        help="timed runs per backend and root",
    )
    ap.add_argument(
        "--timeout",
        type=int,
        default=None,
        help="per-function budget in seconds, both backends",
    )
    ap.add_argument("--verify-jobs", type=int, default=1)
    ap.add_argument(
        "--watchdog",
        type=int,
        default=DEFAULT_WATCHDOG_S,
        help="seconds before one run is killed",
    )
    ap.add_argument("--skips", default=DEFAULT_SKIPS, help="native skip list (TOML)")
    ap.add_argument(
        "--root", action="append", default=None, help="verify only these roots"
    )
    ap.add_argument(
        "--allow-esbmc-version",
        action="store_true",
        help=f"accept an ESBMC other than the pinned {verify_perf.ESBMC_PIN} (the report is provisional)",
    )
    ap.add_argument(
        "--no-perf-gate", action="store_true", help="report cost but do not gate on it"
    )
    ap.add_argument(
        "--output", default=None, help="write the JSON report here instead of stdout"
    )
    args = ap.parse_args(argv)
    args.vowc = [
        os.path.abspath(v)
        for v in (args.vowc or [os.path.join(REPO_ROOT, "build", "vowc")])
    ]
    if len(args.vowc) > 1 and not args.native_only:
        ap.error("several --vowc need --native-only")
    if args.runs < 1 or args.verify_jobs < 1 or args.watchdog < 1:
        ap.error("--runs, --verify-jobs and --watchdog must be >= 1")
    return args


def main(argv=None):
    args = parse_args(argv)
    if not verify_perf.require_linux():
        print("verify_compiler: peak RSS sampling needs Linux /proc", file=sys.stderr)
        return 2
    problems = preflight(args.vowc, not args.native_only, args.allow_esbmc_version)
    try:
        skips = load_skips(args.skips)
        roots = (
            [os.path.abspath(r) for r in args.root] if args.root else discover_roots()
        )
    except (OSError, ValueError, tomllib.TOMLDecodeError) as err:
        problems.append(str(err))
    if problems:
        for p in problems:
            print(f"verify_compiler: {p}", file=sys.stderr)
        return 2
    run = run_native_only if args.native_only else run_compare
    try:
        report = run(args, roots, skips)
    except (OSError, ValueError) as err:
        print(f"verify_compiler: harness failure: {err}", file=sys.stderr)
        return 2
    report = {
        "schema_version": SCHEMA_VERSION,
        "vowc": args.vowc,
        "esbmc_version": None
        if args.native_only
        else verify_perf.probe_version("esbmc"),
        "bitwuzla_version": verify_perf.probe_version("bitwuzla"),
        "runs": 1 if args.native_only else args.runs,
        "verify_jobs": args.verify_jobs,
        "roots": [os.path.relpath(r, REPO_ROOT) for r in roots],
        **report,
    }
    text = json.dumps(report, indent=2, default=str) + "\n"
    if args.output:
        with open(args.output, "w", encoding="utf-8") as fh:
            fh.write(text)
    else:
        sys.stdout.write(text)
    print_summary(report, sys.stderr)
    return 0 if report["gate"]["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
