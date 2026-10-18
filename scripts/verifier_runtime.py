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

import candidate_isolation

REPO_ROOT = Path(__file__).resolve().parent.parent

# A fixture may document that its own contracts are expected to fail; such a
# fixture is a precision-direction subject, not a soundness one.
DIRECTIVE_SKIP = re.compile(r'^// TEST: skip "(.*)"$', re.M)
# A tracked verifier soundness gap (verify_eval.py's KNOWN_GAP): the program is
# genuinely incorrect and the verifier currently accepts it. Reported, not fatal.
DIRECTIVE_KNOWN_GAP = re.compile(r"^// TEST: known-soundness-gap\s*(\S*)", re.M)
# A fixture whose counterexample the native replay harness deliberately cannot
# confirm (full_test.sh enforces that the claim matches reality for the native
# fixtures); a documented skip, reported and not fatal.
DIRECTIVE_REPLAY_SKIPPED = re.compile(r"^// TEST: replay-skipped (.+)$", re.M)

# Generated-corpus directives (scripts/gen_verifier_corpus.py, #1473).
# `gen-class` names the class (`exceeds-bound` or `control`), the shape and the
# regime; `expect-verified` marks an honest control, which must stay `Verified`
# and clean; `known-control-gap` tracks a control the verifier cannot yet prove
# by issue number.
DIRECTIVE_GEN_CLASS = re.compile(r"^// TEST: gen-class (\S+) (\S+)(?: (\S+))?$", re.M)
DIRECTIVE_EXPECT_VERIFIED = re.compile(r"^// TEST: expect-verified$", re.M)
DIRECTIVE_KNOWN_CONTROL_GAP = re.compile(r"^// TEST: known-control-gap (#\d+)$", re.M)

# `replay_reason` fragments for counterexamples that are verifier obligations
# rather than runtime-observable aborts (cli.md "Counterexample replay"). There
# is nothing to replay, so a `skipped` with one of these is not a replay gap.
# Any other reason fails closed.
NOT_A_RUNTIME_ABORT = (
    "a shift-count counterexample is a verifier obligation",
    "no runtime abort is predicted for this obligation",
    "a byte_at counterexample is a verifier obligation",
    "counterexample belongs to a co-emitted callee",
)

DEFAULT_VERIFIERS = {None: "target/release/vow", "esbmc": "target/release/vow"}


def relative_to_repo(path):
    """Repo-relative report key; a corpus outside the repo keeps its own path."""
    try:
        return str(Path(path).relative_to(REPO_ROOT))
    except ValueError:
        return str(path)


def default_verifier(backend):
    return DEFAULT_VERIFIERS.get(backend, "build/vowc")


def _backend_args(backend):
    return ["--backend", backend] if backend else []


def run_json(binary, args, timeout):
    try:
        proc = subprocess.run(
            [str(binary)] + args,
            capture_output=True,
            text=True,
            cwd=REPO_ROOT,
            env=candidate_isolation.scrubbed_env(),
            timeout=timeout,
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
    """Run a --mode debug binary and look for a VowViolation on stderr.

    Returns `(violation, error)` like `run_json`, because "ran and did not
    violate" and "never finished" are different answers: collapsing both to
    None makes a hung binary read as proof that the verifier was right.

    Runs from a disposable directory with a scrubbed environment: this is a
    model-authored candidate's own binary, not the verifier itself (#1188).
    """
    # A relative path resolves against the CHILD's cwd, not the parent's, so
    # this must happen before a disposable cwd changes what "relative" means.
    path = Path(path).resolve()
    try:
        with candidate_isolation.disposable_workdir() as d:
            proc = subprocess.run(
                [str(path)],
                capture_output=True,
                text=True,
                cwd=d,
                env=candidate_isolation.scrubbed_env(),
                timeout=timeout,
                stdin=subprocess.DEVNULL,
            )
    except subprocess.TimeoutExpired:
        return None, f"debug binary timed out after {timeout}s"
    # __vow_violation writes the JSON to stderr, then exits non-zero.
    for line in proc.stderr.splitlines():
        line = line.strip()
        if '"VowViolation"' in line:
            try:
                return json.loads(line), None
            except json.JSONDecodeError:
                return {"error": "VowViolation", "raw": line}, None
    return None, None


NOT_VERIFIED_NOTE = re.compile(r"to contracted (.+?) (?:were|was) not verified")


def _disclaimed_callees(verify):
    """Functions the verifier said calls into were not verified."""
    callees = set()
    for d in verify.get("diagnostics") or []:
        if d.get("error_code") != "VerificationSkipped":
            continue
        note = NOT_VERIFIED_NOTE.search(d.get("message") or "")
        if note:
            callees.update(re.findall(r"`([^`]+)`", note.group(1)))
    return callees


def _violated_function(vow_file, verifier, vow_id, timeout):
    """The function owning contract `vow_id`, or None if it cannot be mapped."""
    contracts, _ = run_json(verifier, ["contracts", str(vow_file)], timeout)
    for c in (contracts or {}).get("contracts") or []:
        if c.get("vow_id") == vow_id:
            return c.get("function")
    return None


def debug_run(vow_file, verifier, outdir, timeout):
    """Build `vow_file` with `--mode debug` and run it: `(violation, error)`."""
    exe = outdir / (Path(vow_file).stem + "_dbg")
    build, err = run_json(
        verifier,
        [
            "build",
            "--mode",
            "debug",
            "--no-verify",
            "--no-cache",
            str(vow_file),
            "-o",
            str(exe),
        ],
        timeout,
    )
    if build is None or not build.get("executable"):
        return None, err or "debug build produced no executable"
    violation, err = run_debug_binary(exe, timeout)
    exe.unlink(missing_ok=True)
    Path(str(exe) + ".o").unlink(missing_ok=True)
    return violation, err


def check_witness(runtime, gen_class):
    """A generated program's runtime must match the class it was generated as.

    The soundness direction is only as strong as the witness: an `exceeds-bound`
    program whose `main` never reaches the false contract, or a `control` that
    violates one, makes a green sweep mean nothing. `runtime` is the
    `(violation, error)` of the debug run, taken whatever the verifier said, so
    the corpus is checked against the runtime itself and not against the
    verifier it is meant to test.

    The violation of an `exceeds-bound` program must blame the Callee: an
    `ensures` of the generated function. A Caller violation means `main` broke
    a `requires`, which a Verified result may disclaim (`unverified-call-site`),
    so it would let a false proof through unmeasured.
    """
    violation, err = runtime

    def verdict(ok, detail):
        return {
            "direction": "witness",
            "verdict": "ok" if ok else "WITNESS-BROKEN",
            "detail": detail,
        }

    if err:
        return verdict(False, err)
    if gen_class == "exceeds-bound":
        if violation is None:
            return verdict(False, "the false contract was never violated at runtime")
        if violation.get("blame") != "Callee":
            return verdict(
                False,
                f"runtime violation blames {violation.get('blame')}, not an `ensures` "
                f"of the generated function (vow_id={violation.get('vow_id')})",
            )
    if gen_class == "control" and violation is not None:
        return verdict(
            False, f"the honest control violated vow_id={violation.get('vow_id')}"
        )
    return verdict(True, f"{gen_class} as generated")


def check_soundness(vow_file, verifier, outdir, timeout, backend=None, runtime=None):
    """Verified ⇒ no VowViolation at runtime.

    Only programs the verifier actually proved are subjects: a fixture it
    already rejects cannot produce a false proof. `runtime` is an optional
    `(violation, error)` of an earlier debug run, reused instead of rebuilding.
    """
    verify, err = run_json(
        verifier,
        ["verify", *_backend_args(backend), "--no-cache", str(vow_file)],
        timeout,
    )
    if verify is None:
        return {"direction": "soundness", "verdict": "skipped", "detail": err}
    if verify.get("status") != "Verified":
        return {
            "direction": "soundness",
            "verdict": "not-applicable",
            "detail": f"status {verify.get('status')}",
            "verify_status": verify.get("verify_status"),
            "counterexamples": len(verify.get("counterexamples") or []),
        }

    violation, err = runtime or debug_run(vow_file, verifier, outdir, timeout)
    if err:
        # The runtime half never answered, so neither did the gate. `skipped`
        # is the file's existing "no verdict" verdict, which the pair-review
        # adapter already turns into an unjudged claim.
        return {"direction": "soundness", "verdict": "skipped", "detail": err}
    if violation is None:
        return {"direction": "soundness", "verdict": "ok", "detail": "no violation"}
    disclaimed = _disclaimed_callees(verify)
    if (
        backend == "native"
        and violation.get("blame") == "Caller"
        and disclaimed
        and _violated_function(vow_file, verifier, violation.get("vow_id"), timeout)
        in disclaimed
    ):
        # `Verified` carried an explicit note that calls into this callee were
        # not verified (its `requires` is a debug-mode runtime check only), so
        # its Caller violation is the disclaimed obligation firing, not a
        # proof that was false.
        return {
            "direction": "soundness",
            "verdict": "unverified-call-site",
            "detail": (
                f"runtime Caller violation vow_id={violation.get('vow_id')} at a "
                "call the verifier reported as not verified"
            ),
        }
    return {
        "direction": "soundness",
        "verdict": "SOUNDNESS",
        "detail": (
            f"verified clean but runtime reported VowViolation "
            f"vow_id={violation.get('vow_id')} blame={violation.get('blame')}"
        ),
    }


def check_precision(vow_file, verifier, timeout, backend=None):
    """VerifyFailed ⇒ every counterexample replays concretely.

    `replay: diverged` is the finding. Under the ESBMC backend `aborted` is an
    honest runtime failure that preempted the predicted vow check and `skipped`
    means the replay harness declined a case it cannot model; neither is a
    precision failure.

    The native backend is held to the stricter contract of cli.md ("a
    counterexample that does not replay is a verifier bug"): `aborted` and any
    `VerifierBug` diagnostic are PRECISION findings, and a `skipped` replay is
    a REPLAY-GAP (the counterexample was never shown to reproduce).
    """
    result, err = run_json(
        verifier,
        [
            "verify",
            *_backend_args(backend),
            "--replay-cex",
            "--no-cache",
            str(vow_file),
        ],
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

    native = backend == "native"
    diverged, aborted, skipped, confirmed = [], [], [], 0
    skipped_reasons = []
    buckets = {"diverged": diverged, "aborted": aborted}
    for ce in ces:
        replay = ce.get("replay")
        if replay in buckets:
            buckets[replay].append(
                f"{ce.get('function')} vow_id={ce.get('vow_id')}: "
                f"{ce.get('replay_reason')}"
            )
        elif replay == "confirmed":
            confirmed += 1
        else:
            skipped.append(f"{ce.get('function')}: {ce.get('replay_reason')}")
            skipped_reasons.append(ce.get("replay_reason") or "no replay reported")

    bugs = [
        f"VerifierBug: {d.get('message')}"
        for d in result.get("diagnostics") or []
        if d.get("error_code") == "VerifierBug"
    ]
    findings = diverged + (aborted if native else []) + bugs
    if findings:
        return {
            "direction": "precision",
            "verdict": "PRECISION",
            "detail": "; ".join(findings),
        }
    gaps = [
        f
        for f, r in zip(skipped, skipped_reasons)
        if not any(frag in r for frag in NOT_A_RUNTIME_ABORT)
    ]
    if native and gaps:
        return {
            "direction": "precision",
            "verdict": "REPLAY-GAP",
            "detail": f"{len(gaps)} replay(s) skipped: " + "; ".join(gaps),
            "replay_skipped": skipped_reasons,
        }
    if confirmed:
        detail = f"{confirmed} counterexample(s) confirmed"
        if aborted:
            detail += f"; {len(aborted)} aborted before reaching the vow check: "
            detail += "; ".join(aborted)
        return {
            "direction": "precision",
            "verdict": "ok",
            "detail": detail,
            "replay_skipped": skipped_reasons,
        }
    if aborted:
        # No counterexample confirmed a VowViolation, so an abort-only result
        # cannot satisfy the differential gate (--min-checked) — it's honest
        # runtime failure evidence, not validation that the model is right.
        return {
            "direction": "precision",
            "verdict": "skipped",
            "detail": f"{len(aborted)} aborted before reaching the vow check: "
            + "; ".join(aborted),
        }
    return {
        "direction": "precision",
        "verdict": "skipped",
        "detail": "; ".join(skipped) or "replay not attempted",
        "replay_skipped": skipped_reasons,
    }


def apply_generated_expectations(check, text):
    """Hold a generated program to the class its directives declare.

    `control`: the verifier must prove it and the binary must run clean. Anything
    else is CONTROL-LOST, unless a `known-control-gap #N` tracks it; a tracked
    gap that no longer reproduces is STALE-KNOWN-GAP, so the ledger cannot rot.

    `exceeds-bound` needs nothing here: a `Verified` run that violates at runtime
    is already SOUNDNESS, and `check_witness` separately guarantees that the
    runtime does violate.
    """
    cls = DIRECTIVE_GEN_CLASS.search(text)
    if check["direction"] != "soundness" or not cls:
        return check
    if DIRECTIVE_EXPECT_VERIFIED.search(text):
        gap = DIRECTIVE_KNOWN_CONTROL_GAP.search(text)
        lost = check["verdict"] in ("not-applicable", "skipped")
        # A tracked gap is a completeness `unknown`; a counterexample, a
        # timeout, an error or a skipped run is something else and stays lost.
        open_unknown = (
            check["verdict"] == "not-applicable"
            and check.get("verify_status") == "unknown"
            and not check.get("counterexamples")
        )
        if open_unknown and gap:
            return {
                **check,
                "verdict": "known-control-gap",
                "detail": f"{check['detail']} (tracked: {gap.group(1)})",
            }
        if lost:
            return {
                **check,
                "verdict": "CONTROL-LOST",
                "detail": f"honest control not proven: {check['detail']}",
            }
        if gap and check["verdict"] == "ok":
            return {
                **check,
                "verdict": "STALE-KNOWN-GAP",
                "detail": f"{gap.group(1)} no longer reproduces; drop its entry",
            }
    return check


FAILING_GENERATED = ("CONTROL-LOST", "STALE-KNOWN-GAP", "WITNESS-BROKEN")


def generated_outcomes(records):
    """`class regime: soundness verdict` -> count, for generated programs only."""
    counts = {}
    for rec in records:
        gen = rec.get("generated")
        if gen:
            key = f"{gen['class']} {gen['regime']}: {rec['checks'][0]['verdict']}"
            counts[key] = counts.get(key, 0) + 1
    return counts


def default_roots(backend=None):
    roots = [REPO_ROOT / "tests" / "verify", REPO_ROOT / "tests" / "verify-fail"]
    if backend == "native":
        roots += [
            REPO_ROOT / "tests" / "verify-native" / d
            for d in ("pass", "fail", "unknown")
        ]
    return roots + [REPO_ROOT / "examples"]


def resolve_roots(roots, extra, backend):
    """Explicit roots, or the backend's defaults, plus any `--extra-root`."""
    return list(roots or default_roots(backend)) + list(extra)


def collect_corpus(roots, shard=None):
    """Sorted, de-duplicated `.vow` files under `roots`, optionally one shard.

    Relative roots resolve against the repo so a CLI argument and an absolute
    default agree; the report keys on repo-relative paths.
    """
    corpus = []
    for root in roots:
        p = Path(root)
        p = p if p.is_absolute() else (REPO_ROOT / p)
        corpus.extend([p] if p.is_file() else sorted(p.rglob("*.vow")))
    corpus = sorted({c.resolve() for c in corpus}, key=str)
    if shard:
        k, n = (int(x) for x in shard.split("/"))
        corpus = [f for i, f in enumerate(corpus) if i % n == k]
    return corpus


def main():
    ap = argparse.ArgumentParser(
        description="Verifier-model vs concrete-runtime differential (#1084)"
    )
    ap.add_argument("roots", nargs="*", default=None)
    ap.add_argument("--verifier", default=None)
    ap.add_argument(
        "--backend",
        choices=("esbmc", "native"),
        default=None,
        help="verification backend passed to `verify` (native needs a vowc)",
    )
    ap.add_argument(
        "--extra-root",
        action="append",
        default=[],
        metavar="PATH",
        help="also sweep PATH (repeatable), e.g. a generated corpus",
    )
    ap.add_argument("--output-dir", default="verifier-runtime.out")
    ap.add_argument("--timeout", type=int, default=300)
    ap.add_argument("--shard", default=None, metavar="K/N")
    ap.add_argument(
        "--min-checked",
        type=int,
        default=1,
        help="fail if fewer than N fixtures reached a verdict",
    )
    args = ap.parse_args()

    verifier = Path(args.verifier or default_verifier(args.backend))
    if args.backend == "native" and verifier.name == "vow":
        print(
            "error: --backend native needs the self-hosted compiler "
            f"(build/vowc); {verifier} is the Rust stage 0 and rejects it",
            file=sys.stderr,
        )
        return 2
    if not verifier.exists():
        print(f"error: verifier not found: {verifier}", file=sys.stderr)
        return 2

    corpus = collect_corpus(
        resolve_roots(args.roots, args.extra_root, args.backend), args.shard
    )

    outdir = Path(args.output_dir)
    outdir.mkdir(parents=True, exist_ok=True)

    print(f"=== Verifier-model vs runtime: {len(corpus)} fixtures ===")
    print(f"  verifier: {verifier}")
    print(f"  backend : {args.backend or 'default'}")
    print()

    records, soundness, precision, checked = [], [], [], 0
    replay_gaps, replay_skipped, generated_failures = [], {}, []
    for i, f in enumerate(corpus, 1):
        rel = relative_to_repo(f)
        text = f.read_text(errors="replace")
        skip = DIRECTIVE_SKIP.search(text)
        if skip:
            records.append(
                {
                    "file": rel,
                    "checks": [
                        {"verdict": "skipped", "detail": f"directive: {skip.group(1)}"}
                    ],
                }
            )
            continue
        known_gap = DIRECTIVE_KNOWN_GAP.search(text)
        replay_skipped_note = DIRECTIVE_REPLAY_SKIPPED.search(text)

        gen = DIRECTIVE_GEN_CLASS.search(text)
        runtime = debug_run(f, verifier, outdir, args.timeout) if gen else None
        checks = [
            apply_generated_expectations(
                check_soundness(
                    f, verifier, outdir, args.timeout, args.backend, runtime
                ),
                text,
            ),
            check_precision(f, verifier, args.timeout, args.backend),
        ]
        if gen:
            checks.append(check_witness(runtime, gen.group(1)))
        for c in checks:
            if c["verdict"] in FAILING_GENERATED:
                generated_failures.append((rel, c))
                print(f"  {c['verdict']} {rel}")
                print(f"            {c['detail']}")
            elif c["verdict"] == "SOUNDNESS":
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
            elif c["verdict"] == "REPLAY-GAP":
                if replay_skipped_note:
                    c["verdict"] = "documented-replay-skip"
                    c["detail"] += f" (documented: {replay_skipped_note.group(1)})"
                else:
                    replay_gaps.append((rel, c))
                    print(f"  REPLAY-GAP {rel}")
                    print(f"            {c['detail']}")
        for c in checks:
            for reason in c.get("replay_skipped", []):
                key = reason[:60]
                replay_skipped[key] = replay_skipped.get(key, 0) + 1
        if any(c["verdict"] == "ok" and c["direction"] != "witness" for c in checks):
            checked += 1
        record = {"file": rel, "checks": checks}
        if gen:
            record["generated"] = {
                "class": gen.group(1),
                "shape": gen.group(2),
                "regime": gen.group(3),
            }
        records.append(record)
        if i % 20 == 0:
            print(f"  ... {i}/{len(corpus)}")

    results = {
        "schema_version": 1,
        "verifier": str(verifier),
        "backend": args.backend,
        "corpus_size": len(corpus),
        "checked": checked,
        "soundness_failures": [f for f, _ in soundness],
        "precision_failures": [f for f, _ in precision],
        "replay_gaps": [f for f, _ in replay_gaps],
        "generated_failures": [f for f, _ in generated_failures],
        "generated_outcomes": generated_outcomes(records),
        "replay_skipped_by_reason": replay_skipped,
        "records": records,
    }
    (outdir / "results.json").write_text(json.dumps(results, indent=2) + "\n")

    print()
    print("=== Summary ===")
    print(f"  fixtures      : {len(corpus)}")
    print(f"  reached an ok : {checked}")
    print(f"  SOUNDNESS     : {len(soundness)}  (false proofs)")
    print(f"  PRECISION     : {len(precision)}  (counterexamples that do not replay)")
    disclaimed = sum(
        c["verdict"] == "unverified-call-site" for r in records for c in r["checks"]
    )
    if disclaimed:
        print(
            f"  unverified    : {disclaimed}  (Caller violation at a disclaimed call)"
        )
    if args.backend == "native":
        print(f"  REPLAY-GAP    : {len(replay_gaps)}  (replays skipped, undocumented)")
        for reason, count in sorted(replay_skipped.items(), key=lambda kv: -kv[1]):
            print(f"    {count:5d}  skipped: {reason}")
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
    outcomes = generated_outcomes(records)
    if outcomes:
        print(
            f"  GENERATED     : {len(generated_failures)} failing "
            "(CONTROL-LOST / STALE-KNOWN-GAP / WITNESS-BROKEN)"
        )
        for key, count in sorted(outcomes.items()):
            print(f"    {count:5d}  {key}")
    print(f"  results       : {outdir / 'results.json'}")

    if checked < args.min_checked:
        print(
            f"\nFAIL: only {checked} fixtures reached a verdict, "
            f"need >= {args.min_checked}.",
            file=sys.stderr,
        )
        return 2
    failing = soundness or precision or replay_gaps or generated_failures
    return 1 if failing else 0


if __name__ == "__main__":
    sys.exit(main())
