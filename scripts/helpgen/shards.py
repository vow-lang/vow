"""Split generated payloads into files small enough for the repository hooks."""

# check-added-large-files rejects a new file over 500 KB, and the layout test
# caps a generated file at 450 KB; a shard stays well under both.
SHARD_SOURCE_BYTES = 200_000
# Raw text per bundle part. A part's emitted source is a small multiple of this.
BUNDLE_PART_TEXT_BYTES = 80_000


def split_text(text: str, limit: int) -> list[str]:
    """Cut `text` into pieces of at most `limit` bytes at line boundaries.

    The pieces rejoin, one newline between each, into `text`; a single line
    longer than `limit` is a piece of its own.
    """
    pieces: list[list[str]] = [[]]
    size = 0
    for line in text.split("\n"):
        line_size = len(line.encode("utf-8")) + 1
        if pieces[-1] and size + line_size > limit:
            pieces.append([])
            size = 0
        pieces[-1].append(line)
        size += line_size
    return ["\n".join(lines) for lines in pieces]


def pack(items: list[tuple[str, str]], limit: int) -> list[list[tuple[str, str]]]:
    """Group consecutive (name, source) items into shards of at most `limit` bytes."""
    shards: list[list[tuple[str, str]]] = [[]]
    size = 0
    for name, source in items:
        source_size = len(source.encode("utf-8"))
        if shards[-1] and size + source_size > limit:
            shards.append([])
            size = 0
        shards[-1].append((name, source))
        size += source_size
    return shards
