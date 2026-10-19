"""Markdown table extraction and option normalisation for the help JSON."""

import re


def _split_table_row(line: str) -> list[str]:
    """Split a markdown table row on unescaped pipes, handling \\| escapes."""
    # Replace escaped pipes with a placeholder, split, then restore
    placeholder = "\x00PIPE\x00"
    line = line.replace("\\|", placeholder)
    cells = [c.strip().replace(placeholder, "|") for c in line.split("|")]
    return [c for c in cells if c != ""]


def extract_table(
    text: str, heading: str, *, heading_level: int = 3
) -> list[list[str]]:
    """Extract rows from a markdown table under a heading.
    Returns list of rows, each row is a list of cell strings (backticks stripped)."""
    prefix = "#" * heading_level + " "
    in_section = False
    in_table = False
    rows: list[list[str]] = []
    for line in text.splitlines():
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
            rows.append(cells)
    return rows


def extract_table_col(text: str, heading: str, col: int = 0, **kw) -> list[str]:
    """Extract a single column from a markdown table."""
    return [row[col] for row in extract_table(text, heading, **kw) if col < len(row)]


def normalize_option(
    flag: str,
    default: str,
    desc: str,
    *,
    merge_mode: bool = False,
) -> tuple[str, str, dict]:
    """Normalize an option row for both legacy dict output and structured help."""
    default = default.strip("`")
    description = desc
    normalized_flag = flag

    if merge_mode:
        normalized_flag = "--mode <debug|release>"
        description = (
            f"Build mode; debug inserts runtime vow checks (default: {default})"
        )
    elif flag == "-o, --output":
        normalized_flag = "-o, --output <path>"
        if default not in ("", "(off)", "(default)"):
            description = f"{desc} (default: {default})"
    elif default not in ("", "(off)", "(default)") and not desc.endswith(")"):
        description = f"{desc} (default: {default})"

    option: dict[str, object] = {
        "form": normalized_flag,
        "description": description,
    }

    if normalized_flag.startswith("-o, --output"):
        option["short"] = "-o"
        option["long"] = "--output"
        option["value_name"] = "path"
        option["value_kind"] = "path"
    else:
        head, _, tail = normalized_flag.partition(" ")
        option["long"] = head
        if tail:
            value_name = tail.strip()[1:-1]
            option["value_name"] = value_name
            if value_name == "N":
                option["value_kind"] = "integer"
            elif "|" in value_name:
                option["value_kind"] = "enum"
                option["values"] = value_name.split("|")
            else:
                option["value_kind"] = "string"
        else:
            option["value_kind"] = "flag"

    if normalized_flag == "--mode <debug|release>":
        option["value_name"] = "mode"
        option["value_kind"] = "enum"
        option["values"] = ["debug", "release"]
        option["default"] = default
    elif normalized_flag == "--debug-trace <off|calls|full>":
        option["value_name"] = "trace"
        option["value_kind"] = "enum"
        option["values"] = ["off", "calls", "full"]
        option["default"] = default
    elif normalized_flag == "--max-k-step <N>":
        option["default"] = int(default) if default.isdigit() else default
    elif default not in ("", "(off)", "(default)"):
        option["default"] = default

    return normalized_flag, description, option
