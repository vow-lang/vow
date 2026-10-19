"""Cross-checks of the catalogue against grammar.md, the compiler docs and models."""

from __future__ import annotations

import re
from pathlib import Path

from opsgen.catalogue import VERIFIER_REASON_FIELD


def _split_table_row(line: str) -> list[str]:
    """Split a `| a | b |`-style row into cells.

    Only the empty strings produced by the row's own leading/trailing pipe
    are dropped -- a genuinely empty interior cell (`| a |  | c |`) is kept,
    so columns never silently shift.
    """
    placeholder = "\x00PIPE\x00"
    line = line.replace("\\|", placeholder)
    cells = [c.strip().replace(placeholder, "|") for c in line.split("|")]
    if cells and cells[0] == "":
        cells = cells[1:]
    if cells and cells[-1] == "":
        cells = cells[:-1]
    return cells


def extract_builtin_signatures_table(grammar_text: str) -> dict[str, tuple[str, str]]:
    """Map builtin name -> (signature, effects) from the whole level-3
    `### Builtin Function Signatures` section of grammar.md, sweeping every
    `####` subsection (Print / IO, Debug, Filesystem, ...) so any catalogued
    op can be looked up by name regardless of which subsection it lives in.

    Raises ValueError on a duplicate row for the same builtin name -- e.g. a
    stale row left behind in another subsection by a bad merge -- rather than
    silently keeping whichever row happens to be read last.
    """
    heading = "Builtin Function Signatures"
    heading_level = 3
    prefix = "#" * heading_level + " "
    in_section = False
    in_table = False
    table: dict[str, tuple[str, str]] = {}
    for line in grammar_text.splitlines():
        if line.startswith(prefix) and heading in line:
            in_section = True
            in_table = False
            continue
        if in_section and line.startswith("#"):
            level = len(line) - len(line.lstrip("#"))
            if level <= heading_level:
                break
            in_table = False
            continue
        if in_section and line.startswith("|") and "---" in line:
            in_table = True
            continue
        if in_section and in_table:
            if not line.startswith("|"):
                in_table = False
                continue
            cells = _split_table_row(line)
            cells = [re.sub(r"`([^`]*)`", r"\1", c) for c in cells]
            if len(cells) >= 3:
                if cells[0] in table:
                    raise ValueError(
                        f"duplicate Builtin Function Signatures row for '{cells[0]}'"
                    )
                table[cells[0]] = (cells[1], cells[2])
    return table


def check_verifier_models(ops: list[dict]) -> list[str]:
    """Drift check: every catalogued builtin must say whether the native
    verifier models it, and an effectful one cannot be `known`. A builtin
    added without a `verifier_model` would otherwise be indistinguishable
    from one nobody has decided on. Verifier-model categories apply only to
    pure operations (PRD #375), so an effectful `known` entry is rejected
    even though the native gate admits callers whose only effect is `panic`;
    modelling a `[panic]` builtin would be a deliberate change to this rule."""
    problems: list[str] = []
    for op in ops:
        name = op["name"]
        model = op.get("verifier_model")
        if model is None:
            problems.append(
                f"docs/spec/operations.json: '{name}' has no verifier_model "
                "(set 'known' or an explicit 'unmodeled')"
            )
        elif model == "known" and op["effects"] != "[]":
            problems.append(
                f"docs/spec/operations.json: '{name}' has effects {op['effects']} "
                f"but verifier_model '{model}'; the native verifier only models "
                "pure builtins"
            )
        reason = op.get(VERIFIER_REASON_FIELD)
        where = f"docs/spec/operations.json: '{name}'"
        pure_unmodeled = op["effects"] == "[]" and model == "unmodeled"
        if reason is not None and (not isinstance(reason, str) or not reason.strip()):
            problems.append(
                f"{where} has an empty or non-string {VERIFIER_REASON_FIELD}"
            )
        elif pure_unmodeled and reason is None:
            problems.append(
                f"{where} is a pure op with verifier_model 'unmodeled' but has no "
                f"{VERIFIER_REASON_FIELD} (say which semantics the native "
                "verifier does not model)"
            )
        elif reason is not None and not pure_unmodeled:
            problems.append(
                f"{where} has a {VERIFIER_REASON_FIELD} but is not a pure "
                "'unmodeled' op"
            )
    return problems


def check_c_models(ops: list[dict], routes: list[str]) -> list[str]:
    """Drift check for the ESBMC C-model category: every pure builtin must say
    whether the model knows it, an effectful one cannot be known (the model's
    gate takes only effect-free functions), and an `_in_arena` variant must be
    a real arena route."""
    problems: list[str] = []
    for op in ops:
        name = op["name"]
        model = op.get("c_model")
        if op["effects"] == "[]" and model is None:
            problems.append(
                f"docs/spec/operations.json: pure builtin '{name}' has no c_model "
                "(set 'base', 'base_and_arena' or an explicit 'unmodeled')"
            )
        if model in ("base", "base_and_arena") and op["effects"] != "[]":
            problems.append(
                f"docs/spec/operations.json: '{name}' has effects {op['effects']} "
                f"but c_model '{model}'; the C model only knows pure builtins"
            )
        if model == "base_and_arena" and op["runtime_symbol"] not in routes:
            problems.append(
                f"docs/spec/operations.json: '{name}' has c_model 'base_and_arena' "
                f"but '{op['runtime_symbol']}' has no arena route"
            )
    return problems


def _help_source(repo_root: Path, generated: str, legacy: str) -> tuple[str, str]:
    """The file holding a compiler's embedded help: the generated module, or the
    older host file when a tree predates the split."""
    rel = generated if (repo_root / generated).exists() else legacy
    return rel, (repo_root / rel).read_text()


def check_doc_facts(ops: list[dict], repo_root: Path) -> list[str]:
    """Cross-check each op's doc facts against grammar.md and the generated help
    text of both compilers. Collects every mismatch instead of failing on the first."""
    mismatches: list[str] = []

    grammar_text = (repo_root / "docs" / "spec" / "grammar.md").read_text()
    table = extract_builtin_signatures_table(grammar_text)
    vow_rel, main_vow_text = _help_source(
        repo_root, "compiler/help_text_gen.vow", "compiler/main.vow"
    )
    rs_rel, skill_rs_text = _help_source(
        repo_root, "vow/src/skill_gen/help_text.rs", "vow/src/skill.rs"
    )

    for op in ops:
        name = op["name"]
        sig = op["doc_signature"]
        eff = op["effects"]

        row = table.get(name)
        if row is None:
            mismatches.append(f"grammar.md: no row found for '{name}'")
        elif row != (sig, eff):
            mismatches.append(
                f"grammar.md: '{name}' row is {row}, catalogue says ({sig!r}, {eff!r})"
            )

        skill_rs_line = f'"{name}": "{sig} {eff}"'
        if skill_rs_line not in skill_rs_text:
            mismatches.append(f"{rs_rel}: missing doc line for '{name}'")

        escaped_sig = sig.replace("\\", "\\\\").replace('"', '\\"')
        main_vow_line = f'\\"{name}\\": \\"{escaped_sig} {eff}\\"'
        if main_vow_line not in main_vow_text:
            mismatches.append(f"{vow_rel}: missing doc line for '{name}'")

    return mismatches
