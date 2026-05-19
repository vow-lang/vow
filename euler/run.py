#!/usr/bin/env python3
"""CLI entry point for the Euler problems demo runner.

Drives an LLM to solve Project Euler problems in Vow, verifying each
solution with ESBMC via `vowc verify`, then optionally compiling and
executing to check the numeric answer.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import resource
import subprocess
import sys
import tempfile
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path

# ---------------------------------------------------------------------------
# LLM abstraction (inline, no dependency on bench/)
# ---------------------------------------------------------------------------

import anthropic
import openai


@dataclass
class LLMResponse:
    content: str
    input_tokens: int
    output_tokens: int


@dataclass
class ModelConfig:
    provider: str
    model_id: str
    max_tokens: int = 8192
    temperature: float = 0.0


def _infer_provider(model_id: str) -> str:
    if model_id.startswith("claude"):
        return "anthropic"
    if model_id.startswith(("gpt", "o1", "o3", "o4")):
        return "openai"
    raise ValueError(f"Cannot infer provider for model: {model_id}")


def make_config(model_id: str) -> ModelConfig:
    return ModelConfig(provider=_infer_provider(model_id), model_id=model_id)


def chat(config: ModelConfig, system: str, messages: list[dict[str, str]]) -> LLMResponse:
    if config.provider == "anthropic":
        client = anthropic.Anthropic(api_key=os.environ["ANTHROPIC_API_KEY"])
        resp = client.messages.create(
            model=config.model_id,
            max_tokens=config.max_tokens,
            temperature=config.temperature,
            system=system,
            messages=messages,
        )
        content = resp.content[0].text if resp.content else ""
        return LLMResponse(content, resp.usage.input_tokens, resp.usage.output_tokens)
    elif config.provider == "openai":
        client = openai.OpenAI(api_key=os.environ["OPENAI_API_KEY"])
        oai_msgs = [{"role": "system", "content": system}] + messages
        resp = client.chat.completions.create(
            model=config.model_id,
            max_tokens=config.max_tokens,
            temperature=config.temperature,
            messages=oai_msgs,
        )
        content = resp.choices[0].message.content or ""
        usage = resp.usage
        return LLMResponse(content, usage.prompt_tokens if usage else 0, usage.completion_tokens if usage else 0)
    raise ValueError(f"Unknown provider: {config.provider}")


# ---------------------------------------------------------------------------
# Manifest / problem loading
# ---------------------------------------------------------------------------

if sys.version_info >= (3, 11):
    import tomllib
else:
    import tomli as tomllib


@dataclass
class EulerProblem:
    id: str
    euler_number: int
    name: str
    difficulty: str
    tags: list[str]
    unwind: int
    answer: int
    spec_md: str
    skeleton_vow: str


def load_problems(root: Path, problem_id: str | None = None) -> list[EulerProblem]:
    manifest_path = root / "euler" / "manifest.toml"
    with open(manifest_path, "rb") as f:
        manifest = tomllib.load(f)

    problems = []
    for entry in manifest["problems"]:
        pid = entry["id"]
        if problem_id and pid != problem_id:
            continue
        prob_dir = root / "euler" / "problems" / f"{pid}_{entry['name']}"
        spec = (prob_dir / "spec.md").read_text()
        skeleton = (prob_dir / "skeleton.vow").read_text()
        problems.append(EulerProblem(
            id=pid,
            euler_number=entry["euler_number"],
            name=entry["name"],
            difficulty=entry["difficulty"],
            tags=entry.get("tags", []),
            unwind=entry.get("unwind", 10),
            answer=entry["answer"],
            spec_md=spec,
            skeleton_vow=skeleton,
        ))
    return problems


# ---------------------------------------------------------------------------
# Prompts
# ---------------------------------------------------------------------------

SKILL_FILES = ["index.md", "grammar.md", "contracts.md", "cli.md", "errors.md", "examples.md"]


def build_system_prompt(root: Path) -> str:
    skill_dir = root / "docs" / "skill"
    parts = []
    for name in SKILL_FILES:
        path = skill_dir / name
        parts.append(f"# {name}\n\n{path.read_text()}")
    return "\n\n---\n\n".join(parts)


def build_initial_prompt(spec_md: str, skeleton_vow: str) -> str:
    return f"""Below is a Project Euler problem specification and a Vow skeleton with contracts.

Fill in the function bodies so that all contracts verify. Return ONLY the complete .vow file, no explanation.

## Specification

{spec_md}

## Skeleton

```vow
{skeleton_vow}
```

Return the complete .vow file with function bodies filled in. Do not change the module name, function signatures, or contracts."""


def build_cegis_prompt(verify_output: str) -> str:
    return f"""Verification failed. Here is the full JSON output from `vow verify`:

```json
{verify_output}
```

Fix the implementation so all contracts verify. Return ONLY the complete updated .vow file, no explanation."""


# ---------------------------------------------------------------------------
# Verifier + executor
# ---------------------------------------------------------------------------

SELF_HOSTED_MEM_LIMIT = 2_000_000 * 1024


@dataclass
class VerifyResult:
    status: str
    raw_json: str
    parsed: dict
    exit_code: int
    timed_out: bool


def run_verify(vow_binary: Path, source: str, timeout: int = 120, memory_limit: int | None = None, unwind: int | None = None) -> VerifyResult:
    with tempfile.NamedTemporaryFile(mode="w", suffix=".vow", delete=False, dir="/tmp") as f:
        f.write(source)
        tmp = f.name

    def _limit():
        if memory_limit:
            resource.setrlimit(resource.RLIMIT_AS, (memory_limit, memory_limit))

    cmd = [str(vow_binary), "verify"]
    if unwind is not None:
        cmd.extend(["--unwind", str(unwind)])
    cmd.append(tmp)

    try:
        result = subprocess.run(
            cmd,
            capture_output=True, text=True, timeout=timeout,
            preexec_fn=_limit if memory_limit else None,
        )
        raw = result.stdout.strip()
        try:
            parsed = json.loads(raw)
        except json.JSONDecodeError:
            parsed = {"status": "CompileFailed", "raw_stdout": raw, "stderr": result.stderr}
        return VerifyResult(parsed.get("status", "CompileFailed"), raw, parsed, result.returncode, False)
    except subprocess.TimeoutExpired:
        return VerifyResult("Timeout", "", {"status": "Timeout"}, -1, True)
    finally:
        Path(tmp).unlink(missing_ok=True)


def run_execute(vow_binary: Path, source: str, memory_limit: int | None = None, timeout: int = 60) -> tuple[int | None, str]:
    """Compile and execute, returning (exit_code, stdout)."""
    with tempfile.NamedTemporaryFile(mode="w", suffix=".vow", delete=False, dir="/tmp") as f:
        f.write(source)
        src_path = f.name

    out_path = src_path.replace(".vow", "")

    def _limit():
        if memory_limit:
            resource.setrlimit(resource.RLIMIT_AS, (memory_limit, memory_limit))

    try:
        # Compile
        comp = subprocess.run(
            [str(vow_binary), "build", "--no-verify", src_path, "-o", out_path],
            capture_output=True, text=True, timeout=timeout,
            preexec_fn=_limit if memory_limit else None,
        )
        if comp.returncode != 0:
            return None, comp.stderr

        # Execute
        exe = subprocess.run(
            [out_path], capture_output=True, text=True, timeout=timeout,
            preexec_fn=_limit if memory_limit else None,
        )
        return exe.returncode, exe.stdout.strip()
    except subprocess.TimeoutExpired:
        return None, "TIMEOUT"
    finally:
        Path(src_path).unlink(missing_ok=True)
        Path(out_path).unlink(missing_ok=True)


# ---------------------------------------------------------------------------
# Code extraction
# ---------------------------------------------------------------------------

def extract_vow_code(response: str) -> str | None:
    if not response.strip():
        return None
    fence = re.compile(r"```(?:vow)?\s*\n(.*?)```", re.DOTALL)
    matches = fence.findall(response)
    if matches:
        code = max(matches, key=len).strip()
        if "module " in code or "fn " in code:
            return code
    lines = response.split("\n")
    for i, line in enumerate(lines):
        if line.strip().startswith("module "):
            return "\n".join(lines[i:]).strip()
    if "fn " in response and ("module " in response or "vow {" in response):
        return response.strip()
    return None


# ---------------------------------------------------------------------------
# CEGIS runner
# ---------------------------------------------------------------------------

@dataclass
class ProblemResult:
    problem_id: str
    euler_number: int
    name: str
    difficulty: str
    expected_answer: int
    status: str  # verified / verify_failed / compile_failed / timeout / max_iterations / empty_response
    answer_correct: bool | None  # None if couldn't execute
    actual_output: str | None
    iterations: int
    wall_clock_seconds: float
    token_usage: dict[str, int]
    final_code: str
    raw_responses: list[str] = field(default_factory=list)
    verify_outputs: list[str] = field(default_factory=list)


def run_problem(
    problem: EulerProblem,
    model_config: ModelConfig,
    system_prompt: str,
    vow_binary: Path,
    max_cegis: int = 5,
    verify_timeout: int = 120,
    memory_limit: int | None = None,
    supports_unwind: bool = True,
) -> ProblemResult:
    start = time.time()
    messages: list[dict[str, str]] = []
    raw_responses: list[str] = []
    verify_outputs: list[str] = []
    total_in = total_out = 0
    final_code = ""
    vr: VerifyResult | None = None

    messages.append({"role": "user", "content": build_initial_prompt(problem.spec_md, problem.skeleton_vow)})

    for iteration in range(1, max_cegis + 1):
        resp = chat(model_config, system_prompt, messages)
        raw_responses.append(resp.content)
        total_in += resp.input_tokens
        total_out += resp.output_tokens
        messages.append({"role": "assistant", "content": resp.content})

        code = extract_vow_code(resp.content)
        if code is None:
            return ProblemResult(
                problem.id, problem.euler_number, problem.name, problem.difficulty,
                problem.answer, "empty_response", None, None, iteration,
                time.time() - start, {"input_tokens": total_in, "output_tokens": total_out},
                "", raw_responses, verify_outputs,
            )

        final_code = code
        unwind_arg = problem.unwind if supports_unwind else None
        vr = run_verify(vow_binary, code, timeout=verify_timeout, memory_limit=memory_limit, unwind=unwind_arg)
        verify_outputs.append(vr.raw_json)

        if vr.status == "Verified":
            # Verified! Now compile + execute to check the answer
            exit_code, output = run_execute(vow_binary, code, memory_limit=memory_limit)
            # Check if the first line of output matches the expected answer
            answer_correct = None
            actual_output = output
            if exit_code is not None and output:
                first_line = output.split("\n")[0].strip()
                try:
                    answer_correct = int(first_line) == problem.answer
                except ValueError:
                    answer_correct = False

            return ProblemResult(
                problem.id, problem.euler_number, problem.name, problem.difficulty,
                problem.answer, "verified", answer_correct, actual_output, iteration,
                time.time() - start, {"input_tokens": total_in, "output_tokens": total_out},
                final_code, raw_responses, verify_outputs,
            )

        if iteration < max_cegis:
            messages.append({"role": "user", "content": build_cegis_prompt(vr.raw_json)})

    # Exhausted iterations — preserve the terminal verifier failure mode
    # instead of collapsing every outcome into "max_iterations".
    elapsed = time.time() - start
    verifier_status_map = {
        "CompileFailed": "compile_failed",
        "VerifyFailed": "verify_failed",
        "Timeout": "timeout",
    }
    terminal_status = verifier_status_map.get(vr.status, "max_iterations") if vr is not None else "max_iterations"
    return ProblemResult(
        problem.id, problem.euler_number, problem.name, problem.difficulty,
        problem.answer, terminal_status, None, None, max_cegis,
        elapsed, {"input_tokens": total_in, "output_tokens": total_out},
        final_code, raw_responses, verify_outputs,
    )


# ---------------------------------------------------------------------------
# Report generation
# ---------------------------------------------------------------------------

def generate_report(results: list[dict], model_id: str) -> str:
    lines = []
    lines.append(f"# Euler Problems Demo Report — {model_id}")
    lines.append("")

    total = len(results)
    verified = sum(1 for r in results if r["status"] == "verified")
    correct = sum(1 for r in results if r.get("answer_correct") is True)

    lines.append(f"**Verified:** {verified}/{total} ({100*verified/total:.0f}%)")
    lines.append(f"**Correct answer:** {correct}/{total} ({100*correct/total:.0f}%)")
    lines.append("")

    # By difficulty
    for diff in ["easy", "medium", "hard"]:
        subset = [r for r in results if r["difficulty"] == diff]
        if not subset:
            continue
        v = sum(1 for r in subset if r["status"] == "verified")
        c = sum(1 for r in subset if r.get("answer_correct") is True)
        lines.append(f"**{diff.title()}:** {v}/{len(subset)} verified, {c}/{len(subset)} correct")

    lines.append("")
    lines.append("## Per-Problem Results")
    lines.append("")
    lines.append(f"| {'#':>3} | {'Euler':>5} | {'Problem':<25} | {'Diff':<6} | {'Status':<15} | {'Answer':>15} | {'Correct':<7} | {'Iters':>5} | {'Time':>6} |")
    lines.append(f"|{'---':>5}|{'---':>7}|{'---':<27}|{'---':<8}|{'---':<17}|{'---':>17}|{'---':<9}|{'---':>7}|{'---':>8}|")

    for i, r in enumerate(results, 1):
        correct_str = "Y" if r.get("answer_correct") is True else ("N" if r.get("answer_correct") is False else "?")
        actual = r.get("actual_output", "")
        if actual and "\n" in actual:
            actual = actual.split("\n")[0]
        if len(actual or "") > 15:
            actual = actual[:12] + "..."
        lines.append(
            f"| {i:>3} | {r['euler_number']:>5} | {r['name']:<25} | {r['difficulty']:<6} | {r['status']:<15} | {actual or '':>15} | {correct_str:<7} | {r['iterations']:>5} | {r['wall_clock_seconds']:>5.1f}s |"
        )

    lines.append("")

    # Token usage
    total_input = sum(r["token_usage"]["input_tokens"] for r in results)
    total_output = sum(r["token_usage"]["output_tokens"] for r in results)
    lines.append(f"**Total tokens:** {total_input:,} input, {total_output:,} output")

    verified_iters = [r["iterations"] for r in results if r["status"] == "verified"]
    if verified_iters:
        mean = sum(verified_iters) / len(verified_iters)
        lines.append(f"**Mean CEGIS iterations (verified):** {mean:.1f}")

    return "\n".join(lines)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def find_root() -> Path:
    return Path(__file__).resolve().parent.parent


def resolve_compiler(root: Path) -> tuple[Path, int | None, bool]:
    # Prefer self-hosted vowc (supports --unwind)
    vowc = root / "vowc"
    if vowc.exists():
        return vowc, SELF_HOSTED_MEM_LIMIT, True
    # Fall back to Rust stage-0 binary (does NOT accept --unwind)
    binary = root / "target" / "release" / "vow"
    if binary.exists():
        return binary, None, False
    print("Error: no vow compiler found. Run scripts/bootstrap.sh or cargo build --release", file=sys.stderr)
    sys.exit(1)


def cmd_run(args: argparse.Namespace) -> None:
    root = find_root()
    vow_binary, memory_limit, supports_unwind = resolve_compiler(root)
    system_prompt = build_system_prompt(root)
    results_dir = Path(__file__).resolve().parent / "results"

    run_id = args.run_id or time.strftime("%Y-%m-%dT%H:%M:%S")
    run_dir = results_dir / run_id
    run_dir.mkdir(parents=True, exist_ok=True)

    model_config = make_config(args.model)
    problems = load_problems(root, args.problem)

    if not problems:
        print(f"Error: no problems found (filter: {args.problem})", file=sys.stderr)
        sys.exit(1)

    output_file = run_dir / f"{args.model}.json"

    # Resume support
    existing: dict[str, dict] = {}
    if args.resume and output_file.exists():
        with open(output_file) as f:
            data = json.load(f)
        for r in data.get("results", []):
            existing[r["problem_id"]] = r

    print(f"Compiler: {vow_binary}")
    print(f"Model: {args.model}")
    print(f"Problems: {len(problems)}")
    print(f"Max CEGIS iterations: {args.max_cegis}")
    print()

    results: list[ProblemResult] = []
    for i, prob in enumerate(problems, 1):
        if prob.id in existing:
            print(f"  [{i}/{len(problems)}] Euler #{prob.euler_number} {prob.name} — skipped (resume)")
            results.append(_dict_to_result(existing[prob.id]))
            continue

        print(f"  [{i}/{len(problems)}] Euler #{prob.euler_number} {prob.name} ...", end=" ", flush=True)
        result = run_problem(
            prob, model_config, system_prompt, vow_binary,
            max_cegis=args.max_cegis,
            memory_limit=memory_limit,
            supports_unwind=supports_unwind,
        )
        results.append(result)

        status_str = result.status.upper()
        if result.status == "verified":
            answer_str = "CORRECT" if result.answer_correct else "WRONG"
            status_str = f"VERIFIED iter={result.iterations} {answer_str}"
        print(f"{status_str} [{result.wall_clock_seconds:.1f}s]")

        # Save incrementally
        _save(output_file, args.model, run_id, results)

    _save(output_file, args.model, run_id, results)
    print(f"\nResults saved to {output_file}")

    # Print summary
    result_dicts = [asdict(r) for r in results]
    report = generate_report(result_dicts, args.model)
    print()
    print(report)


def cmd_report(args: argparse.Namespace) -> None:
    results_dir = Path(__file__).resolve().parent / "results"
    if not args.run_id:
        runs = sorted(results_dir.iterdir()) if results_dir.exists() else []
        runs = [r for r in runs if r.is_dir()]
        if not runs:
            print("No results found", file=sys.stderr)
            sys.exit(1)
        args.run_id = runs[-1].name

    run_dir = results_dir / args.run_id
    for result_file in sorted(run_dir.glob("*.json")):
        with open(result_file) as f:
            data = json.load(f)
        report = generate_report(data["results"], data["model"])
        if args.output:
            Path(args.output).write_text(report)
            print(f"Report written to {args.output}")
        else:
            print(report)


def _save(output_file: Path, model_id: str, run_id: str, results: list[ProblemResult]) -> None:
    result_dicts = [asdict(r) for r in results]
    total = len(result_dicts)
    verified = sum(1 for r in result_dicts if r["status"] == "verified")
    correct = sum(1 for r in result_dicts if r.get("answer_correct") is True)
    data = {
        "run_id": run_id,
        "model": model_id,
        "results": result_dicts,
        "summary": {
            "total": total,
            "verified": verified,
            "correct_answer": correct,
            "verification_rate": verified / total if total else 0,
            "correctness_rate": correct / total if total else 0,
        },
    }
    with open(output_file, "w") as f:
        json.dump(data, f, indent=2)


def _dict_to_result(d: dict) -> ProblemResult:
    return ProblemResult(
        problem_id=d["problem_id"],
        euler_number=d["euler_number"],
        name=d["name"],
        difficulty=d["difficulty"],
        expected_answer=d["expected_answer"],
        status=d["status"],
        answer_correct=d.get("answer_correct"),
        actual_output=d.get("actual_output"),
        iterations=d["iterations"],
        wall_clock_seconds=d["wall_clock_seconds"],
        token_usage=d.get("token_usage", {"input_tokens": 0, "output_tokens": 0}),
        final_code=d.get("final_code", ""),
        raw_responses=d.get("raw_responses", []),
        verify_outputs=d.get("verify_outputs", []),
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Euler problems demo: AI agent solves + verifies in Vow")
    subparsers = parser.add_subparsers(dest="command")

    run_p = subparsers.add_parser("run", help="Run problems against an LLM")
    run_p.add_argument("--model", default="claude-sonnet-4-20250514", help="Model ID")
    run_p.add_argument("--problem", help="Run single problem by ID (e.g. E001)")
    run_p.add_argument("--resume", action="store_true", help="Skip already-completed problems")
    run_p.add_argument("--run-id", help="Run ID (default: timestamp)")
    run_p.add_argument("--max-cegis", type=int, default=5, help="Max CEGIS iterations (default: 5)")
    run_p.set_defaults(func=cmd_run)

    rep_p = subparsers.add_parser("report", help="Generate report from results")
    rep_p.add_argument("--run-id", help="Run ID (default: most recent)")
    rep_p.add_argument("--output", "-o", help="Output file")
    rep_p.set_defaults(func=cmd_report)

    args = parser.parse_args()
    if not args.command:
        parser.print_help()
        sys.exit(1)
    args.func(args)


if __name__ == "__main__":
    main()
