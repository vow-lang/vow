"""The OPERATIONS block targets and their write/check drivers."""

from __future__ import annotations

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
from opsgen.markers import (
    MARKER_END,
    MARKER_START,
    _stale_blocks,
    _stale_whole_files,
    _write_blocks,
    _write_whole_files,
)

# Every generator takes (ops, routes): the catalogue's operations and the
# runtime symbols of its arena routes.
TARGET_FILES = [
    (Path("vow-ir/src/lower/mod.rs"), gen_rust_ir_block),
    (Path("vow-codegen/src/cranelift_backend.rs"), gen_cranelift_block),
    (Path("vow-clif-shim/src/lib.rs"), gen_shim_block),
    (Path("compiler/lower.vow"), gen_vow_lower_block),
    (Path("vow-ir/src/region.rs"), gen_region_block),
    (Path("vow-verify/src/c_emitter.rs"), gen_rust_c_model_block),
    (Path("compiler/c_emitter.vow"), gen_vow_c_model_block),
]


# Whole generated modules: (path, module name, generator). The file is the complete
# generator output, so it is exempt from the authored-file budget and a catalogue
# change never edits an authored module.
WHOLE_FILE_TARGETS = [
    (Path("compiler/catalogue_ir_gen.vow"), "CatalogueIrGen", gen_vow_ir_block),
    (
        Path("compiler/catalogue_vc_ops_gen.vow"),
        "CatalogueVcOpsGen",
        gen_vow_vc_ops_block,
    ),
]


def write_projections(ops: list[dict], repo_root: Path, routes: list[str]) -> None:
    _write_blocks(TARGET_FILES, MARKER_START, MARKER_END, repo_root, ops, routes)
    _write_whole_files(WHOLE_FILE_TARGETS, repo_root, ops, routes)


def check_projections(ops: list[dict], repo_root: Path, routes: list[str]) -> list[str]:
    return _stale_blocks(
        TARGET_FILES,
        MARKER_START,
        MARKER_END,
        repo_root,
        ops,
        routes,
        "spliced block is stale",
    ) + _stale_whole_files(
        WHOLE_FILE_TARGETS, repo_root, ops, routes, "generated file is stale"
    )
