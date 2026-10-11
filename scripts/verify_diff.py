#!/usr/bin/env python3
"""Differential harness: native verifier vs ESBMC (epic #1398, issue #1414).

Runs every program of the verifier-evaluation corpus (tests/verify,
tests/verify-fail, tests/verify-skip, with the ground truth that
scripts/verify_eval.py reads from `// TEST:` directives) under both
`vowc verify --backend esbmc` and `vowc verify --backend native`, and
classifies each fixture:

  match         same verdict (and, when both refute, the same counterexamples)
  more_precise  native reaches a verdict ESBMC does not, or reports a
                counterexample ESBMC missed, and the ground truth agrees
  weaker        native is less conclusive than ESBMC (Skipped / unknown /
                timeout / tool_not_found where ESBMC proves or refutes), drops
                or re-attributes a counterexample, or refutes a program the
                corpus labels correct
  soundness     native proves a program that ESBMC refutes or the corpus labels
                incorrect

Output is a single JSON document (stdout, or --output FILE); a short human
summary of every non-match row goes to stderr.

`--command contracts` and `--command test` (issue #1427) compare the same
corpus under `vowc contracts --verify` and `vowc test --verify`: per clause
(status and `trivially_satisfiable`) for `contracts`, and the per-file test
status for `test`. The same four classes apply, a fixture takes the most severe
class of its clauses.

Exit code: 0 when no row is `weaker` or `soundness`; 1 otherwise; 2 when the
harness itself cannot run (missing binary or solver, unparseable verifier
output).

Usage:
    scripts/verify_diff.py [--vowc build/vowc] [--filter NAME] [--output FILE]
                           [--command verify|contracts|test]
"""

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor

import candidate_isolation
import verify_eval

REPO_ROOT = verify_eval.REPO_ROOT

SCHEMA_VERSION = 1

PROVEN = "proven"
REFUTED = "refuted"
SKIPPED = "skipped"
INCONCLUSIVE = "inconclusive"
ERROR = "error"

MATCH = "match"
MORE_PRECISE = "more_precise"
WEAKER = "weaker"
SOUNDNESS = "soundness"
HARNESS = "harness"

CLASSES = (MATCH, MORE_PRECISE, WEAKER, SOUNDNESS, HARNESS)
FAILING_CLASSES = (WEAKER, SOUNDNESS)

CONCLUSIVE = (PROVEN, REFUTED)

BACKENDS = ("esbmc", "native")

COMMANDS = ("verify", "contracts", "test")

CLAUSE_VERDICT = {
    "proven": PROVEN,
    "proven-ir": PROVEN,
    "failed": REFUTED,
    "unknown": INCONCLUSIVE,
    "timeout": INCONCLUSIVE,
    "error": INCONCLUSIVE,
    "skipped": SKIPPED,
}

SEVERITY = {MATCH: 0, MORE_PRECISE: 1, HARNESS: 2, WEAKER: 3, SOUNDNESS: 4}

SOFT_FAILURES = ("panicked", "error", "crashed")

WATCHDOG_SLACK = 30

FN_DECL = re.compile(r"^\s*(?:pub\s+)?fn\s", re.MULTILINE)
MAIN_DECL = re.compile(r"^\s*fn\s+main\s*\(", re.MULTILINE)


def verdict_of(result):
    """Normalize one `vowc verify` JSON result to a verdict."""
    if result is None:
        return ERROR
    status = result.get("status")
    if status == "Verified":
        return PROVEN
    if status == "Skipped":
        return SKIPPED
    if status == "VerifyFailed":
        if result.get("counterexamples"):
            return REFUTED
        return INCONCLUSIVE
    return ERROR


def cex_keys(cexes):
    """Counterexamples as a set of comparable (fn, blame, vow_id)."""
    return frozenset((c["fn"], c["blame"], c["vow_id"]) for c in cexes)


def compare_cex(esbmc_keys, native_keys):
    """Return (class, detail) for two refutations of the same fixture."""
    missing = esbmc_keys - native_keys
    extra = native_keys - esbmc_keys
    if missing:
        return (
            WEAKER,
            f"native lacks ESBMC counterexample(s) {sorted(missing, key=repr)}",
        )
    if extra:
        return (
            MORE_PRECISE,
            f"native reports extra counterexample(s) {sorted(extra, key=repr)}",
        )
    return MATCH, None


def classify(
    truth,
    esbmc,
    native,
    esbmc_keys=frozenset(),
    native_keys=frozenset(),
    native_status=None,
):
    """Classify one fixture from the two verdicts and the corpus ground truth.

    `truth` is the corpus-expected status (Verified / VerifyFailed / Skipped);
    a known soundness gap counts as VerifyFailed.
    """
    if ERROR in (esbmc, native):
        who = [n for n, v in (("esbmc", esbmc), ("native", native)) if v == ERROR]
        return HARNESS, f"no usable verify result from {', '.join(who)}"

    if native == PROVEN and esbmc != PROVEN and truth == "VerifyFailed":
        return (
            SOUNDNESS,
            f"native proves a program the corpus labels incorrect (ESBMC: {esbmc})",
        )
    if native == PROVEN and esbmc == REFUTED:
        return SOUNDNESS, "native proves a program ESBMC refutes"

    if native == INCONCLUSIVE and native_status in SOFT_FAILURES:
        return (
            WEAKER,
            f"native failed with verify_status `{native_status}` (ESBMC: {esbmc})",
        )

    if esbmc == native:
        if native == REFUTED:
            return compare_cex(esbmc_keys, native_keys)
        return MATCH, None

    if esbmc in CONCLUSIVE and native in CONCLUSIVE:
        if truth == "VerifyFailed":
            return (
                MORE_PRECISE,
                "native refutes a program ESBMC proves; the corpus labels it incorrect",
            )
        return WEAKER, f"native refutes a program ESBMC proves (corpus truth: {truth})"

    if esbmc in CONCLUSIVE:
        return WEAKER, f"native is {native} where ESBMC is {esbmc}"

    if native in CONCLUSIVE:
        if native == REFUTED and truth == "Verified":
            return (
                WEAKER,
                f"native refutes a program the corpus labels correct (ESBMC: {esbmc})",
            )
        return MORE_PRECISE, f"native is {native} where ESBMC is {esbmc}"

    return MATCH, f"neither backend concludes (esbmc: {esbmc}, native: {native})"


def synthetic_failure(verify_status):
    return {
        "status": "VerifyFailed",
        "verify_status": verify_status,
        "counterexamples": [],
    }


def function_count(path):
    """Upper bound on verify targets: `--timeout` is a per-function budget."""
    with open(path, "r", encoding="utf-8") as fh:
        return max(1, len(FN_DECL.findall(fh.read())))


def command_args(vowc, command, backend, timeout, path):
    """The `vowc` command line of `command` under `backend`.

    `test --timeout` is the per-test execution timeout, not the verifier's, so
    `test` takes none: native verification has its own fixed budget there.
    """
    if command == "contracts":
        return [vowc, "contracts", "--verify", "--no-cache", "--backend", backend]
    if command == "test":
        return [vowc, "test", "--verify", "--backend", backend, path]
    return [vowc, "verify", "--no-cache", "--backend", backend]


def has_main(path):
    """`vowc test` runs `main`: a fixture without one has nothing to run."""
    with open(path, "r", encoding="utf-8") as fh:
        return MAIN_DECL.search(fh.read()) is not None


def run_backend(vowc, backend, path, timeout, max_fns, command="verify"):
    """Run one backend under `--timeout`; a hung or crashed process is inconclusive.

    The watchdog scales with the number of functions and adds slack so the
    verifier's own budget fires first; it only reports a verifier that failed
    to honour it. The child runs in its own process group so a kill also
    reaches the solver processes it spawned.
    """
    args = command_args(vowc, command, backend, timeout, path)
    if command != "test":
        args += ["--timeout", str(timeout), path]
    start = time.monotonic()
    proc = subprocess.Popen(
        args,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        cwd=REPO_ROOT,
        env=candidate_isolation.scrubbed_env(),
        start_new_session=True,
    )
    try:
        stdout, stderr = proc.communicate(timeout=timeout * max_fns + WATCHDOG_SLACK)
    except subprocess.TimeoutExpired:
        os.killpg(proc.pid, signal.SIGKILL)
        proc.communicate()
        return synthetic_failure("timeout"), time.monotonic() - start
    elapsed = time.monotonic() - start
    try:
        return json.loads(stdout.strip()), elapsed
    except json.JSONDecodeError:
        pass
    if proc.returncode < 0:
        return synthetic_failure("crashed"), elapsed
    print(
        f"verify_diff: {backend} printed no JSON for {path}: {stderr.rstrip()}",
        file=sys.stderr,
    )
    return None, elapsed


def clause_table(result):
    """(function, kind, vow_id) -> clause row of one `contracts --verify` result."""
    table = {}
    for c in (result or {}).get("contracts") or []:
        table[(c["function"], c["kind"], c["vow_id"])] = c
    return table


def short_circuit_requires(table, function):
    """True when a `requires` of `function` lowers to branches (`&&`, `||`).

    ESBMC plants its vacuity label only in the entry block, so such a function
    reads as `vacuous` there whatever its requires say.
    """
    return any(
        fn == function
        and kind == "requires"
        and ("&&" in c["description"] or "||" in c["description"])
        for (fn, kind, _), c in table.items()
    )


def classify_clause(truth, key, esbmc, native, esbmc_table):
    """Class and detail for one clause present in both `contracts` reports."""
    e, n = esbmc["status"], native["status"]
    if "not_verified" in (e, n):
        return (
            HARNESS,
            f"`{key[0]}` {key[1]} was not verified (esbmc: {e}, native: {n})",
        )
    if "vacuous" in (e, n):
        if e == n:
            return MATCH, None
        if e == "vacuous":
            if short_circuit_requires(esbmc_table, key[0]):
                return (
                    MORE_PRECISE,
                    "ESBMC plants its vacuity label only in the entry block and "
                    "this `requires` lowers to branches",
                )
            return WEAKER, f"native is {n} where ESBMC reports vacuous"
        return WEAKER, f"native reports vacuous where ESBMC is {e}"
    ev, nv = CLAUSE_VERDICT[e], CLAUSE_VERDICT[n]
    if nv == PROVEN and ev in (SKIPPED, INCONCLUSIVE):
        return MORE_PRECISE, f"native proves a clause ESBMC leaves {e}"
    return classify(truth, ev, nv, native_status=n if n == "error" else None)


def compare_contracts(truth, esbmc_result, native_result):
    """(class, detail) of two `contracts --verify` reports of one fixture."""
    if esbmc_result is None or native_result is None:
        return HARNESS, "no usable contracts result"
    etable, ntable = clause_table(esbmc_result), clause_table(native_result)
    if set(etable) != set(ntable):
        return HARNESS, "the two backends list different clauses"
    worst, detail = MATCH, None
    for key in sorted(etable, key=repr):
        e, n = etable[key], ntable[key]
        cls, why = classify_clause(truth, key, e, n, etable)
        if cls == MATCH and e["trivially_satisfiable"] != n["trivially_satisfiable"]:
            cls = WEAKER
            why = (
                f"`{key[0]}` {key[1]} trivially_satisfiable differs "
                f"(esbmc: {e['trivially_satisfiable']}, native: {n['trivially_satisfiable']})"
            )
        if SEVERITY[cls] > SEVERITY[worst]:
            worst, detail = cls, f"`{key[0]}` {key[1]} #{key[2]}: {why}"
    return worst, detail


def test_verdict(result):
    """File-level verdict of one `vowc test --verify` result.

    A test that passed verification counts as proven whatever the program then
    did when it ran: the run is not the verifier's to answer for.
    """
    if (result or {}).get("verify_status") in ("timeout", "crashed"):
        return INCONCLUSIVE
    tests = (result or {}).get("tests") or []
    if not tests:
        return ERROR
    status = tests[0].get("status")
    if status == "verify_failed":
        return REFUTED
    if status == "contract_skipped":
        return SKIPPED
    if status in ("passed", "failed", "timeout"):
        return PROVEN
    return ERROR


def classify_test(truth, esbmc_result, native_result):
    """(class, detail) of two `vowc test --verify` results of one fixture.

    The ESBMC test path fails a file for a reachable checked-arithmetic abort,
    which `verify` only warns about, so a native pass of a program the corpus
    labels correct is not a soundness row.
    """
    esbmc, native = test_verdict(esbmc_result), test_verdict(native_result)
    if native == PROVEN and esbmc == REFUTED and truth == "Verified":
        return (
            MORE_PRECISE,
            "native passes a program the corpus labels correct; the ESBMC test "
            "path fails it",
        )
    status = (native_result or {}).get("verify_status")
    return classify(truth, esbmc, native, native_status=status)


def clause_summary(result):
    """Compact verdict text of a `contracts` report, e.g. `proven:3,failed:1`."""
    counts = {}
    for c in (result or {}).get("contracts") or []:
        label = c["status"] + ("+trivial" if c["trivially_satisfiable"] else "")
        counts[label] = counts.get(label, 0) + 1
    return ",".join(f"{k}:{v}" for k, v in sorted(counts.items())) or "none"


def diff_clauses(vowc, timeout, sub, exp, command, truth, max_fns):
    """The row of one fixture under `contracts` or `test`."""
    row = {
        "fixture": f"{sub}/{exp.name}",
        "truth": truth,
        "known_gap": bool(exp.known_gap),
        "command": command,
    }
    results = {}
    for backend in BACKENDS:
        result, seconds = run_backend(
            vowc, backend, exp.path, timeout, max_fns, command
        )
        results[backend] = result
        verdict = (
            clause_summary(result) if command == "contracts" else test_verdict(result)
        )
        row[backend] = {
            "verdict": verdict,
            "seconds": round(seconds, 3),
            "status": (result or {}).get("status"),
            "verify_status": None,
            "counterexamples": [],
        }
    if command == "contracts":
        row["class"], row["detail"] = compare_contracts(
            truth, results["esbmc"], results["native"]
        )
    else:
        row["class"], row["detail"] = classify_test(
            truth, results["esbmc"], results["native"]
        )
    return row


def diff_fixture(vowc, timeout, sub, exp, command="verify"):
    truth = "VerifyFailed" if exp.known_gap else exp.expected_status
    max_fns = function_count(exp.path)
    if command != "verify":
        return diff_clauses(vowc, timeout, sub, exp, command, truth, max_fns)
    row = {
        "fixture": f"{sub}/{exp.name}",
        "truth": truth,
        "known_gap": bool(exp.known_gap),
    }
    results = {}
    for backend in BACKENDS:
        result, seconds = run_backend(vowc, backend, exp.path, timeout, max_fns)
        result = result or {}
        cexes = verify_eval.actual_cex(result)
        results[backend] = {
            "verdict": verdict_of(result),
            "keys": cex_keys(cexes),
            "verify_status": result.get("verify_status"),
        }
        row[backend] = {
            "verdict": results[backend]["verdict"],
            "seconds": round(seconds, 3),
            "status": result.get("status"),
            "verify_status": result.get("verify_status"),
            "counterexamples": cexes,
        }
    esbmc, native = results["esbmc"], results["native"]
    row["class"], row["detail"] = classify(
        truth,
        esbmc["verdict"],
        native["verdict"],
        esbmc["keys"],
        native["keys"],
        native["verify_status"],
    )
    return row


def build_report(vowc, rows, command="verify"):
    counts = dict.fromkeys(CLASSES, 0)
    for row in rows:
        counts[row["class"]] += 1
    return {
        "schema_version": SCHEMA_VERSION,
        "vowc": vowc,
        "command": command,
        "summary": {"total": len(rows), **counts},
        "rows": rows,
    }


def exit_code(report):
    summary = report["summary"]
    if any(summary[c] for c in FAILING_CLASSES):
        return 1
    if summary[HARNESS]:
        return 2
    return 0


def preflight(vowc):
    problems = []
    if not os.path.isfile(vowc) or not os.access(vowc, os.X_OK):
        problems.append(f"vowc not found or not executable: {vowc}")
    for tool in ("esbmc", "bitwuzla"):
        if shutil.which(tool) is None:
            problems.append(f"`{tool}` is not on PATH")
    return problems


def print_summary(report, stream):
    summary = report["summary"]
    print(
        "verify_diff: "
        + ", ".join(f"{summary[c]} {c}" for c in CLASSES)
        + f" of {summary['total']} fixtures",
        file=stream,
    )
    for row in report["rows"]:
        if row["class"] == MATCH:
            continue
        print(
            f"  [{row['class']}] {row['fixture']}: {row['detail']} "
            f"(esbmc={row['esbmc']['verdict']}, native={row['native']['verdict']})",
            file=stream,
        )


def main(argv=None):
    ap = argparse.ArgumentParser(
        description="Differential harness: native verifier vs ESBMC (#1414)"
    )
    ap.add_argument(
        "--vowc",
        default=os.path.join(REPO_ROOT, "build", "vowc"),
        help="self-hosted vowc binary (default: build/vowc)",
    )
    ap.add_argument(
        "--filter", default=None, help="only fixtures whose name contains this"
    )
    ap.add_argument(
        "--output", default=None, help="write the JSON report here instead of stdout"
    )
    ap.add_argument(
        "--timeout",
        type=int,
        default=60,
        help="per-function verifier budget in seconds for each backend (default 60)",
    )
    ap.add_argument(
        "--command",
        choices=COMMANDS,
        default="verify",
        help="compare `vowc verify` (default), `contracts --verify` or `test --verify`",
    )
    ap.add_argument(
        "--jobs",
        type=int,
        default=1,
        help="fixtures verified concurrently (default 1; contended runs can fake failures)",
    )
    args = ap.parse_args(argv)
    if args.jobs < 1:
        ap.error("--jobs must be >= 1")
    if args.timeout < 1:
        ap.error("--timeout must be >= 1")

    problems = preflight(args.vowc)
    if problems:
        for p in problems:
            print(f"verify_diff: {p}", file=sys.stderr)
        return 2

    try:
        fixtures = [
            (sub, exp)
            for sub, exp in verify_eval.collect(args.filter)
            if not exp.skip_reason and (args.command != "test" or has_main(exp.path))
        ]
        if not fixtures:
            print("verify_diff: no fixtures selected", file=sys.stderr)
            return 2
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            rows = list(
                pool.map(
                    lambda f: diff_fixture(args.vowc, args.timeout, *f, args.command),
                    fixtures,
                )
            )
    except (OSError, ValueError) as err:
        print(f"verify_diff: harness failure: {err}", file=sys.stderr)
        return 2

    report = build_report(args.vowc, rows, args.command)
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
