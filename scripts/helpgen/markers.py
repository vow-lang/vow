"""Replace the generated blocks between `// GENERATE:NAME:START/END` markers."""


def _replace_between_markers(
    content: str, start_marker: str, end_marker: str, replacement: str
) -> str:
    """Replace content between start/end marker lines (inclusive)."""
    start_idx = content.find(start_marker)
    if start_idx == -1:
        raise ValueError(f"Cannot find marker: {start_marker}")
    end_idx = content.find(end_marker, start_idx)
    if end_idx == -1:
        raise ValueError(f"Cannot find marker: {end_marker}")
    end_idx += len(end_marker)
    return content[:start_idx] + replacement + content[end_idx:]
