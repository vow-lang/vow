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
import os
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from functools import lru_cache
from pathlib import Path

from verifier_runtime import check_soundness

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

CFG_TEST = re.compile(r"^#\[cfg\(test\)\][ \t]*$", re.MULTILINE)
CHAR_LIT = re.compile(r"'(?:\\.|[^\\'])'")


def _item_end(text, start):
    """Index just past the brace-balanced Rust item beginning at `start`.

    Brace counting alone is wrong here: the test modules embed Vow programs in
    raw strings, and those contain unbalanced braces at column 0. So comments,
    strings, char literals and raw strings are skipped before any brace counts.
    """
    index, depth, saw_brace = start, 0, False
    end = len(text)
    while index < end:
        char = text[index]
        if text.startswith("//", index):
            newline = text.find("\n", index)
            index = end if newline == -1 else newline + 1
            continue
        if text.startswith("/*", index):
            close = text.find("*/", index + 2)
            index = end if close == -1 else close + 2
            continue
        if char == "r" and index + 1 < end and text[index + 1] in '#"':
            cursor = index + 1
            hashes = 0
            while cursor < end and text[cursor] == "#":
                hashes += 1
                cursor += 1
            if cursor < end and text[cursor] == '"':
                terminator = '"' + "#" * hashes
                close = text.find(terminator, cursor + 1)
                index = end if close == -1 else close + len(terminator)
                continue
        if char == '"':
            index += 1
            while index < end and text[index] != '"':
                index += 2 if text[index] == "\\" else 1
            index += 1
            continue
        if char == "'":
            literal = CHAR_LIT.match(text, index)
            index += len(literal.group()) if literal else 1
            continue
        if char == "{":
            depth += 1
            saw_brace = True
        elif char == "}":
            depth -= 1
            if saw_brace and depth == 0:
                return index + 1
        elif char == ";" and not saw_brace and depth == 0:
            return index + 1
        index += 1
    return end


def strip_cfg_test(text):
    """Drop `#[cfg(test)]` items so the review budget buys implementation code.

    Two thirds of the Rust units in the declared pairs are test functions, and
    the self-hosted side has no counterpart for any of them. Reviewing them
    costs model calls and inflates the coverage figure that stamps the ledger.
    """
    kept, position = [], 0
    for marker in CFG_TEST.finditer(text):
        if marker.start() < position:
            continue
        kept.append(text[position : marker.start()])
        position = _item_end(text, marker.end())
    kept.append(text[position:])
    return "".join(kept)


@dataclass(frozen=True)
class Unit:
    name: str
    text: str
    source: str = ""

    @property
    def label(self):
        return f"{self.source}:{self.name}"


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


def _load_units(files, pattern, strip_tests=False):
    preambles = []
    units = []
    for path in files:
        relative = str(path.relative_to(REPO_ROOT))
        text = path.read_text(errors="replace")
        if strip_tests:
            text = strip_cfg_test(text)
        preamble, source_units = split_units(text, pattern)
        preambles.append((relative, preamble))
        units.extend(Unit(unit.name, unit.text, relative) for unit in source_units)
    return tuple(preambles), units


def load_pair_units(name):
    """Load one declared module pair as preambles and function units.

    Rust test items are dropped from the review, but `hash_pair` still digests
    whole files: a test-only edit therefore forces a re-review it cannot change
    the content of. That is the safe direction to err in.
    """
    rust_paths, self_path = PAIRS[name]
    rust_preambles, rust_units = _load_units(
        _source_files(rust_paths, ".rs"), RUST_FN, strip_tests=True
    )
    self_preambles, self_units = _load_units(_source_files([self_path], ".vow"), VOW_FN)
    return Preambles(rust_preambles, self_preambles), rust_units, self_units


JOIN = "\n\n"


def _unit_part(unit, kind):
    return f"=== {kind}: {unit.source} (function {unit.name}) ===\n{unit.text}"


def _chunk_parts(chunk, preambles, index, total):
    """The rendered pieces of a chunk, in order, before they are joined.

    `plan_chunks` sizes candidate chunks by summing these rather than rendering
    them, so both paths must read the layout from here.
    """
    return [
        f"=== REVIEW CHUNK {index} OF {total} ===\n",
        *(f"=== RUST PREAMBLE: {src} ===\n{txt}" for src, txt in preambles.rust),
        *(_unit_part(unit, "RUST") for unit in chunk.rust_units),
        *(
            f"=== SELF-HOSTED PREAMBLE: {src} ===\n{txt}"
            for src, txt in preambles.self_hosted
        ),
        *(_unit_part(unit, "SELF-HOSTED") for unit in chunk.self_units),
    ]


def render_chunk(chunk, preambles=None, index=1, total=1):
    """Render a review chunk with enough source context to stand alone."""
    return JOIN.join(_chunk_parts(chunk, preambles or Preambles(), index, total))


def plan_chunks(rust_units, self_units, chunk_bytes, preambles=None):
    """Pair related functions and greedily pack complete units into prompts."""
    if chunk_bytes <= 0:
        raise ValueError("chunk_bytes must be positive")
    preambles = preambles or Preambles()
    chunks = []
    current = Chunk()

    # A deliberately wide placeholder keeps the final `i of n` header from
    # pushing a planned chunk over its byte budget.
    fixed = _chunk_parts(Chunk(), preambles, 999_999, 999_999)
    join_bytes = len(JOIN.encode())
    base_bytes = sum(len(part.encode()) for part in fixed) + join_bytes * (
        len(fixed) - 1
    )
    part_bytes = {}

    def part_size(unit, kind):
        # Each unit contributes one part plus the join before it, and a unit's
        # part never varies, so sizing a candidate is arithmetic rather than a
        # re-render of everything already packed.
        key = (kind, unit)
        if key not in part_bytes:
            part_bytes[key] = len(_unit_part(unit, kind).encode()) + join_bytes
        return part_bytes[key]

    def rendered_size(chunk):
        return (
            base_bytes
            + sum(part_size(u, "RUST") for u in chunk.rust_units)
            + sum(part_size(u, "SELF-HOSTED") for u in chunk.self_units)
        )

    def finish():
        nonlocal current
        if current.rust_units or current.self_units:
            chunks.append(current)
            current = Chunk()

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
        # A matched group is the unit of comparison. Splitting it hands the
        # model one implementation with no counterpart, which is not a pair
        # review at all -- so an over-budget group is flagged, never split.
        finish()
        current = Chunk(rust_units=[*group_rust], self_units=[*group_self])
        if rendered_size(current) > chunk_bytes:
            current.oversize_units = [u.label for u in (*group_rust, *group_self)]
            finish()

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
            add_group([rust_unit], [])
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

SYSTEM_SOUNDNESS = """\
You are auditing the Rust and self-hosted C emitters for the Vow verifier. The \
question is model-vs-language soundness: does either emitter add an \
`__ESBMC_assume` that narrows the verifier's model below what the Vow language \
actually permits?

Find concrete programs that `vow verify` proves but whose permitted concrete \
execution violates a vow. A false `Verified` is the only finding class in this \
mode. Do not report false negatives, precision limitations, stylistic \
differences, or a suspicious assumption without a discriminating program. The \
runner will confirm each claim by requiring both `Verified` and a debug-mode \
`VowViolation`, using both compilers independently.

Every program must be complete and start with a `module M` declaration.

Reply with JSON only, no prose outside it:

{"findings": [
  {"claim": "one sentence: how the verifier model is too narrow",
   "area": "short label",
   "program": "module M\\nfn main() -> i32 [io] { ... }\\n",
   "expected_verifier": "why verification incorrectly succeeds",
   "expected_runtime": "which permitted execution violates which vow"}
]}

An empty findings list is a perfectly good answer if you find nothing \
demonstrable."""


def hash_pair(rust_paths, self_path):
    h = hashlib.sha256()
    inner = hashlib.sha256()
    for f in _source_files(rust_paths, ".rs"):
        inner.update(str(f.relative_to(REPO_ROOT)).encode())
        inner.update(f.read_bytes())
    h.update(inner.hexdigest().encode())
    h.update((REPO_ROOT / self_path).read_bytes())
    return h.hexdigest()


# Every one-sided fail_closed detail scripts/equivalence.py writes opens with
# the side it blames -- a panicking or signal-killed compiler, a compiler that
# emitted no parseable JSON, and a generated binary killed by a memory-unsafety
# signal. Anchoring on the side alone rather than on the words that follow it
# keeps a new shape from silently reading as "no side", which is the reading
# that lets an input both implementations choked on pass the gate.
FAIL_CLOSED_SIDE = re.compile(r"^(rust|self-hosted)\b")


def _agreed_by_crashing(divergences):
    """Whether the only evidence is that both implementations failed closed.

    scripts/equivalence.py files a panic, a signal death or a missing-JSON
    contract breach once per side, because each is a bug whatever the peer did.
    Here it is not evidence that the two implementations *disagree*, which is
    the only claim a model may have confirmed -- otherwise any input that kills
    both compilers, or both binaries, passes the gate.

    A shape naming one side only ("rust compiler timed out ...; self-hosted
    completed") is left as a divergence: that IS the two implementations
    behaving differently.
    """
    if any(v["observable"] != "fail_closed" for v in divergences):
        return False
    sides = set()
    for divergence in divergences:
        side = FAIL_CLOSED_SIDE.match(divergence["detail"])
        if side:
            sides.add(side.group(1))
        elif divergence["detail"].startswith("both "):
            return True
    return sides == {"rust", "self-hosted"}


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
        divergences = rec["divergences"]
        if divergences:
            detail = "; ".join(
                f"[{v['observable']}] {v['detail']}" for v in divergences
            )
            if _agreed_by_crashing(divergences):
                return "inconclusive", f"both compilers failed closed: {detail}"
            return "confirmed", detail
        if rec.get("skipped"):
            return "inconclusive", rec["skipped"]
        return "refuted", "both compilers agreed"


def confirm_soundness(program, verifier, timeout):
    """Judge a candidate with the verifier-vs-debug-runtime soundness gate."""
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source = root / "candidate.vow"
        source.write_text(program)
        result = check_soundness(source, Path(verifier), root, timeout)
    verdict = result.get("verdict")
    if verdict == "SOUNDNESS":
        return "confirmed", result.get("detail", "soundness divergence")
    if verdict == "ok":
        return "refuted", result.get("detail", "runtime agrees with proof")
    return "inconclusive", result.get("detail", verdict or "no verdict")


def confirm_soundness_pair(program, rust, self_bin, timeout):
    """Run the model-vs-runtime gate independently for both C emitters."""
    rust_result = confirm_soundness(program, rust, timeout)
    self_result = confirm_soundness(program, self_bin, timeout)
    observed = (
        f"rust: {rust_result[0]} ({rust_result[1]}); "
        f"self-hosted: {self_result[0]} ({self_result[1]})"
    )
    verdicts = {rust_result[0], self_result[0]}
    if "confirmed" in verdicts:
        return "confirmed", observed
    if verdicts == {"refuted"}:
        return "refuted", observed
    return "inconclusive", observed


@dataclass(frozen=True)
class Mode:
    """Everything that differs between the two questions this harness asks."""

    system: str
    confirm: object
    pairs: tuple
    # Only a mode that stamps the ledger may be skipped by it. A mode that
    # reads these hashes without writing them would skip every pair another
    # mode had stamped and exit 0 having asked nothing.
    uses_ledger: bool


MODES = {
    "equivalence": Mode(SYSTEM, confirm, tuple(PAIRS), uses_ledger=True),
    "soundness": Mode(
        SYSTEM_SOUNDNESS, confirm_soundness_pair, ("c_emitter",), uses_ledger=False
    ),
}


def _plan_record(chunks, preambles):
    total = len(chunks)
    records = []
    for index, chunk in enumerate(chunks, 1):
        records.append(
            {
                "index": index,
                "bytes": len(render_chunk(chunk, preambles, index, total).encode()),
                "rust_units": [u.label for u in chunk.rust_units],
                "self_hosted_units": [u.label for u in chunk.self_units],
                "oversize_units": chunk.oversize_units,
            }
        )
    return records


def _unit_bytes(chunks):
    return sum(
        len(unit.text.encode())
        for chunk in chunks
        for unit in (*chunk.rust_units, *chunk.self_units)
    )


def _paired_coverage(chunks, selected):
    """Share of unit bytes a model saw with both implementations in front of it.

    `coverage` answers "was every byte shipped?". This answers "was it shipped
    as a comparison?" -- a run whose tail chunks carry one side only examined
    that surface, it did not compare it.
    """
    total = _unit_bytes(chunks)
    if total == 0:
        return 1.0
    paired = [c for c in selected if c.rust_units and c.self_units]
    return _unit_bytes(paired) / total


def _coverage(preambles, chunks, selected):
    """Share of source bytes a model saw at all, preambles included.

    `plan_chunks` places every unit in exactly one chunk, so `chunks` is the
    whole corpus -- measuring both sides of the ratio through `_unit_bytes`
    keeps this metric and `paired_coverage` counting the same bytes.
    """
    preamble_bytes = sum(
        len(text.encode()) for _, text in (*preambles.rust, *preambles.self_hosted)
    )
    total = preamble_bytes + _unit_bytes(chunks)
    if total == 0:
        return 1.0
    reviewed = (preamble_bytes if selected else 0) + _unit_bytes(selected)
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
    mode="equivalence",
):
    """Plan and, unless dry-running, review every selected function chunk."""
    if mode not in MODES:
        raise ValueError(f"unknown review mode: {mode}")
    preambles, rust_units, self_units = load_pair_units(name)
    chunks = plan_chunks(rust_units, self_units, chunk_bytes, preambles)
    selected_count = min(max_chunks, len(chunks)) if max_chunks else len(chunks)
    selected = chunks[:selected_count]
    result = {
        "pair": name,
        "mode": mode,
        "coverage": _coverage(preambles, chunks, selected),
        "paired_coverage": _paired_coverage(chunks, selected),
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

    spec = MODES[mode]
    confirm_fn = confirm_fn or spec.confirm
    config = llm_module.make_config(model)
    for index, chunk in enumerate(selected, 1):
        body = render_chunk(chunk, preambles, index, len(chunks))
        try:
            response = llm_module.chat(
                config,
                spec.system,
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
        except (AttributeError, ValueError):
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


@lru_cache(maxsize=1)
def _pair_schema():
    schema = json.loads(
        (REPO_ROOT / "docs" / "equivalence" / "ledger.schema.json").read_text()
    )
    return schema["properties"]["pairs"]["additionalProperties"]


def _validate_pair_entry(entry):
    pair_schema = _pair_schema()
    keys = set(entry)
    missing = set(pair_schema["required"]) - keys
    unknown = keys - set(pair_schema["properties"])
    if missing or unknown:
        raise ValueError(
            f"invalid ledger pair entry: missing={sorted(missing)}, "
            f"unknown={sorted(unknown)}"
        )


def _ledger_outcome(findings):
    verdicts = {finding.get("verdict") for finding in findings}
    if "confirmed" in verdicts:
        return "confirmed"
    if "inconclusive" in verdicts:
        return "hypotheses"
    return "clean"


def write_ledger(ledger, results, date, path=LEDGER):
    """Atomically stamp only fully reviewed pair rows in the shared ledger.

    The rows are merged into the ledger as it stands *now*, not into the copy
    loaded before the model calls: a review runs for minutes, and triage edits
    issue numbers and corpus rows in the same file meanwhile.
    """
    path = Path(path)
    if path.exists():
        ledger = json.loads(path.read_text())
    updated = []
    for result in results:
        complete = (
            result.get("coverage") == 1.0
            and not result.get("chunks_deferred")
            and not result.get("errors")
        )
        if not complete:
            continue
        name = result["pair"]
        if name not in ledger.get("pairs", {}):
            raise ValueError(f"ledger has no pair entry for {name}")
        entry = dict(ledger["pairs"][name])
        entry.update(
            {
                "content_hash": result["content_hash"],
                "last_reviewed": date,
                "outcome": _ledger_outcome(result.get("findings", [])),
            }
        )
        _validate_pair_entry(entry)
        ledger["pairs"][name] = entry
        updated.append(name)

    if not updated:
        return updated
    ledger["updated"] = date
    descriptor, temp_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    os.close(descriptor)
    temp_path = Path(temp_name)
    try:
        temp_path.write_text(json.dumps(ledger, indent=2) + "\n")
        os.replace(temp_path, path)
    finally:
        temp_path.unlink(missing_ok=True)
    return updated


def _print_summary(report, skipped, confirmed, outdir):
    """Print the run, leading with everything it did not look at.

    A run that skipped, deferred or errored out of most of the surface must
    never read as a clean bill of health.
    """
    results = report["pairs"]
    print()
    print("=== Pair review ===")
    print(f"  model     : {report['model']}")
    print(f"  mode      : {report['mode']}")
    print(f"  planned   : {len(report['planned'])} {report['planned']}")
    print(f"  reviewed  : {len(report['reviewed'])} {report['reviewed']}")
    if skipped:
        print(f"  unchanged : {len(skipped)} (not re-reviewed)")
        for name, when in skipped:
            print(f"      {name} — last reviewed {when}")
    deferred = [
        (r["pair"], r["chunks_deferred"], r["coverage"])
        for r in results
        if r["chunks_deferred"]
    ]
    if deferred:
        print("  deferred  : review covered only part of these pairs")
        for pair, chunks, coverage in deferred:
            print(f"      {pair} — chunks {chunks}; coverage {coverage:.1%}")
    unpaired = [
        (r["pair"], r["paired_coverage"])
        for r in results
        if r.get("paired_coverage", 1.0) < 1.0
    ]
    if unpaired:
        print("  unpaired  : bytes shown with only one implementation present")
        for pair, fraction in unpaired:
            print(f"      {pair} — {fraction:.1%} of unit bytes seen side by side")
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
    print(f"  CONFIRMED : {report['confirmed']}")
    print(
        f"  hypotheses: {report['hypotheses']} (unconfirmed — not counted as findings)"
    )
    print(f"  refuted   : {report['refuted']}")
    for pair, finding in confirmed:
        print(f"    [{pair}] {finding.get('claim', '?')}")
        print(f"            {finding['verdict_detail']}")
    print(f"  results   : {outdir / 'results.json'}")


def main(argv=None):
    ap = argparse.ArgumentParser(
        description="Adversarial pair review (#1083)", allow_abbrev=False
    )
    ap.add_argument("--model", default="claude-sonnet-4-20250514")
    ap.add_argument(
        "--mode",
        choices=tuple(MODES),
        default="equivalence",
        help="review compiler parity or verifier-model soundness",
    )
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
    ap.add_argument(
        "--update-ledger",
        action="store_true",
        help="stamp complete reviewed pairs in the equivalence ledger",
    )
    ap.add_argument(
        "--date",
        help="deterministic YYYY-MM-DD ledger date (required with --update-ledger)",
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
    out_of_mode = sorted(set(args.pair) - set(MODES[args.mode].pairs))
    if out_of_mode:
        ap.error(f"--mode {args.mode} does not cover pair(s): {', '.join(out_of_mode)}")
    if args.chunk_bytes <= 0:
        ap.error("--chunk-bytes must be positive")
    if args.max_chunks_per_pair < 0:
        ap.error("--max-chunks-per-pair cannot be negative")
    if args.update_ledger and not args.date:
        ap.error("--date is required with --update-ledger")
    if args.update_ledger and args.dry_run:
        ap.error("--update-ledger cannot be used with --dry-run")
    if args.update_ledger and not MODES[args.mode].uses_ledger:
        ap.error(f"{args.mode} results do not update the equivalence ledger")
    if args.date and not re.fullmatch(r"\d{4}-\d{2}-\d{2}", args.date):
        ap.error("--date must use YYYY-MM-DD")

    if not args.dry_run:
        for path in (Path(args.rust), Path(args.self_bin)):
            if not path.exists():
                print(f"error: compiler not found: {path}", file=sys.stderr)
                return 2

    ledger = json.loads(LEDGER.read_text()) if LEDGER.exists() else {"pairs": {}}
    names = args.pair or list(MODES[args.mode].pairs)
    outdir = Path(args.output_dir)
    outdir.mkdir(parents=True, exist_ok=True)

    planned, skipped, results = [], [], []
    for name in names:
        rust_paths, self_path = PAIRS[name]
        digest = hash_pair(rust_paths, self_path)
        prior = ledger.get("pairs", {}).get(name, {})
        if (
            MODES[args.mode].uses_ledger
            and not args.all
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
            mode=args.mode,
        )
        res["content_hash"] = digest
        results.append(res)
        planned.append(name)

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
    ledger_updated = []
    if args.update_ledger:
        ledger_updated = write_ledger(ledger, results, args.date, LEDGER)

    report = {
        "schema_version": 2,
        "model": args.model,
        "mode": args.mode,
        "dry_run": args.dry_run,
        "planned": planned,
        "reviewed": [] if args.dry_run else planned,
        "skipped_unchanged": [n for n, _ in skipped],
        "ledger_updated": ledger_updated,
        "confirmed": len(confirmed),
        "hypotheses": len(hypotheses),
        "refuted": refuted,
        "pairs": results,
    }
    (outdir / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    _print_summary(report, skipped, confirmed, outdir)

    return 1 if confirmed else 0


if __name__ == "__main__":
    sys.exit(main())
