"""Shared paths and the generator module under test."""

import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "scripts"))

import generate_operations as go  # noqa: E402

__all__ = ["REPO_ROOT", "go"]
