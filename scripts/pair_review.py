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
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
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

VOW_FN = re.compile(r"^fn\s+([A-Za-z_][A-Za-z0-9_]*)\b", re.MULTILINE)
RUST_FN = re.compile(
    r"^[ \t]*(?:pub(?:\([^)]*\))?\s+)?"
    r"(?:(?:async|unsafe|const)\s+)*"
    r'(?:extern\s+"[^"]+"\s+)?'
    r"fn\s+([A-Za-z_][A-Za-z0-9_]*)\b",
    re.MULTILINE,
)


@dataclass(frozen=True)
class Unit:
    name: str
    text: str
    source: str = ""


@dataclass(frozen=True)
class Preambles:
    rust: tuple[tuple[str, str], ...] = ()
    self_hosted: tuple[tuple[str, str], ...] = ()


@dataclass
class Chunk:
    rust_units: list[Unit] = field(default_factory=list)
    self_units: list[Unit] = field(default_factory=list)
    oversize_units: list[str] = field(default_factory=list)


def split_units(text, pattern):
    """Split source at function boundaries without dropping any text."""
    matches = list(pattern.finditer(text))
    if not matches:
        return text, []
    preamble = text[: matches[0].start()]
    units = []
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(text)
        units.append(Unit(match.group(1), text[match.start() : end]))
    return preamble, units


def related(left, right):
    """Whether function names use the same name or a receiver prefix."""
    return left == right or left.endswith("_" + right) or right.endswith("_" + left)


def _source_files(specs, suffix):
    files = []
    for spec in specs:
        path = REPO_ROOT / spec
        files.extend(sorted(path.rglob(f"*{suffix}")) if path.is_dir() else [path])
    return files


def _load_units(files, pattern):
    preambles = []
    units = []
    for path in files:
        relative = str(path.relative_to(REPO_ROOT))
        preamble, source_units = split_units(path.read_text(errors="replace"), pattern)
        preambles.append((relative, preamble))
        units.extend(Unit(unit.name, unit.text, relative) for unit in source_units)
    return tuple(preambles), units


def load_pair_units(name):
    """Load one declared module pair as preambles and function units."""
    rust_paths, self_path = PAIRS[name]
    rust_preambles, rust_units = _load_units(_source_files(rust_paths, ".rs"), RUST_FN)
    self_preambles, self_units = _load_units(_source_files([self_path], ".vow"), VOW_FN)
    return Preambles(rust_preambles, self_preambles), rust_units, self_units


def render_chunk(chunk, preambles=None, index=1, total=1):
    """Render a review chunk with enough source context to stand alone."""
    preambles = preambles or Preambles()
    parts = [f"=== REVIEW CHUNK {index} OF {total} ===\n"]
    for source, text in preambles.rust:
        parts.append(f"=== RUST PREAMBLE: {source} ===\n{text}")
    for unit in chunk.rust_units:
        parts.append(f"=== RUST: {unit.source} (function {unit.name}) ===\n{unit.text}")
    for source, text in preambles.self_hosted:
        parts.append(f"=== SELF-HOSTED PREAMBLE: {source} ===\n{text}")
    for unit in chunk.self_units:
        parts.append(
            f"=== SELF-HOSTED: {unit.source} (function {unit.name}) ===\n{unit.text}"
        )
    return "\n\n".join(parts)


def plan_chunks(rust_units, self_units, chunk_bytes, preambles=None):
    """Pair related functions and greedily pack complete units into prompts."""
    if chunk_bytes <= 0:
        raise ValueError("chunk_bytes must be positive")
    preambles = preambles or Preambles()
    chunks = []
    current = Chunk()

    def nonempty(chunk):
        return bool(chunk.rust_units or chunk.self_units)

    def rendered_size(chunk):
        # A deliberately wide placeholder keeps the final `i of n` header from
        # pushing a planned chunk over its byte budget.
        return len(render_chunk(chunk, preambles, 999_999, 999_999).encode())

    def finish():
        nonlocal current
        if nonempty(current):
            chunks.append(current)
            current = Chunk()

    def add_one(unit, side):
        nonlocal current
        candidate = Chunk(
            rust_units=[*current.rust_units],
            self_units=[*current.self_units],
            oversize_units=[*current.oversize_units],
        )
        target = candidate.rust_units if side == "rust" else candidate.self_units
        target.append(unit)
        if nonempty(current) and rendered_size(candidate) > chunk_bytes:
            finish()
            candidate = Chunk()
            target = candidate.rust_units if side == "rust" else candidate.self_units
            target.append(unit)
        if rendered_size(candidate) > chunk_bytes:
            candidate.oversize_units.append(f"{unit.source}:{unit.name}")
        current = candidate

    def add_group(group_rust, group_self):
        nonlocal current
        candidate = Chunk(
            rust_units=[*current.rust_units, *group_rust],
            self_units=[*current.self_units, *group_self],
            oversize_units=[*current.oversize_units],
        )
        if rendered_size(candidate) <= chunk_bytes:
            current = candidate
            return
        if nonempty(current):
            finish()
            candidate = Chunk(rust_units=[*group_rust], self_units=[*group_self])
            if rendered_size(candidate) <= chunk_bytes:
                current = candidate
                return
        for unit in group_rust:
            add_one(unit, "rust")
        for unit in group_self:
            add_one(unit, "self")

    used_rust = set()
    for self_unit in self_units:
        matches = [
            (index, rust_unit)
            for index, rust_unit in enumerate(rust_units)
            if index not in used_rust and related(self_unit.name, rust_unit.name)
        ]
        used_rust.update(index for index, _ in matches)
        add_group([unit for _, unit in matches], [self_unit])

    for index, rust_unit in enumerate(rust_units):
        if index not in used_rust:
            add_one(rust_unit, "rust")
    finish()
    return chunks


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
                "--rust",
                str(rust),
                "--self",
                str(self_bin),
                "--output-dir",
                str(Path(d) / "out"),
                "--timeout",
                str(timeout),
                "--no-ledger",
                "--min-compared",
                "0",
            ],
            capture_output=True,
            text=True,
            cwd=REPO_ROOT,
        )
        results = Path(d) / "out" / "results.json"
        if not results.exists():
            return (
                "inconclusive",
                f"runner produced no results (exit {proc.returncode})",
            )
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


def _plan_record(chunks, preambles):
    total = len(chunks)
    records = []
    for index, chunk in enumerate(chunks, 1):
        records.append(
            {
                "index": index,
                "bytes": len(render_chunk(chunk, preambles, index, total).encode()),
                "rust_units": [f"{u.source}:{u.name}" for u in chunk.rust_units],
                "self_hosted_units": [f"{u.source}:{u.name}" for u in chunk.self_units],
                "oversize_units": chunk.oversize_units,
            }
        )
    return records


def _coverage(preambles, rust_units, self_units, selected):
    preamble_bytes = sum(
        len(text.encode()) for _, text in (*preambles.rust, *preambles.self_hosted)
    )
    total = preamble_bytes + sum(
        len(unit.text.encode()) for unit in (*rust_units, *self_units)
    )
    if total == 0:
        return 1.0
    reviewed = preamble_bytes if selected else 0
    reviewed += sum(
        len(unit.text.encode())
        for chunk in selected
        for unit in (*chunk.rust_units, *chunk.self_units)
    )
    return reviewed / total


def review_pair(
    name,
    model,
    rust,
    self_bin,
    chunk_bytes,
    timeout,
    max_chunks=0,
    dry_run=False,
    llm_module=None,
    confirm_fn=None,
):
    """Plan and, unless dry-running, review every selected function chunk."""
    preambles, rust_units, self_units = load_pair_units(name)
    chunks = plan_chunks(rust_units, self_units, chunk_bytes, preambles)
    selected_count = min(max_chunks, len(chunks)) if max_chunks else len(chunks)
    selected = chunks[:selected_count]
    result = {
        "pair": name,
        "truncated": False,
        "coverage": _coverage(preambles, rust_units, self_units, selected),
        "plan": {
            "chunk_bytes": chunk_bytes,
            "chunks": _plan_record(chunks, preambles),
        },
        "chunks_reviewed": [] if dry_run else list(range(1, selected_count + 1)),
        "chunks_deferred": list(range(selected_count + 1, len(chunks) + 1)),
        "errors": [],
        "input_tokens": 0,
        "output_tokens": 0,
        "findings": [],
    }
    if dry_run:
        return result

    if llm_module is None:
        import llm as llm_module

    confirm_fn = confirm_fn or confirm
    config = llm_module.make_config(model)
    for index, chunk in enumerate(selected, 1):
        body = render_chunk(chunk, preambles, index, len(chunks))
        try:
            response = llm_module.chat(
                config,
                SYSTEM,
                [{"role": "user", "content": body}],
            )
        except Exception as exc:  # noqa: BLE001 - isolate provider failures by chunk
            result["errors"].append(
                {"chunk_index": index, "error": f"model call failed: {exc}"}
            )
            continue
        result["input_tokens"] += response.input_tokens
        result["output_tokens"] += response.output_tokens
        text = response.content.strip()
        # Models wrap JSON in fences despite instructions; strip rather than fail.
        if text.startswith("```"):
            text = text.split("\n", 1)[1].rsplit("```", 1)[0]
        try:
            parsed = json.loads(text)
            findings = parsed.get("findings", [])
            if not isinstance(findings, list):
                raise ValueError("findings is not a list")
        except (json.JSONDecodeError, AttributeError, ValueError):
            result["errors"].append(
                {
                    "chunk_index": index,
                    "error": "model did not return parseable findings JSON",
                    "raw": text[:2000],
                }
            )
            continue

        for raw_finding in findings:
            if not isinstance(raw_finding, dict):
                result["errors"].append(
                    {
                        "chunk_index": index,
                        "error": "model finding was not a JSON object",
                    }
                )
                continue
            finding = dict(raw_finding)
            finding["chunk_index"] = index
            program = finding.get("program", "")
            if not isinstance(program, str) or not program.strip():
                finding["verdict"] = "inconclusive"
                finding["verdict_detail"] = "no program supplied"
            else:
                verdict, detail = confirm_fn(program, rust, self_bin, timeout)
                finding["verdict"] = verdict
                finding["verdict_detail"] = detail
            result["findings"].append(finding)
    return result


def main(argv=None):
    ap = argparse.ArgumentParser(description="Adversarial pair review (#1083)")
    ap.add_argument("--model", default="claude-sonnet-4-20250514")
    ap.add_argument("--rust", default="target/release/vow")
    ap.add_argument("--self", dest="self_bin", default="build/vowc")
    ap.add_argument(
        "--pair",
        action="append",
        default=[],
        help="review only this pair; repeatable (default: all)",
    )
    ap.add_argument("--output-dir", default="pair-review.out")
    ap.add_argument(
        "--chunk-bytes",
        type=int,
        default=120_000,
        help="rendered prompt budget per chunk (default: 120000)",
    )
    ap.add_argument(
        "--max-chunks-per-pair",
        type=int,
        default=0,
        help="review at most N chunks per pair; 0 is unlimited",
    )
    ap.add_argument(
        "--dry-run",
        action="store_true",
        help="write the chunk plan without model or compiler calls",
    )
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument(
        "--all",
        action="store_true",
        help="review every pair even if unchanged since last review",
    )
    args = ap.parse_args(argv)

    unknown = sorted(set(args.pair) - set(PAIRS))
    if unknown:
        ap.error(f"unknown pair(s): {', '.join(unknown)}")
    if args.chunk_bytes <= 0:
        ap.error("--chunk-bytes must be positive")
    if args.max_chunks_per_pair < 0:
        ap.error("--max-chunks-per-pair cannot be negative")

    if not args.dry_run:
        for path in (Path(args.rust), Path(args.self_bin)):
            if not path.exists():
                print(f"error: compiler not found: {path}", file=sys.stderr)
                return 2

    ledger = json.loads(LEDGER.read_text()) if LEDGER.exists() else {"pairs": {}}
    names = args.pair or list(PAIRS)
    outdir = Path(args.output_dir)
    outdir.mkdir(parents=True, exist_ok=True)

    planned, reviewed, skipped, results = [], [], [], []
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
        action = "planning" if args.dry_run else "reviewing"
        print(f"{action} {name} ...")
        res = review_pair(
            name,
            args.model,
            args.rust,
            args.self_bin,
            args.chunk_bytes,
            args.timeout,
            max_chunks=args.max_chunks_per_pair,
            dry_run=args.dry_run,
        )
        res["content_hash"] = digest
        results.append(res)
        planned.append(name)
        if not args.dry_run:
            reviewed.append(name)

    confirmed = [
        (r["pair"], f)
        for r in results
        for f in r["findings"]
        if f.get("verdict") == "confirmed"
    ]
    hypotheses = [
        (r["pair"], f)
        for r in results
        for f in r["findings"]
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
                "dry_run": args.dry_run,
                "planned": planned,
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
    print(f"  planned   : {len(planned)} {planned}")
    print(f"  reviewed  : {len(reviewed)} {reviewed}")
    # Report what was not looked at. A run that skipped everything must never
    # read as a clean bill of health.
    if skipped:
        print(f"  unchanged : {len(skipped)} (not re-reviewed)")
        for n, when in skipped:
            print(f"      {n} — last reviewed {when}")
    deferred = [
        (r["pair"], r["chunks_deferred"], r["coverage"])
        for r in results
        if r["chunks_deferred"]
    ]
    if deferred:
        print("  deferred  : review covered only part of these pairs")
        for pair, chunks, coverage in deferred:
            print(f"      {pair} — chunks {chunks}; coverage {coverage:.1%}")
    oversize = [
        (r["pair"], chunk["index"], chunk["oversize_units"])
        for r in results
        for chunk in r["plan"]["chunks"]
        if chunk["oversize_units"]
    ]
    if oversize:
        print("  oversize  : complete units retained above the byte budget")
        for pair, index, units in oversize:
            print(f"      {pair} chunk {index} — {units}")
    errored = [r["pair"] for r in results if r["errors"]]
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
