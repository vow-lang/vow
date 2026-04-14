#!/usr/bin/env python3
"""CLI entry point for the multi-agent workflow orchestrator."""

from __future__ import annotations

import argparse
import json
import sys
import time
import tomllib
from dataclasses import asdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from orchestrator import WorkflowResult, run_workflow


def find_root() -> Path:
    """Find the vow-lang repo root (parent of workflow/)."""
    return Path(__file__).resolve().parent.parent


def find_vowc(root: Path) -> Path:
    vowc = root / "build" / "vowc"
    if not vowc.exists():
        print(f"Error: vowc not found at {vowc}", file=sys.stderr)
        print("Run: scripts/bootstrap.sh", file=sys.stderr)
        sys.exit(1)
    return vowc


def load_config() -> dict:
    config_path = Path(__file__).resolve().parent / "config.toml"
    with open(config_path, "rb") as f:
        return tomllib.load(f)


def _event_printer(event: str, data: dict) -> None:
    """Default event callback: print progress to stderr."""
    if event == "start":
        print(f"Task: {data['task']}", file=sys.stderr)
        print(f"Max rounds: {data['max_rounds']}", file=sys.stderr)
    elif event == "coder_initial":
        print("  Generating initial code...", file=sys.stderr, flush=True)
    elif event == "round_start":
        print(f"\n--- Round {data['round']} ---", file=sys.stderr)
    elif event == "compile_fix":
        print(f"  Fixing compilation (attempt {data['attempt']})...", file=sys.stderr, flush=True)
    elif event == "verify":
        print(f"  Verifying...", file=sys.stderr, flush=True)
    elif event == "analyst":
        print(f"  Analyst reviewing...", file=sys.stderr, flush=True)
    elif event == "reviewer":
        print(f"  Reviewer reviewing...", file=sys.stderr, flush=True)
    elif event == "coder_feedback":
        print(f"  Coder incorporating feedback...", file=sys.stderr, flush=True)
    elif event == "round_end":
        if not data.get("compile_ok"):
            print(f"  Result: compilation failed", file=sys.stderr)
        else:
            parts = [
                f"verify={data.get('verify_status', '?')}",
                f"analyst={data.get('analyst_verdict', '?')}",
                f"reviewer={data.get('reviewer_verdict', '?')}",
            ]
            converged = data.get("converged", False)
            status = "CONVERGED" if converged else "continuing"
            print(f"  Result: {', '.join(parts)} [{status}]", file=sys.stderr)


def _save_result(result: WorkflowResult, output_path: Path) -> None:
    data = {
        "task_description": result.task_description,
        "status": result.status,
        "total_rounds": result.total_rounds,
        "wall_clock_seconds": round(result.wall_clock_seconds, 2),
        "token_usage": result.token_usage,
        "rounds": [asdict(r) for r in result.rounds],
        "final_code": result.final_code,
    }
    output_path.parent.mkdir(parents=True, exist_ok=True)
    with open(output_path, "w") as f:
        json.dump(data, f, indent=2)


def cmd_run(args: argparse.Namespace) -> None:
    root = find_root()
    vowc = find_vowc(root)
    config = load_config()

    # Override agent models if specified
    if args.coder_model:
        config.setdefault("agents", {}).setdefault("coder", {})["model"] = args.coder_model
    if args.analyst_model:
        config.setdefault("agents", {}).setdefault("analyst", {})["model"] = args.analyst_model
    if args.reviewer_model:
        config.setdefault("agents", {}).setdefault("reviewer", {})["model"] = args.reviewer_model
    if args.max_rounds:
        config.setdefault("defaults", {})["max_rounds"] = args.max_rounds

    # Load task description
    if args.task_file:
        task_description = Path(args.task_file).read_text().strip()
    elif args.task:
        task_description = args.task
    else:
        print("Error: specify --task or --task-file", file=sys.stderr)
        sys.exit(1)

    context = None
    if args.context_file:
        context = Path(args.context_file).read_text().strip()

    callback = _event_printer if not args.quiet else None

    result = run_workflow(
        task_description=task_description,
        root=root,
        vow_binary=vowc,
        config=config,
        context=context,
        on_event=callback,
    )

    # Output
    if args.output:
        output_path = Path(args.output)
    else:
        results_dir = Path(__file__).resolve().parent / "results"
        run_id = time.strftime("%Y-%m-%dT%H:%M:%S")
        output_path = results_dir / f"{run_id}.json"

    _save_result(result, output_path)

    print(f"\nStatus: {result.status}", file=sys.stderr)
    print(f"Rounds: {result.total_rounds}", file=sys.stderr)
    print(f"Time: {result.wall_clock_seconds:.1f}s", file=sys.stderr)
    print(f"Output: {output_path}", file=sys.stderr)

    if result.final_code:
        print(f"\n--- Final Code ---", file=sys.stderr)
        print(result.final_code)


def cmd_show(args: argparse.Namespace) -> None:
    results_dir = Path(__file__).resolve().parent / "results"

    if args.run_file:
        path = Path(args.run_file)
    else:
        if not results_dir.exists():
            print("No results found", file=sys.stderr)
            sys.exit(1)
        files = sorted(results_dir.glob("*.json"))
        if not files:
            print("No results found", file=sys.stderr)
            sys.exit(1)
        path = files[-1]

    with open(path) as f:
        data = json.load(f)

    print(f"Task: {data['task_description']}")
    print(f"Status: {data['status']}")
    print(f"Rounds: {data['total_rounds']}")
    print(f"Time: {data['wall_clock_seconds']}s")

    usage = data.get("token_usage", {})
    for agent, u in usage.items():
        print(f"  {agent}: {u.get('input_tokens', 0)} in / {u.get('output_tokens', 0)} out")

    print(f"\nRound history:")
    for r in data.get("rounds", []):
        vs = r.get("verify_status", "n/a")
        av = r.get("analyst_verdict", "n/a")
        rv = r.get("reviewer_verdict", "n/a")
        conv = "CONVERGED" if r.get("converged") else ""
        print(f"  Round {r['round_num']}: verify={vs} analyst={av} reviewer={rv} {conv}")

    if args.code and data.get("final_code"):
        print(f"\n--- Final Code ---")
        print(data["final_code"])


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Multi-agent workflow orchestrator for Vow development"
    )
    subparsers = parser.add_subparsers(dest="command")

    # run
    run_parser = subparsers.add_parser("run", help="Execute the multi-agent workflow")
    run_parser.add_argument("--task", help="Task description (inline)")
    run_parser.add_argument("--task-file", help="Task description file")
    run_parser.add_argument("--context-file", help="Additional context file")
    run_parser.add_argument("--output", "-o", help="Output JSON file")
    run_parser.add_argument("--max-rounds", type=int, help="Override max convergence rounds")
    run_parser.add_argument("--coder-model", help="Override Coder agent model")
    run_parser.add_argument("--analyst-model", help="Override Analyst agent model")
    run_parser.add_argument("--reviewer-model", help="Override Reviewer agent model")
    run_parser.add_argument("--quiet", "-q", action="store_true", help="Suppress progress output")
    run_parser.set_defaults(func=cmd_run)

    # show
    show_parser = subparsers.add_parser("show", help="Display results from a previous run")
    show_parser.add_argument("run_file", nargs="?", help="Result JSON file (default: most recent)")
    show_parser.add_argument("--code", action="store_true", help="Also print the final code")
    show_parser.set_defaults(func=cmd_show)

    args = parser.parse_args()
    if not args.command:
        parser.print_help()
        sys.exit(1)

    args.func(args)


if __name__ == "__main__":
    main()
