"""Wrapper around `vow verify` subprocess."""

from __future__ import annotations

import json
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path


@dataclass
class VerifyResult:
    status: str  # Verified, VerifyFailed, CompileFailed
    raw_json: str
    parsed: dict
    exit_code: int
    timed_out: bool


def run_verify(vow_binary: Path, vow_source: str, timeout: int = 120) -> VerifyResult:
    with tempfile.NamedTemporaryFile(
        mode="w", suffix=".vow", delete=False, dir="/tmp"
    ) as f:
        f.write(vow_source)
        tmp_path = f.name

    try:
        result = subprocess.run(
            [str(vow_binary), "verify", tmp_path],
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        raw = result.stdout.strip()
        try:
            parsed = json.loads(raw)
        except json.JSONDecodeError:
            parsed = {"status": "CompileFailed", "raw_stdout": raw, "stderr": result.stderr}

        return VerifyResult(
            status=parsed.get("status", "CompileFailed"),
            raw_json=raw,
            parsed=parsed,
            exit_code=result.returncode,
            timed_out=False,
        )
    except subprocess.TimeoutExpired:
        return VerifyResult(
            status="Timeout",
            raw_json="",
            parsed={"status": "Timeout"},
            exit_code=-1,
            timed_out=True,
        )
    finally:
        Path(tmp_path).unlink(missing_ok=True)
