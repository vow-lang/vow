#!/usr/bin/env python3
"""Adversarial AI pair review of Rust <-> self-hosted module pairs (#1083).

Tier 3 of the equivalence programme (docs/equivalence/README.md). Asks a model
where two implementations of the same compiler stage disagree semantically, then
**mechanically confirms every claim** by running the discriminating program
through scripts/equivalence.py.

The confirmation gate is the whole design. A model asked to find subtle
differences between two large files will always produce plausible prose; the
2026-06-12 verification-honesty pass found half of the preceding audit's
severities overstated. So:

    a claim is a HYPOTHESIS until the runner says the compilers disagree.

Confirmed findings are reported and counted. Hypotheses are written to the
report for a human to read, and excluded from the summary counts. A hypothesis
whose program both compilers agree on is evidence the model was wrong, and is
labelled REFUTED — which is information, not noise.

Model calls cost real money, so this is a monthly agent-run command rather than
a CI step, and the ledger (docs/equivalence/ledger.json) keys each pair by
content hash so an unchanged pair is skipped rather than re-reviewed.
"""

import argparse
import hashlib
import json
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "bench"))

LEDGER = REPO_ROOT / "docs" / "equivalence" / "ledger.json"

# The Rust side of a pair is often a directory where the self-hosted side is one
# file; both are hashed so a change anywhere in the pair invalidates its review.
PAIRS = {
    "lexer": (
        ["vow-syntax/src/lexer.rs", "vow-syntax/src/token.rs"],
        "compiler/lexer.vow",
    ),
    "parser": (["vow-syntax/src/parser"], "compiler/parser.vow"),
    "checker": (
        [
            "vow-types/src/check.rs",
            "vow-types/src/linear.rs",
            "vow-types/src/exhaustiveness.rs",
            "vow-types/src/effects.rs",
        ],
        "compiler/checker.vow",
    ),
    "lower": (["vow-ir/src/lower"], "compiler/lower.vow"),
    "c_emitter": (["vow-verify/src/c_emitter.rs"], "compiler/c_emitter.vow"),
}

SYSTEM = """\
You are auditing two independent implementations of the same compiler stage for \
the Vow language. One is in Rust (the bootstrap compiler), one is in Vow itself \
(the self-hosted compiler). They are intended to be semantically identical: for \
every input program they must agree on whether to accept it, on which error code \
they emit when they reject it, and on the runtime behaviour of what they produce.

Your job is to find inputs where they DISAGREE.

Report only differences you can demonstrate with a concrete Vow program. For \
each one, give the smallest complete program that distinguishes the two \
implementations. Every program must start with a `module M` declaration — the \
Rust compiler rejects a file without one.

Do not report: stylistic differences, differences in message wording (only the \
error CODE is compared), performance, or anything you cannot express as a \
program. A claim without a distinguishing program is worthless here, because \
every claim is mechanically checked by compiling the program with both \
compilers.

Prefer the areas where these two implementations have historically diverged: \
match and pattern handling, integer width and signedness coercion, checked vs \
wrapping arithmetic, error and diagnostic emission paths, and silent-acceptance \
paths where one implementation returns an unknown/never type instead of \
reporting an error.

Reply with JSON only, no prose outside it:

{"findings": [
  {"claim": "one sentence: what differs",
   "area": "short label",
   "program": "module M\\nfn main() -> i32 [io] { ... }\\n",
   "expected_rust": "what the Rust implementation does",
   "expected_self": "what the self-hosted implementation does"}
]}

An empty findings list is a perfectly good answer if you find nothing \
demonstrable."""


def hash_pair(rust_paths, self_path):
    h = hashlib.sha256()
    inner = hashlib.sha256()
    for spec in rust_paths:
        p = REPO_ROOT / spec
        files = sorted(p.rglob("*.rs")) if p.is_dir() else [p]
        for f in files:
            inner.update(str(f.relative_to(REPO_ROOT)).encode())
            inner.update(f.read_bytes())
    h.update(inner.hexdigest().encode())
    h.update((REPO_ROOT / self_path).read_bytes())
    return h.hexdigest()


def read_pair(rust_paths, self_path, max_bytes):
    """Assemble the review prompt for one pair.

    Truncation is reported in the prompt rather than done silently: a model that
    does not know it saw half a file will confidently reason about the missing
    half.
    """
    chunks = []
    for spec in rust_paths:
        p = REPO_ROOT / spec
        files = sorted(p.rglob("*.rs")) if p.is_dir() else [p]
        for f in files:
            text = f.read_text(errors="replace")
            rel = f.relative_to(REPO_ROOT)
            if len(text) > max_bytes:
                text = text[:max_bytes] + f"\n// [TRUNCATED at {max_bytes} bytes]\n"
            chunks.append(f"=== RUST: {rel} ===\n{text}")
    sp = REPO_ROOT / self_path
    stext = sp.read_text(errors="replace")
    truncated = len(stext) > max_bytes
    if truncated:
        stext = stext[:max_bytes] + f"\n// [TRUNCATED at {max_bytes} bytes]\n"
    chunks.append(f"=== SELF-HOSTED: {self_path} ===\n{stext}")
    return "\n\n".join(chunks), truncated


def confirm(program, rust, self_bin, timeout):
    """Run one candidate program through the differential runner.

    Returns (verdict, detail). The runner is the judge: `confirmed` means it
    observed a divergence, `refuted` means both compilers agreed, and
    `inconclusive` means it could not compare (both rejected the program, a
    timeout, nondeterminism).
    """
    with tempfile.TemporaryDirectory() as d:
        src = Path(d) / "candidate.vow"
        src.write_text(program)
        proc = subprocess.run(
            [
                sys.executable,
                str(REPO_ROOT / "scripts" / "equivalence.py"),
                str(src),
                "--rust", str(rust),
                "--self", str(self_bin),
                "--output-dir", str(Path(d) / "out"),
                "--timeout", str(timeout),
                "--no-ledger",
                "--min-compared", "0",
            ],
            capture_output=True,
            text=True,
            cwd=REPO_ROOT,
        )
        results = Path(d) / "out" / "results.json"
        if not results.exists():
            return "inconclusive", f"runner produced no results (exit {proc.returncode})"
        data = json.loads(results.read_text())
        rec = data["records"][0] if data["records"] else None
        if rec is None:
            return "inconclusive", "runner examined no file"
        if rec["divergences"]:
            detail = "; ".join(
                f"[{v['observable']}] {v['detail']}" for v in rec["divergences"]
            )
            return "confirmed", detail
        if rec.get("skipped"):
            return "inconclusive", rec["skipped"]
        return "refuted", "both compilers agreed"


def review_pair(name, model, rust, self_bin, max_bytes, timeout):
    import llm

    rust_paths, self_path = PAIRS[name]
    body, truncated = read_pair(rust_paths, self_path, max_bytes)
    config = llm.make_config(model)
    resp = llm.chat(
        config,
        SYSTEM,
        [{"role": "user", "content": body}],
    )
    text = resp.content.strip()
    # Models wrap JSON in fences despite instructions; strip rather than fail.
    if text.startswith("```"):
        text = text.split("\n", 1)[1].rsplit("```", 1)[0]
    try:
        findings = json.loads(text).get("findings", [])
    except json.JSONDecodeError:
        return {
            "pair": name,
            "error": "model did not return parseable JSON",
            "raw": text[:2000],
            "truncated": truncated,
            "findings": [],
        }

    judged = []
    for f in findings:
        program = f.get("program", "")
        if not program.strip():
            f["verdict"] = "inconclusive"
            f["verdict_detail"] = "no program supplied"
        else:
            verdict, detail = confirm(program, rust, self_bin, timeout)
            f["verdict"] = verdict
            f["verdict_detail"] = detail
        judged.append(f)

    return {
        "pair": name,
        "truncated": truncated,
        "input_tokens": resp.input_tokens,
        "output_tokens": resp.output_tokens,
        "findings": judged,
    }


def main():
    ap = argparse.ArgumentParser(description="Adversarial pair review (#1083)")
    ap.add_argument("--model", default="claude-sonnet-4-20250514")
    ap.add_argument("--rust", default="target/release/vow")
    ap.add_argument("--self", dest="self_bin", default="build/vowc")
    ap.add_argument("--pair", action="append", default=[],
                    help="review only this pair; repeatable (default: all)")
    ap.add_argument("--output-dir", default="pair-review.out")
    ap.add_argument("--max-bytes", type=int, default=180_000,
                    help="per-file prompt budget (default: 180000)")
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--all", action="store_true",
                    help="review every pair even if unchanged since last review")
    args = ap.parse_args()

    for p in (Path(args.rust), Path(args.self_bin)):
        if not p.exists():
            print(f"error: compiler not found: {p}", file=sys.stderr)
            return 2

    ledger = json.loads(LEDGER.read_text()) if LEDGER.exists() else {"pairs": {}}
    names = args.pair or list(PAIRS)
    outdir = Path(args.output_dir)
    outdir.mkdir(parents=True, exist_ok=True)

    reviewed, skipped, results = [], [], []
    for name in names:
        rust_paths, self_path = PAIRS[name]
        digest = hash_pair(rust_paths, self_path)
        prior = ledger.get("pairs", {}).get(name, {})
        if (
            not args.all
            and prior.get("content_hash") == digest
            and prior.get("last_reviewed") not in (None, "never")
        ):
            skipped.append((name, prior.get("last_reviewed")))
            continue
        print(f"reviewing {name} ...")
        res = review_pair(
            name, args.model, args.rust, args.self_bin, args.max_bytes, args.timeout
        )
        res["content_hash"] = digest
        results.append(res)
        reviewed.append(name)

    confirmed = [
        (r["pair"], f) for r in results for f in r["findings"]
        if f.get("verdict") == "confirmed"
    ]
    hypotheses = [
        (r["pair"], f) for r in results for f in r["findings"]
        if f.get("verdict") == "inconclusive"
    ]
    refuted = sum(
        1 for r in results for f in r["findings"] if f.get("verdict") == "refuted"
    )

    (outdir / "results.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "model": args.model,
                "reviewed": reviewed,
                "skipped_unchanged": [n for n, _ in skipped],
                "confirmed": len(confirmed),
                "hypotheses": len(hypotheses),
                "refuted": refuted,
                "pairs": results,
            },
            indent=2,
        )
        + "\n"
    )

    print()
    print("=== Pair review ===")
    print(f"  model     : {args.model}")
    print(f"  reviewed  : {len(reviewed)} {reviewed}")
    # Report what was not looked at. A run that skipped everything must never
    # read as a clean bill of health.
    if skipped:
        print(f"  unchanged : {len(skipped)} (not re-reviewed)")
        for n, when in skipped:
            print(f"      {n} — last reviewed {when}")
    truncated = [r["pair"] for r in results if r.get("truncated")]
    if truncated:
        print(f"  truncated : {truncated} — reviewed on partial source")
    errored = [r["pair"] for r in results if r.get("error")]
    if errored:
        print(f"  errors    : {errored}")
    print(f"  CONFIRMED : {len(confirmed)}")
    print(f"  hypotheses: {len(hypotheses)} (unconfirmed — not counted as findings)")
    print(f"  refuted   : {refuted}")
    for pair, f in confirmed:
        print(f"    [{pair}] {f.get('claim', '?')}")
        print(f"            {f['verdict_detail']}")
    print(f"  results   : {outdir / 'results.json'}")

    return 1 if confirmed else 0


if __name__ == "__main__":
    sys.exit(main())
