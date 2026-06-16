#!/usr/bin/env python3
"""Verifier-evaluation harness (issue #334).

Ground-truth acceptance suite for the Vow *verifier*, distinct from the
synthesis suite under benchmarks/. Where benchmarks/ asks "can an agent
synthesise a verifying program from a spec?", this harness asks the
orthogonal question: "is the verifier itself accepting correct programs,
rejecting incorrect ones, and attributing blame correctly?"

It runs `vow verify` over the labelled corpus in tests/verify*, asserts each
program's expected outcome plus the exact counterexample {blame, vow_id} set,
and runs a `vow contracts --verify` vacuity guard over the should-pass set.
Mismatches are classified and surfaced under separate, loud banners so a
silent soundness regression can never hide in an aggregate pass/fail count:

  * SOUNDNESS  (false-accept) — an expected-fail program was Verified, or a
                 should-pass program was proven only vacuously. Hard failure.
  * PRECISION  (false-reject) — a should-pass program was rejected.
  * BLAME      — wrong Caller/Callee attribution, or wrong violated vow_id.
  * STATUS     — any other expected/actual status mismatch (e.g. Skipped).

Ground truth lives next to each program as `// TEST:` directives (the same
comment convention tests/run_tests.sh already uses), extended here with
`category` and `counterexample-vow-id`. The directory carries the coarse
expected status (verify/ -> Verified, verify-fail/ -> VerifyFailed,
verify-skip/ -> Skipped); the directives carry the fine-grained ground truth.

Usage:
    scripts/verify_eval.py [--verifier ./target/release/vow] [--filter NAME]
    scripts/verify_eval.py --discover        # print actual outcomes (authoring aid)

Exit code is 0 only when every program matches its ground truth; non-zero on
any soundness, precision, blame, or status regression.
"""

import argparse
import json
import os
import re
import subprocess
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# Directory -> coarse expected top-level `vow verify` status.
DIR_EXPECT = {
    "verify": "Verified",
    "verify-fail": "VerifyFailed",
    "verify-skip": "Skipped",
}

CATEGORIES = {
    "overflow",
    "bounds",
    "invariant",
    "caller-blame",
    "callee-blame",
    "model-drift",
    "unverifiable",
}

VALID_BLAME = {"caller", "callee", "none"}

# Verdict kinds, ordered by severity for reporting.
SOUNDNESS = "soundness"
PRECISION = "precision"
BLAME = "blame"
STATUS = "status"
HARNESS = "harness"
# A documented, tracked verifier soundness gap: the program is genuinely
# incorrect and *should* fail, but the verifier currently accepts it. Reported
# loudly but non-fatally until the underlying issue is fixed.
KNOWN_GAP = "known_gap"
# The verifier started catching a known gap — a (welcome) change that must flip
# the program from a known-gap xfail to a real verify-fail. Fatal: forces the
# label to be promoted rather than silently drifting.
GAP_FIXED = "gap_fixed"
OK = "ok"


class Expect:
    """Ground truth for one program, parsed from its `// TEST:` directives."""

    def __init__(self, path, expected_status):
        self.path = path
        self.name = os.path.splitext(os.path.basename(path))[0]
        self.expected_status = expected_status
        self.category = None
        self.skip_reason = None
        # When set, this program documents a current verifier soundness gap: it
        # is genuinely incorrect but the verifier accepts it today. (reason, issue)
        self.known_gap = None
        self.known_gap_issue = None
        # Each cex is a dict with optional keys: fn, blame, vow_id.
        self.cex = []


CEX_KV = re.compile(r'(\w+)\s*=\s*(?:"([^"]*)"|(\S+))')


def validate_blame(path, blame):
    """Reject a typo'd blame directive at load time rather than letting it
    masquerade as a BLAME regression after every match_cex fails."""
    if blame not in VALID_BLAME:
        raise ValueError(
            f"{path}: unknown counterexample blame {blame!r} "
            f"(expected one of {sorted(VALID_BLAME)})"
        )
    return blame


def parse_directives(path, default_status):
    """Read `// TEST:` directives from a program into an Expect."""
    exp = Expect(path, default_status)
    legacy = {}
    with open(path, "r", encoding="utf-8") as fh:
        for raw in fh:
            line = raw.strip()
            m = re.match(r"^//\s*TEST:\s*(.+)$", line)
            if not m:
                continue
            body = m.group(1).strip()
            if body.startswith("category"):
                exp.category = body.split(None, 1)[1].strip() if " " in body else None
            elif body.startswith("status"):
                exp.expected_status = body.split(None, 1)[1].strip()
            elif body.startswith("skip"):
                sm = re.match(r'skip\s+"(.+)"', body)
                if sm:
                    exp.skip_reason = sm.group(1)
            elif body.startswith("known-soundness-gap"):
                gm = re.match(r'known-soundness-gap\s+"(.+?)"(?:\s+#(\d+))?', body)
                if gm:
                    exp.known_gap = gm.group(1)
                    exp.known_gap_issue = gm.group(2)
            elif body.startswith("counterexample-fn"):
                fm = re.match(r'counterexample-fn\s+"(.+)"', body)
                if fm:
                    legacy["fn"] = fm.group(1)
            elif body.startswith("counterexample-blame"):
                legacy["blame"] = validate_blame(
                    path, body.split(None, 1)[1].strip().lower()
                )
            elif body.startswith("counterexample-vow-id"):
                legacy["vow_id"] = int(body.split(None, 1)[1].strip())
            elif body.startswith("cex"):
                cex = {}
                for key, q, bare in CEX_KV.findall(body[len("cex"):]):
                    val = q if q != "" or bare == "" else bare
                    if key == "fn":
                        cex["fn"] = val
                    elif key == "blame":
                        cex["blame"] = validate_blame(path, val.lower())
                    elif key == "vow_id":
                        cex["vow_id"] = int(val)
                if cex:
                    exp.cex.append(cex)
    if legacy and not exp.cex:
        exp.cex.append(legacy)
    return exp


def run_json(verifier, args):
    """Invoke the verifier and parse its JSON stdout.

    Returns None on timeout, empty output, or non-JSON output. On every failure
    path the verifier's stderr is forwarded to our own stderr so a CI failure is
    debuggable instead of collapsing to a bare "no parseable JSON". A bounded
    timeout keeps a hung verifier (e.g. ESBMC stuck in a solver) from hanging
    the whole harness — the gating corpus programs are small, so 120s is ample.
    """
    def _forward_stderr(proc):
        if proc.stderr.strip():
            print(
                f"[verify_eval] verifier stderr ({' '.join(args)}): "
                f"{proc.stderr.rstrip()}",
                file=sys.stderr,
            )

    try:
        proc = subprocess.run(
            [verifier] + args,
            capture_output=True,
            text=True,
            cwd=REPO_ROOT,
            timeout=120,
        )
    except subprocess.TimeoutExpired:
        print(
            f"[verify_eval] verifier timed out after 120s: {' '.join(args)}",
            file=sys.stderr,
        )
        return None
    out = proc.stdout.strip()
    if not out:
        _forward_stderr(proc)
        return None
    try:
        return json.loads(out)
    except json.JSONDecodeError:
        _forward_stderr(proc)
        return None


def actual_cex(verify_json):
    """Normalize a verify result's counterexamples into comparable dicts."""
    result = []
    for ce in verify_json.get("counterexamples", []) or []:
        blame = ce.get("blame")
        result.append(
            {
                "fn": ce.get("function"),
                "blame": (blame or "none").lower(),
                "vow_id": ce.get("vow_id"),
            }
        )
    return result


def match_cex(want, actuals):
    """Return (kind, detail, matched) for one expected cex.

    kind is 'ok' (with the matched actual cex as the third element so the
    caller can consume it and detect surplus), or 'blame'/'vow_id'/'missing'
    (with detail and a None matched).
    """
    # An exact match on every specified field is success.
    for got in actuals:
        if "fn" in want and want["fn"] != got["fn"]:
            continue
        if "blame" in want and want["blame"] != got["blame"]:
            continue
        if "vow_id" in want and want["vow_id"] != got["vow_id"]:
            continue
        return "ok", None, got
    # No exact match: diagnose the closest cause for the same function.
    same_fn = [g for g in actuals if g["fn"] == want.get("fn")] or actuals
    if "blame" in want and all(want["blame"] != g["blame"] for g in same_fn):
        got = ", ".join(sorted({g["blame"] for g in same_fn}))
        return "blame", f"blame want={want['blame']} got={got or 'none'}", None
    if "vow_id" in want and all(want["vow_id"] != g["vow_id"] for g in same_fn):
        got = ", ".join(sorted({str(g["vow_id"]) for g in same_fn}))
        return "vow_id", f"vow_id want={want['vow_id']} got={got or 'none'}", None
    return "missing", f"no counterexample matched {want}", None


def vacuity_check(verifier, path):
    """Classify the secondary contracts guard for a should-pass program.

    Returns (verdict, detail):
      OK        — the guard ran and confirmed no vacuous/disproved contract.
      SOUNDNESS — a contract was proven only vacuously, OR `contracts --verify`
                  *disproved* a contract while `vow verify` reported Verified
                  (a verify/contracts divergence on a should-pass program).
      HARNESS   — the guard itself could not run at all (timeout/crash/non-JSON);
                  fail CLOSED so a broken `contracts --verify` cannot silently
                  pass as "not vacuous".

    Per-contract incompleteness counters (`unknown`/`timeout`/`error`/`skipped`)
    are tolerated: the primary `vow verify` is authoritative for the should-pass
    verdict, and the per-clause `contracts --verify` path legitimately returns
    `unknown` for contracts ESBMC cannot decide there (e.g. modulo reasoning in
    tests/verify/modulo_safe.vow) even when the program is genuinely Verified.
    Failing those would be a false reject, not a soundness signal.
    """
    res = run_json(verifier, ["contracts", "--verify", "--no-cache", path])
    if res is None:
        return HARNESS, (
            "vacuity guard could not run: `contracts --verify` produced no "
            "parseable output"
        )
    summary = res.get("summary", {}) or {}
    if summary.get("vacuous", 0):
        return SOUNDNESS, f"vacuous proof: {summary['vacuous']} contract(s) proven vacuously"
    if summary.get("failed", 0):
        return SOUNDNESS, (
            f"verify/contracts divergence: `contracts --verify` disproved "
            f"{summary['failed']} contract(s) on a Verified program"
        )
    return OK, None


def classify(exp, verify_json, verifier):
    """Compare actual verifier output against ground truth -> (verdict, detail)."""
    if verify_json is None:
        return HARNESS, "verifier produced no parseable JSON"
    actual = verify_json.get("status")

    # A documented soundness gap: the program is genuinely incorrect and should
    # fail, but the verifier accepts it today. Tolerated (non-fatal) while the
    # status stays Verified; promoted to a hard failure the moment it changes.
    if exp.known_gap:
        ref = f" (#{exp.known_gap_issue})" if exp.known_gap_issue else ""
        if actual == "Verified":
            return KNOWN_GAP, f"{exp.known_gap}{ref}"
        return GAP_FIXED, (
            f"verifier now reports {actual} — known gap{ref} may be fixed; "
            f"promote to a verify-fail program"
        )

    if exp.expected_status == "Verified":
        if actual != "Verified":
            kind = PRECISION if actual in ("VerifyFailed", "Skipped") else STATUS
            return kind, f"expected Verified, got {actual}"
        vstatus, vdetail = vacuity_check(verifier, exp.path)
        if vstatus != OK:
            return vstatus, vdetail
        return OK, None

    if exp.expected_status == "VerifyFailed":
        if actual == "Verified":
            return SOUNDNESS, "expected a counterexample, verifier proved it"
        if actual != "VerifyFailed":
            return STATUS, f"expected VerifyFailed, got {actual}"
        actuals = actual_cex(verify_json)
        if not exp.cex:
            # Fail closed: a verify-fail fixture promises an exact {blame, vow_id}
            # set, so one with no declared counterexample (missing/typo'd
            # directive) must not pass silently on any VerifyFailed status.
            return HARNESS, (
                "verify-fail fixture declares no expected counterexample — add a "
                "counterexample-fn/blame/vow-id (or cex) directive, or `// TEST: "
                'skip "<reason>"`'
            )
        if not actuals:
            return STATUS, "VerifyFailed but no counterexamples emitted"
        # Enforce the EXACT counterexample set: every expected cex must match a
        # distinct actual, and no surplus actual may remain. Consuming matches
        # lets a verifier regression that emits an extra bogus counterexample
        # (on top of the expected one) surface instead of passing silently.
        remaining = list(actuals)
        for want in exp.cex:
            kind, detail, matched = match_cex(want, remaining)
            if kind != "ok":
                return BLAME, detail
            remaining.remove(matched)
        if remaining:
            extra = remaining[0]
            return BLAME, (
                f"unexpected counterexample fn={extra['fn']} "
                f"blame={extra['blame']} vow_id={extra['vow_id']} "
                f"(actual cex set exceeds the {len(exp.cex)} declared — declare "
                f"every expected counterexample, or this is a verifier regression)"
            )
        return OK, None

    if exp.expected_status == "Skipped":
        if actual == "Skipped":
            return OK, None
        return STATUS, f"expected Skipped, got {actual}"

    return STATUS, f"unknown expected status {exp.expected_status!r}"


def collect(filter_name):
    """Yield (Expect) for every corpus program, ordered by dir then name."""
    for sub, status in DIR_EXPECT.items():
        d = os.path.join(REPO_ROOT, "tests", sub)
        if not os.path.isdir(d):
            continue
        for fname in sorted(os.listdir(d)):
            if not fname.endswith(".vow"):
                continue
            path = os.path.join(d, fname)
            exp = parse_directives(path, status)
            if filter_name and filter_name not in exp.name:
                continue
            yield sub, exp


def discover(verifier, filter_name):
    """Print actual verifier outcomes to aid directive authoring."""
    for sub, exp in collect(filter_name):
        vj = run_json(verifier, ["verify", "--no-cache", exp.path])
        status = vj.get("status") if vj else "<no-json>"
        cex = actual_cex(vj) if vj else []
        line = f"{sub}/{exp.name}: status={status}"
        for ce in cex:
            line += f"  [fn={ce['fn']} blame={ce['blame']} vow_id={ce['vow_id']}]"
        print(line)
    return 0


def banner(title, rows):
    print(f"\n{'=' * 4} {title} ({len(rows)}) {'=' * 4}")
    for name, detail in rows:
        print(f"  {name}: {detail}")


def evaluate(verifier, filter_name, output_dir):
    buckets = {
        SOUNDNESS: [],
        PRECISION: [],
        BLAME: [],
        STATUS: [],
        HARNESS: [],
        GAP_FIXED: [],
    }
    known_gaps = []
    report = []
    passed = 0
    cat_counts = {}
    missing_category = []
    bad_category = []

    for sub, exp in collect(filter_name):
        if exp.skip_reason:
            continue
        if exp.category is None:
            missing_category.append(f"{sub}/{exp.name}")
        elif exp.category not in CATEGORIES:
            bad_category.append(f"{sub}/{exp.name}={exp.category}")
        else:
            cat_counts[exp.category] = cat_counts.get(exp.category, 0) + 1

        vj = run_json(verifier, ["verify", "--no-cache", exp.path])
        verdict, detail = classify(exp, vj, verifier)
        report.append(
            {
                "name": exp.name,
                "dir": sub,
                "category": exp.category,
                "expected_status": exp.expected_status,
                "actual_status": (vj or {}).get("status"),
                "verdict": verdict,
                "detail": detail,
            }
        )
        if verdict == OK:
            passed += 1
        elif verdict == KNOWN_GAP:
            known_gaps.append((f"{sub}/{exp.name}", detail))
        else:
            buckets[verdict].append((f"{sub}/{exp.name}", detail))

    total = passed + len(known_gaps) + sum(len(v) for v in buckets.values())
    print(f"Verifier-evaluation harness: {passed}/{total} programs match ground truth")

    print("\nCategory coverage:")
    for cat in sorted(CATEGORIES):
        print(f"  {cat:<14} {cat_counts.get(cat, 0)}")
    if missing_category:
        print(f"  (!) {len(missing_category)} program(s) missing a category directive")
    if bad_category:
        print(f"  (!) unknown category: {', '.join(bad_category)}")

    if buckets[SOUNDNESS]:
        banner("SOUNDNESS REGRESSIONS — false-accepts (verifier trusted a bad program)",
               buckets[SOUNDNESS])
    if buckets[PRECISION]:
        banner("PRECISION REGRESSIONS — false-rejects (verifier rejected a good program)",
               buckets[PRECISION])
    if buckets[BLAME]:
        banner("BLAME / VOW_ID REGRESSIONS", buckets[BLAME])
    if buckets[STATUS]:
        banner("STATUS MISMATCHES", buckets[STATUS])
    if buckets[GAP_FIXED]:
        banner("KNOWN GAP APPEARS FIXED — promote to a verify-fail program",
               buckets[GAP_FIXED])
    if buckets[HARNESS]:
        banner("HARNESS ERRORS", buckets[HARNESS])
    if known_gaps:
        banner("KNOWN SOUNDNESS GAPS — tracked false-accepts, non-fatal", known_gaps)

    if output_dir:
        os.makedirs(output_dir, exist_ok=True)
        with open(os.path.join(output_dir, "report.json"), "w", encoding="utf-8") as fh:
            json.dump(
                {
                    "passed": passed,
                    "total": total,
                    "known_gaps": len(known_gaps),
                    "category_counts": cat_counts,
                    "results": report,
                },
                fh,
                indent=2,
            )

    failures = sum(len(v) for v in buckets.values())
    if failures == 0 and not missing_category and not bad_category:
        tail = f" ({len(known_gaps)} tracked known-gap(s))" if known_gaps else ""
        print(f"\nAll programs match their ground-truth labels{tail}.")
        return 0
    # Name every reason for the non-zero exit so a label/category problem is not
    # masked by a "0 regression(s)" line that reads like a green run.
    problems = []
    if failures:
        problems.append(f"{failures} regression(s) — see banners above")
    if missing_category:
        problems.append(f"{len(missing_category)} program(s) missing a category directive")
    if bad_category:
        problems.append(f"{len(bad_category)} program(s) with an unknown category")
    print(f"\nFAILED: {'; '.join(problems)}.")
    return 1


def main():
    ap = argparse.ArgumentParser(description="Vow verifier-evaluation harness (#334)")
    ap.add_argument(
        "--verifier",
        default=os.path.join(REPO_ROOT, "target", "release", "vow"),
        help="path to the verifier binary (default: target/release/vow)",
    )
    ap.add_argument("--filter", default=None, help="only programs whose name contains this")
    ap.add_argument(
        "--discover",
        action="store_true",
        help="print actual verifier outcomes instead of asserting ground truth",
    )
    ap.add_argument(
        "--output-dir",
        default=os.path.join(REPO_ROOT, "verify-eval.out"),
        help="directory for the machine-readable report.json",
    )
    args = ap.parse_args()

    if not os.path.exists(args.verifier):
        print(f"verifier not found: {args.verifier}", file=sys.stderr)
        return 2

    if args.discover:
        return discover(args.verifier, args.filter)
    return evaluate(args.verifier, args.filter, args.output_dir)


if __name__ == "__main__":
    sys.exit(main())
