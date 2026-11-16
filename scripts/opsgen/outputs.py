"""Own the Operation Catalogue's output inventory and persistence policy."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

from opsgen.emit_rust import (
    gen_cranelift_block,
    gen_region_block,
    gen_rust_c_model_block,
    gen_rust_ir_block,
    gen_shim_block,
)
from opsgen.emit_vow import (
    gen_vow_c_model_block,
    gen_vow_ir_block,
    gen_vow_lower_block,
    gen_vow_vc_ops_block,
)
from opsgen.facts import (
    FACTS_MARKER_END,
    FACTS_MARKER_START,
    gen_rust_facts_block,
    gen_rust_lowering_facts_block,
    gen_vow_facts_block,
)
from opsgen.markers import (
    MARKER_END,
    MARKER_START,
    _replace_between_markers,
    whole_vow_module,
)

# Runtime projections take (ops, routes); fact renderers take (ops, excluded).
TARGET_FILES = [
    (Path("vow-ir/src/lower/mod.rs"), gen_rust_ir_block),
    (Path("vow-codegen/src/cranelift_backend.rs"), gen_cranelift_block),
    (Path("vow-clif-shim/src/lib.rs"), gen_shim_block),
    (Path("compiler/lower.vow"), gen_vow_lower_block),
    (Path("vow-ir/src/region.rs"), gen_region_block),
    (Path("vow-verify/src/c_emitter.rs"), gen_rust_c_model_block),
    (Path("compiler/c_emitter.vow"), gen_vow_c_model_block),
]
WHOLE_FILE_TARGETS = [
    (Path("compiler/catalogue_ir_gen.vow"), "CatalogueIrGen", gen_vow_ir_block),
    (
        Path("compiler/catalogue_vc_ops_gen.vow"),
        "CatalogueVcOpsGen",
        gen_vow_vc_ops_block,
    ),
]
FACT_TARGET_FILES = [
    (Path("vow-types/src/env.rs"), gen_rust_facts_block),
    (Path("vow-ir/src/lower/mod.rs"), gen_rust_lowering_facts_block),
]
FACT_WHOLE_TARGETS = [
    (
        Path("compiler/tests/catalogue_facts_gen.vow"),
        "CatalogueFactsGen",
        gen_vow_facts_block,
    ),
]


@dataclass(frozen=True)
class _OutputFamily:
    blocks: list[tuple[Path, Callable[..., str]]]
    whole_files: list[tuple[Path, str, Callable[..., str]]]
    start: str
    end: str
    block_stale: str
    whole_stale: str

    def write(self, ops: list[dict], root: Path, extra: list) -> None:
        for rel_path, render in self.blocks:
            path = root / rel_path
            path.write_text(
                _replace_between_markers(
                    path.read_text(), self.start, self.end, render(ops, extra)
                )
            )
        for rel_path, module, render in self.whole_files:
            path = root / rel_path
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(whole_vow_module(module, render(ops, extra)))

    def check(self, ops: list[dict], root: Path, extra: list) -> list[str]:
        mismatches: list[str] = []
        for rel_path, render in self.blocks:
            content = (root / rel_path).read_text()
            expected = _replace_between_markers(
                content, self.start, self.end, render(ops, extra)
            )
            if content != expected:
                mismatches.append(f"{rel_path}: {self.block_stale}")
        for rel_path, module, render in self.whole_files:
            path = root / rel_path
            expected = whole_vow_module(module, render(ops, extra))
            if not path.exists() or path.read_text() != expected:
                mismatches.append(f"{rel_path}: {self.whole_stale}")
        return mismatches


_PROJECTIONS = _OutputFamily(
    TARGET_FILES,
    WHOLE_FILE_TARGETS,
    MARKER_START,
    MARKER_END,
    "spliced block is stale",
    "generated file is stale",
)
_FACTS = _OutputFamily(
    FACT_TARGET_FILES,
    FACT_WHOLE_TARGETS,
    FACTS_MARKER_START,
    FACTS_MARKER_END,
    "spliced fact table is stale",
    "generated fact table is stale",
)
_FAMILIES = (_PROJECTIONS, _FACTS)


def write_outputs(
    ops: list[dict], repo_root: Path, routes: list[str], excluded: list[dict]
) -> list[Path]:
    """Write all outputs and return paths in CLI announcement order.

    Each family completes its writes before the next one starts, rereading
    shared hosts. Announcements group splices before whole files and retain
    duplicate paths for hosts with both marker families.
    """
    _PROJECTIONS.write(ops, repo_root, routes)
    _FACTS.write(ops, repo_root, excluded)
    return [path for family in _FAMILIES for path, _ in family.blocks] + [
        path for family in _FAMILIES for path, _, _ in family.whole_files
    ]


def check_outputs(
    ops: list[dict], repo_root: Path, routes: list[str], excluded: list[dict]
) -> list[str]:
    """Report stale outputs in family order without writing to the tree."""
    return _PROJECTIONS.check(ops, repo_root, routes) + _FACTS.check(
        ops, repo_root, excluded
    )


def whole_output_paths() -> set[str]:
    """Return only whole generated files, without loading or rendering facts."""
    return {
        path.as_posix() for family in _FAMILIES for path, _, _ in family.whole_files
    }


# Keep the entry script's family-specific interface available to existing tests.
def write_projections(ops: list[dict], repo_root: Path, routes: list[str]) -> None:
    _PROJECTIONS.write(ops, repo_root, routes)


def check_projections(ops: list[dict], repo_root: Path, routes: list[str]) -> list[str]:
    return _PROJECTIONS.check(ops, repo_root, routes)


def write_fact_tables(ops: list[dict], repo_root: Path, excluded: list[dict]) -> None:
    _FACTS.write(ops, repo_root, excluded)


def check_fact_tables(
    ops: list[dict], repo_root: Path, excluded: list[dict]
) -> list[str]:
    return _FACTS.check(ops, repo_root, excluded)
