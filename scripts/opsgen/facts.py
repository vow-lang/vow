"""Test-only fact tables spliced between OPERATION_FACTS markers."""

from __future__ import annotations

from pathlib import Path

from opsgen.catalogue import result_shape
from opsgen.markers import (
    _stale_blocks,
    _stale_whole_files,
    _write_blocks,
    _write_whole_files,
)
from opsgen.tokens import DOC_SIGNATURE_RE, _split_top_level_commas

FACTS_MARKER_START = "// GENERATE:OPERATION_FACTS:START"
FACTS_MARKER_END = "// GENERATE:OPERATION_FACTS:END"


def _doc_return(op: dict) -> str:
    return DOC_SIGNATURE_RE.match(op["doc_signature"]).group("ret").strip()


def _doc_params(op: dict) -> list[str]:
    """The parameter types as the docs spell them (`String`, `Vec<String>`)."""
    params = DOC_SIGNATURE_RE.match(op["doc_signature"]).group("params")
    return [p.partition(": ")[2].strip() for p in _split_top_level_commas(params) if p]


def result_tag_token(op: dict) -> str:
    """The op's result tag as one token for the fact tables: `none`, `string`,
    `vec` or `option:<elem>`, derived from `result_shape` so the tables and the
    generated lowering lookups cannot disagree."""
    shape = result_shape(op)
    if shape is None:
        return "none"
    kind, elem = shape
    return f"option:{elem}" if kind == "option" else kind


def _indent_facts_block(lines: list[str]) -> str:
    """Join a fact-table block, indenting every line but the marker line."""
    return "\n".join(
        line if i == 0 or not line else f"    {line}" for i, line in enumerate(lines)
    )


def _quoted_list(items: list[str]) -> str:
    return ", ".join(f'"{item}"' for item in items)


def gen_rust_facts_block(ops: list[dict], excluded: list[dict]) -> str:
    """`CATALOGUE_FACTS` and `EXCLUDED_FACTS` for vow-types' env.rs test module.
    Marked `#[rustfmt::skip]` so the one-row-per-op layout survives
    `cargo fmt`; every line after the first carries the module's indentation."""
    rows = [
        f'    CatalogueFact {{ name: "{op["name"]}", effects: "{op["effects"]}", '
        f'params: &[{_quoted_list(op["params"])}], ret: "{op["return"]}", '
        f"doc_params: &[{_quoted_list(_doc_params(op))}], "
        f'doc_ret: "{_doc_return(op)}" }},'
        for op in ops
    ]
    exclusions = []
    for entry in excluded:
        if "names" in entry:
            exclusions.append(
                f"    ExcludedFact::Names(&[{_quoted_list(entry['names'])}]),"
            )
        elif "prefix" in entry:
            exclusions.append(f'    ExcludedFact::Prefix("{entry["prefix"]}"),')
        else:
            exclusions.append(
                f'    ExcludedFact::Contains {{ needle: "{entry["contains"]}", '
                f"suffixes: &[{_quoted_list(entry['suffixes'])}] }},"
            )
    lines = (
        [
            FACTS_MARKER_START,
            "#[rustfmt::skip]",
            "const CATALOGUE_FACTS: &[CatalogueFact] = &[",
        ]
        + rows
        + ["];", "", "#[rustfmt::skip]", "const EXCLUDED_FACTS: &[ExcludedFact] = &["]
        + exclusions
        + ["];", FACTS_MARKER_END]
    )
    return _indent_facts_block(lines)


def gen_rust_lowering_facts_block(ops: list[dict], excluded: list[dict]) -> str:
    """`LOWERING_FACTS` for vow-ir's lowering test module: the runtime symbol,
    return token and result tag of every op, the columns the lowering lookups
    are checked against. Same layout rules as `gen_rust_facts_block`."""
    rows = [
        f'    LoweringFact {{ name: "{op["name"]}", symbol: "{op["runtime_symbol"]}", '
        f'ret: "{op["return"]}", tag: "{result_tag_token(op)}" }},'
        for op in ops
    ]
    lines = (
        [
            FACTS_MARKER_START,
            "#[rustfmt::skip]",
            "const LOWERING_FACTS: &[LoweringFact] = &[",
        ]
        + rows
        + ["];", FACTS_MARKER_END]
    )
    return _indent_facts_block(lines)


def gen_vow_facts_block(ops: list[dict], excluded: list[dict]) -> str:
    """`op_facts_all` and `op_facts_excluded` for compiler/tests/catalogue_facts.vow.
    Parameter tokens and exclusion lists are comma-joined strings (doc parameter
    types `|`-joined), since Vow has no slice literals; the readers split them
    back."""
    facts = "\n".join(
        f'    v.push(op_fact(String::from("{op["name"]}"), '
        f'String::from("{op["effects"]}"), '
        f'String::from("{",".join(op["params"])}"), '
        f'String::from("{op["return"]}"), '
        f'String::from("{"|".join(_doc_params(op))}"), '
        f'String::from("{_doc_return(op)}"), '
        f'String::from("{op["runtime_symbol"]}"), '
        f'String::from("{result_tag_token(op)}")));'
        for op in ops
    )
    exclusions = []
    for entry in excluded:
        if "names" in entry:
            args = (
                f'OPX_NAMES(), String::from("{",".join(entry["names"])}"), '
                'String::from("")'
            )
        elif "prefix" in entry:
            args = f'OPX_PREFIX(), String::from("{entry["prefix"]}"), String::from("")'
        else:
            args = (
                f'OPX_CONTAINS(), String::from("{entry["contains"]}"), '
                f'String::from("{",".join(entry["suffixes"])}")'
            )
        exclusions.append(f"    v.push(op_excluded({args}));")
    exclusion_rows = "\n".join(exclusions)
    return (
        f"{FACTS_MARKER_START}\n"
        "fn op_facts_all() -> Vec<OpFact> {\n"
        "    let v: Vec<OpFact> = Vec::new();\n"
        f"{facts}\n"
        "    v\n"
        "}\n"
        "\n"
        "fn op_facts_excluded() -> Vec<OpExcluded> {\n"
        "    let v: Vec<OpExcluded> = Vec::new();\n"
        f"{exclusion_rows}\n"
        "    v\n"
        "}\n"
        f"{FACTS_MARKER_END}"
    )


# Test-only fact tables, spliced between their own marker pair so the mutants
# skip-list (`// GENERATE:<NAME>:START`) covers them. They take (ops, excluded)
# where TARGET_FILES generators take (ops, routes).
FACT_TARGET_FILES = [
    (Path("vow-types/src/env.rs"), gen_rust_facts_block),
    (Path("vow-ir/src/lower/mod.rs"), gen_rust_lowering_facts_block),
]

# Whole generated test module: (path, module name, generator).
FACT_WHOLE_TARGETS = [
    (
        Path("compiler/tests/catalogue_facts_gen.vow"),
        "CatalogueFactsGen",
        gen_vow_facts_block,
    ),
]


def write_fact_tables(ops: list[dict], repo_root: Path, excluded: list[dict]) -> None:
    _write_blocks(
        FACT_TARGET_FILES,
        FACTS_MARKER_START,
        FACTS_MARKER_END,
        repo_root,
        ops,
        excluded,
    )
    _write_whole_files(FACT_WHOLE_TARGETS, repo_root, ops, excluded)


def check_fact_tables(
    ops: list[dict], repo_root: Path, excluded: list[dict]
) -> list[str]:
    """Regenerate each fact table in-memory and diff it against the checked-in
    file; a target missing its marker pair raises rather than reporting clean."""
    return _stale_blocks(
        FACT_TARGET_FILES,
        FACTS_MARKER_START,
        FACTS_MARKER_END,
        repo_root,
        ops,
        excluded,
        "spliced fact table is stale",
    ) + _stale_whole_files(
        FACT_WHOLE_TARGETS, repo_root, ops, excluded, "generated fact table is stale"
    )
