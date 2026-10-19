"""Repository paths and defaults shared by the help generators."""

from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SPEC_DIR = REPO / "docs" / "spec"
GRAMMAR = SPEC_DIR / "grammar.md"
CLI = SPEC_DIR / "cli.md"
CONTRACTS = SPEC_DIR / "contracts.md"
METHODOLOGY = SPEC_DIR / "contracts-methodology.md"
INDEX = SPEC_DIR / "index.md"
ERRORS = SPEC_DIR / "errors.md"
EXAMPLES = SPEC_DIR / "examples.md"
STDLIB = SPEC_DIR / "stdlib.md"
SCHEMAS_DIR = SPEC_DIR / "schemas"
SKILL_RS = REPO / "vow" / "src" / "skill.rs"
MAIN_VOW = REPO / "compiler" / "main.vow"
SKILLS_DIR = REPO / "skills" / "vow"

SCHEMA_VERSION = "2"
DEFAULT_MAX_K_STEP = 50
DEFAULT_SOLVER = "auto"
DEFAULT_ENCODING = "auto"
