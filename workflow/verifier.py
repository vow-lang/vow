"""Subprocess wrappers for vowc: compile-only and full verification."""

from __future__ import annotations

import json
import resource
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path

@dataclass
class CompileResult:
    success: bool
    raw_output: str
    parsed: dict
    exit_code: int
    stderr: str


@dataclass
class VerifyResult:
    status: str  # Verified, VerifyFailed, CompileFailed, Timeout
    raw_json: str
    parsed: dict
    exit_code: int
    timed_out: bool


def _write_temp(source: str) -> str:
    with tempfile.NamedTemporaryFile(
        mode="w", suffix=".vow", delete=False
    ) as f:
        f.write(source)
        return f.name


def _make_preexec(memory_limit: int | None):
    if memory_limit is None:
        return None

    def _set_limit() -> None:
        resource.setrlimit(resource.RLIMIT_AS, (memory_limit, memory_limit))

    return _set_limit


def run_compile(
    vow_binary: Path,
    vow_source: str,
    timeout: int = 60,
    memory_limit: int | None = None,
) -> CompileResult:
    """Run syntax and type checking only (no verification, no codegen)."""
    tmp_path = _write_temp(vow_source)
    out_path = tmp_path + ".out"
    try:
        result = subprocess.run(
            [str(vow_binary), "build", "--no-verify", "-o", out_path, tmp_path],
            capture_output=True,
            text=True,
            timeout=timeout,
            preexec_fn=_make_preexec(memory_limit),
        )
        raw = result.stdout.strip()
        try:
            parsed = json.loads(raw)
        except json.JSONDecodeError:
            parsed = {"raw_stdout": raw, "stderr": result.stderr}

        success = result.returncode == 0
        return CompileResult(
            success=success,
            raw_output=raw,
            parsed=parsed,
            exit_code=result.returncode,
            stderr=result.stderr,
        )
    except subprocess.TimeoutExpired:
        return CompileResult(
            success=False,
            raw_output="",
            parsed={"error": "compile_timeout"},
            exit_code=-1,
            stderr="Compilation timed out",
        )
    finally:
        Path(tmp_path).unlink(missing_ok=True)
        Path(out_path).unlink(missing_ok=True)
        Path(out_path + ".o").unlink(missing_ok=True)


def run_verify(
    vow_binary: Path,
    vow_source: str,
    timeout: int = 120,
    memory_limit: int | None = None,
) -> VerifyResult:
    """Run full verification pipeline."""
    tmp_path = _write_temp(vow_source)
    try:
        result = subprocess.run(
            [str(vow_binary), "verify", tmp_path],
            capture_output=True,
            text=True,
            timeout=timeout,
            preexec_fn=_make_preexec(memory_limit),
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
