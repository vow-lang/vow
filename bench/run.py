#!/usr/bin/env python3
"""CLI entry point for the vericoding benchmark runner."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from dataclasses import asdict
from pathlib import Path

# Ensure sibling modules are importable regardless of CWD
sys.path.insert(0, str(Path(__file__).resolve().parent))

from llm import make_config
from manifest import load_applicable, load_manifest
from prompts import build_system_prompt
from report import generate_report
from runner import BenchmarkResult, run_benchmark
from verifier import SELF_HOSTED_MEM_LIMIT, run_verify


def find_root() -> Path:
    """Find the vow-lang repo root (parent of bench/)."""
    return Path(__file__).resolve().parent.parent


def find_vow_binary(root: Path) -> Path:
    binary = root / "target" / "release" / "vow"
    if not binary.exists():
        print(f"Error: vow binary not found at {binary}", file=sys.stderr)
        print("Run: cargo build --all --release", file=sys.stderr)
        sys.exit(1)
    return binary


def find_self_hosted_binary(root: Path) -> Path:
    binary = root / "target" / "release" / "vow_self"
    if not binary.exists():
        rust_binary = find_vow_binary(root)
        compiler_src = root / "compiler" / "main.vow"
        print(f"Building self-hosted compiler → {binary} ...", file=sys.stderr)
        result = subprocess.run(
            [str(rust_binary), "--no-verify", str(compiler_src), "-o", str(binary)],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            print(f"Error building self-hosted compiler:\n{result.stderr}", file=sys.stderr)
            sys.exit(1)
        print("Self-hosted compiler built.", file=sys.stderr)
    return binary


def resolve_compiler(root: Path, compiler_name: str) -> tuple[Path, int | None]:
    if compiler_name == "self-hosted":
        return find_self_hosted_binary(root), SELF_HOSTED_MEM_LIMIT
    return find_vow_binary(root), None


def cmd_run(args: argparse.Namespace) -> None:
    root = find_root()
    vow_binary, memory_limit = resolve_compiler(root, args.compiler)
    system_prompt = build_system_prompt(root)
    results_dir = Path(__file__).resolve().parent / "results"

    # Create run directory
    run_id = args.run_id or time.strftime("%Y-%m-%dT%H:%M:%S")
    run_dir = results_dir / run_id
    run_dir.mkdir(parents=True, exist_ok=True)

    # Determine models
    if args.all:
        model_ids = _default_models()
    elif args.model:
        model_ids = [args.model]
    else:
        print("Error: specify --model <id> or --all", file=sys.stderr)
        sys.exit(1)

    # Load benchmarks
    all_benchmarks = load_manifest(root)
    applicable = [b for b in all_benchmarks if b.expected_status != "Stretch"]
    stretch = [b for b in all_benchmarks if b.expected_status == "Stretch"]

    # Filter to single benchmark if requested
    if args.benchmark:
        applicable = [b for b in applicable if b.id == args.benchmark]
        stretch = [b for b in stretch if b.id == args.benchmark]
        if not applicable and not stretch:
            print(f"Error: benchmark {args.benchmark} not found", file=sys.stderr)
            sys.exit(1)

    for model_id in model_ids:
        model_config = make_config(model_id)
        output_file = run_dir / f"{model_id}.json"

        # Resume support: load existing results
        existing_results: dict[str, dict] = {}
        stretch_results: list[dict] = []
        if args.resume and output_file.exists():
            with open(output_file) as f:
                existing = json.load(f)
            for r in existing.get("results", []):
                existing_results[r["benchmark_id"]] = r
            stretch_results = existing.get("stretch_results", [])

        print(f"\n=== Running model: {model_id} ===")
        results: list[BenchmarkResult] = []

        for i, bench in enumerate(applicable, 1):
            if bench.id in existing_results:
                print(f"  [{i}/{len(applicable)}] {bench.id} {bench.name} — skipped (already done)")
                results.append(_dict_to_result(existing_results[bench.id]))
                continue

            print(f"  [{i}/{len(applicable)}] {bench.id} {bench.name} ...", end=" ", flush=True)
            result = run_benchmark(bench, model_config, system_prompt, vow_binary, memory_limit=memory_limit)
            results.append(result)
            status_str = result.status.upper()
            if result.status == "verified":
                status_str = f"VERIFIED (iter {result.iterations})"
            print(f"{status_str} [{result.wall_clock_seconds:.1f}s]")

            # Save incrementally
            _save_results(output_file, model_id, run_id, results, stretch_results, args.compiler)

        # Run stretch benchmarks (informational)
        if not args.benchmark:
            for bench in stretch:
                bid = bench.id
                if args.resume and bid in {r["benchmark_id"] for r in stretch_results}:
                    continue
                print(f"  [stretch] {bid} {bench.name} ...", end=" ", flush=True)
                result = run_benchmark(bench, model_config, system_prompt, vow_binary, memory_limit=memory_limit)
                stretch_results.append(asdict(result))
                print(f"{result.status.upper()} [{result.wall_clock_seconds:.1f}s]")

        _save_results(output_file, model_id, run_id, results, stretch_results, args.compiler)
        print(f"\nResults saved to {output_file}")


def cmd_report(args: argparse.Namespace) -> None:
    results_dir = Path(__file__).resolve().parent / "results"
    if not args.run_id:
        # Use most recent
        runs = sorted(results_dir.iterdir()) if results_dir.exists() else []
        runs = [r for r in runs if r.is_dir()]
        if not runs:
            print("No results found", file=sys.stderr)
            sys.exit(1)
        args.run_id = runs[-1].name

    report = generate_report(results_dir, args.run_id)
    if args.output:
        Path(args.output).write_text(report)
        print(f"Report written to {args.output}")
    else:
        print(report)


def cmd_validate_references(args: argparse.Namespace) -> None:
    root = find_root()
    benchmarks = load_applicable(root)

    if args.compare:
        _validate_compare(root, benchmarks)
        return

    vow_binary, memory_limit = resolve_compiler(root, args.compiler)
    compiler_label = args.compiler

    passed = 0
    failed = 0
    print(f"Compiler: {compiler_label}")
    for bench in benchmarks:
        vr = run_verify(vow_binary, bench.reference_vow, memory_limit=memory_limit)
        status = "OK" if vr.status == "Verified" else f"FAIL ({vr.status})"
        print(f"  {bench.id} {bench.name}: {status}")
        if vr.status == "Verified":
            passed += 1
        else:
            failed += 1

    print(f"\n{passed}/{passed + failed} references verified ({compiler_label})")
    if failed:
        sys.exit(1)


def _validate_compare(root: Path, benchmarks: list) -> None:
    rust_binary, _ = resolve_compiler(root, "rust")
    self_binary, self_mem = resolve_compiler(root, "self-hosted")

    print(f"{'ID':<5} {'Name':<35} {'Rust':<12} {'Self-Hosted':<12} {'Match'}")
    print("-" * 75)

    mismatches = 0
    for bench in benchmarks:
        rust_vr = run_verify(rust_binary, bench.reference_vow)
        self_vr = run_verify(self_binary, bench.reference_vow, memory_limit=self_mem)
        rust_ok = rust_vr.status == "Verified"
        self_ok = self_vr.status == "Verified"
        match = rust_ok == self_ok
        if not match:
            mismatches += 1
        rust_str = "OK" if rust_ok else f"FAIL ({rust_vr.status})"
        self_str = "OK" if self_ok else f"FAIL ({self_vr.status})"
        match_str = "YES" if match else "NO"
        print(f"  {bench.id:<5} {bench.name:<35} {rust_str:<12} {self_str:<12} {match_str}")

    total = len(benchmarks)
    matched = total - mismatches
    print(f"\n{matched}/{total} references match across both compilers")
    if mismatches:
        sys.exit(1)


def _default_models() -> list[str]:
    return [
        "claude-sonnet-4-20250514",
    ]


def _save_results(
    output_file: Path,
    model_id: str,
    run_id: str,
    results: list[BenchmarkResult],
    stretch_results: list[dict],
    compiler: str = "rust",
) -> None:
    result_dicts = [asdict(r) for r in results]
    summary = _compute_summary(result_dicts)
    data = {
        "run_id": run_id,
        "model": model_id,
        "compiler": compiler,
        "results": result_dicts,
        "stretch_results": stretch_results,
        "summary": summary,
    }
    with open(output_file, "w") as f:
        json.dump(data, f, indent=2)


def _compute_summary(results: list[dict]) -> dict:
    total = len(results)
    verified = sum(1 for r in results if r["status"] == "verified")
    by_diff: dict[str, dict] = {}
    for r in results:
        d = r["difficulty"]
        if d not in by_diff:
            by_diff[d] = {"total": 0, "verified": 0}
        by_diff[d]["total"] += 1
        if r["status"] == "verified":
            by_diff[d]["verified"] += 1

    verified_iters = [r["iterations"] for r in results if r["status"] == "verified"]
    mean_iters = sum(verified_iters) / len(verified_iters) if verified_iters else 0

    return {
        "total_applicable": total,
        "verified": verified,
        "verification_rate": verified / total if total else 0,
        "by_difficulty": by_diff,
        "mean_cegis_iterations": round(mean_iters, 2),
    }


def _dict_to_result(d: dict) -> BenchmarkResult:
    return BenchmarkResult(
        benchmark_id=d["benchmark_id"],
        benchmark_name=d["benchmark_name"],
        difficulty=d["difficulty"],
        status=d["status"],
        iterations=d["iterations"],
        wall_clock_seconds=d["wall_clock_seconds"],
        failure_mode=d.get("failure_mode"),
        token_usage=d.get("token_usage", {"input_tokens": 0, "output_tokens": 0}),
        final_code=d.get("final_code", ""),
        raw_responses=d.get("raw_responses", []),
        verify_outputs=d.get("verify_outputs", []),
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Vericoding benchmark runner")
    subparsers = parser.add_subparsers(dest="command")

    # run
    run_parser = subparsers.add_parser("run", help="Run benchmarks")
    run_parser.add_argument("--model", help="Model ID (e.g. claude-sonnet-4-20250514)")
    run_parser.add_argument("--all", action="store_true", help="Run all configured models")
    run_parser.add_argument("--benchmark", help="Run single benchmark by ID (e.g. E01)")
    run_parser.add_argument("--resume", action="store_true", help="Skip already-completed benchmarks")
    run_parser.add_argument("--run-id", help="Run ID (default: timestamp)")
    run_parser.add_argument("--compiler", choices=["rust", "self-hosted"], default="rust",
                            help="Which compiler to use for verification (default: rust)")
    run_parser.set_defaults(func=cmd_run)

    # report
    report_parser = subparsers.add_parser("report", help="Generate comparison report")
    report_parser.add_argument("--run-id", help="Run ID (default: most recent)")
    report_parser.add_argument("--output", "-o", help="Output file (default: stdout)")
    report_parser.set_defaults(func=cmd_report)

    # validate-references
    val_parser = subparsers.add_parser("validate-references", help="Verify all reference.vow files")
    val_parser.add_argument("--compiler", choices=["rust", "self-hosted"], default="rust",
                            help="Which compiler to use for verification (default: rust)")
    val_parser.add_argument("--compare", action="store_true",
                            help="Run both compilers and compare results side-by-side")
    val_parser.set_defaults(func=cmd_validate_references)

    args = parser.parse_args()
    if not args.command:
        parser.print_help()
        sys.exit(1)

    args.func(args)


if __name__ == "__main__":
    main()
