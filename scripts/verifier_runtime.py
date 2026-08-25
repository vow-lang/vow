#!/usr/bin/env python3
"""Differential: ESBMC C model vs concrete runtime (#1084).

The Rust and self-hosted compilers are two implementations of Vow's semantics.
The C model that `c_emitter` hands to ESBMC is a third, and it is the one the
language's central claim rests on. This sweep holds it to the other two.

Two directions, and they fail differently:

  SOUNDNESS  `vow verify` says Verified, but running the program in --mode debug
             reports a VowViolation. The proof was false. This is the only bug
             class that invalidates Vow's thesis outright.

  PRECISION  `vow verify` says VerifyFailed and emits a counterexample, but that
             counterexample does not reproduce when replayed concretely. The
             proof is honest; the evidence handed to the agent is not. Since
             CEGIS feeds counterexamples back as the input to the next fix, a
             lying counterexample actively misleads the repair loop.

Both directions are measured against fixtures the repo already maintains, so a
regression here is attributable to a compiler change rather than to a generated
program nobody has seen before. `--replay-cex` (#335) does the concrete replay;
this script is the sweep that applies it to the whole corpus instead of the two
fixtures that currently carry a `// TEST: replay` directive.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

SELF_MEM_LIMIT = 2_000_000 * 1024

# A fixture may document that its own contracts are expected to fail; such a
# fixture is a precision-direction subject, not a soundness one.
DIRECTIVE_SKIP = re.compile(r'^// TEST: skip "(.*)"$', re.M)
# A tracked verifier soundness gap (verify_eval.py's KNOWN_GAP): the program is
# genuinely incorrect and the verifier currently accepts it. Reported, not fatal.
DIRECTIVE_KNOWN_GAP = re.compile(r"^// TEST: known-soundness-gap\s*(\S*)", re.M)


def _limit():
    import resource

    resource.setrlimit(resource.RLIMIT_AS, (SELF_MEM_LIMIT, SELF_MEM_LIMIT))


def run_json(binary, args, timeout, limit=False):
    try:
        proc = subprocess.run(
            [str(binary)] + args,
            capture_output=True,
            text=True,
            cwd=REPO_ROOT,
            timeout=timeout,
            preexec_fn=_limit if limit else None,
        )
    except subprocess.TimeoutExpired:
        return None, "timeout"
    out = proc.stdout.strip()
    if not out:
        return None, f"no stdout (exit {proc.returncode})"
    try:
        return json.loads(out), None
    except json.JSONDecodeError:
        return None, "unparseable JSON"


def run_debug_binary(path, timeout):
    """Run a --mode debug binary and look for a VowViolation on stderr."""
    try:
        proc = subprocess.run(
            [str(path)],
            capture_output=True,
            text=True,
            cwd=REPO_ROOT,
            timeout=timeout,
            stdin=subprocess.DEVNULL,
        )
    except subprocess.TimeoutExpired:
        return None
    # __vow_violation writes the JSON to stderr, then exits non-zero.
    for line in proc.stderr.splitlines():
        line = line.strip()
        if '"VowViolation"' in line:
            try:
                return json.loads(line)
            except json.JSONDecodeError:
                return {"error": "VowViolation", "raw": line}
    return None


def check_soundness(vow_file, verifier, outdir, timeout):
    """Verified ⇒ no VowViolation at runtime.

    Only programs the verifier actually proved are subjects: a fixture it
    already rejects cannot produce a false proof.
    """
    verify, err = run_json(
        verifier, ["verify", "--no-cache", str(vow_file)], timeout
    )
    if verify is None:
        return {"direction": "soundness", "verdict": "skipped", "detail": err}
    if verify.get("status") != "Verified":
        return {
            "direction": "soundness",
            "verdict": "not-applicable",
            "detail": f"status {verify.get('status')}",
        }

    exe = outdir / (Path(vow_file).stem + "_dbg")
    build, err = run_json(
        verifier,
        ["build", "--mode", "debug", "--no-verify", "--no-cache",
         str(vow_file), "-o", str(exe)],
        timeout,
    )
    if build is None or not build.get("executable"):
        return {
            "direction": "soundness",
            "verdict": "skipped",
            "detail": err or "debug build produced no executable",
        }

    violation = run_debug_binary(exe, timeout)
    exe.unlink(missing_ok=True)
    Path(str(exe) + ".o").unlink(missing_ok=True)
    if violation is None:
        return {"direction": "soundness", "verdict": "ok", "detail": "no violation"}
    return {
        "direction": "soundness",
        "verdict": "SOUNDNESS",
        "detail": (
            f"verified clean but runtime reported VowViolation "
            f"vow_id={violation.get('vow_id')} blame={violation.get('blame')}"
        ),
    }


def check_precision(vow_file, verifier, timeout):
    """VerifyFailed ⇒ every counterexample replays concretely.

    `replay: diverged` is the finding. `skipped` is not: the replay harness
    declines cases it cannot model (aggregates on some paths, an entry file that
    defines main), and counting those as failures would drown the real ones.
    """
    result, err = run_json(
        verifier,
        ["verify", "--replay-cex", "--no-cache", str(vow_file)],
        timeout,
    )
    if result is None:
        return {"direction": "precision", "verdict": "skipped", "detail": err}
    if result.get("status") != "VerifyFailed":
        return {
            "direction": "precision",
            "verdict": "not-applicable",
            "detail": f"status {result.get('status')}",
        }

    ces = result.get("counterexamples") or []
    if not ces:
        return {
            "direction": "precision",
            "verdict": "skipped",
            "detail": "VerifyFailed with no counterexample",
        }

    diverged, skipped, confirmed = [], [], 0
    for ce in ces:
        replay = ce.get("replay")
        if replay == "diverged":
            diverged.append(
                f"{ce.get('function')} vow_id={ce.get('vow_id')}: "
                f"{ce.get('replay_reason')}"
            )
        elif replay == "confirmed":
            confirmed += 1
        else:
            skipped.append(f"{ce.get('function')}: {ce.get('replay_reason')}")

    if diverged:
        return {
            "direction": "precision",
            "verdict": "PRECISION",
            "detail": "; ".join(diverged),
        }
    if confirmed:
        return {
            "direction": "precision",
            "verdict": "ok",
            "detail": f"{confirmed} counterexample(s) replayed",
        }
    return {
        "direction": "precision",
        "verdict": "skipped",
        "detail": "; ".join(skipped) or "replay not attempted",
    }


def main():
    ap = argparse.ArgumentParser(
        description="Verifier-model vs concrete-runtime differential (#1084)"
    )
    ap.add_argument("roots", nargs="*", default=None)
    ap.add_argument("--verifier", default="target/release/vow")
    ap.add_argument("--output-dir", default="verifier-runtime.out")
    ap.add_argument("--timeout", type=int, default=300)
    ap.add_argument("--shard", default=None, metavar="K/N")
    ap.add_argument("--min-checked", type=int, default=1,
                    help="fail if fewer than N fixtures reached a verdict")
    args = ap.parse_args()

    verifier = Path(args.verifier)
    if not verifier.exists():
        print(f"error: verifier not found: {verifier}", file=sys.stderr)
        return 2

    roots = args.roots or [
        REPO_ROOT / "tests" / "verify",
        REPO_ROOT / "tests" / "verify-fail",
        REPO_ROOT / "examples",
    ]
    corpus = []
    for root in roots:
        # Resolve so a relative CLI argument and an absolute default agree; the
        # report keys on repo-relative paths.
        p = Path(root)
        p = p if p.is_absolute() else (REPO_ROOT / p)
        corpus.extend([p] if p.is_file() else sorted(p.rglob("*.vow")))
    corpus = sorted({c.resolve() for c in corpus}, key=str)
    if args.shard:
        k, n = (int(x) for x in args.shard.split("/"))
        corpus = [f for i, f in enumerate(corpus) if i % n == k]

    outdir = Path(args.output_dir)
    outdir.mkdir(parents=True, exist_ok=True)

    print(f"=== Verifier-model vs runtime: {len(corpus)} fixtures ===")
    print(f"  verifier: {verifier}")
    print()

    records, soundness, precision, checked = [], [], [], 0
    for i, f in enumerate(corpus, 1):
        rel = str(f.relative_to(REPO_ROOT))
        text = f.read_text(errors="replace")
        skip = DIRECTIVE_SKIP.search(text)
        if skip:
            records.append({"file": rel, "checks": [
                {"verdict": "skipped", "detail": f"directive: {skip.group(1)}"}]})
            continue
        known_gap = DIRECTIVE_KNOWN_GAP.search(text)

        checks = [
            check_soundness(f, verifier, outdir, args.timeout),
            check_precision(f, verifier, args.timeout),
        ]
        for c in checks:
            if c["verdict"] == "SOUNDNESS":
                # A documented gap is reported loudly but is not news.
                if known_gap:
                    c["verdict"] = "known-gap"
                    c["detail"] += f" (tracked: {known_gap.group(1) or 'documented'})"
                else:
                    soundness.append((rel, c))
                    print(f"  SOUNDNESS {rel}")
                    print(f"            {c['detail']}")
            elif c["verdict"] == "PRECISION":
                precision.append((rel, c))
                print(f"  PRECISION {rel}")
                print(f"            {c['detail']}")
        if any(c["verdict"] == "ok" for c in checks):
            checked += 1
        records.append({"file": rel, "checks": checks})
        if i % 20 == 0:
            print(f"  ... {i}/{len(corpus)}")

    results = {
        "schema_version": 1,
        "verifier": str(verifier),
        "corpus_size": len(corpus),
        "checked": checked,
        "soundness_failures": [f for f, _ in soundness],
        "precision_failures": [f for f, _ in precision],
        "records": records,
    }
    (outdir / "results.json").write_text(json.dumps(results, indent=2) + "\n")

    print()
    print("=== Summary ===")
    print(f"  fixtures      : {len(corpus)}")
    print(f"  reached an ok : {checked}")
    print(f"  SOUNDNESS     : {len(soundness)}  (false proofs)")
    print(f"  PRECISION     : {len(precision)}  (counterexamples that do not replay)")
    # Why a fixture produced no verdict matters as much as the verdicts: a sweep
    # where everything was not-applicable measured nothing.
    reasons = {}
    for rec in records:
        for c in rec["checks"]:
            if c["verdict"] in ("skipped", "not-applicable"):
                key = f"{c['verdict']}: {c['detail'].split('(')[0].strip()[:40]}"
                reasons[key] = reasons.get(key, 0) + 1
    if reasons:
        print("  no verdict:")
        for reason, count in sorted(reasons.items(), key=lambda kv: -kv[1])[:8]:
            print(f"    {count:5d}  {reason}")
    print(f"  results       : {outdir / 'results.json'}")

    if checked < args.min_checked:
        print(
            f"\nFAIL: only {checked} fixtures reached a verdict, "
            f"need >= {args.min_checked}.",
            file=sys.stderr,
        )
        return 2
    return 1 if (soundness or precision) else 0


if __name__ == "__main__":
    sys.exit(main())
