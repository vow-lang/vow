"""The catalogue `excluded` section: loading and matching."""

from __future__ import annotations

import json
import re
from pathlib import Path

EXCLUDED_NAME_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
EXCLUDED_FRAGMENT_RE = re.compile(r"^[A-Za-z0-9_]+$")
EXCLUDED_MATCHERS = ("names", "prefix", "contains")
EXCLUDED_FIELDS = {"reason", "suffixes", *EXCLUDED_MATCHERS}


def excluded_label(index: int, entry: dict) -> str:
    """How a violation names an excluded entry: its position and its matcher."""
    for key in EXCLUDED_MATCHERS:
        if key in entry:
            return f"excluded entry #{index} ({key} {json.dumps(entry[key])})"
    return f"excluded entry #{index}"


def excluded_matches(entry: dict, name: str) -> bool:
    """Whether a validated `excluded` entry covers a builtin name. The matcher
    vocabulary is deliberately closed (explicit names, a prefix, or a substring
    plus a suffix set; no regex) so the Rust and Vow tests evaluate it
    identically."""
    if "names" in entry:
        return name in entry["names"]
    if "prefix" in entry:
        return name.startswith(entry["prefix"])
    return entry["contains"] in name and name.endswith(tuple(entry["suffixes"]))


def _check_excluded_matcher(label: str, entry: dict, errors: list[str]) -> None:
    present = [k for k in EXCLUDED_MATCHERS if k in entry]
    if len(present) != 1:
        errors.append(
            f"{label} must have exactly one matcher of {list(EXCLUDED_MATCHERS)}, "
            f"has {present}"
        )
        return
    kind = present[0]
    if kind != "contains" and "suffixes" in entry:
        errors.append(f"{label} has 'suffixes' but its matcher is '{kind}'")
    value = entry[kind]
    if kind == "names":
        if not isinstance(value, list) or not value:
            errors.append(f"{label} must list at least one name")
            return
        for name in value:
            if not isinstance(name, str) or not EXCLUDED_NAME_RE.match(name):
                errors.append(f"{label} has malformed name {name!r}")
        if len(set(map(str, value))) != len(value):
            errors.append(f"{label} lists a name more than once")
        return
    if not isinstance(value, str) or not EXCLUDED_FRAGMENT_RE.match(value):
        errors.append(f"{label} has malformed {kind} {value!r}")
    if kind == "contains":
        suffixes = entry.get("suffixes")
        if not isinstance(suffixes, list) or not suffixes:
            errors.append(f"{label} needs a non-empty 'suffixes' list with 'contains'")
        else:
            for suffix in suffixes:
                if not isinstance(suffix, str) or not EXCLUDED_FRAGMENT_RE.match(
                    suffix
                ):
                    errors.append(f"{label} has malformed suffix {suffix!r}")


def load_excluded(repo_root: Path, ops: list[dict]) -> list[dict]:
    """Read and validate the catalogue's `excluded` section: the builtin names
    the type-checker signature tables hold that the catalogue deliberately does
    not list, each with the reason. A catalogue without the section excludes
    nothing.

    Collects every violation before raising a ValueError, like load_catalogue:
    a missing reason, a malformed or ambiguous matcher, a name listed twice, and
    a name that is both catalogued and excluded.
    """
    catalogue_path = repo_root / "docs" / "spec" / "operations.json"
    entries = json.loads(catalogue_path.read_text()).get("excluded", [])
    if not isinstance(entries, list):
        raise ValueError(f"{catalogue_path}: 'excluded' key must be a list")

    errors: list[str] = []
    valid: list[tuple[int, dict]] = []
    seen_names: dict[str, int] = {}
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict):
            errors.append(f"excluded entry #{index} must be an object")
            continue
        label = excluded_label(index, entry)
        before = len(errors)
        unknown = set(entry) - EXCLUDED_FIELDS
        if unknown:
            errors.append(
                f"{label} has unknown field(s) {sorted(unknown)} "
                f"(allowed: {sorted(EXCLUDED_FIELDS)})"
            )
        reason = entry.get("reason")
        if not isinstance(reason, str) or not reason.strip():
            errors.append(f"{label} has no reason")
        _check_excluded_matcher(label, entry, errors)
        if len(errors) != before:
            continue
        valid.append((index, entry))
        for name in entry.get("names", []):
            if name in seen_names:
                errors.append(
                    f"{label} lists '{name}', already excluded by "
                    f"entry #{seen_names[name]}"
                )
            seen_names[name] = index

    for index, entry in valid:
        for op in ops:
            if excluded_matches(entry, op["name"]):
                errors.append(
                    f"operation '{op['name']}' is both catalogued and excluded by "
                    f"{excluded_label(index, entry)}"
                )

    if errors:
        raise ValueError("\n".join(errors))
    return [entry for _, entry in valid]
