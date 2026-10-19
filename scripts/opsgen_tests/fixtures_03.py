"""Operation catalogue test fixtures."""

import tempfile
from pathlib import Path

from opsgen_tests.common import go
from opsgen_tests.fixtures_01 import PRINT_OPS
from opsgen_tests.fixtures_02 import _write_excluded_catalogue


def _load_excluded_errors(excluded, ops=None) -> list[str]:
    with tempfile.TemporaryDirectory() as d:
        _write_excluded_catalogue(Path(d), excluded, ops)
        try:
            go.load_excluded(Path(d), PRINT_OPS if ops is None else ops)
        except ValueError as e:
            return str(e).splitlines()
    raise AssertionError(f"load_excluded accepted {excluded!r}")


EXCLUDED = [
    {"names": ["debug_str"], "reason": "r"},
    {"prefix": "__vow_clif_", "reason": "r"},
    {"contains": "_to_", "suffixes": ["_try", "_wrap"], "reason": "r"},
]


FACT_FIXTURES = [
    (
        "vow-types/src/env.rs",
        "    // GENERATE:OPERATION_FACTS:START\n    // GENERATE:OPERATION_FACTS:END\n",
    ),
    (
        "vow-ir/src/lower/mod.rs",
        "    // GENERATE:OPERATION_FACTS:START\n    // GENERATE:OPERATION_FACTS:END\n",
    ),
    (
        "compiler/tests/catalogue_facts.vow",
        "// GENERATE:OPERATION_FACTS:START\n// GENERATE:OPERATION_FACTS:END\n",
    ),
]


def _write_fact_fixtures(tmp: Path) -> None:
    for rel, body in FACT_FIXTURES:
        path = tmp / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        existing = path.read_text() if path.exists() else "before\n"
        if "OPERATION_FACTS:START" in existing:
            continue
        path.write_text(f"{existing}{body}after\n")
