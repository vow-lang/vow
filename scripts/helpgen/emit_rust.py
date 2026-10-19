"""Emit the generated help and skill helpers for the Rust vow/src/skill.rs."""

from pathlib import Path

from helpgen.markers import _replace_between_markers


def inject_rust(skill_rs: Path, json_str: str, human_str: str) -> str:
    content = skill_rs.read_text()

    content = _replace_between_markers(
        content,
        "// GENERATE:SKILL_JSON:START",
        "// GENERATE:SKILL_JSON:END",
        f'// GENERATE:SKILL_JSON:START\nfn skill_json() -> String {{\n    r##"{json_str}"##\n    .to_string()\n}}\n// GENERATE:SKILL_JSON:END',
    )

    content = _replace_between_markers(
        content,
        "// GENERATE:SKILL_HUMAN:START",
        "// GENERATE:SKILL_HUMAN:END",
        f'// GENERATE:SKILL_HUMAN:START\nfn skill_human() -> String {{\n    r##"{human_str}"##\n        .to_string()\n}}\n// GENERATE:SKILL_HUMAN:END',
    )

    return content


def _rust_raw_string(text: str) -> str:
    for n in range(1, 16):
        hashes = "#" * n
        if f'"{hashes}' not in text:
            return f'r{hashes}"{text}"{hashes}'
    raise ValueError("could not find Rust raw string delimiter")


def inject_skill_rust(
    content: str, entrypoint_md: str, bundle_md: str, support_files: dict[str, str]
) -> str:
    """Inject generated skill helpers into Rust skill.rs."""
    # Emit each tuple in rustfmt's preferred multi-line form (with trailing
    # comma after the last entry) so the GENERATE block stays clean under
    # `cargo fmt --check`.
    entries = "".join(
        (
            "(\n"
            f"            {_rust_raw_string(path)},\n"
            f"            {_rust_raw_string(body)},\n"
            "        ),\n        "
        )
        for path, body in support_files.items()
    ).rstrip()
    replacement = f"""// GENERATE:SKILL_FULL:START
fn skill_entrypoint_markdown() -> String {{
    {_rust_raw_string(entrypoint_md)}
    .to_string()
}}

fn skill_bundle_markdown() -> String {{
    {_rust_raw_string(bundle_md)}
    .to_string()
}}

fn skill_support_files() -> &'static [(&'static str, &'static str)] {{
    &[
        {entries}
    ]
}}
// GENERATE:SKILL_FULL:END"""
    return _replace_between_markers(
        content,
        "// GENERATE:SKILL_FULL:START",
        "// GENERATE:SKILL_FULL:END",
        replacement,
    )
