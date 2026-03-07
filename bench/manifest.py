"""Load benchmark manifest and per-benchmark metadata."""

from __future__ import annotations

import tomllib
from dataclasses import dataclass
from pathlib import Path


@dataclass
class BenchmarkInfo:
    id: str
    name: str
    difficulty: str
    path: str  # relative to benchmarks/
    expected_status: str
    max_cegis_iterations: int
    tags: list[str]
    spec_md: str
    skeleton_vow: str
    reference_vow: str


def load_manifest(root: Path) -> list[BenchmarkInfo]:
    manifest_path = root / "benchmarks" / "manifest.toml"
    with open(manifest_path, "rb") as f:
        manifest = tomllib.load(f)

    benchmarks = []
    for entry in manifest["benchmarks"]:
        bench_dir = root / "benchmarks" / entry["path"]
        meta_path = bench_dir / "meta.toml"
        with open(meta_path, "rb") as f:
            meta = tomllib.load(f)["benchmark"]

        benchmarks.append(BenchmarkInfo(
            id=entry["id"],
            name=entry["name"],
            difficulty=entry["difficulty"],
            path=entry["path"],
            expected_status=entry["expected_status"],
            max_cegis_iterations=meta.get("max_cegis_iterations", 5),
            tags=meta.get("tags", []),
            spec_md=(bench_dir / "spec.md").read_text(),
            skeleton_vow=(bench_dir / "skeleton.vow").read_text(),
            reference_vow=(bench_dir / "reference.vow").read_text(),
        ))

    return benchmarks


def load_applicable(root: Path) -> list[BenchmarkInfo]:
    """Load only non-Stretch benchmarks."""
    return [b for b in load_manifest(root) if b.expected_status != "Stretch"]
