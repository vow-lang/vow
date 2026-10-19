#!/usr/bin/env python3
"""Assemble the large spec documents from their authored parts.

docs/spec/<doc>.md for every directory docs/spec/src/<doc>/ is the byte-exact
concatenation of that directory's parts in file-name order. Edit the parts; the
assembled file is generated, committed and exempt from the authored-file budget
(`token-limit-ignore` in .gitattributes), so every consumer keeps reading the
same path and embeds the same bytes.

A part may begin with a blank line (a cut falls before the blank that separated
two sections) but never ends with one, which keeps the whitespace hooks from
rewriting a part and silently changing the assembled file.

Usage:
    python3 scripts/spec_assemble.py                  # rewrite assembled files
    python3 scripts/spec_assemble.py --check          # fail if one is stale
    python3 scripts/spec_assemble.py --import         # port edits made to an
                                                      # assembled file into parts
    python3 scripts/spec_assemble.py --init DOC       # split docs/spec/DOC.md
    python3 scripts/spec_assemble.py --split PART     # halve an oversized part
"""

from __future__ import annotations

import argparse
import difflib
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SPEC = REPO / "docs" / "spec"
MAX_PART_BYTES = 20_000
TARGET_PART_BYTES = 12_000
MIN_PART_BYTES = 5_000
NUMBER_STEP = 10


def src_dir(spec: Path) -> Path:
    return spec / "src"


def doc_names(spec: Path = SPEC) -> list[str]:
    root = src_dir(spec)
    if not root.is_dir():
        return []
    return sorted(p.name for p in root.iterdir() if p.is_dir())


def part_paths(doc: str, spec: Path = SPEC) -> list[Path]:
    return sorted((src_dir(spec) / doc).glob("*.md"))


def read_text(path: Path) -> str:
    return path.read_bytes().decode("utf-8")


def write_text(path: Path, text: str) -> None:
    path.write_bytes(text.encode("utf-8"))


def assemble(doc: str, spec: Path = SPEC) -> str:
    return "".join(read_text(p) for p in part_paths(doc, spec))


def oversized_parts(doc: str, spec: Path = SPEC) -> list[Path]:
    return [p for p in part_paths(doc, spec) if p.stat().st_size > MAX_PART_BYTES]


def write_all(spec: Path = SPEC) -> list[str]:
    """Rewrite each stale assembled file and return the documents it changed."""
    changed = []
    for doc in doc_names(spec):
        target = spec / f"{doc}.md"
        text = assemble(doc, spec)
        if not target.exists() or read_text(target) != text:
            write_text(target, text)
            changed.append(doc)
    return changed


def check(spec: Path = SPEC) -> list[str]:
    """Return one message per stale assembled file and per oversized part."""
    problems = []
    for doc in doc_names(spec):
        target = spec / f"{doc}.md"
        if not target.exists() or read_text(target) != assemble(doc, spec):
            problems.append(
                f"docs/spec/{doc}.md is stale: edit docs/spec/src/{doc}/*.md and run "
                "python3 scripts/spec_assemble.py (python3 scripts/spec_assemble.py "
                "--import ports edits already made to the assembled file)"
            )
        for part in oversized_parts(doc, spec):
            problems.append(
                f"{part.relative_to(spec.parent.parent)} is over {MAX_PART_BYTES:,} "
                f"bytes: python3 scripts/spec_assemble.py --split {part}"
            )
    return problems


def import_doc(doc: str, spec: Path = SPEC) -> bool:
    """Apply a line diff of the assembled file back onto its parts."""
    paths = part_paths(doc, spec)
    parts = [read_text(p).splitlines(keepends=True) for p in paths]
    old = [line for part in parts for line in part]
    owner = [i for i, part in enumerate(parts) for _ in part]
    starts = [sum(len(p) for p in parts[:i]) for i in range(len(parts))]
    new_text = read_text(spec / f"{doc}.md")
    new = new_text.splitlines(keepends=True)
    if old == new:
        return False
    matcher = difflib.SequenceMatcher(None, old, new, autojunk=False)
    for tag, i1, i2, j1, j2 in reversed(matcher.get_opcodes()):
        if tag == "equal":
            continue
        if i1 == i2:
            # Pure insertion: it extends the part holding the preceding line.
            first = owner[i1 - 1] if i1 > 0 else 0
            offset = i1 - starts[first]
            parts[first][offset:offset] = new[j1:j2]
            continue
        first = owner[i1]
        offset = i1 - starts[first]
        for p in sorted({owner[i] for i in range(i1, i2)}, reverse=True):
            lo = max(i1, starts[p]) - starts[p]
            hi = min(i2, starts[p] + len(parts[p])) - starts[p]
            del parts[p][lo:hi]
        parts[first][offset:offset] = new[j1:j2]
    for path, lines in zip(paths, parts):
        write_text(path, "".join(lines))
    if assemble(doc, spec) != new_text:
        raise ValueError(f"could not port docs/spec/{doc}.md into its parts")
    return True


def slug(lines: list[str]) -> str:
    """Name a part after its first level-2 heading, else its first heading."""
    headings = [m for m in (re.match(r"(#{1,6})\s+(.*)", x) for x in lines) if m]
    if not headings:
        return "continued"
    chosen = next((m for m in headings if len(m.group(1)) == 2), headings[0])
    words = re.sub(r"[^a-z0-9]+", "-", chosen.group(2).lower()).strip("-").split("-")
    name = ""
    for word in words:
        if len(name) + len(word) + 1 > 36:
            break
        name = f"{name}-{word}" if name else word
    return name or "section"


def line_bytes(lines: list[str]) -> int:
    return sum(len(line.encode("utf-8")) for line in lines)


def fence_flags(lines: list[str]) -> list[bool]:
    """For each line, whether it sits inside a fenced code block."""
    flags, inside = [], False
    for line in lines:
        if line.lstrip().startswith("```"):
            inside = not inside
        flags.append(inside)
    return flags


def pick_cut(lines: list[str], start: int, target: int) -> int:
    """Index where the next part begins; lines[start:cut] fits the byte target."""
    fenced = fence_flags(lines)
    best = {"heading": None, "blank": None, "any": None}
    size = 0
    for i in range(start, len(lines) - 1):
        size += len(lines[i].encode("utf-8"))
        cut = i + 1
        if size > target and best["any"] is not None:
            break
        best["any"] = cut
        if lines[cut].strip() != "" or fenced[cut] or size < MIN_PART_BYTES:
            continue
        best["blank"] = cut
        following = next((x for x in lines[cut:] if x.strip()), "")
        if following.startswith("#"):
            best["heading"] = cut
    for kind in ("heading", "blank", "any"):
        if best[kind] is not None:
            return best[kind]
    return len(lines)


def split_lines(lines: list[str], target: int = TARGET_PART_BYTES) -> list[list[str]]:
    parts, start = [], 0
    while line_bytes(lines[start:]) > MAX_PART_BYTES:
        cut = pick_cut(lines, start, target)
        parts.append(lines[start:cut])
        start = cut
    parts.append(lines[start:])
    return parts


def part_name(number: int, lines: list[str], previous: str, taken=()) -> str:
    name = slug(lines)
    if name == "continued":
        name = re.sub(r"-part-\d+$", "", previous)
    unique, count = name, 2
    while unique in taken:
        unique, count = f"{name}-part-{count}", count + 1
    return f"{number:03d}-{unique}.md"


def init_doc(doc: str, spec: Path = SPEC) -> list[Path]:
    """Split docs/spec/<doc>.md into numbered parts under docs/spec/src/<doc>/."""
    source = spec / f"{doc}.md"
    target = src_dir(spec) / doc
    if target.exists() and any(target.iterdir()):
        raise ValueError(f"{target} already holds parts")
    lines = read_text(source).splitlines(keepends=True)
    target.mkdir(parents=True, exist_ok=True)
    written, previous = [], doc
    for index, part in enumerate(split_lines(lines), start=1):
        taken = [p.name[4:-3] for p in written]
        name = part_name(index * NUMBER_STEP, part, previous, taken)
        previous = name[4:-3]
        path = target / name
        write_text(path, "".join(part))
        written.append(path)
    return written


def split_part(path: Path) -> list[Path]:
    """Halve one part in place, numbering the second half between its neighbours."""
    siblings = sorted(path.parent.glob("*.md"))
    index = siblings.index(path)
    lines = read_text(path).splitlines(keepends=True)
    cut = pick_cut(lines, 0, line_bytes(lines) // 2)
    if cut >= len(lines):
        raise ValueError(f"{path} has no line boundary to split at")
    number = int(path.name[:3])
    later = (
        int(siblings[index + 1].name[:3]) if index + 1 < len(siblings) else number + 20
    )
    new_number = (number + later) // 2
    if new_number <= number:
        raise ValueError("no free part number; run --renumber on the document")
    taken = [p.name[4:-3] for p in siblings]
    second = path.parent / part_name(new_number, lines[cut:], path.stem[4:], taken)
    write_text(path, "".join(lines[:cut]))
    write_text(second, "".join(lines[cut:]))
    return [path, second]


def renumber(doc: str, spec: Path = SPEC) -> None:
    """Renumber a document's parts in steps of ten, keeping their names."""
    paths = part_paths(doc, spec)
    for path in paths:
        path.rename(path.with_name("tmp-" + path.name))
    if not paths:
        return
    staged = sorted(paths[0].parent.glob("tmp-*.md"))
    for index, path in enumerate(staged, start=1):
        path.rename(path.with_name(f"{index * NUMBER_STEP:03d}-{path.name[8:]}"))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--check", action="store_true", help="fail if anything is stale")
    group.add_argument("--import", dest="import_", action="store_true")
    group.add_argument("--init", metavar="DOC")
    group.add_argument("--split", metavar="PART")
    group.add_argument("--renumber", metavar="DOC")
    args = parser.parse_args(argv)
    try:
        if args.check:
            problems = check()
            for problem in problems:
                print(problem, file=sys.stderr)
            return 1 if problems else 0
        if args.import_:
            for doc in doc_names():
                if import_doc(doc):
                    print(f"Ported docs/spec/{doc}.md into docs/spec/src/{doc}/")
        elif args.init:
            for path in init_doc(args.init):
                print(f"Wrote {path.relative_to(REPO)}")
        elif args.split:
            for path in split_part(Path(args.split).resolve()):
                print(f"Wrote {path.relative_to(REPO)}")
        elif args.renumber:
            renumber(args.renumber)
        for doc in write_all():
            print(f"Updated docs/spec/{doc}.md")
    except ValueError as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
