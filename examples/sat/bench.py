#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import lzma
import os
import shutil
import subprocess
import sys
import time
import urllib.request
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent.parent
LOCAL_DIR = SCRIPT_DIR / ".local"
BENCH_DIR = LOCAL_DIR / "benchmarks"
RESULTS_DIR = LOCAL_DIR / "results"
BIN_DIR = LOCAL_DIR / "bin"
MANIFEST_PATH = SCRIPT_DIR / "benchmarks.json"
DEFAULT_TMPDIR = Path("/dev/shm") if Path("/dev/shm").is_dir() else Path("/tmp")
DEFAULT_SOLVER_BIN = BIN_DIR / "vow-sat"

DEFAULT_TIMEOUTS = {
    "smoke": 5,
    "core": 60,
    "stretch": 300,
}

BASELINE_CANDIDATES = {
    "minisat": ["minisat", "MiniSat"],
    "kissat": ["kissat"],
    "cadical": ["cadical", "cadical153", "cadical-sc2025"],
}


def load_manifest() -> list[dict]:
    return json.loads(MANIFEST_PATH.read_text())


def select_entries(manifest: list[dict], tiers: list[str]) -> list[dict]:
    wanted = set(tiers)
    return [entry for entry in manifest if entry["tier"] in wanted]


def compressed_path(entry: dict) -> Path:
    return BENCH_DIR / entry["filename"]


def plain_path(entry: dict) -> Path:
    path = compressed_path(entry)
    if path.suffix == ".xz":
        return path.with_suffix("")
    return path


def benchmark_url(entry: dict) -> str:
    return f"https://benchmark-database.de/file/{entry['id']}"


def ensure_dirs() -> None:
    BENCH_DIR.mkdir(parents=True, exist_ok=True)
    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    BIN_DIR.mkdir(parents=True, exist_ok=True)


def download_entry(entry: dict, force: bool = False) -> Path:
    ensure_dirs()
    comp = compressed_path(entry)
    plain = plain_path(entry)
    if plain.exists() and not force:
        return plain

    if force:
        comp.unlink(missing_ok=True)
        plain.unlink(missing_ok=True)

    url = benchmark_url(entry)
    with urllib.request.urlopen(url, timeout=120) as response:
        data = response.read()
    comp.write_bytes(data)

    if comp.suffix == ".xz":
        with lzma.open(comp, "rb") as src, plain.open("wb") as dst:
            shutil.copyfileobj(src, dst)
    else:
        plain = comp

    return plain


def detect_baselines() -> dict[str, str]:
    found: dict[str, str] = {}
    for label, candidates in BASELINE_CANDIDATES.items():
        for candidate in candidates:
            resolved = shutil.which(candidate)
            if resolved:
                found[label] = resolved
                break
    return found


def solver_sources() -> list[Path]:
    return sorted(SCRIPT_DIR.glob("*.vow"))


def needs_rebuild(binary: Path) -> bool:
    if not binary.exists():
        return True
    bin_mtime = binary.stat().st_mtime
    for source in solver_sources():
        if source.stat().st_mtime > bin_mtime:
            return True
    return False


def build_vow_solver(force: bool = False) -> Path:
    ensure_dirs()
    if not force and not needs_rebuild(DEFAULT_SOLVER_BIN):
        return DEFAULT_SOLVER_BIN

    env = dict(os.environ)
    env["TMPDIR"] = env.get("TMPDIR", str(DEFAULT_TMPDIR))
    cmd = (
        f"ulimit -v 2000000; "
        f"'{REPO_ROOT / 'build' / 'vowc'}' build --no-verify "
        f"'{SCRIPT_DIR / 'main.vow'}' -o '{DEFAULT_SOLVER_BIN}'"
    )
    subprocess.run(["zsh", "-lc", cmd], check=True, env=env)
    return DEFAULT_SOLVER_BIN


def parse_solver_status(stdout: str, stderr: str, code: int) -> str | None:
    combined_lines = [line.strip() for line in (stdout + "\n" + stderr).splitlines()]
    for line in combined_lines:
        if not line:
            continue
        if "UNSATISFIABLE" in line or line == "UNSAT" or line == "s UNSATISFIABLE":
            return "UNSAT"
        if "SATISFIABLE" in line or line == "SAT" or line == "s SATISFIABLE":
            return "SAT"
    if code == 10:
        return "SAT"
    if code == 20:
        return "UNSAT"
    return None


def parse_v_assignment(stdout: str, num_vars: int) -> dict[int, bool] | None:
    assignment: dict[int, bool] = {}
    saw_zero = False
    for raw_line in stdout.splitlines():
        line = raw_line.strip()
        if not line.startswith("v "):
            continue
        for token in line.split()[1:]:
            lit = int(token)
            if lit == 0:
                saw_zero = True
                break
            assignment[abs(lit)] = lit > 0
        if saw_zero:
            break
    if not saw_zero:
        return None
    for var in range(1, num_vars + 1):
        assignment.setdefault(var, True)
    return assignment


def parse_dimacs(path: Path) -> tuple[int, list[list[int]]]:
    num_vars = 0
    clauses: list[list[int]] = []
    with path.open("r", encoding="utf-8", errors="replace") as handle:
        for raw in handle:
            line = raw.strip()
            if not line or line.startswith("c"):
                continue
            if line.startswith("p "):
                parts = line.split()
                if len(parts) >= 4:
                    num_vars = int(parts[2])
                continue
            clause: list[int] = []
            for tok in line.split():
                lit = int(tok)
                if lit == 0:
                    clauses.append(clause)
                    clause = []
                else:
                    clause.append(lit)
    return num_vars, clauses


def assignment_satisfies(path: Path, assignment: dict[int, bool]) -> bool:
    _, clauses = parse_dimacs(path)
    for clause in clauses:
        if not any((assignment[abs(lit)] if lit > 0 else not assignment[abs(lit)]) for lit in clause):
            return False
    return True


def classify_vow_result(entry: dict, bench_path: Path, status: str | None, stdout: str, stderr: str, code: int) -> str:
    if code == 1 and status is None:
        return "parse_error"
    if status is None:
        return "bad_output"
    if status == "SAT":
        num_vars, _ = parse_dimacs(bench_path)
        assignment = parse_v_assignment(stdout, num_vars)
        if assignment is None:
            return "bad_output"
        if not assignment_satisfies(bench_path, assignment):
            return "bad_output"
        if entry["expected"] == "UNSAT":
            return "bad_output"
        return "solved_sat"
    if status == "UNSAT":
        if entry["expected"] == "SAT":
            return "bad_output"
        return "solved_unsat"
    return "bad_output"


def classify_baseline_result(entry: dict, status: str | None, code: int) -> str:
    if status is None:
        return "crash" if code != 1 else "parse_error"
    if status == "SAT":
        if entry["expected"] == "UNSAT":
            return "bad_output"
        return "solved_sat"
    if status == "UNSAT":
        if entry["expected"] == "SAT":
            return "bad_output"
        return "solved_unsat"
    return "bad_output"


def run_command(command: list[str], timeout_s: int) -> tuple[int | None, str, str, float, bool]:
    start = time.perf_counter()
    try:
        proc = subprocess.run(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout_s,
        )
        elapsed = time.perf_counter() - start
        return proc.returncode, proc.stdout, proc.stderr, elapsed, False
    except subprocess.TimeoutExpired as exc:
        elapsed = time.perf_counter() - start
        stdout = exc.stdout or ""
        stderr = exc.stderr or ""
        return None, stdout, stderr, elapsed, True


def tier_timeout(entry: dict, override: int | None) -> int:
    if override is not None:
        return override
    return DEFAULT_TIMEOUTS[entry["tier"]]


def run_benchmarks(entries: list[dict], args: argparse.Namespace) -> dict:
    ensure_dirs()
    baselines = detect_baselines()
    vow_solver = Path(args.solver) if args.solver else build_vow_solver(force=args.rebuild)

    solver_commands: dict[str, list[str]] = {"vow-sat": [str(vow_solver)]}
    for label, path in baselines.items():
        solver_commands[label] = [path]

    results: list[dict] = []
    for entry in entries:
        bench_path = plain_path(entry)
        if not bench_path.exists():
            results.append(
                {
                    "benchmark": entry["filename"],
                    "tier": entry["tier"],
                    "solver": "vow-sat",
                    "classification": "skipped",
                    "detail": "download missing",
                }
            )
            continue

        for solver_name, command in solver_commands.items():
            timeout_s = tier_timeout(entry, args.timeout)
            code, stdout, stderr, elapsed, timed_out = run_command(command + [str(bench_path)], timeout_s)
            if timed_out:
                classification = "timeout"
                status = None
            else:
                status = parse_solver_status(stdout, stderr, code if code is not None else 1)
                if solver_name == "vow-sat":
                    classification = classify_vow_result(entry, bench_path, status, stdout, stderr, code if code is not None else 1)
                else:
                    classification = classify_baseline_result(entry, status, code if code is not None else 1)

            if entry["expected"] == "UNKNOWN" and classification in {"solved_sat", "solved_unsat"}:
                classification = f"{classification}_unknown_expected"

            results.append(
                {
                    "benchmark": entry["filename"],
                    "tier": entry["tier"],
                    "expected": entry["expected"],
                    "solver": solver_name,
                    "status": status,
                    "classification": classification,
                    "elapsed_s": round(elapsed, 6),
                    "returncode": code,
                    "stdout": stdout,
                    "stderr": stderr,
                }
            )

    report = {
        "generated_at_unix": int(time.time()),
        "tiers": sorted({entry["tier"] for entry in entries}),
        "solvers": list(solver_commands.keys()),
        "baseline_solvers_found": baselines,
        "results": results,
    }
    out_path = RESULTS_DIR / "latest.json"
    out_path.write_text(json.dumps(report, indent=2))
    return report


def print_summary(report: dict) -> None:
    print(f"saved report: {RESULTS_DIR / 'latest.json'}")
    baselines = report.get("baseline_solvers_found", {})
    if baselines:
        print("baseline solvers:")
        for name in sorted(baselines):
            print(f"  {name}: {baselines[name]}")
    else:
        print("baseline solvers: none detected on PATH")
    counts: dict[str, int] = {}
    for row in report["results"]:
        key = f"{row['solver']}::{row['classification']}"
        counts[key] = counts.get(key, 0) + 1
    for key in sorted(counts):
        print(f"{key} {counts[key]}")

    print("")
    for row in report["results"]:
        print(
            f"{row['tier']:7s} {row['solver']:8s} {row['classification']:26s} "
            f"{row['elapsed_s']:8.3f}s {row['benchmark']}"
        )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Local benchmark helper for examples/sat")
    sub = parser.add_subparsers(dest="command", required=True)

    download = sub.add_parser("download", help="download and decompress the selected benchmark subset")
    download.add_argument("--tier", action="append", choices=["smoke", "core", "stretch"], help="repeatable tier filter")
    download.add_argument("--force", action="store_true", help="redownload benchmarks even if already present")

    run = sub.add_parser("run", help="run the Vow SAT solver and any detected baselines on the selected subset")
    run.add_argument("--tier", action="append", choices=["smoke", "core", "stretch"], help="repeatable tier filter")
    run.add_argument("--timeout", type=int, help="override per-instance timeout in seconds")
    run.add_argument("--solver", help="path to an already-built SAT solver binary")
    run.add_argument("--rebuild", action="store_true", help="force a fresh build of the Vow SAT solver")

    sub.add_parser("list", help="print the curated manifest")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    manifest = load_manifest()

    if args.command == "list":
        print(json.dumps(manifest, indent=2))
        return 0

    tiers = args.tier if args.tier else ["smoke", "core", "stretch"]
    entries = select_entries(manifest, tiers)

    if args.command == "download":
        ensure_dirs()
        for entry in entries:
            path = download_entry(entry, force=args.force)
            print(f"downloaded {entry['tier']:7s} {entry['filename']} -> {path}")
        return 0

    if args.command == "run":
        report = run_benchmarks(entries, args)
        print_summary(report)
        return 0

    return 1


if __name__ == "__main__":
    raise SystemExit(main())
