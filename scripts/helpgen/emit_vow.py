"""Emit the generated help and skill helpers for the self-hosted main.vow."""

import hashlib
import re
from pathlib import Path

from helpgen.markers import _replace_between_markers

# Upper bound, in bytes, for one emitted payload literal. Whole lines are packed
# into a chunk up to this size, so chunking never produces a literal longer than
# max(this cap, the longest source line). The verifier sizes its C string
# buffers by the longest literal in the module; chunks therefore never raise it.
VOW_LITERAL_CHUNK_BYTES = 2048


def _vow_escape(chunk: str) -> str:
    return chunk.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")


def _vow_payload_fn(name: str, text: str) -> str:
    """Emit `fn name() -> String` building `text` from chunked string literals.

    The payload is `text` (em dashes become `--`) plus a trailing newline, packed
    into whole-line chunks of at most VOW_LITERAL_CHUNK_BYTES (a longer line is a
    chunk of its own).
    """
    chunks: list[str] = []
    current = ""
    for line in text.replace("\u2014", "--").split("\n"):
        line += "\n"
        if current and len((current + line).encode()) > VOW_LITERAL_CHUNK_BYTES:
            chunks.append(current)
            current = ""
        current += line
    chunks.append(current)
    body = [f'    let r: String = String::from("{_vow_escape(chunks[0])}");']
    body += [
        f'    r.push_str(String::from("{_vow_escape(chunk)}"));' for chunk in chunks[1:]
    ]
    return "\n".join([f"fn {name}() -> String {{", *body, "    r", "}"])


def inject_vow(main_vow: Path, json_str: str, human_str: str) -> str:
    content = main_vow.read_text()

    json_fn = (
        "// GENERATE:SKILL_JSON:START\n"
        + _vow_payload_fn("skill_json", json_str)
        + "\n// GENERATE:SKILL_JSON:END"
    )
    content = _replace_between_markers(
        content,
        "// GENERATE:SKILL_JSON:START",
        "// GENERATE:SKILL_JSON:END",
        json_fn,
    )

    human_fn = (
        "// GENERATE:SKILL_HUMAN:START\n"
        + _vow_payload_fn("skill_human", human_str)
        + "\n// GENERATE:SKILL_HUMAN:END"
    )
    content = _replace_between_markers(
        content,
        "// GENERATE:SKILL_HUMAN:START",
        "// GENERATE:SKILL_HUMAN:END",
        human_fn,
    )

    return content


def _vow_skill_support_content_fn_name(path: str) -> str:
    stem = re.sub(r"[^A-Za-z0-9]+", "_", path).strip("_").lower()
    if not stem:
        stem = "file"
    digest = hashlib.sha256(path.encode("utf-8")).hexdigest()[:8]
    return f"skill_support_content_{stem}_{digest}"


def inject_skill_vow(
    content: str, entrypoint_md: str, bundle_md: str, support_files: dict[str, str]
) -> str:
    """Inject generated skill helpers into self-hosted main.vow."""
    support_entries = [
        (path, body, _vow_skill_support_content_fn_name(path))
        for path, body in support_files.items()
    ]

    sections = [
        "// GENERATE:SKILL_FULL:START",
        _vow_payload_fn("skill_entrypoint", entrypoint_md),
        "",
        _vow_payload_fn("skill_bundle", bundle_md),
        "",
        "fn skill_support_count() -> u64 {",
        f"    {len(support_entries)}",
        "}",
        "",
        "fn skill_support_path(index: u64) -> String vow {",
        "    requires: index < skill_support_count()",
        "} {",
    ]
    for idx, (path, _, _) in enumerate(support_entries):
        escaped = _vow_escape(path)
        sections.append(
            f'    if index == {idx} {{ return String::from("{escaped}"); }}'
        )
    sections.extend(['    String::from("")', "}", ""])

    for _, body, fn_name in support_entries:
        sections.extend([_vow_payload_fn(fn_name, body), ""])

    sections.extend(
        [
            "fn skill_support_content_index_guard(index: u64) vow {",
            "    requires: index < skill_support_count()",
            "} {",
            "    let _ok: i64 = 0;",
            "}",
            "",
            "fn skill_support_content(index: u64) -> String {",
            '    if index >= skill_support_count() { return String::from(""); }',
            "    skill_support_content_index_guard(index);",
        ]
    )
    for idx, (_, _, fn_name) in enumerate(support_entries):
        sections.append(f"    if index == {idx} {{ return {fn_name}(); }}")
    sections.extend(['    String::from("")', "}", "// GENERATE:SKILL_FULL:END"])

    fn_body = "\n".join(sections)
    return _replace_between_markers(
        content,
        "// GENERATE:SKILL_FULL:START",
        "// GENERATE:SKILL_FULL:END",
        fn_body,
    )
